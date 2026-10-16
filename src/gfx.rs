//! GDI+ 绘图封装 + GDI 文本。
//!
//! 分工：GDI+ 负责抗锯齿的圆角矩形 / 圆 / 路径，GDI 负责文字（中文 ClearType
//! 渲染质量明显好于 GDI+ 的 DrawString）。两者通过 GdipGetDC / GdipReleaseDC
//! 交替使用同一个 HDC，这是微软文档给出的混用方式。

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, DrawTextW, GetDC, GetTextMetricsW, ReleaseDC, SelectObject,
    SetBkMode, SetTextColor, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DRAW_TEXT_FORMAT, FONT_QUALITY,
    HDC, HFONT, HGDIOBJ, OUT_TT_PRECIS, TEXTMETRICW, TRANSPARENT,
};
use windows::Win32::Graphics::GdiPlus::*;

use crate::util::{colorref, Rect};

// ---------------------------------------------------------------------------
// 文本对齐
// ---------------------------------------------------------------------------

pub const DT_LEFT: u32 = 0x0000;
pub const DT_CENTER: u32 = 0x0001;
pub const DT_VCENTER: u32 = 0x0004;
pub const DT_SINGLELINE: u32 = 0x0020;
pub const DT_NOPREFIX: u32 = 0x0800;

pub const ALIGN_LEFT: u32 = DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX;
pub const ALIGN_CENTER: u32 = DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX;

// ---------------------------------------------------------------------------
// GDI+ 生命周期
// ---------------------------------------------------------------------------

pub struct GdiPlus {
    token: usize,
}

impl GdiPlus {
    pub fn init() -> Option<Self> {
        let mut token: usize = 0;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            DebugEventCallback: 0,
            SuppressBackgroundThread: false.into(),
            SuppressExternalCodecs: false.into(),
        };
        let status = unsafe { GdiplusStartup(&mut token, &input, std::ptr::null_mut()) };
        if status.0 == 0 {
            Some(Self { token })
        } else {
            None
        }
    }
}

impl Drop for GdiPlus {
    fn drop(&mut self) {
        unsafe { GdiplusShutdown(self.token) };
    }
}

// ---------------------------------------------------------------------------
// 路径
// ---------------------------------------------------------------------------

pub struct Path(*mut GpPath);

impl Path {
    fn new() -> Option<Self> {
        let mut p: *mut GpPath = std::ptr::null_mut();
        let st = unsafe { GdipCreatePath(FillModeAlternate, &mut p) };
        if st.0 == 0 && !p.is_null() {
            Some(Path(p))
        } else {
            None
        }
    }

    /// 圆角矩形路径
    pub fn round_rect(r: Rect, radius: f32) -> Option<Self> {
        let path = Path::new()?;
        let max_r = (r.w.min(r.h) as f32) / 2.0;
        let rad = radius.min(max_r).max(0.0);
        let d = rad * 2.0;
        let (x, y, w, h) = (r.x as f32, r.y as f32, r.w as f32, r.h as f32);
        unsafe {
            GdipAddPathArc(path.0, x, y, d, d, 180.0, 90.0);
            GdipAddPathArc(path.0, x + w - d, y, d, d, 270.0, 90.0);
            GdipAddPathArc(path.0, x + w - d, y + h - d, d, d, 0.0, 90.0);
            GdipAddPathArc(path.0, x, y + h - d, d, d, 90.0, 90.0);
            GdipClosePathFigure(path.0);
        }
        Some(path)
    }

    /// 圆弧扇形（用于相机光圈叶片）
    pub fn petal(cx: f32, cy: f32, outer: f32, inner: f32, start_deg: f32, sweep_deg: f32) -> Option<Self> {
        let path = Path::new()?;
        unsafe {
            // 外弧（顺时针 sweep_deg）
            GdipAddPathArc(
                path.0,
                cx - outer,
                cy - outer,
                outer * 2.0,
                outer * 2.0,
                start_deg,
                sweep_deg,
            );
            // 收进内圈
            let a1 = (start_deg + sweep_deg).to_radians();
            GdipAddPathLine(
                path.0,
                cx + outer * a1.cos(),
                cy + outer * a1.sin(),
                cx + inner * a1.cos(),
                cy + inner * a1.sin(),
            );
            // 内弧返回起点
            GdipAddPathArc(
                path.0,
                cx - inner,
                cy - inner,
                inner * 2.0,
                inner * 2.0,
                start_deg + sweep_deg,
                -sweep_deg,
            );
            GdipClosePathFigure(path.0);
        }
        Some(path)
    }

