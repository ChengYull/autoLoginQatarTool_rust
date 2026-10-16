//! 主题：颜色常量与布局计算。
//!
//! 配色直接沿用客户端（`autoLoginQatarTool_rust/src/theme.rs`）那套，
//! 两个程序摆在同一个桌面上应当像一家人。

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
pub const HINT: u32 = 0x94_A3B8;

pub const INPUT_BG: u32 = 0xFF_FFFF;
pub const INPUT_BORDER: u32 = 0xDB_E2EA;
pub const INPUT_BORDER_HOVER: u32 = 0xC3_CCD8;
pub const INPUT_BORDER_FOCUS: u32 = 0x25_63EB;
pub const INPUT_TEXT: u32 = 0x0F_172A;
pub const INPUT_DISABLED_BG: u32 = 0xF8_FAFC;
pub const INPUT_DISABLED_TEXT: u32 = 0x94_A3B8;
pub const INPUT_DISABLED_BORDER: u32 = 0xEE_F2F7;

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

pub const DANGER_FG: u32 = 0xDC_2626;
pub const DANGER_HOVER_BG: u32 = 0xFE_F2F2;
pub const DANGER_HOVER_BORDER: u32 = 0xFC_A5A5;
pub const DANGER_PRESSED_BG: u32 = 0xFE_E2E2;

pub const DOT_RUNNING: u32 = 0x22_C55E;
pub const DOT_STOPPED: u32 = 0x94_A3B8;

pub const BADGE_RUN_BG: u32 = 0xDC_FCE7;
pub const BADGE_RUN_FG: u32 = 0x15_803D;
pub const BADGE_STOP_BG: u32 = 0xF1_F5F9;
pub const BADGE_STOP_FG: u32 = 0x64_748B;
pub const BADGE_LATEST_BG: u32 = 0xDB_EAFE;
pub const BADGE_LATEST_FG: u32 = 0x1D_4ED8;
pub const BADGE_FORCE_BG: u32 = 0xFE_F3C7;
pub const BADGE_FORCE_FG: u32 = 0xB4_5309;

/// 列表
pub const LIST_BG: u32 = 0xFF_FFFF;
pub const LIST_BORDER: u32 = 0xDB_E2EA;
pub const LIST_HEADER_BG: u32 = 0xF8_FAFC;
pub const LIST_HEADER_FG: u32 = 0x64_748B;
pub const LIST_ROW_ALT_BG: u32 = 0xFC_FDFE;
pub const LIST_ROW_SEL_BG: u32 = 0xEF_F6FF;
pub const LIST_ROW_SEL_BORDER: u32 = 0x93_C5FD;
pub const LIST_TEXT: u32 = 0x33_4155;
pub const LIST_MUTED: u32 = 0x94_A3B8;
pub const LIST_EMPTY_FG: u32 = 0xCB_D5E1;

/// 日志区
pub const LOG_BG: u32 = 0xF8_FAFC;
pub const LOG_BORDER: u32 = 0xEE_F2F7;
pub const LOG_TIME: u32 = 0x94_A3B8;
pub const LOG_TEXT: u32 = 0x47_5569;
pub const LOG_OK: u32 = 0x15_803D;
pub const LOG_WARN: u32 = 0xB4_5309;
pub const LOG_ERR: u32 = 0xDC_2626;

// ---------------------------------------------------------------------------
// 文案
// ---------------------------------------------------------------------------

pub const APP_TITLE: &str = "更新服务端";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const VERSION_BADGE_TEXT: &str = concat!("v", env!("CARGO_PKG_VERSION"));

// ---------------------------------------------------------------------------
// 尺寸（逻辑像素 @96dpi）
// ---------------------------------------------------------------------------

pub const CARD_W: f32 = 520.0;
pub const CARD_H: f32 = 460.0;
pub const CARD_RADIUS: f32 = 14.0;

const TITLE_BAR_H: f32 = 42.0;
const PAD_X: f32 = 16.0;
const PAD_TOP: f32 = 2.0;
const PAD_BOTTOM: f32 = 16.0;
const GAP: f32 = 10.0;

const STATUS_CARD_H: f32 = 40.0;
const ROW_H: f32 = 30.0;
const BTN_ROW_H: f32 = 32.0;
const SECTION_TITLE_H: f32 = 16.0;
const SECTION_GAP: f32 = 12.0;

const WINBTN_W: f32 = 30.0;
const WINBTN_H: f32 = 28.0;
const FIELD_PAD_X: f32 = 10.0;

/// 版本列表能显示的行数
pub const LIST_ROWS: usize = 4;
const LIST_ROW_H: f32 = 26.0;
const LIST_HEADER_H: f32 = 22.0;

