//! 更新检查：向更新服务端问一次最新版本，决定标题栏版本徽章的颜色。
//!
//! 服务端是同级 `server/` 目录里的 `AutoLoginQatarServer`，接口见 `server/src/http.rs`。
//! 客户端只做「查 + 显示」，不自动下载替换 —— 那是下一步的事。

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
}

impl UpdateOutcome {
    fn unknown() -> Self {
        Self {
            state: VersionState::Latest,
            latest: String::new(),
            force: false,
            notes: String::new(),
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
