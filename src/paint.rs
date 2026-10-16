//! 界面绘制。所有组件都对齐原版 QSS 的配色与圆角。

use crate::gfx::{ALIGN_CENTER, ALIGN_LEFT, Canvas, Fonts};
use crate::icon::{draw_camera, draw_eye};
use crate::model::{BtnKind, LoginState, Widget};
use crate::theme::*;
use crate::util::{Rect, rgb, rgba};
use crate::version::{self, VersionState};

/// 一次绘制所需的全部视觉状态
pub struct VisualState<'a> {
    pub login_state: LoginState,
    pub account: &'a str,
    pub password: String,
    pub password_visible: bool,
    pub toggle_pos: f32,
    pub hover: Option<Widget>,
    pub pressed: Option<Widget>,
    /// 账户 / 密码输入框是否可用（运行中不可改）
    pub fields_enabled: bool,
    pub account_focus: bool,
    pub password_focus: bool,
    pub account_hover: bool,
    pub password_hover: bool,
    /// 主按钮是否可用
    pub start_enabled: bool,
    pub stop_enabled: bool,
    pub reset_enabled: bool,
    /// 当前版本相对服务端的状态，决定标题栏版本徽章的颜色（绿 = 最新，黄 = 有更新）
    pub version_state: VersionState,
}

impl<'a> VisualState<'a> {
    fn hovered(&self, w: Widget) -> bool {
        self.hover == Some(w)
    }
    fn pressed(&self, w: Widget) -> bool {
        self.pressed == Some(w)
    }
}

/// 绘制整窗。
///
/// `widgets` 为 true 时连同子控件（按钮 / 开关 / 眼睛 / 输入文本）一起画，
/// 用于离屏预览；实际运行时子控件是独立窗口，各自绘制，故传 false。
pub fn paint_all(canvas: &Canvas, l: &Layout, f: &Fonts, vs: &VisualState, widgets: bool) {
    if widgets {
        // 离屏预览：这里没有背景层窗口，卡片自己画全（含 1px 描边）
        canvas.round_rect_filled_bordered(
            l.window,
            CARD_RADIUS * l.scale,
            rgb(CARD_BG),
            rgb(CARD_BORDER),
            1.0,
        );
    } else {
        // 运行时：卡片外观（抗锯齿圆角 + 描边）由背景层窗口负责，
        // 主窗口只负责铺底色 —— 它被裁成内缩 2px 的圆角区域，
        // 边界落在背景层的白色卡片内部，同色所以看不见。
        canvas.fill_rect(l.window, rgb(CARD_BG));
    }

    // 标题栏
    draw_camera(canvas, l.icon.x, l.icon.y, l.icon.w);
    canvas.text(APP_TITLE, l.title_text, f.title, rgb(TITLE_TEXT), ALIGN_LEFT);
    paint_version_badge(canvas, l, f, vs.version_state);

    // 运行状态卡片
    paint_status_card(canvas, l, f, vs);

    // 字段标签
    canvas.text("账户", l.account_label, f.label, rgb(LABEL), ALIGN_LEFT);
    canvas.text("密码", l.password_label, f.label, rgb(LABEL), ALIGN_LEFT);
    canvas.text("开机自启动", l.switch_title, f.label, rgb(LABEL), ALIGN_LEFT);
    canvas.text("随 Windows 启动后常驻系统托盘", l.switch_desc, f.small, rgb(SWITCH_DESC), ALIGN_LEFT);

    // 输入框外框
    paint_field(
        canvas,
        l.account_field,
        l.scale,
        vs.account_focus,
        vs.account_hover,
        !vs.fields_enabled,
    );
    paint_field(
        canvas,
        l.password_field,
        l.scale,
        vs.password_focus,
        vs.password_hover,
        !vs.fields_enabled,
    );

    if !widgets {
        return;
    }

    // ---- 以下仅离屏预览需要（真实运行时由子控件自绘）----

    // 标题栏按钮
    paint_button(canvas, f, l.btn_min, BtnKind::WinMin, "—", l.scale, vs.hovered(Widget::Min), vs.pressed(Widget::Min), false);
    paint_button(canvas, f, l.btn_close, BtnKind::WinClose, "✕", l.scale, vs.hovered(Widget::Close), vs.pressed(Widget::Close), false);

    // 输入框内容。用整行矩形而不是 EDIT 的矩形：DT_VCENTER 会把字高盒子
    // 居中，而原生 EDIT 是顶对齐 —— 两者只有基于「整行」时才落在同一条基线上。
    paint_field_text(canvas, f, l.account_text, vs.account, "Emp ID / 员工ID / Uid", vs.fields_enabled);
    paint_field_text(canvas, f, l.password_text, &vs.password, "请输入客户端登录密码", vs.fields_enabled);

    // 眼睛 + 重置
    draw_eye(
        canvas,
        l.eye,
        !vs.password_visible,
        if vs.fields_enabled { WINBTN_FG } else { INPUT_DISABLED_TEXT },
        1.8 * l.scale,
    );
    paint_button(canvas, f, l.reset_btn, BtnKind::Reset, "重置", l.scale, vs.hovered(Widget::Reset), vs.pressed(Widget::Reset), !vs.reset_enabled);

    // 开机自启开关
    paint_toggle(canvas, l.toggle, vs.toggle_pos);

    // 启动 / 停止
    paint_button(canvas, f, l.start_btn, BtnKind::Primary, "启动", l.scale, vs.hovered(Widget::Start), vs.pressed(Widget::Start), !vs.start_enabled);
    paint_button(canvas, f, l.stop_btn, BtnKind::Secondary, "停止", l.scale, vs.hovered(Widget::Stop), vs.pressed(Widget::Stop), !vs.stop_enabled);
}

