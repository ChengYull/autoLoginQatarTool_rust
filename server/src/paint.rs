//! 界面绘制。

use crate::gfx::{ALIGN_CENTER, ALIGN_LEFT, ALIGN_RIGHT, Canvas, Fonts};
use crate::http::LogEntry;
use crate::icon::draw_server;
use crate::model::{BtnKind, Widget};
use crate::store::{Release, human_size};
use crate::theme::*;
use crate::util::{Rect, rgb, rgba};

/// 一次绘制所需的全部视觉状态
pub struct VisualState<'a> {
    pub running: bool,
    pub bind: &'a str,
    /// 已发布版本，已按版本号从新到旧排好。
    ///
    /// 这里持有一个 `Vec` 而不是切片：排序结果和日志快照都是每次绘制现算的，
    /// 借用局部变量的话函数一返回就悬空了。借用的是 `App::repo` 里的数据本身。
    pub releases: Vec<&'a Release>,
    /// 当前对外提供的最新版本号（空串 = 未设置）
    pub latest: &'a str,
    /// 当前选中行在 `releases` 里的下标
    pub selected: Option<usize>,
    /// 列表滚动偏移（第一行对应的下标）
    pub scroll: usize,
    pub logs: Vec<LogEntry>,
    pub total_hits: u64,
    pub total_downloads: u64,

    pub hover: Option<Widget>,
    pub pressed: Option<Widget>,

    pub addr_enabled: bool,
    pub addr_focus: bool,
    pub addr_hover: bool,
    pub start_enabled: bool,
    pub stop_enabled: bool,
    pub publish_enabled: bool,
    pub set_latest_enabled: bool,
    pub delete_enabled: bool,
}

impl<'a> VisualState<'a> {
    fn hovered(&self, w: Widget) -> bool {
        self.hover == Some(w)
    }
    fn pressed(&self, w: Widget) -> bool {
        self.pressed == Some(w)
    }
}

/// 绘制整窗。`widgets` 为 true 时连同子控件（按钮 / 输入文本）一起画，用于离屏预览。
pub fn paint_all(canvas: &Canvas, l: &Layout, f: &Fonts, vs: &VisualState, widgets: bool) {
    if widgets {
        canvas.round_rect_filled_bordered(
            l.window,
            CARD_RADIUS * l.scale,
            rgb(CARD_BG),
            rgb(CARD_BORDER),
            1.0,
        );
    } else {
        // 运行时卡片外观由背景层窗口负责，这里只铺底色
        canvas.fill_rect(l.window, rgb(CARD_BG));
    }

    // ---- 标题栏 ----
    draw_server(canvas, l.icon.x, l.icon.y, l.icon.w);
    canvas.text(APP_TITLE, l.title_text, f.title, rgb(TITLE_TEXT), ALIGN_LEFT);
    paint_version_badge(canvas, l, f);

    paint_status(canvas, l, f, vs);
    paint_addr(canvas, l, f, vs);
    paint_list(canvas, l, f, vs);
    paint_log(canvas, l, f, vs);

    if !widgets {
        return;
    }

    // ---- 以下仅离屏预览需要（真实运行时由子控件自绘）----
    paint_button(canvas, f, l.btn_min, BtnKind::WinMin, "—", l.scale, vs.hovered(Widget::Min), vs.pressed(Widget::Min), false);
    paint_button(canvas, f, l.btn_close, BtnKind::WinClose, "✕", l.scale, vs.hovered(Widget::Close), vs.pressed(Widget::Close), false);

    paint_button(canvas, f, l.btn_start, BtnKind::Primary, "启动", l.scale, vs.hovered(Widget::Start), vs.pressed(Widget::Start), !vs.start_enabled);
    paint_button(canvas, f, l.btn_stop, BtnKind::Secondary, "停止", l.scale, vs.hovered(Widget::Stop), vs.pressed(Widget::Stop), !vs.stop_enabled);

    paint_button(canvas, f, l.btn_publish, BtnKind::Primary, "发布新版本", l.scale, vs.hovered(Widget::Publish), vs.pressed(Widget::Publish), !vs.publish_enabled);
    paint_button(canvas, f, l.btn_latest, BtnKind::Secondary, "设为最新", l.scale, vs.hovered(Widget::SetLatest), vs.pressed(Widget::SetLatest), !vs.set_latest_enabled);
    paint_button(canvas, f, l.btn_delete, BtnKind::Danger, "删除", l.scale, vs.hovered(Widget::Delete), vs.pressed(Widget::Delete), !vs.delete_enabled);
    paint_button(canvas, f, l.btn_opendir, BtnKind::Secondary, "打开数据目录", l.scale, vs.hovered(Widget::OpenDir), vs.pressed(Widget::OpenDir), false);

    // 监听地址输入框里的文字（真实运行时由 EDIT 自己画）
    let addr_color = if vs.addr_enabled { INPUT_TEXT } else { INPUT_DISABLED_TEXT };
    canvas.text(vs.bind, l.addr_text, f.body, rgb(addr_color), ALIGN_LEFT);
}

