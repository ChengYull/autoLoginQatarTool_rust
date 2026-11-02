//! 极简 HTTP/1.1 服务。
//!
//! 手写而不是引框架，是因为**这里的 HTTP 需求只有四个接口**：
//! 查版本（含历史列表）、下载产物、健康检查、远程发布。没有表单、没有 multipart、
//! 没有长连接、没有 TLS。为此拉一个异步运行时进来会让单 exe 体积和依赖树都不划算。
//!
//! 因此只实现必要的部分：`GET` / `HEAD` / `POST`（仅 `/api/publish`）、
//! `Connection: close`（不做 keep-alive）、固定几个路由。任何超出这个范围的东西
//! 都直接 404 / 405。

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::store::{self, Repo};

/// 访问日志最多留这么多条（界面只显示最近一屏，多了没意义还占内存）
const MAX_LOGS: usize = 200;

/// 单次请求的读取上限，防住畸形请求把内存吃光
const MAX_HEADER_BYTES: usize = 16 * 1024;

/// 远程发布允许的最大请求体。客户端 exe 现在约 620 KB，留 32 MB 足够，
/// 又不至于让一个乱发的请求把内存吃光。
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;


#[derive(Clone, Debug)]
pub struct LogEntry {
    pub time: String,
    pub peer: String,
    pub line: String,
    pub status: u16,
}

/// 日志文件名（放在数据目录下，方便「打开数据目录」直接取）
pub const LOG_FILE: &str = "server.log";

/// 服务端与请求处理线程共享的状态
pub struct Shared {
    pub repo: Mutex<Repo>,
    pub data_dir: PathBuf,
    pub logs: Mutex<VecDeque<LogEntry>>,
    /// 落盘日志。界面只显示最近几十条且会被截断，出问题时要能拿到完整记录。
    log_file: Mutex<Option<File>>,
    /// 远程发布接口的令牌（`X-Publish-Token` 头要比对它）
    pub publish_token: String,
    /// 累计请求数（含失败）
    pub hits: AtomicU64,
    /// 累计下载次数
    pub downloads: AtomicU64,
    /// 上传临时文件的序号（避免并发发布互相覆盖同一个临时文件）
    upload_seq: AtomicU64,
}

impl Shared {
    pub fn new(data_dir: PathBuf, repo: Repo, publish_token: String) -> Arc<Self> {
        // 追加模式：服务端重启不清空历史，方便对照「上一次是怎么坏的」
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(data_dir.join(LOG_FILE))
            .ok();

        Arc::new(Self {
            repo: Mutex::new(repo),
            data_dir,
            logs: Mutex::new(VecDeque::new()),
            log_file: Mutex::new(log_file),
            publish_token,
            hits: AtomicU64::new(0),
            downloads: AtomicU64::new(0),
            upload_seq: AtomicU64::new(0),
        })
    }

    fn next_upload_path(&self) -> PathBuf {
        let n = self.upload_seq.fetch_add(1, Ordering::Relaxed);
        self.data_dir.join(format!(".upload-{n}.tmp"))
    }

    pub fn push_log(&self, peer: String, line: String, status: u16) {
        let time = store::now_string();

        // 先落盘（完整、不截断），再放进界面用的环形缓冲
        if let Ok(mut guard) = self.log_file.lock() {
            if let Some(file) = guard.as_mut() {
                let _ = writeln!(file, "{time} [{status}] {peer}  {line}");
                let _ = file.flush();
            }
        }

        if let Ok(mut logs) = self.logs.lock() {
            logs.push_front(LogEntry {
                time,
                peer,
                line,
                status,
            });
            while logs.len() > MAX_LOGS {
                logs.pop_back();
            }
        }
    }

    pub fn snapshot_logs(&self) -> Vec<LogEntry> {
        self.logs
            .lock()
            .map(|l| l.iter().cloned().collect())
            .unwrap_or_default()
    }
}

/// 正在运行的监听线程
pub struct Server {
    stop: Arc<AtomicBool>,
    addr: String,
}

