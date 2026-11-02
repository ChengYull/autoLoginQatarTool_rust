//! 服务端自身的小配置（监听地址、发布令牌）。
//!
//! 和版本仓库分开放：`releases.json` 是业务数据（会随发布不断增长），
//! 这里是程序设置。混在一起以后备份/迁移都不方便。

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const CONFIG_FILE: &str = "server.json";

/// 默认监听全部网卡：客户端在别的机器上，只听 127.0.0.1 就没人能连上了。
pub const DEFAULT_BIND: &str = "0.0.0.0:18082";

fn default_bind() -> String {
    DEFAULT_BIND.to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    #[serde(default = "default_bind")]
    pub bind: String,
    /// 远程发布接口的令牌（`POST /api/publish` 的 `X-Publish-Token` 头）。
    ///
    /// 为什么要有它：发布接口能往仓库里塞任意 exe，而客户端拿到更新后会**下载并
    /// 直接替换自己**。接口裸着就等于内网里任何人都能给别人机器投毒。
    /// 令牌在第一次启动时随机生成、落进 `server.json`，不需要用户自己想密码。
    /// 空串表示「还没生成过」，[`Config::load_or_init`] 会补一个。
    #[serde(default)]
    pub publish_token: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            publish_token: String::new(),
        }
    }
}

impl Config {
    /// 读配置。文件不存在或内容坏了都退回默认值 —— 配置坏了不该让程序起不来。
    pub fn load(dir: &Path) -> Self {
        fs::read_to_string(dir.join(CONFIG_FILE))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// 读配置，缺发布令牌就生成一个并落盘。
    ///
    /// 单独一个方法是因为 `load` 必须是纯读取（`--publish` 之类的一次性命令
    /// 不该顺手改配置），而服务端启动时必须保证令牌存在。
    pub fn load_or_init(dir: &Path) -> Self {
        let mut cfg = Self::load(dir);
        if cfg.publish_token.trim().is_empty() {
            cfg.publish_token = new_token();
            if let Err(err) = cfg.save(dir) {
                crate::tlog!("生成发布令牌后写配置失败：{err}");
            }
            crate::tlog!("已生成发布令牌（见 {CONFIG_FILE} 的 publish_token）");
        }
        cfg
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败：{e}"))?;
        let text = serde_json::to_string_pretty(self).map_err(|e| format!("序列化失败：{e}"))?;
        fs::write(dir.join(CONFIG_FILE), text).map_err(|e| format!("写入 {CONFIG_FILE} 失败：{e}"))
    }
}

/// 32 位十六进制的随机令牌。
///
/// 用 `RandomState` 而不是自己造种子：它背后是系统 CSPRNG（Windows 上是
/// RtlGenRandom），拿来生成令牌足够了，也不值得为此引一个 rand 依赖。
fn new_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let mut out = String::with_capacity(32);
    while out.len() < 32 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(std::process::id() as u64);
        h.write_u128(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0));
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out.truncate(32);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_32_hex_chars_and_unique() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b, "两次生成的令牌不应该一样");
    }

    #[test]
    fn config_without_token_loads_with_empty_one() {
        // 老版本写出来的 server.json 里没有 publish_token 字段，
        // 反序列化不能失败，否则升级后服务端起不来。
        let cfg: Config = serde_json::from_str(r#"{"bind":"0.0.0.0:18082"}"#).unwrap();
        assert_eq!(cfg.bind, DEFAULT_BIND);
        assert!(cfg.publish_token.is_empty());
    }
}

