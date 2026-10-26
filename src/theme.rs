//! 主题：颜色常量与布局计算。
//!
//! 颜色、尺寸全部对齐原 Qt 版 QSS（`#container` / `#input` / `#primaryBtn` 等），
//! 保证复刻后的观感一致。布局用「逻辑像素」描述，按 DPI 缩放因子换算到物理像素。

use crate::util::Rect;

// ---------------------------------------------------------------------------
// 颜色（0xRRGGBB）
// ---------------------------------------------------------------------------

pub const CARD_BG: u32 = 0xFF_FFFF;
pub const CARD_BORDER: u32 = 0xE6_EBF2;

pub const TITLE_TEXT: u32 = 0x0F_172A;
pub const WINBTN_FG: u32 = 0x94_A3B8;
pub const WINBTN_MIN_HOVER_BG: u32 = 0xF1_F5F9;
pub const WINBTN_MIN_HOVER_FG: u32 = 0x0F_172A;
pub const WINBTN_CLOSE_HOVER_BG: u32 = 0xEF_4444;
pub const WINBTN_CLOSE_HOVER_FG: u32 = 0xFF_FFFF;

pub const STATUS_CARD_BG: u32 = 0xF8_FAFC;
pub const STATUS_CARD_BORDER: u32 = 0xEE_F2F7;
pub const STATUS_TITLE: u32 = 0x33_4155;

pub const LABEL: u32 = 0x33_4155;
pub const SWITCH_DESC: u32 = 0x94_A3B8;

pub const INPUT_BG: u32 = 0xFF_FFFF;
pub const INPUT_BORDER: u32 = 0xDB_E2EA;
pub const INPUT_BORDER_HOVER: u32 = 0xC3_CCD8;
pub const INPUT_BORDER_FOCUS: u32 = 0x25_63EB;
pub const INPUT_TEXT: u32 = 0x0F_172A;
pub const INPUT_DISABLED_BG: u32 = 0xF8_FAFC;
pub const INPUT_DISABLED_TEXT: u32 = 0x94_A3B8;
pub const INPUT_DISABLED_BORDER: u32 = 0xEE_F2F7;
pub const PLACEHOLDER: u32 = 0x94_A3B8;

pub const RESET_FG: u32 = 0x25_63EB;
pub const RESET_HOVER_BG: u32 = 0xEF_F6FF;
pub const RESET_HOVER_BORDER: u32 = 0x93_C5FD;
pub const RESET_PRESSED_BG: u32 = 0xDB_EAFE;
pub const RESET_DISABLED_FG: u32 = 0xCB_D5E1;

pub const PRIMARY_BG: u32 = 0x25_63EB;
pub const PRIMARY_HOVER: u32 = 0x1D_4ED8;
pub const PRIMARY_PRESSED: u32 = 0x1E_40AF;
pub const PRIMARY_DISABLED_BG: u32 = 0xE2_E8F0;
pub const PRIMARY_FG: u32 = 0xFF_FFFF;

pub const SECONDARY_FG: u32 = 0x47_5569;
pub const SECONDARY_BORDER: u32 = 0xDB_E2EA;
pub const SECONDARY_HOVER_BG: u32 = 0xF8_FAFC;
pub const SECONDARY_HOVER_BORDER: u32 = 0x94_A3B8;
pub const SECONDARY_HOVER_FG: u32 = 0x0F_172A;
pub const SECONDARY_PRESSED_BG: u32 = 0xF1_F5F9;
pub const SECONDARY_DISABLED_FG: u32 = 0xCB_D5E1;

pub const DOT_RUNNING: u32 = 0x22_C55E;
pub const DOT_HANDLING: u32 = 0xF5_9E0B;
pub const DOT_STOPPED: u32 = 0x94_A3B8;

pub const BADGE_RUN_BG: u32 = 0xDC_FCE7;
pub const BADGE_RUN_FG: u32 = 0x15_803D;
pub const BADGE_HANDLE_BG: u32 = 0xFE_F3C7;
pub const BADGE_HANDLE_FG: u32 = 0xB4_5309;
pub const BADGE_STOP_BG: u32 = 0xF1_F5F9;
pub const BADGE_STOP_FG: u32 = 0x64_748B;

