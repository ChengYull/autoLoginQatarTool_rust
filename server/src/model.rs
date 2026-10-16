//! 交互状态枚举。

/// 可交互的自绘控件标识（用于 hover / pressed 判定）
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Widget {
    Min,
    Close,
    Start,
    Stop,
    Publish,
    SetLatest,
    Delete,
    OpenDir,
}

/// 自绘按钮的外观种类
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum BtnKind {
    WinMin,
    WinClose,
    Primary,
    Secondary,
    /// 破坏性操作（删除），红字 + 悬停淡红底
    Danger,
}
