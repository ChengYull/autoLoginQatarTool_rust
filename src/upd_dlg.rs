//! 「发现新版本」对话框：展示服务端上的**全部历史版本说明**。
//!
//! 和 `dlg.rs` 同一套骨架（无边框圆角卡片 + 逐像素 alpha 背景层 + 内缩区域），
//! 但内容区是一个**只读多行 EDIT**：
//!
//! - 版本可能发布了很多个，说明也可能很长，纯 `MessageBox` 没法滚动，
//!   内容一多就被屏幕切掉，用户根本看不到旧版本的说明；
//! - 用原生 EDIT 直接白拿滚轮、键盘翻页、滚动条和「选中复制」，
//!   自己画一个可滚动文本区要写滚动条 + 命中测试 + 裁剪，收益不成正比。
//!
//! 顺序由服务端决定（当前对外版本永远在最上，其余从新到旧），这里只管照单渲染 ——
//! 客户端自己再排一次的话，回滚场景下「版本号最大的那个」会顶到最上面，是错的。

use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, EM_SETMARGINS, ODS_SELECTED};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::backdrop::{Backdrop, REGION_INSET};
use crate::gfx::{Canvas, Fonts, ALIGN_LEFT};
use crate::model::BtnKind;
use crate::paint::paint_button;
use crate::theme;
use crate::util::{colorref, rgb, to_wide, Rect};

pub const UPD_DLG_CLASS: &str = "RustAutoLoginUpdateDlg";

const ID_TEXT: usize = 3200;
const ID_LATER: usize = 3201;
const ID_UPDATE: usize = 3202;
const ID_CLOSE: usize = 3203;

const TITLE_H: f32 = 42.0;
const PAD: f32 = 16.0;

struct Geo {
    w: i32,
    h: i32,
    title: Rect,
    close_btn: Rect,
    sub: Rect,
    /// 文本区外框（父窗口画圆角 + 描边）
    text_box: Rect,
    /// 真正装文本的 EDIT
    text_edit: Rect,
    hint: Rect,
    later: Rect,
    update: Rect,
}

fn build_geo(scale: f32) -> Geo {
    let s = |v: f32| (v * scale).round() as i32;
    let w = s(420.0);
    let h = s(398.0);

    let inner_x = s(PAD);
    let inner_w = w - s(PAD) * 2;

    let title = Rect::new(0, 0, w, s(TITLE_H));
    let close_btn = Rect::new(
        w - s(10.0) - s(30.0),
        s((TITLE_H - 28.0) / 2.0),
        s(30.0),
        s(28.0),
    );

    let sub = Rect::new(inner_x, s(TITLE_H) + s(4.0), inner_w, s(18.0));

    let text_box = Rect::new(inner_x, s(70.0), inner_w, s(230.0));
    // 内缩 1px 让父窗口画的描边露出来（EDIT 是不透明的白色，会盖住边框）
    let text_edit = Rect::new(text_box.x + 1, text_box.y + 1, text_box.w - 2, text_box.h - 2);

    let hint = Rect::new(inner_x, text_box.bottom() + s(10.0), inner_w, s(34.0));

    let btn_h = s(32.0);
    let half = (inner_w - s(10.0)) / 2;
    let later = Rect::new(inner_x, h - s(PAD) - btn_h, half, btn_h);
    let update = Rect::new(inner_x + half + s(10.0), h - s(PAD) - btn_h, half, btn_h);

    Geo {
        w,
        h,
        title,
        close_btn,
        sub,
        text_box,
        text_edit,
        hint,
        later,
        update,
    }
}

struct UpdDlg {
    hwnd: HWND,
    hinst: HINSTANCE,
    scale: f32,
    fonts: Fonts,
    backdrop: Option<Backdrop>,
    geo: Geo,
    current: String,
    latest: String,
    body: String,
    /// `Some(true)` = 用户点了「立即更新」
    result: Option<bool>,
}

pub fn register(hinst: HINSTANCE) -> bool {
    unsafe {
        let class = to_wide(UPD_DLG_CLASS);
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
            crate::tlog!("注册更新对话框窗口类失败");
            return false;
        }
    }
    true
}

