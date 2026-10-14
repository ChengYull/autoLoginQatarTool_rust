//! 重置密码接口。
//!
//! 原版用 QNetworkAccessManager 发 GET：`?uid=工号`，成功时响应体只有一个字符 `1`，
//! 重置后的新密码由服务端发送到钉钉。这里用系统自带的 WinHTTP，零第三方依赖。

use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, WINHTTP_OPEN_REQUEST_FLAGS,
};

use crate::util::to_wide;

const DEFAULT_HOST: &str = "10.170.1.137";
const DEFAULT_PORT: u16 = 18081;
const PATH: &str = "/api/user/forgetpwd";
const TIMEOUT_MS: i32 = 10_000;

/// 接口地址。允许用环境变量覆盖，便于自测与灰度（默认走生产地址）。
fn endpoint() -> (String, u16) {
    let host = std::env::var("AUTOLOGIN_RESET_HOST").unwrap_or_else(|_| DEFAULT_HOST.to_string());
    let port = std::env::var("AUTOLOGIN_RESET_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);
    (host, port)
}

/// 结构化的请求结果
#[derive(Debug)]
pub enum ResetResult {
    /// 接口返回体（已 trim）
    Body(String),
    /// 请求失败
    Failed(String),
}

/// 调用重置密码接口。阻塞调用，需在后台线程里执行。
pub fn reset_password(uid: &str) -> ResetResult {
    let query = format!("{}?uid={}", PATH, percent_encode(uid));
    match http_get(&query) {
        Ok(body) => ResetResult::Body(body.trim().to_string()),
        Err(err) => ResetResult::Failed(err),
    }
}

fn http_get(path_and_query: &str) -> Result<String, String> {
    let (host_name, port) = endpoint();
    unsafe {
        let agent = to_wide("AutoLoginQatarTool");
        let session = WinHttpOpen(
            PCWSTR(agent.as_ptr()),
            WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        );
        if session.is_null() {
            return Err("无法初始化网络会话".into());
        }
        let session_guard = HandleGuard(session);

        let _ = WinHttpSetTimeouts(session, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS);

        let host = to_wide(&host_name);
        let connect = WinHttpConnect(session, PCWSTR(host.as_ptr()), port, 0);
        if connect.is_null() {
            drop(session_guard);
            return Err("无法连接到服务器".into());
        }
        let connect_guard = HandleGuard(connect);

        let verb = to_wide("GET");
        let object = to_wide(path_and_query);
        let request = WinHttpOpenRequest(
            connect,
            PCWSTR(verb.as_ptr()),
            PCWSTR(object.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_OPEN_REQUEST_FLAGS(0), // 明文 HTTP，不需要 WINHTTP_FLAG_SECURE
        );
        if request.is_null() {
            drop(connect_guard);
            drop(session_guard);
            return Err("无法创建请求".into());
        }
        let request_guard = HandleGuard(request);

        if let Err(err) = WinHttpSendRequest(request, None, None, 0, 0, 0) {
            drop(request_guard);
            drop(connect_guard);
            drop(session_guard);
            return Err(format!("发送请求失败：{}", err.message()));
        }
        if let Err(err) = WinHttpReceiveResponse(request, std::ptr::null_mut()) {
            drop(request_guard);
            drop(connect_guard);
            drop(session_guard);
            return Err(format!("接收响应失败：{}", err.message()));
        }

        let mut body: Vec<u8> = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let mut available: u32 = 0;
            if WinHttpQueryDataAvailable(request, &mut available).is_err() {
                break;
            }
            if available == 0 {
                break;
            }
            let to_read = available.min(buffer.len() as u32);
            let mut read: u32 = 0;
            if WinHttpReadData(
                request,
                buffer.as_mut_ptr() as *mut c_void,
                to_read,
                &mut read,
            )
            .is_err()
            {
                break;
            }
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buffer[..read as usize]);
        }

        Ok(String::from_utf8_lossy(&body).into_owned())
    }
}

/// 保证句柄在任意返回路径上都被释放
struct HandleGuard(*mut c_void);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

/// QUrlQuery 会对参数做百分号编码，这里保持一致
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}
