//! 版本仓库：元数据持久化 + 发布文件管理。
//!
//! 磁盘布局（数据目录默认是 exe 同级的 `data/`，可用 `--data` 覆盖）：
//!
//! ```text
//! data/
//! ├── releases.json                     版本元数据（serde_json）
//! └── files/
//!     └── AutoLoginQatarTool-1.0.1.exe  各版本产物，客户端从这里下载
//! ```
//!
//! **版本号不靠人工输入**：发布时直接从 exe 的 `VERSIONINFO` 资源里读。
//! 客户端的版本号是 `Cargo.toml` 编译期注入的，服务端再手输一遍必然有一天会打错，
//! 而且打错之后「服务端说最新是 1.0.3、客户端实际是 1.0.2」这种问题很难查。

use std::ffi::c_void;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::core::PCWSTR;
use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::util::to_wide;

pub const REPO_FILE: &str = "releases.json";
pub const FILES_DIR: &str = "files";

/// 一个已发布的版本
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Release {
    /// 版本号，如 `1.0.1`（不含 `v` 前缀，从 exe 资源里读出来的）
    pub version: String,
    /// 更新说明，会原样返回给客户端
    #[serde(default)]
    pub notes: String,
    /// 是否强制更新（客户端可据此弹不可跳过的提示）
    #[serde(default)]
    pub force: bool,
    /// 在 `files/` 下的文件名
    pub file: String,
    /// 字节数，客户端下载后可据此校验
    pub size: u64,
    /// 发布时间 `YYYY-MM-DD HH:MM:SS`
    pub published_at: String,
    /// 被下载次数
    #[serde(default)]
    pub downloads: u64,
}

/// 整个仓库
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Repo {
    pub releases: Vec<Release>,
    /// 当前对外提供的最新版本号。
    ///
    /// **是显式设置的，不是"取最大的那个"** —— 这样旧版本出问题时可以把最新版回滚到它，
    /// 而不用把出问题的版本从列表里删掉（删掉就没法排查了）。空串表示当前没有对外版本。
    #[serde(default)]
    pub latest: String,
}

impl Repo {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join(REPO_FILE);
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&path).map_err(|e| format!("读取 {REPO_FILE} 失败：{e}"))?;
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(&text).map_err(|e| format!("解析 {REPO_FILE} 失败：{e}"))
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败：{e}"))?;
        let text = serde_json::to_string_pretty(self).map_err(|e| format!("序列化失败：{e}"))?;
        let path = dir.join(REPO_FILE);
        // 先写临时文件再改名：中途断电/崩溃不会把原文件截断成半截 JSON。
        let tmp = dir.join(format!("{REPO_FILE}.tmp"));
        fs::write(&tmp, text).map_err(|e| format!("写入 {REPO_FILE} 失败：{e}"))?;
        fs::rename(&tmp, &path).map_err(|e| format!("替换 {REPO_FILE} 失败：{e}"))
    }

    pub fn find(&self, version: &str) -> Option<&Release> {
        self.releases.iter().find(|r| r.version == version)
    }

    pub fn latest_release(&self) -> Option<&Release> {
        if self.latest.is_empty() {
            None
        } else {
            self.find(&self.latest)
        }
    }

    /// 按版本号从新到旧排（用于界面展示）
    pub fn sorted(&self) -> Vec<&Release> {
        let mut list: Vec<&Release> = self.releases.iter().collect();
        list.sort_by(|a, b| compare_version(&b.version, &a.version));
        list
    }

    /// 对外展示的顺序：**当前对外版本永远排第一**，其余按版本号从新到旧。
    ///
    /// 为什么不单纯按版本号排：`latest` 是显式设置的，出问题时可以把对外版本
    /// 回滚到旧版。回滚之后「版本号最大的那个」并不是用户该升级到的版本，
    /// 把它摆在第一位会让人误以为那才是最新版。
    pub fn display_order(&self) -> Vec<&Release> {
        let mut list = self.sorted();
        if let Some(pos) = list.iter().position(|r| r.version == self.latest) {
            let item = list.remove(pos);
            list.insert(0, item);
        }
        list
    }

    pub fn add(&mut self, release: Release) {
        self.releases.retain(|r| r.version != release.version);
        self.releases.push(release);
    }

    /// 把刚发布的版本并入仓库，比当前对外版本新就自动顶上。
    ///
    /// 界面发布和远程发布（`POST /api/publish`）都走这里，免得两条路各写一份
    /// 「要不要设成最新」的判断 —— 那种重复迟早会分叉。
    /// 返回是否顶上了对外版本。
    pub fn apply_publish(&mut self, release: Release) -> bool {
        let is_newer = self
            .latest_release()
            .map(|cur| compare_version(&release.version, &cur.version).is_gt())
            .unwrap_or(true);
        let promote = is_newer || self.latest.is_empty();
        let version = release.version.clone();
        self.add(release);
        if promote {
            self.latest = version;
        }
        promote
    }

    /// 删除版本，同时删掉磁盘上的产物文件
    pub fn remove(&mut self, dir: &Path, version: &str) -> Result<(), String> {
        let Some(pos) = self.releases.iter().position(|r| r.version == version) else {
            return Err(format!("没有版本 {version}"));
        };
        let release = self.releases.remove(pos);
        let _ = fs::remove_file(dir.join(FILES_DIR).join(&release.file));
        if self.latest == version {
            // 最新版被删了，顺位落到剩下的最高版本；一个都不剩就清空
            self.latest = self
                .sorted()
                .first()
                .map(|r| r.version.clone())
                .unwrap_or_default();
        }
        Ok(())
    }
}

