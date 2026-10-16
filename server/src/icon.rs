//! 服务端图标：一台机架服务器（外框 + 三层分割线 + 指示灯）。
//!
//! 和客户端一样用 GDI+ 现画而不是内嵌位图：任意尺寸都清晰，也不用额外图片依赖。
//! 几何全部按 0..1 归一化后乘尺寸，所以 16px 的任务栏和 256px 的资源管理器
//! 用的是同一份描述。

use windows::Win32::Graphics::GdiPlus::*;
use windows::Win32::UI::WindowsAndMessaging::HICON;

use crate::gfx::{Canvas, Path};
use crate::util::{rgb, Rect};

const PIXEL_FORMAT_32BPP_ARGB: i32 = 0x0026_200A;

const FRAME: u32 = 0x25_63EB; // 机箱描边
const DIVIDER: u32 = 0x93_C5FD; // 分层线
const LAMP: u32 = 0x22_C55E; // 指示灯

/// 在 `(ox, oy)` 处画一个边长为 `size` 的图标
pub fn draw_server(canvas: &Canvas, ox: i32, oy: i32, size: i32) {
    let s = size as f32;
    let px = |v: f32| ox as f32 + v * s;
    let py = |v: f32| oy as f32 + v * s;
    let d = |v: f32| (v * s).max(1.0);

    // 机箱：外框描边
    let frame = Rect::new(
        px(0.10).round() as i32,
        py(0.10).round() as i32,
        d(0.80).round() as i32,
        d(0.80).round() as i32,
    );
    let stroke = (s * 0.075).max(1.0);
    if let Some(path) = Path::round_rect(frame, d(0.16)) {
        canvas.stroke_path(&path, rgb(FRAME), stroke);
    }

    // 三层：两条分割线 + 每层左侧一颗指示灯
    let lamp_r = 0.045;
    for i in 0..3 {
        let center = 0.10 + 0.80 * (i as f32 + 0.5) / 3.0;

        // 指示灯
        canvas.fill_ellipse(
            Rect::new(
                px(0.22 - lamp_r).round() as i32,
                py(center - lamp_r).round() as i32,
                d(lamp_r * 2.0).round() as i32,
                d(lamp_r * 2.0).round() as i32,
            ),
            rgb(LAMP),
        );

        // 分割线（最后一层下面没有线）
        if i < 2 {
            let ly = py(0.10 + 0.80 * (i as f32 + 1.0) / 3.0);
            canvas.line(
                px(0.36),
                ly,
                px(0.82),
                ly,
                rgb(DIVIDER),
                (s * 0.055).max(1.0),
            );
        }
    }
}

/// 把图标渲染成 HICON（窗口 / 任务栏共用）
pub fn create_hicon(size: i32) -> Option<HICON> {
    unsafe {
        let mut bmp: *mut GpBitmap = std::ptr::null_mut();
        if GdipCreateBitmapFromScan0(size, size, 0, PIXEL_FORMAT_32BPP_ARGB, None, &mut bmp).0 != 0
            || bmp.is_null()
        {
            return None;
        }

        if let Some(canvas) = Canvas::from_bitmap(bmp) {
            draw_server(&canvas, 0, 0, size);
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
