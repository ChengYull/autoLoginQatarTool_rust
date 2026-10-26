//! 密码相关对话框（1.0.5）。
//!
//! 两个窗口共用一套骨架：无边框卡片 + 逐像素 alpha 背景层（复用 `backdrop`）
//! + 内缩 2px 的圆角区域，和主窗口同一套做法 —— 这样对话框和主界面的圆角、
//! 描边、阴影过渡完全一致，不会出现「主窗口是圆角、弹窗是方角」的割裂感。
//!
//! 这里只负责**收集输入**，不碰网络：用户点「设置」/「保存」后把结果返回给
//! `App`，由主窗口那边起后台线程调接口。好处是对话框不用关心请求状态，
//! 关掉对话框之后的重试、提示都归 `App` 管。

use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, EM_SETMARGINS, EM_SETPASSWORDCHAR, ODS_SELECTED};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::backdrop::{Backdrop, REGION_INSET};
use crate::gfx::{Canvas, Fonts, ALIGN_CENTER, ALIGN_LEFT};
use crate::model::BtnKind;
use crate::paint::paint_button;
use crate::pwd::{self, POOL_SIZE};
use crate::theme;
use crate::util::{colorref, from_wide, rgb, to_wide, Rect};

pub const DLG_CLASS: &str = "RustAutoLoginDlg";

const ID_ACCOUNT: usize = 3100;
const ID_PWD_BASE: usize = 3110;
const ID_GEN: usize = 3120;
/// 每个密码框右侧的小眼睛：ID_EYE_BASE + i
const ID_EYE_BASE: usize = 3140;
const ID_CANCEL: usize = 3130;
const ID_OK: usize = 3131;
/// 标题栏关闭按钮。单独一个 ID，是为了让它只关窗口、不触发校验。
const ID_CLOSE_BTN: usize = 3132;

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Kind {
    /// 自定义密码：单次手动设置
    Custom,
    /// 自动重置密码：7 个密码 + 轮换开关
    Auto,
}

/// 对话框关闭时交回给主窗口的结果
pub struct Outcome {
    /// 哪个对话框产生的（调用方一般已经知道，留着便于日志与将来复用）
    #[allow(dead_code)]
    pub kind: Kind,
    /// 对话框里填的账户（默认预填主界面的账户，允许改）
    pub account: String,
    /// Custom 只有一个；Auto 是 7 个
    pub passwords: Vec<String>,
    pub auto: bool,
}

// ---------------------------------------------------------------------------
// 几何
// ---------------------------------------------------------------------------

struct Geo {
    w: i32,
    h: i32,
    title: Rect,
    close_btn: Rect,
    account_label: Rect,
    /// 账户输入框外框 + 内部 EDIT
    account_box: Rect,
    account_edit: Rect,
    pwd_labels: Vec<Rect>,
    pwd_boxes: Vec<Rect>,
    pwd_edits: Vec<Rect>,
    /// 每个密码框右侧的小眼睛（1 个或 7 个）
    eyes: Vec<Rect>,
    hint: Rect,
    error: Rect,
    gen_btn: Rect,
    next: Rect,
    cancel: Rect,
    ok: Rect,
}

const TITLE_H: f32 = 42.0;
const PAD: f32 = 16.0;
const ROW_H: f32 = 28.0;
const GAP: f32 = 8.0;
/// 输入框右侧给眼睛留的宽度
const EYE_W: f32 = 26.0;
const EYE_SIZE: f32 = 16.0;

