//! 服务端自身的小配置（监听地址）。
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
}

impl Default for Config {
    fn default() -> Self {
        Self { bind: default_bind() }
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

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败：{e}"))?;
        let text = serde_json::to_string_pretty(self).map_err(|e| format!("序列化失败：{e}"))?;
        fs::write(dir.join(CONFIG_FILE), text).map_err(|e| format!("写入 {CONFIG_FILE} 失败：{e}"))
    }
}