/// 发布一个新版本：把 exe 复制进仓库并登记元数据。
///
/// 版本号取自 exe 自身的 `VERSIONINFO`，所以**发错版本号这件事在流程上就不可能发生**。
pub fn publish(dir: &Path, src: &Path, notes: &str, force: bool) -> Result<Release, String> {
    if !src.is_file() {
        return Err(format!("文件不存在：{}", src.display()));
    }
    let version = read_exe_version(src)?;
    let file_name = format!("AutoLoginQatarTool-{version}.exe");

    let files_dir = dir.join(FILES_DIR);
    fs::create_dir_all(&files_dir).map_err(|e| format!("创建 files 目录失败：{e}"))?;
    let dest = files_dir.join(&file_name);
    fs::copy(src, &dest).map_err(|e| format!("复制文件失败：{e}"))?;
    let size = fs::metadata(&dest).map_err(|e| format!("读取文件大小失败：{e}"))?.len();

    Ok(Release {
        version,
        notes: notes.trim().to_string(),
        force,
        file: file_name,
        size,
        published_at: now_string(),
        downloads: 0,
    })
}

/// 从 PE 文件的 `VERSIONINFO` 资源里读版本号，形如 `1.0.1`。
///
/// 走 `VS_FIXEDFILEINFO`（语言无关）而不是 `StringFileInfo` —— 后者要拼
/// `\StringFileInfo\<语言块>\FileVersion` 这样的路径，语言块 ID 会随系统/编译器变，
/// 在中文机器上编出来的资源块 ID 和英文机器可能不同，硬编码必然踩雷。
pub fn read_exe_version(path: &Path) -> Result<String, String> {
    let wide = to_wide(&path.to_string_lossy());
    unsafe {
        let mut handle: u32 = 0;
        let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), Some(&mut handle));
        if size == 0 {
            return Err("该文件没有版本信息资源（不是本工具编出来的 exe？）".into());
        }

        let mut buf = vec![0u8; size as usize];
        GetFileVersionInfoW(
            PCWSTR(wide.as_ptr()),
            None,
            size,
            buf.as_mut_ptr() as *mut c_void,
        )
        .map_err(|e| format!("读取版本资源失败：{}", e.message()))?;

        // 根节点 "\" 指向 VS_FIXEDFILEINFO
        let sub = to_wide("\\");
        let mut ptr: *mut c_void = std::ptr::null_mut();
        let mut len: u32 = 0;
        let ok = VerQueryValueW(
            buf.as_ptr() as *const c_void,
            PCWSTR(sub.as_ptr()),
            &mut ptr,
            &mut len,
        )
        .as_bool();
        if !ok || ptr.is_null() || (len as usize) < std::mem::size_of::<u32>() * 4 {
            return Err("版本资源格式异常".into());
        }

        // VS_FIXEDFILEINFO 的字段顺序是：
        //   [0] dwSignature      = 0xFEEF04BD
        //   [1] dwStrucVersion
        //   [2] dwFileVersionMS  ← 版本号从第 3 个 DWORD 才开始
        //   [3] dwFileVersionLS
        // 直接拿前两个当版本号会得到 65263.1213.1 这种鬼东西。
        let raw = std::slice::from_raw_parts(ptr as *const u32, 4);
        if raw[0] != 0xFEEF_04BD {
            return Err("版本资源签名不符（不是 VS_FIXEDFILEINFO）".into());
        }
        let (ms, ls) = (raw[2], raw[3]);
        let (major, minor) = (ms >> 16, ms & 0xFFFF);
        let (build, revision) = (ls >> 16, ls & 0xFFFF);

        // 末段为 0 时省略：`1,0,1,0` 显示成 `1.0.1` 而不是 `1.0.1.0`
        Ok(if revision == 0 {
            format!("{major}.{minor}.{build}")
        } else {
            format!("{major}.{minor}.{build}.{revision}")
        })
    }
}

