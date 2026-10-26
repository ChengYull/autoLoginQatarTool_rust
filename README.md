# AutoLoginQatarTool（Rust 版）

Qatar / QtPacker 客户端自动登录工具 —— 用 Rust 对同级目录 `autoLoginQatarTool`（C++ / Qt 版）的完整复刻。
仓库里还带一个**更新服务端**（`server/`），负责发布新版本并让客户端知道该不该升级。

**产物是单个 exe，拷到任何 Windows 10/11 机器上直接双击即可运行**，不需要安装运行库、不需要带 DLL。
客户端和服务端都是如此，各自只有一个文件。

当前版本：`v1.0.5`

版本号**只在 `Cargo.toml` 的 `version` 字段维护一份**，另外两处都由它派生：

| 出现位置 | 取值方式 |
| --- | --- |
| 标题栏右侧的版本徽章 | `version.rs` 里 `env!("CARGO_PKG_VERSION")`，编译期常量 |
| exe 属性里的文件 / 产品版本 | `build.rs` 把版本填进 `app.rc` 的 `@VERSION_STR@` / `@VERSION_COMMA@` 占位符 |

所以升版本只改 `Cargo.toml` 一行，界面、exe 属性不会再各说各话。

---

## 与原版的关系

行为、界面、配置全部对齐原版，配置项位置也完全一致，因此**从 C++ 版换过来无需重新填写账号密码**。

| 功能 | 实现位置 | 说明 |
| --- | --- | --- |
| 进程监测 | `process.rs` | 每 500ms 一次 Toolhelp32 快照，匹配 `QaTar.exe` / `QtPacker.exe`，并对映像名做 `qtpacker` 不区分大小写兜底 |
| 多客户端分发 | `app.rs` | 按当前出现的对话框判定目标客户端，分发到 Qatar / QtPacker 两套流程 |
| 环境自适应 | `automation.rs` | 简体 / 繁体 / 英文三种对话框标题与登录按钮文案 |
| 控件定位 | `automation.rs` | 按几何位置而非控件顺序定位（密码框取最下方 Edit、账户框取密码框上方纵坐标最大的 ComboBox） |
| 密码错误识别 | `automation.rs` | Qatar 匹配与语言无关的返回码 `ec: -3`；QtPacker 匹配标题 `Message` 且正文含 `Fail to login` 的提示框 |
| 设置密码 | `net.rs` | `POST /api/user/setpassword`，JSON `{"uid","pwd"}`，**响应体恰好是 `ok`** 才算成功；失败时原样展示服务端文案 |
| 密码自动重置 | `pwd.rs` + `app.rs` | 7 个密码按 7 天滚动切换，49 天一轮回；提前 6 小时触发，启动后每分钟检查一次 |
| 密码池存储 | `settings.rs` | 7 个密码用 DPAPI 加密后写 `REG_BINARY pwdPool`，只有当前 Windows 用户能解开 |
| 密码对话框 | `dlg.rs` | 三点菜单 →「自定义密码」单次设置 /「自动重置密码」7 个密码 + 随机生成；不合规的输入框描红 |
| 开机自启 | `settings.rs` | 写入 `HKCU\...\CurrentVersion\Run\autoLoginQatarApp` |
| 配置持久化 | `settings.rs` | `HKCU\Software\Qtech\autoLoginQatarApp` |
| 系统托盘 | `tray.rs` | 双击恢复主界面，右键菜单含状态展示 / 启停 / 显示 / 退出 |
| 三态状态机 | `app.rs` | `Stopped` / `Running` / `Handling`，`Handling` 期间可被"停止"取消 |
| 版本标识 | `version.rs` | 标题右侧的胶囊徽章显示当前版本，绿色 = 已是最新，黄色 = 服务端上有更新版本（配色与「运行中 / 登录中」一致） |
| 现代界面 | `paint.rs` + `theme.rs` | 无边框圆角卡片、状态徽标、自绘滑动开关、密码明文切换、矢量相机图标 |

### 有意的差异