// 标题栏的版本徽章：绿色 = 已是最新，黄色 = 服务端上有更新版本。
// 直接复用上面状态徽章的绿 / 黄，让标题栏和状态卡片是一套配色。
pub const BADGE_VER_LATEST_BG: u32 = BADGE_RUN_BG;
pub const BADGE_VER_LATEST_FG: u32 = BADGE_RUN_FG;
pub const BADGE_VER_OLD_BG: u32 = BADGE_HANDLE_BG;
pub const BADGE_VER_OLD_FG: u32 = BADGE_HANDLE_FG;

pub const TOGGLE_OFF: u32 = 0xCB_D5E1;
pub const TOGGLE_ON: u32 = 0x25_63EB;
pub const TOGGLE_KNOB: u32 = 0xFF_FFFF;

pub const MENU_BG: u32 = 0xFF_FFFF;
pub const MENU_FG: u32 = 0x33_4155;
pub const MENU_SEL_BG: u32 = 0xEF_F6FF;
pub const MENU_SEL_FG: u32 = 0x1D_4ED8;
pub const MENU_DISABLED_FG: u32 = 0x94_A3B8;
pub const MENU_SEPARATOR: u32 = 0xEE_F2F7;

/// 密码不合规时的输入框描边与提示文字（1.0.5）
pub const DANGER_BORDER: u32 = 0xEF_4444;
pub const DANGER_TEXT: u32 = 0xDC_2626;

/// 相机图标上的多彩光圈（与 app.svg 一致）
pub const LENS_PETALS: [u32; 6] = [0x46_93E4, 0x7F_D04B, 0x4E_C1C6, 0xA8_D84A, 0x58_A9EA, 0x6C_C9A0];

// ---------------------------------------------------------------------------
// 文案
// ---------------------------------------------------------------------------

/// 窗口标题栏上的标题文字。绘制与文字宽度测量都要用它，
/// 写两处必然会有一天对不上，所以收在这里。
pub const APP_TITLE: &str = "自动登录工具";

// ---------------------------------------------------------------------------
// 尺寸（逻辑像素 @96dpi）
// ---------------------------------------------------------------------------

pub const CARD_W: f32 = 312.0;
/// 1.0.5 比 1.0.4 多了一行「下次重置：X 天后」（15px + 6px 间距），
/// 卡片高度相应 +21。
pub const CARD_H: f32 = 287.0;
pub const CARD_RADIUS: f32 = 14.0;

const TITLE_BAR_H: f32 = 42.0;
const PAD_X: f32 = 16.0;
const PAD_TOP: f32 = 2.0;
const PAD_BOTTOM: f32 = 16.0;
const GAP: f32 = 10.0;

const STATUS_CARD_H: f32 = 40.0;
const ROW_H: f32 = 30.0;
const SWITCH_ROW_H: f32 = 34.0;
const BTN_ROW_H: f32 = 32.0;

const LABEL_W: f32 = 36.0;
const LABEL_GAP: f32 = 8.0;
const RESET_W: f32 = 46.0;
const BTN_GAP: f32 = 10.0;
const WINBTN_W: f32 = 30.0;
const WINBTN_H: f32 = 28.0;
const TOGGLE_W: f32 = 46.0;
const TOGGLE_H: f32 = 26.0;
const FIELD_PAD_X: f32 = 10.0;
const EYE_SIZE: f32 = 18.0;

/// 版本徽章：高度与状态卡片里的「运行中」徽章一致，左右各留 8px 内边距
const VER_BADGE_H: f32 = 22.0;
const VER_BADGE_PAD_X: f32 = 8.0;
/// 版本徽章与标题文字之间的间距
const VER_BADGE_GAP: f32 = 6.0;

/// 96dpi 下 12px Microsoft YaHei UI 的实测行高（`tmHeight`）。
/// 只用于在拿不到字体度量的场合兜底（比如只为了算窗口尺寸的那次布局）。
const BODY_LINE_H_96: f32 = 17.0;

