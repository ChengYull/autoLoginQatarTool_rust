//! 更新检查：向更新服务端问一次最新版本，决定标题栏版本徽章的颜色。
//!
//! 服务端是同级 `server/` 目录里的 `AutoLoginQatarServer`，接口见 `server/src/http.rs`。
//! 客户端只做「查 + 显示」，不自动下载替换 —— 那是下一步的事。

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::net;
use crate::version::{self, VersionState};

/// 默认更新服务端地址
const DEFAULT_HOST: &str = "10.170.108.115";
const DEFAULT_PORT: u16 = 18082;

/// 重新检查的间隔（6 小时）。程序常驻托盘可能跑很久，只在启动时查一次不够。
pub const CHECK_INTERVAL_MS: u32 = 6 * 60 * 60 * 1000;

/// 启动后多久做第一次检查：等界面先出来，别跟初始化抢时间
pub const FIRST_CHECK_DELAY_MS: u32 = 3000;

/// 一个历史版本的说明（服务端 `/api/version` 里 `releases` 数组的元素）
#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub version: String,
    pub notes: String,
    pub published_at: String,
    pub force: bool,
    /// 是否是当前对外版本（服务端 `latest` 指向的那个）
    pub is_latest: bool,
}

/// 一次检查的结果
#[derive(Debug, Clone)]
pub struct UpdateOutcome {
    pub state: VersionState,
    /// 服务端上的最新版本号，取不到时为空串
    pub latest: String,
    pub force: bool,
    pub notes: String,
    /// 服务端给的产物字节数，用于下载后校验；0 表示服务端没给
    pub size: u64,
    /// 全部历史版本，**服务端已经按「最新版在最前、其余从新到旧」排好序**。
    /// 老服务端不返回这个字段时为空，展示时退回只用 `latest` + `notes`。
    pub releases: Vec<ReleaseInfo>,
}

impl UpdateOutcome {
    fn unknown() -> Self {
        Self {
            state: VersionState::Latest,
            latest: String::new(),
            force: false,
            notes: String::new(),
            size: 0,
            releases: Vec::new(),
        }
    }

    /// 拼出「更新说明」对话框要显示的正文：所有历史版本倒序，最新版在最上。
    ///
    /// 服务端没给 `releases`（老版本服务端）时，用 `latest` + `notes` 兜一条，
    /// 保证界面永远有内容可显示，不会弹出一个空框。
    pub fn history_text(&self) -> String {
        let mut list: Vec<ReleaseInfo> = self.releases.clone();
        if list.is_empty() {
            if self.latest.is_empty() {
                return String::new();
            }
            list.push(ReleaseInfo {
                version: self.latest.clone(),
                notes: self.notes.clone(),
                published_at: String::new(),
                force: self.force,
                is_latest: true,
            });
        }

        let mut out = String::new();
        for (i, r) in list.iter().enumerate() {
            if i > 0 {
                out.push_str("\r\n\r\n");
            }
            // 标题行：版本号 + 发布时间 + 标记
            out.push_str(&format!("v{}", r.version));
            if !r.published_at.is_empty() {
                out.push_str(&format!("    {}", r.published_at));
            }
            if r.is_latest {
                out.push_str("    [最新]");
            }
            if r.force {
                out.push_str("    [强制]");
            }
            out.push_str("\r\n");

            // 说明正文：多行时缩进两格，和标题行区分开
            let notes = r.notes.trim();
            if notes.is_empty() {
                out.push_str("  （这个版本没有填写更新说明）\r\n");
            } else {
                for line in notes.replace("\r\n", "\n").split('\n') {
                    out.push_str("  ");
                    out.push_str(line.trim_end());
                    out.push_str("\r\n");
                }
            }
        }
        out
    }
}


