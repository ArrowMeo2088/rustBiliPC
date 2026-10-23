//! API 层：bpi-rs 的调用封装与码流选择。
//!
//! ## 调用约定（P1 起所有页面都按这个模式写）
//!
//! bpi-rs 是 tokio/reqwest 栈，gpui 的执行器不是 tokio，所以**不要**把
//! `client.search(..)` 直接 `cx.spawn`。正确做法：先 [`spawn_on_api_runtime`] 把请求
//! 丢到 [`crate::app::runtime`]，再在 gpui 侧 await 返回的 `JoinHandle`——
//! await `JoinHandle` 只是等一个 waker，不需要 reactor，因此是安全的。
//!
//! ```ignore
//! let join = crate::api::spawn_on_api_runtime(async move { client.search().video(params).await });
//! cx.spawn(async move |this, cx| {
//!     let result = join.await;
//!     if let Some(view) = this.upgrade() {
//!         view.update(cx, |view, cx| { /* 写回页面状态 */ }).ok();
//!     }
//! })
//! // 末尾的 detach() 不能省：`Task` 被 drop 即取消任务（docs/architecture.md 第 4.9 节）。
//! .detach();
//! ```
//!
//! ## 已知需要自己补的空白
//!
//! - `bpi_rs::session::Account` 只 derive 了 `Deserialize`，没有 `Serialize`，库也不落盘
//!   → 落盘由 [`crate::store::credentials`] 负责。
//! - 视频链路的 `DashInfo` 上**没有** `best_video()`（那个助手在 `models::stream::VideoStreamData`
//!   上，属于番剧/课程响应），所以码流选择见 [`stream_select`]。
//! - bpi-rs 不外发它自己用的请求头 → mpv 需要的 Referer/UA 常量在 [`crate::player`]。

pub mod detail;
pub mod favorites;
pub mod history;
pub mod login;
pub mod profile;
pub mod search;
pub mod stream;
pub mod stream_select;

use std::future::Future;

/// 把一个 bpi-rs 请求投递到 API 专用 runtime 上执行。
///
/// 返回的 `JoinHandle` 可以在 gpui 的 async 闭包里安全 await（它不依赖 tokio reactor）。
pub fn spawn_on_api_runtime<F, T>(future: F) -> tokio::task::JoinHandle<T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    crate::app::runtime().spawn(future)
}

/// 秒数 → `mm:ss`，超过一小时变 `h:mm:ss`。
///
/// 放在 api 层是因为**两处以上要用而且单位口径不一样**：历史/详情/收藏给的是数字秒，
/// 而搜索接口直接给 `"mm:ss"` 字符串（那个不需要格式化）。边界（0、正好一小时）容易写错，
/// 所以是纯函数并进单测。
pub fn format_seconds(seconds: u32) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes:02}:{secs:02}")
    }
}

/// 把接口里常见的"数字秒可能是 u64"收窄成 `format_seconds` 要的 u32。
///
/// 一百年以上的时长才会溢出，真出现就当上限截断，不值得为此让整页报错。
pub fn seconds_to_u32(seconds: u64) -> u32 {
    seconds.min(u32::MAX as u64) as u32
}
