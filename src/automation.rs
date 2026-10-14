//! 客户端窗口自动化：定位登录对话框、填入账号密码、点击登录。
//!
//! 逐条对照原 `mainwidget.cpp` 的实现移植，包括那些看起来"绕"但必要的细节：
//! 控件不按顺序取而是按几何位置定位、账户下拉框优先选中已有项、QtPacker 用
//! PostMessage 异步点击、错误提示按与语言无关的返回码匹配。

use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{FALSE, HWND, LPARAM, TRUE, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetClassNameW, GetWindowRect, GetWindowTextW,
    IsWindowVisible, PostMessageW, SendMessageW, BM_CLICK, CB_GETCOUNT, CB_GETLBTEXT,
    CB_GETLBTEXTLEN, CB_SETCURSEL, WM_CHAR, WM_SETTEXT,
};

use crate::util::{from_wide, to_wide};

/// 支持同时兼容的三种客户端环境
pub struct DialogEnv {
    /// 密码对话框标题
    pub title: &'static str,
    /// 登录按钮文字
    pub login_btn: &'static str,
}

pub const DIALOG_ENVS: [DialogEnv; 3] = [
    DialogEnv { title: "密码对话框", login_btn: "登录" },      // 简体环境
    DialogEnv { title: "密碼對話框", login_btn: "Login" },     // 繁体环境
    DialogEnv { title: "Password Dialog", login_btn: "Login" }, // 英文环境
];

/// QtPacker 凭据对话框标题带版本号与运行环境，只取固定片段做"包含"匹配
const PACKER_DIALOG_TITLE_KEY: &str = "Enter your credential";
/// QtPacker 登录失败提示框
const PACKER_ERROR_TITLE: &str = "Message";
const PACKER_ERROR_TEXT_KEY: &str = "Fail to login";

// ---------------------------------------------------------------------------
// 基础工具
// ---------------------------------------------------------------------------

/// 获取父窗口的直接子控件（按类名过滤，None 表示不过滤）
fn direct_children(parent: HWND, class_name: Option<&str>) -> Vec<HWND> {
    let mut out = Vec::new();
    if parent.0.is_null() {
        return out;
    }
    let class = class_name.map(to_wide);
    let class_ptr = match &class {
        Some(buf) => PCWSTR(buf.as_ptr()),
        None => PCWSTR::null(),
    };

    let mut child = HWND::default();
    loop {
        match unsafe { FindWindowExW(Some(parent), Some(child), class_ptr, PCWSTR::null()) } {
            Ok(next) if !next.0.is_null() => {
                out.push(next);
                child = next;
            }
            _ => break,
        }
    }
    out
}

/// 控件在屏幕上的纵坐标
fn window_top(hwnd: HWND) -> i32 {
    let mut rect = Default::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
        rect.top
    } else {
        -1
    }
}

/// 可见控件中取最下方（纵坐标最大）的一个
fn bottom_most(handles: &[HWND]) -> Option<HWND> {
    let mut result: Option<HWND> = None;
    let mut max_top = -1;
    for &handle in handles {
        if !unsafe { IsWindowVisible(handle) }.as_bool() {
            continue;
        }
        let top = window_top(handle);
        if top > max_top {
            max_top = top;
            result = Some(handle);
        }
    }
    result
}

/// 递归收集所有指定类名的后代控件
fn collect_descendants(parent: HWND, class_name: &str, out: &mut Vec<HWND>) {
    if parent.0.is_null() {
        return;
    }
    for child in direct_children(parent, None) {
        let mut buf = [0u16; 256];
        let len = unsafe { GetClassNameW(child, &mut buf) };
        if len > 0 && from_wide(&buf[..len as usize]).eq_ignore_ascii_case(class_name) {
            out.push(child);
        }
        collect_descendants(child, class_name, out);
    }
}

/// 窗口内所有可见 Edit，按屏幕纵坐标自小到大排序
fn visible_edits_sorted(root: HWND) -> Vec<HWND> {
    let mut edits = Vec::new();
    collect_descendants(root, "Edit", &mut edits);
    edits.retain(|&e| unsafe { IsWindowVisible(e) }.as_bool());
    edits.sort_by_key(|&e| window_top(e));
    edits
}

fn window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if len <= 0 {
        String::new()
    } else {
        from_wide(&buf[..len as usize])
    }
}

struct FindTitleCtx {
    needle: String,
    result: HWND,
}

unsafe extern "system" fn find_title_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut FindTitleCtx);
    if !IsWindowVisible(hwnd).as_bool() {
        return TRUE;
    }
    let text = window_text(hwnd);
    if !text.is_empty() && text.contains(&ctx.needle) {
        ctx.result = hwnd;
        return FALSE;
    }
    TRUE
}

