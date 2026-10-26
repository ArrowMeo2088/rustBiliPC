//! 凭据过期（`-101` 那一类）的统一处置：清掉本地凭据，**自动开一个扫码登录小窗**。
//!
//! ## 为什么要有这一层（P13，用户点名的三件事之一）
//!
//! 他要的原话："凭据过期（-101）→ 自动开登录小窗重登"。
//! 之前只有启动那一次会判定"有没有凭据"（`main.rs`）；`SESSDATA` 是会过期的，
//! 过期之后每一路接口都回 `-101`，界面就只能一行红字"未登录"，他得自己想到"去把 auth.json 删了重开"。
//! 现在：任何一路请求回"要登录"，就在**当前这个进程里**把登录窗开出来，扫完码继续用。
//!
//! ## 判定用库的，不自己猜码
//!
//! [`BpiError::requires_login`]（`ref/bpi-rs/src/err/error.rs:310`）已经把口径定好了：
//! `code ∈ {-101, -401, 800501007}` 或 HTTP 401。这一层不重复列一遍数字，
//! 免得库改了口径我们还在按老码判。
//!
//! ## 三条去重与自愈的规则
//!
//! 1. **一次只开一个登录窗**。同一屏数据往往是四五路请求一起回来的（首页、头像、收藏…），
//!    它们会同时吃 `-101`。"登录窗开没开"记在 [`COORDINATOR`] 里，开着就不再开。
//! 2. **记的是窗口句柄而不是 bool**：用户可能直接把这个小窗关掉（不扫了）。
//!    那就把它当"没开"——判据是"句柄还能不能 update 成功"，所以关掉之后下一次 `-101` 会再开一次，
//!    不需要 `on_window_closed` 那种观察者（它要给 `WindowId`，还得有人长期持有 `Subscription`）。
//! 3. **没登记客户端就不开**。启动时本机就没有凭据的话，`main.rs` 已经开了登录窗，
//!    那时候 [`MainShell`] 还不存在；这一层只处理"跑起来之后凭据才失效"的情况。
//!
//! ## 登录成功之后做什么
//!
//! [`crate::pages::gate`] 的 `Relogin` 模式**只关掉自己**（不能再开一个主窗口，
//! 那会出现两个主窗），然后叫外壳用新凭据重取第一屏。子进程（mpv）不受影响：
//! 外壳不会因为重登而清掉下钻栈，正在放的片子不会被打断（QODER.md"子进程必须有归宿"）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use gpui_kit::*;

use crate::pages::shell::MainShell;
use crate::pages::windows;
use crate::store::auth::Auth;

/// "这一批结果里有过一次 -101"。由 [`message`] 记、由 [`flush`] 清。
static NEEDS_RELOGIN: AtomicBool = AtomicBool::new(false);

/// 进程内的重登现场。三个字段都可能还没人登记，所以全是 `Option`。
struct Coordinator {
    /// 与主窗口共用那一份客户端（登录窗要拿它扫码、写 cookie）。
    client: Option<Arc<bpi_rs::BpiClient>>,
    /// 主窗口的外壳。登录成功后叫它重取第一屏；用弱引用，窗口关了它就该没了。
    shell: Option<WeakEntity<MainShell>>,
    /// 已经开出去的登录窗（启动的那一个也登记在这里，见 [`note_login_window`]）。
    login_window: Option<AnyWindowHandle>,
}

static COORDINATOR: Mutex<Coordinator> = Mutex::new(Coordinator {
    client: None,
    shell: None,
    login_window: None,
});

