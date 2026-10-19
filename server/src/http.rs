//! 极简 HTTP/1.1 服务。
//!
//! 手写而不是引框架，是因为**这里的 HTTP 需求只有三个 GET**：
//! 查最新版本、下载产物、健康检查。没有表单、没有 multipart（发布是管理界面
//! 直接读本地文件，不经过网络）、没有长连接、没有 TLS。为此拉一个异步运行时进来
//! 会让单 exe 体积和依赖树都不划算。
//!
//! 因此只实现必要的部分：`GET` / `HEAD`、`Connection: close`（不做 keep-alive）、
//! 固定几个路由。任何超出这个范围的东西都直接 404 / 405。

use std::collections::VecDeque;
use std::fs::File;
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

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub time: String,
    pub peer: String,
    pub line: String,
    pub status: u16,
}

/// 服务端与请求处理线程共享的状态
pub struct Shared {
    pub repo: Mutex<Repo>,
    pub data_dir: PathBuf,
    pub logs: Mutex<VecDeque<LogEntry>>,
    /// 累计请求数（含失败）
    pub hits: AtomicU64,
    /// 累计下载次数
    pub downloads: AtomicU64,
}

impl Shared {
    pub fn new(data_dir: PathBuf, repo: Repo) -> Arc<Self> {
        Arc::new(Self {
            repo: Mutex::new(repo),
            data_dir,
            logs: Mutex::new(VecDeque::new()),
            hits: AtomicU64::new(0),
            downloads: AtomicU64::new(0),
        })
    }

    pub fn push_log(&self, peer: String, line: String, status: u16) {
        if let Ok(mut logs) = self.logs.lock() {
            logs.push_front(LogEntry {
                time: store::now_string(),
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
}

fn handle_connection(mut stream: TcpStream, peer: String, shared: &Arc<Shared>) {
    let request = match read_request(&stream) {
        Ok(r) => r,
        Err(err) => {
            let _ = write_simple(&mut stream, 400, "text/plain; charset=utf-8", err.as_bytes());
            shared.push_log(peer, "<解析失败>".into(), 400);
            return;
        }
    };

    let log_line = format!("{} {}", request.method, display_target(&request));

    let head_only = request.method == "HEAD";
    if request.method != "GET" && !head_only {
        let _ = write_simple(
            &mut stream,
            405,
            "text/plain; charset=utf-8",
            "仅支持 GET / HEAD".as_bytes(),
        );
        shared.push_log(peer, log_line, 405);
        return;
    }

    shared.hits.fetch_add(1, Ordering::Relaxed);
    let status = route(&mut stream, &request, shared, head_only, &peer);
    shared.push_log(peer, log_line, status);
}

fn display_target(req: &Request) -> String {
    if req.query.is_empty() {
        req.path.clone()
    } else {
        format!("{}?{}", req.path, req.query)
    }
}

/// 只读到请求头结束为止，不读 body（本服务没有带 body 的接口）
fn read_request(stream: &TcpStream) -> Result<Request, String> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("读取请求行失败：{e}"))?;
    if line.is_empty() {
        return Err("空请求".into());
    }

    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    if method.is_empty() || target.is_empty() {
        return Err("请求行格式错误".into());
    }

    // 把剩下的头读完（必须读干净，否则响应还没写完对方就 reset 了）
    let mut consumed = line.len();
    loop {
        let mut header = String::new();
        let n = reader
            .read_line(&mut header)
            .map_err(|e| format!("读取请求头失败：{e}"))?;
        if n == 0 {
            break;
        }
        consumed += n;
        if consumed > MAX_HEADER_BYTES {
            return Err("请求头过大".into());
        }
        if header == "\r\n" || header == "\n" {
            break;
        }
    }

    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target, String::new()),
    };
    Ok(Request {
        method,
        path,
        query,
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
    let path = req.path.trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };

    match path {
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
fn version_json(shared: &Arc<Shared>, current: &str) -> Vec<u8> {
    let repo = match shared.repo.lock() {
        Ok(r) => r,
        Err(_) => return br#"{"error":"repo locked"}"#.to_vec(),
    };

    let Some(latest) = repo.latest_release() else {
        // 还没发布过任何版本：明确告诉客户端"没有更新"，别让它去解析空字段
        return br#"{"update":false,"latest":"","notes":"","force":false,"url":"","size":0}"#
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

    let payload = serde_json::json!({
        "update": has_update,
        "latest": latest.version,
        "notes": latest.notes,
        "force": latest.force,
        "url": format!("/api/download/{}", latest.version),
        "size": size,
        "published_at": latest.published_at,
    });
    serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec())
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

    let header = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: application/octet-stream\r\n\
         Content-Length: {size}\r\n\
         Content-Disposition: attachment; filename=\"{version}.exe\"\r\n\
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
                    abort = Some(format!("发送失败：{e}"));
                    break;
                }
                sent += n as u64;
            }
            Err(e) => {
                abort = Some(format!("读取产物失败：{e}"));
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
        404 => "Not Found",
        405 => "Method Not Allowed",
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