// ---------------------------------------------------------------------------
// 版本徽章
// ---------------------------------------------------------------------------

fn paint_version_badge(canvas: &Canvas, l: &Layout, f: &Fonts) {
    let r = l.version_badge;
    if r.w <= 0 || r.h <= 0 {
        return;
    }
    canvas.fill_round_rect(r, r.h as f32 / 2.0, rgb(BADGE_LATEST_BG));
    canvas.text(VERSION_BADGE_TEXT, r, f.badge, rgb(BADGE_LATEST_FG), ALIGN_CENTER);
}

// ---------------------------------------------------------------------------
// 服务状态卡片
// ---------------------------------------------------------------------------

fn paint_status(canvas: &Canvas, l: &Layout, f: &Fonts, vs: &VisualState) {
    canvas.round_rect_filled_bordered(
        l.status_card,
        10.0 * l.scale,
        rgb(STATUS_CARD_BG),
        rgb(STATUS_CARD_BORDER),
        1.0,
    );

    let (dot, badge_bg, badge_fg, text) = if vs.running {
        (DOT_RUNNING, BADGE_RUN_BG, BADGE_RUN_FG, "运行中")
    } else {
        (DOT_STOPPED, BADGE_STOP_BG, BADGE_STOP_FG, "已停止")
    };

    canvas.fill_ellipse(l.status_dot, rgb(dot));
    canvas.text("服务状态", l.status_title, f.label, rgb(STATUS_TITLE), ALIGN_LEFT);

    canvas.fill_round_rect(l.status_badge, l.status_badge.h as f32 / 2.0, rgb(badge_bg));
    canvas.text(text, l.status_badge, f.badge, rgb(badge_fg), ALIGN_CENTER);

    // 右侧统计：请求数 / 下载数（放在徽章左边，靠右对齐）
    let stats = format!("请求 {} · 下载 {}", vs.total_hits, vs.total_downloads);
    let stats_rect = Rect::new(
        l.status_title.right(),
        l.status_card.y,
        (l.status_badge.x - 8 - l.status_title.right()).max(0),
        l.status_card.h,
    );
    canvas.text(&stats, stats_rect, f.small, rgb(HINT), ALIGN_RIGHT);
}

// ---------------------------------------------------------------------------
// 监听地址
// ---------------------------------------------------------------------------

fn paint_addr(canvas: &Canvas, l: &Layout, f: &Fonts, vs: &VisualState) {
    canvas.text("监听地址", l.addr_label, f.label, rgb(LABEL), ALIGN_LEFT);
    paint_field(canvas, l.addr_field, l.scale, vs.addr_focus, vs.addr_hover, !vs.addr_enabled);
}

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

// ---------------------------------------------------------------------------
// 已发布版本列表
// ---------------------------------------------------------------------------

/// 列宽（逻辑像素），总宽 = 列表内宽
const COL_VERSION: f32 = 100.0;
const COL_TIME: f32 = 128.0;
const COL_SIZE: f32 = 70.0;
const COL_DOWNLOADS: f32 = 56.0;
const CELL_PAD: f32 = 12.0;

