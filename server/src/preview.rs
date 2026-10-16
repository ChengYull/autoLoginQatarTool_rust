//! 离屏渲染：把界面画到 GDI+ 位图并导出 PNG；另外负责生成 exe 图标资源。
//!
//! 用途是在没有真实桌面的情况下也能逐像素检查 UI：
//! `AutoLoginQatarServer.exe --render-preview <输出目录>`。

use std::fs;

use windows::core::{PCWSTR, GUID};
use windows::Win32::Graphics::GdiPlus::*;

use crate::gfx::{Canvas, Fonts, text_width};
use crate::http::LogEntry;
use crate::model::Widget;
use crate::paint::{paint_all, VisualState};
use crate::store::Release;
use crate::theme::{self, Layout};
use crate::util::to_wide;

const PIXEL_FORMAT_32BPP_ARGB: i32 = 0x0026_200A;

/// PNG 编码器的 CLSID（不是 ImageFormatPNG 那个"图像格式"GUID，传错会得到 0 字节文件）
const PNG_ENCODER_CLSID: GUID =
    GUID::from_u128(0x557cf406_1a04_11d3_9a73_0000f81ef32e);

pub struct PreviewCase {
    pub name: &'static str,
    pub running: bool,
    pub with_releases: bool,
    pub selected: Option<usize>,
    pub with_logs: bool,
    pub hover: Option<Widget>,
}

pub const CASES: [PreviewCase; 4] = [
    PreviewCase {
        name: "01-运行中-已发布版本",
        running: true,
        with_releases: true,
        selected: Some(0),
        with_logs: true,
        hover: None,
    },
    PreviewCase {
        name: "02-已停止-空仓库",
        running: false,
        with_releases: false,
        selected: None,
        with_logs: false,
        hover: None,
    },
    PreviewCase {
        name: "03-悬停发布按钮",
        running: true,
        with_releases: true,
        selected: None,
        with_logs: true,
        hover: Some(Widget::Publish),
    },
    PreviewCase {
        name: "04-悬停删除按钮",
        running: true,
        with_releases: true,
        selected: Some(1),
        with_logs: true,
        hover: Some(Widget::Delete),
    },
];

/// 演示用的假数据 —— 界面上出现的工号/地址必须是假的，仓库是公开的
fn demo_releases() -> Vec<Release> {
    vec![
        Release {
            version: "1.0.2".into(),
            notes: "修复登录延时偶发失效".into(),
            force: true,
            file: "AutoLoginQatarTool-1.0.2.exe".into(),
            size: 458_752,
            published_at: "2026-10-16 10:30:00".into(),
            downloads: 27,
        },
        Release {
            version: "1.0.1".into(),
            notes: "标题栏加版本徽章".into(),
            force: false,
            file: "AutoLoginQatarTool-1.0.1.exe".into(),
            size: 453_632,
            published_at: "2026-10-16 09:35:12".into(),
            downloads: 132,
        },
        Release {
            version: "1.0.0".into(),
            notes: "首个正式版本".into(),
            force: false,
            file: "AutoLoginQatarTool-1.0.0.exe".into(),
            size: 451_072,
            published_at: "2026-10-15 17:02:44".into(),
            downloads: 208,
        },
    ]
}

fn demo_logs() -> Vec<LogEntry> {
    let rows = [
        ("10:41:07", "10.170.8.42", "GET /api/version?current=1.0.1", 200),
        ("10:40:52", "10.170.8.42", "GET /api/download/1.0.2", 200),
        ("10:39:18", "10.170.9.15", "GET /api/version?current=1.0.0", 200),
        ("10:38:03", "10.170.9.15", "GET /api/version?current=1.0.2", 200),
        ("10:36:41", "10.170.8.42", "GET /api/health", 200),
    ];
    rows.iter()
        .map(|(time, peer, line, status)| LogEntry {
            time: (*time).into(),
            peer: (*peer).into(),
            line: (*line).into(),
            status: *status,
        })
        .collect()
}