// ---------------------------------------------------------------------------
// 版本徽章
// ---------------------------------------------------------------------------

/// 标题右侧的版本徽章。
///
/// 外观与状态卡片里的「运行中」徽章一致（胶囊圆角 + 浅底深字），只用颜色区分新旧：
/// 绿色 = 当前已是最新，黄色 = 服务端上有更新版本。未摆放时（宽 0）不画。
fn paint_version_badge(canvas: &Canvas, l: &Layout, f: &Fonts, state: VersionState) {
    let r = l.version_badge;
    if r.w <= 0 || r.h <= 0 {
        return;
    }
    let (bg, fg) = state.badge_colors();
    canvas.fill_round_rect(r, r.h as f32 / 2.0, rgb(bg));
    canvas.text(version::BADGE_TEXT, r, f.badge, rgb(fg), ALIGN_CENTER);
}

// ---------------------------------------------------------------------------
// 运行状态卡片
// ---------------------------------------------------------------------------

fn paint_status_card(canvas: &Canvas, l: &Layout, f: &Fonts, vs: &VisualState) {
    canvas.round_rect_filled_bordered(
        l.status_card,
        10.0 * l.scale,
        rgb(STATUS_CARD_BG),
        rgb(STATUS_CARD_BORDER),
        1.0,
    );

    let (dot, badge_bg, badge_fg, badge_text) = match vs.login_state {
        LoginState::Running => (DOT_RUNNING, BADGE_RUN_BG, BADGE_RUN_FG, "运行中"),
        LoginState::Handling => (DOT_HANDLING, BADGE_HANDLE_BG, BADGE_HANDLE_FG, "登录中"),
        LoginState::Stopped => (DOT_STOPPED, BADGE_STOP_BG, BADGE_STOP_FG, "已停止"),
    };

    canvas.fill_ellipse(l.status_dot, rgb(dot));
    canvas.text("运行状态", l.status_title, f.label, rgb(STATUS_TITLE), ALIGN_LEFT);

    canvas.fill_round_rect(l.badge, l.badge.h as f32 / 2.0, rgb(badge_bg));
    canvas.text(badge_text, l.badge, f.badge, rgb(badge_fg), ALIGN_CENTER);
}

// ---------------------------------------------------------------------------
// 输入框
// ---------------------------------------------------------------------------

fn paint_field(canvas: &Canvas, r: Rect, scale: f32, focused: bool, hovered: bool, disabled: bool) {
    let (bg, border) = if disabled {
        (INPUT_DISABLED_BG, INPUT_DISABLED_BORDER)
    } else if focused {
        (INPUT_BG, INPUT_BORDER_FOCUS)
    } else if hovered {
        (INPUT_BG, INPUT_BORDER_HOVER)
    } else {
        (INPUT_BG, INPUT_BORDER)
    };
    canvas.round_rect_filled_bordered(r, 8.0 * scale, rgb(bg), rgb(border), 1.0);
}

fn paint_field_text(canvas: &Canvas, f: &Fonts, r: Rect, value: &str, placeholder: &str, enabled: bool) {
    if value.is_empty() {
        canvas.text(placeholder, r, f.body, rgb(PLACEHOLDER), ALIGN_LEFT);
    } else {
        let color = if enabled { INPUT_TEXT } else { INPUT_DISABLED_TEXT };
        canvas.text(value, r, f.body, rgb(color), ALIGN_LEFT);
    }
}

// ---------------------------------------------------------------------------
// 按钮
// ---------------------------------------------------------------------------