- **没有外发光阴影**。原版用 `QGraphicsDropShadowEffect` 做柔化投影，而逐像素 alpha 的分层窗口无法承载原生输入框子控件，这里就没做投影。
- **圆角卡片由一个独立的背景层窗口绘制**。窗口区域（`SetWindowRgn`）是 1 位掩码，圆弧只能用整像素台阶逼近，深色壁纸下会露出一圈毛刺；所以把「外观」和「输入」拆成两层：背景层是逐像素 alpha 的分层窗口，抗锯齿地画圆角与描边；主窗口只承载子控件，自身裁成**内缩 2px 的圆角区域**，硬边落在背景层白色卡片内部，同色所以看不见。见 `backdrop.rs`。
- **提示框用系统 MessageBox，密码对话框是自绘圆角卡片**（`dlg.rs`，复用背景层那套双窗口结构）。
- **托盘菜单为自绘菜单项**（白底 / 圆角高亮），但没有做菜单外框的圆角。
- **标题右侧多了一个版本徽章**。原版标题栏只有标题文字；这里加了版本胶囊，
  为后续的自动更新留位置（服务端接上后，徽章变黄即代表有新版本）。托盘提示、
  任务栏标题仍与原版一致，不带版本号。

---

## 更新服务端

`server/` 是一个独立的小程序：内置 HTTP 服务 + 一个原生管理窗口，管"发布新版本"这件事。

### 它解决什么问题

客户端的版本号是编译期写死在 exe 里的，靠人工通知"出新版了、去换一下"迟早会漏。
有了服务端之后：发布一次 → 客户端启动时自己问一次 → 版本旧了徽章变黄。
**客户端不做自动下载替换**（那是下一步），这里只负责"让用户知道该升级了"。

### 管理界面

双击 `AutoLoginServer.exe` 即用，打开就自动开始监听（不用再点一次「启动」）：

| 区域 | 说明 |
| --- | --- |
| 服务状态 | 运行中 / 已停止，右侧显示累计请求数与下载数 |
| 监听地址 | 默认 `0.0.0.0:18082`，改完点「启动」生效；运行中不可改 |
| 已发布版本 | 版本号 / 发布时间 / 大小 / 下载次数 / 是否「最新」「强制」 |
| 最近请求 | 最近 200 条访问日志，按状态码着色 |
| 底部操作 | 发布新版本 / 设为最新 / 删除 / 打开数据目录 |

**版本号不用手输**：选好 exe 之后，服务端直接从它的 `VERSIONINFO` 资源里读。
人工再输一遍版本号是这类工具最常见的低级错误来源 —— 打错了表现为
"服务端说最新是 1.0.3，客户端实际是 1.0.2"，很难查。

**「最新」是显式设置的，不是取最大版本号**。这样某个版本出问题时可以把对外版本
回滚到旧版，而不用把它从列表里删掉（删掉就没法排查了）。新发布的版本如果比当前
对外版本新，会自动顶上；更旧则只入库、不动对外版本。

### 接口

| 路径 | 说明 |
| --- | --- |
| `GET /api/version?current=1.0.2` | 返回最新版本、更新说明、是否强制、下载地址、大小 |
| `GET /api/download/<版本>` | 下载该版本的 exe（流式，不占内存） |
| `GET /api/health` | 健康检查，返回 `{"status":"ok"}` |

`/api/version` 的响应示例：

```json
{
  "update": true,
  "latest": "1.0.2",
  "notes": "修复登录延时偶发失效",
  "force": false,
  "url": "/api/download/1.0.2",
  "size": 458752,
  "published_at": "2026-10-16 10:30:00"
}
```

还没发布过任何版本时返回 `latest` 为空串的哨兵响应，客户端据此判定"没有更新"。

### 自动更新（客户端侧）

**三个入口都能触发更新检查**：

| 入口 | 行为 |
| --- | --- |
| 版本徽章（黄） | 左键点击 → 直接弹更新确认框 |
| 版本徽章 | 右键 → 「检查更新」（绿色也可以点，用于主动查询） |
| 托盘图标 | 右键菜单 → 「检查更新」 |

手动检查时会给明确反馈：有新版本就直接进确认框，已是最新会提示"当前已是最新版本"，
连不上服务端会说明是网络/地址问题 —— 不会点了菜单什么都没发生。