impl Server {
    /// 启动监听。`bind` 形如 `0.0.0.0:18082`。
    ///
    /// 立即返回，接受连接的工作在后台线程里做 —— 界面线程不能被 accept 阻塞住。
    pub fn start(bind: &str, shared: Arc<Shared>) -> Result<Self, String> {
        let listener = TcpListener::bind(bind).map_err(|e| format!("无法监听 {bind}：{e}"))?;
        // 非阻塞 + 轮询停止标志：Rust 的 TcpListener 没有 shutdown，
        // 想让 accept 循环能被外部叫停，这是最省事的办法（50ms 一次，开销可忽略）。
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("设置非阻塞失败：{e}"))?;

        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let addr = bind.to_string();

        std::thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, peer)) => {
                        // **关键：Windows 上 accept 返回的 socket 会继承 listener 的非阻塞属性**
                        // （Linux 不会继承）。我们为了让 accept 循环能被停止标志叫停，
                        // 把 listener 设成了非阻塞 —— 结果每条连接也成了非阻塞的：
                        // `write_all` 一碰到发送缓冲区满就返回 WouldBlock(10035) 而不是等待，
                        // 几百 KB 的文件必然在 64KB（发送缓冲区大小）处断掉。
                        // 跨网络时缓冲区很容易满，所以表现得像"必现"；本机回环太快，
                        // 缓冲区从不满，所以本地测试一直是对的 —— 这就是为什么查了这么久。
                        // 另外非阻塞模式下 set_read_timeout / set_write_timeout 也是无效的。
                        if let Err(err) = stream.set_nonblocking(false) {
                            crate::tlog!("切换阻塞模式失败：{err}");
                        }
                        let shared = shared.clone();
                        std::thread::spawn(move || {
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                            let _ = stream.set_write_timeout(Some(Duration::from_secs(60)));
                            handle_connection(stream, peer.to_string(), &shared);
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(200)),
                }
            }
        });

        Ok(Self { stop, addr })
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// 请求处理
// ---------------------------------------------------------------------------

struct Request {
    method: String,
    path: String,
    query: String,
    /// 头名统一小写存，取的时候不用关心大小写
    headers: Vec<(String, String)>,
    /// 请求体。只有 POST 会读，GET / HEAD 恒为空。
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

fn handle_connection(mut stream: TcpStream, peer: String, shared: &Arc<Shared>) {
    let request = match read_request(&stream) {
        Ok(r) => r,
        Err((status, err)) => {
            let _ = write_simple(&mut stream, status, "text/plain; charset=utf-8", err.as_bytes());
            shared.push_log(peer, "<解析失败>".into(), status);
            return;
        }
    };

    let log_line = format!("{} {}", request.method, display_target(&request));

    let head_only = request.method == "HEAD";
    let is_post = request.method == "POST";
    let path = normalize_path(&request.path);
    // POST 只开放给远程发布，别的一律 405 —— 免得以后加了新路由忘记考虑写操作
    let allowed = request.method == "GET" || head_only || (is_post && path == "/api/publish");
    if !allowed {
        let _ = write_simple(
            &mut stream,
            405,
            "text/plain; charset=utf-8",
            "仅支持 GET / HEAD（POST 只有 /api/publish）".as_bytes(),
        );
        shared.push_log(peer, log_line, 405);
        return;
    }

    shared.hits.fetch_add(1, Ordering::Relaxed);
    let status = if is_post {
        remote_publish(&mut stream, &request, shared, &peer)
    } else {
        route(&mut stream, &request, shared, head_only, &peer)
    };
    shared.push_log(peer, log_line, status);
}

fn display_target(req: &Request) -> String {
    if req.query.is_empty() {
        req.path.clone()
    } else {
        format!("{}?{}", req.path, req.query)
    }
}

fn normalize_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 读完请求行 + 请求头（+ POST 的请求体）。
///
/// 出错时返回 `(HTTP 状态码, 说明)`：状态码要能区分「请求本身畸形」（400）和
/// 「请求体太大」（413），不然客户端只能看到一个笼统的失败。
///
/// **请求体必须在这里一次读完**：`BufReader` 可能已经把 body 的前几百字节
/// 读进自己的缓冲区了，函数返回后再从 `TcpStream` 上读就会丢掉那一截。
fn read_request(stream: &TcpStream) -> Result<Request, (u16, String)> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| (400, format!("读取请求行失败：{e}")))?;
    if line.is_empty() {
        return Err((400, "空请求".into()));
    }

    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    if method.is_empty() || target.is_empty() {
        return Err((400, "请求行格式错误".into()));
    }

    // 把剩下的头读完（必须读干净，否则响应还没写完对方就 reset 了）
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut consumed = line.len();
    loop {
        let mut header = String::new();
        let n = reader
            .read_line(&mut header)
            .map_err(|e| (400, format!("读取请求头失败：{e}")))?;
        if n == 0 {
            break;
        }
        consumed += n;
        if consumed > MAX_HEADER_BYTES {
            return Err((400, "请求头过大".into()));
        }
        if header == "\r\n" || header == "\n" {
            break;
        }
        if let Some((k, v)) = header.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }

    // 请求体：只按 Content-Length 读。不支持 chunked —— 我们的客户端不会发。
    let mut body = Vec::new();
    let declared = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse::<usize>().ok());
    if let Some(len) = declared {
        if len > MAX_BODY_BYTES {
            return Err((
                413,
                format!(
                    "请求体过大：{len} 字节（上限 {} 字节）",
                    MAX_BODY_BYTES
                ),
            ));
        }
        body.resize(len, 0);
        reader
            .read_exact(&mut body)
            .map_err(|e| (400, format!("读取请求体失败（声明 {len} 字节）：{e}")))?;
    }

    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target, String::new()),
    };
    Ok(Request {
        method,
        path,
        query,
        headers,
        body,
    })
}