pub fn paint_button(
    canvas: &Canvas,
    f: &Fonts,
    r: Rect,
    kind: BtnKind,
    text: &str,
    scale: f32,
    hovered: bool,
    pressed: bool,
    disabled: bool,
) {
    match kind {
        BtnKind::WinMin | BtnKind::WinClose => {
            let is_close = kind == BtnKind::WinClose;
            let hover_bg = if is_close { WINBTN_CLOSE_HOVER_BG } else { WINBTN_MIN_HOVER_BG };
            let hover_fg = if is_close { WINBTN_CLOSE_HOVER_FG } else { WINBTN_MIN_HOVER_FG };
            let radius = 7.0 * scale;

            let (fg, has_bg) = if disabled {
                (WINBTN_FG, false)
            } else if hovered || pressed {
                canvas.fill_round_rect(r, radius, rgb(hover_bg));
                (hover_fg, true)
            } else {
                (WINBTN_FG, false)
            };
            let _ = has_bg;

            // 手绘最小化 / 关闭图形，比字形更锐利
            let stroke = (1.4 * scale).max(1.0);
            if is_close {
                let d = (4.5 * scale).round() as i32;
                canvas.line(
                    (r.cx() - d) as f32,
                    (r.cy() - d) as f32,
                    (r.cx() + d) as f32,
                    (r.cy() + d) as f32,
                    rgba_from(fg),
                    stroke,
                );
                canvas.line(
                    (r.cx() - d) as f32,
                    (r.cy() + d) as f32,
                    (r.cx() + d) as f32,
                    (r.cy() - d) as f32,
                    rgba_from(fg),
                    stroke,
                );
            } else {
                let w = (5.0 * scale).round() as i32;
                canvas.line(
                    (r.cx() - w) as f32,
                    r.cy() as f32,
                    (r.cx() + w) as f32,
                    r.cy() as f32,
                    rgba_from(fg),
                    stroke,
                );
            }
        }
        BtnKind::Reset => {
            let (bg, border, fg) = if disabled {
                (INPUT_DISABLED_BG, INPUT_DISABLED_BORDER, RESET_DISABLED_FG)
            } else if pressed {
                (RESET_PRESSED_BG, RESET_HOVER_BORDER, RESET_FG)
            } else if hovered {
                (RESET_HOVER_BG, RESET_HOVER_BORDER, RESET_FG)
            } else {
                (INPUT_BG, INPUT_BORDER, RESET_FG)
            };
            canvas.round_rect_filled_bordered(r, 8.0 * scale, rgb(bg), rgb(border), 1.0);
            canvas.text(text, r, f.label, rgb(fg), ALIGN_CENTER);
        }
        BtnKind::Primary => {
            let (bg, fg) = if disabled {
                (PRIMARY_DISABLED_BG, WINBTN_FG)
            } else if pressed {
                (PRIMARY_PRESSED, PRIMARY_FG)
            } else if hovered {
                (PRIMARY_HOVER, PRIMARY_FG)
            } else {
                (PRIMARY_BG, PRIMARY_FG)
            };
            canvas.fill_round_rect(r, 8.0 * scale, rgb(bg));
            canvas.text(text, r, f.label, rgb(fg), ALIGN_CENTER);
        }
        BtnKind::Secondary => {
            let (bg, border, fg) = if disabled {
                (INPUT_DISABLED_BG, INPUT_DISABLED_BORDER, SECONDARY_DISABLED_FG)
            } else if pressed {
                (SECONDARY_PRESSED_BG, SECONDARY_BORDER, SECONDARY_FG)
            } else if hovered {
                (SECONDARY_HOVER_BG, SECONDARY_HOVER_BORDER, SECONDARY_HOVER_FG)
            } else {
                (INPUT_BG, SECONDARY_BORDER, SECONDARY_FG)
            };
            canvas.round_rect_filled_bordered(r, 8.0 * scale, rgb(bg), rgb(border), 1.0);
            canvas.text(text, r, f.label, rgb(fg), ALIGN_CENTER);
        }
    }
}

// ---------------------------------------------------------------------------
// 滑动开关
// ---------------------------------------------------------------------------

pub fn paint_toggle(canvas: &Canvas, r: Rect, position: f32) {
    let p = position.clamp(0.0, 1.0);
    // 轨道颜色随滑块位置在关 / 开两色之间平滑过渡
    let track = mix(TOGGLE_OFF, TOGGLE_ON, p);
    canvas.fill_round_rect(r, r.h as f32 / 2.0, rgba_from(track));

    let margin = (3.0 * r.h as f32 / 26.0).round() as i32;
    let knob_d = r.h - margin * 2;
    let travel = (r.w - knob_d - margin * 2) as f32;
    let knob_x = r.x + margin + (travel * p).round() as i32;
    canvas.fill_ellipse(
        Rect::new(knob_x, r.y + margin, knob_d, knob_d),
        rgb(TOGGLE_KNOB),
    );
}

fn mix(from: u32, to: u32, t: f32) -> u32 {
    let f = |shift: u32| ((from >> shift) & 0xFF) as f32;
    let g = |shift: u32| ((to >> shift) & 0xFF) as f32;
    let lerp = |a: f32, b: f32| (a + (b - a) * t).round().clamp(0.0, 255.0) as u32;
    (lerp(f(16), g(16)) << 16) | (lerp(f(8), g(8)) << 8) | lerp(f(0), g(0))
}

fn rgba_from(hex: u32) -> u32 {
    rgba(
        ((hex >> 16) & 0xFF) as u8,
        ((hex >> 8) & 0xFF) as u8,
        (hex & 0xFF) as u8,
        255,
    )
}