/// 日志区能显示的行数
pub const LOG_ROWS: usize = 4;
const LOG_ROW_H: f32 = 20.0;

const SMALL_BTN_W: f32 = 56.0;

/// 版本徽章
const VER_BADGE_H: f32 = 22.0;
const VER_BADGE_PAD_X: f32 = 8.0;
const VER_BADGE_GAP: f32 = 6.0;

/// 96dpi 下 12px 微软雅黑 UI 的实测行高，仅用于建窗口前估尺寸
const BODY_LINE_H_96: f32 = 17.0;

#[derive(Copy, Clone, Debug)]
pub struct Layout {
    pub scale: f32,
    pub window: Rect,
    pub title_bar: Rect,
    pub icon: Rect,
    pub title_text: Rect,
    pub version_badge: Rect,
    pub btn_min: Rect,
    pub btn_close: Rect,

    pub status_card: Rect,
    pub status_dot: Rect,
    pub status_title: Rect,
    pub status_badge: Rect,

    pub addr_label: Rect,
    pub addr_field: Rect,
    pub addr_edit: Rect,
    pub addr_text: Rect,
    pub btn_start: Rect,
    pub btn_stop: Rect,

    pub list_title: Rect,
    pub list_header: Rect,
    pub list: Rect,

    pub log_title: Rect,
    pub log: Rect,

    pub btn_publish: Rect,
    pub btn_latest: Rect,
    pub btn_delete: Rect,
    pub btn_opendir: Rect,
}

impl Layout {
    /// 只按缩放因子算布局（字体度量用 96dpi 的实测值兜底），用于建窗口前估尺寸。
    pub fn compute(scale: f32) -> Self {
        Self::compute_with(scale, (BODY_LINE_H_96 * scale).round() as i32)
    }

