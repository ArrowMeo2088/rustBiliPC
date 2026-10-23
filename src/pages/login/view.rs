//! 扫码登录页。
//!
//! 流程（定稿：登录先行，没有凭据就先看到这一页）：
//! 1. `qr_generate` 拿 URL + `qrcode_key`，本地编码成模块矩阵画出来；
//! 2. 每 1.5 秒 `qr_poll` 一次，按 B 站的码更新提示；
//! 3. `code == 0` 时组装 `SavedCredential`、写盘、`set_account`，然后发 [`LoginSucceeded`]；
//! 4. 二维码过期则自增 generation 并重新走一遍，旧循环发现自己过期后自行退出。
//!
//! ## 两条必须遵守的规矩（都是真机踩出来的）
//!
//! - **`cx.spawn` 返回的 `Task` 必须被持有或 `detach()`**。gpui 的执行器写得很明确
//!   （`gpui-pre-scheduler/src/executor.rs:396-402`）：*"If you drop a task it will be
//!   cancelled immediately"*，而且 `Task` 是 `#[must_use]`。之前这里写成裸 `cx.spawn(...);`，
//!   future 在第一次 poll 前就被取消：tokio 侧请求照发（`fetch_qr` 在 spawn 之前同步调用），
//!   界面却永远等不到结果，日志里连闭包第一行都没有 —— CI 的 `unused_must_use` 警告就是它。
//!   现在靠 `[lints] unused_must_use = "deny"` 让这类写法直接编译失败。
//! - **不要从任务自己的 future 里丢弃该任务的 `Task`**（`Task::cancel` 会等任务停止，
//!   自己等自己 = 主线程死锁）。所以这里统一 `detach()`，改用 `generation` 序号让旧循环
//!   自己判断"我已经被取代了"然后退出。
//!
//! 不开第二个窗口：登录成功后由根视图在同一窗口内换成主界面（`docs/architecture.md` 第 4.6 节）。

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;

use crate::api::login::{self, QrPollState, QrSession, credential_from_cookies};
use crate::store::credentials::SavedCredential;
use crate::ui::qr;

/// 轮询间隔。B 站的 poll 接口对频率不敏感，1.5 秒既跟手又不刷接口。
const POLL_INTERVAL: Duration = Duration::from_millis(1500);

/// 心跳次数，仅用于诊断（证明前台执行器能被后台事件唤醒）。
const HEARTBEAT_TICKS: u8 = 5;

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
    /// 每启动一轮新流程就 +1；旧循环发现自己不是最新代就退出。
    generation: u32,
}

