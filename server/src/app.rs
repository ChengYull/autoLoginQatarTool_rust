//! 主窗口：服务状态、版本列表、请求日志与发布操作。

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::EM_SETMARGINS;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::backdrop::Backdrop;
use crate::config::Config;
use crate::gfx::{text_width, Canvas, Fonts};
use crate::http::{Server, Shared};
use crate::model::{BtnKind, Widget};
use crate::paint::{paint_all, paint_button, VisualState};
use crate::publish;
use crate::store::{self, Repo};
use crate::theme::{self, Layout};
use crate::util::{from_wide, to_wide, Rect};

// ---------------------------------------------------------------------------
// 常量
// ---------------------------------------------------------------------------

pub const MAIN_CLASS: &str = "RustAutoLoginServerMain";
pub const BTN_CLASS: &str = "RustAutoLoginServerBtn";

/// 控件 ID
pub const ID_ADDR: usize = 1001;
pub const ID_START: usize = 1002;
pub const ID_STOP: usize = 1003;
pub const ID_PUBLISH: usize = 1004;
pub const ID_LATEST: usize = 1005;
pub const ID_DELETE: usize = 1006;
pub const ID_OPENDIR: usize = 1007;
pub const ID_MIN: usize = 1008;
pub const ID_CLOSE: usize = 1009;

/// 定时器：刷新日志与统计
const TIMER_REFRESH: usize = 1;
const REFRESH_INTERVAL_MS: u32 = 1000;

/// 自定义消息：窗口显示之后再启动监听
pub const WM_START_SERVER: u32 = WM_APP + 1;

/// 数据目录。进程内只有一个主窗口，用全局量传比往 `CreateWindowExW` 的
/// `lpParam` 里塞裸指针安全 —— 后者指向栈上局部变量，函数一返回就悬空了。
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_data_dir(dir: PathBuf) {
    let _ = DATA_DIR.set(dir);
}