/// 返回 HTTP 状态码（用于访问日志）
fn route(
    stream: &mut TcpStream,
    req: &Request,
    shared: &Arc<Shared>,
    head_only: bool,
    peer: &str,
) -> u16 {
    let path = normalize_path(&req.path);

    match path.as_str() {
        "/api/health" => {
            let body = br#"{"status":"ok"}"#;
            let _ = write_simple(stream, 200, "application/json; charset=utf-8", body);
            200
        }
        "/api/version" => {
            let current = query_param(&req.query, "current").unwrap_or_default();
            let body = version_json(shared, &current);
            let _ = write_body(stream, 200, "application/json; charset=utf-8", &body, head_only);
            200
        }
        _ if path.starts_with("/api/download/") => {
            let version = path.trim_start_matches("/api/download/");
            download(stream, version, shared, head_only, peer)
        }
        _ => {
            let _ = write_simple(
                stream,
                404,
                "text/plain; charset=utf-8",
                "没有这个接口".as_bytes(),
            );
            404
        }
    }
}

/// `/api/version` 的响应体
///
/// 字段设计成「服务端把判断结果也算好」：`update` 直接告诉客户端要不要更新，
/// 但同时也给出 `latest`，让客户端可以自己再比一次（两边都用同一套分段比较规则）。
/// 客户端只依赖 `latest` 也不会错。
///
/// `releases` 是**全部历史版本**，顺序由 [`Repo::display_order`] 决定：
/// 当前对外版本永远在第一个，其余按版本号从新到旧。客户端拿它渲染「更新说明」，
/// 老客户端不认识这个字段会直接忽略，不影响它们升级。
fn version_json(shared: &Arc<Shared>, current: &str) -> Vec<u8> {
    let repo = match shared.repo.lock() {
        Ok(r) => r,
        Err(_) => return br#"{"error":"repo locked"}"#.to_vec(),
    };

    let Some(latest) = repo.latest_release() else {
        // 还没发布过任何版本：明确告诉客户端"没有更新"，别让它去解析空字段
        return br#"{"update":false,"latest":"","notes":"","force":false,"url":"","size":0,"releases":[]}"#
            .to_vec();
    };

    let has_update = if current.is_empty() {
        false
    } else {
        store::compare_version(&latest.version, current) == std::cmp::Ordering::Greater
    };

    // size 同样以磁盘上的实际文件为准。内存里的是"发布那一刻"的快照，
    // 文件后来被重新发布/手工替换过的话就会过期 —— 客户端拿它做下载校验，
    // 一旦偏小，截断的 exe 反而能通过校验（见 download() 里的说明）。
    let size = std::fs::metadata(shared.data_dir.join(store::FILES_DIR).join(&latest.file))
        .map(|m| m.len())
        .unwrap_or(latest.size);

    let releases: Vec<serde_json::Value> = repo
        .display_order()
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "version": r.version,
                "notes": r.notes,
                "force": r.force,
                "published_at": r.published_at,
                "size": r.size,
                "latest": r.version == repo.latest,
            })
        })
        .collect();

    let payload = serde_json::json!({
        "update": has_update,
        "latest": latest.version,
        "notes": latest.notes,
        "force": latest.force,
        "url": format!("/api/download/{}", latest.version),
        "size": size,
        "published_at": latest.published_at,
        "releases": releases,
    });
    serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec())
}