#[derive(Copy, Clone, Debug)]
pub struct Layout {
    pub scale: f32,
    pub window: Rect,
    pub title_bar: Rect,
    pub icon: Rect,
    pub title_text: Rect,
    /// 标题右侧的版本徽章。位置与宽度依赖实测文字宽度，
    /// 由 [`Layout::place_version_badge`] 摆放；在它之前是个空矩形（不绘制）。
    pub version_badge: Rect,
    pub btn_min: Rect,
    pub btn_close: Rect,
    /// 标题栏「三个点」，摆在最小化左边
    pub btn_more: Rect,
    pub status_card: Rect,
    pub status_dot: Rect,
    pub status_title: Rect,
    pub badge: Rect,
    pub account_label: Rect,
    pub account_field: Rect,
    pub account_edit: Rect,
    /// 预览用：和 `account_edit` 里的文字落在同一条基线上的整行矩形
    pub account_text: Rect,
    pub password_label: Rect,
    pub password_field: Rect,
    pub password_edit: Rect,
    pub password_text: Rect,
    pub eye: Rect,
    pub copy_btn: Rect,
    pub switch_title: Rect,
    pub switch_desc: Rect,
    pub toggle: Rect,
    /// 开关下方的一行小字：下次重置：X 天后
    pub next_reset: Rect,
    pub start_btn: Rect,
    pub stop_btn: Rect,
}

impl Layout {
    /// 只按缩放因子算布局（字体度量用 96dpi 下的实测值兜底）。
    /// 仅用于创建窗口前估算尺寸；真正用到的布局请走 [`Layout::compute_with`]。
    pub fn compute(scale: f32) -> Self {
        Self::compute_with(scale, (BODY_LINE_H_96 * scale).round() as i32)
    }

