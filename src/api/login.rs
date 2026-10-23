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
//! `Account::validate_complete` 要求四个 cookie 全部非空（`src/session/account.rs:110-115`），
//! 但 `qr_poll` 成功响应的 Set-Cookie 里一般只有 `DedeUserID / SESSDATA / bili_jct`
//! （bpi-rs 只是把 `response.cookies()` 原样塞进 `data.cookies`，见 `src/login/client.rs:211-223`），
//! `buvid3` 是设备标识、要靠 `misc().buvid()` 单取。所以 [`poll_qr`] 在 code==0 且缺 buvid3 时
//! 自己去补一个，[`credential_from_cookies`] 仍要求四个字段全部命中。

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
///
/// 成功时额外补一次 `misc().buvid()`：B 站 poll 响应的 Set-Cookie 只给登录三件套
/// （DedeUserID / SESSDATA / bili_jct），而 `Account::validate_complete` 还要求 buvid3，
/// 缺它就构造不出账号、登录会在最后一步失败。buvid3 是设备标识，匿名接口 `/x/frontend/finger/spi`
/// 就能拿到，与登录态无关。
pub fn poll_qr(client: Arc<BpiClient>, qrcode_key: String) -> tokio::task::JoinHandle<BpiResult<QrPollState>> {
    log::debug!("提交 qr_poll 到 API runtime");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let params = bpi_rs::login::LoginQrPollParams::new(qrcode_key)?;
        let mut data = client.login().qr_poll(params).await?;
        log::debug!(
            "qr_poll 返回 code={} cookie=[{}] 用时={:?}",
            data.code,
            data.cookies
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(","),
            started.elapsed()
        );

        if needs_buvid3_backfill(data.code, &data.cookies) {
            match client.misc().buvid().await {
                Ok(buvid) => {
                    log::debug!("qr_poll 成功但响应没有 buvid3，已向 finger/spi 补取一个");
                    data
                        .cookies
                        .push(("buvid3".to_string(), buvid.buvid3));
                }
                // 补不到就让它走到"凭据不完整"的分支：界面会提示，日志里有 finger/spi 的错误。
                Err(error) => log::warn!("向 finger/spi 补取 buvid3 失败: {error}"),
            }
        }

        Ok(QrPollState::from_code(
            data.code,
            data.message,
            data.cookies,
            data.refresh_token,
        ))
    })
}

/// 这次轮询需不需要自己去补一个 buvid3。
///
/// 抽成纯函数是为了在 CI 里锁住这条真机才暴露出来的规则：**成功**（`code == 0`）且响应里
/// **没有** buvid3 才补；其余情况（还在等待扫码、已过期）不要多发一次无意义的请求。
pub fn needs_buvid3_backfill(code: i32, cookies: &[(String, String)]) -> bool {
    code == 0 && !cookies.iter().any(|(name, _)| name == "buvid3")
}