版本徽章变黄时，**点它就能升级**：

1. 弹出确认框，显示服务端写的更新说明（可以取消，取消后徽章仍是黄的，随时能再点）
2. 确认后后台下载新版本，状态徽章变成「更新中」
3. 下载完先校验：**HTTP 状态码必须 200**、字节数必须和服务端标注的一致、文件头必须是 `MZ`
4. 校验通过才替换，替换完自动重启。账户密码、开机自启这些设置都在注册表里，不受影响

**替换自己的原理**：Windows 不允许覆盖正在运行的 exe，但**允许给它改名**。
所以顺序是「自己改名成 `.old` → 新文件就位 → 启动新进程 → 老进程退出」，
新版本启动时再把 `.old` 删掉。

> ⚠️ 这里有个容易踩的坑：版本徽章在标题栏里面，而标题栏的 `WM_NCHITTEST` 返回 `HTCAPTION`
> 让系统负责拖动窗口 —— 一旦这么返回，鼠标按下就变成"拖窗口"，`WM_LBUTTONDOWN` 根本不会发过来，
> 表现就是**点徽章毫无反应**。可点的徽章区域必须从标题栏命中测试里排除掉、返回 `HTCLIENT`。

**失败不会把自己搞坏**：改名成功但写入失败时会把名字改回去；
下载不完整 / 不是可执行文件 / 服务端返回非 200，都在替换之前就丢弃并提示原因，
原程序继续正常运行。

### 数据与部署

数据目录默认是 **exe 同级的 `data/`**（可用 `--data <目录>` 覆盖），所以「拷到哪、数据就在哪」：

```text
data/
├── server.json                        监听地址等本机配置
├── releases.json                      版本元数据
└── files/
    └── AutoLoginQatarTool-1.0.2.exe   各版本产物，客户端从这里下载
```

`releases.json` 写入走的是「先写 `.tmp` 再改名」，中途断电不会把原文件截成半截 JSON。

客户端默认连 `10.170.108.115:18082`，可用环境变量覆盖（自测用）：

```
AUTOLOGIN_UPDATE_HOST=127.0.0.1
AUTOLOGIN_UPDATE_PORT=18082
```

> ⚠️ 服务端目前是**明文 HTTP 且无鉴权**：任何能连上端口的人都能发布版本、下载产物。
> 只适合放在可信内网。要往外网暴露的话，前面得挂一层带认证的反向代理。

---

## 编译

只依赖 Rust 工具链（`x86_64-pc-windows-gnu`），不需要 MSVC，也不需要 Qt。

```bash
cargo build --release                  # 客户端
cd server && cargo build --release     # 服务端
```

产物：

| 程序 | 路径 | 体积 |
| --- | --- | --- |
| 客户端 | `target/x86_64-pc-windows-gnu/release/AutoLoginQatarTool.exe` | ~450 KB |
| 服务端 | `server/target/x86_64-pc-windows-gnu/release/AutoLoginServer.exe` | ~576 KB |

两个都是单文件、只依赖系统 DLL。`server/` 是独立 crate，但会继承仓库根的
`.cargo/config.toml`（`+crt-static` 与 GNU target），所以静态链接配置只有一份。

> 服务端比客户端多一个纯 Rust 依赖 `serde` / `serde_json`：版本元数据要落盘持久化，
> 更新说明里会有中文、引号、换行，转义写错了很难发现 —— 这类正确性不值得自己造轮子。
> 它是静态链接的，不产生额外 DLL，单 exe 的约定不受影响。

### 单 exe 是怎么做到的

- `.cargo/config.toml` 里开了 `-C target-feature=+crt-static`：UCRT 与 MinGW 运行库（libgcc / libwinpthread / libstdc++）全部静态链接进 exe。
- `Cargo.toml` 的 release profile 开了 `lto = "fat"`、`codegen-units = 1`、`panic = "abort"`、`strip = "symbols"`。
- 唯一的外部依赖是 `windows` crate（纯 Rust 的 Win32 绑定，只产生对系统 DLL 的调用）。

验证方式：

```bash
objdump -p target/x86_64-pc-windows-gnu/release/AutoLoginQatarTool.exe | grep "DLL Name"
```

