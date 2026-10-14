//! 配置持久化：等价于原版的 `QSettings`。
//!
//! 键位置与原版完全一致，因此从 C++ 版切过来配置可以直接复用：
//!   配置：HKCU\Software\Qtech\autoLoginQatarApp
//!   自启：HKCU\Software\Microsoft\Windows\CurrentVersion\Run

use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_ALL_ACCESS, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_DWORD,
    REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE,
};

use crate::util::{from_wide, to_wide};

const APP_KEY: &str = r"Software\Qtech\autoLoginQatarApp";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "autoLoginQatarApp";

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub account: String,
    pub password: String,
    pub auto_start: bool,
}

impl Settings {
    pub fn load() -> Self {
        let mut s = Settings::default();
        unsafe {
            let key = APP_KEY;
            let sub = to_wide(key);
            let mut hkey = HKEY::default();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(sub.as_ptr()),
                None,
                KEY_QUERY_VALUE,
                &mut hkey,
            ) != ERROR_SUCCESS
            {
                return s;
            }

            s.account = read_string(hkey, "account");
            s.password = read_string(hkey, "password");
            s.auto_start = read_dword(hkey, "autoStart").unwrap_or(0) != 0;
            crate::tlog!(
                "配置加载: account={:?} password_len={} auto_start={}",
                s.account,
                s.password.chars().count(),
                s.auto_start
            );

            let _ = RegCloseKey(hkey);
        }
        s
    }

    pub fn save(&self) {
        unsafe {
            let sub = to_wide(APP_KEY);
            let mut hkey = HKEY::default();
            if RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(sub.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_ALL_ACCESS,
                None,
                &mut hkey,
                None,
            ) != ERROR_SUCCESS
            {
                return;
            }

            write_string(hkey, "account", &self.account);
            write_string(hkey, "password", &self.password);
            write_dword(hkey, "autoStart", self.auto_start as u32);

            let _ = RegCloseKey(hkey);
        }
    }
}

/// 写入 / 删除开机自启项
pub fn set_auto_start(enabled: bool, exe_path: &str) {
    unsafe {
        let sub = to_wide(RUN_KEY);
        let name = to_wide(RUN_VALUE);
        if enabled {
            let mut hkey = HKEY::default();
            if RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(sub.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut hkey,
                None,
            ) != ERROR_SUCCESS
            {
                return;
            }
            // 与原版一致：写入 exe 全路径，分隔符统一为反斜杠
            let path = exe_path.replace('/', "\\");
            write_string(hkey, RUN_VALUE, &path);
            let _ = RegCloseKey(hkey);
        } else {
            let mut hkey = HKEY::default();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(sub.as_ptr()),
                None,
                KEY_SET_VALUE,
                &mut hkey,
            ) == ERROR_SUCCESS
            {
                let _ = RegDeleteValueW(hkey, PCWSTR(name.as_ptr()));
                let _ = RegCloseKey(hkey);
            }
        }
    }
}

unsafe fn read_string(hkey: HKEY, name: &str) -> String {
    let value_name = to_wide(name);
    let mut ty = REG_VALUE_TYPE::default();
    let mut size: u32 = 0;
    let rc = RegQueryValueExW(
        hkey,
        PCWSTR(value_name.as_ptr()),
        None,
        Some(&mut ty),
        None,
        Some(&mut size),
    );
    if rc != ERROR_SUCCESS || size == 0 {
        return String::new();
    }

    let mut buf = vec![0u8; size as usize];
    let rc = RegQueryValueExW(
        hkey,
        PCWSTR(value_name.as_ptr()),
        None,
        Some(&mut ty),
        Some(buf.as_mut_ptr()),
        Some(&mut size),
    );
    if rc != ERROR_SUCCESS {
        return String::new();
    }

    match ty {
        REG_SZ => {
            let wide: Vec<u16> = buf
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            from_wide(&wide)
        }
        // 兼容早期把字符串写成其它类型的情况
        _ => String::new(),
    }
}

unsafe fn read_dword(hkey: HKEY, name: &str) -> Option<u32> {
    let value_name = to_wide(name);
    let mut ty = REG_VALUE_TYPE::default();
    let mut data: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let rc = RegQueryValueExW(
        hkey,
        PCWSTR(value_name.as_ptr()),
        None,
        Some(&mut ty),
        Some(&mut data as *mut u32 as *mut u8),
        Some(&mut size),
    );
    if rc == ERROR_SUCCESS {
        return Some(data);
    }
    // 有的写入方会把 bool 存成字符串 "true"/"false"
    if rc == ERROR_FILE_NOT_FOUND {
        return None;
    }
    let text = read_string(hkey, name);
    match text.trim().to_ascii_lowercase().as_str() {
        "true" | "1" => Some(1),
        "false" | "0" | "" => Some(0),
        _ => None,
    }
}

unsafe fn write_string(hkey: HKEY, name: &str, value: &str) {
    let value_name = to_wide(name);
    let data = to_wide(value);
    let bytes: &[u8] =
        std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * std::mem::size_of::<u16>());
    let _ = RegSetValueExW(hkey, PCWSTR(value_name.as_ptr()), None, REG_SZ, Some(bytes));
}

unsafe fn write_dword(hkey: HKEY, name: &str, value: u32) {
    let value_name = to_wide(name);
    let bytes = value.to_le_bytes();
    let _ = RegSetValueExW(
        hkey,
        PCWSTR(value_name.as_ptr()),
        None,
        REG_DWORD,
        Some(&bytes),
    );
}