fn paint_list(canvas: &Canvas, l: &Layout, f: &Fonts, vs: &VisualState) {
    let count = vs.releases.len();
    let title = if count == 0 {
        "已发布版本".to_string()
    } else {
        format!("已发布版本（{count}）")
    };
    canvas.text(&title, l.list_title, f.label, rgb(LABEL), ALIGN_LEFT);

    // 表头
    canvas.fill_round_rect(
        Rect::new(l.list_header.x, l.list_header.y, l.list_header.w, l.list_header.h + 4),
        6.0 * l.scale,
        rgb(LIST_HEADER_BG),
    );
    let cols = column_rects(l, l.list_header);
    for (i, name) in ["版本", "发布时间", "大小", "下载", "状态"].iter().enumerate() {
        canvas.text(name, cols[i], f.small, rgb(LIST_HEADER_FG), ALIGN_LEFT);
    }

    // 列表体
    canvas.round_rect_filled_bordered(
        l.list,
        8.0 * l.scale,
        rgb(LIST_BG),
        rgb(LIST_BORDER),
        1.0,
    );

    if count == 0 {
        canvas.text("还没有发布任何版本，点下面「发布新版本」开始", l.list, f.body, rgb(LIST_EMPTY_FG), ALIGN_CENTER);
        return;
    }

    let visible = l.visible_rows();
    let start = vs.scroll.min(count.saturating_sub(1));
    let end = (start + visible).min(count);

    for (slot, idx) in (start..end).enumerate() {
        let row = l.list_row(slot);
        let release = vs.releases[idx];
        let selected = vs.selected == Some(idx);

        if selected {
            canvas.round_rect_filled_bordered(
                row,
                6.0 * l.scale,
                rgb(LIST_ROW_SEL_BG),
                rgb(LIST_ROW_SEL_BORDER),
                1.0,
            );
        } else if slot % 2 == 1 {
            canvas.fill_rect(row, rgb(LIST_ROW_ALT_BG));
        }

        let cells = column_rects(l, row);
        canvas.text(
            &format!("v{}", release.version),
            cells[0],
            f.body,
            rgb(LIST_TEXT),
            ALIGN_LEFT,
        );
        canvas.text(&release.published_at, cells[1], f.small, rgb(LIST_MUTED), ALIGN_LEFT);
        canvas.text(&human_size(release.size), cells[2], f.small, rgb(LIST_MUTED), ALIGN_LEFT);
        canvas.text(&release.downloads.to_string(), cells[3], f.small, rgb(LIST_MUTED), ALIGN_LEFT);

        // 状态列：最新 / 强制，最多两个小胶囊
        let mut bx = cells[4].x;
        let badge_h = (18.0 * l.scale).round() as i32;
        let by = row.y + (row.h - badge_h) / 2;
        if release.version == vs.latest {
            bx = paint_pill(canvas, f, bx, by, badge_h, "最新", BADGE_LATEST_BG, BADGE_LATEST_FG, l.scale);
        }
        if release.force {
            paint_pill(canvas, f, bx, by, badge_h, "强制", BADGE_FORCE_BG, BADGE_FORCE_FG, l.scale);
        }
    }
}

/// 画一个小胶囊，返回右边界（供后面接着摆下一个）
fn paint_pill(
    canvas: &Canvas,
    f: &Fonts,
    x: i32,
    y: i32,
    h: i32,
    text: &str,
    bg: u32,
    fg: u32,
    scale: f32,
) -> i32 {
    let pad = (6.0 * scale).round() as i32;
    let w = crate::gfx::text_width(text, f.badge) + pad * 2;
    let r = Rect::new(x, y, w, h);
    canvas.fill_round_rect(r, h as f32 / 2.0, rgb(bg));
    canvas.text(text, r, f.badge, rgb(fg), ALIGN_CENTER);
    r.right() + (4.0 * scale).round() as i32
}

/// 把一行的可用宽度切成 5 列
fn column_rects(l: &Layout, row: Rect) -> [Rect; 5] {
    let s = |v: f32| (v * l.scale).round() as i32;
    let pad = s(CELL_PAD);
    let x0 = row.x + pad;
    let avail = row.w - pad * 2;
    let c1 = s(COL_VERSION).min(avail);
    let c2 = s(COL_TIME).min(avail - c1);
    let c3 = s(COL_SIZE).min(avail - c1 - c2);
    let c4 = s(COL_DOWNLOADS).min(avail - c1 - c2 - c3);
    let c5 = avail - c1 - c2 - c3 - c4;

    let mk = |x: i32, w: i32| Rect::new(x, row.y, w, row.h);
    [
        mk(x0, c1),
        mk(x0 + c1, c2),
        mk(x0 + c1 + c2, c3),
        mk(x0 + c1 + c2 + c3, c4),
        mk(x0 + c1 + c2 + c3 + c4, c5),
    ]
}

