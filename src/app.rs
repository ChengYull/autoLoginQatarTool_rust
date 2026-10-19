//! 主窗口：界面、托盘、状态机与自动登录流程。
//!
//! 对应原版的 `mainwidget.cpp`。差异只在于：Qt 的控件换成了 Win32 原生控件
//! 加 GDI+ 自绘，其余逻辑（状态机、定时器、错误判定、注册表、HTTP）逐条对齐。

use std::ffi::c_void;
use std::sync::mpsc;

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::Controls::EM_SETPASSWORDCHAR;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::automation;
use crate::backdrop::Backdrop;
use crate::gfx::{text_width, Canvas, Fonts};
use crate::icon;
use crate::model::{LoginState, LoginTarget, Widget};
use crate::net::{self, ResetResult};
use crate::paint::{paint_all, paint_button, paint_toggle, VisualState};
use crate::process;
use crate::settings::{self, Settings};
use crate::theme::{self, Layout};
use crate::tray::Tray;
use crate::update::{self, UpdateOutcome};
use crate::util::{colorref, from_wide, to_wide, Rect};
use crate::version::{self, VersionState};

// ---------------------------------------------------------------------------
// 常量
// ---------------------------------------------------------------------------

pub const MAIN_CLASS: &str = "RustAutoLoginMain";
pub const BTN_CLASS: &str = "RustAutoLoginBtn";
pub const TOGGLE_CLASS: &str = "RustAutoLoginToggle";
pub const EYE_CLASS: &str = "RustAutoLoginEye";

pub const APP_NAME: &str = "AutoLoginQatarTool";

// 控件 ID
pub const ID_ACCOUNT: usize = 1001;
pub const ID_PASSWORD: usize = 1002;
pub const ID_EYE: usize = 1003;
pub const ID_RESET: usize = 1004;
pub const ID_TOGGLE: usize = 1005;
pub const ID_START: usize = 1006;
pub const ID_STOP: usize = 1007;
pub const ID_MIN: usize = 1008;
pub const ID_CLOSE: usize = 1009;

// 托盘菜单命令 ID
pub const IDM_STATUS: usize = 2001;
pub const IDM_TOGGLE: usize = 2002;
pub const IDM_SHOW: usize = 2003;
pub const IDM_EXIT: usize = 2004;

// 定时器
const TIMER_POLL: usize = 1;
const TIMER_LOGIN: usize = 2;
const TIMER_VERIFY: usize = 3;
const TIMER_ANIM: usize = 4;
/// 更新检查：先是一次性的"启动后延迟几秒"，之后改成 6 小时一轮
const TIMER_UPDATE: usize = 5;

/// 轮询间隔（毫秒）
const POLL_INTERVAL_MS: u32 = 500;
/// 检测到客户端后延时多久执行登录（等对话框与控件完全就绪）
const LOGIN_DELAY_MS: u32 = 2000;
/// QtPacker 登录结果校验延时与最大校验次数
const PACKER_VERIFY_DELAY_MS: u32 = 1500;
const PACKER_MAX_VERIFY: i32 = 6;
/// 开关动画时长
const TOGGLE_ANIM_MS: u64 = 140;

pub const WM_TRAY: u32 = WM_APP + 1;
pub const WM_RESET_DONE: u32 = WM_APP + 2;
pub const WM_UPDATE_DONE: u32 = WM_APP + 3;
/// 更新包已下载完毕（后台线程 → 界面线程）
pub const WM_UPDATE_READY: u32 = WM_APP + 4;


// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

/// 构建布局：按 DPI 算尺寸之后，还要按**实测文字宽度**摆放标题右侧的版本徽章。
///
/// 必须传已经建好的 `fonts` —— 徽章的起点和宽度分别由标题文字、版本文字的实测宽度决定，
/// 而这两个宽度又随 DPI 变化，所以「先建字体、再算布局」这个顺序不能反。
fn build_layout(fonts: &Fonts, dpi: u32) -> Layout {
    let mut layout = Layout::compute_with(dpi as f32 / 96.0, fonts.body_line_h);
    layout.place_version_badge(
        text_width(theme::APP_TITLE, fonts.title),
        text_width(version::BADGE_TEXT, fonts.badge),
    );
    layout
}

pub struct App {
    pub hwnd: HWND,
    pub hinst: HINSTANCE,
    pub layout: Layout,
    pub fonts: Fonts,

    /// 卡片背景层：逐像素 alpha 的分层窗口，永远贴在主窗口正下方，
    /// 负责抗锯齿的圆角与描边（详见 `backdrop.rs`）。
    pub backdrop: Option<Backdrop>,

    // 子控件
    pub h_account: HWND,
    pub h_password: HWND,
    pub h_eye: HWND,
    pub h_reset: HWND,
    pub h_toggle: HWND,
    pub h_start: HWND,
    pub h_stop: HWND,
    pub h_min: HWND,
    pub h_close: HWND,

    // 业务状态
    pub login_state: LoginState,
    /// 当前版本相对服务端的状态（决定标题栏版本徽章的颜色）。
    /// 自动更新服务端还没接，目前由 [`version::detect_state`] 给初值。
    pub version_state: VersionState,
    pub active_target: LoginTarget,
    pub account: String,
    pub password: String,
    pub auto_start: bool,
    pub password_visible: bool,
    pub packer_verify_count: i32,

