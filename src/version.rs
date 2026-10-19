//! 版本标识与新旧版本状态。
//!
//! **版本号只有一个来源：`Cargo.toml` 的 `version` 字段。** 构建时 cargo 会把它
//! 注入成环境变量 `CARGO_PKG_VERSION`，代码里用 `env!` 取，`app.rc` 的 exe 版本
//! 资源也由 `build.rs` 从同一个地方注入 —— 所以升版本只改 `Cargo.toml` 一处，
//! 界面徽章、exe 属性、README 不会再各说各话。
//!
//! 自动更新的服务端还没实现，当前只做「显示 + 颜色区分」：
//! 徽章底色绿 = 已是最新，黄 = 服务端上有更新的版本。等接上服务端后，
//! 只需把 [`detect_state`] 换成「拿服务端返回的版本号和 [`APP_VERSION`] 比大小」。

/// 当前程序版本，如 `1.0.1`（不含 `v` 前缀）
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 标题栏徽章上显示的文字，如 `v1.0.1`。
///
/// `concat!` + `env!` 都是编译期展开，所以这是常量、不产生运行时分配。
pub const BADGE_TEXT: &str = concat!("v", env!("CARGO_PKG_VERSION"));

/// 当前版本相对服务端的状态
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum VersionState {
    /// 已是最新（或尚未检查）—— 徽章绿色
    Latest,
    /// 服务端上有更新的版本 —— 徽章黄色
    Outdated,
}

impl VersionState {
    /// 徽章底色与文字色。绿色沿用「运行中」、黄色沿用「登录中」的配色，
    /// 让标题栏和状态卡片看起来是一套东西。
    pub fn badge_colors(self) -> (u32, u32) {
        match self {
            VersionState::Latest => (crate::theme::BADGE_VER_LATEST_BG, crate::theme::BADGE_VER_LATEST_FG),
            VersionState::Outdated => (crate::theme::BADGE_VER_OLD_BG, crate::theme::BADGE_VER_OLD_FG),
        }
    }
}

/// 判定当前版本状态。
///
/// 自动更新服务端还没接，所以默认一律「最新」；只有自测用的环境变量
/// `AUTOLOGIN_FORCE_UPDATE=1` 会强制显示成「有更新」，用来核对黄色徽章的外观。
/// 接上服务端后，这里换成真实比对即可，界面侧无需改动。
pub fn detect_state() -> VersionState {
    match std::env::var("AUTOLOGIN_FORCE_UPDATE").as_deref() {
        Ok("1") | Ok("true") => VersionState::Outdated,
        _ => VersionState::Latest,
    }
}

/// 从 PE 文件的 `VERSIONINFO` 资源里读版本号，形如 `1.0.1`。
///
/// 用于下载完更新包之后的校验：**截断的 exe 读不出完整版本资源**，
/// 放错的文件、代理塞进来的错误页同理 —— 这是替换自己之前的最后一道闸。
///
/// 走 `VS_FIXEDFILEINFO`（语言无关）而不是 `StringFileInfo`：后者要拼
/// `\StringFileInfo\<语言块>\FileVersion`，语言块 ID 会随系统和编译器变。
pub fn read_exe_version(path: &std::path::Path) -> Result<String, String> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };

    let wide = crate::util::to_wide(&path.to_string_lossy());
    unsafe {
        let mut handle: u32 = 0;
        let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), Some(&mut handle));
        if size == 0 {
            return Err("文件没有版本信息资源".into());
        }

        let mut buf = vec![0u8; size as usize];
        GetFileVersionInfoW(
            PCWSTR(wide.as_ptr()),
            None,
            size,
            buf.as_mut_ptr() as *mut std::ffi::c_void,
        )
        .map_err(|e| format!("读取版本资源失败：{}", e.message()))?;

        let sub = crate::util::to_wide("\\");
        let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut len: u32 = 0;
        let ok = VerQueryValueW(
            buf.as_ptr() as *const std::ffi::c_void,
            PCWSTR(sub.as_ptr()),
            &mut ptr,
            &mut len,
        )
        .as_bool();
        if !ok || ptr.is_null() || (len as usize) < std::mem::size_of::<u32>() * 4 {
            return Err("版本资源格式异常".into());
        }

        // VS_FIXEDFILEINFO 前两个 DWORD 是 dwSignature / dwStrucVersion，
        // 版本号从第 3 个才开始 —— 拿前两个当版本号会读出 65263.1213.1。
        let raw = std::slice::from_raw_parts(ptr as *const u32, 4);
        if raw[0] != 0xFEEF_04BD {
            return Err("版本资源签名不符".into());
        }
        let (ms, ls) = (raw[2], raw[3]);
        let (major, minor) = (ms >> 16, ms & 0xFFFF);
        let (build, revision) = (ls >> 16, ls & 0xFFFF);
        Ok(if revision == 0 {
            format!("{major}.{minor}.{build}")
        } else {
            format!("{major}.{minor}.{build}.{revision}")
        })
    }
}

/// 版本号比较：`a` 是否比 `b` 新。
///
/// 按点分段逐段比数值（`1.10.0 > 1.9.0`，字符串直接比会得出相反结果）。
/// 服务端返回的版本号带不带 `v` 前缀都能处理，段数不足的按 0 补齐。
/// 现在还没有调用方，等接上更新接口时用。
#[allow(dead_code)]
pub fn is_newer(a: &str, b: &str) -> bool {
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
    x > y
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 版本号要按段比数值，按字符串比会得出 1.9.0 > 1.10.0 的错误结论
    #[test]
    fn compare_by_segments_not_lexicographic() {
        assert!(is_newer("1.10.0", "1.9.0"));
        assert!(is_newer("v1.0.2", "1.0.1"));
        assert!(is_newer("1.1", "1.0.9"));
        assert!(!is_newer("1.0.1", "1.0.1"));
        assert!(!is_newer("0.9.9", "1.0.0"));
    }
}
