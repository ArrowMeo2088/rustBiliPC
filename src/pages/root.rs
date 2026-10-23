//! 根视图：决定这一轮显示登录页还是主界面。
//!
//! 启动规则（定稿）：可执行文件同目录没有完整凭据 → 先扫码登录；有 → 直接进主界面，
//! 并把凭据灌进共享的 `BpiClient`。登录成功后由 [`LoginPage`] 发 [`LoginSucceeded`]（带 mid），
//! 这里换成 [`MainShell`]，同一个窗口继续用，不额外开窗口。
//!
//! 主界面之后不再由根视图管页面内容：搜索/个人的切换收在 `pages/shell.rs` 那一层。

use std::sync::Arc;

use gpui_kit::*;

use crate::pages::login::{LoginPage, LoginSucceeded};
use crate::pages::shell::MainShell;
use crate::store::credentials::SavedCredential;

/// 当前阶段。持有子视图实体——`Entity<V>: IntoElement` 成立（`gpui/src/view.rs:104`），
/// 所以可以直接当 child 渲染。
enum Stage {
    Login(Entity<LoginPage>),
    Main(Entity<MainShell>),
}

/// 窗口根视图。
pub struct RootView {
    stage: Stage,
    /// 登录页与主界面共用同一个客户端（cookie 与 WBI 缓存都在它里面）。
    client: Arc<bpi_rs::BpiClient>,
    /// 订阅要跟根视图同寿命。
    _subscriptions: Vec<Subscription>,
}

impl RootView {
    /// 依据本机凭据决定初始阶段。
    pub fn new(window: &mut Window, cx: &mut Context<Self>, client: Arc<bpi_rs::BpiClient>) -> Self {
        // 分开记两件事：文件能不能读，和读到的内容够不够格当凭据（缺 buvid3 就不合格）。
        let saved = SavedCredential::load();
        log::debug!(
            "启动判定：凭据文件 {:?}（可读={}，字段完整={:?}）",
            SavedCredential::path(),
            saved.is_some(),
            saved.as_ref().map(|credential| credential.is_complete())
        );

        match saved.filter(|credential| credential.is_complete()) {
            Some(credential) => {
                log::info!("启动判定：本机已有凭据，直接进入主界面 mid={}", credential.dede_user_id);
                if let Err(error) = client.set_account(credential.to_account()) {
                    // 凭据可能已失效；不致命，后续接口会用错误码提示重新登录。
                    log::warn!("复用已保存的登录态失败: {error}");
                }

                let mid = credential.dede_user_id.clone();
                let shell = cx.new(|cx| {
                    MainShell::new(window, cx, Arc::clone(&client), mid)
                });
                Self {
                    stage: Stage::Main(shell),
                    client,
                    _subscriptions: Vec::new(),
                }
            }
            None => {
                log::info!("启动判定：无有效凭据，进入扫码登录页");
                let login = cx.new(|cx| LoginPage::new(Arc::clone(&client), cx));
                let subscription =
                    cx.subscribe_in(&login, window, |this, _login, event: &LoginSucceeded, window, cx| {
                        log::info!("收到登录成功事件，mid={}", event.mid);
                        let shell = cx.new(|cx| {
                            MainShell::new(window, cx, Arc::clone(&this.client), event.mid.clone())
                        });
                        this.stage = Stage::Main(shell);
                    });

                Self {
                    stage: Stage::Login(login),
                    client,
                    _subscriptions: vec![subscription],
                }
            }
        }
    }
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(match &self.stage {
            Stage::Login(view) => view.clone().into_any_element(),
            Stage::Main(view) => view.clone().into_any_element(),
        })
    }
}
