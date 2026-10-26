//! 图标绘制。
//!
//! 原版把 `assets/icons/app.svg` 用 QSvgRenderer 渲染成多尺寸位图给窗口 / 任务栏 /
//! 托盘用。这里不引入 SVG 解析库，而是直接用 GDI+ 把同一份 SVG 的几何重画一遍，
//! 好处是零依赖、任意尺寸都清晰，且不需要额外的图片插件。

use std::ffi::c_void;

use windows::Win32::Graphics::GdiPlus::*;
use windows::Win32::UI::WindowsAndMessaging::HICON;

use crate::gfx::{Canvas, Path};
use crate::theme;
use crate::util::{rgb, rgba, Rect};

const PIXEL_FORMAT_32BPP_ARGB: i32 = 0x0026_200A;

/// 相机图标：与 app.svg 的几何完全对应
/// （SVG 里对整组做了 translate(-23.04,-23.04) scale(1.18)）
pub fn draw_camera(canvas: &Canvas, ox: i32, oy: i32, size: i32) {
    let k = size as f32 / 256.0;
    let u = 1.18_f32;
    let t = -23.04_f32;

    let ox = ox as f32;
    let oy = oy as f32;
    // 坐标映射（含 SVG 的组变换）
    let m = |v: f32| (v * u + t) * k;
    // 尺寸映射
    let d = |v: f32| v * u * k;

    let rect = |x: f32, y: f32, w: f32, h: f32| {
        Rect::new(
            (ox + m(x)).round() as i32,
            (oy + m(y)).round() as i32,
            d(w).round().max(1.0) as i32,
            d(h).round().max(1.0) as i32,
        )
    };
    let circle = |cx: f32, cy: f32, r: f32| {
        Rect::new(
            (ox + m(cx) - d(r)).round() as i32,
            (oy + m(cy) - d(r)).round() as i32,
            (d(r) * 2.0).round().max(1.0) as i32,
            (d(r) * 2.0).round().max(1.0) as i32,
        )
    };

    // 机身：右侧快门凸台 + 机身主体
    canvas.fill_round_rect(rect(150.0, 50.0, 74.0, 32.0), d(12.0), rgb(0x3A_4048));
    canvas.fill_round_rect(rect(34.0, 72.0, 188.0, 134.0), d(28.0), rgb(0x3A_4048));
    // 取景窗凹槽
    canvas.fill_round_rect(rect(52.0, 84.0, 40.0, 15.0), d(7.5), rgb(0x2B_3138));

    // 镜头外圈
    canvas.fill_ellipse(circle(128.0, 140.0, 57.0), rgb(0x23_282E));

    // 六片多彩光圈
    let cx = ox + m(128.0);
    let cy = oy + m(140.0);
    let outer = d(48.0);
    let inner = d(20.0);
    for (i, color) in theme::LENS_PETALS.iter().enumerate() {
        let start = i as f32 * 60.0;
        if let Some(path) = Path::petal(cx, cy, outer, inner, start, 60.0) {
            canvas.fill_path(&path, rgb(*color));
        }
    }

    // 镜心
    canvas.fill_ellipse(circle(128.0, 140.0, 20.0), rgb(0x2B_3138));
    canvas.fill_ellipse(circle(128.0, 140.0, 7.5), rgb(0x5A_A9E8));
}

