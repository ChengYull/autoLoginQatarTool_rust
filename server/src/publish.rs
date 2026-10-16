//! 发布新版本的交互：选文件 → 填参数。
//!
//! 版本号不在这里输入 —— 它由 `store::read_exe_version` 从选中的 exe 资源里读出来，
//! 对话框上只做展示。人工再输一遍版本号是这类工具最常见的低级错误来源。
//!
//! 参数对话框是自己画的（跟主窗口同一套配色和控件），只有多行更新说明用了原生 EDIT。
//! 它是**模态**的：打开期间禁用主窗口，关掉再恢复。

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app;
use crate::gfx::{ALIGN_LEFT, Canvas, Fonts};
use crate::model::BtnKind;
use crate::paint::paint_button;
use crate::theme::*;
use crate::util::{colorref, from_wide, to_wide, Rect};

pub const DLG_CLASS: &str = "RustAutoLoginServerPublishDlg";
pub const DLG_BTN_CLASS: &str = "RustAutoLoginServerDlgBtn";

const ID_NOTES: usize = 2001;
const ID_OK: usize = 2003;
const ID_CANCEL: usize = 2004;

const DLG_W: f32 = 440.0;
const DLG_H: f32 = 326.0;
const DLG_RADIUS: f32 = 12.0;

/// 发布参数
#[derive(Clone, Debug)]
pub struct PublishParams {
    pub notes: String,
    pub force: bool,
}

// ---------------------------------------------------------------------------
// 文件选择
// ---------------------------------------------------------------------------