pub fn data_dir() -> PathBuf {
    DATA_DIR
        .get()
        .cloned()
        .unwrap_or_else(store::default_data_dir)
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub struct App {
    pub hwnd: HWND,
    pub hinst: HINSTANCE,
    pub layout: Layout,
    pub fonts: Fonts,
    pub backdrop: Option<Backdrop>,

    // 子控件
    pub h_addr: HWND,
    pub h_start: HWND,
    pub h_stop: HWND,
    pub h_publish: HWND,
    pub h_latest: HWND,
    pub h_delete: HWND,
    pub h_opendir: HWND,
    pub h_min: HWND,
    pub h_close: HWND,

    // 数据
    pub data_dir: PathBuf,
    pub config: Config,
    pub repo: Repo,
    pub shared: Arc<Shared>,
    pub server: Option<Server>,

    // 交互状态
    pub hover: Option<Widget>,
    pub pressed: Option<Widget>,
    pub selected: Option<usize>,
    pub scroll: usize,
    pub addr_focus: bool,
    pub addr_hover: bool,

    /// 上一次刷新时看到的统计值，用来判断"要不要重绘"
    last_hits: u64,
    last_logs: usize,
}

impl App {
    pub fn new(hwnd: HWND, hinst: HINSTANCE, dpi: u32, data_dir: PathBuf) -> Box<Self> {
        let fonts = Fonts::new(dpi);
        let layout = build_layout(&fonts, dpi);

        let config = Config::load(&data_dir);
        let repo = Repo::load(&data_dir).unwrap_or_default();
        let shared = Shared::new(data_dir.clone(), repo.clone());

        Box::new(App {
            hwnd,
            hinst,
            layout,
            fonts,
            backdrop: None,
            h_addr: HWND::default(),
            h_start: HWND::default(),
            h_stop: HWND::default(),
            h_publish: HWND::default(),
            h_latest: HWND::default(),
            h_delete: HWND::default(),
            h_opendir: HWND::default(),
            h_min: HWND::default(),
            h_close: HWND::default(),
            data_dir,
            config,
            repo,
            shared,
            server: None,
            hover: None,
            pressed: None,
            selected: None,
            scroll: 0,
            addr_focus: false,
            addr_hover: false,
            last_hits: 0,
            last_logs: 0,
        })
    }

    /// WM_CREATE 阶段调用
    pub fn init_ui(&mut self) {
        self.create_children();
        self.create_backdrop();
        self.apply_window_region();
        self.set_addr_text(&self.config.bind.clone());
        self.layout_children();
        self.update_enabled();
        // 打开界面就把服务拉起来：这是个"双击就该开始工作"的工具，
        // 让用户再点一次「启动」是多余的一步。
        //
        // 但要用 PostMessage 延后到窗口显示之后再启动：WM_CREATE 时窗口还没
        // ShowWindow，万一端口被占用要弹错误框，挂在一个不可见的 owner 上
        // 会跑到屏幕角落去，看着像卡住了。
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_START_SERVER, WPARAM(0), LPARAM(0));
            let _ = SetTimer(Some(self.hwnd), TIMER_REFRESH, REFRESH_INTERVAL_MS, None);
        }
    }

    fn create_backdrop(&mut self) {
        let size = (self.layout.window.w, self.layout.window.h);
        self.backdrop = Backdrop::new(self.hinst, self.layout.scale, size);
        self.sync_backdrop();
    }

    pub fn sync_backdrop(&self) {
        let Some(bd) = self.backdrop.as_ref() else {
            return;
        };
        let visible = unsafe { IsWindowVisible(self.hwnd) }.as_bool();
        bd.sync(self.hwnd, visible);
    }

    /// 主窗口自身裁成「内缩 2px、半径减 2」的圆角区域，硬边藏在背景层白卡里
    pub fn apply_window_region(&self) {
        let l = &self.layout;
        let inset = (crate::backdrop::REGION_INSET * l.scale).round() as i32;
        let radius = ((theme::CARD_RADIUS - crate::backdrop::REGION_INSET) * l.scale).round() as i32;
        let region = unsafe {
            CreateRoundRectRgn(
                inset,
                inset,
                l.window.w - inset + 1,
                l.window.h - inset + 1,
                radius * 2,
                radius * 2,
            )
        };
        unsafe {
            let _ = SetWindowRgn(self.hwnd, Some(region), true);
        }
    }

    // -----------------------------------------------------------------------
    // 子控件
    // -----------------------------------------------------------------------

    fn create_children(&mut self) {
        let l = self.layout;
        self.h_addr = self.create_child(
            "EDIT",
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            ID_ADDR,
            l.addr_edit,
        );
        self.h_start = self.create_child(BTN_CLASS, WS_CHILD | WS_VISIBLE, ID_START, l.btn_start);
        self.h_stop = self.create_child(BTN_CLASS, WS_CHILD | WS_VISIBLE, ID_STOP, l.btn_stop);
        self.h_publish = self.create_child(BTN_CLASS, WS_CHILD | WS_VISIBLE, ID_PUBLISH, l.btn_publish);
        self.h_latest = self.create_child(BTN_CLASS, WS_CHILD | WS_VISIBLE, ID_LATEST, l.btn_latest);
        self.h_delete = self.create_child(BTN_CLASS, WS_CHILD | WS_VISIBLE, ID_DELETE, l.btn_delete);
        self.h_opendir = self.create_child(BTN_CLASS, WS_CHILD | WS_VISIBLE, ID_OPENDIR, l.btn_opendir);
        self.h_min = self.create_child(BTN_CLASS, WS_CHILD | WS_VISIBLE, ID_MIN, l.btn_min);
        self.h_close = self.create_child(BTN_CLASS, WS_CHILD | WS_VISIBLE, ID_CLOSE, l.btn_close);

        unsafe {
            for hwnd in [self.h_addr] {
                SendMessageW(
                    hwnd,
                    WM_SETFONT,
                    Some(WPARAM(self.fonts.body.0 as usize)),
                    Some(LPARAM(1)),
                );
                // EDIT 默认贴着左边框，给它留出和绘制一致的内边距
                SendMessageW(
                    hwnd,
                    EM_SETMARGINS,
                    Some(WPARAM((EC_LEFTMARGIN | EC_RIGHTMARGIN) as usize)),
                    Some(LPARAM((10 | (10 << 16)) as isize)),
                );
            }
        }
    }

    fn create_child(&self, class: &str, style: WINDOW_STYLE, id: usize, r: Rect) -> HWND {
        let class_w = to_wide(class);
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class_w.as_ptr()),
                PCWSTR::null(),
                style,
                r.x,
                r.y,
                r.w,
                r.h,
                Some(self.hwnd),
                Some(HMENU(id as *mut c_void)),
                Some(self.hinst),
                None,
            )
        };
        match hwnd {
            Ok(h) => h,
            Err(err) => {
                crate::tlog!("创建子控件失败 id={id}: {}", err.message());
                HWND::default()
            }
        }
    }

    /// 按当前布局摆放所有子控件
    pub fn layout_children(&self) {
        let l = self.layout;
        unsafe {
            for (hwnd, r) in [
                (self.h_addr, l.addr_edit),
                (self.h_start, l.btn_start),
                (self.h_stop, l.btn_stop),
                (self.h_publish, l.btn_publish),
                (self.h_latest, l.btn_latest),
                (self.h_delete, l.btn_delete),
                (self.h_opendir, l.btn_opendir),
                (self.h_min, l.btn_min),
                (self.h_close, l.btn_close),
            ] {
                if !hwnd.0.is_null() {
                    let _ = MoveWindow(hwnd, r.x, r.y, r.w, r.h, true);
                }
            }
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
            SendMessageW(
                self.h_addr,
                WM_SETFONT,
                Some(WPARAM(self.fonts.body.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        self.apply_window_region();
        self.layout_children();
        self.repaint();
    }

    // -----------------------------------------------------------------------
    // 状态与启用关系
    // -----------------------------------------------------------------------

    pub fn running(&self) -> bool {
        self.server.is_some()
    }

    /// 按当前状态刷新控件可用性
    pub fn update_enabled(&self) {
        let running = self.running();
        unsafe {
            let _ = EnableWindow(self.h_addr, !running);
            let _ = EnableWindow(self.h_start, !running);
            let _ = EnableWindow(self.h_stop, running);
            let _ = EnableWindow(self.h_latest, self.selected.is_some());
            let _ = EnableWindow(self.h_delete, self.selected.is_some());
        }
        self.repaint_widgets();
    }

    pub fn set_addr_text(&self, text: &str) {
        let wide = to_wide(text);
        unsafe {
            let _ = SetWindowTextW(self.h_addr, PCWSTR(wide.as_ptr()));
        }
    }

    fn addr_text(&self) -> String {
        let mut buf = [0u16; 256];
        let n = unsafe { GetWindowTextW(self.h_addr, &mut buf) };
        if n <= 0 {
            return String::new();
        }
        from_wide(&buf[..n as usize])
    }

    // -----------------------------------------------------------------------
    // 绘制
    // -----------------------------------------------------------------------

    pub fn visual_state(&self) -> VisualState<'_> {
        VisualState {
            running: self.running(),
            bind: &self.config.bind,
            releases: self.repo.sorted(),
            latest: &self.repo.latest,
            selected: self.selected,
            scroll: self.scroll,
            logs: self.shared.snapshot_logs(),
            total_hits: self.shared.hits.load(std::sync::atomic::Ordering::Relaxed),
            total_downloads: self.shared.downloads.load(std::sync::atomic::Ordering::Relaxed),
            hover: self.hover,
            pressed: self.pressed,
            addr_enabled: !self.running(),
            addr_focus: self.addr_focus,
            addr_hover: self.addr_hover,
            start_enabled: !self.running(),
            stop_enabled: self.running(),
            publish_enabled: true,
            set_latest_enabled: self.selected.is_some(),
            delete_enabled: self.selected.is_some(),
        }
    }

    pub fn repaint(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    pub fn repaint_child(&self, hwnd: HWND) {
        if !hwnd.0.is_null() {
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
    }

    pub fn repaint_widgets(&self) {
        for hwnd in [
            self.h_min,
            self.h_close,
            self.h_start,
            self.h_stop,
            self.h_publish,
            self.h_latest,
            self.h_delete,
            self.h_opendir,
        ] {
            self.repaint_child(hwnd);
        }
    }

    pub fn paint(&self, hdc: HDC) {
        let l = self.layout;
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
        canvas.fill_rect(Rect::new(0, 0, w, h), crate::util::rgb(theme::CARD_BG));

        let (r, kind, text, disabled) = match id {
            ID_MIN => (l.btn_min, BtnKind::WinMin, "—", false),
            ID_CLOSE => (l.btn_close, BtnKind::WinClose, "✕", false),
            ID_START => (l.btn_start, BtnKind::Primary, "启动", self.running()),
            ID_STOP => (l.btn_stop, BtnKind::Secondary, "停止", !self.running()),
            ID_PUBLISH => (l.btn_publish, BtnKind::Primary, "发布新版本", false),
            ID_LATEST => (l.btn_latest, BtnKind::Secondary, "设为最新", self.selected.is_none()),
            ID_DELETE => (l.btn_delete, BtnKind::Danger, "删除", self.selected.is_none()),
            ID_OPENDIR => (l.btn_opendir, BtnKind::Secondary, "打开数据目录", false),
            _ => return,
        };

        let rect = Rect::new(0, 0, r.w, r.h);
        let hovered = self.hover == Some(widget_of(id));
        let pressed = self.pressed == Some(widget_of(id));
        paint_button(
            &canvas,
            &self.fonts,
            rect,
            kind,
            text,
            l.scale,
            hovered,
            pressed,
            disabled,
        );
    }

    // -----------------------------------------------------------------------
    // 交互
    // -----------------------------------------------------------------------

    pub fn on_mouse_move(&mut self, x: i32, y: i32) {
        let l = self.layout;
        self.addr_hover = l.addr_field.contains(x, y);
    }

    pub fn on_mouse_leave(&mut self) {
        self.addr_hover = false;
        self.repaint();
    }

    /// 返回 true 表示消费了这次点击
    pub fn on_lbutton_down(&mut self, x: i32, y: i32) -> bool {
        let l = self.layout;
        // 列表区：点行 = 选中，点空白 = 取消选中
        if l.list.contains(x, y) {
            let row_h = l.list_row_h().max(1);
            let slot = ((y - l.list.y) / row_h) as usize;
            let idx = self.scroll + slot;
            self.selected = if idx < self.repo.releases.len() {
                Some(idx)
            } else {
                None
            };
            self.update_enabled();
            self.repaint();
            return true;
        }
        if l.list_header.contains(x, y) {
            return true;
        }
        false
    }

    pub fn on_mouse_wheel(&mut self, delta: i32) {
        let count = self.repo.releases.len();
        let visible = self.layout.visible_rows();
        if count <= visible {
            self.scroll = 0;
            return;
        }
        let max = count - visible;
        // 一格滚一行；WHEEL_DELTA = 120
        let steps = delta / 120;
        let next = (self.scroll as i32 - steps).clamp(0, max as i32);
        if next as usize != self.scroll {
            self.scroll = next as usize;
            self.repaint();
        }
    }

    // -----------------------------------------------------------------------
    // 服务启停
    // -----------------------------------------------------------------------

    pub fn start_server(&mut self) {
        if self.running() {
            return;
        }
        let bind = self.addr_text();
        let bind = if bind.trim().is_empty() {
            crate::config::DEFAULT_BIND.to_string()
        } else {
            bind.trim().to_string()
        };

        match Server::start(&bind, self.shared.clone()) {
            Ok(server) => {
                crate::tlog!("服务已启动，监听 {}", server.addr());
                // 落一条到日志文件：排查时第一件事就是确认"对方跑的是哪个版本"
                self.shared.push_log(
                    "本机".into(),
                    format!(
                        "服务端启动 v{}，监听 {}，数据目录 {}",
                        theme::APP_VERSION,
                        server.addr(),
                        self.data_dir.display()
                    ),
                    200,
                );
                self.config.bind = bind;
                if let Err(err) = self.config.save(&self.data_dir) {
                    crate::tlog!("保存配置失败：{err}");
                }
                self.server = Some(server);
                self.set_addr_text(&self.config.bind.clone());
            }
            Err(err) => {
                crate::tlog!("服务启动失败：{err}");
                self.error_box("启动失败", &err);
            }
        }
        self.update_enabled();
        self.repaint();
    }

    pub fn stop_server(&mut self) {
        if let Some(server) = self.server.take() {
            crate::tlog!("服务已停止（原监听 {}）", server.addr());
            drop(server); // Drop 里置停止标志，监听线程下一轮退出
        }
        self.update_enabled();
        self.repaint();
    }

    // -----------------------------------------------------------------------
    // 版本管理
    // -----------------------------------------------------------------------

    pub fn on_publish(&mut self) {
        let Some(file) = publish::pick_exe(self.hwnd) else {
            return;
        };
        // 版本号从 exe 资源里读，读不出来就没法发布（也说明选错了文件）
        let version = match store::read_exe_version(&file) {
            Ok(v) => v,
            Err(err) => {
                self.error_box("无法发布", &format!("{}\n\n{}", file.display(), err));
                return;
            }
        };

        let Some(params) = publish::ask_params(self.hwnd, &version, &file) else {
            return;
        };

        match store::publish(&self.data_dir, &file, &params.notes, params.force) {
            Ok(release) => {
                let is_newer = self
                    .repo
                    .latest_release()
                    .map(|cur| store::compare_version(&release.version, &cur.version).is_gt())
                    .unwrap_or(true);
                self.repo.add(release.clone());
                // 比当前最新版还新就自动顶上，否则只入库（避免误把旧版本设为对外版本）
                if is_newer || self.repo.latest.is_empty() {
                    self.repo.latest = release.version.clone();
                }
                if let Err(err) = self.repo.save(&self.data_dir) {
                    self.error_box("保存失败", &err);
                }
                self.sync_shared_repo();
                self.selected = Some(0);
                self.scroll = 0;
                self.info_box(
                    "发布成功",
                    &format!(
                        "版本 v{}\n大小 {}\n{}",
                        release.version,
                        store::human_size(release.size),
                        if is_newer { "已设为对外最新版本" } else { "已入库（比当前最新版旧，未改动对外版本）" }
                    ),
                );
            }
            Err(err) => self.error_box("发布失败", &err),
        }
        self.update_enabled();
        self.repaint();
    }

    pub fn on_set_latest(&mut self) {
        let releases = self.repo.sorted();
        let Some(idx) = self.selected else { return };
        let Some(release) = releases.get(idx) else {
            return;
        };
        let version = release.version.clone();
        if self.repo.latest == version {
            return;
        }
        let previous = self.repo.latest.clone();
        self.repo.latest = version.clone();
        if let Err(err) = self.repo.save(&self.data_dir) {
            self.error_box("保存失败", &err);
            return;
        }
        self.sync_shared_repo();
        self.info_box(
            "已切换对外版本",
            &format!("客户端现在会收到 v{version}\n（原来指向 {}）", if previous.is_empty() { "无" } else { &previous }),
        );
        self.repaint();
    }

    pub fn on_delete(&mut self) {
        let releases = self.repo.sorted();
        let Some(idx) = self.selected else { return };
        let Some(release) = releases.get(idx) else {
            return;
        };
        let version = release.version.clone();

        let (c, t) = (to_wide("删除版本"), to_wide(&format!(
            "确定删除 v{version} 吗？\n\n磁盘上的产物文件也会一并删除，此操作不可撤销。"
        )));
        let yes = unsafe {
            MessageBoxW(
                Some(self.hwnd),
                PCWSTR(t.as_ptr()),
                PCWSTR(c.as_ptr()),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
            )
        } == IDYES;
        if !yes {
            return;
        }

        match self.repo.remove(&self.data_dir, &version) {
            Ok(()) => {
                if let Err(err) = self.repo.save(&self.data_dir) {
                    self.error_box("保存失败", &err);
                }
                self.sync_shared_repo();
                self.selected = None;
                self.scroll = 0;
            }
            Err(err) => self.error_box("删除失败", &err),
        }
        self.update_enabled();
        self.repaint();
    }

    /// 把仓库的改动同步给 HTTP 服务线程（否则接口还在返回老版本）
    fn sync_shared_repo(&self) {
        if let Ok(mut repo) = self.shared.repo.lock() {
            *repo = self.repo.clone();
        }
    }

    pub fn on_open_data_dir(&self) {
        if let Err(err) = std::fs::create_dir_all(&self.data_dir) {
            self.error_box("打开失败", &format!("无法创建数据目录：{err}"));
            return;
        }
        let op = to_wide("open");
        let dir = to_wide(&self.data_dir.to_string_lossy());
        unsafe {
            ShellExecuteW(
                Some(self.hwnd),
                PCWSTR(op.as_ptr()),
                PCWSTR(dir.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        }
    }

    /// 定时刷新：只有真的变了才重绘，别每秒无脑刷
    pub fn on_timer(&mut self) {
        let hits = self.shared.hits.load(std::sync::atomic::Ordering::Relaxed);
        let logs = self.shared.snapshot_logs().len();
        if hits == self.last_hits && logs == self.last_logs {
            return;
        }
        self.last_hits = hits;
        self.last_logs = logs;

        // HTTP 线程只改内存里的下载计数，落盘由这里负责（见 http.rs 里的说明）。
        // 长度不同说明界面这边刚改过仓库、还没同步过去，别拿旧的回读覆盖它。
        if let Ok(repo) = self.shared.repo.lock() {
            if repo.releases.len() == self.repo.releases.len() {
                let counts_changed = repo
                    .releases
                    .iter()
                    .zip(self.repo.releases.iter())
                    .any(|(live, mine)| live.downloads != mine.downloads);
                self.repo = repo.clone();
                if counts_changed {
                    if let Err(err) = self.repo.save(&self.data_dir) {
                        crate::tlog!("保存下载计数失败：{err}");
                    }
                }
            }
        }
        self.repaint();
    }

    // -----------------------------------------------------------------------
    // 消息框
    // -----------------------------------------------------------------------

    fn box_with(&self, caption: &str, text: &str, style: MESSAGEBOX_STYLE) {
        let (c, t) = (to_wide(caption), to_wide(text));
        unsafe {
            MessageBoxW(Some(self.hwnd), PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), style);
        }
    }

    pub fn error_box(&self, caption: &str, text: &str) {
        self.box_with(caption, text, MB_OK | MB_ICONERROR);
    }

    pub fn info_box(&self, caption: &str, text: &str) {
        self.box_with(caption, text, MB_OK | MB_ICONINFORMATION);
    }
}

/// 控件 ID → 交互标识
pub fn widget_of(id: usize) -> Widget {
    match id {
        ID_MIN => Widget::Min,
        ID_CLOSE => Widget::Close,
        ID_START => Widget::Start,
        ID_STOP => Widget::Stop,
        ID_PUBLISH => Widget::Publish,
        ID_LATEST => Widget::SetLatest,
        ID_DELETE => Widget::Delete,
        _ => Widget::OpenDir,
    }
}

/// 控件 ID → 是否可用（子窗口过程用它决定要不要派发点击）
pub fn widget_enabled(app: &App, id: usize) -> bool {
    match id {
        ID_START => !app.running(),
        ID_STOP => app.running(),
        ID_LATEST | ID_DELETE => app.selected.is_some(),
        _ => true,
    }
}

/// 构建布局：按 DPI 算尺寸之后，还要按实测文字宽度摆放标题右侧的版本徽章。
fn build_layout(fonts: &Fonts, dpi: u32) -> Layout {
    let mut layout = Layout::compute_with(dpi as f32 / 96.0, fonts.body_line_h);
    layout.place_version_badge(
        text_width(theme::APP_TITLE, fonts.title),
        text_width(theme::VERSION_BADGE_TEXT, fonts.badge),
    );
    layout
}

// ---------------------------------------------------------------------------
// 窗口创建
// ---------------------------------------------------------------------------

pub fn main_instance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).map(|h| HINSTANCE(h.0)).unwrap_or_default() }
}

pub fn set_dpi_aware() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

pub fn window_dpi(hwnd: HWND) -> u32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        96
    } else {
        dpi
    }
}

/// 取挂在窗口上的 App（WM_NCCREATE 时挂上去的）
pub fn app_ref(hwnd: HWND) -> Option<&'static mut App> {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut App;
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { &mut *ptr })
    }
}

/// 从任意子控件往上找到根窗口，再取 App
pub fn root_app(hwnd: HWND) -> Option<&'static mut App> {
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    app_ref(root)
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
    let title = to_wide(&format!("{} v{}", theme::APP_TITLE, theme::APP_VERSION));

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