    /// 眼形轮廓（上下两段椭圆弧拼成的闭合曲线）
    pub fn new_arc_eye(r: Rect) -> Option<Self> {
        let path = Path::new()?;
        let (x, y, w, h) = (r.x as f32, r.y as f32, r.w as f32, r.h as f32);
        unsafe {
            GdipAddPathArc(path.0, x, y, w, h, 180.0, 180.0);
            GdipAddPathArc(path.0, x, y, w, h, 0.0, 180.0);
            GdipClosePathFigure(path.0);
        }
        Some(path)
    }
}

impl Drop for Path {
    fn drop(&mut self) {
        unsafe { GdipDeletePath(self.0) };
    }
}

// ---------------------------------------------------------------------------
// 画布
// ---------------------------------------------------------------------------

pub struct Canvas {
    g: *mut GpGraphics,
}

impl Canvas {
    fn prepare(g: *mut GpGraphics) -> Canvas {
        let canvas = Canvas { g };
        unsafe {
            GdipSetSmoothingMode(canvas.g, SmoothingModeAntiAlias);
            GdipSetPixelOffsetMode(canvas.g, PixelOffsetModeHighQuality);
            GdipSetInterpolationMode(canvas.g, InterpolationModeHighQualityBicubic);
            GdipSetTextRenderingHint(canvas.g, TextRenderingHintClearTypeGridFit);
        }
        canvas
    }

    /// 从 HDC 创建（窗口绘制 / 内存 DC）
    pub fn from_hdc(hdc: HDC) -> Option<Self> {
        let mut g: *mut GpGraphics = std::ptr::null_mut();
        if unsafe { GdipCreateFromHDC(hdc, &mut g) }.0 != 0 || g.is_null() {
            return None;
        }
        Some(Self::prepare(g))
    }

    /// 从 GDI+ 位图创建（离屏预览）
    pub fn from_bitmap(bmp: *mut GpBitmap) -> Option<Self> {
        let mut g: *mut GpGraphics = std::ptr::null_mut();
        if unsafe { GdipGetImageGraphicsContext(bmp as *mut GpImage, &mut g) }.0 != 0 || g.is_null() {
            return None;
        }
        Some(Self::prepare(g))
    }

    fn solid(&self, color: u32) -> *mut GpSolidFill {
        let mut b: *mut GpSolidFill = std::ptr::null_mut();
        unsafe {
            GdipCreateSolidFill(color, &mut b);
        }
        b
    }

    fn pen(&self, color: u32, width: f32) -> *mut GpPen {
        let mut p: *mut GpPen = std::ptr::null_mut();
        unsafe {
            GdipCreatePen1(color, width, UnitPixel, &mut p);
        }
        p
    }

    pub fn fill_path(&self, path: &Path, color: u32) {
        let b = self.solid(color);
        if !b.is_null() {
            unsafe {
                GdipFillPath(self.g, b as *mut GpBrush, path.0);
                GdipDeleteBrush(b as *mut GpBrush);
            }
        }
    }

    pub fn stroke_path(&self, path: &Path, color: u32, width: f32) {
        let p = self.pen(color, width);
        if !p.is_null() {
            unsafe {
                GdipDrawPath(self.g, p, path.0);
                GdipDeletePen(p);
            }
        }
    }

    pub fn fill_round_rect(&self, r: Rect, radius: f32, color: u32) {
        if let Some(path) = Path::round_rect(r, radius) {
            self.fill_path(&path, color);
        }
    }

    /// 圆角矩形 + 描边，一次画完（描边压在填充上，边界更干净）
    pub fn round_rect_filled_bordered(
        &self,
        r: Rect,
        radius: f32,
        fill: u32,
        border: u32,
        border_width: f32,
    ) {
        if let Some(path) = Path::round_rect(r, radius) {
            self.fill_path(&path, fill);
            self.stroke_path(&path, border, border_width);
        }
    }

    pub fn fill_ellipse(&self, r: Rect, color: u32) {
        let b = self.solid(color);
        if !b.is_null() {
            unsafe {
                GdipFillEllipse(
                    self.g,
                    b as *mut GpBrush,
                    r.x as f32,
                    r.y as f32,
                    r.w as f32,
                    r.h as f32,
                );
                GdipDeleteBrush(b as *mut GpBrush);
            }
        }
    }

    pub fn stroke_ellipse(&self, r: Rect, color: u32, width: f32) {
        let p = self.pen(color, width);
        if !p.is_null() {
            unsafe {
                GdipDrawEllipse(self.g, p, r.x as f32, r.y as f32, r.w as f32, r.h as f32);
                GdipDeletePen(p);
            }
        }
    }

