//! 卡片背景层：一个逐像素 alpha 的分层窗口，永远贴在主窗口正下方。
//!
//! 为什么不直接把主窗口裁成圆角？`SetWindowRgn` 的区域是 **1 位掩码**，
//! 圆弧只能用整像素阶梯逼近。实测半径 14px 的圆角在深色壁纸上会露出
//! 4~5 级台阶，也就是肉眼可见的一圈毛刺 —— 这不是绘制质量问题，
//! 是「窗口形状」这一层根本表达不了灰度。
//!
//! 而真正能抗锯齿的逐像素 alpha 分层窗口（`UpdateLayeredWindow`）
//! 又不承载原生子控件（EDIT 会整片不显示）。所以把两件事拆开：
//!
//!   * 背景层（本模块）：抗锯齿的圆角卡片 + 1px 描边，不接受输入；
//!   * 主窗口：所有原生 / 自绘子控件，用**内缩 2px 的圆角区域**裁掉自己的硬边。
//!     它的边界完全落在背景层的白色卡片内部，两边同色，所以看不见；
//!     真正可见的轮廓只剩背景层那一条抗锯齿的边。
//!
//! 光栅化没有借助 GDI+：往 HDC 上画 GDI+ **不会写 alpha 通道**，
//! 而 `UpdateLayeredWindow` 要的是预乘 alpha。这里直接按 8x8 超采样算
//! 圆角矩形的像素覆盖率，得到 alpha = 覆盖率、RGB = 颜色 × 覆盖率，
//! 结果完全确定，也便于逐像素核对。

use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, EndPaint, GetDC,
    ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER,
    BLENDFUNCTION, DIB_RGB_COLORS, HBRUSH, HBITMAP, HDC, HGDIOBJ, PAINTSTRUCT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::theme;
use crate::util::to_wide;

pub const BACKDROP_CLASS: &str = "RustAutoLoginBackdrop";

/// 主窗口区域相对卡片边缘内缩的像素数。必须大于抗锯齿过渡带的宽度（约 1px）
/// 再加上取整误差，2px 足够；同时也不能太大，否则圆角外侧那一圈会明显变窄。
pub const REGION_INSET: f32 = 2.0;

// ---------------------------------------------------------------------------
// 窗口类
// ---------------------------------------------------------------------------

pub fn register(hinst: HINSTANCE) -> bool {
    let class = to_wide(BACKDROP_CLASS);
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(backdrop_wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinst,
        hIcon: HICON::default(),
        hCursor: HCURSOR::default(),
        hbrBackground: HBRUSH::default(),
        lpszMenuName: PCWSTR::null(),
        lpszClassName: PCWSTR(class.as_ptr()),
    };
    if unsafe { RegisterClassW(&wc) } == 0 {
        crate::tlog!("注册背景层窗口类失败");
        return false;
    }
    crate::tlog!("注册背景层窗口类 ok");
    true
}

