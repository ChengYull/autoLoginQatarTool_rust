//! HTTP 请求。
//!
//! 原版用 QNetworkAccessManager 发 GET：`?uid=工号`，成功时响应体只有一个字符 `1`，
//! 重置后的新密码由服务端发送到钉钉。这里用系统自带的 WinHTTP，零第三方依赖。
//!
//! 除了重置密码，`update.rs` 的更新检查也走这里的 [`http_get_on`] —— 同一个
//! WinHTTP 封装，只是换了 host / port。

use std::ffi::c_void;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetTimeouts, WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, WINHTTP_OPEN_REQUEST_FLAGS,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

use crate::util::to_wide;

const DEFAULT_HOST: &str = "10.170.1.137";
const DEFAULT_PORT: u16 = 18081;
const PATH: &str = "/api/user/forgetpwd";
const TIMEOUT_MS: i32 = 10_000;

/// 重置密码接口地址。允许用环境变量覆盖，便于自测与灰度（默认走生产地址）。
fn reset_endpoint() -> (String, u16) {
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
    let (host, port) = reset_endpoint();
    http_get_on(&host, port, path_and_query)
}

/// 明文 HTTP GET，返回响应体（已按 UTF-8 宽松解码）。
///
/// 同步阻塞调用，调用方负责放到后台线程里跑。
pub fn http_get_on(host_name: &str, port: u16, path_and_query: &str) -> Result<String, String> {
    let mut body: Vec<u8> = Vec::new();
    http_fetch(host_name, port, path_and_query, &mut |chunk| {
        body.extend_from_slice(chunk);
        true
    })?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// 下载到本地文件，返回写入的字节数。
///
/// 边收边写，不把整个 exe 攒在内存里。
pub fn http_download_to(
    host_name: &str,
    port: u16,
    path_and_query: &str,
    dest: &Path,
) -> Result<u64, String> {
    let mut file = File::create(dest).map_err(|e| format!("创建临时文件失败：{e}"))?;
    let mut written: u64 = 0;
    http_fetch(host_name, port, path_and_query, &mut |chunk| {
        if file.write_all(chunk).is_err() {
            return false;
        }
        written += chunk.len() as u64;
        true
    })?;
    let _ = file.flush();
    Ok(written)
}

/// 发一个明文 HTTP GET，把响应体逐块交给 `on_chunk`（返回 false 表示中止）。
fn http_fetch(
    host_name: &str,
    port: u16,
    path_and_query: &str,
    on_chunk: &mut dyn FnMut(&[u8]) -> bool,
) -> Result<(), String> {
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
        // 句柄守卫活到函数结束，任意返回路径上都会释放（drop 顺序与创建顺序相反）
        let _session = HandleGuard(session);

        let _ = WinHttpSetTimeouts(session, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS);

        let host = to_wide(host_name);
        let connect = WinHttpConnect(session, PCWSTR(host.as_ptr()), port, 0);
        if connect.is_null() {
            return Err("无法连接到服务器".into());
        }
        let _connect = HandleGuard(connect);

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
            return Err("无法创建请求".into());
        }
        let _request = HandleGuard(request);

        WinHttpSendRequest(request, None, None, 0, 0, 0)
            .map_err(|e| format!("发送请求失败：{}", e.message()))?;
        WinHttpReceiveResponse(request, std::ptr::null_mut())
            .map_err(|e| format!("接收响应失败：{}", e.message()))?;

        // 状态码必须查。下载更新包时，如果把 404/500 的错误页当成 exe 存下来，
        // 下一步就会拿它替换掉自己 —— 这个检查是最后一道闸。
        let mut status: u32 = 0;
        let mut len = std::mem::size_of::<u32>() as u32;
        let mut index: u32 = 0;
        WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut c_void),
            &mut len,
            &mut index,
        )
        .map_err(|e| format!("读取响应状态码失败：{}", e.message()))?;
        if status != 200 {
            return Err(format!("服务端返回 HTTP {status}"));
        }

        let mut buffer = [0u8; 64 * 1024];
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
            if !on_chunk(&buffer[..read as usize]) {
                break;
            }
        }
        Ok(())
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
