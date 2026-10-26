//! 登录窗口的根视图：显示二维码 + 登录成功后按模式收尾。
//!
//! ## 它只做两件我们已经在别处验证过的事
//!
//! 1. 把 [`LoginPage`] 当 child 摆上（实体当元素用，`Entity<V>: IntoElement` 成立）；
//! 2. 订阅它发出来的 [`LoginSucceeded`]，然后在**任务里**收尾。
//!
//! 第 2 件为什么必须走任务而不是在事件回调里直接开：`App::open_window` 会强制画一帧才返回
//! （`ref/gpui/gpui/src/app.rs:1520`），而事件回调正跑在登录窗自己的一次更新里，
//! 在那儿开新窗是不安全写法。任务里拿到的是 `AsyncApp`，`cx.update(|app| ...)` 之后
//! 才是"应用层"的上下文——这也是 kit 文档给的姿势（`crates/kit/src/lib.rs:180`）。
//! 换窗口的两条硬约束（**先开主窗、再关登录窗**）写在 [`windows`](crate::pages::windows) 文件头。
//!
//! ## 两种模式（P13 加）
//!
//! [`GateMode::Startup`] 是原来那条：本机没凭据 → 登录完开主窗、关自己。
//! [`GateMode::Relogin`] 是"跑起来之后凭据过期"那一条（[`relogin`] 开的窗）：
//! **主窗口已经在了，所以只关自己**，再叫外壳用新凭据重取第一屏。
//! 走错模式的症状很具体：重登时开主窗 = 桌面上出现两个一模一样的主窗口。
//!
//! ## 为什么还要留一个自己的实体
//!
//! 本来可以让 `LoginPage` 自己发事件、`main.rs` 在应用层订阅，但那条 `Subscription` 得有人持有：
//! `main.rs` 的 run 闭包里的局部变量在闭包返回时就析构了，订阅当场被取消（`subscription.rs:147-152`）。
//! 挂在一个真实存在的视图上最省事，也符合本项目"订阅跟着实体字段走"的既有做法。

use std::sync::Arc;

use gpui_kit::*;

use crate::pages::login::{LoginPage, LoginSucceeded};
use crate::pages::{relogin, windows};

/// 登录窗登录成功之后该干什么。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateMode {
    /// 本机没凭据：开主窗口，关掉自己。
    Startup,
    /// 凭据过期后的重登：主窗口还在，只关掉自己并让它刷新。
    Relogin,
}

/// 登录窗口根视图。
pub struct LoginGate {
    client: Arc<bpi_rs::BpiClient>,
    login: Entity<LoginPage>,
    /// 自己所在的那个窗口：主窗口开起来之后要关掉它。
    /// 从构造时的 `&mut Window` 取（`window.rs:2371` 的 `window_handle()`），
    /// 不需要 `main.rs` 把句柄传进来。
    window: AnyWindowHandle,
    mode: GateMode,
    /// 订阅要跟这个视图同寿命，一 drop 就取消。
    _subscription: Subscription,
}

impl LoginGate {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        client: Arc<bpi_rs::BpiClient>,
        mode: GateMode,
    ) -> Self {
        let handle = window.window_handle();
        let login = cx.new(|cx| LoginPage::new(Arc::clone(&client), cx));
        let subscription =
            cx.subscribe_in(&login, window, |this, _login, event: &LoginSucceeded, _window, cx| {
                this.hand_off(event.mid.clone(), cx);
            });

        Self {
            client,
            login,
            window: handle,
            mode,
            _subscription: subscription,
        }
    }

    /// 登录成功后的收尾，按模式分两条路（都在任务里做，理由见文件头）。
    fn hand_off(&mut self, mid: String, cx: &mut Context<Self>) {
        let login_window = self.window;
        match self.mode {
            GateMode::Startup => {
                log::info!("登录窗：准备开主窗口并关闭自己（mid={mid}）");
                let client = Arc::clone(&self.client);
                cx.spawn(async move |_this, cx| {
                    let outcome = cx.update(|app| {
                        windows::open_main_and_close_login(client, mid, login_window, app)
                    });
                    if let Err(error) = outcome {
                        // 开主窗失败时**不关登录窗**（那条路径在 `open_main_and_close_login` 里就返回了）：
                        // 用户至少还看得见二维码与"登录成功"那行字，日志里有原因。
                        log::error!("登录后换窗口失败: {error}");
                    }
                })
                .detach();
            }
            GateMode::Relogin => {
                log::info!("重登窗：凭据已换新，准备关掉自己并刷新主窗口（mid={mid}）");
                let shell = relogin::take_shell();
                cx.spawn(async move |_this, cx| {
                    // 这一段整体在 `AsyncApp::update` 里做（开/关窗口都不能在实体 update 里，
                    // 理由见 `pages/windows.rs` 文件头）。里面的失败都各自写了日志，
                    // 所以这里不再包一层 `Result`——`update` 的返回值是 `()`（第四十四轮的 E0308）。
                    cx.update(|app| {
                        // 先关窗再刷新：刷新会发一批请求，带着旧登录窗在场也只是多一个遮在上面的小窗。
                        let closed = login_window
                            .update(app, |_view, window, _cx| window.remove_window());
                        if closed.is_err() {
                            log::warn!("重登窗：关不掉自己（窗口可能已被用户关掉），仍然继续刷新");
                        }
                        relogin::finish_login();
                        let Some(shell) = shell else {
                            log::warn!("重登：没登记外壳句柄，只能等用户自己切页刷新");
                            return;
                        };
                        if shell
                            .update(app, |shell, cx| shell.reload_after_relogin(mid, cx))
                            .is_err()
                        {
                            log::warn!("重登：主窗口已经不在了，跳过刷新");
                        }
                    });
                })
                .detach();
            }
        }
    }
}

impl Render for LoginGate {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.login.clone())
    }
}
