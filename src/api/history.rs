//! 观看历史列表。
//!
//! ## 已核实的接口形状
//!
//! - `historytoview().history_list(HistoryListParams) -> BpiResult<HistoryListData>`
//!   （`src/historytoview/client.rs:36`，走 `/x/web-interface/history/cursor`，**不做 WBI 签名**，
//!   但需要登录 cookie）
//! - 分页是**游标**而不是页码：`HistoryListData.cursor{ max, view_at, business, ps }`
//!   （`src/historytoview/history.rs:14-23`，取法是 `data.cursor`，`:121`），下一页要把**三个字段一起**
//!   回传给 `with_max`(`params.rs:98`，返回 `Self`) / `with_view_at`(`:117`，返回 `Self`) /
//!   `with_raw_business`(`:110`，返回 `BpiResult`：它拒绝空串，所以空值时我们干脆不设这一项)
//!   ——2026-10-26 加"再加载 30 条"就是照这条做的。
//! - **这条接口没有 `keyword`**（`params.rs:84-90` 只有 `max/business/view_at/typ/ps`，
//!   库里也没有任何 `history/search` 端点），所以历史页的搜索是**本地过滤已加载的条目**，
//!   口径见 [`matches_keyword`](crate::pages::matches_keyword)。
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

use super::{format_seconds, progress_text};
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

/// 翻页游标（我们从接口原样搬出来的三样，`ps` 由我们自己给，不回传）。
///
/// 为什么要三个一起：B 站这条是**游标**分页，`max` 是"最后一项的稿件 ID"、
/// `view_at` 是"那一项的观看时间戳"、`business` 是那一项的业务分类（archive/series…），
/// 少给任何一个，下一页的起点就不确定。类型是我们自己的镜像而不是 bpi 的 `HistoryCursor`，
/// 这样页面层不用接触 bpi 类型（分层规则）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryCursor {
    pub max: u64,
    pub view_at: u64,
    pub business: String,
}

/// 一次取历史的结果：条目 + 还能不能往下翻。
#[derive(Clone, Debug, Default)]
pub struct HistoryBatch {
    pub items: Vec<HistoryItem>,
    /// 下一页的起点。空列表时是 `None`（翻到头了，界面据此停掉"再加载"）。
    ///
    /// 这里刻意**不**用"接口给的 cursor 一直在"来判断：那条 cursor 就算 list 为空也会回，
    /// 照它判断会出现"点了再加载、什么也没多、按钮也不消失"的死圈。
    pub next: Option<HistoryCursor>,
}

/// 取一段历史记录（只要稿件视频，`limit` 条）。`after` 是上一页回传的游标，首页给 `None`。
pub fn fetch_history(
    client: Arc<BpiClient>,
    limit: u32,
    after: Option<HistoryCursor>,
) -> tokio::task::JoinHandle<BpiResult<HistoryBatch>> {
    let resumed = after.is_some();
    log::debug!("提交 观看历史 到 API runtime：limit={limit} 续翻={resumed}");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let mut params = HistoryListParams::new()
            .with_type(HistoryListType::Archive)
            .with_page_size(limit)?;
        if let Some(cursor) = after {
            params = params
                .with_max(cursor.max)
                .with_view_at(cursor.view_at);
            if cursor.business.is_empty() {
                // `with_raw_business` 会拒绝空串（`params.rs:110`）：这一项宁可不设，
                // 也不因为一个次要字段把整次翻页判失败；日志留一行，真机出问题能找到。
                log::warn!("历史游标没给 business，这次翻页少带这一项");
            } else {
                params = params.with_raw_business(cursor.business)?;
            }
        }
        let data = client.historytoview().history_list(params).await?;
        log::debug!(
            "历史返回 {} 条（cursor.max={}），用时 {:?}",
            data.list.len(),
            data.cursor.max,
            started.elapsed()
        );

        let items: Vec<HistoryItem> = data
            .list
            .into_iter()
            .map(|item| {
                let progress = progress_text(item.progress);
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
            .collect();
        // 空页 = 翻到头（或 B 站这次什么都没给），不回游标，界面因此停住"再加载"。
        let next = if items.is_empty() {
            None
        } else {
            Some(HistoryCursor {
                max: data.cursor.max,
                view_at: data.cursor.view_at,
                business: data.cursor.business,
            })
        };
        Ok(HistoryBatch { items, next })
    })
}
