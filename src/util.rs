//! 基础工具：矩形、颜色、宽字符串。


#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
    pub const fn right(&self) -> i32 {
        self.x + self.w
    }
    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
    pub fn cx(&self) -> i32 {
        self.x + self.w / 2
    }
    pub fn cy(&self) -> i32 {
        self.y + self.h / 2
    }
    pub fn to_win(&self) -> windows::Win32::Foundation::RECT {
        windows::Win32::Foundation::RECT {
            left: self.x,
            top: self.y,
            right: self.right(),
            bottom: self.bottom(),
        }
    }
}

// ---------------------------------------------------------------------------
// 颜色
//
// 统一用 0xRRGGBB 表示不透明色，需要透明度时用 rgba() 得到 0xAARRGGBB。
// ---------------------------------------------------------------------------

/// GDI+ 用 0xAARRGGBB
pub const fn rgb(hex: u32) -> u32 {
    0xFF00_0000 | (hex & 0x00FF_FFFF)
}

/// GDI+ 用 0xAARRGGBB
pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// GDI 用 COLORREF（0x00BBGGRR）
pub const fn colorref(hex: u32) -> u32 {
    let r = (hex >> 16) & 0xFF;
    let g = (hex >> 8) & 0xFF;
    let b = hex & 0xFF;
    (b << 16) | (g << 8) | r
}

// ---------------------------------------------------------------------------
// 宽字符串
// ---------------------------------------------------------------------------

/// 转成带结尾 0 的 UTF-16 缓冲
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 从 UTF-16 缓冲读取到第一个 0 为止
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}


/// 不区分大小写的子串查找（等价于 C++ 里的 containsIgnoreCase）
pub fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    haystack
        .to_lowercase()
        .contains(&needle.to_lowercase())
}

// ---------------------------------------------------------------------------
// 剪贴板
// ---------------------------------------------------------------------------

/// 把文本写进剪贴板（1.0.5 的「复制当前密码」）。
///
/// 返回值只用于日志：剪贴板被别的进程占着时会失败，没必要弹框打扰用户。
pub fn copy_to_clipboard(text: &str) -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let _ = EmptyClipboard();

        let wide = to_wide(text);
        let bytes = wide.len() * std::mem::size_of::<u16>();
        let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, bytes) else {
            let _ = CloseClipboard();
            return false;
        };
        let ptr = GlobalLock(mem) as *mut u16;
        if ptr.is_null() {
            let _ = CloseClipboard();
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
        let _ = GlobalUnlock(mem);

        // 13 = CF_UNICODETEXT。所有权在 SetClipboardData 成功后归系统，
        // 这里不能再 GlobalFree，否则会把系统手里的内存释放掉。
        let ok = SetClipboardData(13, Some(HANDLE(mem.0))).is_ok();
        let _ = CloseClipboard();
        ok
    }
}