    // 交互状态
    pub hover: Option<Widget>,
    pub pressed: Option<Widget>,
    pub account_focus: bool,
    pub password_focus: bool,
    pub account_hover: bool,
    pub password_hover: bool,

    // 开关动画
    pub toggle_pos: f32,
    toggle_from: f32,
    toggle_to: f32,
    toggle_start: u64,

    // 托盘
    pub tray: Tray,
    pub hicon: HICON,
    pub taskbar_created_msg: u32,
    pub tray_tip: String,

    // 重置密码请求
    pub reset_busy: bool,
    reset_rx: Option<mpsc::Receiver<ResetResult>>,

    /// 更新检查的结果回传通道（检查在后台线程里做）
    update_rx: Option<mpsc::Receiver<UpdateOutcome>>,
    /// 最近一次检查的结果。点版本徽章触发更新时要用（版本号、说明、大小）。
    pub update_info: Option<UpdateOutcome>,
    /// 正在下载 / 安装更新
    pub updating: bool,
    /// 鼠标停在版本徽章上（有新版本时徽章可点）
    pub badge_hover: bool,
    /// 更新包下载的结果回传通道
    install_rx: Option<mpsc::Receiver<Result<update::StagedUpdate, String>>>,

    // 画刷
    pub brush_white: HBRUSH,
    pub brush_disabled: HBRUSH,

    /// 初始化期间为 true：此时正在往控件里写初始文本，
    /// 而 SetWindowText 会同步触发 EN_CHANGE 回调回读控件内容，
    /// 中途另一个控件还是空的，会把状态冲掉，故用这个标志屏蔽回调。
    pub syncing: bool,
}

impl App {
    pub fn new(hwnd: HWND, hinst: HINSTANCE, dpi: u32, hicon: HICON) -> Box<Self> {
        let saved = Settings::load();
        // 字体必须先建：布局里输入框的垂直位置依赖正文字体的行高，
        // 版本徽章的摆放还要按实测文字宽度来
        let fonts = Fonts::new(dpi);
        let layout = build_layout(&fonts, dpi);

        let taskbar_created_msg = {
            let name = to_wide("TaskbarCreated");
            unsafe { RegisterWindowMessageW(PCWSTR(name.as_ptr())) }
        };

        let mut app = Box::new(App {
            hwnd,
            hinst,
            layout,
            fonts,
            backdrop: None,
            h_account: HWND::default(),
            h_password: HWND::default(),
            h_eye: HWND::default(),
            h_reset: HWND::default(),
            h_toggle: HWND::default(),
            h_start: HWND::default(),
            h_stop: HWND::default(),
            h_min: HWND::default(),
            h_close: HWND::default(),
            login_state: LoginState::Stopped,
            version_state: version::detect_state(),
            active_target: LoginTarget::Qatar,
            account: saved.account,
            password: saved.password,
            auto_start: saved.auto_start,
            password_visible: false,
            packer_verify_count: 0,
            hover: None,
            pressed: None,
            account_focus: false,
            password_focus: false,
            account_hover: false,
            password_hover: false,
            toggle_pos: if saved.auto_start { 1.0 } else { 0.0 },
            toggle_from: 0.0,
            toggle_to: 0.0,
            toggle_start: 0,
            tray: Tray::new(hwnd, hicon),
            hicon,
            taskbar_created_msg,
            tray_tip: String::new(),
            reset_rx: None,
            update_rx: None,
            update_info: None,
            updating: false,
            badge_hover: false,
            install_rx: None,
            reset_busy: false,
            brush_white: unsafe { CreateSolidBrush(COLORREF(colorref(theme::INPUT_BG))) },
            brush_disabled: unsafe {
                CreateSolidBrush(COLORREF(colorref(theme::INPUT_DISABLED_BG)))
            },
            syncing: true,
        });

        let menu_font = app.fonts.body;
        app.tray.set_font(menu_font);
        app
    }

    /// WM_CREATE 阶段调用：创建子控件、背景层与托盘图标
    pub fn init_ui(&mut self) {
        // 上一版更新时留下的 .old 文件，顺手清掉（删不掉也无所谓）
        update::cleanup_old_version();
        self.create_children();
        self.create_backdrop();
        self.tray.show(self.hicon);
        self.refresh_tray_tip();
        self.update_status_ui();
        // 延迟几秒做第一次更新检查（等界面先出来），之后 on_update_timer 会改成 6 小时一轮。
        // selftest 是调试模式，总共只活 4 秒，没必要等 —— 否则日志和截图里
        // 永远看不到检查结果，等于没法验证这条链路。
        let delay = if crate::trace::enabled() {
            300
        } else {
            update::FIRST_CHECK_DELAY_MS
        };
        unsafe {
            let _ = SetTimer(Some(self.hwnd), TIMER_UPDATE, delay, None);
        }
    }

    /// 背景层要在主窗口显示之前建好，否则第一帧会先看到没有圆角的方角卡片
    fn create_backdrop(&mut self) {
        let size = (self.layout.window.w, self.layout.window.h);
        self.backdrop = Backdrop::new(self.hinst, self.layout.scale, size);
        self.sync_backdrop();
    }