impl LoginPage {
    /// 建页并启动登录流程。
    pub fn new(client: Arc<bpi_rs::BpiClient>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            client,
            session: None,
            matrix: Vec::new(),
            status: "正在获取登录二维码…".into(),
            finished: false,
            generation: 0,
        };
        this.refresh_qr(cx);
        this.start_heartbeat(cx);
        this
    }

    /// 申请新二维码（初次进入、过期重载、手动刷新都走这里）。
    fn refresh_qr(&mut self, cx: &mut Context<Self>) {
        if self.finished {
            return;
        }

        self.generation += 1;
        let generation = self.generation;
        self.session = None;
        self.matrix = Vec::new();
        self.status = "正在获取登录二维码…".into();
        cx.notify();

        let join = login::fetch_qr(Arc::clone(&self.client));
        log::debug!("登录页：等待 qr_generate 的结果（代 {generation}）");

        cx.spawn(async move |this, cx| {
            let outcome = match join.await {
                Ok(inner) => inner.map_err(|error| error.to_string()),
                Err(join_error) => Err(crate::app::describe_join_error(join_error)),
            };
            log::debug!("登录页：qr_generate 结果已回到前台（代 {generation}）");

            let Some(view) = this.upgrade() else {
                log::warn!("登录页：等待期间实体已释放，丢弃二维码结果");
                return;
            };
            view.update(cx, |view, cx| {
                if !view.is_current(generation) {
                    log::debug!("登录页：代 {generation} 已被取代，丢弃一次结果");
                    return;
                }
                match outcome {
                    Ok(session) => view.on_qr_received(session, cx),
                    Err(message) => {
                        log::warn!("登录页：取二维码失败: {message}");
                        view.status = format!("获取二维码失败: {message}").into();
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// 每 2 秒记一行，共 `HEARTBEAT_TICKS` 次；不显示任何界面内容。
    fn start_heartbeat(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |_this, cx| {
            for tick in 1..=HEARTBEAT_TICKS {
                cx.background_executor()
                    .timer(Duration::from_millis(2000))
                    .await;
                log::debug!("登录页心跳 {tick}/{HEARTBEAT_TICKS}（前台执行器在跑）");
            }
        })
        .detach();
    }

    /// 这一代任务是否还是最新的。
    fn is_current(&self, generation: u32) -> bool {
        self.generation == generation && !self.finished
    }

    /// 拿到二维码后：编码、显示、启动轮询。
    fn on_qr_received(&mut self, session: QrSession, cx: &mut Context<Self>) {
        log::debug!("登录页：进入 on_qr_received，开始编码二维码");
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

    /// 轮询循环：一直等到成功、失败或过期（过期时新循环已在 `refresh_qr` 里接班，本循环退出）。
    fn start_polling(&mut self, session: QrSession, cx: &mut Context<Self>) {
        let client = Arc::clone(&self.client);
        let key = session.qrcode_key;
        let generation = self.generation;
        log::debug!("登录页：启动扫码轮询（代 {generation}）");

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;

                // 已经被更新的流程取代（例如过期后重建），本循环让位。
                let alive = this
                    .upgrade()
                    .map(|view| view.update(cx, |view, _cx| view.is_current(generation)))
                    .unwrap_or(false);
                if !alive {
                    log::debug!("登录页：代 {generation} 的轮询退出");
                    return;
                }

                let join = login::poll_qr(Arc::clone(&client), key.clone());
                let state = match join.await {
                    Ok(Ok(state)) => state,
                    // 网络抖动或接口错误：不打断用户，下一轮继续问。
                    Ok(Err(error)) => {
                        log::warn!("扫码轮询失败: {error}");
                        continue;
                    }
                    Err(join_error) => {
                        let message = crate::app::describe_join_error(join_error);
                        log::error!("扫码轮询任务异常: {message}");
                        if let Some(view) = this.upgrade() {
                            view.update(cx, |view, cx| {
                                if view.is_current(generation) {
                                    view.status = message.into();
                                    cx.notify();
                                }
                            });
                        }
                        continue;
                    }
                };

                match state {
                    QrPollState::Waiting => continue,
                    QrPollState::Scanned => {
                        if let Some(view) = this.upgrade() {
                            view.update(cx, |view, cx| {
                                if view.is_current(generation) {
                                    view.status = "已扫码，请在手机上确认登录".into();
                                    cx.notify();
                                }
                            });
                        }
                        continue;
                    }
                    QrPollState::Expired => {
                        log::debug!("登录页：二维码过期，重新申请一张");
                        if let Some(view) = this.upgrade() {
                            view.update(cx, |view, cx| view.refresh_qr(cx));
                        }
                        return;
                    }
                    QrPollState::Authorized {
                        cookies,
                        refresh_token,
                    } => {
                        match credential_from_cookies(&cookies, &refresh_token) {
                            Some(credential) => {
                                if let Some(view) = this.upgrade() {
                                    view.update(cx, |view, cx| {
                                        view.complete(credential, cx)
                                    });
                                }
                            }
                            None => {
                                // 只记 cookie 的**名字**：日志是要被人贴出来排查问题的，
                                // 把 SESSDATA 打进去等于把登录态贴出去。
                                let names = cookies
                                    .iter()
                                    .map(|(name, _)| name.as_str())
                                    .collect::<Vec<_>>()
                                    .join(",");
                                log::error!("登录成功但响应里缺少必要 cookie: [{names}]");
                                if let Some(view) = this.upgrade() {
                                    view.update(cx, |view, cx| {
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
                            view.update(cx, |view, cx| {
                                view.status = format!("登录失败（{code}）: {message}").into();
                                cx.notify();
                            });
                        }
                        return;
                    }
                }
            }
        })
        .detach();
    }

    /// 落盘凭据、灌进客户端、通知根视图。
    fn complete(&mut self, credential: SavedCredential, cx: &mut Context<Self>) {
        if self.finished {
            return;
        }
        self.finished = true;
        log::info!("登录成功，写入凭据 mid={}", credential.dede_user_id);

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
            // 出问题时用户能直接找到日志。
            .child(
                div()
                    .text_xs()
                    .child(format!("日志：{}", crate::logging::log_path().display())),
            )
    }
}