/// 按"标题包含子串"查找顶层窗口
fn find_top_window_by_title(needle: &str) -> Option<HWND> {
    let mut ctx = FindTitleCtx {
        needle: needle.to_string(),
        result: HWND::default(),
    };
    unsafe {
        let _ = EnumWindows(Some(find_title_proc), LPARAM(&mut ctx as *mut _ as isize));
    }
    if ctx.result.0.is_null() {
        None
    } else {
        Some(ctx.result)
    }
}

/// 写入下拉框内容：优先选中列表中已有的项，否则直接写入文本
fn set_combo_text(hwnd: HWND, text: &str) -> bool {
    let target = text.trim();
    if hwnd.0.is_null() || target.is_empty() {
        return false;
    }

    let count = unsafe { SendMessageW(hwnd, CB_GETCOUNT, None, None) }.0 as i32;
    for i in 0..count {
        let len = unsafe { SendMessageW(hwnd, CB_GETLBTEXTLEN, Some(windows::Win32::Foundation::WPARAM(i as usize)), None) }.0 as i32;
        if len <= 0 {
            continue;
        }
        let mut buffer = vec![0u16; len as usize + 1];
        unsafe {
            SendMessageW(
                hwnd,
                CB_GETLBTEXT,
                Some(windows::Win32::Foundation::WPARAM(i as usize)),
                Some(LPARAM(buffer.as_mut_ptr() as isize)),
            );
        }
        if from_wide(&buffer).trim() == target {
            unsafe {
                SendMessageW(
                    hwnd,
                    CB_SETCURSEL,
                    Some(windows::Win32::Foundation::WPARAM(i as usize)),
                    None,
                );
            }
            return true;
        }
    }

    let wide = to_wide(target);
    unsafe {
        SendMessageW(hwnd, WM_SETTEXT, None, Some(LPARAM(wide.as_ptr() as isize)));
    }
    true
}

/// 先清空再逐字符输入，规避输入框里的残留内容
fn type_password(hwnd: HWND, password: &str) {
    let empty = to_wide("");
    unsafe {
        SendMessageW(hwnd, WM_SETTEXT, None, Some(LPARAM(empty.as_ptr() as isize)));
        for ch in password.chars() {
            let mut unit = [0u16; 2];
            for u in ch.encode_utf16(&mut unit) {
                SendMessageW(
                    hwnd,
                    WM_CHAR,
                    Some(windows::Win32::Foundation::WPARAM(*u as usize)),
                    None,
                );
            }
        }
    }
}

fn set_text(hwnd: HWND, text: &str) {
    let wide = to_wide(text);
    unsafe {
        SendMessageW(hwnd, WM_SETTEXT, None, Some(LPARAM(wide.as_ptr() as isize)));
    }
}

// ---------------------------------------------------------------------------
// Qatar 客户端
// ---------------------------------------------------------------------------

/// 查找密码对话框（自动适配简体 / 繁体 / 英文环境）
pub fn find_dialog() -> Option<HWND> {
    for env in DIALOG_ENVS.iter() {
        let title = to_wide(env.title);
        if let Ok(hwnd) = unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) } {
            if !hwnd.0.is_null() {
                return Some(hwnd);
            }
        }
    }
    None
}

/// 对话框内的面板控件
fn find_panel(dialog: HWND) -> Option<HWND> {
    let class = to_wide("wxWindowNR");
    let hwnd = unsafe { FindWindowExW(Some(dialog), None, PCWSTR(class.as_ptr()), PCWSTR::null()) };
    match hwnd {
        Ok(h) if !h.0.is_null() => Some(h),
        _ => None,
    }
}

/// 密码输入框：面板内最下方的可见 Edit
fn find_password(dialog: HWND) -> Option<HWND> {
    let panel = find_panel(dialog)?;
    bottom_most(&direct_children(panel, Some("Edit")))
}

/// 账户（Emp ID / 员工ID）下拉框：密码框上方纵坐标最大的 ComboBox
fn find_account(dialog: HWND) -> Option<HWND> {
    let panel = find_panel(dialog)?;
    let password = find_password(dialog)?;
    let password_top = window_top(password);

    let mut result: Option<HWND> = None;
    let mut max_top = -1;
    for combo in direct_children(panel, Some("ComboBox")) {
        if !unsafe { IsWindowVisible(combo) }.as_bool() {
            continue;
        }
        let top = window_top(combo);
        // 服务器、端口也在密码框上方，取纵坐标最大的一个即为账户行
        if top < password_top && top > max_top {
            max_top = top;
            result = Some(combo);
        }
    }
    result
}

/// 登录按钮：逐个环境匹配按钮文案（「加密狗 / Keypro」不会与登录按钮重名）
fn find_login_button(dialog: HWND) -> Option<HWND> {
    let panel = find_panel(dialog)?;
    let class = to_wide("Button");
    for env in DIALOG_ENVS.iter() {
        let text = to_wide(env.login_btn);
        if let Ok(hwnd) = unsafe {
            FindWindowExW(Some(panel), None, PCWSTR(class.as_ptr()), PCWSTR(text.as_ptr()))
        } {
            if !hwnd.0.is_null() {
                return Some(hwnd);
            }
        }
    }
    None
}

