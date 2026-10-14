//! 窗口类注册与消息处理。

use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::{
    DRAWITEMSTRUCT, MEASUREITEMSTRUCT, WM_MOUSELEAVE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{self, App, WM_RESET_DONE, WM_TRAY};
use crate::tlog;
use crate::icon;
use crate::model::Widget;
use crate::theme;
use crate::util::{colorref, from_wide, to_wide};

const EN_SETFOCUS: u16 = 0x0100;
const EN_KILLFOCUS: u16 = 0x0200;
const EN_CHANGE: u16 = 0x0300;
const BN_CLICKED: u16 = 0;

fn loword(v: usize) -> u16 {
    (v & 0xFFFF) as u16
}
fn hiword(v: usize) -> u16 {
    ((v >> 16) & 0xFFFF) as u16
}

fn widget_of(id: usize) -> Option<Widget> {
    match id {
        app::ID_MIN => Some(Widget::Min),
        app::ID_CLOSE => Some(Widget::Close),
        app::ID_RESET => Some(Widget::Reset),
        app::ID_START => Some(Widget::Start),
        app::ID_STOP => Some(Widget::Stop),
        app::ID_TOGGLE => Some(Widget::Toggle),
        app::ID_EYE => Some(Widget::Eye),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 窗口类注册
// ---------------------------------------------------------------------------

pub fn register_all(hinst: HINSTANCE) -> bool {
    unsafe {
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let hand = LoadCursorW(None, IDC_HAND).unwrap_or_default();

        // 主窗口
        let main_class = to_wide(app::MAIN_CLASS);
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(main_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: HICON::default(),
            hCursor: cursor,
            hbrBackground: HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: PCWSTR(main_class.as_ptr()),
        };
        if RegisterClassW(&wc) == 0 {
            crate::tlog!("注册主窗口类失败");
            return false;
        }
        crate::tlog!("注册主窗口类 ok");

        // 自绘子控件：按钮 / 开关用同一个过程，眼睛单独一套（手型光标）
        for (class, proc_) in [
            (app::BTN_CLASS, widget_wndproc as unsafe extern "system" fn(_, _, _, _) -> _),
            (app::TOGGLE_CLASS, widget_wndproc as unsafe extern "system" fn(_, _, _, _) -> _),
            (app::EYE_CLASS, widget_wndproc as unsafe extern "system" fn(_, _, _, _) -> _),
        ] {
            let name = to_wide(class);
            let child = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(proc_),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinst,
                hIcon: HICON::default(),
                hCursor: if class == app::EYE_CLASS { hand } else { hand },
                hbrBackground: HBRUSH::default(),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: PCWSTR(name.as_ptr()),
            };
            if RegisterClassW(&child) == 0 {
                crate::tlog!("注册子控件类失败 class={}", class);
                return false;
            }
            crate::tlog!("注册子控件类 ok class={}", class);
        }
    }
    true
}

// ---------------------------------------------------------------------------
// 主窗口
// ---------------------------------------------------------------------------

unsafe extern "system" fn main_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // 创建前先把 App 挂上去，后续消息才能拿到状态
    if msg == WM_NCCREATE {
        tlog!("WM_NCCREATE");
        let hinst = app::main_instance();
        let dpi = app::window_dpi(hwnd);
        let hicon_big = icon::create_hicon(32).unwrap_or_default();
        let hicon_small = icon::create_hicon(16).unwrap_or_default();
        tlog!("图标已生成");

        let boxed = App::new(hwnd, hinst, dpi, hicon_big);
        tlog!("App 已构造");
        let ptr = Box::into_raw(boxed);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);

        SendMessageW(hwnd, WM_SETICON, Some(WPARAM(ICON_BIG as usize)), Some(LPARAM(hicon_big.0 as isize)));
        SendMessageW(hwnd, WM_SETICON, Some(WPARAM(ICON_SMALL as usize)), Some(LPARAM(hicon_small.0 as isize)));
        tlog!("WM_NCCREATE 返回");
        return LRESULT(1);
    }

    let app = app::app_ref(hwnd);

    match msg {
        WM_CREATE => {
            tlog!("WM_CREATE");
            if let Some(app) = app {
                app.init_ui();
                tlog!("init_ui 完成");
                app.layout_children();
                tlog!("layout_children 完成");
            }
            LRESULT(0)
        }

        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if let Some(app) = app {
                app.paint(hdc);
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
            if let Some(app) = app {
                // 标题栏区域交给系统做原生拖动（含 Aero Snap）
                if app.layout.title_bar.contains(pt.x, pt.y) {
                    return LRESULT(HTCAPTION as isize);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        // 固定尺寸窗口：吞掉标题栏双击，避免被最大化
        WM_NCLBUTTONDBLCLK => {
            if wparam.0 == HTCAPTION as usize {
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_MOUSEMOVE => {
            if let Some(app) = app {
                app.refresh_hover_from_cursor();
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
                app.repaint();
            }
            LRESULT(0)
        }

        WM_MOUSELEAVE => {
            if let Some(app) = app {
                app.clear_hover();
                app.repaint();
            }
            LRESULT(0)
        }

        // 点到输入框的留白区域时，把焦点转给里面的原生 EDIT
        WM_LBUTTONDOWN => {
            if let Some(app) = app {
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                if app.layout.account_field.contains(x, y) {
                    let _ = SetFocus(Some(app.h_account));
                } else if app.layout.password_field.contains(x, y) {
                    let _ = SetFocus(Some(app.h_password));
                }
            }
            LRESULT(0)
        }

        WM_TIMER => {
            if wparam.0 == 99 {
                // 自检模式的自动退出
                tlog!("自检计时到，退出");
                unsafe { PostQuitMessage(0) };
                return LRESULT(0);
            }
            if let Some(app) = app {
                match wparam.0 {
                    1 => app.on_poll(),
                    2 => app.on_login_timer(),
                    3 => app.on_verify_timer(),
                    4 => app.on_anim_timer(),
                    _ => {}
                }
            }
            LRESULT(0)
        }

        WM_COMMAND => {
            if let Some(app) = app {
                let id = loword(wparam.0);
                let code = hiword(wparam.0);
                match id as usize {
                    app::ID_ACCOUNT | app::ID_PASSWORD => {
                        let is_account = id as usize == app::ID_ACCOUNT;
                        match code {
                            EN_SETFOCUS => {
                                if is_account {
                                    app.account_focus = true;
                                } else {
                                    app.password_focus = true;
                                }
                                app.repaint();
                            }
                            EN_KILLFOCUS => {
                                if is_account {
                                    app.account_focus = false;
                                } else {
                                    app.password_focus = false;
                                }
                                read_fields(app);
                                app.repaint();
                            }
                            EN_CHANGE => read_fields(app),
                            _ => {}
                        }
                    }
                    app::ID_RESET if code == BN_CLICKED => app.on_reset_password(),
                    app::ID_START if code == BN_CLICKED => app.enable_auto_login(),
                    app::ID_STOP if code == BN_CLICKED => app.disable_auto_login(),
                    app::ID_MIN | app::ID_CLOSE if code == BN_CLICKED => app.hide_to_tray(),
                    app::ID_TOGGLE if code == BN_CLICKED => {
                        let next = !app.auto_start;
                        app.set_auto_start(next);
                    }
                    app::ID_EYE if code == BN_CLICKED => app.toggle_password_visible(),
                    _ => {}
                }
            }
            LRESULT(0)
        }

        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            if let Some(app) = app {
                let hdc = HDC(wparam.0 as *mut c_void);
                let ctrl = HWND(lparam.0 as *mut c_void);
                let is_password = ctrl == app.h_password;
                let is_account = ctrl == app.h_account;
                if is_password || is_account {
                    let enabled = app.login_state == crate::model::LoginState::Stopped;
                    SetBkMode(hdc, TRANSPARENT);
                    SetTextColor(hdc, app::text_color(enabled));
                    let brush = if enabled {
                        app.brush_white
                    } else {
                        app.brush_disabled
                    };
                    return LRESULT(brush.0 as isize);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_MEASUREITEM => {
            if let Some(app) = app {
                let mis = &mut *(lparam.0 as *mut MEASUREITEMSTRUCT);
                app.tray.measure_item(mis);
            }
            LRESULT(1)
        }

        WM_DRAWITEM => {
            if let Some(app) = app {
                let dis = &*(lparam.0 as *const DRAWITEMSTRUCT);
                app.tray.draw_item(dis);
            }
            LRESULT(1)
        }

        WM_TRAY => {
            if let Some(app) = app {
                match lparam.0 as u32 {
                    WM_LBUTTONUP | WM_LBUTTONDBLCLK => app.show_from_tray(),
                    WM_RBUTTONUP => {
                        let cmd = app.tray.popup();
                        match cmd {
                            app::IDM_TOGGLE => app.tray_toggle(),
                            app::IDM_SHOW => app.show_from_tray(),
                            app::IDM_EXIT => app.on_exit(),
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }

        WM_RESET_DONE => {
            if let Some(app) = app {
                app.on_reset_done();
            }
            LRESULT(0)
        }

        WM_DPICHANGED => {
            if let Some(app) = app {
                let new_dpi = hiword(wparam.0) as u32;
                let rect = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                app.rebuild_for_dpi(new_dpi);
                let font = app.fonts.body;
                app.tray.set_font(font);
            }
            LRESULT(0)
        }

        WM_CLOSE => {
            if let Some(app) = app {
                // 关闭只最小化到托盘，不退出进程
                app.hide_to_tray();
            }
            LRESULT(0)
        }

        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }

        WM_NCDESTROY => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
            if !ptr.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ptr));
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        _ => {
            // 资源管理器重启后需要重新添加托盘图标
            if let Some(app) = app {
                if msg == app.taskbar_created_msg && app.taskbar_created_msg != 0 {
                    let icon = app.hicon;
                    app.tray.added = false;
                    app.tray.show(icon);
                    return LRESULT(0);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}

fn read_fields(app: &mut App) {
    // 初始化期间正在写初始文本，此刻回读会读到还没填好的另一个控件
    if app.syncing {
        return;
    }
    app.account = get_window_text(app.h_account);
    app.password = get_window_text(app.h_password);
}

fn get_window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if len <= 0 {
        String::new()
    } else {
        from_wide(&buf[..len as usize])
    }
}

// ---------------------------------------------------------------------------
// 自绘子控件
// ---------------------------------------------------------------------------

unsafe extern "system" fn widget_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // WM_NCCREATE 必须返回 TRUE，否则窗口创建会被直接中断（连 WM_CREATE 都不会发出）。
    // 实测本机把该消息原样交给 DefWindowProc 会拿到 0，导致所有自绘子控件创建失败，
    // 因此这里显式返回 TRUE，同时仍调用一次 DefWindowProc 完成默认初始化。
    if msg == WM_NCCREATE {
        DefWindowProcW(hwnd, msg, wparam, lparam);
        return LRESULT(1);
    }

    let id = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize;
    let widget = widget_of(id);

    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);

            let mem = CreateCompatibleDC(Some(hdc));
            let bmp = CreateCompatibleBitmap(hdc, w.max(1), h.max(1));
            let old = SelectObject(mem, HGDIOBJ(bmp.0));
            if let Some(app) = app::app_ref(hwnd) {
                app.paint_widget(id, mem, w, h);
            }
            let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
            SelectObject(mem, old);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
            let _ = DeleteDC(mem);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }

        WM_ERASEBKGND => LRESULT(1),

        WM_MOUSEMOVE => {
            if let Some(app) = app::app_ref(hwnd) {
                app.set_hover(widget);
                app.repaint_widgets();
            }
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
            if let Some(app) = app::app_ref(hwnd) {
                if app.hover == widget {
                    app.set_hover(None);
                }
                app.repaint_widgets();
            }
            LRESULT(0)
        }

        WM_LBUTTONDOWN => {
            if let Some(app) = app::app_ref(hwnd) {
                app.pressed = widget;
                app.repaint_widgets();
            }
            let _ = SetCapture(hwnd);
            LRESULT(0)
        }

        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            if let Some(app) = app::app_ref(hwnd) {
                let inside = {
                    let pt = POINT {
                        x: (lparam.0 & 0xFFFF) as i16 as i32,
                        y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                    };
                    let mut rc = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rc);
                    let _ = pt;
                    rc.left <= pt.x && pt.x < rc.right && rc.top <= pt.y && pt.y < rc.bottom
                };
                let was_pressed = app.pressed == widget;
                app.pressed = None;
                app.repaint_widgets();
                if was_pressed && inside {
                    match widget {
                        // 开关与眼睛直接改状态，其余转发 WM_COMMAND 给主窗口
                        Some(Widget::Toggle) => {
                            let next = !app.auto_start;
                            app.set_auto_start(next);
                        }
                        Some(Widget::Eye) => app.toggle_password_visible(),
                        _ => {
                            let parent = GetParent(hwnd).unwrap_or_default();
                            let _ = SendMessageW(
                                parent,
                                WM_COMMAND,
                                Some(WPARAM(id)),
                                Some(LPARAM(hwnd.0 as isize)),
                            );
                        }
                    }
                }
            }
            LRESULT(0)
        }

        WM_SETCURSOR => {
            let hand = LoadCursorW(None, IDC_HAND).unwrap_or_default();
            SetCursor(Some(hand));
            LRESULT(1)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// 让 `theme` 常量在此模块内可用（避免未使用告警）
#[allow(dead_code)]
fn _keep() -> u32 {
    colorref(theme::CARD_BG)
}
