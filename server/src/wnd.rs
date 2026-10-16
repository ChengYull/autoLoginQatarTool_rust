//! 窗口类注册与消息处理。

use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{self, App, BTN_CLASS, MAIN_CLASS};
use crate::icon;
use crate::model::Widget;
use crate::theme;
use crate::util::{colorref, to_wide};

const EN_SETFOCUS: u16 = 0x0100;
const EN_KILLFOCUS: u16 = 0x0200;
const BN_CLICKED: u16 = 0;

fn loword(v: usize) -> u16 {
    (v & 0xFFFF) as u16
}
fn hiword(v: usize) -> u16 {
    ((v >> 16) & 0xFFFF) as u16
}

// ---------------------------------------------------------------------------
// 窗口类注册
// ---------------------------------------------------------------------------

pub fn register_all(hinst: HINSTANCE) -> bool {
    unsafe {
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();

        let main_class = to_wide(MAIN_CLASS);
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(main_wndproc),
            hInstance: hinst,
            // 窗口类上就带好图标，省掉一次 WM_SETICON
            hIcon: icon::create_hicon(32).unwrap_or_default(),
            hCursor: cursor,
            hbrBackground: HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: PCWSTR(main_class.as_ptr()),
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            crate::tlog!("注册主窗口类失败");
            return false;
        }

        let btn_class = to_wide(BTN_CLASS);
        let child = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(widget_wndproc),
            hInstance: hinst,
            hCursor: cursor,
            hbrBackground: HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: PCWSTR(btn_class.as_ptr()),
            ..Default::default()
        };
        if RegisterClassW(&child) == 0 {
            crate::tlog!("注册子控件类失败 class={BTN_CLASS}");
            return false;
        }

        if !crate::backdrop::register(hinst) {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// 主窗口
// ---------------------------------------------------------------------------

unsafe extern "system" fn main_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // 创建前先把 App 挂上去，后续消息才能拿到状态
    if msg == WM_NCCREATE {
        let hinst = app::main_instance();
        let dpi = app::window_dpi(hwnd);
        let boxed = App::new(hwnd, hinst, dpi, app::data_dir());
        let ptr = Box::into_raw(boxed);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
        // 这个返回值必须显式给 TRUE：DefWindowProc 对 WM_NCCREATE 返回 0 时
        // 窗口创建会被直接中断，WM_CREATE 根本不发出，且 GetLastError 也是 0。
        return LRESULT(1);
    }

    let app = app::app_ref(hwnd);

    match msg {
        WM_CREATE => {
            if let Some(app) = app {
                app.init_ui();
                app.layout_children();
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
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                app.on_mouse_move(x, y);
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
            if let Some(app) = app {
                app.on_mouse_leave();
            }
            LRESULT(0)
        }

        WM_LBUTTONDOWN => {
            if let Some(app) = app {
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                app.on_lbutton_down(x, y);
            }
            LRESULT(0)
        }

        WM_MOUSEWHEEL => {
            if let Some(app) = app {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as i16 as i32;
                app.on_mouse_wheel(delta);
            }
            LRESULT(0)
        }

        WM_TIMER => {
            if wparam.0 == 99 {
                // 自检模式的自动退出
                crate::tlog!("自检计时到，退出");
                PostQuitMessage(0);
                return LRESULT(0);
            }
            if let Some(app) = app {
                if wparam.0 == 1 {
                    app.on_timer();
                }
            }
            LRESULT(0)
        }

        WM_COMMAND => {
            if let Some(app) = app {
                let id = loword(wparam.0) as usize;
                let code = hiword(wparam.0);
                match id {
                    app::ID_ADDR => match code {
                        EN_SETFOCUS => {
                            app.addr_focus = true;
                            app.repaint();
                        }
                        EN_KILLFOCUS => {
                            app.addr_focus = false;
                            app.repaint();
                        }
                        _ => {}
                    },
                    app::ID_START if code == BN_CLICKED => app.start_server(),
                    app::ID_STOP if code == BN_CLICKED => app.stop_server(),
                    app::ID_PUBLISH if code == BN_CLICKED => app.on_publish(),
                    app::ID_LATEST if code == BN_CLICKED => app.on_set_latest(),
                    app::ID_DELETE if code == BN_CLICKED => app.on_delete(),
                    app::ID_OPENDIR if code == BN_CLICKED => app.on_open_data_dir(),
                    app::ID_MIN if code == BN_CLICKED => {
                        let _ = ShowWindow(hwnd, SW_MINIMIZE);
                    }
                    app::ID_CLOSE if code == BN_CLICKED => {
                        let _ = SendMessageW(hwnd, WM_CLOSE, None, None);
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }

        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            if let Some(app) = app {
                let hdc = HDC(wparam.0 as *mut c_void);
                let ctrl = HWND(lparam.0 as *mut c_void);
                if ctrl == app.h_addr {
                    let enabled = !app.running();
                    SetBkMode(hdc, TRANSPARENT);
                    let (text, bg) = if enabled {
                        (theme::INPUT_TEXT, theme::INPUT_BG)
                    } else {
                        (theme::INPUT_DISABLED_TEXT, theme::INPUT_DISABLED_BG)
                    };
                    SetTextColor(hdc, COLORREF(colorref(text)));
                    return LRESULT(colorref(bg) as isize);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
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
            }
            LRESULT(0)
        }

        // 位置 / 尺寸 / 显隐 / z 序任何一项变化，背景层都要跟着走
        WM_WINDOWPOSCHANGED => {
            if let Some(app) = app {
                app.sync_backdrop();
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_CLOSE => {
            // 服务随进程一起停（Server 的 Drop 会通知监听线程退出）
            let _ = DestroyWindow(hwnd);
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

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// 自绘子控件（按钮）
// ---------------------------------------------------------------------------

unsafe extern "system" fn widget_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // 显式返回 TRUE，理由同主窗口
        WM_NCCREATE => LRESULT(1),

        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if let Some(app) = app::root_app(hwnd) {
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let id = GetDlgCtrlID(hwnd) as usize;
                app.paint_widget(id, hdc, rc.right - rc.left, rc.bottom - rc.top);
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }

        WM_ERASEBKGND => LRESULT(1),

        WM_MOUSEMOVE => {
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut tme);
            set_hover(hwnd, true);
            LRESULT(0)
        }

        WM_MOUSELEAVE => {
            set_hover(hwnd, false);
            LRESULT(0)
        }

        WM_LBUTTONDOWN => {
            let id = GetDlgCtrlID(hwnd) as usize;
            if let Some(app) = app::root_app(hwnd) {
                app.pressed = Some(app::widget_of(id));
                app.repaint_child(hwnd);
            }
            let _ = SetCapture(hwnd);
            LRESULT(0)
        }

        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            let id = GetDlgCtrlID(hwnd) as usize;
            if let Some(app) = app::root_app(hwnd) {
                app.pressed = None;
                app.repaint_child(hwnd);
                // 只有指针还在按钮上、且按钮当前可用时才派发点击
                if app.hover == Some(app::widget_of(id)) && app::widget_enabled(app, id) {
                    let parent = GetParent(hwnd).unwrap_or_default();
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

fn set_hover(hwnd: HWND, on: bool) {
    let id = unsafe { GetDlgCtrlID(hwnd) } as usize;
    let widget: Widget = app::widget_of(id);
    if let Some(app) = app::root_app(hwnd) {
        // 清之前要确认当前 hover 的确实是这个控件，否则在两个按钮之间
        // 移动时，先到的 WM_MOUSELEAVE 会把后一个按钮刚设上的 hover 清掉
        if on {
            app.hover = Some(widget);
        } else if app.hover == Some(widget) {
            app.hover = None;
        }
        app.repaint_child(hwnd);
    }
}