/// 密码框右侧的眼睛图标（对应 eye.svg / eye-off.svg）
pub fn draw_eye(canvas: &Canvas, r: Rect, off: bool, color: u32, stroke: f32) {
    let k = r.w as f32 / 24.0;
    let m = |x: f32, y: f32| (r.x as f32 + x * k, r.y as f32 + y * k);
    let s = |v: f32| v * k;

    // 眼形轮廓：24x24 视口里是一条 (2,12)-(22,12)、上下鼓起的闭合曲线，
    // 用上下两段椭圆弧等价表达。
    let (x0, y0) = m(2.0, 5.6);
    let outline = Rect::new(
        x0.round() as i32,
        y0.round() as i32,
        s(20.0).round().max(2.0) as i32,
        s(12.8).round().max(2.0) as i32,
    );
    if let Some(path) = Path::new_arc_eye(outline) {
        canvas.stroke_path(&path, rgba_from(color), stroke.max(1.0));
    }

    // 瞳孔
    let (px, py) = m(12.0, 12.0);
    let pr = s(3.1);
    canvas.stroke_ellipse(
        Rect::new(
            (px - pr).round() as i32,
            (py - pr).round() as i32,
            (pr * 2.0).round().max(2.0) as i32,
            (pr * 2.0).round().max(2.0) as i32,
        ),
        rgba_from(color),
        stroke.max(1.0),
    );

    if off {
        let (x1, y1) = m(3.6, 20.4);
        let (x2, y2) = m(20.4, 3.6);
        canvas.line(x1, y1, x2, y2, rgba_from(color), stroke.max(1.0));
    }
}

/// 标题栏「三个点」图标（1.0.5）
pub fn draw_more(canvas: &Canvas, r: Rect, color: u32) {
    let k = r.w as f32 / 24.0;
    let cy = r.y as f32 + 12.0 * k;
    for i in 0..3 {
        let cx = r.x as f32 + (5.0 + i as f32 * 7.0) * k;
        let rad = (1.7 * k).max(1.0);
        canvas.fill_ellipse(
            Rect::new(
                (cx - rad).round() as i32,
                (cy - rad).round() as i32,
                (rad * 2.0).round().max(1.0) as i32,
                (rad * 2.0).round().max(1.0) as i32,
            ),
            rgba_from(color),
        );
    }
}

/// 复制图标：两个错位的圆角方框（1.0.5，取代原来的「重置」文字按钮）
///
/// 先画后面那个（右下）的描边，再用按钮底色把前面那个填一遍 ——
/// 否则两条描边在重叠区交叉，缩小到 15px 就是一团糊。
pub fn draw_copy(canvas: &Canvas, r: Rect, fg: u32, bg: u32, stroke: f32) {
    let k = r.w as f32 / 24.0;
    let at = |x: f32, y: f32, w: f32, h: f32| {
        Rect::new(
            (r.x as f32 + x * k).round() as i32,
            (r.y as f32 + y * k).round() as i32,
            (w * k).round().max(2.0) as i32,
            (h * k).round().max(2.0) as i32,
        )
    };
    let radius = (2.6 * k).max(1.0);

    let back = at(8.5, 8.5, 12.5, 12.5);
    if let Some(p) = Path::round_rect(back, radius) {
        canvas.stroke_path(&p, rgba_from(fg), stroke);
    }

    let front = at(3.0, 3.0, 12.5, 12.5);
    canvas.fill_round_rect(front, radius, rgba_from(bg));
    if let Some(p) = Path::round_rect(front, radius) {
        canvas.stroke_path(&p, rgba_from(fg), stroke);
    }
}

fn rgba_from(hex: u32) -> u32 {
    rgba(
        ((hex >> 16) & 0xFF) as u8,
        ((hex >> 8) & 0xFF) as u8,
        (hex & 0xFF) as u8,
        255,
    )
}

/// 把相机图标渲染成 HICON（窗口 / 任务栏 / 托盘共用）
pub fn create_hicon(size: i32) -> Option<HICON> {
    unsafe {
        let mut bmp: *mut GpBitmap = std::ptr::null_mut();
        if GdipCreateBitmapFromScan0(size, size, 0, PIXEL_FORMAT_32BPP_ARGB, None, &mut bmp).0 != 0
            || bmp.is_null()
        {
            return None;
        }

        if let Some(canvas) = Canvas::from_bitmap(bmp) {
            draw_camera(&canvas, 0, 0, size);
        }

        let mut icon = HICON(std::ptr::null_mut());
        let st = GdipCreateHICONFromBitmap(bmp, &mut icon);
        GdipDisposeImage(bmp as *mut GpImage);
        if st.0 == 0 && !icon.0.is_null() {
            Some(icon)
        } else {
            None
        }
    }
}

// 让编译器确认 c_void 被引用（Path 的 FFI 指针类型）
#[allow(dead_code)]
fn _assert(_: *mut c_void) {}