    pub fn fill_rect(&self, r: Rect, color: u32) {
        let b = self.solid(color);
        if !b.is_null() {
            unsafe {
                GdipFillRectangle(
                    self.g,
                    b as *mut GpBrush,
                    r.x as f32,
                    r.y as f32,
                    r.w as f32,
                    r.h as f32,
                );
                GdipDeleteBrush(b as *mut GpBrush);
            }
        }
    }

    pub fn line(&self, x1: f32, y1: f32, x2: f32, y2: f32, color: u32, width: f32) {
        let p = self.pen(color, width);
        if !p.is_null() {
            unsafe {
                GdipDrawLine(self.g, p, x1, y1, x2, y2);
                GdipDeletePen(p);
            }
        }
    }

    /// 用 GDI 在同一个 HDC 上绘制文字
    pub fn text(&self, s: &str, r: Rect, font: HFONT, color: u32, flags: u32) {
        let mut hdc = HDC(std::ptr::null_mut());
        unsafe {
            if GdipGetDC(self.g, &mut hdc).0 != 0 {
                return;
            }
        }
        draw_text(hdc, s, r, font, color, flags);
        unsafe {
            GdipReleaseDC(self.g, hdc);
        }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe { GdipDeleteGraphics(self.g) };
    }
}

/// 在指定 DC 上绘制文字（GDI）
pub fn draw_text(hdc: HDC, s: &str, r: Rect, font: HFONT, color: u32, flags: u32) {
    if s.is_empty() {
        return;
    }
    let mut buf: Vec<u16> = s.encode_utf16().collect();
    let mut rc: RECT = r.to_win();
    unsafe {
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(colorref(color)));
        DrawTextW(hdc, &mut buf, &mut rc, DRAW_TEXT_FORMAT(flags));
        SelectObject(hdc, old);
    }
}

// ---------------------------------------------------------------------------
// 字体
// ---------------------------------------------------------------------------

pub struct Fonts {
    pub title: HFONT, // 13px 600
    pub label: HFONT, // 12px 600
    pub body: HFONT,  // 12px 400
    pub small: HFONT, // 11px 400
    pub badge: HFONT, // 11px 600
    /// 正文字体的行高（`TEXTMETRICW::tmHeight`）。
    ///
    /// 布局需要它：原生单行 EDIT 把文字**顶对齐**到客户区上沿，
    /// 基线 = 上沿 + `tmAscent`。而 GDI 的 `DT_VCENTER` 是把整个字高盒子
    /// 垂直居中。两者要落在同一条基线上，就必须知道 `tmHeight`。
    pub body_line_h: i32,
}

const FW_NORMAL: i32 = 400;
const FW_SEMIBOLD: i32 = 600;
const CLEARTYPE_QUALITY_V: FONT_QUALITY = FONT_QUALITY(5);

impl Fonts {
    pub fn new(dpi: u32) -> Self {
        let body = make_font(12, FW_NORMAL, dpi);
        Self {
            title: make_font(13, FW_SEMIBOLD, dpi),
            label: make_font(12, FW_SEMIBOLD, dpi),
            body,
            small: make_font(11, FW_NORMAL, dpi),
            badge: make_font(11, FW_SEMIBOLD, dpi),
            body_line_h: line_height(body),
        }
    }
}

/// 查询字体行高；拿不到就退回 96dpi 下 12px 雅黑实测的 17px
fn line_height(font: HFONT) -> i32 {
    unsafe {
        let hdc = GetDC(None);
        if hdc.0.is_null() {
            return 17;
        }
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        let mut tm = TEXTMETRICW::default();
        let ok = GetTextMetricsW(hdc, &mut tm).as_bool() && tm.tmHeight > 0;
        SelectObject(hdc, old);
        ReleaseDC(None, hdc);
        if ok {
            tm.tmHeight
        } else {
            17
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            for f in [self.title, self.label, self.body, self.small, self.badge] {
                if !f.0.is_null() {
                    let _ = DeleteObject(HGDIOBJ(f.0));
                }
            }
        }
    }
}

fn make_font(px: i32, weight: i32, dpi: u32) -> HFONT {
    let height = -((px * dpi as i32 + 48) / 96);
    let face = crate::util::to_wide("Microsoft YaHei UI");
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_TT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY_V,
            0, // pitch and family
            PCWSTR(face.as_ptr()),
        )
    }
}