fn build_geo(kind: Kind, scale: f32, body_line_h: i32) -> Geo {
    let s = |v: f32| (v * scale).round() as i32;
    let body_h = body_line_h.max(1);

    let (w_f, h_f) = match kind {
        Kind::Custom => (320.0, 248.0),
        // 去掉原来那行「显示密码 / 启用自动重置」勾选框，高度相应收紧
        Kind::Auto => (360.0, 414.0),
    };
    let (w, h) = (s(w_f), s(h_f));

    let title = Rect::new(0, 0, w, s(TITLE_H));
    let close_btn = Rect::new(w - s(10.0) - s(30.0), s((TITLE_H - 28.0) / 2.0), s(30.0), s(28.0));

    let inner_x = s(PAD);
    let inner_w = w - s(PAD) * 2;
    let mut y = s(TITLE_H) + s(8.0);

    // 账户：可编辑输入框，样式和密码框一致，只是预填当前账户
    let account_label = Rect::new(inner_x, y, s(40.0), s(ROW_H));
    let account_box = Rect::new(inner_x + s(48.0), y, inner_w - s(48.0), s(ROW_H));
    let account_edit = Rect::new(
        account_box.x + s(9.0),
        account_box.y + (account_box.h - body_h) / 2,
        account_box.w - s(18.0),
        body_h + s(4.0),
    );
    y += s(ROW_H) + s(GAP);

    let mut pwd_labels = Vec::new();
    let mut pwd_boxes = Vec::new();
    let mut pwd_edits = Vec::new();
    let mut eyes = Vec::new();
    let hint;
    let gen_btn;
    let next;
    let error;

    match kind {
        Kind::Custom => {
            let label = Rect::new(inner_x, y, inner_w, s(16.0));
            y += s(16.0) + s(4.0);
            let fld = Rect::new(inner_x, y, inner_w, s(ROW_H));
            let eye = Rect::new(
                fld.right() - s(6.0) - s(EYE_SIZE),
                fld.y + (fld.h - s(EYE_SIZE)) / 2,
                s(EYE_SIZE),
                s(EYE_SIZE),
            );
            let edit = Rect::new(
                fld.x + s(10.0),
                fld.y + (fld.h - body_h) / 2,
                fld.w - s(10.0) - s(EYE_W),
                body_h + s(4.0),
            );
            pwd_labels.push(label);
            pwd_boxes.push(fld);
            pwd_edits.push(edit);
            eyes.push(eye);
            y += s(ROW_H) + s(GAP);

            hint = Rect::new(inner_x, y, inner_w, s(15.0));
            y += s(15.0) + s(10.0);
            error = Rect::new(inner_x, y, inner_w, s(15.0));

            gen_btn = Rect::new(0, 0, 0, 0);
            next = Rect::new(0, 0, 0, 0);
        }
        Kind::Auto => {
            // 「7 个密码按顺序轮换」+ 生成按钮
            hint = Rect::new(inner_x, y, inner_w - s(56.0), s(15.0));
            gen_btn = Rect::new(inner_x + inner_w - s(52.0), y - s(5.0), s(52.0), s(24.0));
            y += s(15.0) + s(8.0);

            for _ in 0..POOL_SIZE {
                pwd_labels.push(Rect::new(inner_x, y, s(14.0), s(26.0)));
                let fld = Rect::new(inner_x + s(18.0), y, inner_w - s(18.0), s(26.0));
                let eye = Rect::new(
                    fld.right() - s(5.0) - s(EYE_SIZE),
                    fld.y + (fld.h - s(EYE_SIZE)) / 2,
                    s(EYE_SIZE),
                    s(EYE_SIZE),
                );
                let edit = Rect::new(
                    fld.x + s(9.0),
                    fld.y + (fld.h - body_h) / 2,
                    fld.w - s(9.0) - s(EYE_W),
                    body_h + s(4.0),
                );
                pwd_boxes.push(fld);
                pwd_edits.push(edit);
                eyes.push(eye);
                y += s(26.0) + s(4.0);
            }
            y += s(6.0);

            next = Rect::new(inner_x, y, inner_w, s(15.0));
            y += s(15.0) + s(4.0);
            error = Rect::new(inner_x, y, inner_w, s(15.0));
        }
    }

    let btn_h = s(32.0);
    let half = (inner_w - s(10.0)) / 2;
    let cancel = Rect::new(inner_x, h - s(PAD) - btn_h, half, btn_h);
    let ok = Rect::new(inner_x + half + s(10.0), h - s(PAD) - btn_h, half, btn_h);

    Geo {
        w,
        h,
        title,
        close_btn,
        account_label,
        account_box,
        account_edit,
        pwd_labels,
        pwd_boxes,
        pwd_edits,
        eyes,
        hint,
        error,
        gen_btn,
        next,
        cancel,
        ok,
    }
}

// ---------------------------------------------------------------------------
// 对话框状态
// ---------------------------------------------------------------------------

