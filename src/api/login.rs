//! 扫码登录的接口封装（默认登录方式）。
//!
//! ## 已核实的接口形状
//!
//! - `login().qr_generate() -> BpiResult<GenerateQrCodeData { url, qrcode_key }>`
//!   （`src/login/client.rs:193`）
//! - `login().qr_poll(LoginQrPollParams::new(key)?) -> BpiResult<CheckQrCodeStatusData>`
//!   （`src/login/client.rs:202`，字段：`url / refresh_token / timestamp / code / message /
//!   cookies: Vec<(String, String)>`）
//! - 库**不做轮询、不做超时**，节奏由我们掌握（`src/login/cookie_refresh.rs` 是空文件）。
//!
//! ## 凭据字段
//!
//! `qr_poll` 成功时响应的 Set-Cookie 里带着 `DedeUserID / SESSDATA / bili_jct / buvid3`
//! （bpi-rs 的 cookie 白名单见 `src/client.rs:79`），四个都齐才能通过
//! `Account::validate_complete`，所以 [`credential_from_cookies`] 要求四个字段全部命中。

use std::sync::Arc;

use bpi_rs::{BpiClient, BpiResult};

use super::spawn_on_api_runtime;
use crate::store::credentials::SavedCredential;

/// 一次扫码会话的初始信息。
#[derive(Clone, Debug)]
pub struct QrSession {
    /// 供二维码编码的登录 URL。
    pub url: String,
    /// 轮询用的 key。
    pub qrcode_key: String,
}

/// 轮询一次得到的状态（已把 B 站的数字码翻译过）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QrPollState {
    /// 等待用户扫码。
    Waiting,
    /// 已扫码、待用户在手机上确认。
    Scanned,
    /// 二维码过期，需要重新生成。
    Expired,
    /// 登录成功，`cookies` 与 `refresh_token` 可用于构造凭据。
    Authorized {
        cookies: Vec<(String, String)>,
        refresh_token: String,
    },
    /// 其它返回码，`message` 是 B 站给的文案。
    Failed { code: i32, message: String },
}

impl QrPollState {
    /// 把 B 站的数字码翻译成状态。
    ///
    /// 单独抽成纯函数是为了能在 `cargo test` 里覆盖：这些码只有真机登录时才会遇到，
    /// 但翻译错了会直接表现为"扫了码却没登录上"。
    pub fn from_code(code: i32, message: String, cookies: Vec<(String, String)>, refresh_token: String) -> Self {
        match code {
            0 => Self::Authorized { cookies, refresh_token },
            86101 => Self::Waiting,
            86038 => Self::Expired,
            // 已扫码未确认：B 站的码值文档里没有稳定承诺，按"继续等但提示用户"处理。
            _ if message.contains("扫描") || code == 86090 => Self::Scanned,
            _ => Self::Failed { code, message },
        }
    }
}

/// 从响应 cookie 里凑出凭据。四个必需字段缺任何一个都返回 `None`。
pub fn credential_from_cookies(cookies: &[(String, String)], refresh_token: &str) -> Option<SavedCredential> {
    let pick = |name: &str| {
        cookies
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };

    let credential = SavedCredential {
        dede_user_id: pick("DedeUserID")?,
        sessdata: pick("SESSDATA")?,
        bili_jct: pick("bili_jct")?,
        buvid3: pick("buvid3")?,
        refresh_token: refresh_token.to_string(),
        saved_at_unix: SavedCredential::now_unix(),
    };

    credential.is_complete().then_some(credential)
}

/// 申请一张新的登录二维码。
pub fn fetch_qr(client: Arc<BpiClient>) -> tokio::task::JoinHandle<BpiResult<QrSession>> {
    log::debug!("提交 qr_generate 到 API runtime");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        log::debug!("tokio 任务开始执行 qr_generate");
        let data = client.login().qr_generate().await?;
        log::debug!(
            "qr_generate 返回，用时 {:?}，key 长度 {}",
            started.elapsed(),
            data.qrcode_key.len()
        );
        Ok(QrSession {
            url: data.url,
            qrcode_key: data.qrcode_key,
        })
    })
}

/// 轮询一次扫码状态。
pub fn poll_qr(client: Arc<BpiClient>, qrcode_key: String) -> tokio::task::JoinHandle<BpiResult<QrPollState>> {
    log::debug!("提交 qr_poll 到 API runtime");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let params = bpi_rs::login::LoginQrPollParams::new(qrcode_key)?;
        let data = client.login().qr_poll(params).await?;
        log::debug!(
            "qr_poll 返回 code={} cookie 数={} 用时={:?}",
            data.code,
            data.cookies.len(),
            started.elapsed()
        );
        Ok(QrPollState::from_code(
            data.code,
            data.message,
            data.cookies,
            data.refresh_token,
        ))
    })
}
