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

/// 1.0.5 的「设置密码」接口（POST JSON）。
///
/// `apv1.qtech.com` 实测解析到 `10.170.1.137` —— 跟 1.0.4 那个 `forgetpwd`
/// （`GET /api/user/forgetpwd?uid=`，服务端把新密码发钉钉）是同一台服务器，
/// 只是换了路径和语义。那条老通路在 1.0.5 已经随界面按钮一起删掉了。
///
/// 这里仍然写域名：万一日后换 IP，DNS 能兜住；真解析不了再退回 IP。
const SETPWD_HOST: &str = "apv1.qtech.com";
const SETPWD_PORT: u16 = 18081;
const SETPWD_PATH: &str = "/api/user/setpassword";
/// 成功时服务端**只**返回这个字符串，没有任何状态码可言
const SETPWD_OK: &str = "ok";

const TIMEOUT_MS: i32 = 10_000;
/// 下载产物用的超时。这个值是「两次收到数据之间的最大间隔」，
/// 传几百 KB 时网络稍微顿一下就会被 10 秒的默认值误判成超时。
const DOWNLOAD_TIMEOUT_MS: i32 = 60_000;

/// 设置密码接口地址，同样支持环境变量覆盖。
///
/// **自测必须覆盖这两个变量指向本地 mock** —— 这个接口会真的改掉真人密码，
/// 打生产环境等于把人锁在系统外面。
fn setpwd_endpoint() -> (String, u16) {
    let host = std::env::var("AUTOLOGIN_SETPWD_HOST").unwrap_or_else(|_| SETPWD_HOST.to_string());
    let port = std::env::var("AUTOLOGIN_SETPWD_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(SETPWD_PORT);
    (host, port)
}

/// 「设置密码」接口的结果
#[derive(Debug, Clone)]
pub enum SetPwdResult {
    /// 服务端返回 `ok`，密码已生效
    Ok,
    /// 服务端明确拒绝，附带原文（如「密码不符合规范！」「该密码在最近一个月使用过。请尝试新密码!」）
    Rejected(String),
    /// 网络 / 协议层失败，重试可能有救
    Failed(String),
}

/// 服务端是否因为「最近一个月用过」而拒绝。
///
/// 这条错误是唯一可以自愈的失败：换下一个候选密码重试就行。
/// 判断放在这里，是因为这句文案属于服务端契约，改的时候只改一处。
pub fn is_reuse_rejection(text: &str) -> bool {
    text.contains("最近一个月")
}

/// 调用设置密码接口。阻塞调用，需在后台线程里执行。
///
/// 服务端的约定很土：**HTTP 200 + 响应体恰好是 `ok` 才算成功**，
/// 失败时响应体就是一句中文原因。所以这里不做状态码推断，
/// 非 `ok` 一律原样交给上层展示 —— 自己编文案只会让排查更难。
pub fn set_password(uid: &str, pwd: &str) -> SetPwdResult {
    let body = format!(
        "{{\"uid\":\"{}\",\"pwd\":\"{}\"}}",
        json_escape(uid),
        json_escape(pwd)
    );
    let (host, port) = setpwd_endpoint();
    match http_post_json(&host, port, SETPWD_PATH, &body) {
        Ok(text) => {
            let trimmed = text.trim().trim_start_matches('\u{feff}').trim();
            if trimmed.eq_ignore_ascii_case(SETPWD_OK) {
                SetPwdResult::Ok
            } else if trimmed.is_empty() {
                SetPwdResult::Rejected("接口未返回内容，请检查网络或接口地址。".into())
            } else {
                SetPwdResult::Rejected(trimmed.to_string())
            }
        }
        Err(err) => SetPwdResult::Failed(err),
    }
}

/// 最小 JSON 字符串转义。密码里理论上只会有 ASCII 可见字符，
/// 但用户手输时可能带上 `"` 或 `\`，不转义会直接构造出非法 JSON。
fn json_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// 明文 HTTP GET，返回响应体（已按 UTF-8 宽松解码）。
///
/// 同步阻塞调用，调用方负责放到后台线程里跑。
pub fn http_get_on(host_name: &str, port: u16, path_and_query: &str) -> Result<String, String> {
    let mut body: Vec<u8> = Vec::new();
    http_fetch(
        host_name,
        port,
        "GET",
        path_and_query,
        None,
        None,
        TIMEOUT_MS,
        &mut |chunk| {
            body.extend_from_slice(chunk);
            Ok(())
        },
    )?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// 明文 HTTP POST，请求体是一段 JSON，返回响应体。
pub fn http_post_json(
    host_name: &str,
    port: u16,
    path: &str,
    json: &str,
) -> Result<String, String> {
    let mut body: Vec<u8> = Vec::new();
    http_fetch(
        host_name,
        port,
        "POST",
        path,
        Some("Content-Type: application/json; charset=utf-8"),
        Some(json.as_bytes()),
        TIMEOUT_MS,
        &mut |chunk| {
            body.extend_from_slice(chunk);
            Ok(())
        },
    )?;
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
    // 下载用更长的超时：`WinHttpSetTimeouts` 的最后一个是「两次收到数据之间的最大间隔」，
    // 查版本接口 10 秒足够，但传几百 KB 时网络稍微顿一下就会误判成超时。
    http_fetch(
        host_name,
        port,
        "GET",
        path_and_query,
        None,
        None,
        DOWNLOAD_TIMEOUT_MS,
        &mut |chunk| {
            file.write_all(chunk)
                .map_err(|e| format!("写入本地文件失败：{e}"))?;
            written += chunk.len() as u64;
            Ok(())
        },
    )?;
    let _ = file.flush();
    Ok(written)
}

/// 发一个明文 HTTP 请求，把响应体逐块交给 `on_chunk`。
///
/// `method` / `headers` / `body` 三个参数是为 1.0.5 的 POST JSON 接口加的；
/// GET 调用传 `("GET", None, None)`，行为与之前完全一致。
///
/// **出错必须返回 Err，不能静默 break** —— 下载更新包时「少收了几个字节」和
/// 「传完了」的区别，就是「把自己替换成半截 exe」和「正常升级」的区别。
fn http_fetch(
    host_name: &str,
    port: u16,
    method: &str,
    path_and_query: &str,
    headers: Option<&str>,
    body: Option<&[u8]>,
    timeout_ms: i32,
    on_chunk: &mut dyn FnMut(&[u8]) -> Result<(), String>,
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

        let _ = WinHttpSetTimeouts(session, timeout_ms, timeout_ms, timeout_ms, timeout_ms);

        let host = to_wide(host_name);
        let connect = WinHttpConnect(session, PCWSTR(host.as_ptr()), port, 0);
        if connect.is_null() {
            return Err("无法连接到服务器".into());
        }
        let _connect = HandleGuard(connect);

        let verb = to_wide(method);
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

        // WinHttpSendRequest 收的是「UTF-16 切片 + 显式长度」，切片里不能带结尾的 0，
        // 否则 dwHeadersLength 会多算一个字符，某些服务端会因此判成非法头。
        let header_buf: Vec<u16> = headers
            .map(|h| {
                let mut v: Vec<u16> = h.encode_utf16().collect();
                v.push(0);
                v
            })
            .unwrap_or_default();
        let header_slice: Option<&[u16]> = if headers.is_some() {
            Some(&header_buf[..header_buf.len() - 1])
        } else {
            None
        };

        let body_len = body.map(|b| b.len() as u32).unwrap_or(0);
        let body_ptr = body.map(|b| b.as_ptr() as *const c_void);

        WinHttpSendRequest(request, header_slice, body_ptr, body_len, body_len, 0)
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
            // 服务端会在响应体里写清楚原因（比如「产物文件损坏：磁盘上 65536 字节」），
            // 把它带出来 —— 只说"HTTP 500"等于让用户干瞪眼。
            let mut detail = String::new();
            let mut buf = [0u8; 512];
            let mut read: u32 = 0;
            if WinHttpReadData(
                request,
                buf.as_mut_ptr() as *mut c_void,
                buf.len() as u32,
                &mut read,
            )
            .is_ok()
                && read > 0
            {
                detail = String::from_utf8_lossy(&buf[..read as usize]).trim().to_string();
            }
            return Err(if detail.is_empty() {
                format!("服务端返回 HTTP {status}")
            } else {
                format!("服务端返回 HTTP {status}：{detail}")
            });
        }

        let mut buffer = [0u8; 64 * 1024];
        loop {
            let mut available: u32 = 0;
            WinHttpQueryDataAvailable(request, &mut available)
                .map_err(|e| format!("读取响应失败（可能超时）：{}", e.message()))?;
            if available == 0 {
                break; // 数据读完，正常结束
            }
            let to_read = available.min(buffer.len() as u32);
            let mut read: u32 = 0;
            WinHttpReadData(
                request,
                buffer.as_mut_ptr() as *mut c_void,
                to_read,
                &mut read,
            )
            .map_err(|e| format!("读取响应数据失败：{}", e.message()))?;
            if read == 0 {
                break;
            }
            on_chunk(&buffer[..read as usize])?;
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