/// 按点分段比数值，用于排序。
///
/// **不能直接比字符串**：`"1.10.0" < "1.9.0"` 在字典序下成立，但那显然不是我们要的顺序。
pub fn compare_version(a: &str, b: &str) -> std::cmp::Ordering {
    fn segs(s: &str) -> Vec<u64> {
        s.trim()
            .trim_start_matches(['v', 'V'])
            .split(['.', '-', '+'])
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect()
    }
    let (mut x, mut y) = (segs(a), segs(b));
    let n = x.len().max(y.len());
    x.resize(n, 0);
    y.resize(n, 0);
    x.cmp(&y)
}

/// 当前本地时间，`YYYY-MM-DD HH:MM:SS`
pub fn now_string() -> String {
    let t: SYSTEMTIME = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

/// 人类可读的字节数
pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let b = bytes as f64;
    if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// 数据目录默认值：exe 同级的 `data/`
pub fn default_data_dir() -> PathBuf {
    crate::util::exe_dir().join("data")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare_is_numeric_not_lexicographic() {
        assert_eq!(compare_version("1.10.0", "1.9.0"), std::cmp::Ordering::Greater);
        assert_eq!(compare_version("1.0.1", "1.0.1"), std::cmp::Ordering::Equal);
        assert_eq!(compare_version("1.1", "1.0.9"), std::cmp::Ordering::Greater);
        assert_eq!(compare_version("v2.0.0", "1.99.99"), std::cmp::Ordering::Greater);
    }

    #[test]
    fn human_size_reads_naturally() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(453_632), "443 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn repo_round_trips_through_json() {
        let dir = std::env::temp_dir().join("alq-store-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut repo = Repo::default();
        repo.add(Release {
            version: "1.0.1".into(),
            notes: "修了「引号\"与\n换行」".into(),
            force: true,
            file: "AutoLoginQatarTool-1.0.1.exe".into(),
            size: 453632,
            published_at: "2026-10-16 10:30:00".into(),
            downloads: 3,
        });
        repo.latest = "1.0.1".into();
        repo.save(&dir).unwrap();

        let back = Repo::load(&dir).unwrap();
        assert_eq!(back.releases.len(), 1);
        // 说明文字里的引号和换行必须原样回来
        assert_eq!(back.releases[0].notes, "修了「引号\"与\n换行」");
        assert!(back.releases[0].force);
        assert_eq!(back.latest_release().unwrap().version, "1.0.1");

        let _ = fs::remove_dir_all(&dir);
    }

    fn sample(version: &str) -> Release {
        Release {
            version: version.into(),
            notes: String::new(),
            force: false,
            file: format!("AutoLoginQatarTool-{version}.exe"),
            size: 1,
            published_at: "2026-10-01 00:00:00".into(),
            downloads: 0,
        }
    }

    /// 对外展示顺序：`latest` 排第一，其余按版本号从新到旧。
    /// 关键是**回滚场景** —— 把 latest 指回旧版后，版本号最大的那个不能顶到最上面，
    /// 否则客户端会把「比当前对外版本还新、但没人该升上去」的版本摆在第一位。
    #[test]
    fn display_order_puts_current_latest_first_even_after_rollback() {
        let mut repo = Repo::default();
        for v in ["1.0.3", "1.0.5", "1.0.4"] {
            repo.add(sample(v));
        }
        repo.latest = "1.0.4".into();
        let order: Vec<&str> = repo
            .display_order()
            .iter()
            .map(|r| r.version.as_str())
            .collect();
        assert_eq!(order, vec!["1.0.4", "1.0.5", "1.0.3"]);
    }

    /// 发布接口和界面发布共用这套判断：只有更新的版本才顶上对外版本
    #[test]
    fn apply_publish_promotes_only_newer_versions() {
        let mut repo = Repo::default();
        // 空仓库：第一个发布的版本必然成为对外版本
        assert!(repo.apply_publish(sample("1.0.5")));
        assert_eq!(repo.latest, "1.0.5");
        // 更旧：只入库
        assert!(!repo.apply_publish(sample("1.0.4")));
        assert_eq!(repo.latest, "1.0.5");
        // 更新：顶上
        assert!(repo.apply_publish(sample("1.0.6")));
        assert_eq!(repo.latest, "1.0.6");
        assert_eq!(repo.releases.len(), 3);
    }
}