unsafe extern "system" fn backdrop_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // 分层窗口的内容由 UpdateLayeredWindow 直接提交，不需要重绘
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let _ = BeginPaint(hwnd, &mut ps);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        // 背景层只露出主窗口外圈那 2px 和四个圆角。命中这里就自己吞掉，
        // 别让它穿透到桌面（穿透会误选中桌面图标），也绝不接受激活。
        WM_NCHITTEST => LRESULT(HTCLIENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// Backdrop
// ---------------------------------------------------------------------------

pub struct Backdrop {
    pub hwnd: HWND,
    /// 预乘 alpha 的 32bpp DIB，既是分层窗口的内容，也是我们的绘制缓冲
    bits: *mut c_void,
    memdc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    size: (i32, i32),
}

impl Backdrop {
    /// 建窗口 + 分配缓冲 + 首次提交。此时窗口尚未显示。
    pub fn new(hinst: HINSTANCE, scale: f32, size: (i32, i32)) -> Option<Self> {
        let class = to_wide(BACKDROP_CLASS);
        let empty = to_wide("");
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                PCWSTR(class.as_ptr()),
                PCWSTR(empty.as_ptr()),
                WS_POPUP,
                0,
                0,
                size.0.max(1),
                size.1.max(1),
                None,
                None,
                Some(hinst),
                None,
            )
        }
        .ok()?;

        let mut bd = Backdrop {
            hwnd,
            bits: std::ptr::null_mut(),
            memdc: HDC::default(),
            bmp: HBITMAP::default(),
            old: HGDIOBJ::default(),
            size: (0, 0),
        };
        if bd.rebuild(scale, size).is_none() {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return None;
        }
        Some(bd)
    }

    /// 重新分配缓冲并重画（DPI 变化或尺寸变化时）
    pub fn rebuild(&mut self, scale: f32, size: (i32, i32)) -> Option<()> {
        let (w, h) = (size.0.max(1), size.1.max(1));
        if self.size == (w, h) && !self.bits.is_null() {
            self.paint(scale);
            return Some(());
        }

        let screen = unsafe { GetDC(None) };
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h; // 负高度 = 自顶向下，与我们的扫描顺序一致
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB.0;

        let mut bits: *mut c_void = std::ptr::null_mut();
        let bmp = unsafe {
            CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
        };
        let memdc = unsafe { CreateCompatibleDC(Some(screen)) };
        unsafe { ReleaseDC(None, screen) };

        let bmp = match bmp {
            std::result::Result::Ok(b) if !bits.is_null() => b,
            _ => {
                crate::tlog!("背景层 DIB 分配失败");
                unsafe {
                    let _ = DeleteDC(memdc);
                }
                return None;
            }
        };
        let old = unsafe { SelectObject(memdc, HGDIOBJ(bmp.0)) };

        self.release_buffer();
        self.bits = bits;
        self.memdc = memdc;
        self.bmp = bmp;
        self.old = old;
        self.size = (w, h);

        self.paint(scale);
        Some(())
    }

    /// 把卡片画进缓冲并提交给分层窗口
    pub fn paint(&self, scale: f32) {
        if self.bits.is_null() {
            return;
        }
        let (w, h) = self.size;
        let buf = unsafe {
            std::slice::from_raw_parts_mut(self.bits as *mut u8, (w as usize) * (h as usize) * 4)
        };
        render_card(
            buf,
            w,
            h,
            theme::CARD_RADIUS * scale,
            theme::CARD_BG,
            theme::CARD_BORDER,
        );
        unsafe {
            let _ = self.commit(0, 0);
        }
    }

    /// 摆到主窗口正下方（含 z 序）并同步显隐
    pub fn sync(&self, main: HWND, visible: bool) {
        if self.hwnd.0.is_null() {
            return;
        }
        let mut rc = RECT::default();
        if unsafe { GetWindowRect(main, &mut rc) }.is_err() {
            return;
        }
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        if w <= 0 || h <= 0 {
            return;
        }
        unsafe {
            // hWndInsertAfter 传主窗口 = 插到主窗口后面（即正下方）
            let flags = SWP_NOACTIVATE
                | (if visible {
                    SWP_SHOWWINDOW
                } else {
                    SWP_HIDEWINDOW
                });
            let _ = SetWindowPos(self.hwnd, Some(main), rc.left, rc.top, w, h, flags);
        }
    }

    pub fn destroy(&mut self) {
        self.release_buffer();
        if !self.hwnd.0.is_null() {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
            self.hwnd = HWND::default();
        }
    }

    fn release_buffer(&mut self) {
        unsafe {
            if !self.old.0.is_null() {
                SelectObject(self.memdc, self.old);
                self.old = HGDIOBJ::default();
            }
            if !self.bmp.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(self.bmp.0));
                self.bmp = HBITMAP::default();
            }
            if !self.memdc.0.is_null() {
                let _ = DeleteDC(self.memdc);
                self.memdc = HDC::default();
            }
        }
        self.bits = std::ptr::null_mut();
    }

    unsafe fn commit(&self, x: i32, y: i32) -> windows::core::Result<()> {
        let pt_dst = POINT { x, y };
        let size = SIZE {
            cx: self.size.0,
            cy: self.size.1,
        };
        let pt_src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        UpdateLayeredWindow(
            self.hwnd,
            None,
            Some(&pt_dst),
            Some(&size),
            Some(self.memdc),
            Some(&pt_src),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
    }
}

