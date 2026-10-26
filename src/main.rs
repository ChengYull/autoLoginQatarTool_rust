// 发布版不弹控制台窗口
#![windows_subsystem = "windows"]
// 窗口过程里大量用常量做 match 分支。若某个消息常量没被正确导入，
// Rust 会把它当成"变量绑定"而不是常量，于是它变成 catch-all，
// 后面的所有分支静默失效（只给一条 warning）。这里直接升级为编译错误。
#![deny(unreachable_patterns)]

mod app;
mod automation;
mod backdrop;
mod dlg;
mod gfx;
mod icon;
mod model;
mod net;
mod paint;
mod preview;
mod process;
mod pwd;
mod settings;
mod theme;
mod trace;
mod tray;
mod update;
mod util;
mod version;
mod wnd;

use windows::Win32::Graphics::Gdi::UpdateWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, ShowWindow, TranslateMessage, MSG, SW_SHOW,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // 诊断模式：把启动过程落盘，便于排查"窗口不出现"这类问题
    if let Some(idx) = args.iter().position(|a| a == "--selftest") {
        let path = args
            .get(idx + 1)
            .cloned()
            .unwrap_or_else(|| "selftest.log".into());
        trace::init(&path);
    }
    tlog!("main 进入，版本 v{}", version::APP_VERSION);

    app::set_dpi_aware();
    tlog!("DPI 感知已设置");

    // GDI+ 必须在整个进程生命周期内保持初始化
    let Some(_gdiplus) = gfx::GdiPlus::init() else {
        tlog!("GDI+ 初始化失败");
        return;
    };
    tlog!("GDI+ 初始化完成");

    // 离屏渲染模式：把界面画成 PNG，便于在没有桌面的环境下检查视觉
    if let Some(idx) = args.iter().position(|a| a == "--render-preview") {
        let dir = args.get(idx + 1).cloned().unwrap_or_else(|| ".".into());
        let scale = args
            .get(idx + 2)
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(2.0);
        let _ = std::fs::create_dir_all(&dir);
        let _ = preview::render_icon(&dir, 256);
        for case in preview::CASES.iter() {
            let _ = preview::render_case(&dir, scale, case);
        }
        return;
    }

    let hinst = app::main_instance();
    tlog!("hinstance={:?}", hinst.0);
    if !wnd::register_all(hinst) {
        tlog!("注册窗口类失败");
        return;
    }
    tlog!("窗口类注册完成");

    let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForSystem() };
    tlog!("系统 DPI={}", dpi);

    let hwnd = match app::create_main_window(dpi) {
        Ok(h) => h,
        Err(err) => {
            tlog!("创建主窗口失败: {}", err.message());
            return;
        }
    };
    tlog!("主窗口已创建 hwnd={:?}", hwnd.0);

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
    }
    tlog!("主窗口已显示");

    // 调试入口：直接打开密码对话框，方便在没有鼠标的环境下核对视觉
    if let Some(idx) = args.iter().position(|a| a == "--open-dialog") {
        let kind = match args.get(idx + 1).map(|s| s.as_str()) {
            Some("auto") => dlg::Kind::Auto,
            _ => dlg::Kind::Custom,
        };
        let sample: Vec<String> = (0..pwd::POOL_SIZE)
            .map(|i| format!("TestPass{}a.", i))
            .collect();
        let _ = dlg::open(
            hwnd,
            hinst,
            dpi,
            kind,
            "12322106",
            &sample,
            "下次重置：6 天后",
        );
    }

    if trace::enabled() {
        // 自检模式：4 秒后自动退出
        let tid = unsafe {
            windows::Win32::UI::WindowsAndMessaging::SetTimer(Some(hwnd), 99, 4000, None)
        };
        tlog!("自检定时器 id={}", tid);
    }

    tlog!("进入消息循环");
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
    }
    tlog!("消息循环结束");
}
