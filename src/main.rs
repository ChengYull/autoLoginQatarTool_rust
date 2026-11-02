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
mod upd_dlg;
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
        // auto-on：直接以「已开启」状态打开，用来核对锁定态的视觉。
        // 不能靠点「自动重置」进这个状态 —— 那会真的调生产重置接口。
        let (kind, auto_on) = match args.get(idx + 1).map(|s| s.as_str()) {
            Some("auto") => (dlg::Kind::Auto, false),
            Some("auto-on") => (dlg::Kind::Auto, true),
            _ => (dlg::Kind::Custom, false),
        };
        let sample: Vec<String> = (0..pwd::POOL_SIZE)
            .map(|i| format!("TestPass{}a.", i))
            .collect();
        let _ = dlg::open(
            hwnd,
            hinst,
            dpi,
            kind,
            // 调试用的假工号，别用真实工号 —— 仓库是公开的
            "10000001",
            &sample,
            auto_on,
            "下次重置：6 天后",
        );
    }

    // 调试入口：直接打开「发现新版本」对话框，核对历史版本列表的排版与滚动。
    // 真实路径要走「服务端有新版本 + 点徽章」，没有服务端时用这个看界面。
    if args.iter().any(|a| a == "--update-dialog") {
        let sample = "\
v1.0.6    2026-11-02 09:00    [最新]\r\n\
  新增服务端远程发布接口；更新说明改为展示全部历史版本，最新版排在最上面。\r\n\
  修掉 exe 属性里「文件说明」的乱码。\r\n\
\r\n\
v1.0.5    2026-10-26 10:37\r\n\
  新增密码自定义与密码自动重置接口；主页新增自动重置开关。\r\n\
\r\n\
v1.0.4    2026-10-19 14:57\r\n\
  自动更新：版本徽章变黄即代表有新版本，点它就能升级。\r\n\
\r\n\
v1.0.3    2026-10-19 11:09\r\n\
  修复设置密码成功被误判为失败。\r\n\
\r\n\
v1.0.2    2026-10-19 11:09\r\n\
  修复登录延时偶发失效；密码输入框支持明文切换。\r\n";
        let _ = upd_dlg::open(hwnd, hinst, dpi, version::APP_VERSION, "1.0.6", sample);
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