输出应当只有系统 DLL：`KERNEL32 / user32 / gdi32 / gdiplus / shell32 / advapi32 / winhttp / oleaut32 / ntdll` 以及 UCRT 的 `api-ms-win-crt-*`。

### exe 图标

`app.rc` 里的图标与版本信息由 `build.rs` 调用 MinGW 的 `windres` 编译后交给链接器。
`build.rs` 会先在 `PATH` 里找 `windres`，再回退到几个常见 MinGW 安装路径；找不到时只打警告、不中断构建（此时 exe 不带文件图标）。

`app.rc` 里的版本号写的是占位符，`build.rs` 会把 `CARGO_PKG_VERSION` 填进去
（`1.0.1` → `1,0,1,0` 给 `FILEVERSION`），所以**别在 `app.rc` 里硬编码版本号**。
替换后的脚本落在 `OUT_DIR`，不污染仓库源文件。

---

## 使用

与原版一致：

1. 运行 `AutoLoginQatarTool.exe`，填写"账户 / 工号"与"密码"。
2. 按需打开"开机自启动"。
3. 点 **启动**，状态徽标变绿"运行中"，此时账户与密码框会被禁用；要改先点 **停止**。
4. 保持程序在托盘后台运行。`QaTar.exe` 或 `QtPacker.exe` 启动并弹出登录框时，程序延时 2 秒后自动填入账号密码并点击登录。
5. 提示"密码与工号不匹配"说明账号或密码有误，改好后状态会自动恢复监测。
6. 关闭窗口只会最小化到托盘；右键托盘图标 → **退出** 才真正结束进程。
7. 密码默认掩码显示，点输入框右侧眼睛图标可切明文。
8. 忘记密码时点 **重置**，程序按当前账户调用重置接口，新密码由服务端发到钉钉。
9. 标题右侧的胶囊是当前版本号。绿色 = 已是最新；**黄色 = 服务端上有更新版本，点它就能升级**（见下面的「自动更新」）。

### 配置位置

```
HKCU\Software\Qtech\autoLoginQatarApp     account / password / autoStart
HKCU\Software\Microsoft\Windows\CurrentVersion\Run\autoLoginQatarApp
```

> ⚠️ 密码以明文保存在注册表中，仅适合可信的个人办公机环境（与原版一致）。

---

## 实现要点（踩过的坑）

写原生 Win32 GUI 时遇到几个不查到底就会被误导的坑，记在这里：

### 1. 消息常量没导入会被当成"变量绑定"

窗口过程里用常量做 `match` 分支时，如果这个常量**不在作用域内**，Rust 不会报错，而是把它当成一个新的变量绑定（catch-all），于是它之后的所有分支静默失效，只给一条 `unreachable_patterns` 警告。

本项目就踩了：`WM_MOUSELEAVE` 在 `windows` crate 里定义于 `Win32::UI::Controls` 而非 `WindowsAndMessaging`，结果它之后的分支（`WM_TIMER`、`WM_COMMAND`、托盘、关闭……）全部不可达并被优化掉 —— 界面看着正常，但按钮点击、定时器、托盘菜单统统无效。

现在 `main.rs` 顶部有 `#![deny(unreachable_patterns)]`，把这类问题直接升级为编译错误。

### 2. WM_NCCREATE 必须显式返回 TRUE

`DefWindowProc` 对 `WM_NCCREATE` 的返回值在本机实测为 `0`。若原样返回，窗口创建会被直接中断：`WM_CREATE` 根本不会发出，紧接着就是 `WM_NCDESTROY`，`CreateWindowEx` 返回 NULL 且 `GetLastError` 为 0（完全指不到真因）。

`wnd.rs` 里的 `widget_wndproc` 因此显式 `return LRESULT(1)`。

### 3. 自绘子控件要自己铺底色

自绘控件都是圆角形状，四个角在形状之外；而子窗口有独立的绘制表面，双缓冲位图又是未初始化的。不先铺一层卡片底色，圆角外就会露出黑角。见 `App::paint_widget`。

### 4. 写初始文本会触发回调回读