/// 服务端地址，允许用环境变量覆盖（自测与灰度用）
pub fn endpoint() -> (String, u16) {
    let host = std::env::var("AUTOLOGIN_UPDATE_HOST").unwrap_or_else(|_| DEFAULT_HOST.to_string());
    let port = std::env::var("AUTOLOGIN_UPDATE_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);
    (host, port)
}

/// 检查一次。**阻塞调用**，必须放到后台线程里跑。
///
/// 请求失败一律当成「已是最新」：网络不通时把徽章变黄，用户既没法处理、
/// 也没法判断是网络问题还是真有新版本，只会造成困扰。真有问题去看服务端日志。
pub fn check() -> UpdateOutcome {
    let current = version::APP_VERSION;
    let (host, port) = endpoint();
    let path = format!("/api/version?current={current}");

    let Ok(body) = net::http_get_on(&host, port, &path) else {
        return UpdateOutcome::unknown();
    };

    let latest = json_field(&body, "latest").unwrap_or_default();
    let state = if latest.is_empty() || !version::is_newer(&latest, current) {
        VersionState::Latest
    } else {
        VersionState::Outdated
    };

    UpdateOutcome {
        state,
        latest,
        force: json_field(&body, "force").map(|v| v == "true").unwrap_or(false),
        notes: json_field(&body, "notes").unwrap_or_default(),
        size: json_field(&body, "size")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0),
        releases: parse_releases(&body),
    }
}

/// 解析 `"releases":[{...},{...}]`。
///
/// 顺序照抄服务端 —— 它已经把当前对外版本排在第一，客户端不再自己排，
/// 免得两边规则不一致（比如回滚之后「版本号最大的」并不是该升级到的那个）。
fn parse_releases(body: &str) -> Vec<ReleaseInfo> {
    array_objects(body, "releases")
        .iter()
        .filter_map(|obj| {
            let version = json_field(obj, "version").unwrap_or_default();
            if version.is_empty() {
                return None;
            }
            Some(ReleaseInfo {
                version,
                notes: json_field(obj, "notes").unwrap_or_default(),
                published_at: json_field(obj, "published_at").unwrap_or_default(),
                force: json_field(obj, "force").map(|v| v == "true").unwrap_or(false),
                is_latest: json_field(obj, "latest").map(|v| v == "true").unwrap_or(false),
            })
        })
        .collect()
}

