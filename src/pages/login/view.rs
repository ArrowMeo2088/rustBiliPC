//! 扫码登录页。
//!
//! 流程（定稿：登录是启动的第一步，没有凭据就先看到这一页）：
//! 1. `qr_generate` 拿 URL + `qrcode_key`，本地编码成模块矩阵画出来；
//! 2. 每 1.5 秒 `qr_poll` 一次，按 B 站的码更新提示；
//! 3. `code == 0` 时用响应 cookie 组装 `SavedCredential`、写盘、`set_account`，
//!    然后发 [`LoginSucceeded`] 让根视图切到主界面；
//! 4. 二维码过期则自动重新生成并重启轮询。
//!
//! 为什么不开第二个窗口：`AsyncWindowContext::open_window` 虽然可用，但"关掉登录窗口"要拿到
//! 窗口句柄再走 `Window::remove_window`，链路里有两处未在参考仓库里见过的写法。单窗口 +
//! 根视图切换阶段效果相同、风险更低；确实要独立窗口时再改。

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;

use crate::api::login::{self, QrPollState, QrSession, credential_from_cookies};
use crate::store::credentials::SavedCredential;
use crate::ui::qr;

/// 轮询间隔。B 站的 poll 接口对频率不敏感，1.5 秒既跟手又不刷接口。
const POLL_INTERVAL: Duration = Duration::from_millis(1500);

/// 登录成功事件。根视图订阅它，把界面从登录切到主视图。
#[derive(Clone, Debug)]
pub struct LoginSucceeded;

/// 扫码登录页的根视图。
pub struct LoginPage {
    /// 与主界面共用同一个客户端：登录成功后 cookie 直接写进它，省掉重建。
    client: Arc<bpi_rs::BpiClient>,
    /// 当前二维码会话（key 用于轮询）。
    session: Option<QrSession>,
    /// 已编码的模块矩阵；为空表示还在取码或编码失败。
    matrix: Vec<Vec<bool>>,
    /// 页面提示文案。
    status: SharedString,
    /// 成功后停止一切轮询，避免重复发事件。
    finished: bool,
}

