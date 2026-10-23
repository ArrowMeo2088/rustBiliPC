//! 个人中心头部信息：昵称、头像、等级、签名、关注/粉丝数。
//!
//! ## 为什么是这两个接口
//!
//! - `user().space_info(UserSpaceParams)` → `UserSpaceProfile`（`src/user/client.rs:251`，
//!   内部自动做 WBI 签名）给 `name / face / sign / level / vip`；
//! - `login().stat()` → `LoginStats`（`src/login/client.rs:107`，feature `login`，
//!   不需要额外开 `web_widget`）给 `following / follower / dynamic_count`。
//!
//! `misc().` 那一组里没有任何用户资料接口：`misc::NavData` 只是两个 WBI key
//! （`src/misc/sign/bili_ticket.rs:31`），别被名字骗了。`login().nav()` 能给出
//! `is_login/mid/uname/face`，但拿不到等级与签名，所以这里走 `space_info`。
//!
//! ## 容错口径
//!
//! `space_info` 失败 = 整个头部失败（界面直接显示原因，规则见第 4.7 节）；
//! `stat` 失败只记日志、计数显示 0——头像和昵称还在，页面不该因为一个计数接口整个空白。

use std::sync::Arc;

use bpi_rs::{BpiClient, BpiResult, ids::Mid, user::UserSpaceParams};

use super::spawn_on_api_runtime;

/// 头部要展示的信息，全部是可直接渲染的最终形态。
#[derive(Clone, Debug, Default)]
pub struct ProfileSummary {
    /// 用户 mid（已从 bpi 的 `Mid` 拍平成字符串，页面不接触 bpi 类型）。
    pub mid: String,
    /// 昵称。
    pub name: String,
    /// 头像 URL（交给 gpui 的 URI 图片加载）。
    pub face: String,
    /// 个性签名。
    pub sign: String,
    /// 账号等级。
    pub level: u8,
    /// 会员文案；非会员为空字符串。
    pub vip_label: String,
    /// 关注数。
    pub following: u64,
    /// 粉丝数。
    pub follower: u64,
}

/// VIP 文案。
///
/// 只认"已开通"这一件事：`vip.status == 0` 或 `kind == 0` 都算非会员。
/// B 站的 `kind` 具体档位（月度/年度/十年）在不同时期含义有变，宁可少说也不说错。
pub fn vip_label(status: i32, kind: i32) -> String {
    if status == 0 || kind == 0 {
        String::new()
    } else {
        "大会员".to_string()
    }
}

/// 取当前登录账号的头部信息。
pub fn fetch_profile(
    client: Arc<BpiClient>,
    mid: u64,
) -> tokio::task::JoinHandle<BpiResult<ProfileSummary>> {
    log::debug!("提交 个人资料 到 API runtime：mid={mid}");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let mid = Mid::new(mid)?;
        let profile = client.user().space_info(UserSpaceParams::new(mid)).await?;
        log::debug!(
            "space_info 返回 name={} level={} 用时={:?}",
            profile.name,
            profile.level,
            started.elapsed()
        );

        // 计数失败不影响头部主体：记一条 warn，界面留 0。
        let (following, follower) = match client.login().stat().await {
            Ok(stat) => (stat.following, stat.follower),
            Err(error) => {
                log::warn!("取关注/粉丝计数失败，头部按 0 显示: {error}");
                (0, 0)
            }
        };

        let (vip_status, vip_kind) = match profile.vip.as_ref() {
            Some(vip) => (vip.status, vip.kind),
            None => (0, 0),
        };

        Ok(ProfileSummary {
            mid: profile.mid.to_string(),
            name: profile.name,
            face: profile.face,
            sign: profile.sign,
            level: profile.level,
            vip_label: vip_label(vip_status, vip_kind),
            following,
            follower,
        })
    })
}