/// 从扁平 JSON 里取出 `"key": [...]` 这个数组里的每个 `{...}` 对象（原文）。
///
/// 必须做字符串感知的扫描：更新说明里完全可能出现 `]`、`}`、引号，直接按字符
/// 找括号会在说明文字中间断开，把一条说明拆成两条。
fn array_objects(body: &str, key: &str) -> Vec<String> {
    let needle = format!("\"{key}\"");
    let bytes = body.as_bytes();

    // 先定位到数组的 '['：要求 key 前面是 `{` 或 `,`（和 json_field 同样的规则），
    // 否则会匹配到某条说明正文里出现的同名字样。
    let mut from = 0usize;
    let start = loop {
        let Some(pos) = body[from..].find(&needle) else {
            return Vec::new();
        };
        let at = from + pos;
        let before = body[..at].trim_end().bytes().last();
        if matches!(before, None | Some(b'{') | Some(b',')) {
            let mut i = at + needle.len();
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b':' {
                i += 1;
            }
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'[' {
                break i + 1;
            }
        }
        from = at + needle.len();
    };

    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    let mut obj_start: Option<usize> = None;

    for (offset, ch) in body[start..].char_indices() {
        let idx = start + offset;
        if in_str {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        match ch {
            '"' => in_str = true,
            '{' => {
                if depth == 0 {
                    obj_start = Some(idx);
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = obj_start.take() {
                        out.push(body[s..=idx].to_string());
                    }
                }
            }
            ']' if depth == 0 => break,
            _ => {}
        }
    }
    out
}


// ---------------------------------------------------------------------------
// 下载与安装
// ---------------------------------------------------------------------------

/// 已经下载并校验过、等待替换的更新包
pub struct StagedUpdate {
    pub version: String,
    pub path: PathBuf,
    pub size: u64,
}

/// 把指定版本下载到临时目录并做基本校验。
///
/// 校验是必须的：更新流程下一步就是拿这个文件覆盖自己的 exe，
/// 万一下载到的是错误页或者半截文件，程序就再也起不来了。
pub fn download(latest: &str, expect_size: u64) -> Result<StagedUpdate, String> {
    // 传输中断是真实存在的（超时、连接被重置、网络抖动），而这里失败一次
    // 就意味着用户看到"下载失败"弹框、还得自己再点一次。重试两轮能挡掉绝大多数偶发。
    const ATTEMPTS: u32 = 3;
    let mut last_err = String::new();
    for attempt in 1..=ATTEMPTS {
        match download_once(latest, expect_size) {
            Ok(staged) => return Ok(staged),
            Err(err) => {
                crate::tlog!("下载 v{latest} 第 {attempt}/{ATTEMPTS} 次失败：{err}");
                last_err = err;
                if attempt < ATTEMPTS {
                    std::thread::sleep(std::time::Duration::from_millis(600));
                }
            }
        }
    }
    Err(format!("重试 {ATTEMPTS} 次仍失败：{last_err}"))
}

fn download_once(latest: &str, expect_size: u64) -> Result<StagedUpdate, String> {
    let (host, port) = endpoint();
    let path = format!("/api/download/{latest}");
    let dest = std::env::temp_dir().join(format!("AutoLoginQatarTool-{latest}.new.exe"));
    let _ = std::fs::remove_file(&dest); // 清掉上次的残留

    let written = net::http_download_to(&host, port, &path, &dest)?;

    if expect_size > 0 && written != expect_size {
        let _ = std::fs::remove_file(&dest);
        return Err(format!(
            "下载不完整：收到 {written} 字节，服务端标注 {expect_size} 字节"
        ));
    }
    if !looks_like_exe(&dest) {
        let _ = std::fs::remove_file(&dest);
        return Err(format!("下载内容不是可执行文件（{written} 字节），已丢弃"));
    }

    // 最后一道校验：新包自己的版本号必须等于目标版本。
    // 截断的 exe 读不出完整版本资源，放错的文件版本号也对不上 ——
    // 这两种情况如果放过去，下一步就是拿它覆盖自己，程序就再也起不来了。
    match version::read_exe_version(&dest) {
        Ok(found) if found == latest => {}
        Ok(found) => {
            let _ = std::fs::remove_file(&dest);
            return Err(format!("下载到的文件版本是 v{found}，与目标 v{latest} 不符，已丢弃"));
        }
        Err(err) => {
            let _ = std::fs::remove_file(&dest);
            return Err(format!("下载到的文件无法读取版本号（可能不完整）：{err}"));
        }
    }

    Ok(StagedUpdate {
        version: latest.to_string(),
        path: dest,
        size: written,
    })
}

/// PE 文件头是 `MZ`。比"大小看起来差不多"可靠得多 ——
/// 错误页、代理提示页、半截文件都会被这一条挡下来。
fn looks_like_exe(path: &Path) -> bool {
    let Ok(mut f) = File::open(path) else {
        return false;
    };
    let mut head = [0u8; 2];
    f.read_exact(&mut head).is_ok() && &head == b"MZ"
}

/// 用已下载的新版本替换当前 exe，并启动新进程。
///
/// 调用方在返回 `Ok` 之后应当立即退出，让新进程接管。
///
/// 原理：Windows **不允许覆盖正在运行的 exe，但允许给它改名**。
/// 所以顺序是「自己改名 → 新文件就位 → 启动新的 → 老的退出」。
/// 第 2 步失败必须把名字改回去，否则当前目录里就一个能跑的 exe 都没有了。
pub fn install_and_restart(staged: &StagedUpdate) -> Result<(), String> {
    let current = std::env::current_exe().map_err(|e| format!("取当前程序路径失败：{e}"))?;
    let old = current.with_extension("old.exe");

    // 上次更新可能留下 .old，先清掉（删不掉也无所谓，不影响这次替换）
    let _ = std::fs::remove_file(&old);

    std::fs::rename(&current, &old)
        .map_err(|e| format!("无法重命名当前程序（可能没有写权限）：{e}"))?;

    if let Err(err) = std::fs::copy(&staged.path, &current) {
        // 回滚：把名字改回来，至少保证程序还能用
        let _ = std::fs::rename(&old, &current);
        return Err(format!("写入新版本失败，已回滚到原版本：{err}"));
    }

    // 新文件已经就位，临时文件可以删了
    let _ = std::fs::remove_file(&staged.path);

    // 启动新进程。即使这一步失败，磁盘上已经是新版本了，用户手动打开即可
    if let Err(err) = std::process::Command::new(&current).spawn() {
        return Err(format!(
            "新版本已就位，但自动启动失败（{err}）。请手动打开：{}",
            current.display()
        ));
    }

    Ok(())
}

/// 新版本启动时清理上一版留下的 `.old` 文件。
///
/// 刚启动时旧进程可能还没完全退出，删不掉是正常的 —— 下次启动再删。
pub fn cleanup_old_version() {
    let Ok(current) = std::env::current_exe() else {
        return;
    };
    let old = current.with_extension("old.exe");
    if !old.exists() {
        return;
    }
    match std::fs::remove_file(&old) {
        Ok(()) => crate::tlog!("已清理上一版残留 {}", old.display()),
        // 旧进程可能还没完全退出，删不掉很正常，下次启动再试
        Err(err) => crate::tlog!("清理 {} 失败：{err}", old.display()),
    }
}

/// 从扁平的 JSON 对象里取一个字段。
///
/// 这不是通用 JSON 解析器，只够解析我们自己服务端产出的固定结构（扁平对象）。
/// 为了这几十行把 serde 拉进客户端不划算 —— 单 exe 的体积和依赖树是要守住的东西。
/// 键名要求前面是 `{` 或 `,`，免得匹配到更新说明里出现的同名字样。
fn json_field(body: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut from = 0;
    while let Some(pos) = body[from..].find(&needle) {
        let at = from + pos;
        let before = body[..at].trim_end().chars().last();
        if matches!(before, None | Some('{') | Some(',')) {
            let rest = body[at + needle.len()..].trim_start();
            let rest = rest.strip_prefix(':')?.trim_start();
            return Some(parse_value(rest));
        }
        from = at + needle.len();
    }
    None
}

fn parse_value(s: &str) -> String {
    let Some(rest) = s.strip_prefix('"') else {
        // 数字 / 布尔：读到分隔符为止
        let end = rest_end(s);
        return s[..end].trim().to_string();
    };

    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Ok(code) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(code) {
                            out.push(ch);
                        }
                    }
                }
                Some(other) => out.push(other),
                None => break,
            },
            _ => out.push(c),
        }
    }
    out
}