`SetWindowTextW` 会同步触发 `EN_CHANGE` 回调，回调里回读两个输入框写回状态。初始化时先写账户框，此刻密码框还是空的，密码就被冲成空串了。`App` 用 `syncing` 标志在初始化期间屏蔽该回调。

### 5. `GdipSaveImageToFile` 要的是编码器 CLSID

`ImageFormatPNG` 是"图像格式"标识，不是"编码器" CLSID，两者 GUID 不同。传错会得到一个 0 字节的空文件，且没有明显报错。见 `preview.rs`。

### 6. 窗口圆角区域是 1 位掩码，做不出抗锯齿

`SetWindowRgn` / `CreateRoundRectRgn` 生成的区域只有「在内 / 不在」两种状态，半径 14px 的圆弧只能用 4~5 级整像素台阶逼近，深色壁纸下就是一圈肉眼可见的毛刺 —— 这不是绘制质量问题，是窗口形状这一层根本表达不了灰度。

而真正能抗锯齿的 `UpdateLayeredWindow` 逐像素 alpha 分层窗口**不承载原生子控件**（EDIT 会整片不显示）。两者不可兼得，所以拆成两个窗口：背景层画外观（`backdrop.rs`），主窗口装控件并把自身区域内缩 2px 藏进背景层的白卡里。细节见 `backdrop.rs` 顶部注释。

注意 `UpdateLayeredWindow` 的 `AC_SRC_ALPHA` 要的是**预乘 alpha**，而 GDI+ 往 HDC 上画不会写 alpha 通道 —— 所以背景层没有用 GDI+，而是自己按 8x8 超采样算覆盖率、直接写预乘的 BGRA。

### 7. 原生单行 EDIT 的文字是顶对齐的，不是垂直居中

单行 EDIT 把文字顶到客户区上沿，基线 = 上沿 + `tmAscent`（`tmAscent` 含 `tmInternalLeading`）。而 GDI 的 `DT_VCENTER` 是把整个字高盒子垂直居中。同一个 30px 高的输入框里，前者比后者高 5px 左右 —— 看起来就是"字浮在框偏上方"。

要让两者落在同一条基线上，EDIT 的上沿必须摆在「字段中心 − `tmHeight` / 2」，其中 `tmHeight` 取自 `GetTextMetricsW`。见 `theme.rs` 的 `Layout::compute_with` 与 `gfx.rs` 的 `Fonts::body_line_h`。

同理，离屏预览里画输入框文字要用**整行**矩形（`account_text`）而不是 EDIT 的矩形，否则预览和真实窗口对不上。

---

## 目录结构

```
src/                    客户端
├── main.rs         程序入口、消息循环、DPI 感知、离屏渲染模式
├── app.rs          App 状态、子控件创建与布局、状态机、托盘联动、绘制调度
├── wnd.rs          窗口类注册与窗口过程
├── backdrop.rs     卡片背景层（分层窗口 + 抗锯齿圆角光栅化）
├── automation.rs   客户端窗口自动化（定位 / 填值 / 点击 / 错误判定）
├── process.rs      基于 Toolhelp32 快照的进程检测
├── settings.rs     注册表读写（QSettings 等价）
├── net.rs          重置密码接口（WinHTTP）
├── tray.rs         托盘图标与自绘右键菜单
├── paint.rs        各组件绘制
├── theme.rs        配色与布局计算
├── gfx.rs          GDI+ 绘图封装 + GDI 文本
├── icon.rs         相机图标（GDI+ 复刻 app.svg）与 HICON 生成
├── preview.rs      离屏渲染成 PNG
├── version.rs      版本号与新旧版本状态（版本号唯一来源）
├── update.rs       向服务端查最新版本，驱动版本徽章的颜色
├── model.rs        状态枚举
├── trace.rs        启动追踪（--selftest）
└── util.rs         矩形 / 颜色 / 宽字符串工具

server/                 更新服务端（独立 crate）
├── main.rs         程序入口、命令行（--publish / --render-preview / --export-icon）
├── app.rs          管理窗口：状态、版本列表、请求日志、发布操作
├── wnd.rs          窗口类注册与窗口过程
├── http.rs         极简 HTTP/1.1 服务（std::net，零框架）
├── store.rs        版本仓库：元数据持久化 + exe 版本号解析
├── publish.rs      发布交互：文件选择 + 参数对话框
├── config.rs       监听地址等本机配置
├── paint.rs        管理界面绘制
├── theme.rs        配色与布局
├── backdrop.rs     卡片背景层（与客户端同一套圆角方案）
├── gfx.rs          GDI+ 绘图封装 + GDI 文本
├── icon.rs         服务端图标（机架服务器）
├── preview.rs      离屏渲染 + 多尺寸 ico 生成
├── model.rs        交互状态枚举
└── trace.rs / util.rs
```

