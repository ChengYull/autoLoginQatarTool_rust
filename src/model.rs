//! 状态模型。

/// 自动登录状态机（三态，避免"先停后启"写法在延时窗口内出现状态不一致）
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum LoginState {
    /// 未启用
    Stopped,
    /// 监测中
    Running,
    /// 已检测到客户端，正在执行登录动作
    Handling,
}

/// 本次登录动作对应的客户端
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum LoginTarget {
    /// QaTar.exe
    Qatar,
    /// QtPacker.exe
    QtPacker,
}

/// 可交互的自绘控件标识（用于 hover / pressed / focus 判定）
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Widget {
    Min,
    Close,
    Reset,
    Start,
    Stop,
    Toggle,
    Eye,
}

/// 自绘按钮的外观种类
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum BtnKind {
    WinMin,
    WinClose,
    Reset,
    Primary,
    Secondary,
}

