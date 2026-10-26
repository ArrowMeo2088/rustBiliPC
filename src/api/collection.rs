//! 合集与系列（P12）：空间页那一格，和点进去之后的一集一行。
//!
//! ## 三条接口与它们的差别（证据都在行号上）
//!
//! | 块 | 调用 | 签名 | 分页口径 |
//! | --- | --- | --- | --- |
//! | 合集+系列列表 | `video().seasons_series_list(VideoCollectionSeasonsSeriesParams)` → `GetSeasonsSeriesData`（`ref/bpi-rs/src/video/client.rs:142-153`） | **库里自动签 WBI**（`get_wbi_sign2`，`:146`），端点 `x/polymer/web-space/seasons_series_list` | 页码 `page_num`/`page_size`，**一次同时给 `seasons_list` 与 `series_list`** |
//! | 合集内容 | `video().seasons_archives_list(VideoCollectionSeasonsArchivesParams)` → `GetSeasonsArchivesData`（`client.rs:113-125`） | 签 WBI（`:117`），端点 `x/polymer/web-space/seasons_archives_list` | 页码 + **`meta.total`（合集内稿件数，权威）** |
//! | 系列内容 | `video().series_archives(VideoCollectionSeriesArchivesParams)` → `GetSeriesArchivesData`（`client.rs:168-177`） | **不签 WBI**（只 `query`，`:172-176`），端点 `x/series/archives` | 页码 + `page.total`；**响应里没有系列名**，标题由调用方从上一页带过来 |
//!
//! ## 三条口径上的坑（都影响界面怎么说这句话）
//!
//! 1. **`Archive` 里没有 `videos`（分 P 数）**（`ref/bpi-rs/src/video/collection/info.rs:26-55`）。
//!    所以合集内容页那一行**不猜"这一集有几个 P"**——想显示它只能在点进那一集之后读它自己的
//!    `view`，或者读详情页那一份 `ugc_season`（那里 `arc.videos` 是有的，见 [`crate::api::detail`]）。
//! 2. **`items_lists.page.total`（合集与系列**列表**那个）语义没有文档**：它到底是"两类条目的总数"
//!    还是"页数"，库里与 fixture 都给不出结论，所以空间页那一格**不显示它**，
//!    只说"已列出 N 个"，翻页判据是"**任何一列给满了就还可能还有下一页**"
//!    （`page_size` 是对两列各自生效的，所以判据不能拿两列之和去比一个 `page_size`）。
//!    两条**内容页**接口的 `page.total` / `meta.total` 按"条目数"用（`seasons_archives_list` 里
//!    它与 `meta.total` 同源），但 [`CollectionArchives::total_known`] 仍拿已加载条数校一次：
//!    **总数比已经拿到的还小，就说明它不是条目数**，这时退回"本页给满了就还能翻"，
//!    界面上也绝不显示那个自相矛盾的数。
//! 3. **`Archive.state` 的取值没有枚举也没有常量**。删稿/审核中的稿件大概会在这里露出来，
//!    但我们不猜数字含义：照常列出来，标题就是接口给的标题。
//!
//! ## 结构严格，坏一次就是整块坏
//!
//! `Archive` 一个 `#[serde(default)]` 都没有（`info.rs:25-55`），`SeasonsMeta`/`SeriesMeta` 同理，
//! 与待看（第 4.20 节）、收藏夹内容（风险表第 23 条）同一类风险：B 站少给任一字段就整次解析失败。
//! 处置也一样——错误只写进本页的提示位，别处照常，日志里有原文。

use std::sync::Arc;

use bpi_rs::{
    BpiClient, BpiResult,
    ids::{Mid, SeasonId},
    video::collection::{
        Archive, VideoCollectionSeasonsArchivesParams, VideoCollectionSeasonsSeriesParams,
        VideoCollectionSeriesArchivesParams,
    },
};

use super::search::format_count;
use super::{format_seconds, seconds_to_u32, spawn_on_api_runtime};

/// 空间页一次取几个合集/系列。接口默认 10，这里写明以免"没给默认值"时两边理解不同。
pub const COLLECTION_PAGE_SIZE: u64 = 10;
/// 合集/系列内容页一页几集。30 与 B 站网页端那一列一致，也和空间页投稿那一路同数量级。
pub const ARCHIVE_PAGE_SIZE: u64 = 30;

/// 这一格是"合集"还是"系列"。两条接口、两种 id，所以必须带类型走完全程。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionKind {
    /// 合集（`season_id`）。
    Season,
    /// 系列（`series_id`）。
    Series,
}

impl CollectionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Season => "合集",
            Self::Series => "系列",
        }
    }
}

