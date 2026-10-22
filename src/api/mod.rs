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
//! cx.spawn(async move |_, cx| {
//!     let result = join.await;
//!     cx.update(|_, cx| { /* 写回页面状态 */ }).ok();
//! });
//! ```
//!
//! ## 已知需要自己补的空白
//!
//! - `bpi_rs::session::Account` 只 derive 了 `Deserialize`，没有 `Serialize`，库也不落盘
//!   → 落盘由 [`crate::store::credentials`] 负责。
//! - 视频链路的 `DashInfo` 上**没有** `best_video()`（那个助手在 `models::stream::VideoStreamData`
//!   上，属于番剧/课程响应），所以码流选择见 [`stream_select`]。
//! - bpi-rs 不外发它自己用的请求头 → mpv 需要的 Referer/UA 常量在 [`crate::player`]。

pub mod login;
pub mod search;
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