impl LoginPage {
    /// 建页并立刻去取第一张二维码。
    pub fn new(client: Arc<bpi_rs::BpiClient>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            client,
            session: None,
            matrix: Vec::new(),
            status: "正在获取登录二维码…".into(),
            finished: false,
        };
        this.refresh_qr(cx);
        this
    }

    /// 申请新二维码（初次进入、过期重载、手动刷新都走这里）。
    fn refresh_qr(&mut self, cx: &mut Context<Self>) {
        if self.finished {
            return;
        }

        self.session = None;
        self.matrix = Vec::new();
        self.status = "正在获取登录二维码…".into();
        cx.notify();

        let join = login::fetch_qr(Arc::clone(&self.client));
        cx.spawn(async move |this, cx| {
            match join.await {
                Ok(Ok(session)) => {
                    if let Some(this) = this.upgrade() {
                        let _ = this.update(cx, |this, cx| this.on_qr_received(session, cx));
                    }
                }
                Ok(Err(error)) => {
                    if let Some(this) = this.upgrade() {
                        let _ = this.update(cx, |this, cx| {
                            this.status = format!("获取二维码失败: {error}").into();
                            cx.notify();
                        });
                    }
                }
                Err(join_error) => {
                    // 任务 panic 会每次都一样地失败，必须显示出来，否则界面永远停在"正在获取"。
                    let message = crate::app::describe_join_error(join_error);
                    log::error!("二维码请求任务异常: {message}");
                    if let Some(this) = this.upgrade() {
                        let _ = this.update(cx, |this, cx| {
                            this.status = message.into();
                            cx.notify();
                        });
                    }
                }
            }
        });
    }

    /// 拿到二维码后：编码、显示、启动轮询。
    fn on_qr_received(&mut self, session: QrSession, cx: &mut Context<Self>) {
        self.matrix = qr::matrix_for(&session.url);
        self.status = if self.matrix.is_empty() {
            "二维码生成失败，请点下方按钮重试".into()
        } else {
            "请使用哔哩哔哩 App 扫描二维码登录".into()
        };
        self.session = Some(session.clone());
        cx.notify();

        self.start_polling(session, cx);
    }

    /// 轮询循环：一直等到成功、失败或过期（过期会重启一次流程）。
    fn start_polling(&mut self, session: QrSession, cx: &mut Context<Self>) {
        let client = Arc::clone(&self.client);
        let key = session.qrcode_key;

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;

                let join = login::poll_qr(Arc::clone(&client), key.clone());
                let state = match join.await {
                    Ok(Ok(state)) => state,
                    // 网络抖动或接口错误：不打断用户，下一轮继续问。
                    Ok(Err(error)) => {
                        log::warn!("扫码轮询失败: {error}");
                        continue;
                    }
                    Err(join_error) => {
                        // panic 类错误重试也不会变好，直接停止本轮并显示，避免"无限轮询"假象。
                        let message = crate::app::describe_join_error(join_error);
                        log::error!("扫码轮询任务异常: {message}");
                        if let Some(view) = this.upgrade() {
                            let _ = view.update(cx, |view, cx| {
                                view.status = message.into();
                                cx.notify();
                            });
                        }
                        return;
                    }
                };

                match state {
                    QrPollState::Waiting => continue,
                    QrPollState::Scanned => {
                        if let Some(view) = this.upgrade() {
                            let _ = view.update(cx, |view, cx| {
                                view.status = "已扫码，请在手机上确认登录".into();
                                cx.notify();
                            });
                        }
                        continue;
                    }
                    QrPollState::Expired => {
                        if let Some(view) = this.upgrade() {
                            let _ = view.update(cx, |view, cx| view.refresh_qr(cx));
                        }
                        // refresh_qr 会启动新一轮轮询，本轮直接退出，避免两条循环并存。
                        return;
                    }
                    QrPollState::Authorized {
                        cookies,
                        refresh_token,
                    } => {
                        let credential = credential_from_cookies(&cookies, &refresh_token);
                        match credential {
                            Some(credential) => {
                                if let Some(view) = this.upgrade() {
                                    let _ = view.update(cx, |view, cx| view.complete(credential, cx));
                                }
                            }
                            None => {
                                log::error!("登录成功但响应里缺少必要 cookie: {cookies:?}");
                                if let Some(view) = this.upgrade() {
                                    let _ = view.update(cx, |view, cx| {
                                        view.status = "登录返回的凭据不完整，请重试".into();
                                        cx.notify();
                                    });
                                }
                            }
                        }
                        return;
                    }
                    QrPollState::Failed { code, message } => {
                        if let Some(view) = this.upgrade() {
                            let _ = view.update(cx, |view, cx| {
                                view.status = format!("登录失败（{code}）: {message}").into();
                                cx.notify();
                            });
                        }
                        return;
                    }
                }
            }
        });
    }

    /// 落盘凭据、灌进客户端、通知根视图。
    fn complete(&mut self, credential: SavedCredential, cx: &mut Context<Self>) {
        if self.finished {
            return;
        }
        self.finished = true;

        if let Err(error) = credential.save() {
            // 写盘失败不影响本次使用，只是下次启动还要重新扫码。
            log::error!("保存登录凭据失败: {error}");
            self.status = format!("登录成功，但凭据保存失败: {error}").into();
        } else {
            self.status = "登录成功".into();
        }

        if let Err(error) = self.client.set_account(credential.to_account()) {
            log::error!("应用登录态失败: {error}");
        }

        cx.notify();
        cx.emit(LoginSucceeded);
    }
}

impl EventEmitter<LoginSucceeded> for LoginPage {}

impl Render for LoginPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let code = if self.matrix.is_empty() {
            // 占位方块保持尺寸稳定，避免二维码到位时整页跳动。
            div()
                .size(px(qr::MODULE_PIXELS * 25.))
                .bg(rgb(0x2A2A2A))
                .into_any_element()
        } else {
            qr::render(&self.matrix)
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(div().text_sm().child("登录后才能解锁 1080P 与个人内容"))
            .child(code)
            .child(div().text_sm().child(self.status.clone()))
            .child(
                Button::new("refresh-qr")
                    .label("重新获取二维码")
                    .on_click(cx.listener(|this, _, _window, cx| this.refresh_qr(cx))),
            )
    }
}
