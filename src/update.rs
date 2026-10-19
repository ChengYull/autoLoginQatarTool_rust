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
}

impl UpdateOutcome {
    fn unknown() -> Self {
        Self {
            state: VersionState::Latest,
            latest: String::new(),
            force: false,
            notes: String::new(),
            size: 0,
        }
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
    }
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
}