    /// `body_line_h` = 正文字体的 `TEXTMETRICW::tmHeight`（物理像素）。
    ///
    /// 原生单行 EDIT 把文字**顶对齐**到客户区上沿（基线 = 上沿 + `tmAscent`），
    /// 不是垂直居中。所以要让框里的字看起来居中，EDIT 的上沿必须摆在
    /// 「字段中心 − 行高 / 2」的位置 —— 这个位置随字体度量走，不能写死。
    pub fn compute_with(scale: f32, body_line_h: i32) -> Self {
        let s = |v: f32| (v * scale).round() as i32;
        let r = |x: f32, y: f32, w: f32, h: f32| Rect::new(s(x), s(y), s(w), s(h));
        let body_h = body_line_h.max(1);

        let win_w = CARD_W;
        let win_h = CARD_H;

        let title_bar = r(0.0, 0.0, win_w, TITLE_BAR_H);

        let icon = r(PAD_X, (TITLE_BAR_H - 18.0) / 2.0, 18.0, 18.0);
        let close_x = win_w - 10.0 - WINBTN_W;
        let min_x = close_x - 8.0 - WINBTN_W;
        let more_x = min_x - 8.0 - WINBTN_W;
        let btn_y = (TITLE_BAR_H - WINBTN_H) / 2.0;
        let btn_close = r(close_x, btn_y, WINBTN_W, WINBTN_H);
        let btn_min = r(min_x, btn_y, WINBTN_W, WINBTN_H);
        let btn_more = r(more_x, btn_y, WINBTN_W, WINBTN_H);
        let title_text = Rect::new(
            icon.right() + s(4.0),
            title_bar.y,
            (s(more_x) - icon.right() - s(4.0)).max(0),
            title_bar.h,
        );
        // 空矩形 = 还没摆。真实位置见 place_version_badge。
        let version_badge = Rect::new(0, 0, 0, 0);

        let inner_x = PAD_X;
        let inner_w = win_w - PAD_X * 2.0;
        let mut y = TITLE_BAR_H + PAD_TOP;

        let status_card = r(inner_x, y, inner_w, STATUS_CARD_H);
        let status_dot = r(inner_x + 12.0, y + (STATUS_CARD_H - 10.0) / 2.0, 10.0, 10.0);
        let status_title = r(inner_x + 12.0 + 10.0 + 8.0, y, 80.0, STATUS_CARD_H);
        let badge_w = 64.0;
        let badge = r(
            inner_x + inner_w - 12.0 - badge_w,
            y + (STATUS_CARD_H - 22.0) / 2.0,
            badge_w,
            22.0,
        );
        y += STATUS_CARD_H + GAP;

        let account_label = r(inner_x, y, LABEL_W, ROW_H);
        let account_field = r(inner_x + LABEL_W + LABEL_GAP, y, inner_w - LABEL_W - LABEL_GAP, ROW_H);
        let account_edit = Rect::new(
            account_field.x + s(FIELD_PAD_X),
            account_field.y + (account_field.h - body_h) / 2,
            account_field.w - s(FIELD_PAD_X) * 2,
            body_h + s(4.0),
        );
        let account_text = Rect::new(
            account_field.x + s(FIELD_PAD_X),
            account_field.y,
            account_field.w - s(FIELD_PAD_X) * 2,
            account_field.h,
        );
        y += ROW_H + GAP;

        let password_label = r(inner_x, y, LABEL_W, ROW_H);
        let copy_btn = r(inner_x + inner_w - RESET_W, y, RESET_W, ROW_H);
        let password_field = r(
            inner_x + LABEL_W + LABEL_GAP,
            y,
            (inner_x + inner_w - RESET_W - LABEL_GAP) - (inner_x + LABEL_W + LABEL_GAP),
            ROW_H,
        );
        let eye = Rect::new(
            password_field.right() - s(6.0) - s(EYE_SIZE),
            password_field.y + (password_field.h - s(EYE_SIZE)) / 2,
            s(EYE_SIZE),
            s(EYE_SIZE),
        );
        let password_edit = Rect::new(
            password_field.x + s(FIELD_PAD_X),
            password_field.y + (password_field.h - body_h) / 2,
            password_field.w - s(FIELD_PAD_X) - s(6.0 + EYE_SIZE + 4.0),
            body_h + s(4.0),
        );
        let password_text = Rect::new(
            password_field.x + s(FIELD_PAD_X),
            password_field.y,
            password_field.w - s(FIELD_PAD_X) - s(6.0 + EYE_SIZE + 4.0),
            password_field.h,
        );
        y += ROW_H + GAP;

        let switch_title = r(inner_x, y + 2.0, 200.0, 16.0);
        let switch_desc = r(inner_x, y + 2.0 + 16.0 + 2.0, 200.0, 15.0);
        let toggle = r(inner_x + inner_w - TOGGLE_W, y + (SWITCH_ROW_H - TOGGLE_H) / 2.0, TOGGLE_W, TOGGLE_H);
        y += SWITCH_ROW_H + GAP;

        // 密码轮换状态：一行小字，右对齐到卡片内边距
        let next_reset = r(inner_x, y, inner_w, 15.0);
        y += 15.0 + 6.0;

        let half = (inner_w - BTN_GAP) / 2.0;
        let start_btn = r(inner_x, y, half, BTN_ROW_H);
        let stop_btn = r(inner_x + half + BTN_GAP, y, half, BTN_ROW_H);
        y += BTN_ROW_H + PAD_BOTTOM;

        let window = Rect::new(0, 0, s(win_w), s(win_h));
        debug_assert!(y <= win_h + 0.5, "布局高度超出卡片");

        Layout {
            scale,
            window,
            title_bar,
            icon,
            title_text,
            version_badge,
            btn_min,
            btn_close,
            btn_more,
            status_card,
            status_dot,
            status_title,
            badge,
            account_label,
            account_field,
            account_edit,
            account_text,
            password_label,
            password_field,
            password_edit,
            password_text,
            eye,
            copy_btn,
            switch_title,
            switch_desc,
            toggle,
            next_reset,
            start_btn,
            stop_btn,
        }
    }

    /// 摆放标题右侧的版本徽章。
    ///
    /// 两个宽度都必须传**实测值**（`gfx::text_width`）：标题文字宽度决定徽章从哪里起，
    /// 版本文字宽度决定徽章有多宽。两者都随 DPI、字体和版本号位数（`v1.0.1` → `v1.10.2`）
    /// 变化，按字符数硬算迟早会错位 —— 版本号恰恰是最容易变长的那类文字。
    ///
    /// 必须在 [`Fonts`](crate::gfx::Fonts) 建好之后调用（要先有字体才量得出宽度）。
    pub fn place_version_badge(&mut self, title_w: i32, version_w: i32) {
        let s = |v: f32| (v * self.scale).round() as i32;
        let pad = s(VER_BADGE_PAD_X);
        let h = s(VER_BADGE_H);

        let x = self.title_text.x + title_w.max(0) + s(VER_BADGE_GAP);
        // 右侧留出到「三个点」按钮的距离，别压上去
        let limit = self.btn_more.x - s(VER_BADGE_GAP);
        let w = (version_w.max(1) + pad * 2).min((limit - x).max(0));

        let y = self.title_bar.y + (self.title_bar.h - h) / 2;
        self.version_badge = Rect::new(x, y, w, h);
    }
}
