//! 配置持久化：等价于原版的 `QSettings`。
//!
//! 键位置与原版完全一致，因此从 C++ 版切过来配置可以直接复用：
//!   配置：HKCU\Software\Qtech\autoLoginQatarApp
//!   自启：HKCU\Software\Microsoft\Windows\CurrentVersion\Run
//!
//! 1.0.5 新增密码池（7 个密码 + 轮换状态）。这一项**不能明文存** ——
//! 原版只存一个密码是历史包袱，7 个密码再裸放注册表就太过了。
//! 用 DPAPI（`CryptProtectData`，用户级密钥，不落盘）加密成 REG_BINARY：
//! 只有当前 Windows 用户能解开，换账户或拷到别的机器上都是废数据。

use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{LocalFree, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HLOCAL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_ALL_ACCESS, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_BINARY, REG_DWORD,
    REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE,
};

use crate::pwd::Pool;
use crate::util::{from_wide, to_wide};

const APP_KEY: &str = r"Software\Qtech\autoLoginQatarApp";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "autoLoginQatarApp";
/// 密码池的注册表值名
const POOL_VALUE: &str = "pwdPool";

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub account: String,
    pub password: String,
    pub auto_start: bool,
    /// 7 个密码的轮换池。老版本升级上来时是 `Pool::default()`（未配置）。
    pub pool: Pool,
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
            // 解不开 / 格式不对都退回空池：宁可判成「没配置过」不自动重置，
            // 也不能拿半截数据去改人家的密码。
            s.pool = read_binary(hkey, POOL_VALUE)
                .and_then(|enc| dpapi_unprotect(&enc))
                .and_then(|plain| Pool::from_text(&String::from_utf8_lossy(&plain)))
                .unwrap_or_default();
            crate::tlog!(
                "配置加载: account={:?} password_len={} auto_start={} pool_ready={} pool_auto={} next_days={}",
                s.account,
                s.password.chars().count(),
                s.auto_start,
                s.pool.is_configured(),
                s.pool.auto,
                s.pool.days_until_due(crate::pwd::now_unix())
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

            match dpapi_protect(self.pool.to_text().as_bytes()) {
                Some(enc) => write_binary(hkey, POOL_VALUE, &enc),
                // 加密失败还硬写明文等于白加密；宁可这次不存，也不能把 7 个密码摊在注册表里
                None => crate::tlog!("密码池加密失败，本次未写入"),
            }

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

unsafe fn read_binary(hkey: HKEY, name: &str) -> Option<Vec<u8>> {
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
    if rc != ERROR_SUCCESS || size == 0 || ty != REG_BINARY {
        return None;
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
        return None;
    }
    buf.truncate(size as usize);
    Some(buf)
}

unsafe fn write_binary(hkey: HKEY, name: &str, data: &[u8]) {
    let value_name = to_wide(name);
    let _ = RegSetValueExW(
        hkey,
        PCWSTR(value_name.as_ptr()),
        None,
        REG_BINARY,
        Some(data),
    );
}

// ---------------------------------------------------------------------------
// DPAPI
// ---------------------------------------------------------------------------

/// 用当前用户的 DPAPI 密钥加密。密钥由系统托管，不在磁盘上留副本，
/// 所以这段密文换个用户账户 / 换台机器都解不开。
///
/// `CRYPTPROTECT_UI_FORBIDDEN` 是必须的：万一密钥环出问题，系统默认会弹一个
/// 密码输入框，而这个调用可能发生在后台线程里，弹框会挂在那里没人点。
fn dpapi_protect(plain: &[u8]) -> Option<Vec<u8>> {
    if plain.is_empty() {
        return None;
    }
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: plain.len() as u32,
            pbData: plain.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        CryptProtectData(
            &input,
            PCWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .ok()?;

        let result = if output.pbData.is_null() {
            None
        } else {
            Some(std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec())
        };
        if !output.pbData.is_null() {
            let _ = LocalFree(Some(HLOCAL(output.pbData as *mut c_void)));
        }
        result
    }
}

fn dpapi_unprotect(cipher: &[u8]) -> Option<Vec<u8>> {
    if cipher.is_empty() {
        return None;
    }
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: cipher.len() as u32,
            pbData: cipher.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .ok()?;

        let result = if output.pbData.is_null() {
            None
        } else {
            Some(std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec())
        };
        if !output.pbData.is_null() {
            let _ = LocalFree(Some(HLOCAL(output.pbData as *mut c_void)));
        }
        result
    }
}