impl Drop for Backdrop {
    fn drop(&mut self) {
        self.destroy();
    }
}

// ---------------------------------------------------------------------------
// 光栅化
// ---------------------------------------------------------------------------

/// 圆角矩形卡片 → BGRA（**预乘 alpha**）缓冲。
///
/// 描边占最外 1px：外层圆角矩形减内层圆角矩形就是描边环。
/// 覆盖率用 8x8 超采样算；离边界超过 1px 的像素直接走快路径，
/// 所以 312x266 的画布只需要对边界那一圈做真正的采样。
fn render_card(buf: &mut [u8], w: i32, h: i32, radius: f32, fill: u32, border: u32) {
    const SS: i32 = 8;
    const N: f32 = (SS * SS) as f32;

    let (fw, fh) = (w as f32, h as f32);
    let r_out = radius.max(0.0).min(fw / 2.0).min(fh / 2.0);
    let r_in = (r_out - 1.0).max(0.0);
    let (in_x, in_y, in_w, in_h) = (1.0, 1.0, fw - 2.0, fh - 2.0);

    let fr = ((fill >> 16) & 0xFF) as f32;
    let fg = ((fill >> 8) & 0xFF) as f32;
    let fb = (fill & 0xFF) as f32;
    let br = ((border >> 16) & 0xFF) as f32;
    let bg = ((border >> 8) & 0xFF) as f32;
    let bb = (border & 0xFF) as f32;

    let step = 1.0 / SS as f32;

    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;

            let cx = x as f32 + 0.5;
            let cy = y as f32 + 0.5;
            let sd_out = sd_round_rect(cx, cy, 0.0, 0.0, fw, fh, r_out);
            let sd_in = sd_round_rect(cx, cy, in_x, in_y, in_w, in_h, r_in);

            // 完全在内层矩形里 → 纯填充
            if sd_in <= -0.75 {
                buf[i] = fb as u8;
                buf[i + 1] = fg as u8;
                buf[i + 2] = fr as u8;
                buf[i + 3] = 255;
                continue;
            }
            // 完全在外层矩形外 → 全透明
            if sd_out >= 0.75 {
                buf[i] = 0;
                buf[i + 1] = 0;
                buf[i + 2] = 0;
                buf[i + 3] = 0;
                continue;
            }

            let mut cov_out = 0u32;
            let mut cov_in = 0u32;
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f32 + (sx as f32 + 0.5) * step;
                    let py = y as f32 + (sy as f32 + 0.5) * step;
                    if sd_round_rect(px, py, 0.0, 0.0, fw, fh, r_out) <= 0.0 {
                        cov_out += 1;
                    }
                    if sd_round_rect(px, py, in_x, in_y, in_w, in_h, r_in) <= 0.0 {
                        cov_in += 1;
                    }
                }
            }

            let a_out = cov_out as f32 / N;
            let a_in = cov_in as f32 / N;
            let a_bd = (a_out - a_in).max(0.0);

            // 预乘 alpha：每个通道直接写「颜色 × 覆盖率」的和
            let r = fr * a_in + br * a_bd;
            let g = fg * a_in + bg * a_bd;
            let b = fb * a_in + bb * a_bd;

            buf[i] = b.round().clamp(0.0, 255.0) as u8;
            buf[i + 1] = g.round().clamp(0.0, 255.0) as u8;
            buf[i + 2] = r.round().clamp(0.0, 255.0) as u8;
            buf[i + 3] = (a_out * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// 点到圆角矩形边界的有符号距离（负 = 在内）
fn sd_round_rect(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let qx = (px - (x + w / 2.0)).abs() - (w / 2.0 - r);
    let qy = (py - (y + h / 2.0)).abs() - (h / 2.0 - r);
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
}