/// 打开对话框（模态）。返回 `true` 表示用户选择「立即更新」。
pub fn open(
    main: HWND,
    hinst: HINSTANCE,
    dpi: u32,
    current: &str,
    latest: &str,
    body: &str,
) -> bool {
    let scale = dpi as f32 / 96.0;
    let fonts = Fonts::new(dpi);
    let geo = build_geo(scale);

    let dlg = Box::new(UpdDlg {
        hwnd: HWND::default(),
        hinst,
        scale,
        fonts,
        backdrop: None,
        geo,
        current: current.to_string(),
        latest: latest.to_string(),
        body: body.to_string(),
        result: None,
    });
    let ptr = Box::into_raw(dlg);

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

    let title = to_wide(&format!("发现新版本 v{latest}"));
    // 提前绑定：直接写 PCWSTR(to_wide(..).as_ptr()) 会让临时 Vec 在语句末尾释放
    let class_w = to_wide(UPD_DLG_CLASS);
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
            crate::tlog!("创建更新对话框失败");
            drop(unsafe { Box::from_raw(ptr) });
            return false;
        }
    };

    unsafe {
        let _ = EnableWindow(main, false);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }

    let mut msg = MSG::default();
    unsafe {
        while IsWindow(Some(hwnd)).as_bool() {
            let ret = GetMessageW(&mut msg, None, 0, 0);
            if ret.0 <= 0 {
                break;
            }
            // Enter / Esc 自己处理：按钮是自绘的，没有 BS_DEFPUSHBUTTON，
            // IsDialogMessage 不会替我们把 Enter 变成「确定」。
            if msg.message == WM_KEYDOWN {
                let key = msg.wParam.0 as u16;
                if key == VK_ESCAPE.0 {
                    let _ = DestroyWindow(hwnd);
                    continue;
                }
                if key == VK_RETURN.0 {
                    let d = &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut UpdDlg);
                    d.result = Some(true);
                    let _ = DestroyWindow(hwnd);
                    continue;
                }
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
    dlg.result.unwrap_or(false)
}

// ---------------------------------------------------------------------------
// 窗口过程
// ---------------------------------------------------------------------------

unsafe extern "system" fn dlg_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        let dlg = &mut *(cs.lpCreateParams as *mut UpdDlg);
        dlg.hwnd = hwnd;
        return LRESULT(1);
    }

    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut UpdDlg;
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
            if code == 0 {
                match id {
                    ID_LATER | ID_CLOSE => {
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
                    }
                    ID_UPDATE => {
                        dlg.result = Some(true);
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
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

        // 只读多行 EDIT 走的是 WM_CTLCOLORSTATIC。
        // **必须回实心画刷**：回 NULL_BRUSH 等于不擦背景，滚动时旧文字会留在原地
        // 和新文字叠在一起（`dlg.rs` 踩过同一个坑，见那里的注释）。
        WM_CTLCOLORSTATIC => {
            let hdc = HDC(wparam.0 as *mut c_void);
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, COLORREF(colorref(theme::INPUT_TEXT)));
            LRESULT(GetStockObject(WHITE_BRUSH).0 as isize)
        }

        WM_WINDOWPOSCHANGED => {
            sync_backdrop(dlg);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// 子控件
// ---------------------------------------------------------------------------

fn create_children(dlg: &mut UpdDlg) {
    // 正文：只读多行 EDIT。ES_AUTOVSCROLL + WS_VSCROLL 提供原生滚动条，
    // 滚轮和 PgUp/PgDn 由 EDIT 自己处理，不用写一行代码。
    let text = create_child(
        dlg,
        "EDIT",
        WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WS_VSCROLL
            | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32),
        ID_TEXT,
        dlg.geo.text_edit,
    );
    let body = to_wide(&dlg.body);
    unsafe {
        let _ = SetWindowTextW(text, PCWSTR(body.as_ptr()));
        SendMessageW(text, WM_SETFONT, Some(WPARAM(dlg.fonts.body.0 as usize)), Some(LPARAM(1)));
        // 左右各留 8px 内边距，文字不要贴着边框
        SendMessageW(
            text,
            EM_SETMARGINS,
            Some(WPARAM(0x0003)), // EC_LEFTMARGIN | EC_RIGHTMARGIN
            Some(LPARAM((8 | (8 << 16)) as isize)),
        );
    }

    let later = create_child(
        dlg,
        "BUTTON",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(0x000B), // BS_OWNERDRAW
        ID_LATER,
        dlg.geo.later,
    );
    let update = create_child(
        dlg,
        "BUTTON",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(0x000B),
        ID_UPDATE,
        dlg.geo.update,
    );
    let close = create_child(
        dlg,
        "BUTTON",
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x000B),
        ID_CLOSE,
        dlg.geo.close_btn,
    );
    unsafe {
        for h in [later, update, close] {
            SendMessageW(h, WM_SETFONT, Some(WPARAM(dlg.fonts.body.0 as usize)), Some(LPARAM(1)));
        }
    }
}

fn create_child(dlg: &UpdDlg, class: &str, style: WINDOW_STYLE, id: usize, r: Rect) -> HWND {
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
                crate::tlog!("更新对话框控件创建失败 id={} : {}", id, err.message());
                HWND::default()
            }
        }
    }
}