---

## 调试辅助

### 离屏渲染

没有桌面也能逐像素检查界面：

```bash
AutoLoginQatarTool.exe --render-preview <输出目录> [缩放倍数]
```

会输出 `00-图标.png` 与 `01..06-*.png`（已停止 / 运行中 / 登录中-密码明文 / 悬停停止按钮 / 悬停关闭按钮 / 版本有更新）。

`06-版本有更新.png` 专门用来核对「有新版本」时的黄色徽章 —— 绿色那几张已经覆盖了「已是最新」。

### 直接打开密码对话框

```bash
AutoLoginQatarTool.exe --open-dialog auto     # 自动重置密码（7 个密码）
AutoLoginQatarTool.exe --open-dialog custom   # 自定义密码
```

不写界面交互就能核对对话框的排版，配合 `--render-preview` 一起用。

### 启动追踪

窗口不出现时，让程序自己把"走到哪一步"落盘：

```bash
AutoLoginQatarTool.exe --selftest <日志文件>
```

自检模式会在 4 秒后自动退出。

### 接口地址覆盖

重置密码接口的地址可用环境变量覆盖，便于自测与灰度：

```
AUTOLOGIN_RESET_HOST=127.0.0.1
AUTOLOGIN_RESET_PORT=18099
```

### 强制显示「有新版本」

不看服务端，直接把徽章改成黄色，用来核对样式：

```
AUTOLOGIN_FORCE_UPDATE=1
```

不会真的去下载任何东西。

### 服务端的命令行

管理界面能做的事，命令行也能做 —— 便于脚本化部署与 CI：

```bash
# 发布一个版本：版本号自动从 exe 资源里读，不需要手输
AutoLoginServer.exe --publish <exe 路径> --notes "更新说明" [--force]

# 数据目录换一个（默认是 exe 同级的 data/）
AutoLoginServer.exe --data <目录>

# 离屏渲染管理界面 / 重新生成 exe 图标
AutoLoginServer.exe --render-preview <目录> [缩放]
AutoLoginServer.exe --export-icon assets/icons/server.ico
```

`--publish` 和界面里的「发布新版本」走的是同一份 `store::publish`，
所以两条路发出来的版本不会不一致。

---

## 未验证的部分

- **未对生产接口发起过真实重置请求**（会给真人钉钉发新密码，副作用太大）。网络路径是用本地回环 HTTP 服务验证的：确认程序发出了 `GET /api/user/forgetpwd?uid=<工号>` 并正确处理了响应体。
- **未在真实 `QaTar.exe` / `QtPacker.exe` 客户端上跑过完整登录**。窗口自动化逻辑是逐条对照原版 C++ 代码移植的，定位规则、消息发送方式（`SendMessage` / `PostMessage`）、错误码匹配均保持一致。
- **服务端只在同一台机器上用回环地址验证过**（本机起服务端 → 客户端连 `127.0.0.1` → 徽章变色；下载内容与源文件 sha256 一致）。跨机器、跨网段、防火墙放行这些都还没试过，部署前先确认 `18082` 端口在目标网段是通的。
- **自动更新的自我替换只在测试目录里验证过**（1.0.2 → 点徽章 → 换成 1.0.3 → 旧文件变 `.old` → 重启）。没在「exe 放在 `Program Files` 之类需要管理员权限的目录」的场景下试过 —— 那种情况改名会失败，程序会提示"可能没有写权限"并保持原样。
- **服务端的 `force`（强制更新）字段目前只是透传**：客户端会把它读进来、写进日志，但还没有"不可取消"的强制升级行为。
