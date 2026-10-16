// 发布版不弹控制台窗口
#![windows_subsystem = "windows"]
// 窗口过程里大量用常量做 match 分支。若某个消息常量没被正确导入，
// Rust 会把它当成"变量绑定"而不是常量，于是它变成 catch-all，
// 后面的所有分支静默失效（只给一条 warning）。这里直接升级为编译错误。
#![deny(unreachable_patterns)]

mod app;
mod backdrop;
mod config;
mod gfx;
mod http;
mod icon;
mod model;
mod paint;
mod preview;
mod publish;
mod store;
mod theme;
mod trace;
mod util;
mod wnd;

use std::path::PathBuf;

use std::path::Path;

use windows::Win32::Graphics::Gdi::UpdateWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, ShowWindow, TranslateMessage, MSG, SW_SHOW,
};

/// 命令行发布：入库 + 设为对外最新版本。
///
/// 复用界面那套存储逻辑（`store::publish`），版本号同样从 exe 资源里读 ——
/// 命令行和界面两条路走的是同一份代码，不会出现"界面发出来的和命令行发出来的不一样"。
fn cli_publish(data_dir: &Path, file: &Path, notes: &str, force: bool) -> Result<String, String> {
    let mut repo = store::Repo::load(data_dir)?;
    let release = store::publish(data_dir, file, notes, force)?;
    let version = release.version.clone();
    repo.add(release);
    repo.latest = version.clone();
    repo.save(data_dir)?;
    Ok(version)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if let Some(idx) = args.iter().position(|a| a == "--selftest") {
        let path = args
            .get(idx + 1)
            .cloned()
            .unwrap_or_else(|| "selftest.log".into());
        trace::init(&path);
    }
    tlog!("main 进入，服务端版本 v{}", theme::APP_VERSION);

    // 数据目录：默认 exe 同级的 data/，可用 --data 覆盖
    let data_dir = args
        .iter()
        .position(|a| a == "--data")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(store::default_data_dir);
    app::set_data_dir(data_dir.clone());
    tlog!("数据目录 {}", data_dir.display());

    app::set_dpi_aware();

    // GDI+ 必须在整个进程生命周期内保持初始化
    let Some(_gdiplus) = gfx::GdiPlus::init() else {
        tlog!("GDI+ 初始化失败");
        return;
    };

    // 命令行发布：便于脚本化部署 / CI，也方便自动化验证（不需要点界面）
    if let Some(idx) = args.iter().position(|a| a == "--publish") {
        let Some(file) = args.get(idx + 1) else {
            tlog!("用法：--publish <exe 路径> [--notes \"更新说明\"] [--force]");
            std::process::exit(2);
        };
        let notes = args
            .iter()
            .position(|a| a == "--notes")
            .and_then(|i| args.get(i + 1))
            .cloned()
            .unwrap_or_default();
        let force = args.iter().any(|a| a == "--force");

        match cli_publish(&data_dir, PathBuf::from(file).as_path(), &notes, force) {
            Ok(version) => {
                tlog!("已发布 v{version}（对外最新版本已更新）");
                println!("published {version}");
            }
            Err(err) => {
                tlog!("发布失败：{err}");
                eprintln!("publish failed: {err}");
                std::process::exit(1);
            }
        }
        return;
    }

    // 生成 exe 图标资源用的多尺寸 ico（打包用，不需要桌面）
    if let Some(idx) = args.iter().position(|a| a == "--export-icon") {
        let path = args.get(idx + 1).cloned().unwrap_or_else(|| "server.ico".into());
        match preview::export_ico(&path) {
            Ok(()) => tlog!("图标已导出到 {path}"),
            Err(err) => tlog!("导出图标失败：{err}"),
        }
        return;
    }

    // 离屏渲染：把界面画成 PNG，便于在没有桌面的环境下检查视觉
    if let Some(idx) = args.iter().position(|a| a == "--render-preview") {
        let dir = args.get(idx + 1).cloned().unwrap_or_else(|| ".".into());
        let scale = args
            .get(idx + 2)
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(2.0);
        let _ = std::fs::create_dir_all(&dir);
        for case in preview::CASES.iter() {
            if let Err(err) = preview::render_case(&dir, scale, case) {
                tlog!("渲染 {} 失败：{err}", case.name);
            }
        }
        return;
    }

    let hinst = app::main_instance();
    if !wnd::register_all(hinst) {
        tlog!("注册窗口类失败");
        return;
    }

    let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForSystem() };
    tlog!("系统 DPI={dpi}");

    let hwnd = match app::create_main_window(dpi) {
        Ok(h) => h,
        Err(err) => {
            tlog!("创建主窗口失败: {}", err.message());
            return;
        }
    };

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
    }
    tlog!("主窗口已显示");

    if trace::enabled() {
        // 自检模式：4 秒后自动退出
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetTimer(Some(hwnd), 99, 4000, None);
        }
    }

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
