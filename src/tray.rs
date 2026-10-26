//! 系统托盘图标与右键菜单。
//!
//! 菜单项用自绘（MF_OWNERDRAW）还原原版 QSS 里的白底 / 圆角 / 蓝色高亮观感。

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_DISABLED, ODS_GRAYED, ODS_SELECTED};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{IDM_CHECK_UPDATE, IDM_EXIT, IDM_SHOW, IDM_STATUS, IDM_TOGGLE, WM_TRAY};
use crate::gfx::{draw_text, Canvas, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER};
use crate::theme;
use crate::util::{colorref, from_wide, to_wide, Rect};

const TRAY_UID: u32 = 1;

pub struct Tray {
    pub hwnd: HWND,
    pub uid: u32,
    pub menu: HMENU,
    pub added: bool,
    pub icon: HICON,
    pub status_text: String,
    pub running: bool,
    pub font: HFONT,
}

impl Tray {
    pub fn new(hwnd: HWND, hicon: HICON) -> Self {
        let mut tray = Tray {
            hwnd,
            uid: TRAY_UID,
            menu: HMENU::default(),
            added: false,
            icon: hicon,
            status_text: "已停止".into(),
            running: false,
            font: HFONT::default(),
        };
        tray.build_menu();
        tray
    }

    /// 菜单字体由主窗口提供（与正文同一套字体，保证 DPI 变化后同步）
    pub fn set_font(&mut self, font: HFONT) {
        self.font = font;
    }

    fn build_menu(&mut self) {
        unsafe {
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            self.menu = menu;

            // 菜单背景刷（白底）
            let brush = CreateSolidBrush(COLORREF(colorref(theme::MENU_BG)));
            let info = MENUINFO {
                cbSize: std::mem::size_of::<MENUINFO>() as u32,
                fMask: MIM_BACKGROUND,
                dwStyle: MENUINFO_STYLE(0),
                cyMax: 0,
                hbrBack: brush,
                dwContextHelpID: 0,
                dwMenuData: 0,
            };
            let _ = SetMenuInfo(menu, &info);

            // 自绘项：lpNewItem 位置用来携带命令 ID，WM_DRAWITEM 里再取回
            let _ = AppendMenuW(menu, MF_OWNERDRAW | MF_DISABLED, IDM_STATUS, PCWSTR(IDM_STATUS as *const u16));
            let _ = AppendMenuW(menu, MF_OWNERDRAW | MF_DISABLED, 0, PCWSTR(std::ptr::null()));
            let _ = AppendMenuW(menu, MF_OWNERDRAW, IDM_TOGGLE, PCWSTR(IDM_TOGGLE as *const u16));
            let _ = AppendMenuW(menu, MF_OWNERDRAW | MF_DISABLED, 0, PCWSTR(std::ptr::null()));
            let _ = AppendMenuW(menu, MF_OWNERDRAW, IDM_CHECK_UPDATE, PCWSTR(IDM_CHECK_UPDATE as *const u16));
            let _ = AppendMenuW(menu, MF_OWNERDRAW, IDM_SHOW, PCWSTR(IDM_SHOW as *const u16));
            let _ = AppendMenuW(menu, MF_OWNERDRAW, IDM_EXIT, PCWSTR(IDM_EXIT as *const u16));
        }
    }