    /// 让背景层跟上主窗口的位置、尺寸、z 序与显隐状态。
    /// 主窗口每次 `WM_WINDOWPOSCHANGED`（移动 / 缩放 / 显示 / 隐藏 / DPI 变化）都调用一次。
    pub fn sync_backdrop(&self) {
        let Some(bd) = self.backdrop.as_ref() else {
            return;
        };
        let visible = unsafe { IsWindowVisible(self.hwnd) }.as_bool();
        bd.sync(self.hwnd, visible);
    }

    // -----------------------------------------------------------------------
    // 子控件
    // -----------------------------------------------------------------------

    fn create_children(&mut self) {
        unsafe {
            let l = self.layout;

            // 账户 / 密码输入框：无边框原生 EDIT，白底与卡片背景一致，
            // 圆角边框由父窗口绘制，因此不需要额外的容器控件。
            self.h_account = self.create_child(
                "EDIT",
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE((ES_AUTOHSCROLL | ES_LEFT) as u32),
                ID_ACCOUNT,
                l.account_edit,
            );
            self.h_password = self.create_child(
                "EDIT",
                WS_CHILD
                    | WS_VISIBLE
                    | WS_TABSTOP
                    | WINDOW_STYLE((ES_AUTOHSCROLL | ES_LEFT | ES_PASSWORD) as u32),
                ID_PASSWORD,
                l.password_edit,
            );

            for (hwnd, font) in [
                (self.h_account, self.fonts.body),
                (self.h_password, self.fonts.body),
            ] {
                SendMessageW(
                    hwnd,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
            }

            // 自绘子控件
            self.h_eye = self.create_widget(EYE_CLASS, ID_EYE, l.eye);
            self.h_reset = self.create_widget(BTN_CLASS, ID_RESET, l.reset_btn);
            self.h_toggle = self.create_widget(TOGGLE_CLASS, ID_TOGGLE, l.toggle);
            self.h_start = self.create_widget(BTN_CLASS, ID_START, l.start_btn);
            self.h_stop = self.create_widget(BTN_CLASS, ID_STOP, l.stop_btn);
            self.h_min = self.create_widget(BTN_CLASS, ID_MIN, l.btn_min);
            self.h_close = self.create_widget(BTN_CLASS, ID_CLOSE, l.btn_close);

            // 初始文本
            let account = to_wide(&self.account);
            let password = to_wide(&self.password);
            let _ = SetWindowTextW(self.h_account, PCWSTR(account.as_ptr()));
            let _ = SetWindowTextW(self.h_password, PCWSTR(password.as_ptr()));

            // 密码掩码字符
            SendMessageW(
                self.h_password,
                EM_SETPASSWORDCHAR,
                Some(WPARAM(0x25CF)),
                None,
            );

            // 运行中不可编辑
            self.apply_enabled_state();
        }
        self.syncing = false;
    }

    fn create_child(&self, class: &str, style: WINDOW_STYLE, id: usize, r: Rect) -> HWND {
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
                Some(self.hwnd),
                Some(HMENU(id as *mut c_void)),
                Some(self.hinst),
                None,
            ) {
                Ok(h) => h,
                Err(err) => {
                    crate::tlog!(
                        "创建控件失败 class={} id={} hinst={:?} : {}",
                        class,
                        id,
                        self.hinst.0,
                        err.message()
                    );
                    HWND::default()
                }
            }
        }
    }

    fn create_widget(&self, class: &str, id: usize, r: Rect) -> HWND {
        let hwnd = self.create_child(class, WS_CHILD | WS_VISIBLE, id, r);
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, id as isize);
        }
        hwnd
    }

    /// 按当前布局重新摆放所有子控件
    pub fn layout_children(&mut self) {
        let l = self.layout;
        let items = [
            (self.h_account, l.account_edit),
            (self.h_password, l.password_edit),
            (self.h_eye, l.eye),
            (self.h_reset, l.reset_btn),
            (self.h_toggle, l.toggle),
            (self.h_start, l.start_btn),
            (self.h_stop, l.stop_btn),
            (self.h_min, l.btn_min),
            (self.h_close, l.btn_close),
        ];
        unsafe {
            for (hwnd, r) in items {
                if !hwnd.0.is_null() {
                    let _ = MoveWindow(hwnd, r.x, r.y, r.w, r.h, true);
                }
            }
            // 主窗口自己的裁剪区域：内缩 2px 的圆角矩形。
            //
            // 它的边界必须**完全落在背景层那张白色卡片的内部**，这样两边同色、
            // 看不见接缝；真正可见的圆角轮廓只剩背景层抗锯齿画出来的那一条。
            // 区域的硬边是 1 位掩码，必然有台阶，所以绝不能让这条边落在卡片边缘上。
            // 内缩后圆角半径也要同步减掉内缩量，圆弧才与原卡片同心。
            let inset = (crate::backdrop::REGION_INSET * l.scale).round() as i32;
            let radius = ((theme::CARD_RADIUS - crate::backdrop::REGION_INSET) * l.scale).round() as i32;
            let region = CreateRoundRectRgn(
                inset,
                inset,
                l.window.w - inset + 1,
                l.window.h - inset + 1,
                radius * 2,
                radius * 2,
            );
            let ok = SetWindowRgn(self.hwnd, Some(region), true);
            crate::tlog!("窗口圆角区域设置 -> {}", ok);
        }
    }

    /// 重建字体与布局（DPI 变化时）
    pub fn rebuild_for_dpi(&mut self, dpi: u32) {
        self.fonts = Fonts::new(dpi);
        self.layout = build_layout(&self.fonts, dpi);
        if let Some(bd) = self.backdrop.as_mut() {
            let size = (self.layout.window.w, self.layout.window.h);
            bd.rebuild(self.layout.scale, size);
        }
        unsafe {
            for (hwnd, font) in [
                (self.h_account, self.fonts.body),
                (self.h_password, self.fonts.body),
            ] {
                SendMessageW(
                    hwnd,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
            }
        }
        self.layout_children();
        self.repaint();
    }

    // -----------------------------------------------------------------------
    // 绘制
    // -----------------------------------------------------------------------

    pub fn visual_state(&self) -> VisualState<'_> {
        VisualState {
            login_state: self.login_state,
            account: &self.account,
            password: if self.password_visible {
                self.password.clone()
            } else {
                "*".repeat(self.password.chars().count())
            },
            password_visible: self.password_visible,
            toggle_pos: self.toggle_pos,
            hover: self.hover,
            pressed: self.pressed,
            fields_enabled: self.login_state == LoginState::Stopped,
            account_focus: self.account_focus,
            password_focus: self.password_focus,
            account_hover: self.account_hover,
            password_hover: self.password_hover,
            start_enabled: self.login_state == LoginState::Stopped,
            stop_enabled: self.login_state != LoginState::Stopped,
            reset_enabled: !self.reset_busy,
            version_state: self.version_state,
            updating: self.updating,
            badge_hover: self.badge_hover,
        }
    }

    /// 更新版本状态并重绘（有变化才重绘）
    pub fn set_version_state(&mut self, state: VersionState) {
        if self.version_state != state {
            self.version_state = state;
            self.repaint();
        }
    }

    // -----------------------------------------------------------------------
    // 更新检查
    // -----------------------------------------------------------------------

    /// 起一次后台更新检查。上一次还没回来就跳过，避免请求堆积。
    pub fn start_update_check(&mut self) {
        if self.update_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.update_rx = Some(rx);
        // HWND 里是裸指针，不是 Send，不能直接搬进线程；按 isize 传
        let hwnd_raw = self.hwnd.0 as isize;
        std::thread::spawn(move || {
            let outcome = update::check();
            // 线程结束后用 PostMessage 把结果交回界面线程：
            // 界面状态只能在界面线程里改，直接跨线程写会踩数据竞争。
            if tx.send(outcome).is_ok() {
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd_raw as *mut c_void)),
                        WM_UPDATE_DONE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        });
    }

    pub fn on_update_done(&mut self) {
        let Some(rx) = self.update_rx.take() else {
            return;
        };
        let Ok(outcome) = rx.try_recv() else {
            return;
        };
        if !outcome.latest.is_empty() {
            // 说明可能多行，落进日志前压成一行并截断
            let notes: String = outcome.notes.replace(['\r', '\n'], " ").chars().take(60).collect();
            crate::tlog!(
                "更新检查：本机 v{}，服务端 v{}（{}）{}",
                version::APP_VERSION,
                outcome.latest,
                if outcome.force { "强制更新" } else { "常规" },
                notes
            );
        }
        let state = outcome.state;
        self.update_info = Some(outcome);
        self.set_version_state(state);
    }

    /// 版本徽章是否可点（有新版本、且当前没在更新）
    pub fn badge_clickable(&self) -> bool {
        !self.updating
            && self
                .update_info
                .as_ref()
                .map(|info| info.state == VersionState::Outdated && !info.latest.is_empty())
                .unwrap_or(false)
    }

    /// 点了版本徽章：确认后开始下载新版本
    pub fn on_update_click(&mut self) {
        if !self.badge_clickable() {
            return;
        }
        let Some(info) = self.update_info.clone() else {
            return;
        };

        let notes = if info.notes.trim().is_empty() {
            "（服务端没有提供更新说明）".to_string()
        } else {
            info.notes.trim().to_string()
        };
        let text = format!(
            "发现新版本 v{}\n\n{}\n\n更新完成后程序会自动重启，账户密码与开机自启设置保持不变。\n\n是否现在更新？",
            info.latest, notes
        );
        if !self.confirm_box("发现新版本", &text) {
            return;
        }

        crate::tlog!("开始下载 v{}", info.latest);
        self.updating = true;
        self.repaint();

        let (tx, rx) = mpsc::channel();
        self.install_rx = Some(rx);
        let hwnd_raw = self.hwnd.0 as isize;
        let latest = info.latest.clone();
        let size = info.size;
        std::thread::spawn(move || {
            let result = update::download(&latest, size);
            let _ = tx.send(result);
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(hwnd_raw as *mut c_void)),
                    WM_UPDATE_READY,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        });
    }

    /// 下载结束：成功就替换并重启，失败就恢复原状并说明原因
    pub fn on_update_ready(&mut self) {
        let Some(rx) = self.install_rx.take() else {
            return;
        };
        let Ok(result) = rx.try_recv() else {
            return;
        };

        match result {
            Ok(staged) => match update::install_and_restart(&staged) {
                Ok(()) => {
                    crate::tlog!(
                        "v{} 已就位（{} 字节），退出让新进程接管",
                        staged.version,
                        staged.size
                    );
                    // 新进程已经启动，这边直接退出
                    unsafe {
                        let _ = DestroyWindow(self.hwnd);
                    }
                }
                Err(err) => {
                    crate::tlog!("更新失败：{err}");
                    self.updating = false;
                    self.repaint();
                    self.error_box("更新失败", &err);
                }
            },
            Err(err) => {
                crate::tlog!("下载失败：{err}");
                self.updating = false;
                self.repaint();
                self.error_box("下载失败", &err);
            }
        }
    }

    /// 更新检查定时器：第一次是「启动后延迟几秒」，之后固定 6 小时一轮
    pub fn on_update_timer(&mut self) {
        self.start_update_check();
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_UPDATE);
            let _ = SetTimer(Some(self.hwnd), TIMER_UPDATE, update::CHECK_INTERVAL_MS, None);
        }
    }

    /// 整窗重绘（双缓冲，避免闪烁）
    pub fn repaint(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// 重绘某个子控件
    pub fn repaint_child(&self, hwnd: HWND) {
        if !hwnd.0.is_null() {
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
    }

    /// 重绘全部自绘子控件（hover / pressed 状态变化时）
    pub fn repaint_widgets(&self) {
        for hwnd in [
            self.h_min,
            self.h_close,
            self.h_reset,
            self.h_start,
            self.h_stop,
            self.h_toggle,
            self.h_eye,
        ] {
            self.repaint_child(hwnd);
        }
    }

    pub fn paint(&self, hdc: HDC) {
        let l = self.layout;
        // 内存 DC 双缓冲
        let mem = unsafe { CreateCompatibleDC(Some(hdc)) };
        let bitmap = unsafe { CreateCompatibleBitmap(hdc, l.window.w.max(1), l.window.h.max(1)) };
        let old = unsafe { SelectObject(mem, HGDIOBJ(bitmap.0)) };

        if let Some(canvas) = Canvas::from_hdc(mem) {
            let vs = self.visual_state();
            paint_all(&canvas, &l, &self.fonts, &vs, false);
        }

        unsafe {
            let _ = BitBlt(hdc, 0, 0, l.window.w, l.window.h, Some(mem), 0, 0, SRCCOPY);
            SelectObject(mem, old);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(mem);
        }
    }

    /// 绘制一个子控件（供子窗口 wndproc 调用）
    pub fn paint_widget(&self, id: usize, hdc: HDC, w: i32, h: i32) {
        let l = self.layout;
        let Some(canvas) = Canvas::from_hdc(hdc) else {
            return;
        };

        // 自绘控件都是圆角形状，四个角在形状之外。子窗口有独立的绘制表面，
        // 双缓冲位图又是未初始化的，不先铺一层卡片底色就会露出黑角。
        canvas.fill_rect(Rect::new(0, 0, w, h), crate::util::rgb(theme::CARD_BG));

        let vs = self.visual_state();
        let _ = vs;

        match id {
            ID_MIN => paint_button(
                &canvas,
                &self.fonts,
                Rect::new(0, 0, l.btn_min.w, l.btn_min.h),
                crate::model::BtnKind::WinMin,
                "—",
                l.scale,
                self.hover == Some(Widget::Min),
                self.pressed == Some(Widget::Min),
                false,
            ),
            ID_CLOSE => paint_button(
                &canvas,
                &self.fonts,
                Rect::new(0, 0, l.btn_close.w, l.btn_close.h),
                crate::model::BtnKind::WinClose,
                "✕",
                l.scale,
                self.hover == Some(Widget::Close),
                self.pressed == Some(Widget::Close),
                false,
            ),
            ID_RESET => paint_button(
                &canvas,
                &self.fonts,
                Rect::new(0, 0, l.reset_btn.w, l.reset_btn.h),
                crate::model::BtnKind::Reset,
                "重置",
                l.scale,
                self.hover == Some(Widget::Reset),
                self.pressed == Some(Widget::Reset),
                self.reset_busy,
            ),
            ID_START => paint_button(
                &canvas,
                &self.fonts,
                Rect::new(0, 0, l.start_btn.w, l.start_btn.h),
                crate::model::BtnKind::Primary,
                "启动",
                l.scale,
                self.hover == Some(Widget::Start),
                self.pressed == Some(Widget::Start),
                self.login_state != LoginState::Stopped,
            ),
            ID_STOP => paint_button(
                &canvas,
                &self.fonts,
                Rect::new(0, 0, l.stop_btn.w, l.stop_btn.h),
                crate::model::BtnKind::Secondary,
                "停止",
                l.scale,
                self.hover == Some(Widget::Stop),
                self.pressed == Some(Widget::Stop),
                self.login_state == LoginState::Stopped,
            ),
            ID_TOGGLE => paint_toggle(&canvas, Rect::new(0, 0, l.toggle.w, l.toggle.h), self.toggle_pos),
            ID_EYE => icon::draw_eye(
                &canvas,
                Rect::new(0, 0, l.eye.w, l.eye.h),
                !self.password_visible,
                if self.login_state == LoginState::Stopped {
                    theme::WINBTN_FG
                } else {
                    theme::INPUT_DISABLED_TEXT
                },
                1.8 * l.scale,
            ),
            _ => {}
        }
        let _ = vs;
    }

    // -----------------------------------------------------------------------
    // 状态机
    // -----------------------------------------------------------------------

    pub fn set_login_state(&mut self, state: LoginState) {
        self.login_state = state;
        self.update_status_ui();
        self.repaint();
    }

    fn apply_enabled_state(&self) {
        let stopped = self.login_state == LoginState::Stopped;
        unsafe {
            let _ = EnableWindow(self.h_account, stopped);
            let _ = EnableWindow(self.h_password, stopped);
            let _ = EnableWindow(self.h_eye, stopped);
            let _ = EnableWindow(self.h_start, stopped);
            let _ = EnableWindow(self.h_stop, !stopped);
            let _ = EnableWindow(self.h_reset, !self.reset_busy);
        }
    }

    /// 刷新状态徽标、按钮可用性与托盘提示
    pub fn update_status_ui(&mut self) {
        self.apply_enabled_state();

        let badge = match self.login_state {
            LoginState::Running => "运行中",
            LoginState::Handling => "登录中",
            LoginState::Stopped => "已停止",
        };
        self.tray.set_status(badge, self.login_state != LoginState::Stopped);
        self.refresh_tray_tip();
        self.repaint();
        self.repaint_child(self.h_start);
        self.repaint_child(self.h_stop);
        self.repaint_child(self.h_eye);
        self.repaint_child(self.h_reset);
    }

    fn refresh_tray_tip(&mut self) {
        let badge = match self.login_state {
            LoginState::Running => "运行中",
            LoginState::Handling => "登录中",
            LoginState::Stopped => "已停止",
        };
        self.tray_tip = format!("{}\n程序状态: {}", APP_NAME, badge);
        self.tray.set_tip(&self.tray_tip);
    }

    /// 开启自动登录
    pub fn enable_auto_login(&mut self) {
        crate::tlog!(
            "enable_auto_login: password_len={} state={:?}",
            self.password.chars().count(),
            self.login_state
        );
        if self.login_state == LoginState::Running {
            self.error_box("自动登录已启用！", "自动登录已启用...");
            return;
        }
        // 卡控密码为空
        if self.password.is_empty() {
            self.error_box("密码为空！", "请先配置密码...");
            return;
        }
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_POLL, POLL_INTERVAL_MS, None);
        }
        self.set_login_state(LoginState::Running);
    }

    /// 停止自动登录（「登录中」同样可停，并会取消待执行的登录动作）
    pub fn disable_auto_login(&mut self) {
        if self.login_state == LoginState::Stopped {
            self.error_box("自动登录未启用！", "自动登录未启用...");
            return;
        }
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_LOGIN);
            let _ = KillTimer(Some(self.hwnd), TIMER_VERIFY);
            let _ = KillTimer(Some(self.hwnd), TIMER_POLL);
        }
        self.packer_verify_count = 0;
        self.set_login_state(LoginState::Stopped);
    }

    /// 托盘菜单的启停动作
    pub fn tray_toggle(&mut self) {
        if self.login_state == LoginState::Stopped {
            // 密码为空时会弹提示，先恢复主窗口，避免提示框挂在隐藏窗口上
            if self.password.is_empty() {
                self.show_from_tray();
            }
            self.enable_auto_login();
        } else {
            self.disable_auto_login();
        }
    }

    /// 轮询到目标进程
    pub fn on_poll(&mut self) {
        if self.login_state != LoginState::Running {
            return;
        }
        if !process::is_target_running() {
            return;
        }

        // 两个客户端的凭据对话框都未出现时不做处理，避免登录成功后客户端仍驻留
        // 导致监测被无意义地反复暂停与恢复
        if automation::find_dialog().is_some() {
            self.active_target = LoginTarget::Qatar;
        } else if automation::find_packer_dialog().is_some() {
            self.active_target = LoginTarget::QtPacker;
        } else {
            return;
        }

        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_POLL);
            SetTimer(Some(self.hwnd), TIMER_LOGIN, LOGIN_DELAY_MS, None);
        }
        self.set_login_state(LoginState::Handling);
    }

    /// 登录动作延时到期
    pub fn on_login_timer(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_LOGIN);
        }
        // 等待期间若已被停止，本次登录不再执行，也不再恢复监测
        if self.login_state != LoginState::Handling {
            return;
        }
        match self.active_target {
            LoginTarget::QtPacker => self.perform_packer_login(),
            LoginTarget::Qatar => self.perform_qatar_login(),
        }
    }

    fn resume_monitor(&mut self) {
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_POLL, POLL_INTERVAL_MS, None);
        }
        self.set_login_state(LoginState::Running);
    }

    /// Qatar 客户端登录
    fn perform_qatar_login(&mut self) {
        if let Some(dialog) = automation::find_dialog() {
            if automation::perform_qatar_login(dialog, &self.account, &self.password) {
                self.report_password_error();
                return;
            }
        }
        self.resume_monitor();
    }

    /// QtPacker 客户端登录：填 Uid / Pwd 后异步点击，再交给校验定时器
    fn perform_packer_login(&mut self) {
        let Some(dialog) = automation::find_packer_dialog() else {
            // 对话框已消失（可能已被手动登录），直接恢复监测
            self.resume_monitor();
            return;
        };

        match automation::perform_packer_login(dialog, &self.account, &self.password) {
            automation::PackerOutcome::NotReady => {
                // 控件尚未就绪，恢复监测等待下一轮重试
                self.resume_monitor();
            }
            automation::PackerOutcome::DialogGone => self.resume_monitor(),
            automation::PackerOutcome::Submitted => {
                self.packer_verify_count = 0;
                unsafe {
                    SetTimer(Some(self.hwnd), TIMER_VERIFY, PACKER_VERIFY_DELAY_MS, None);
                }
            }
        }
    }

    /// 校验 QtPacker 登录结果：出现失败提示框即密码错误；对话框关闭视为成功
    pub fn on_verify_timer(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_VERIFY);
        }
        if self.login_state != LoginState::Handling {
            return;
        }

        if automation::find_packer_error_msg().is_some() {
            self.report_password_error();
            return;
        }

        if automation::find_packer_dialog().is_some() {
            self.packer_verify_count += 1;
            if self.packer_verify_count < PACKER_MAX_VERIFY {
                // 登录可能仍在处理中，等待后再校验
                unsafe {
                    SetTimer(Some(self.hwnd), TIMER_VERIFY, PACKER_VERIFY_DELAY_MS, None);
                }
                return;
            }
        }

        self.packer_verify_count = 0;
        self.resume_monitor();
    }

    /// 密码错误：停止监测、窗口置顶提示，避免反复重试
    fn report_password_error(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_LOGIN);
            let _ = KillTimer(Some(self.hwnd), TIMER_VERIFY);
            let _ = KillTimer(Some(self.hwnd), TIMER_POLL);
        }
        self.packer_verify_count = 0;
        self.login_state = LoginState::Stopped;
        self.update_status_ui();

        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.error_box("密码错误", "密码与工号不匹配, 请重新配置密码...");
    }

    // -----------------------------------------------------------------------
    // 重置密码
    // -----------------------------------------------------------------------

    pub fn on_reset_password(&mut self) {
        let uid = self.account.trim().to_string();
        if uid.is_empty() {
            self.warning_box("账户为空", "请先填写账户，再重置密码。");
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.reset_rx = Some(rx);
        self.reset_busy = true;
        self.update_status_ui();

        let hwnd_raw = self.hwnd.0 as isize;
        std::thread::spawn(move || {
            let result = net::reset_password(&uid);
            let _ = tx.send(result);
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd_raw as *mut c_void)), WM_RESET_DONE, WPARAM(0), LPARAM(0));
            }
        });
    }

    pub fn on_reset_done(&mut self) {
        self.reset_busy = false;
        let result = self.reset_rx.as_ref().and_then(|rx| rx.try_recv().ok());
        self.reset_rx = None;
        self.update_status_ui();

        match result {
            Some(ResetResult::Body(body)) => {
                if body == "1" {
                    self.info_box("重置成功", "密码已重置，新密码将通过钉钉发送，请查收。");
                } else if body.is_empty() {
                    self.error_box("重置失败", "接口未返回内容，请检查网络或接口地址。");
                } else {
                    self.error_box("重置失败", &format!("接口返回异常：{}", body));
                }
            }
            Some(ResetResult::Failed(err)) => {
                self.error_box("重置失败", &format!("请求失败：{}", err));
            }
            None => {}
        }
    }

    // -----------------------------------------------------------------------
    // 开关动画
    // -----------------------------------------------------------------------

    pub fn set_auto_start(&mut self, value: bool) {
        if self.auto_start == value {
            return;
        }
        self.auto_start = value;
        self.toggle_from = self.toggle_pos;
        self.toggle_to = if value { 1.0 } else { 0.0 };
        self.toggle_start = now_ms();
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_ANIM, 16, None);
        }
    }

    pub fn on_anim_timer(&mut self) {
        let elapsed = now_ms().saturating_sub(self.toggle_start);
        let t = (elapsed as f32 / TOGGLE_ANIM_MS as f32).clamp(0.0, 1.0);
        let eased = ease_in_out_cubic(t);
        self.toggle_pos = self.toggle_from + (self.toggle_to - self.toggle_from) * eased;
        if t >= 1.0 {
            self.toggle_pos = self.toggle_to;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_ANIM);
            }
        }
        self.repaint_child(self.h_toggle);
    }

    pub fn toggle_password_visible(&mut self) {
        self.password_visible = !self.password_visible;
        unsafe {
            let ch = if self.password_visible { 0 } else { 0x25CF };
            SendMessageW(self.h_password, EM_SETPASSWORDCHAR, Some(WPARAM(ch)), None);
            let _ = InvalidateRect(Some(self.h_password), None, true);
        }
        self.repaint_child(self.h_eye);
    }

    // -----------------------------------------------------------------------
    // 窗口显示 / 隐藏 / 退出
    // -----------------------------------------------------------------------

    pub fn show_from_tray(&self) {
        // 先把背景层亮出来，再显示主窗口：否则第一帧会露出主窗口自己的方角硬边
        if let Some(bd) = self.backdrop.as_ref() {
            bd.sync(self.hwnd, true);
        }
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.sync_backdrop();
    }

    /// 关闭按钮 / ✕：保存配置并最小化到托盘（不退出进程）
    pub fn hide_to_tray(&mut self) {
        self.save_settings();
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.tray.show(self.hicon);
    }

    pub fn save_settings(&mut self) {
        let s = Settings {
            account: self.account.clone(),
            password: self.password.clone(),
            auto_start: self.auto_start,
        };
        s.save();

        let exe = current_exe_path();
        settings::set_auto_start(self.auto_start, &exe);
    }

    /// 托盘菜单「退出」
    pub fn on_exit(&mut self) {
        self.show_from_tray();
        let yes = unsafe {
            MessageBoxW(
                Some(self.hwnd),
                windows::core::w!("确定要退出程序吗？"),
                windows::core::w!("退出"),
                MB_YESNO | MB_ICONQUESTION,
            )
        } == IDYES;
        if !yes {
            return;
        }

        self.tray.remove();
        if self.login_state != LoginState::Stopped {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_POLL);
                let _ = KillTimer(Some(self.hwnd), TIMER_LOGIN);
                let _ = KillTimer(Some(self.hwnd), TIMER_VERIFY);
            }
            self.login_state = LoginState::Stopped;
        }
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }

    // -----------------------------------------------------------------------
    // 消息框
    // -----------------------------------------------------------------------

    pub fn error_box(&self, caption: &str, text: &str) {
        let (c, t) = (to_wide(caption), to_wide(text));
        unsafe {
            MessageBoxW(
                Some(self.hwnd),
                PCWSTR(t.as_ptr()),
                PCWSTR(c.as_ptr()),
                MB_OK | MB_ICONERROR,
            );
        }
    }

    pub fn warning_box(&self, caption: &str, text: &str) {
        let (c, t) = (to_wide(caption), to_wide(text));
        unsafe {
            MessageBoxW(
                Some(self.hwnd),
                PCWSTR(t.as_ptr()),
                PCWSTR(c.as_ptr()),
                MB_OK | MB_ICONWARNING,
            );
        }
    }

    pub fn info_box(&self, caption: &str, text: &str) {
        let (c, t) = (to_wide(caption), to_wide(text));
        unsafe {
            MessageBoxW(
                Some(self.hwnd),
                PCWSTR(t.as_ptr()),
                PCWSTR(c.as_ptr()),
                MB_OK | MB_ICONINFORMATION,
            );
        }
    }

    /// 是 / 否确认框
    pub fn confirm_box(&self, caption: &str, text: &str) -> bool {
        let (c, t) = (to_wide(caption), to_wide(text));
        unsafe {
            MessageBoxW(
                Some(self.hwnd),
                PCWSTR(t.as_ptr()),
                PCWSTR(c.as_ptr()),
                MB_YESNO | MB_ICONQUESTION,
            ) == IDYES
        }
    }

    // -----------------------------------------------------------------------
    // 命中测试 / 交互
    // -----------------------------------------------------------------------

    pub fn set_hover(&mut self, hover: Option<Widget>) {
        if self.hover != hover {
            self.hover = hover;
        }
    }

    pub fn refresh_hover_from_cursor(&mut self) {
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            return;
        }
        let mut client = pt;
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut client);
        }
        self.account_hover = self.layout.account_field.contains(client.x, client.y);
        self.password_hover = self.layout.password_field.contains(client.x, client.y);
    }

    pub fn clear_hover(&mut self) {
        self.account_hover = false;
        self.password_hover = false;
    }
}

impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.brush_white.0));
            let _ = DeleteObject(HGDIOBJ(self.brush_disabled.0));
        }
    }
}

// ---------------------------------------------------------------------------
// 辅助
// ---------------------------------------------------------------------------

pub fn now_ms() -> u64 {
    unsafe { GetTickCount64() }
}

fn ease_in_out_cubic(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        let f = -2.0 * t + 2.0;
        1.0 - f * f * f / 2.0
    }
}

pub fn current_exe_path() -> String {
    let mut buf = [0u16; 1024];
    let len = unsafe { GetModuleFileNameW(None, &mut buf) };
    if len == 0 {
        String::new()
    } else {
        from_wide(&buf[..len as usize])
    }
}

/// 供子控件 wndproc 取回 App
pub unsafe fn app_ref(hwnd: HWND) -> Option<&'static mut App> {
    let mut root = hwnd;
    loop {
        let parent = GetParent(root).unwrap_or_default();
        if parent.0.is_null() {
            break;
        }
        root = parent;
    }
    let ptr = GetWindowLongPtrW(root, GWLP_USERDATA) as *mut App;
    if ptr.is_null() {
        None
    } else {
        Some(&mut *ptr)
    }
}

pub fn main_instance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).map(|h| HINSTANCE(h.0)).unwrap_or_default() }
}

/// 创建主窗口
pub fn create_main_window(dpi: u32) -> windows::core::Result<HWND> {
    let hinst = main_instance();
    let layout = Layout::compute(dpi as f32 / 96.0);

    let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    let x = (screen_w - layout.window.w) / 2;
    let y = (screen_h - layout.window.h) / 2;

    let class = to_wide(MAIN_CLASS);
    let title = to_wide(theme::APP_TITLE);
    unsafe {
        CreateWindowExW(
            WS_EX_APPWINDOW,
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_POPUP | WS_CLIPCHILDREN,
            x,
            y,
            layout.window.w,
            layout.window.h,
            None,
            None,
            Some(hinst),
            None,
        )
    }
}

/// 主窗口 DPI
pub fn window_dpi(hwnd: HWND) -> u32 {
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        if dpi == 0 {
            96
        } else {
            dpi
        }
    }
}

pub fn set_dpi_aware() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

pub fn text_color(enabled: bool) -> COLORREF {
    let c = if enabled {
        theme::INPUT_TEXT
    } else {
        theme::INPUT_DISABLED_TEXT
    };
    COLORREF(colorref(c))
}
