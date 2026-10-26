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
    /// 标题栏「三个点」：点开密码相关菜单（1.0.5）
    More,
    /// 密码行右侧的复制按钮（1.0.5，取代原来的「重置」）
    Copy,
    Start,
    Stop,
    /// 开机自启动开关
    Toggle,
    /// 密码自动重置开关（1.0.5）
    ResetToggle,
    Eye,
}

/// 自绘按钮的外观种类
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum BtnKind {
    WinMin,
    WinClose,
    /// 只画 hover 底色，不画任何图形 —— 给「三个点」这类自己绘制图标的标题栏按钮用。
    /// 复用 WinMin 会把最小化那条横线也画出来，跟图标叠在一起。
    WinHover,
    /// 无边框的幽灵按钮：只有 hover 底色，用于图标按钮
    Ghost,
    Primary,
    Secondary,
}