/// 登录错误信息：面板内任意 Static 文本包含 `ec: -3`
fn find_error_msg(dialog: HWND) -> Option<HWND> {
    let panel = find_panel(dialog)?;
    for stat in direct_children(panel, Some("Static")) {
        if !unsafe { IsWindowVisible(stat) }.as_bool() {
            continue;
        }
        // 三种环境下的提示都是 "Right: 0, Area: 0x99, ec: -3(...)"，
        // 该返回码不随界面语言变化
        if window_text(stat).contains("ec: -3") {
            return Some(stat);
        }
    }
    None
}

/// 执行 Qatar 客户端登录动作
pub fn perform_qatar_login(dialog: HWND, account: &str, password: &str) -> bool {
    let Some(password_hwnd) = find_password(dialog) else {
        return false;
    };

    // 自动填入账户，未配置账户时保持对话框原有内容
    let account_text = account.trim();
    if !account_text.is_empty() {
        if let Some(account_hwnd) = find_account(dialog) {
            set_combo_text(account_hwnd, account_text);
        }
    }

    type_password(password_hwnd, password);

    if let Some(login_hwnd) = find_login_button(dialog) {
        unsafe {
            SendMessageW(login_hwnd, BM_CLICK, Some(WPARAM(0)), Some(LPARAM(0)));
        }
    }

    // 命中错误提示即视为密码错误
    find_error_msg(dialog).is_some()
}

// ---------------------------------------------------------------------------
// QtPacker 客户端
// ---------------------------------------------------------------------------

/// 查找 QtPacker 凭据对话框（标题含 "Enter your credential"）
pub fn find_packer_dialog() -> Option<HWND> {
    find_top_window_by_title(PACKER_DIALOG_TITLE_KEY)
}

/// Uid 输入框：对话框内最上方的可见 Edit
fn find_packer_account(dialog: HWND) -> Option<HWND> {
    visible_edits_sorted(dialog).first().copied()
}

/// Pwd 输入框：对话框内最下方的可见 Edit
fn find_packer_password(dialog: HWND) -> Option<HWND> {
    let edits = visible_edits_sorted(dialog);
    if edits.len() < 2 {
        None
    } else {
        edits.last().copied()
    }
}

/// Login 按钮：递归查找文本为 Login 的 Button
fn find_packer_login(dialog: HWND) -> Option<HWND> {
    let mut buttons = Vec::new();
    collect_descendants(dialog, "Button", &mut buttons);
    for button in buttons {
        if !unsafe { IsWindowVisible(button) }.as_bool() {
            continue;
        }
        if window_text(button).trim().eq_ignore_ascii_case("Login") {
            return Some(button);
        }
    }
    None
}

/// QtPacker 登录失败提示框（标题 Message，正文含 Fail to login）
pub fn find_packer_error_msg() -> Option<HWND> {
    let title = to_wide(PACKER_ERROR_TITLE);
    let class = to_wide("#32770");
    let Ok(hwnd) = (unsafe {
        FindWindowW(PCWSTR(class.as_ptr()), PCWSTR(title.as_ptr()))
    }) else {
        return None;
    };
    if hwnd.0.is_null() {
        return None;
    }
    for stat in direct_children(hwnd, Some("Static")) {
        if window_text(stat).contains(PACKER_ERROR_TEXT_KEY) {
            return Some(hwnd);
        }
    }
    None
}

/// QtPacker 登录动作的执行结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackerOutcome {
    /// 已投递登录点击，等待 verify 校验
    Submitted,
    /// 对话框已消失（可能已被手动登录）
    #[allow(dead_code)]
    DialogGone,
    /// 控件尚未就绪，等待下一轮重试
    NotReady,
}

/// 执行 QtPacker 登录动作：填 Uid / Pwd 后异步点击 Login
pub fn perform_packer_login(dialog: HWND, account: &str, password: &str) -> PackerOutcome {
    // 自动填入账户（Uid），未配置账户时保持对话框原有内容
    let account_text = account.trim();
    if !account_text.is_empty() {
        if let Some(account_hwnd) = find_packer_account(dialog) {
            set_text(account_hwnd, account_text);
        }
    }

    if let Some(password_hwnd) = find_packer_password(dialog) {
        type_password(password_hwnd, password);
    }

    let Some(login_hwnd) = find_packer_login(dialog) else {
        return PackerOutcome::NotReady;
    };

    // 用 PostMessage 异步点击：登录失败时客户端会弹模态提示框，
    // 若用 SendMessage 会把本进程的界面线程阻塞在这个调用上
    unsafe {
        let _ = PostMessageW(Some(login_hwnd), BM_CLICK, WPARAM(0), LPARAM(0));
    }
    PackerOutcome::Submitted
}
