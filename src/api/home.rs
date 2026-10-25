//! 主页（B 站首页推荐流）的接口封装。
//!
//! ## 用的是哪条接口
//!
//! `video().homepage_recommendations(VideoHomepageRecommendationsParams)` →
//! `RcmdFeedResponseData`（`ref/bpi-rs/src/video/client.rs:215-225`、
//! `src/video/recommend.rs:165-172`），端点是
//! `https://api.bilibili.com/x/web-interface/wbi/index/top/feed/rcmd`（`recommend.rs:6-7`）。
//! **WBI 签名由库里自己做**（`client.rs:218` 的 `get_wbi_sign2`），我们不需要像搜索那样另取签名；
//! 参数只有 `ps`/`fresh_idx`/`fetch_row`（`src/video/params.rs:350-397`），
//! 没有关键词、也没有"总页数"，所以这一页是"换一批"而不是分页条。
//!
//! ## 载荷是严格结构，缺字段会整批失败
//!
//! `RcmdItem`（`recommend.rs:125-163`）里除了 `business_info`/`rcmd_reason`/`room_info`/`stat`
//! 这几个 `Option`，其余字段**既不是 Option 也没有 `#[serde(default)]`**：
//! B 站少给任意一个（`track_id`、`is_stock`、`preload_expose_pct` 这类最容易变）都会变成
//! `BpiError::parse`，症状是"主页空白 + 一行原因"。这是已知风险表第 12 条那一类，
//! 处置方式也一样：**原因必须显示在界面上**，不许只进日志。
//!
//! ## 必须按 `goto` 过滤
//!
//! 推荐流里混着 `"live"`（直播）与 `"ogv"`（剧集/影吧）项（`recommend.rs:135-136` 的注释写明），
//! 那些项的 `bvid` 不是稿件号：点进详情页必然失败。所以只留 `"av"`，
//! 过滤规则单独做成 [`is_archive_goto`] 并进单测——它是这一页唯一真正会"点错东西"的地方。
//!
//! ## 为什么不做"分区列表"
//!
//! `video_zone_v2.rs` 里有分区类型，Bili.Copilot 的侧栏也确实是"分区 + 该区的内容"。
//! 但那要先决定侧栏怎么摆、点分区取哪条流（`region_newlist`/`region_dynamic` 都在另一个 feature 里），
//! 属于结构问题，等他拍板，不在这次里顺手加。

use std::sync::Arc;

use bpi_rs::{
    BpiClient, BpiResult,
    video::VideoHomepageRecommendationsParams,
    video::recommend::{HomeRmdStat, RcmdItem},
};

use super::{format_seconds, seconds_to_u32, search::format_count, spawn_on_api_runtime};

/// 一次拉多少条。B 站这个接口上限是 30（`params.rs:366-371` 会拒 0 与 >30），
/// 12 条一屏不到、图片也不会一次下太多（第 4.5 节那条"图迟迟不来"的教训）。
pub const HOME_PAGE_SIZE: u8 = 12;

/// 一条主页推荐。字段都已经是可直接展示的最终形态（页面不做数字格式化）。
#[derive(Clone, Debug, Default)]
pub struct HomeHit {
    /// 稿件号（已确认 `goto == "av"`，所以这个 bvid 一定能进详情页）。
    pub bvid: String,
    pub title: String,
    /// UP 主昵称（`owner.name`）。
    pub author: String,
    /// 封面（`pic`，接口原样地址，显示前还要过 `ui::cover`）。
    pub cover_url: String,
    /// 时长文本。**这里的 `duration` 是数字秒**（`recommend.rs:131`），
    /// 与搜索接口直接给 `"mm:ss"` 字符串不同，必须自己格式化。
    pub duration: String,
    /// 播放量的中文读法；`stat` 缺失时是空字符串（**不写 0**，"没给"和"没人看"是两件事）。
    pub play_text: String,
    /// 推荐理由（`rcmd_reason.content`，如"高点赞量""已关注的UP主"）。空则卡片不占那一格。
    pub reason: String,
}

/// 只有稿件（`av`）能进详情页；直播与番剧项的 `bvid` 不是稿件号。
pub fn is_archive_goto(goto: &str) -> bool {
    goto == "av"
}

/// 播放量读法。`stat` 在这个接口里是 `Option`（`recommend.rs:155`），直播项没有它。
pub fn play_text_of(stat: Option<&HomeRmdStat>) -> String {
    stat.map(|stat| format_count(stat.view))
        .unwrap_or_default()
}

/// 把一条推荐项折成展示用的 [`HomeHit`]；不是稿件就返回 `None`。
fn hit_of(item: RcmdItem) -> Option<HomeHit> {
    if !is_archive_goto(&item.goto) {
        return None;
    }
    Some(HomeHit {
        bvid: item.bvid,
        title: item.title,
        author: item.owner.name,
        cover_url: item.pic,
        duration: format_seconds(seconds_to_u32(item.duration)),
        play_text: play_text_of(item.stat.as_ref()),
        reason: item.rcmd_reason.and_then(|reason| reason.content).unwrap_or_default(),
    })
}

/// 取一页首页推荐，返回可以直接铺成卡片的列表。
///
/// 与其余 `fetch_*` 一样：在 API runtime 上跑，返回 `JoinHandle`，由页面 `await` 后回填；
/// `page` 会同时给 `fresh_idx` 与 `fetch_row`（B 站网页端也是两者一起递增，
/// 见 `params.rs:392-393` 把 `brush` 也写成 `fresh_idx`）。
pub fn fetch_home(
    client: Arc<BpiClient>,
    page: u32,
) -> tokio::task::JoinHandle<BpiResult<Vec<HomeHit>>> {
    log::debug!("提交 主页推荐 到 API runtime：第 {page} 批");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let params = VideoHomepageRecommendationsParams::new()
            .page_size(HOME_PAGE_SIZE)?
            .fresh_idx(page)?
            .fetch_row(page)?;
        let data = client.video().homepage_recommendations(params).await?;
        let total = data.item.len();
        let hits = data.item.into_iter().filter_map(hit_of).collect::<Vec<_>>();
        log::debug!(
            "主页推荐返回 {total} 条（过滤后 {} 条是稿件），用时 {:?}",
            hits.len(),
            started.elapsed()
        );
        Ok(hits)
    })
}