/// 取一份协调器的拷贝（句柄与 Arc 都是可克隆的轻东西）。
///
/// 每次都是"取出来—用完—写回去"，不长期持有锁：`open_login_window` 会强制画一帧，
/// 抱着锁等它等于把整条 gpui 主线程的路堵在自己手里。
/// 锁中毒（别处在更新协调器时 panic 过）直接沿用内部值：这里只有一个结构体，
/// 里面不可能有"半个更新"留下的坏数据。
fn with<T>(f: impl FnOnce(&mut Coordinator) -> T) -> T {
    let mut guard = COORDINATOR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

/// 登记客户端（`main.rs` 建好 `BpiClient` 之后调一次）。
pub fn set_client(client: Arc<bpi_rs::BpiClient>) {
    with(|state| state.client = Some(client));
}

/// 登记外壳（`MainShell::new` 调）。之后重登成功才知道该刷新谁。
pub fn set_shell(shell: WeakEntity<MainShell>) {
    with(|state| state.shell = Some(shell));
}

/// 登记"现在有一个登录窗开着"。启动时 `main.rs` 开的那一个也要登记进来，
/// 否则跑起来后第一次 `-101` 会再开一个同样的窗。
pub fn note_login_window(handle: AnyWindowHandle) {
    with(|state| state.login_window = Some(handle));
}

/// 登录窗现在到底还在不在（顺便自愈掉已被关掉的句柄）。
fn login_window_alive(cx: &mut App) -> bool {
    let Some(handle) = with(|state| state.login_window.clone()) else {
        return false;
    };
    // 句柄能 update 成功 = 窗口还在。用户按 X 关掉之后这里必然失败，就把登记清掉。
    let alive = handle.update(cx, |_view, _window, _cx| ()).is_ok();
    if !alive {
        with(|state| state.login_window = None);
    }
    alive
}

/// 重登成功后收尾：清掉登录窗登记，外壳句柄留着（下一次过期还要用它）。
pub fn finish_login() {
    with(|state| state.login_window = None);
}

/// 拿走外壳句柄（`gate` 的 Relogin 模式要用它刷新第一屏）。
pub fn take_shell() -> Option<WeakEntity<MainShell>> {
    with(|state| state.shell.clone())
}

/// 有一次"要登录"的错误回来了 —— 该不该弹重登窗？
///
/// 这条只是**判断**，不碰界面（`AsyncApp` 里开不了窗口，见调用点）。
/// 判定用 [`bpi_rs::BpiError::requires_login`]，不在这里抄一遍错误码。
pub fn needs_relogin(error: &bpi_rs::BpiError) -> bool {
    error.requires_login()
}

/// 把错误翻成界面文案，顺手记下"这一路是凭据失效"（给不走 [`loading`](super::loading) 样板的手写 `cx.spawn` 用）。
///
/// 为什么用进程级标志而不是返回值：这些调用点写的是 `inner.map_err(|error| error.to_string())`，
/// 换成返回值要多改一遍 `Result` 的形状；一个原子布尔就把"这一批请求里有过 -101"记住了。
pub fn message(error: &bpi_rs::BpiError) -> String {
    if needs_relogin(error) {
        NEEDS_RELOGIN.store(true, Ordering::Relaxed);
    }
    error.to_string()
}

/// 把攒下的"要登录"处置掉一次（在 `AsyncApp` 这一层调，也就是实体更新之外）。
///
/// 标志是 `swap` 掉的：一批四五路请求同时吃 `-101` 时，第一个 flush 到的人开窗，
/// 后面的看见标志已经清零就什么都不做（[`prompt`] 里那句"窗还开着就不再开"是第二道保险）。
pub fn flush(cx: &mut AsyncApp) {
    if !NEEDS_RELOGIN.swap(false, Ordering::Relaxed) {
        return;
    }
    log::info!("重登：这一批请求里有凭据失效的，交给重登处置");
    cx.update(|app| prompt(app));
}

/// 处置一次"要登录"：清凭据 + 开登录窗。**必须在能开窗口的上下文里调**（`&mut App`）。
///
/// 幂等：登录窗已经开着就只记一条日志。没登记客户端（也就是还没有主窗口）时什么都不做——
/// 那种情况由 `main.rs` 的启动分支负责开窗口。
pub fn prompt(cx: &mut App) {
    if login_window_alive(cx) {
        log::debug!("重登：登录窗已经开着，不再开第二个");
        return;
    }
    let Some(client) = with(|state| state.client.clone()) else {
        log::debug!("重登：还没登记客户端（主窗口还没建起来），交给启动流程");
        return;
    };

    // 过期凭据留在盘上只有坏处：下载与 CDN 探测都直接读它（`probe::cdn_headers`），
    // 留着就是"每一起播都带着一条死 cookie 去撞 403"。删掉不可逆，所以先写日志再删。
    if let Err(error) = Auth::clear() {
        log::warn!("重登：删除过期凭据失败（仍然开登录窗）: {error}");
    } else {
        log::info!("重登：凭据已被接口判定为无效，删除本地凭据并开扫码登录窗");
    }

    match windows::open_login_window(client, crate::pages::gate::GateMode::Relogin, cx) {
        Ok(handle) => note_login_window(handle),
        Err(error) => log::error!("重登：开登录小窗失败: {error}"),
    }
}