struct Dlg {
    hwnd: HWND,
    hinst: HINSTANCE,
    kind: Kind,
    scale: f32,
    fonts: Fonts,
    backdrop: Option<Backdrop>,
    geo: Geo,
    account: String,
    /// 打开时预填进输入框的密码（编辑已有配置）
    initial: Vec<String>,
    pwd_edits: Vec<HWND>,
    /// 每个密码框右侧的小眼睛
    eye_btns: Vec<HWND>,
    /// 每个密码框当前是否「爆红」
    invalid: Vec<bool>,
    /// 每个密码框当前是否明文显示（眼睛的状态）
    show: [bool; POOL_SIZE],
    next_text: String,
    /// 用户点过「设置 / 保存」之后才显示错误 —— 否则一打开满屏红框
    show_errors: bool,
    hover: usize,
    outcome: Option<Outcome>,
}

impl Dlg {
    /// 读第 i 个密码框的文本
    fn read_pwd(&self, i: usize) -> String {
        let hwnd = match self.pwd_edits.get(i) {
            Some(&h) => h,
            None => return String::new(),
        };
        let mut buf = [0u16; 64];
        let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
        if len <= 0 {
            String::new()
        } else {
            from_wide(&buf[..len as usize])
        }
    }

    fn set_pwd(&self, i: usize, text: &str) {
        if let Some(&hwnd) = self.pwd_edits.get(i) {
            let wide = to_wide(text);
            unsafe {
                let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
            }
        }
    }