/// 远程发布：`POST /api/publish`，请求体就是 exe 的原始字节。
///
/// 为什么用原始字节而不是 multipart / base64：
/// - multipart 要写边界解析，几百行代码换不来什么；
/// - base64 会膨胀 33%，还得先全量解码；
/// - `curl --data-binary @app.exe` 直接就能用，脚本化部署最省事。
///
/// 版本号**不接收参数**，一律从上传文件的 `VERSIONINFO` 里读 —— 和界面发布
/// 走同一个 `store::publish`，所以「接口发出来的版本号」和「exe 里写的」不可能不一致。
///
/// 说明文字走 query（`?notes=...`，百分号编码）或 `X-Publish-Notes` 头，
/// 强制更新走 `?force=1`。
fn remote_publish(
    stream: &mut TcpStream,
    req: &Request,
    shared: &Arc<Shared>,
    peer: &str,
) -> u16 {
    // ---- 鉴权 ----
    // 发布接口能把任意 exe 塞进仓库，而客户端拿到更新后会下载并替换自己 ——
    // 裸奔等于内网里谁都能给别人的机器投毒，所以必须带令牌。
    let given = req.header("x-publish-token").unwrap_or_default();
    if given.is_empty() || !token_matches(given, &shared.publish_token) {
        shared.push_log(
            peer.to_string(),
            "发布被拒绝：令牌缺失或不匹配".into(),
            401,
        );
        return json_reply(
            stream,
            401,
            &serde_json::json!({
                "ok": false,
                "error": "缺少或错误的发布令牌。令牌在服务端数据目录 server.json 的 publish_token 字段里，请求时放在 X-Publish-Token 头里。"
            }),
        );
    }

    if req.body.is_empty() {
        return json_reply(
            stream,
            400,
            &serde_json::json!({ "ok": false, "error": "请求体是空的，应该把 exe 的原始字节放进来" }),
        );
    }

    let notes = req
        .header("x-publish-notes")
        .map(|v| percent_decode(v))
        .or_else(|| query_param(&req.query, "notes"))
        .unwrap_or_default();
    // 说明会被原样返回给客户端，长度卡一下免得有人往 releases.json 里灌大文本
    let notes: String = notes.trim().chars().take(2000).collect();
    let force = req
        .header("x-publish-force")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .or_else(|| query_param(&req.query, "force").map(|v| v == "1" || v.eq_ignore_ascii_case("true")))
        .unwrap_or(false);

    // ---- 落成临时文件 ----
    // store::publish 需要一个真实文件：版本号是从 PE 的 VERSIONINFO 资源里读的，
    // 而那要走 GetFileVersionInfoW，只能对着路径读。
    if let Err(err) = std::fs::create_dir_all(&shared.data_dir) {
        return json_reply(
            stream,
            500,
            &serde_json::json!({ "ok": false, "error": format!("创建数据目录失败：{err}") }),
        );
    }
    let tmp = shared.next_upload_path();
    if let Err(err) = std::fs::write(&tmp, &req.body) {
        let _ = std::fs::remove_file(&tmp);
        return json_reply(
            stream,
            500,
            &serde_json::json!({ "ok": false, "error": format!("写入临时文件失败：{err}") }),
        );
    }

    let result = store::publish(&shared.data_dir, &tmp, &notes, force);
    // 临时文件用完就删：store::publish 已经把内容复制进 files/ 了
    let _ = std::fs::remove_file(&tmp);

    let release = match result {
        Ok(r) => r,
        Err(err) => {
            shared.push_log(peer.to_string(), format!("发布失败：{err}"), 400);
            return json_reply(
                stream,
                400,
                &serde_json::json!({ "ok": false, "error": err }),
            );
        }
    };

    // ---- 入库 ----
    // **必须持着 repo 锁做「改 + 落盘」**：界面线程发布时也走这把锁（见 app.rs
    // 的 mutate_repo），两边各自手里都有一份完整仓库，不串行化就会互相覆盖 ——
    // 典型后果是「远程发完，界面那边一保存，新版本从 releases.json 里消失」。
    let (promote, latest_now) = {
        let Ok(mut repo) = shared.repo.lock() else {
            return json_reply(
                stream,
                500,
                &serde_json::json!({ "ok": false, "error": "仓库锁不可用" }),
            );
        };
        let promote = repo.apply_publish(release.clone());
        let latest_now = repo.latest.clone();
        if let Err(err) = repo.save(&shared.data_dir) {
            return json_reply(
                stream,
                500,
                &serde_json::json!({
                    "ok": false,
                    "error": format!("产物已就位，但写入 releases.json 失败：{err}。请检查磁盘权限。")
                }),
            );
        }
        (promote, latest_now)
    };

    json_reply(
        stream,
        200,
        &serde_json::json!({
            "ok": true,
            "version": release.version,
            "size": release.size,
            "latest": latest_now,
            "promoted": promote,
        }),
    )
}

