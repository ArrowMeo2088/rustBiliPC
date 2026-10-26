//! 登录窗口的根视图：显示二维码 + 登录成功后换窗口。
//!
//! ## 它只做两件我们已经在别处验证过的事
//!
//! 1. 把 [`LoginPage`] 当 child 摆上（实体当元素用，`Entity<V>: IntoElement` 成立）；
//! 2. 订阅它发出来的 [`LoginSucceeded`]，然后在**任务里**换窗口。
//!
//! 第 2 件为什么必须走任务而不是在事件回调里直接开：`App::open_window` 会强制画一帧才返回
//! （`ref/gpui/gpui/src/app.rs:1520`），而事件回调正跑在登录窗自己的一次更新里，
//! 在那儿开新窗是不安全写法。任务里拿到的是 `AsyncApp`，`cx.update(|app| ...)` 之后
//! 才是"应用层"的上下文——这也是 kit 文档给的姿势（`crates/kit/src/lib.rs:180`）。
//! 换窗口的两条硬约束（**先开主窗、再关登录窗**）写在 [`windows`](crate::pages::windows) 文件头。
//!
//! ## 为什么还要留一个自己的实体
//!
//! 本来可以让 `LoginPage` 自己发事件、`main.rs` 在应用层订阅，但那条 `Subscription` 得有人持有：
//! `main.rs` 的 run 闭包里的局部变量在闭包返回时就析构了，订阅当场被取消（`subscription.rs:147-152`）。
//! 挂在一个真实存在的视图上最省事，也符合本项目"订阅跟着实体字段走"的既有做法。

use std::sync::Arc;

use gpui_kit::*;

use crate::pages::login::{LoginPage, LoginSucceeded};
use crate::pages::windows;

/// 登录窗口根视图。
pub struct LoginGate {
    client: Arc<bpi_rs::BpiClient>,
    login: Entity<LoginPage>,
    /// 自己所在的那个窗口：主窗口开起来之后要关掉它。
    /// 从构造时的 `&mut Window` 取（`window.rs:2371` 的 `window_handle()`），
    /// 不需要 `main.rs` 把句柄传进来。
    window: AnyWindowHandle,
    /// 订阅要跟这个视图同寿命，一 drop 就取消。
    _subscription: Subscription,
}

impl LoginGate {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        client: Arc<bpi_rs::BpiClient>,
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
            _subscription: subscription,
        }
    }

    /// 登录成功后换窗口：开主窗口 → 关掉自己这个窗口。
    fn hand_off(&mut self, mid: String, cx: &mut Context<Self>) {
        log::info!("登录窗：准备开主窗口并关闭自己（mid={mid}）");
        let client = Arc::clone(&self.client);
        let login_window = self.window;
        cx.spawn(async move |_this, cx| {
            let outcome =
                cx.update(|app| windows::open_main_and_close_login(client, mid, login_window, app));
            if let Err(error) = outcome {
                // 开主窗失败时**不关登录窗**（那条路径在 `open_main_and_close_login` 里就返回了）：
                // 用户至少还看得见二维码与"登录成功"那行字，日志里有原因。
                log::error!("登录后换窗口失败: {error}");
            }
        })
        .detach();
    }
}

impl Render for LoginGate {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.login.clone())
    }
}
