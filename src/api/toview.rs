//! 待看（B 站叫"稍后再看"）的接口封装。
//!
//! ## 这一条接口与前面几块的形状不一样
//!
//! | 项 | 值 | 证据 |
//! | --- | --- | --- |
//! | 调用 | `historytoview().toview_list()` | `ref/bpi-rs/src/historytoview/client.rs:54` |
//! | 端点 | GET `https://api.bilibili.com/x/v2/history/toview` | `ref/bpi-rs/src/historytoview/client.rs:8` |
//! | 参数 | **一个都没有** | 库里断言过零 query（`ref/bpi-rs/src/historytoview/toview.rs:212`） |
//! | 分页 | 没有：一次把整列表给完（`ToViewListData { count, list }`，`toview.rs:139-144`） | 界面上因此**不做翻页**，只做"重新加载" |
//! | 登录 | 要。走 `client.get(..)` → `apply_default_headers` 注入 COOKIE（`client.rs:367`、`:404-412`），未登录是 `-101`（`src/err/error.rs:310`） |
//! | WBI | **不签**：这条路没碰 `wbi_key_cache` |
//! | feature | `historytoview = []`（空开关，不拉依赖，`ref/bpi-rs/Cargo.toml:60`），**我们的 Cargo.toml 早就开着**（历史用的就是它）⇒ 这一轮 `Cargo.lock` 不动 |
//!
//! ## 严格结构是这块最大的风险
//!
//! `ToViewVideoItem` 及其嵌套的 `ToViewRights`/`ToViewOwner`/`ToViewStat`/`ToViewDimension`
//! 全都没有 `#[serde(default)]`（`ref/bpi-rs/src/historytoview/toview.rs:16-135`），
//! B 站少给任何一个必填字段就是整次 `BpiError::Decode`。
//! 处置与主页（第 4.16 节）、评论区（第 19 条风险）同一套：**错误只砸在待看这一页的提示位**，
//! 别处照常；日志里有原始错误文本，能一眼看出缺哪个字段。真机若稳定复现，
//! 再改成自己解析 `serde_json::Value`（主页 `popular_list` 同法的产物）。
//!
//! ## 只留用得到的字段
//!
//! `aid`/`cid`/`tid`/`videos`/`rights`/`stat` 这些都不进 DTO：点卡片走的是 `bvid`（详情页只认 bvid），
//! 界面上要显示的只有封面、标题、UP 主、时长、子分区与"看到哪儿"。
//! `progress`（`toview.rs:112`，单位秒）就是"看到哪儿"的来源；`add_at` 不给单位也不易读，不做排序依据。

use std::sync::Arc;

use bpi_rs::{BpiClient, BpiResult};

use super::{format_seconds, progress_text, spawn_on_api_runtime};

/// 一条待看。字段已经是展示形态。
#[derive(Clone, Debug, Default)]
pub struct ToViewItem {
    pub bvid: String,
    pub title: String,
    /// 封面（`pic`，显示前要过 `ui::cover`）。
    pub cover_url: String,
    /// UP 主名（`owner.name`）。
    pub author: String,
    /// 总时长，`mm:ss`（接口给的是秒）。
    pub duration: String,
    /// 子分区名（`tname`），与历史页卡片第三行左格同一档信息。
    pub tname: String,
    /// "看到 mm:ss"；没看过是空串（口径见 [`progress_text`](crate::api::progress_text)，与历史同一份）。
    pub progress_text: String,
}

/// 一次拉到的待看列表。
#[derive(Clone, Debug, Default)]
pub struct ToViewList {
    /// 接口自己报的条数（`count`）。列表被过滤过（见 [`fetch_toview`] 里那条）时，
    /// 界面上用这个数说明"总共几条"。
    pub count: u32,
    pub items: Vec<ToViewItem>,
}

/// 取待看列表（一次给全，没有分页）。
pub fn fetch_toview(
    client: Arc<BpiClient>,
) -> tokio::task::JoinHandle<BpiResult<ToViewList>> {
    log::debug!("提交 待看列表 到 API runtime");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let data = client.historytoview().toview_list().await?;
        let count = data.count;
        // 没有 bvid 的条目进不了详情页（我们的下钻只认 bvid），留在列表里只会是一张点不动的卡。
        let items: Vec<ToViewItem> = data
            .list
            .into_iter()
            .filter(|item| !item.bvid.is_empty())
            .map(|item| ToViewItem {
                bvid: item.bvid,
                title: item.title,
                cover_url: item.pic,
                author: item.owner.name,
                duration: format_seconds(item.duration),
                tname: item.tname,
                progress_text: progress_text(item.progress),
            })
            .collect();
        log::debug!(
            "待看列表返回 {count} 条（可用 {} 条），用时 {:?}",
            items.len(),
            started.elapsed()
        );
        Ok(ToViewList {
            count,
            items,
        })
    })
}
