//! 离屏渲染：把界面画到 GDI+ 位图并导出 PNG。
//!
//! 用途是在没有真实桌面的情况下也能逐像素检查 UI 效果 ——
//! `AutoLoginQatarTool.exe --render-preview <输出目录>`。

use windows::core::PCWSTR;
use windows::Win32::Graphics::GdiPlus::*;

use crate::gfx::{text_width, Canvas, Fonts};
use crate::model::{LoginState, Widget};
use crate::paint::{paint_all, VisualState};
use crate::theme::{self, Layout};
use crate::util::to_wide;
use crate::version::{self, VersionState};

const PIXEL_FORMAT_32BPP_ARGB: i32 = 0x0026_200A;

/// PNG 编码器的 CLSID。
/// 注意：`ImageFormatPNG` 是"图像格式"标识，而 GdipSaveImageToFile 要的是"编码器"
/// CLSID，两者 GUID 不同 —— 传错会得到一个 0 字节的空文件。
const PNG_ENCODER_CLSID: windows::core::GUID =
    windows::core::GUID::from_u128(0x557cf406_1a04_11d3_9a73_0000f81ef32e);

pub struct PreviewCase {
    pub name: &'static str,
    pub login_state: LoginState,
    pub account: &'static str,
    pub password: &'static str,
    pub password_visible: bool,
    pub auto_start: bool,
    pub hover: Option<Widget>,
    pub pressed: Option<Widget>,
    pub version_state: VersionState,
}

pub const CASES: [PreviewCase; 6] = [
    PreviewCase {
        name: "01-已停止",
        login_state: LoginState::Stopped,
        account: "",
        password: "",
        password_visible: false,
        auto_start: false,
        hover: None,
        pressed: None,
        version_state: VersionState::Latest,
    },
    PreviewCase {
        name: "02-运行中",
        login_state: LoginState::Running,
        account: "10012345",
        password: "Passw0rd!",
        password_visible: false,
        auto_start: true,
        hover: None,
        pressed: None,
        version_state: VersionState::Latest,
    },
    PreviewCase {
        name: "03-登录中-密码明文",
        login_state: LoginState::Handling,
        account: "10012345",
        password: "Passw0rd!",
        password_visible: true,
        auto_start: true,
        hover: None,
        pressed: None,
        version_state: VersionState::Latest,
    },
    PreviewCase {
        name: "04-悬停停止按钮",
        login_state: LoginState::Running,
        account: "10012345",
        password: "Passw0rd!",
        password_visible: false,
        auto_start: true,
        hover: Some(Widget::Stop),
        pressed: None,
        version_state: VersionState::Latest,
    },
    PreviewCase {
        name: "05-悬停关闭按钮",
        login_state: LoginState::Stopped,
        account: "",
        password: "",
        password_visible: false,
        auto_start: false,
        hover: Some(Widget::Close),
        pressed: None,
        version_state: VersionState::Latest,
    },
    // 版本徽章的黄色态：绿色那几张已经覆盖了「已是最新」，
    // 这里专门留一张核对「服务端上有更新版本」的观感
    PreviewCase {
        name: "06-版本有更新",
        login_state: LoginState::Running,
        account: "10012345",
        password: "Passw0rd!",
        password_visible: false,
        auto_start: true,
        hover: None,
        pressed: None,
        version_state: VersionState::Outdated,
    },
];

/// 单独把相机图标渲染成 PNG，用于核对图标几何
pub fn render_icon(dir: &str, size: i32) -> Result<String, String> {
    let path = format!("{}/00-图标.png", dir.trim_end_matches(['/', '\\']));
    let wide_path = to_wide(&path);

    unsafe {
        let mut bitmap: *mut GpBitmap = std::ptr::null_mut();
        if GdipCreateBitmapFromScan0(size, size, 0, PIXEL_FORMAT_32BPP_ARGB, None, &mut bitmap).0 != 0
            || bitmap.is_null()
        {
            return Err("无法创建位图".into());
        }
        if let Some(canvas) = Canvas::from_bitmap(bitmap) {
            // 浅灰底，便于看清透明区域
            canvas.fill_rect(
                crate::util::Rect::new(0, 0, size, size),
                crate::util::rgb(0xF2_F4F7),
            );
            crate::icon::draw_camera(&canvas, 0, 0, size);
        }
        let st = GdipSaveImageToFile(
            bitmap as *mut GpImage,
            PCWSTR(wide_path.as_ptr()),
            &PNG_ENCODER_CLSID,
            std::ptr::null(),
        );
        GdipDisposeImage(bitmap as *mut GpImage);
        if st.0 != 0 {
            return Err(format!("保存失败（{}）", st.0));
        }
    }
    std::result::Result::Ok(path)
}

pub fn render_case(dir: &str, scale: f32, case: &PreviewCase) -> Result<String, String> {
    let dpi = (96.0 * scale).round() as u32;
    let fonts = Fonts::new(dpi);
    // 先有字体才量得出文字宽度，版本徽章的位置和宽度都依赖它
    let mut layout = Layout::compute(scale);
    layout.place_version_badge(
        text_width(theme::APP_TITLE, fonts.title),
        text_width(version::BADGE_TEXT, fonts.badge),
    );

    let path = format!("{}/{}.png", dir.trim_end_matches(['/', '\\']), case.name);
    let wide_path = to_wide(&path);

    unsafe {
        let mut bitmap: *mut GpBitmap = std::ptr::null_mut();
        let st = GdipCreateBitmapFromScan0(
            layout.window.w,
            layout.window.h,
            0,
            PIXEL_FORMAT_32BPP_ARGB,
            None,
            &mut bitmap,
        );
        if st.0 != 0 || bitmap.is_null() {
            return Err("无法创建离屏位图".into());
        }

        {
            let Some(canvas) = Canvas::from_bitmap(bitmap) else {
                GdipDisposeImage(bitmap as *mut GpImage);
                return Err("无法创建绘图上下文".into());
            };
            let vs = VisualState {
                login_state: case.login_state,
                account: case.account,
                password: if case.password_visible {
                    case.password.to_string()
                } else {
                    "*".repeat(case.password.chars().count())
                },
                password_visible: case.password_visible,
                toggle_pos: if case.auto_start { 1.0 } else { 0.0 },
                hover: case.hover,
                pressed: case.pressed,
                fields_enabled: case.login_state == LoginState::Stopped,
                account_focus: false,
                password_focus: false,
                account_hover: false,
                password_hover: false,
                start_enabled: case.login_state == LoginState::Stopped,
                stop_enabled: case.login_state != LoginState::Stopped,
                copy_enabled: true,
                reset_desc: "下次重置：6 天后".into(),
                reset_pos: 1.0,
                version_state: case.version_state,
                updating: false,
                badge_hover: false,
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

    std::result::Result::Ok(path)
}