    /// 收集输入并校验。返回 Err(第一个出问题的框下标, 原因)。
    fn collect(&self) -> Result<Vec<String>, (usize, String)> {
        let n = match self.kind {
            Kind::Custom => 1,
            Kind::Auto => POOL_SIZE,
        };
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let pwd = self.read_pwd(i);
            if let Err(msg) = pwd::validate(&pwd) {
                return Err((i, msg));
            }
            out.push(pwd);
        }
        if self.kind == Kind::Auto {
            if let Err(e) = pwd::validate_pool(&out) {
                return Err(e);
            }
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// 窗口类
// ---------------------------------------------------------------------------

pub fn register(hinst: HINSTANCE) -> bool {
    unsafe {
        let class = to_wide(DLG_CLASS);
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(dlg_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: HICON::default(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: PCWSTR(class.as_ptr()),
        };
        if RegisterClassW(&wc) == 0 {
            crate::tlog!("注册对话框窗口类失败");
            return false;
        }
    }
    true
}

/// 打开对话框（模态）。返回 `None` 表示用户取消。
pub fn open(
    main: HWND,
    hinst: HINSTANCE,
    dpi: u32,
    kind: Kind,
    account: &str,
    passwords: &[String],
    next_text: &str,
) -> Option<Outcome> {
    let scale = dpi as f32 / 96.0;
    let fonts = Fonts::new(dpi);
    let geo = build_geo(kind, scale, fonts.body_line_h);

    let dlg = Box::new(Dlg {
        hwnd: HWND::default(),
        hinst,
        kind,
        scale,
        fonts,
        backdrop: None,
        geo,
        account: account.to_string(),
        initial: passwords.to_vec(),
        pwd_edits: Vec::new(),
        eye_btns: Vec::new(),
        invalid: vec![false; POOL_SIZE],
        show: [false; POOL_SIZE],
        next_text: next_text.to_string(),
        show_errors: false,
        hover: 0,
        outcome: None,
    });
    let ptr = Box::into_raw(dlg);

    // 居中到主窗口
    let mut parent = RECT::default();
    unsafe {
        let _ = GetWindowRect(main, &mut parent);
    }
    let (w, h) = {
        let d = unsafe { &*ptr };
        (d.geo.w, d.geo.h)
    };
    let x = parent.left + ((parent.right - parent.left) - w) / 2;
    let y = parent.top + ((parent.bottom - parent.top) - h) / 2;

    let title = to_wide(match kind {
        Kind::Custom => "自定义密码",
        Kind::Auto => "自动重置密码",
    });
    // 必须提前绑定：直接写 `PCWSTR(to_wide(..).as_ptr())` 会让临时 Vec 在语句末尾
    // 就被释放，指针当场悬空（这类 bug 只在特定优化下才炸）。
    let class_w = to_wide(DLG_CLASS);
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_TOOLWINDOW,
            PCWSTR(class_w.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_POPUP | WS_CLIPCHILDREN,
            x,
            y,
            w,
            h,
            Some(main),
            None,
            Some(hinst),
            Some(ptr as *const c_void),
        )
    };
    let hwnd = match hwnd {
        Ok(h) if !h.0.is_null() => h,
        _ => {
            crate::tlog!("创建对话框失败");
            drop(unsafe { Box::from_raw(ptr) });
            return None;
        }
    };

    unsafe {
        let _ = EnableWindow(main, false);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }

    // 局部消息循环 = 真模态。IsDialogMessage 负责 Tab 切换和 Enter 默认按钮。
    let mut msg = MSG::default();
    unsafe {
        while IsWindow(Some(hwnd)).as_bool() {
            let ret = GetMessageW(&mut msg, None, 0, 0);
            if ret.0 <= 0 {
                break;
            }
            if !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let _ = EnableWindow(main, true);
        let _ = SetForegroundWindow(main);
    }

    let dlg = unsafe { Box::from_raw(ptr) };
    let mut dlg = dlg;
    if let Some(bd) = dlg.backdrop.as_mut() {
        bd.destroy();
    }
    dlg.outcome.take()
}

// ---------------------------------------------------------------------------
// 窗口过程
// ---------------------------------------------------------------------------

unsafe extern "system" fn dlg_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        let dlg = &mut *(cs.lpCreateParams as *mut Dlg);
        dlg.hwnd = hwnd;
        return LRESULT(1);
    }

    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Dlg;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let dlg = &mut *ptr;

    match msg {
        WM_CREATE => {
            create_children(dlg);
            create_backdrop(dlg);
            LRESULT(0)
        }

        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            paint(dlg, hdc);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }

        WM_ERASEBKGND => LRESULT(1),

        WM_NCHITTEST => {
            let mut pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            let _ = ScreenToClient(hwnd, &mut pt);
            if dlg.geo.title.contains(pt.x, pt.y) && !dlg.geo.close_btn.contains(pt.x, pt.y) {
                return LRESULT(HTCAPTION as isize);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as usize;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u16;
            // 按钮和复选框的 BN_CLICKED 都是 0，靠 ID 区分
            if code == 0 {
                if (ID_EYE_BASE..ID_EYE_BASE + POOL_SIZE).contains(&id) {
                    let i = id - ID_EYE_BASE;
                    if let Some(slot) = dlg.show.get_mut(i) {
                        *slot = !*slot;
                    }
                    apply_password_mask(dlg);
                    // 重画眼睛本身（睁开 / 划掉）
                    if let Some(&eye) = dlg.eye_btns.get(i) {
                        let _ = InvalidateRect(Some(eye), None, true);
                    }
                    return LRESULT(0);
                }
                match id {
                    ID_GEN => fill_random(dlg),
                    ID_OK => {
                        try_submit(dlg);
                    }
                    ID_CANCEL | ID_CLOSE_BTN => {
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }

        WM_DRAWITEM => {
            let dis = &*(lparam.0 as *const DRAWITEMSTRUCT);
            draw_button(dlg, dis);
            LRESULT(1)
        }

        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            let hdc = HDC(wparam.0 as *mut c_void);
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, COLORREF(colorref(theme::INPUT_TEXT)));
            // 静态文本（标签 / 复选框）用透明底，避免出现一块灰底
            if msg == WM_CTLCOLORSTATIC {
                return LRESULT(GetStockObject(NULL_BRUSH).0 as isize);
            }
            // 用系统的白色 stock brush：每次都 CreateSolidBrush 会一帧泄漏一个 GDI 对象
            LRESULT(GetStockObject(WHITE_BRUSH).0 as isize)
        }

        // 拖动时背景层要跟着走，否则卡片会留在原地
        WM_WINDOWPOSCHANGED => {
            sync_backdrop(dlg);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }

        WM_DESTROY => LRESULT(0),

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// 按每行自己的开关刷新掩码字符
fn apply_password_mask(dlg: &Dlg) {
    unsafe {
        for (i, &edit) in dlg.pwd_edits.iter().enumerate() {
            let mask = if dlg.show.get(i).copied().unwrap_or(false) { 0 } else { 0x25CF };
            SendMessageW(edit, EM_SETPASSWORDCHAR, Some(WPARAM(mask)), None);
            let _ = InvalidateRect(Some(edit), None, true);
        }
    }
}

// ---------------------------------------------------------------------------
// 子控件
// ---------------------------------------------------------------------------

fn create_children(dlg: &mut Dlg) {
    let g = &dlg.geo;

    // 账户：可编辑输入框，预填当前账户。
    // 允许改是因为偶尔要帮同事设一次密码，改完这个对话框里的请求就用新账户。
    let acc = create_child(
        dlg,
        "EDIT",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(0x0080), // ES_AUTOHSCROLL
        ID_ACCOUNT,
        g.account_edit,
    );
    let text = to_wide(&dlg.account);
    unsafe {
        let _ = SetWindowTextW(acc, PCWSTR(text.as_ptr()));
        SendMessageW(acc, WM_SETFONT, Some(WPARAM(dlg.fonts.body.0 as usize)), Some(LPARAM(1)));
        SendMessageW(acc, EM_SETMARGINS, Some(WPARAM(0)), Some(LPARAM(0)));
    }

    let n = match dlg.kind {
        Kind::Custom => 1,
        Kind::Auto => POOL_SIZE,
    };
    for i in 0..n {
        let edit = create_child(
            dlg,
            "EDIT",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(0x0080), // ES_AUTOHSCROLL
            ID_PWD_BASE + i,
            g.pwd_edits[i],
        );
        unsafe {
            SendMessageW(edit, WM_SETFONT, Some(WPARAM(dlg.fonts.body.0 as usize)), Some(LPARAM(1)));
            SendMessageW(edit, EM_SETPASSWORDCHAR, Some(WPARAM(0x25CF)), None);
            // 单行 EDIT 默认有 1px 内边距，把它清零，文字才能和自绘外框对齐
            SendMessageW(edit, EM_SETMARGINS, Some(WPARAM(0)), Some(LPARAM(0)));
        }
        dlg.pwd_edits.push(edit);

        // 每行一个小眼睛，点一下切换该行明文 / 掩码
        let eye = create_child(
            dlg,
            "BUTTON",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(0x000B), // BS_OWNERDRAW
            ID_EYE_BASE + i,
            g.eyes[i],
        );
        dlg.eye_btns.push(eye);
    }

    // 预填已有密码（「自动重置密码」再打开时要能看到之前填的 7 个）
    for (i, pwd) in dlg.initial.iter().enumerate() {
        if i < dlg.pwd_edits.len() {
            let wide = to_wide(pwd);
            unsafe {
                let _ = SetWindowTextW(dlg.pwd_edits[i], PCWSTR(wide.as_ptr()));
            }
        }
    }

    if dlg.kind == Kind::Auto {
        // 生成按钮
        let gen = create_child(
            dlg,
            "BUTTON",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(0x000B), // BS_OWNERDRAW
            ID_GEN,
            g.gen_btn,
        );
        unsafe {
            SendMessageW(gen, WM_SETFONT, Some(WPARAM(dlg.fonts.small.0 as usize)), Some(LPARAM(1)));
        }
    }

    // 取消 / 确定
    let cancel = create_child(
        dlg,
        "BUTTON",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(0x000B),
        ID_CANCEL,
        g.cancel,
    );
    let ok = create_child(
        dlg,
        "BUTTON",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(0x000B),
        ID_OK,
        g.ok,
    );
    unsafe {
        for h in [cancel, ok] {
            SendMessageW(h, WM_SETFONT, Some(WPARAM(dlg.fonts.body.0 as usize)), Some(LPARAM(1)));
        }
    }

    // 关闭按钮（标题栏右侧，自绘）
    let close = create_child(
        dlg,
        "BUTTON",
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x000B),
        ID_CLOSE_BTN,
        g.close_btn,
    );
    unsafe {
        SendMessageW(close, WM_SETFONT, Some(WPARAM(dlg.fonts.body.0 as usize)), Some(LPARAM(1)));
    }
}

fn create_child(dlg: &Dlg, class: &str, style: WINDOW_STYLE, id: usize, r: Rect) -> HWND {
    let class_w = to_wide(class);
    let empty = to_wide("");
    unsafe {
        match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class_w.as_ptr()),
            PCWSTR(empty.as_ptr()),
            style,
            r.x,
            r.y,
            r.w,
            r.h,
            Some(dlg.hwnd),
            Some(HMENU(id as *mut c_void)),
            Some(dlg.hinst),
            None,
        ) {
            Ok(h) => h,
            Err(err) => {
                crate::tlog!("对话框控件创建失败 id={} : {}", id, err.message());
                HWND::default()
            }
        }
    }
}

fn create_backdrop(dlg: &mut Dlg) {
    dlg.backdrop = Backdrop::new(dlg.hinst, dlg.scale, (dlg.geo.w, dlg.geo.h));
    // 主窗口被裁成内缩 2px 的圆角，对话框照做 —— 硬边藏在背景层白卡里
    unsafe {
        let inset = (REGION_INSET * dlg.scale).round() as i32;
        let radius = ((theme::CARD_RADIUS - REGION_INSET) * dlg.scale).round() as i32;
        let region = CreateRoundRectRgn(
            inset,
            inset,
            dlg.geo.w - inset + 1,
            dlg.geo.h - inset + 1,
            radius * 2,
            radius * 2,
        );
        let _ = SetWindowRgn(dlg.hwnd, Some(region), true);
    }
    sync_backdrop(dlg);
}

fn sync_backdrop(dlg: &Dlg) {
    if let Some(bd) = dlg.backdrop.as_ref() {
        let visible = unsafe { IsWindowVisible(dlg.hwnd).as_bool() };
        bd.sync(dlg.hwnd, visible);
    }
}

// ---------------------------------------------------------------------------
// 绘制
// ---------------------------------------------------------------------------

fn paint(dlg: &Dlg, hdc: HDC) {
    let Some(canvas) = Canvas::from_hdc(hdc) else {
        return;
    };
    let g = &dlg.geo;
    let f = &dlg.fonts;

    canvas.fill_rect(Rect::new(0, 0, g.w, g.h), rgb(theme::CARD_BG));

    // 标题
    let title = match dlg.kind {
        Kind::Custom => "自定义密码",
        Kind::Auto => "自动重置密码",
    };
    canvas.text(
        title,
        Rect::new((PAD * dlg.scale) as i32, g.title.y, g.title.w - 100, g.title.h),
        f.title,
        rgb(theme::TITLE_TEXT),
        ALIGN_LEFT,
    );

    // 账户
    canvas.text("账户", g.account_label, f.label, rgb(theme::LABEL), ALIGN_LEFT);
    canvas.fill_round_rect(g.account_box, 6.0 * dlg.scale, rgb(theme::INPUT_BG));
    if let Some(path) = crate::gfx::Path::round_rect(g.account_box, 6.0 * dlg.scale) {
        canvas.stroke_path(&path, rgb(theme::INPUT_BORDER), 1.0);
    }

    // 密码标签 + 外框
    for (i, label) in g.pwd_labels.iter().enumerate() {
        if dlg.kind == Kind::Auto {
            canvas.text(
                &format!("{}", i + 1),
                *label,
                f.small,
                rgb(theme::SWITCH_DESC),
                ALIGN_CENTER,
            );
        } else {
            canvas.text("新密码", *label, f.label, rgb(theme::LABEL), ALIGN_LEFT);
        }
    }
    for (i, fld) in g.pwd_boxes.iter().enumerate() {
        let bad = dlg.invalid.get(i).copied().unwrap_or(false);
        let border = if bad {
            theme::DANGER_BORDER
        } else {
            theme::INPUT_BORDER
        };
        canvas.fill_round_rect(*fld, 6.0 * dlg.scale, rgb(theme::INPUT_BG));
        if let Some(path) = crate::gfx::Path::round_rect(*fld, 6.0 * dlg.scale) {
            canvas.stroke_path(&path, rgb(border), if bad { 1.5 } else { 1.0 });
        }
    }

    match dlg.kind {
        Kind::Custom => {
            canvas.text(
                "提交后立即生效，不写入自动轮换列表",
                g.hint,
                f.small,
                rgb(theme::SWITCH_DESC),
                ALIGN_LEFT,
            );
        }
        Kind::Auto => {
            canvas.text(
                "7 个密码按顺序轮换，每 7 天自动切换一个",
                g.hint,
                f.small,
                rgb(theme::SWITCH_DESC),
                ALIGN_LEFT,
            );
            canvas.text(&dlg.next_text, g.next, f.small, rgb(theme::SWITCH_DESC), ALIGN_LEFT);
        }
    }

    // 错误提示
    if let Some((idx, msg)) = current_error(dlg) {
        let text = if dlg.kind == Kind::Auto {
            format!("第 {} 个：{}", idx + 1, msg)
        } else {
            msg
        };
        canvas.text(&text, g.error, f.small, rgb(theme::DANGER_TEXT), ALIGN_LEFT);
    }
}

fn current_error(dlg: &Dlg) -> Option<(usize, String)> {
    if !dlg.show_errors {
        return None;
    }
    // 输入一变就重新校验，错误提示跟着消失
    match dlg.collect() {
        Ok(_) => None,
        Err(e) => Some(e),
    }
}

fn draw_button(dlg: &Dlg, dis: &DRAWITEMSTRUCT) {
    let id = dis.CtlID as usize;
    let rc = dis.rcItem;
    let r = Rect::new(rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top);
    let selected = (dis.itemState.0 & ODS_SELECTED.0) != 0;
    let Some(canvas) = Canvas::from_hdc(dis.hDC) else {
        return;
    };
    canvas.fill_rect(r, rgb(theme::CARD_BG));

    // 小眼睛：睁眼 = 明文，划掉 = 掩码
    if (ID_EYE_BASE..ID_EYE_BASE + POOL_SIZE).contains(&id) {
        let i = id - ID_EYE_BASE;
        let on = dlg.show.get(i).copied().unwrap_or(false);
        crate::icon::draw_eye(&canvas, r, !on, theme::WINBTN_FG, 1.6 * dlg.scale);
        return;
    }

    let (kind, text) = match id {
        ID_OK => (
            BtnKind::Primary,
            // 自动重置页的主按钮就是「保存并启用」，不再是「保存」
            if dlg.kind == Kind::Custom { "设置" } else { "自动重置" },
        ),
        ID_CANCEL => (BtnKind::Secondary, "取消"),
        ID_GEN => (BtnKind::Ghost, "生成"),
        ID_CLOSE_BTN => (BtnKind::WinClose, "✕"),
        _ => return,
    };

    let hovered = dlg.hover == id;
    paint_button(
        &canvas,
        &dlg.fonts,
        Rect::new(0, 0, r.w, r.h),
        kind,
        text,
        dlg.scale,
        hovered,
        selected,
        false,
    );
}

// ---------------------------------------------------------------------------
// 结果提交
// ---------------------------------------------------------------------------

fn text_of(hwnd: HWND) -> String {
    if hwnd.0.is_null() {
        return String::new();
    }
    let mut buf = [0u16; 256];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if len <= 0 {
        String::new()
    } else {
        from_wide(&buf[..len as usize])
    }
}

/// 校验并提交。返回 true 表示对话框已关闭。
fn try_submit(dlg: &mut Dlg) -> bool {
    // 账户在对话框里可改，提交时以输入框为准
    let acc_hwnd = unsafe { GetDlgItem(Some(dlg.hwnd), ID_ACCOUNT as i32) }.unwrap_or_default();
    let account = text_of(acc_hwnd).trim().to_string();
    if account.is_empty() {
        unsafe {
            if !acc_hwnd.0.is_null() {
                let _ = SetFocus(Some(acc_hwnd));
            }
            let _ = MessageBoxW(
                Some(dlg.hwnd),
                windows::core::w!("请先填写账户。"),
                windows::core::w!("账户为空"),
                MB_OK | MB_ICONWARNING,
            );
        }
        return false;
    }

    match dlg.collect() {
        Ok(passwords) => {
            dlg.outcome = Some(Outcome {
                kind: dlg.kind,
                account,
                passwords,
                // 自动重置页没有独立开关了：点主按钮就等于「保存并启用」
                auto: dlg.kind == Kind::Auto,
            });
            unsafe {
                let _ = DestroyWindow(dlg.hwnd);
            }
            true
        }
        Err((idx, _msg)) => {
            dlg.show_errors = true;
            for v in dlg.invalid.iter_mut() {
                *v = false;
            }
            if let Some(slot) = dlg.invalid.get_mut(idx) {
                *slot = true;
            }
            unsafe {
                if let Some(&edit) = dlg.pwd_edits.get(idx) {
                    let _ = SetFocus(Some(edit));
                }
                let _ = InvalidateRect(Some(dlg.hwnd), None, true);
            }
            false
        }
    }
}

fn fill_random(dlg: &mut Dlg) {
    let all = pwd::Pool::generate_all();
    for (i, text) in all.iter().enumerate() {
        dlg.set_pwd(i, text);
    }
    for v in dlg.invalid.iter_mut() {
        *v = false;
    }
    invalidate(dlg.hwnd);
}