/// 空间页"合集与系列"里的一格。
#[derive(Clone, Debug)]
pub struct CollectionBrief {
    pub kind: CollectionKind,
    /// 合集是 `season_id`，系列是 `series_id`。
    pub id: u64,
    pub title: String,
    /// `meta.total`：这一格里有几条稿件。**这个字段语义明确，可以显示**。
    pub total: u64,
    /// 接口顺手带的最近稿件标题（最多取 3 条），界面拿第一条当"这格子里大概是什么"的线索。
    ///
    /// 这里**不留封面与简介**（`meta.cover` / `meta.description` 都有）：这一格排成一行文字，
    /// 两个字段没人读。留着的下场就是"DTO 里躺着一份与界面不一致的旧数据"（第 4.20 节那条老教训）。
    pub samples: Vec<String>,
}

/// 空间页那一格的一页。
#[derive(Clone, Debug, Default)]
pub struct CollectionList {
    pub items: Vec<CollectionBrief>,
    /// 这是第几页（回显用）。
    pub page: u64,
    /// 还能不能再翻一页（口径见文件头第 2 条）。
    pub has_more: bool,
}

/// 合集/系列里的一集。
#[derive(Clone, Debug)]
pub struct CollectionEpisode {
    pub bvid: String,
    pub title: String,
    /// 时长文本（`Archive.duration` 是**数字秒**，与投稿接口直接给 `"mm:ss"` 的 `length` 不同）。
    pub duration_text: String,
    pub play_text: String,
    /// 发布日期（`pubdate` 换算，0 或缺失时是空串）。
    pub date_text: String,
}

/// 合集/系列内容页的一页。
#[derive(Clone, Debug, Default)]
pub struct CollectionArchives {
    /// 这一格的标题：合集取接口的 `meta.name`，系列接口不给，由调用方从上一页带过来。
    pub title: String,
    pub desc: String,
    /// 总条数。`None` = 接口没给（这时"还有下一页"只能按"本页给满了"判）。
    pub total: Option<u64>,
    pub page: u64,
    pub items: Vec<CollectionEpisode>,
}

impl CollectionArchives {
    /// 这个总数**可信不可信**：它比我们已经拿到的条数还小，就说明它不是"条目数"
    /// （B 站这几个 `page.total` 的语义没有文档，见文件头第 2 条），一律当没给。
    ///
    /// 不做这层校验的后果是能画出"共 7 集 · 已加载 30"这种自相矛盾的一行。
    pub fn total_known(&self, loaded: u64) -> Option<u64> {
        self.total.filter(|total| *total >= loaded)
    }

    /// 还能不能再翻一页。
    ///
    /// 有可信的 `total` 就按已加载条数比总数；没有就退回"这一页给满了就还能翻"——
    /// 与评论区那条同口径（多翻一次是空的，比明明还有却把按钮藏起来更难查）。
    pub fn has_more(&self, loaded: u64) -> bool {
        match self.total_known(loaded) {
            Some(total) => loaded < total,
            None => self.items.len() as u64 >= ARCHIVE_PAGE_SIZE,
        }
    }
}

/// 取空间页的"合集与系列"（一次同时给两类，我们按"合集在前、系列在后"合并成一列）。
pub fn fetch_collections(
    client: Arc<BpiClient>,
    mid: u64,
    page: u64,
) -> tokio::task::JoinHandle<BpiResult<CollectionList>> {
    log::debug!("提交 合集与系列 到 API runtime：mid={mid} 第 {page} 页");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let params = VideoCollectionSeasonsSeriesParams::new(Mid::new(mid)?)
            .with_page_num(page)?
            .with_page_size(COLLECTION_PAGE_SIZE)?;
        let data = client.video().seasons_series_list(params).await?;
        let lists = data.items_lists;
        let season_count = lists.seasons_list.len();
        let series_count = lists.series_list.len();

        let mut items: Vec<CollectionBrief> = Vec::with_capacity(season_count + series_count);
        for season in lists.seasons_list {
            items.push(CollectionBrief {
                kind: CollectionKind::Season,
                id: season.meta.season_id,
                title: season.meta.name,
                total: season.meta.total,
                samples: sample_titles(&season.archives),
            });
        }
        for series in lists.series_list {
            items.push(CollectionBrief {
                kind: CollectionKind::Series,
                id: series.meta.series_id,
                title: series.meta.name,
                total: series.meta.total,
                samples: sample_titles(&series.archives),
            });
        }

        log::debug!(
            "合集与系列返回 合集={season_count} 系列={series_count} 用时={:?}",
            started.elapsed()
        );
        Ok(CollectionList {
            // `page_size` 对两列**各自**生效：某一列正好给满，才说明那一类可能还有下一页。
            // 用"两列之和 >= page_size"会在"合集 6 + 系列 5"这种页上误报还能翻
            // （两列都没满，其实是到底了），白点一次"再加载一页"拿到空结果。
            has_more: season_count >= COLLECTION_PAGE_SIZE as usize
                || series_count >= COLLECTION_PAGE_SIZE as usize,
            items,
            page,
        })
    })
}

