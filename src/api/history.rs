//! 观看历史列表。
//!
//! ## 已核实的接口形状
//!
//! - `historytoview().history_list(HistoryListParams) -> BpiResult<HistoryListData>`
//!   （`src/historytoview/client.rs:36`，走 `/x/web-interface/history/cursor`，**不做 WBI 签名**，
//!   但需要登录 cookie）
//! - 分页是**游标**而不是页码：`HistoryListData.cursor{ max, view_at, business, ps }`
//!   （`src/historytoview/history.rs:14`），下一页要把它回传给
//!   `HistoryListParams::with_max / with_view_at / with_business`。
//! - `bvid` 不在条目顶层，在 `item.history.bvid: Option<String>`（`history.rs:47`）；
//!   专栏/直播条目那里是 `None`，所以用 `HistoryListType::Archive` 只要稿件，
//!   这样列表里每一项都能直接进详情页（P3）。
//!
//! ## 时长与进度
//!
//! 接口给的都是**数字秒**（`duration: Option<u32>`、`progress: i32`，`history.rs:97/101`），
//! 没有 `"12:34"` 这种字符串字段（搜索接口给的是字符串，两边别混）。
//! `progress` 在"没看过/看完"时可能是 `-1` 或 `0`，所以只在 `> 0` 时显示。
//!
//! `view_at` 也是秒级时间戳，但本项目没有日期库（也不想为一条次要文案引入 chrono），
//! 现阶段不显示观看时间；要做的时候优先加依赖而不是手写日历换算。

use std::sync::Arc;

use bpi_rs::{BpiClient, BpiResult, historytoview::{HistoryListParams, HistoryListType}};

use super::spawn_on_api_runtime;

/// 一条历史记录。字段已处理好可直接展示。
#[derive(Clone, Debug)]
pub struct HistoryItem {
    /// 稿件号；理论上 `Archive` 类型必有，缺失时为空字符串（界面不显示跳转）。
    pub bvid: String,
    /// 标题。
    pub title: String,
    /// UP 主昵称。
    pub author: String,
    /// 封面 URL。
    pub cover: String,
    /// 总时长文本，如 `12:34` / `1:02:03`。
    pub duration_text: String,
    /// 上次看到的位置文本，如 `看到 03:21`；没进度时为空字符串。
    pub progress_text: String,
    /// 子分区名（B 站的 `tag_name`），可为空。
    pub tag: String,
}

/// 秒数 → `mm:ss` 或 `h:mm:ss`。
///
/// 单独成函数是因为历史记录、收藏内容、播放进度都要用它，且边界（0、正好一小时）容易写错。
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

/// 取最近的历史记录（只要稿件视频，`limit` 条）。
pub fn fetch_history(
    client: Arc<BpiClient>,
    limit: u32,
) -> tokio::task::JoinHandle<BpiResult<Vec<HistoryItem>>> {
    log::debug!("提交 观看历史 到 API runtime：limit={limit}");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let params = HistoryListParams::new()
            .with_type(HistoryListType::Archive)
            .with_page_size(limit)?;
        let data = client.historytoview().history_list(params).await?;
        log::debug!(
            "历史返回 {} 条，用时 {:?}",
            data.list.len(),
            started.elapsed()
        );

        Ok(data
            .list
            .into_iter()
            .map(|item| {
                let progress = if item.progress > 0 {
                    format!("看到 {}", format_seconds(item.progress.max(0) as u32))
                } else {
                    String::new()
                };
                HistoryItem {
                    bvid: item.history.bvid.unwrap_or_default(),
                    title: item.title,
                    author: item.author_name.unwrap_or_default(),
                    cover: item.cover.unwrap_or_default(),
                    duration_text: item.duration.map(format_seconds).unwrap_or_default(),
                    progress_text: progress,
                    tag: item.tag_name.unwrap_or_default(),
                }
            })
            .collect())
    })
}