    fn notify_data(&self, flags: NOTIFY_ICON_DATA_FLAGS) -> NOTIFYICONDATAW {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: self.uid,
            uFlags: flags,
            uCallbackMessage: WM_TRAY,
            hIcon: self.icon,
            ..Default::default()
        };
        let tip = to_wide(&self.tip());
        let n = tip.len().min(nid.szTip.len());
        nid.szTip[..n].copy_from_slice(&tip[..n]);
        nid
    }

    fn tip(&self) -> String {
        format!("AutoLoginQatarTool\n程序状态: {}", self.status_text)
    }

    pub fn show(&mut self, hicon: HICON) {
        self.icon = hicon;
        let nid = self.notify_data(NIF_ICON | NIF_MESSAGE | NIF_TIP);
        let action = if self.added { NIM_MODIFY } else { NIM_ADD };
        let ok = unsafe { Shell_NotifyIconW(action, &nid) };
        crate::tlog!("托盘图标添加/更新 -> {}", ok.as_bool());
        self.added = true;
    }

    pub fn set_tip(&mut self, _tip: &str) {
        if !self.added {
            return;
        }
        let nid = self.notify_data(NIF_TIP);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    /// 弹一个托盘气泡。用于「密码已复制」「密码已自动重置」这类
    /// 不该打断用户、但必须让他知道的反馈。
    pub fn balloon(&self, title: &str, text: &str) {
        if !self.added {
            return;
        }
        let mut nid = self.notify_data(NIF_INFO);
        nid.dwInfoFlags = NIIF_INFO;
        let t = to_wide(title);
        let n = t.len().saturating_sub(1).min(nid.szInfoTitle.len() - 1);
        nid.szInfoTitle[..n].copy_from_slice(&t[..n]);
        let m = to_wide(text);
        let n = m.len().saturating_sub(1).min(nid.szInfo.len() - 1);
        nid.szInfo[..n].copy_from_slice(&m[..n]);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    pub fn set_status(&mut self, text: &str, running: bool) {
        self.status_text = text.to_string();
        self.running = running;
        self.set_tip("");
    }

    pub fn remove(&mut self) {
        if !self.added {
            return;
        }
        let nid = self.notify_data(NIF_ICON | NIF_MESSAGE | NIF_TIP);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        }
        self.added = false;
    }

    /// 弹出右键菜单，返回被选中的命令 ID（0 表示取消）
    pub fn popup(&self) -> usize {
        let mut pt = POINT::default();
        unsafe {
            if GetCursorPos(&mut pt).is_err() {
                return 0;
            }
            // 弹出前先抢前台，否则菜单不会在点击别处时自动消失
            let _ = SetForegroundWindow(self.hwnd);
            let cmd = TrackPopupMenu(
                self.menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                pt.x,
                pt.y,
                None,
                self.hwnd,
                None,
            );
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            cmd.0 as usize
        }
    }

    /// 某个菜单项的显示文本
    pub fn item_text(&self, id: usize) -> String {
        match id {
            IDM_STATUS => format!("当前状态：{}", self.status_text),
            IDM_TOGGLE => {
                if self.running {
                    "停止".into()
                } else {
                    "启动".into()
                }
            }
            IDM_CHECK_UPDATE => "检查更新".into(),
            IDM_SHOW => "显示主界面".into(),
            IDM_EXIT => "退出".into(),
            _ => String::new(),
        }
    }

    /// 自绘菜单项的尺寸
    pub fn measure_item(&self, mis: &mut MEASUREITEMSTRUCT) {
        let scale = 1.0;
        mis.itemHeight = match mis.itemData {
            0 => (9.0 * scale) as u32,
            IDM_STATUS => (28.0 * scale) as u32,
            _ => (30.0 * scale) as u32,
        };
        mis.itemWidth = (150.0 * scale) as u32;
    }

    /// 自绘菜单项
    pub fn draw_item(&self, dis: &DRAWITEMSTRUCT) {
        let rc = dis.rcItem;
        let rect = Rect::new(rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top);
        let selected = (dis.itemState.0 & ODS_SELECTED.0) != 0;
        let disabled = (dis.itemState.0 & (ODS_DISABLED.0 | ODS_GRAYED.0)) != 0;

        if dis.itemData == 0 {
            // 分隔线
            let y = rect.cy();
            if let Some(canvas) = Canvas::from_hdc(dis.hDC) {
                canvas.line(
                    (rect.x + 10) as f32,
                    y as f32,
                    (rect.right() - 10) as f32,
                    y as f32,
                    rgba_from(theme::MENU_SEPARATOR),
                    1.0,
                );
            }
            return;
        }

        if selected && !disabled {
            if let Some(canvas) = Canvas::from_hdc(dis.hDC) {
                canvas.fill_round_rect(
                    Rect::new(rect.x + 4, rect.y + 1, rect.w - 8, rect.h - 2),
                    6.0,
                    rgba_from(theme::MENU_SEL_BG),
                );
            }
        }

        let fg = if disabled {
            theme::MENU_DISABLED_FG
        } else if selected {
            theme::MENU_SEL_FG
        } else {
            theme::MENU_FG
        };
        let text_rect = Rect::new(rect.x + 14, rect.y, rect.w - 20, rect.h);
        draw_text(
            dis.hDC,
            &self.item_text(dis.itemData),
            text_rect,
            self.font,
            fg,
            DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_NOPREFIX,
        );
    }
}

fn rgba_from(hex: u32) -> u32 {
    crate::util::rgba(
        ((hex >> 16) & 0xFF) as u8,
        ((hex >> 8) & 0xFF) as u8,
        (hex & 0xFF) as u8,
        255,
    )
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
        unsafe {
            if !self.menu.0.is_null() {
                let _ = DestroyMenu(self.menu);
            }
        }
    }
}

/// 从宽字符缓冲读取菜单标题（备用）
#[allow(dead_code)]
pub fn read_menu_title(buf: &[u16]) -> String {
    from_wide(buf)
}