/// 定长比较，避免用 `==` 提前返回泄漏令牌前缀（内网也顺手做对）
fn token_matches(given: &str, expected: &str) -> bool {
    let (a, b) = (given.as_bytes(), expected.as_bytes());
    if a.len() != b.len() || b.is_empty() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

fn json_reply(stream: &mut TcpStream, status: u16, value: &serde_json::Value) -> u16 {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    let _ = write_body(stream, status, "application/json; charset=utf-8", &body, false);
    status
}


/// 下载某个版本的产物，流式写出（不把整个 exe 读进内存）
fn download(
    stream: &mut TcpStream,
    version: &str,
    shared: &Arc<Shared>,
    head_only: bool,
    peer: &str,
) -> u16 {
    let (path, recorded_size) = {
        let Ok(mut repo) = shared.repo.lock() else {
            let _ = write_simple(stream, 500, "text/plain; charset=utf-8", "内部错误".as_bytes());
            return 500;
        };
        // 先把要用的值取出来，结束对 repo 的不可变借用 —— 下面还要可变借用来加计数。
        // 这里只取文件名：文件大小要用磁盘上的实际值，见下面 File::open 之后的说明。
        let found = repo
            .releases
            .iter()
            .find(|r| r.version == version)
            .map(|r| (r.file.clone(), r.size));
        let Some((file_name, recorded_size)) = found else {
            drop(repo);
            let _ = write_simple(
                stream,
                404,
                "text/plain; charset=utf-8",
                format!("没有版本 {version}").as_bytes(),
            );
            return 404;
        };
        // 计数在这里加：能走到这一步说明版本存在，下载是"真的发生了"。
        //
        // **但只改内存，不落盘** —— 磁盘写入统一由界面线程负责。
        // 这里 save 会和「界面正在发布」撞车：界面先 add 再 save，中间还有个
        // sync_shared_repo 的窗口期，此刻 HTTP 线程手里是不含新版本的旧仓库，
        // 它一 save 就把刚发布的版本从 releases.json 里抹掉了。
        // 界面线程的定时器会把计数回读并落盘，最坏只丢一秒的计数，不会丢版本。
        if let Some(r) = repo.releases.iter_mut().find(|r| r.version == version) {
            r.downloads += 1;
        }
        (
            shared.data_dir.join(store::FILES_DIR).join(file_name),
            recorded_size,
        )
    };

    shared.downloads.fetch_add(1, Ordering::Relaxed);

    let Ok(mut file) = File::open(&path) else {
        let _ = write_simple(
            stream,
            500,
            "text/plain; charset=utf-8",
            "产物文件丢失，请重新发布".as_bytes(),
        );
        return 500;
    };

    // **Content-Length 必须用文件的实际大小，不能用仓库里记的 size。**
    //
    // 内存里的数字只反映"发布那一刻"的文件大小。一旦文件被重新发布、
    // 手工拷贝、或者服务端启动后磁盘文件被替换过，两者就会对不上，后果很严重：
    //   · 声明偏小 → 客户端按 Content-Length 读完就停，拿到**截断的 exe**，
    //     而且字节数刚好等于它期望的 size，校验居然能通过，最后把自己换成坏文件；
    //   · 声明偏大 → 客户端一直等剩余数据，直到超时。
    // 磁盘上的文件才是唯一真相。
    let size = match file.metadata() {
        Ok(meta) => meta.len(),
        Err(err) => {
            let _ = write_simple(
                stream,
                500,
                "text/plain; charset=utf-8",
                format!("读取产物信息失败：{err}").as_bytes(),
            );
            return 500;
        }
    };

    // 磁盘文件必须和仓库记录的大小一致。不一致说明文件被拷坏了 / 拷贝中断了 /
    // 只替换了一半 —— 这种时候**宁可拒绝，也不能把半截 exe 发出去**：
    // 客户端拿到半截文件后可能会拿它覆盖自己，程序就再也起不来了。
    if recorded_size > 0 && size != recorded_size {
        let _ = write_simple(
            stream,
            500,
            "text/plain; charset=utf-8",
            format!(
                "产物文件损坏：磁盘上 {size} 字节，记录应为 {recorded_size} 字节。请在服务端重新发布该版本。"
            )
            .as_bytes(),
        );
        return 500;
    }

    // 下载文件名带完整产品名：网页上点按钮下载时，浏览器会用这个名字存盘。
    // 只写版本号的话用户拿到的是 "1.0.4.exe"，放进下载目录根本认不出来。
    let header = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: application/octet-stream\r\n\
         Content-Length: {size}\r\n\
         Content-Disposition: attachment; filename=\"AutoLoginQatarTool-{version}.exe\"\r\n\
         Connection: close\r\n\
         \r\n"
    );
    if stream.write_all(header.as_bytes()).is_err() {
        return 200;
    }
    if head_only {
        return 200;
    }

    // 64KB 一块往前推，边读边写。
    // 中断原因一定要记下来 —— 以前这里是 `if ...is_err() { break; }`，
    // 断了就断了，界面上只看到一条 200，完全不知道发到哪、为什么停。
    let mut buf = vec![0u8; 64 * 1024];
    let mut sent: u64 = 0;
    let mut abort: Option<String> = None;
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Err(e) = stream.write_all(&buf[..n]) {
                    // 错误码很关键：10054=对端重置，10053=本机中止，10060=超时。
                    // 光写「发送失败」等于没说，三种原因的处理方式完全不同。
                    abort = Some(format!(
                        "发送失败：{e}（kind={:?}, os={:?}）",
                        e.kind(),
                        e.raw_os_error()
                    ));
                    break;
                }
                sent += n as u64;
            }
            Err(e) => {
                abort = Some(format!(
                    "读取产物失败：{e}（kind={:?}, os={:?}）",
                    e.kind(),
                    e.raw_os_error()
                ));
                break;
            }
        }
    }
    let _ = stream.flush();

    if sent != size || abort.is_some() {
        // 正常下载不记（避免刷屏），只有没发全才记一条到界面日志
        shared.push_log(
            peer.to_string(),
            format!(
                "下载 {version} 中断：已发 {sent}/{size} 字节{}",
                abort
                    .as_deref()
                    .map(|r| format!("（{r}）"))
                    .unwrap_or_default()
            ),
            500,
        );
    }
    200
}

// ---------------------------------------------------------------------------
// 响应工具
// ---------------------------------------------------------------------------

fn write_simple(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) -> std::io::Result<()> {
    write_body(stream, status, content_type, body, false)
}

fn write_body(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    head_only: bool,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        _ => "Unknown",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         Cache-Control: no-store\r\n\
         \r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    if !head_only {
        stream.write_all(body)?;
    }
    stream.flush()
}

/// 从查询串里取一个参数（只做 `&` / `=` 拆分，值做最小限度的百分号解码）
pub fn query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=')?;
        if k == key {
            return Some(percent_decode(v));
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_param_picks_the_right_value() {
        assert_eq!(query_param("current=1.0.1", "current").as_deref(), Some("1.0.1"));
        assert_eq!(query_param("a=1&current=2.0", "current").as_deref(), Some("2.0"));
        assert_eq!(query_param("a=1&b=2", "current"), None);
        // 版本号里的点不需要转义，但万一客户端编码了也要能解回来
        assert_eq!(query_param("current=1%2E0%2E1", "current").as_deref(), Some("1.0.1"));
    }
}