/// 弹出系统文件选择框，返回选中的 exe 路径
pub fn pick_exe(hwnd: HWND) -> Option<PathBuf> {
    let mut buf = [0u16; 1024];
    // 过滤器是「显示名\0通配符\0显示名\0通配符\0\0」的双 0 结尾格式
    let filter: Vec<u16> = "可执行文件 (*.exe)\0*.exe\0所有文件 (*.*)\0*.*\0\0"
        .encode_utf16()
        .collect();
    let title = to_wide("选择要发布的 exe");

    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        // OFN_NOCHANGEDIR：别把进程的当前目录改到用户选文件的地方去，
        // 后面还有一堆相对路径要按 exe 目录算
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };

    if !unsafe { GetOpenFileNameW(&mut ofn) }.as_bool() {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    if len == 0 {
        return None;
    }
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

// ---------------------------------------------------------------------------
// 参数对话框
// ---------------------------------------------------------------------------

/// 对话框的交互控件标识
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum DlgWidget {
    Ok,
    Cancel,
    Force,
}

#[derive(Clone, Copy)]
struct DlgLayout {
    scale: f32,
    window: Rect,
    title_bar: Rect,
    title_text: Rect,
    version_label: Rect,
    version_value: Rect,
    file_label: Rect,
    file_value: Rect,
    notes_label: Rect,
    notes_edit: Rect,
    force_box: Rect,
    force_text: Rect,
    btn_cancel: Rect,
    btn_ok: Rect,
}

impl DlgLayout {
    fn compute(scale: f32) -> Self {
        let s = |v: f32| (v * scale).round() as i32;
        let r = |x: f32, y: f32, w: f32, h: f32| Rect::new(s(x), s(y), s(w), s(h));

        let pad = 16.0;
        let inner_w = DLG_W - pad * 2.0;

        let title_bar = r(0.0, 0.0, DLG_W, 40.0);
        let title_text = r(pad, 0.0, inner_w, 40.0);

        let version_label = r(pad, 52.0, 40.0, 26.0);
        let version_value = r(pad + 40.0 + 8.0, 52.0, inner_w - 48.0, 26.0);

        let file_label = r(pad, 82.0, 40.0, 20.0);
        let file_value = r(pad + 40.0 + 8.0, 82.0, inner_w - 48.0, 20.0);

        let notes_label = r(pad, 112.0, inner_w, 18.0);
        let notes_field = r(pad, 134.0, inner_w, 100.0);
        // 原生多行 EDIT 是顶对齐的，内缩一点让它看起来有内边距
        let notes_edit = Rect::new(
            notes_field.x + s(8.0),
            notes_field.y + s(6.0),
            notes_field.w - s(16.0),
            notes_field.h - s(12.0),
        );

        let force_box = r(pad, 246.0, 16.0, 16.0);
        let force_text = r(pad + 16.0 + 8.0, 244.0, inner_w - 24.0, 20.0);

        let btn_w = 88.0;
        let btn_ok = r(DLG_W - pad - btn_w, 278.0, btn_w, 32.0);
        let btn_cancel = r(DLG_W - pad - btn_w * 2.0 - 10.0, 278.0, btn_w, 32.0);

        DlgLayout {
            scale,
            window: Rect::new(0, 0, s(DLG_W), s(DLG_H)),
            title_bar,
            title_text,
            version_label,
            version_value,
            file_label,
            file_value,
            notes_label,
            notes_edit,
            force_box,
            force_text,
            btn_cancel,
            btn_ok,
        }
    }

    /// 更新说明输入框的外框（画圆角边框用，比 EDIT 本身大一圈）
    fn notes_frame(&self) -> Rect {
        let s = (8.0 * self.scale).round() as i32;
        Rect::new(
            self.notes_edit.x - s,
            self.notes_edit.y - (6.0 * self.scale).round() as i32,
            self.notes_edit.w + s * 2,
            self.notes_edit.h + (12.0 * self.scale).round() as i32,
        )
    }
}

struct Dialog {
    hwnd: HWND,
    layout: DlgLayout,
    fonts: Fonts,
    backdrop: Option<crate::backdrop::Backdrop>,
    version: String,
    file: PathBuf,
    force: bool,
    hover: Option<DlgWidget>,
    pressed: Option<DlgWidget>,
    h_notes: HWND,
    h_ok: HWND,
    h_cancel: HWND,
    /// 更新说明输入框的背景刷。`WM_CTLCOLOREDIT` 必须返回 **HBRUSH**，
    /// 返回 COLORREF 只会得到一个随机画刷（那个参数是刷子句柄，不是颜色值）。
    brush_white: HBRUSH,
    /// 结果。对话框销毁后由 `ask_params` 从这里取走。
    result: Arc<Mutex<Option<PublishParams>>>,
}

/// 弹出参数对话框，返回 `None` 表示用户取消。
pub fn ask_params(owner: HWND, version: &str, file: &Path) -> Option<PublishParams> {
    let hinst = app::main_instance();
    if !register_dialog_classes(hinst) {
        return None;
    }

    let dpi = app::window_dpi(owner);
    let layout = DlgLayout::compute(dpi as f32 / 96.0);
    let result: Arc<Mutex<Option<PublishParams>>> = Arc::new(Mutex::new(None));

    let class = to_wide(DLG_CLASS);
    let title = to_wide("发布新版本");

    // 居中于主窗口
    let mut owner_rect = RECT::default();
    unsafe {
        let _ = GetWindowRect(owner, &mut owner_rect);
    }
    let x = owner_rect.left + ((owner_rect.right - owner_rect.left) - layout.window.w) / 2;
    let y = owner_rect.top + ((owner_rect.bottom - owner_rect.top) - layout.window.h) / 2;

    let payload = Box::new(DialogSeed {
        version: version.to_string(),
        file: file.to_path_buf(),
        layout,
        result: result.clone(),
    });
    let payload_ptr = Box::into_raw(payload);

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_POPUP | WS_CLIPCHILDREN,
            x,
            y,
            layout.window.w,
            layout.window.h,
            Some(owner),
            None,
            Some(hinst),
            Some(payload_ptr as *const c_void),
        )
    };

    let hwnd = match hwnd {
        Ok(h) => h,
        Err(err) => {
            crate::tlog!("创建发布对话框失败：{}", err.message());
            unsafe { drop(Box::from_raw(payload_ptr)) };
            return None;
        }
    };

    unsafe {
        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }

    // 局部消息循环：主窗口已被禁用，这里只管把这个对话框跑完
    let mut msg = MSG::default();
    loop {
        let ret = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if !ret.as_bool() {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            break;
        }
    }

    unsafe {
        let _ = EnableWindow(owner, true);
        let _ = SetForegroundWindow(owner);
    }

    result.lock().ok().and_then(|mut r| r.take())
}