fn create_backdrop(dlg: &mut UpdDlg) {
    dlg.backdrop = Backdrop::new(dlg.hinst, dlg.scale, (dlg.geo.w, dlg.geo.h));
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

fn sync_backdrop(dlg: &UpdDlg) {
    if let Some(bd) = dlg.backdrop.as_ref() {
        let visible = unsafe { IsWindowVisible(dlg.hwnd).as_bool() };
        bd.sync(dlg.hwnd, visible);
    }
}

// ---------------------------------------------------------------------------
// 绘制
// ---------------------------------------------------------------------------

fn paint(dlg: &UpdDlg, hdc: HDC) {
    let Some(canvas) = Canvas::from_hdc(hdc) else {
        return;
    };
    let g = &dlg.geo;
    let f = &dlg.fonts;

    canvas.fill_rect(Rect::new(0, 0, g.w, g.h), rgb(theme::CARD_BG));

    canvas.text(
        &format!("发现新版本 v{}", dlg.latest),
        Rect::new((PAD * dlg.scale) as i32, g.title.y, g.title.w - 100, g.title.h),
        f.title,
        rgb(theme::TITLE_TEXT),
        ALIGN_LEFT,
    );

    canvas.text(
        &format!("当前版本 v{}，下面是服务端上的全部版本记录", dlg.current),
        g.sub,
        f.small,
        rgb(theme::SWITCH_DESC),
        ALIGN_LEFT,
    );

    // 文本区外框
    canvas.fill_round_rect(g.text_box, 6.0 * dlg.scale, rgb(theme::INPUT_BG));
    if let Some(path) = crate::gfx::Path::round_rect(g.text_box, 6.0 * dlg.scale) {
        canvas.stroke_path(&path, rgb(theme::INPUT_BORDER), 1.0);
    }

    canvas.text(
        "更新完成后程序会自动重启，\n账户密码与开机自启设置保持不变。",
        g.hint,
        f.small,
        rgb(theme::SWITCH_DESC),
        ALIGN_LEFT,
    );
}

fn draw_button(dlg: &UpdDlg, dis: &DRAWITEMSTRUCT) {
    let id = dis.CtlID as usize;
    let rc = dis.rcItem;
    let r = Rect::new(rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top);
    let selected = (dis.itemState.0 & ODS_SELECTED.0) != 0;
    let Some(canvas) = Canvas::from_hdc(dis.hDC) else {
        return;
    };
    canvas.fill_rect(r, rgb(theme::CARD_BG));

    let (kind, text) = match id {
        ID_UPDATE => (BtnKind::Primary, "立即更新"),
        ID_LATER => (BtnKind::Secondary, "稍后"),
        ID_CLOSE => (BtnKind::WinClose, "✕"),
        _ => return,
    };

    paint_button(
        &canvas,
        &dlg.fonts,
        Rect::new(0, 0, r.w, r.h),
        kind,
        text,
        dlg.scale,
        false,
        selected,
        false,
    );
}

// ---------------------------------------------------------------------------
// 正文拼装（对外暴露，方便预览与测试）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geo_keeps_text_area_inside_card() {
        let g = build_geo(1.0);
        assert!(g.text_box.x > 0 && g.text_box.right() < g.w);
        assert!(g.text_box.bottom() < g.hint.y, "说明文字要在文本区下方");
        assert!(g.hint.bottom() < g.later.y, "按钮要在说明文字下方");
        assert!(g.later.right() < g.update.x, "两个按钮不能重叠");
    }
}