/// 取合集里的一页稿件。
pub fn fetch_season_archives(
    client: Arc<BpiClient>,
    mid: u64,
    season_id: u64,
    page: u64,
) -> tokio::task::JoinHandle<BpiResult<CollectionArchives>> {
    log::debug!("提交 合集内容 到 API runtime：mid={mid} season_id={season_id} 第 {page} 页");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let params = VideoCollectionSeasonsArchivesParams::new(Mid::new(mid)?, SeasonId::new(season_id)?)
            .with_page_num(page)?
            .with_page_size(ARCHIVE_PAGE_SIZE)?;
        let data = client.video().seasons_archives_list(params).await?;
        let total = data.meta.total.max(data.page.total);
        log::debug!(
            "合集内容返回 {} 条（总数 {total}、本页 {}/{}) 用时={:?}",
            data.archives.len(),
            data.page.page_num,
            data.page.page_size,
            started.elapsed()
        );
        Ok(CollectionArchives {
            title: data.meta.name,
            desc: data.meta.description,
            total: Some(total),
            page,
            items: data.archives.iter().map(episode_of).collect(),
        })
    })
}

/// 取系列里的一页稿件。`title` 是从空间页带过来的系列名（这条接口不给名字）。
pub fn fetch_series_archives(
    client: Arc<BpiClient>,
    mid: u64,
    series_id: u64,
    title: String,
    page: u64,
) -> tokio::task::JoinHandle<BpiResult<CollectionArchives>> {
    log::debug!("提交 系列内容 到 API runtime：mid={mid} series_id={series_id} 第 {page} 页");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let params = VideoCollectionSeriesArchivesParams::new(Mid::new(mid)?, series_id)?
            .with_page_num(page)?
            .with_page_size(ARCHIVE_PAGE_SIZE)?;
        let data = client.video().series_archives(params).await?;
        log::debug!(
            "系列内容返回 {} 条（总数 {}）用时={:?}",
            data.archives.len(),
            data.page.total,
            started.elapsed()
        );
        Ok(CollectionArchives {
            title,
            // 这条接口的响应里没有描述，只有稿件与分页（`GetSeriesArchivesData`，`info.rs:193-200`），
            // 所以这一格留空，界面上那一行直接不画。
            desc: String::new(),
            total: Some(data.page.total),
            page,
            items: data.archives.iter().map(episode_of).collect(),
        })
    })
}

/// `Archive` → 界面上的一集。
fn episode_of(item: &Archive) -> CollectionEpisode {
    CollectionEpisode {
        bvid: item.bvid.clone(),
        title: item.title.clone(),
        duration_text: format_seconds(seconds_to_u32(item.duration)),
        play_text: format_count(item.stat.view),
        date_text: date_text(item.pubdate),
    }
}

/// 最近稿件标题最多取三条（再多这一格就比名字显眼了）。
fn sample_titles(archives: &[Archive]) -> Vec<String> {
    archives
        .iter()
        .take(3)
        .map(|item| item.title.clone())
        .collect()
}

/// Unix 秒 → `YYYY-MM-DD`，**按北京时间**（B 站给的时间戳本来就是 CST 语义，
/// 直接按 UTC 换算会让"凌晨发布的稿件"显示成前一天）。
///
/// 0 一律给空串：`pubdate` 缺失或没意义时不编一个 1970-01-01 出来。
/// 不引 chrono（依赖图里现在没有它，为一个日期格式加时区库不值当），
/// 日期换算是 Howard Hinnant 的 `civil_from_days`，边界进单测。
pub fn date_text(unix: u64) -> String {
    if unix == 0 {
        return String::new();
    }
    const BEIJING_OFFSET: u64 = 8 * 3600;
    let days = (unix + BEIJING_OFFSET) / 86_400;
    // days → 公历（算法里 719468 是 1970-01-01 到 0000-03-01 的天数，400 年一个 146097 天的周期）。
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 + year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    // 算法以 3 月为一年之始，1、2 月要归到上一年的末尾。
    if month <= 2 {
        year += 1;
    }
    format!("{year:04}-{month:02}-{day:02}")
}