/// `CreateWindowExW` 的 `lpParam` 里传进来的东西
struct DialogSeed {
    version: String,
    file: PathBuf,
    layout: DlgLayout,
    result: Arc<Mutex<Option<PublishParams>>>,
}

fn register_dialog_classes(hinst: HINSTANCE) -> bool {
    unsafe {
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();

        let dlg = to_wide(DLG_CLASS);
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(dlg_wndproc),
            hInstance: hinst,
            hCursor: cursor,
            hbrBackground: HBRUSH::default(),
            lpszClassName: PCWSTR(dlg.as_ptr()),
            ..Default::default()
        };
        // 重复注册返回 0，是正常的（第二次调用时类已存在）
        RegisterClassW(&wc);

        let btn = to_wide(DLG_BTN_CLASS);
        let bwc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(dlg_btn_wndproc),
            hInstance: hinst,
            hCursor: cursor,
            hbrBackground: HBRUSH::default(),
            lpszClassName: PCWSTR(btn.as_ptr()),
            ..Default::default()
        };
        if RegisterClassW(&bwc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            crate::tlog!("注册对话框按钮类失败");
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// 对话框窗口过程
// ---------------------------------------------------------------------------

unsafe extern "system" fn dlg_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        // 裸指针上的字段访问必须先解引用（`ptr.field` 不成立）
        let cs = lparam.0 as *const CREATESTRUCTW;
        let seed = Box::from_raw((*cs).lpCreateParams as *mut DialogSeed);

        let dialog = Box::new(Dialog {
            hwnd,
            layout: seed.layout,
            fonts: Fonts::new(app::window_dpi(hwnd)),
            backdrop: None,
            version: seed.version.clone(),
            file: seed.file.clone(),
            force: false,
            hover: None,
            pressed: None,
            h_notes: HWND::default(),
            h_ok: HWND::default(),
            h_cancel: HWND::default(),
            brush_white: unsafe { CreateSolidBrush(COLORREF(colorref(INPUT_BG))) },
            result: seed.result.clone(),
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(dialog) as isize);
        return LRESULT(1);
    }

    let dlg = dlg_ref(hwnd);

    match msg {
        WM_CREATE => {
            if let Some(d) = dlg {
                create_dialog_children(d);
                // 圆角区域 + 背景层（跟主窗口同一套做法）
                let inset = (crate::backdrop::REGION_INSET * d.layout.scale).round() as i32;
                let radius =
                    ((DLG_RADIUS - crate::backdrop::REGION_INSET) * d.layout.scale).round() as i32;
                let region = CreateRoundRectRgn(
                    inset,
                    inset,
                    d.layout.window.w - inset + 1,
                    d.layout.window.h - inset + 1,
                    radius * 2,
                    radius * 2,
                );
                let _ = SetWindowRgn(hwnd, Some(region), true);

                let size = (d.layout.window.w, d.layout.window.h);
                d.backdrop = crate::backdrop::Backdrop::new(app::main_instance(), d.layout.scale, size);
                if let Some(bd) = d.backdrop.as_ref() {
                    bd.sync(hwnd, true);
                }
            }
            LRESULT(0)
        }

        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if let Some(d) = dlg {
                paint_dialog(hdc, d, false);
            }
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
            if let Some(d) = dlg {
                if d.layout.title_bar.contains(pt.x, pt.y) {
                    return LRESULT(HTCAPTION as isize);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_NCLBUTTONDBLCLK => {
            if wparam.0 == HTCAPTION as usize {
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_MOUSEMOVE => {
            if let Some(d) = dlg {
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                // 复选框是自绘的，hover / 点击都得自己判坐标
                let over = d.layout.force_box.contains(x, y)
                    || expand(d.layout.force_box, 6).contains(x, y);
                let next = if over { Some(DlgWidget::Force) } else { None };
                if d.hover != next {
                    d.hover = next;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
            }
            LRESULT(0)
        }

        WM_MOUSELEAVE => {
            if let Some(d) = dlg {
                d.hover = None;
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }

        WM_LBUTTONDOWN => {
            if let Some(d) = dlg {
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                if expand(d.layout.force_box, 6).contains(x, y) {
                    d.force = !d.force;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        }

        WM_COMMAND => {
            if let Some(d) = dlg {
                let id = (wparam.0 & 0xFFFF) as usize;
                match id {
                    ID_OK => accept(d),
                    ID_CANCEL => {
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }

        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            if let Some(d) = dlg {
                let hdc = HDC(wparam.0 as *mut c_void);
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, COLORREF(colorref(INPUT_TEXT)));
                // 必须返回画刷句柄
                return LRESULT(d.brush_white.0 as isize);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_WINDOWPOSCHANGED => {
            if let Some(d) = dlg {
                if let Some(bd) = d.backdrop.as_ref() {
                    bd.sync(hwnd, true);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }

        WM_NCDESTROY => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Dialog;
            if !ptr.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                let dialog = Box::from_raw(ptr);
                if !dialog.brush_white.0.is_null() {
                    let _ = DeleteObject(HGDIOBJ(dialog.brush_white.0));
                }
                drop(dialog);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// 把更新说明和强制开关写进结果，然后关窗
fn accept(d: &mut Dialog) {
    let notes = if d.h_notes.0.is_null() {
        String::new()
    } else {
        let mut buf = vec![0u16; 4096];
        let n = unsafe { GetWindowTextW(d.h_notes, &mut buf) };
        let text = if n > 0 {
            from_wide(&buf[..n as usize])
        } else {
            String::new()
        };
        // EDIT 里是 \r\n，统一成 \n 再存
        text.replace("\r\n", "\n").trim().to_string()
    };

    if let Ok(mut slot) = d.result.lock() {
        *slot = Some(PublishParams {
            notes,
            force: d.force,
        });
    }
    unsafe {
        let _ = DestroyWindow(d.hwnd);
    }
}

fn expand(r: Rect, by: i32) -> Rect {
    Rect::new(r.x - by, r.y - by, r.w + by * 2, r.h + by * 2)
}

fn dlg_ref(hwnd: HWND) -> Option<&'static mut Dialog> {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Dialog;
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { &mut *ptr })
    }
}

fn create_dialog_children(d: &mut Dialog) {
    let l = &d.layout;
    let hinst = app::main_instance();

    let mk = |class: &str, style: WINDOW_STYLE, id: usize, r: Rect| -> HWND {
        let class_w = to_wide(class);
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class_w.as_ptr()),
                PCWSTR::null(),
                style,
                r.x,
                r.y,
                r.w,
                r.h,
                Some(d.hwnd),
                Some(HMENU(id as *mut c_void)),
                Some(hinst),
                None,
            )
        }
        .unwrap_or_default()
    };

    // 更新说明：原生多行 EDIT。**不要 WS_BORDER** —— 边框交给外面那圈自绘圆角框，
    // 否则会在圆角框里多出一圈方角黑边。
    let notes = mk(
        "EDIT",
        WS_CHILD
            | WS_VISIBLE
            | WINDOW_STYLE(ES_MULTILINE as u32)
            | WINDOW_STYLE(ES_AUTOVSCROLL as u32)
            | WS_VSCROLL,
        ID_NOTES,
        l.notes_edit,
    );
    d.h_notes = notes;

    d.h_cancel = mk(DLG_BTN_CLASS, WS_CHILD | WS_VISIBLE, ID_CANCEL, l.btn_cancel);
    d.h_ok = mk(DLG_BTN_CLASS, WS_CHILD | WS_VISIBLE, ID_OK, l.btn_ok);

    unsafe {
        SendMessageW(
            notes,
            WM_SETFONT,
            Some(WPARAM(d.fonts.body.0 as usize)),
            Some(LPARAM(1)),
        );
    }
}

// ---------------------------------------------------------------------------
// 对话框绘制
// ---------------------------------------------------------------------------

fn paint_dialog(hdc: HDC, d: &Dialog, widgets: bool) {
    let l = &d.layout;
    let mem = unsafe { CreateCompatibleDC(Some(hdc)) };
    let bitmap = unsafe { CreateCompatibleBitmap(hdc, l.window.w.max(1), l.window.h.max(1)) };
    let old = unsafe { SelectObject(mem, HGDIOBJ(bitmap.0)) };

    if let Some(canvas) = Canvas::from_hdc(mem) {
        paint_dialog_content(&canvas, d, widgets);
    }

    unsafe {
        let _ = BitBlt(hdc, 0, 0, l.window.w, l.window.h, Some(mem), 0, 0, SRCCOPY);
        SelectObject(mem, old);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem);
    }
}

/// 对话框按钮子控件的绘制（由 `dlg_btn_wndproc` 调用）
fn paint_dialog_button(hdc: HDC, d: &Dialog, id: usize, w: i32, h: i32) {
    let l = &d.layout;
    let Some(canvas) = Canvas::from_hdc(hdc) else {
        return;
    };
    canvas.fill_rect(Rect::new(0, 0, w, h), crate::util::rgb(CARD_BG));

    let (rect, kind, text, widget) = match id {
        ID_OK => (l.btn_ok, BtnKind::Primary, "发布", DlgWidget::Ok),
        _ => (l.btn_cancel, BtnKind::Secondary, "取消", DlgWidget::Cancel),
    };
    paint_button(
        &canvas,
        &d.fonts,
        Rect::new(0, 0, rect.w, rect.h),
        kind,
        text,
        l.scale,
        d.hover == Some(widget),
        d.pressed == Some(widget),
        false,
    );
}

fn paint_dialog_content(canvas: &Canvas, d: &Dialog, widgets: bool) {
    let l = &d.layout;
    let f = &d.fonts;
    let s = l.scale;

    canvas.round_rect_filled_bordered(
        l.window,
        DLG_RADIUS * s,
        crate::util::rgb(CARD_BG),
        crate::util::rgb(CARD_BORDER),
        1.0,
    );

    canvas.text("发布新版本", l.title_text, f.title, crate::util::rgb(TITLE_TEXT), ALIGN_LEFT);

    canvas.text("版本", l.version_label, f.label, crate::util::rgb(LABEL), ALIGN_LEFT);
    canvas.text(
        &format!("v{}", d.version),
        l.version_value,
        f.title,
        crate::util::rgb(PRIMARY_BG),
        ALIGN_LEFT,
    );

    canvas.text("文件", l.file_label, f.label, crate::util::rgb(LABEL), ALIGN_LEFT);
    canvas.text(
        &file_name(&d.file),
        l.file_value,
        f.small,
        crate::util::rgb(HINT),
        ALIGN_LEFT,
    );

    canvas.text("更新说明（可选）", l.notes_label, f.label, crate::util::rgb(LABEL), ALIGN_LEFT);
    // 原生 EDIT 是方角的，外面套一个圆角边框把它盖住
    canvas.round_rect_filled_bordered(
        l.notes_frame(),
        8.0 * s,
        crate::util::rgb(INPUT_BG),
        crate::util::rgb(INPUT_BORDER),
        1.0,
    );

    paint_checkbox(canvas, l.force_box, d.force, d.hover == Some(DlgWidget::Force));
    canvas.text(
        "强制更新（客户端必须升级）",
        l.force_text,
        f.body,
        crate::util::rgb(LABEL),
        ALIGN_LEFT,
    );

    if widgets {
        paint_button(canvas, f, l.btn_ok, BtnKind::Primary, "发布", s, false, false, false);
        paint_button(canvas, f, l.btn_cancel, BtnKind::Secondary, "取消", s, false, false, false);
    }
}

fn paint_checkbox(canvas: &Canvas, r: Rect, checked: bool, hovered: bool) {
    let scale = r.w as f32 / 16.0;
    let (bg, border) = if checked {
        (PRIMARY_BG, PRIMARY_BG)
    } else if hovered {
        (INPUT_BG, INPUT_BORDER_HOVER)
    } else {
        (INPUT_BG, INPUT_BORDER)
    };
    canvas.round_rect_filled_bordered(
        r,
        4.0 * scale,
        crate::util::rgb(bg),
        crate::util::rgb(border),
        1.2 * scale,
    );

    if checked {
        // 白色对勾：两段折线，坐标按 16x16 归一化
        let stroke = (1.8 * scale).max(1.0);
        let p = |x: f32, y: f32| {
            (
                r.x as f32 + x * scale,
                r.y as f32 + y * scale,
            )
        };
        let (x1, y1) = p(4.0, 8.4);
        let (x2, y2) = p(6.8, 11.2);
        let (x3, y3) = p(12.0, 4.8);
        canvas.line(x1, y1, x2, y2, crate::util::rgba(255, 255, 255, 255), stroke);
        canvas.line(x2, y2, x3, y3, crate::util::rgba(255, 255, 255, 255), stroke);
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// 对话框按钮子控件
// ---------------------------------------------------------------------------

unsafe extern "system" fn dlg_btn_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCCREATE => LRESULT(1),

        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let parent = GetParent(hwnd).unwrap_or_default();
            if let Some(d) = dlg_ref(parent) {
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let id = GetDlgCtrlID(hwnd) as usize;
                paint_dialog_button(hdc, d, id, rc.right - rc.left, rc.bottom - rc.top);
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }

        WM_ERASEBKGND => LRESULT(1),

        WM_MOUSEMOVE => {
            set_dlg_hover(hwnd, true);
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut tme);
            LRESULT(0)
        }

        WM_MOUSELEAVE => {
            set_dlg_hover(hwnd, false);
            LRESULT(0)
        }

        WM_LBUTTONDOWN => {
            let parent = GetParent(hwnd).unwrap_or_default();
            let id = GetDlgCtrlID(hwnd) as usize;
            if let Some(d) = dlg_ref(parent) {
                d.pressed = Some(if id == ID_OK { DlgWidget::Ok } else { DlgWidget::Cancel });
            }
            let _ = SetCapture(hwnd);
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }

        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            let parent = GetParent(hwnd).unwrap_or_default();
            let id = GetDlgCtrlID(hwnd) as usize;
            if let Some(d) = dlg_ref(parent) {
                let widget = if id == ID_OK { DlgWidget::Ok } else { DlgWidget::Cancel };
                let inside = d.hover == Some(widget);
                d.pressed = None;
                let _ = InvalidateRect(Some(hwnd), None, false);
                if inside {
                    let _ = SendMessageW(
                        parent,
                        WM_COMMAND,
                        Some(WPARAM(id)),
                        Some(LPARAM(hwnd.0 as isize)),
                    );
                }
            }
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn set_dlg_hover(hwnd: HWND, on: bool) {
    let parent = unsafe { GetParent(hwnd) }.unwrap_or_default();
    let id = unsafe { GetDlgCtrlID(hwnd) } as usize;
    let widget = if id == ID_OK { DlgWidget::Ok } else { DlgWidget::Cancel };
    if let Some(d) = dlg_ref(parent) {
        if on {
            d.hover = Some(widget);
        } else if d.hover == Some(widget) {
            d.hover = None;
        }
    }
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}
