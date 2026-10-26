//! 窗口这一层：两个窗口各要多大、怎么开、登录完怎么换。
//!
//! ## 为什么窗口要分开（2026-10-26 用户定稿）
//!
//! 原话："登录单独做一个窗口，登录完成后凭据保存到运行同目录的 auth.json……
//! 启动时若没有 auth.json，先弹窗让用户扫码登录，注意这个扫码登录的窗口做小一点，
//! 只要能显示 rustBiliPC 的 logo 和登录二维码即可。"
//!
//! 于是 `root.rs` 那套"同一窗口内换视图"退役了（旧结论留在 `docs/architecture.md` 第 4.6 节，
//! 这一节是它的新版本）：**没凭据 → 只开一个 320×420 的小登录窗；有凭据 → 直接开主窗口**。
//!
//! ## 换窗口时的两条硬约束（顺序不能反）
//!
//! 1. **先开主窗口，再关登录窗口**。gpui 在 Windows 上的默认 `QuitMode` 是
//!    "最后一个窗口关闭就退进程"（`ref/gpui/gpui/src/app.rs:458` 的 `Default` 取
//!    `cfg!(not(target_os = "macos"))`，判定在 `:3180-3188`）。反过来的话中间会出现
//!    一个"零窗口"的瞬间，进程当场自己退出，主窗口根本来不及开。
//! 2. **不能在主窗口自己的 update 里开/关窗口**。`App::open_window` 会强制画一帧才返回
//!    （`app.rs:1520`），从别的窗口更新过程中重入是不安全写法；所以 [`open_main_and_close_login`]
//!    只在任务里调用（`Context::spawn` → `AsyncApp::update`），这也是 kit 文档给的姿势
//!    （"In an async context, call this inside `cx.update`"，`ref/gpui-kit/crates/kit/src/lib.rs:180`）。
//!
//! ## 登录窗为什么保留系统标题栏
//!
//! 无边框要自己处理拖动与关闭按钮（kit 的 `TitleBar`），为了一个小窗引一套弹层状态不值得；
//! 它只需要装得下 logo + 二维码 + 一行状态 + 一个按钮。

use std::sync::Arc;

use gpui_kit::*;

use crate::pages::shell::MainShell;

/// 主窗口尺寸：1120×760，与之前几版一致（视觉是按 `ref/Bili.Copilot` 的
/// "240 侧栏 + 主区 + 360 右栏"排的，侧栏收到 80 之后这块只需要更宽，不动它）。
pub const MAIN_WINDOW_WIDTH: f32 = 1120.;
pub const MAIN_WINDOW_HEIGHT: f32 = 760.;

/// 登录窗口尺寸。里面只有：应用名一行、二维码（25 模块 × 6 px = 150，见 `ui::qr`）、
/// 一行状态、一个按钮，加上上下内边距 420 足够；宽 320 是按 150 的二维码 + 左右 16 反推的。
pub const LOGIN_WINDOW_WIDTH: f32 = 320.;
pub const LOGIN_WINDOW_HEIGHT: f32 = 420.;

fn window(title: &str, width: f32, height: f32, resizable: bool, cx: &mut App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::centered(
            size(px(width), px(height)),
            cx,
        )),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            ..Default::default()
        }),
        // `focus` 默认是 false（`platform.rs:2501`），不写就等于新窗口开出来不带键盘焦点。
        focus: true,
        is_resizable: resizable,
        ..Default::default()
    }
}

/// 开主窗口。`mid_raw` 是凭据里的 `DedeUserID`，外壳拿它去要自己的数据。
pub fn open_main_window(
    client: Arc<bpi_rs::BpiClient>,
    mid_raw: String,
    cx: &mut App,
) -> anyhow::Result<()> {
    let options = window("rustBiliPC", MAIN_WINDOW_WIDTH, MAIN_WINDOW_HEIGHT, true, cx);
    gpui_kit::open_window(options, cx, move |window, cx| {
        // 闭包按 FnOnce 用即可，但 clone 一份客户端进去：这样两种签名下都不会借用外部状态。
        let client = Arc::clone(&client);
        let mid = mid_raw.clone();
        cx.new(|cx| MainShell::new(window, cx, client, mid))
    })?;
    Ok(())
}

/// 登录成功后换窗口：**开主窗口 → 关掉登录窗口**，两步之间不出现零窗口。
///
/// `login_window` 是登录窗自己的句柄（由 `LoginGate` 从构造时的 `&mut Window` 取到，
/// `ref/gpui/gpui/src/window.rs:2371`）。主窗口开失败时**不关登录窗**：
/// 用户至少还看得见二维码和错误提示，比两片空白强。
pub fn open_main_and_close_login(
    client: Arc<bpi_rs::BpiClient>,
    mid_raw: String,
    login_window: AnyWindowHandle,
    cx: &mut App,
) -> anyhow::Result<()> {
    open_main_window(client, mid_raw, cx)?;
    let _ = login_window.update(cx, |_view, window, _cx| window.remove_window());
    Ok(())
}

/// 开登录窗口（只在"本机没有可用凭据"时调用）。
pub fn open_login_window(
    client: Arc<bpi_rs::BpiClient>,
    cx: &mut App,
) -> anyhow::Result<()> {
    let options = window(
        "登录 rustBiliPC",
        LOGIN_WINDOW_WIDTH,
        LOGIN_WINDOW_HEIGHT,
        false,
        cx,
    );
    gpui_kit::open_window(options, cx, move |window, cx| {
        let client = Arc::clone(&client);
        cx.new(|cx| crate::pages::gate::LoginGate::new(window, cx, client))
    })?;
    Ok(())
}