    /// `body_line_h` = 正文字体的 `tmHeight`（物理像素）。
    /// 原生单行 EDIT 是顶对齐的，要让它看起来居中，上沿必须摆在「字段中心 − 行高/2」。
    pub fn compute_with(scale: f32, body_line_h: i32) -> Self {
        let s = |v: f32| (v * scale).round() as i32;
        let r = |x: f32, y: f32, w: f32, h: f32| Rect::new(s(x), s(y), s(w), s(h));
        let body_h = body_line_h.max(1);

        let win_w = CARD_W;
        let win_h = CARD_H;
        let inner_x = PAD_X;
        let inner_w = win_w - PAD_X * 2.0;

        // ---- 标题栏 ----
        let title_bar = r(0.0, 0.0, win_w, TITLE_BAR_H);
        let icon = r(PAD_X, (TITLE_BAR_H - 18.0) / 2.0, 18.0, 18.0);
        let close_x = win_w - 10.0 - WINBTN_W;
        let min_x = close_x - 8.0 - WINBTN_W;
        let btn_y = (TITLE_BAR_H - WINBTN_H) / 2.0;
        let btn_close = r(close_x, btn_y, WINBTN_W, WINBTN_H);
        let btn_min = r(min_x, btn_y, WINBTN_W, WINBTN_H);
        let title_text = Rect::new(
            icon.right() + s(4.0),
            title_bar.y,
            (s(min_x) - icon.right() - s(4.0)).max(0),
            title_bar.h,
        );
        let version_badge = Rect::new(0, 0, 0, 0); // 由 place_version_badge 摆放

        let mut y = TITLE_BAR_H + PAD_TOP;

        // ---- 服务状态卡片 ----
        let status_card = r(inner_x, y, inner_w, STATUS_CARD_H);
        let status_dot = r(inner_x + 12.0, y + (STATUS_CARD_H - 10.0) / 2.0, 10.0, 10.0);
        let status_title = r(inner_x + 12.0 + 10.0 + 8.0, y, 120.0, STATUS_CARD_H);
        let status_badge_w = 64.0;
        let status_badge = r(
            inner_x + inner_w - 12.0 - status_badge_w,
            y + (STATUS_CARD_H - 22.0) / 2.0,
            status_badge_w,
            22.0,
        );
        y += STATUS_CARD_H + GAP;

        // ---- 监听地址 + 启停 ----
        // 横向排布（逻辑像素）：标签 56 | 8 | 输入框 296 | 8 | 启动 56 | 8 | 停止 56
        const LABEL_W: f32 = 56.0;
        let btn_stop = r(inner_x + inner_w - SMALL_BTN_W, y, SMALL_BTN_W, ROW_H);
        let btn_start = r(
            inner_x + inner_w - SMALL_BTN_W * 2.0 - 8.0,
            y,
            SMALL_BTN_W,
            ROW_H,
        );
        let field_x = inner_x + LABEL_W + 8.0;
        // 扣掉：标签 56 + 标签到框 8 + 两个按钮 112 + 按钮之间 8 + 框到启动按钮 8
        let field_w = inner_w - LABEL_W - 8.0 - SMALL_BTN_W * 2.0 - 8.0 * 2.0;
        let addr_label = r(inner_x, y, LABEL_W, ROW_H);
        let addr_field = r(field_x, y, field_w, ROW_H);
        let addr_edit = Rect::new(
            addr_field.x + s(FIELD_PAD_X),
            addr_field.y + (addr_field.h - body_h) / 2,
            addr_field.w - s(FIELD_PAD_X) * 2,
            body_h + s(4.0),
        );
        let addr_text = Rect::new(
            addr_field.x + s(FIELD_PAD_X),
            addr_field.y,
            addr_field.w - s(FIELD_PAD_X) * 2,
            addr_field.h,
        );
        y += ROW_H + SECTION_GAP;

        // ---- 已发布版本 ----
        let list_title = r(inner_x, y, inner_w, SECTION_TITLE_H);
        y += SECTION_TITLE_H + 6.0;
        let list_header = r(inner_x, y, inner_w, LIST_HEADER_H);
        y += LIST_HEADER_H;
        let list = r(inner_x, y, inner_w, LIST_ROW_H * LIST_ROWS as f32);
        y += LIST_ROW_H * LIST_ROWS as f32 + SECTION_GAP;

        // ---- 最近请求 ----
        let log_title = r(inner_x, y, inner_w, SECTION_TITLE_H);
        y += SECTION_TITLE_H + 6.0;
        let log = r(inner_x, y, inner_w, LOG_ROW_H * LOG_ROWS as f32);
        y += LOG_ROW_H * LOG_ROWS as f32 + SECTION_GAP;

        // ---- 底部按钮（逻辑宽度依次 104 / 84 / 68 / 104，两两间隔 GAP）----
        const W1: f32 = 104.0;
        const W2: f32 = 84.0;
        const W3: f32 = 68.0;
        const W4: f32 = 104.0;
        let btn_publish = r(inner_x, y, W1, BTN_ROW_H);
        let btn_latest = r(inner_x + W1 + GAP, y, W2, BTN_ROW_H);
        let btn_delete = r(inner_x + W1 + GAP + W2 + GAP, y, W3, BTN_ROW_H);
        let btn_opendir = r(inner_x + W1 + GAP + W2 + GAP + W3 + GAP, y, W4, BTN_ROW_H);
        y += BTN_ROW_H + PAD_BOTTOM;

        let window = Rect::new(0, 0, s(win_w), s(win_h));
        debug_assert!(y <= win_h + 0.5, "布局高度超出卡片：{y} > {win_h}");

        Layout {
            scale,
            window,
            title_bar,
            icon,
            title_text,
            version_badge,
            btn_min,
            btn_close,
            status_card,
            status_dot,
            status_title,
            status_badge,
            addr_label,
            addr_field,
            addr_edit,
            addr_text,
            btn_start,
            btn_stop,
            list_title,
            list_header,
            list,
            log_title,
            log,
            btn_publish,
            btn_latest,
            btn_delete,
            btn_opendir,
        }
    }

    /// 摆放标题右侧的版本徽章。两个宽度都必须是实测值（`gfx::text_width`）。
    pub fn place_version_badge(&mut self, title_w: i32, version_w: i32) {
        let s = |v: f32| (v * self.scale).round() as i32;
        let pad = s(VER_BADGE_PAD_X);
        let h = s(VER_BADGE_H);
        let x = self.title_text.x + title_w.max(0) + s(VER_BADGE_GAP);
        let limit = self.btn_min.x - s(VER_BADGE_GAP);
        let w = (version_w.max(1) + pad * 2).min((limit - x).max(0));
        let y = self.title_bar.y + (self.title_bar.h - h) / 2;
        self.version_badge = Rect::new(x, y, w, h);
    }

    /// 版本列表某一行的矩形（`index` 从 0 起）
    pub fn list_row(&self, index: usize) -> Rect {
        let rh = (LIST_ROW_H * self.scale).round() as i32;
        Rect::new(self.list.x, self.list.y + rh * index as i32, self.list.w, rh)
    }

    pub fn list_row_h(&self) -> i32 {
        (LIST_ROW_H * self.scale).round() as i32
    }

    /// 日志区某一行的矩形
    pub fn log_row(&self, index: usize) -> Rect {
        let rh = (LOG_ROW_H * self.scale).round() as i32;
        Rect::new(self.log.x, self.log.y + rh * index as i32, self.log.w, rh)
    }

    /// 列表可见高度能容纳的行数（用于滚动范围）
    pub fn visible_rows(&self) -> usize {
        LIST_ROWS
    }
}