pub fn render_case(dir: &str, scale: f32, case: &PreviewCase) -> Result<String, String> {
    let dpi = (96.0 * scale).round() as u32;
    let fonts = Fonts::new(dpi);
    let mut layout = Layout::compute(scale);
    layout.place_version_badge(
        text_width(theme::APP_TITLE, fonts.title),
        text_width(theme::VERSION_BADGE_TEXT, fonts.badge),
    );

    let releases_owned = if case.with_releases { demo_releases() } else { Vec::new() };
    let releases: Vec<&Release> = releases_owned.iter().collect();
    let logs = if case.with_logs { demo_logs() } else { Vec::new() };

    let vs = VisualState {
        running: case.running,
        bind: crate::config::DEFAULT_BIND,
        releases,
        latest: if case.with_releases { "1.0.2" } else { "" },
        selected: case.selected,
        scroll: 0,
        logs,
        total_hits: if case.with_logs { 1284 } else { 0 },
        total_downloads: if case.with_logs { 367 } else { 0 },
        hover: case.hover,
        pressed: None,
        addr_enabled: !case.running,
        addr_focus: false,
        addr_hover: false,
        start_enabled: !case.running,
        stop_enabled: case.running,
        publish_enabled: true,
        set_latest_enabled: case.selected.is_some(),
        delete_enabled: case.selected.is_some(),
    };

    let path = format!("{}/{}.png", dir.trim_end_matches(['/', '\\']), case.name);
    let wide_path = to_wide(&path);

    unsafe {
        let mut bitmap: *mut GpBitmap = std::ptr::null_mut();
        if GdipCreateBitmapFromScan0(
            layout.window.w,
            layout.window.h,
            0,
            PIXEL_FORMAT_32BPP_ARGB,
            None,
            &mut bitmap,
        )
        .0 != 0
            || bitmap.is_null()
        {
            return Err("无法创建离屏位图".into());
        }

        {
            let Some(canvas) = Canvas::from_bitmap(bitmap) else {
                GdipDisposeImage(bitmap as *mut GpImage);
                return Err("无法创建绘图上下文".into());
            };
            paint_all(&canvas, &layout, &fonts, &vs, true);
        }

        let st = GdipSaveImageToFile(
            bitmap as *mut GpImage,
            PCWSTR(wide_path.as_ptr()),
            &PNG_ENCODER_CLSID,
            std::ptr::null(),
        );
        GdipDisposeImage(bitmap as *mut GpImage);
        if st.0 != 0 {
            return Err(format!("保存 PNG 失败（状态码 {}）", st.0));
        }
    }

    // 注意写全路径：GdiPlus 里有个 `Status::Ok` 常量，glob 导入后会遮蔽 `Result::Ok`
    std::result::Result::Ok(path)
}

// ---------------------------------------------------------------------------
// exe 图标：把服务端图标渲染成多尺寸 ICO
// ---------------------------------------------------------------------------

/// 需要内嵌的尺寸。16/20/24/32/48 是资源管理器和任务栏实际会用到的，
/// 大尺寸给"超大图标"视图和以后可能用到的商店图。
const ICO_SIZES: [i32; 9] = [16, 20, 24, 32, 40, 48, 64, 128, 256];

/// 把服务端图标导出成 ICO 文件（`--export-icon`）。
///
/// ICO 里直接内嵌 PNG（Vista 以后都支持），所以不用手工拼 BMP 的 AND/XOR 掩码。
/// 先在临时目录落 PNG 再读回来，是为了避开 GDI+ 的 IStream 编码接口 ——
/// 那条路要自己管 `CreateStreamOnHGlobal` 的生命周期，不值得。
pub fn export_ico(path: &str) -> Result<(), String> {
    let mut images: Vec<(i32, Vec<u8>)> = Vec::new();
    for size in ICO_SIZES {
        images.push((size, render_icon_png(size)?));
    }

    let count = images.len();
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type = 1 (icon)
    out.extend_from_slice(&(count as u16).to_le_bytes());

    let mut offset = 6 + 16 * count;
    for (size, data) in &images {
        // 256 在目录项里用 0 表示
        let dim = if *size >= 256 { 0u8 } else { *size as u8 };
        out.push(dim);
        out.push(dim);
        out.push(0); // 调色板数（真彩为 0）
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bpp
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += data.len();
    }
    for (_, data) in &images {
        out.extend_from_slice(data);
    }

    fs::write(path, out).map_err(|e| format!("写入 {path} 失败：{e}"))
}

fn render_icon_png(size: i32) -> Result<Vec<u8>, String> {
    let tmp = std::env::temp_dir().join(format!("alq-server-icon-{size}.png"));
    let wide = to_wide(&tmp.to_string_lossy());

    unsafe {
        let mut bitmap: *mut GpBitmap = std::ptr::null_mut();
        if GdipCreateBitmapFromScan0(size, size, 0, PIXEL_FORMAT_32BPP_ARGB, None, &mut bitmap).0 != 0
            || bitmap.is_null()
        {
            return Err("无法创建图标位图".into());
        }
        if let Some(canvas) = Canvas::from_bitmap(bitmap) {
            crate::icon::draw_server(&canvas, 0, 0, size);
        }
        let st = GdipSaveImageToFile(
            bitmap as *mut GpImage,
            PCWSTR(wide.as_ptr()),
            &PNG_ENCODER_CLSID,
            std::ptr::null(),
        );
        GdipDisposeImage(bitmap as *mut GpImage);
        if st.0 != 0 {
            return Err(format!("保存临时 PNG 失败（状态码 {}）", st.0));
        }
    }

    let bytes = fs::read(&tmp).map_err(|e| format!("读取临时 PNG 失败：{e}"))?;
    let _ = fs::remove_file(&tmp);
    std::result::Result::Ok(bytes)
}