fn rest_end(s: &str) -> usize {
    s.find([',', '}']).unwrap_or(s.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"update":true,"latest":"1.0.2","notes":"修了\"引号\"与\n换行","force":true,"url":"/api/download/1.0.2","size":458752}"#;

    #[test]
    fn extracts_plain_and_escaped_fields() {
        assert_eq!(json_field(SAMPLE, "latest").as_deref(), Some("1.0.2"));
        assert_eq!(json_field(SAMPLE, "notes").as_deref(), Some("修了\"引号\"与\n换行"));
        assert_eq!(json_field(SAMPLE, "force").as_deref(), Some("true"));
        assert_eq!(json_field(SAMPLE, "size").as_deref(), Some("458752"));
        assert_eq!(json_field(SAMPLE, "url").as_deref(), Some("/api/download/1.0.2"));
        assert_eq!(json_field(SAMPLE, "nope"), None);
    }

    /// 空仓库时服务端返回的哨兵响应，不能被解析成"有新版本"
    #[test]
    fn empty_repo_response_means_no_update() {
        let body = r#"{"update":false,"latest":"","notes":"","force":false,"url":"","size":0}"#;
        let latest = json_field(body, "latest").unwrap_or_default();
        assert!(latest.is_empty());
        assert!(!version::is_newer(&latest, version::APP_VERSION));
    }

    /// 更新说明里出现 "latest" 字样时，不能把说明里的值当成字段值
    #[test]
    fn does_not_match_key_names_inside_notes() {
        let body = r#"{"latest":"1.0.2","notes":"字段 latest 现在有值了"}"#;
        assert_eq!(json_field(body, "latest").as_deref(), Some("1.0.2"));
        assert_eq!(json_field(body, "notes").as_deref(), Some("字段 latest 现在有值了"));
    }

    /// 历史版本列表要按服务端给的顺序解析出来（服务端已排好：最新版在最前），
    /// 并且说明里的 `[` `]` `{` `}` 引号不能打断解析。
    #[test]
    fn parses_release_history_in_server_order() {
        let body = r#"{"update":true,"latest":"1.0.6","notes":"顶部","releases":[
            {"version":"1.0.6","notes":"修了 [括号] {花括号} 和 \"引号\"","published_at":"2026-11-02 09:00","size":633344,"latest":true},
            {"version":"1.0.5","notes":"新增 7 个密码\n第二行","published_at":"2026-10-26 10:37","size":633344,"latest":false,"force":true}
        ]}"#;
        let list = parse_releases(body);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].version, "1.0.6");
        assert!(list[0].is_latest);
        assert_eq!(list[0].notes, "修了 [括号] {花括号} 和 \"引号\"");
        assert_eq!(list[1].version, "1.0.5");
        assert!(!list[1].is_latest);
        assert!(list[1].force);
        assert_eq!(list[1].notes, "新增 7 个密码\n第二行");
    }

    /// 老服务端不返回 releases 字段时，不能报错也不能凭空造数据
    #[test]
    fn missing_releases_field_yields_empty_list() {
        let body = r#"{"update":false,"latest":"1.0.1","notes":"x"}"#;
        assert!(parse_releases(body).is_empty());
    }

    /// 没有历史列表时，对话框正文要退回「最新版 + 说明」，别弹空框
    #[test]
    fn history_text_falls_back_to_latest_notes() {
        let mut outcome = UpdateOutcome::unknown();
        outcome.latest = "1.0.6".into();
        outcome.notes = "修了叠字".into();
        let text = outcome.history_text();
        assert!(text.contains("v1.0.6"));
        assert!(text.contains("修了叠字"));
        assert!(text.contains("[最新]"));
    }

    /// 历史列表存在时，正文顺序必须和服务端一致（最新在最上）
    #[test]
    fn history_text_keeps_latest_on_top() {
        let mut outcome = UpdateOutcome::unknown();
        outcome.latest = "1.0.6".into();
        outcome.releases = vec![
            ReleaseInfo {
                version: "1.0.6".into(),
                notes: "新的".into(),
                published_at: "2026-11-02 09:00".into(),
                force: false,
                is_latest: true,
            },
            ReleaseInfo {
                version: "1.0.5".into(),
                notes: "旧的".into(),
                published_at: "2026-10-26 10:37".into(),
                force: false,
                is_latest: false,
            },
        ];
        let text = outcome.history_text();
        let new_at = text.find("v1.0.6").unwrap();
        let old_at = text.find("v1.0.5").unwrap();
        assert!(new_at < old_at, "最新版必须排在最前面");
        // EDIT 控件换行要 CRLF
        assert!(text.contains("\r\n"));
    }
}