// ---------------------------------------------------------------------------
// 最近请求
// ---------------------------------------------------------------------------

const LOG_TIME_W: f32 = 68.0;
const LOG_PEER_W: f32 = 110.0;
const LOG_STATUS_W: f32 = 44.0;

fn paint_log(canvas: &Canvas, l: &Layout, f: &Fonts, vs: &VisualState) {
    canvas.text("最近请求", l.log_title, f.label, rgb(LABEL), ALIGN_LEFT);
    canvas.round_rect_filled_bordered(
        l.log,
        8.0 * l.scale,
        rgb(LOG_BG),
        rgb(LOG_BORDER),
        1.0,
    );

    if vs.logs.is_empty() {
        canvas.text("暂无请求", l.log, f.small, rgb(LIST_EMPTY_FG), ALIGN_CENTER);
        return;
    }

    let s = |v: f32| (v * l.scale).round() as i32;
    let pad = s(CELL_PAD);
    for (i, entry) in vs.logs.iter().take(LOG_ROWS).enumerate() {
        let row = l.log_row(i);
        let x0 = row.x + pad;
        let avail = row.w - pad * 2;
        let w1 = s(LOG_TIME_W).min(avail);
        let w2 = s(LOG_PEER_W).min(avail - w1);
        let w3 = s(LOG_STATUS_W).min(avail - w1 - w2);
        let w4 = avail - w1 - w2 - w3;

        canvas.text(&entry.time, Rect::new(x0, row.y, w1, row.h), f.small, rgb(LOG_TIME), ALIGN_LEFT);
        canvas.text(&entry.peer, Rect::new(x0 + w1, row.y, w2, row.h), f.small, rgb(LOG_TIME), ALIGN_LEFT);
        canvas.text(&entry.line, Rect::new(x0 + w1 + w2, row.y, w4, row.h), f.small, rgb(LOG_TEXT), ALIGN_LEFT);

        let color = match entry.status {
            200 => LOG_OK,
            400..=499 => LOG_WARN,
            _ => LOG_ERR,
        };
        canvas.text(
            &entry.status.to_string(),
            Rect::new(x0 + w1 + w2 + w4, row.y, w3, row.h),
            f.small,
            rgb(color),
            ALIGN_LEFT,
        );
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
            let fg = if hovered || pressed {
                canvas.fill_round_rect(r, 7.0 * scale, rgb(hover_bg));
                hover_fg
            } else {
                WINBTN_FG
            };

            let stroke = (1.4 * scale).max(1.0);
            if is_close {
                let d = (4.5 * scale).round() as i32;
                canvas.line((r.cx() - d) as f32, (r.cy() - d) as f32, (r.cx() + d) as f32, (r.cy() + d) as f32, rgba_from(fg), stroke);
                canvas.line((r.cx() - d) as f32, (r.cy() + d) as f32, (r.cx() + d) as f32, (r.cy() - d) as f32, rgba_from(fg), stroke);
            } else {
                let w = (5.0 * scale).round() as i32;
                canvas.line((r.cx() - w) as f32, r.cy() as f32, (r.cx() + w) as f32, r.cy() as f32, rgba_from(fg), stroke);
            }
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
        BtnKind::Danger => {
            let (bg, border, fg) = if disabled {
                (INPUT_DISABLED_BG, INPUT_DISABLED_BORDER, SECONDARY_DISABLED_FG)
            } else if pressed {
                (DANGER_PRESSED_BG, DANGER_HOVER_BORDER, DANGER_FG)
            } else if hovered {
                (DANGER_HOVER_BG, DANGER_HOVER_BORDER, DANGER_FG)
            } else {
                (INPUT_BG, SECONDARY_BORDER, DANGER_FG)
            };
            canvas.round_rect_filled_bordered(r, 8.0 * scale, rgb(bg), rgb(border), 1.0);
            canvas.text(text, r, f.label, rgb(fg), ALIGN_CENTER);
        }
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
