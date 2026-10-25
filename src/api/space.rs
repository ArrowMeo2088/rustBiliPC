//! 用户空间（P7）的接口封装：资料卡、投稿（带分区筛选）、图文、收藏夹入口。
//!
//! ## 三条接口与它们的差别（证据都在行号上）
//!
//! | 块 | 调用 | 签名 | 分页口径 |
//! | --- | --- | --- | --- |
//! | 资料卡 | `user().card(UserCardParams::new(mid))` → `UserCardProfile`（`ref/bpi-rs/src/user/client.rs:124`） | **不签 WBI**（`client.rs:125-130` 只 `query(..)`） | 无 |
//! | 投稿 | `user().uploaded_videos(UserUploadedVideosParams)` → `UserUploadedVideos`（`client.rs:271-282`） | **库里自动签 WBI**（`sign_wbi_params`，`client.rs:273`），端点 `x/space/wbi/arc/search` | 页码 `pn`/`ps` + 总数 `page.count` |
//! | 图文 | `opus().space_feed(OpusSpaceFeedParams)` → `SpaceData`（`src/opus/client.rs:24-31`） | 不签 WBI，端点 `x/polymer/web-dynamic/v1/opus/feed/space` | **游标**：`offset` + `has_more`（`src/opus/space.rs:44-53`） |
//!
//! 收藏夹那一块**不在这里**：直接复用 [`crate::api::favorites::fetch_folders`]，
//! 它收的是任意 `mid`（`fav().created_list/collected_list` 的 `up_mid` 就是入参），
//! 所以"我的"和"TA 的"是同一份代码，只差界面上那行标签怎么说。
//!
//! ## 投稿的宽容度比详情好，但有一格是 Value
//!
//! `UserUploadedVideo` 除 `aid/bvid/mid/title` 外**几乎全都带 `#[serde(default)]`**
//! （`src/user/model.rs:821-861`），少给字段不会整块失败；`length` 已经是 `"mm:ss"` 字符串，
//! 不要再去格式化数字秒。
//! 唯一别扭的是分区筛选的数据源 `list.tlist` 是 `serde_json::Value`（`model.rs:812-814`）——
//! 这就是界面上那排筛选 chips 的来源，B 站的形状是 `{"17":{"tid":17,"count":3,"name":"单机游戏"}}`。
//! 我们**不假设它一定长这样**：[`parse_tlist`] 只认"`object` 的每个值是带 `name` 的 `object`"，
//! 认不出来就少一个 chip，绝不因为多一个键位整个页面报错。这条也进单测（含畸形输入的用例）。
//!
//! ## 图文条目没有标题，正文就是全部
//!
//! `SpaceItem { content, cover: Option, jump_url, opus_id, stat: { like: String, view: Option } }`
//! （`src/opus/space.rs:29-40`）：接口不给标题、不给发布时间，`stat.like` 已经是**字符串**。
//! 所以那一行只排"封面 + 正文 + 点赞"，别去编一个标题位出来。

use std::sync::Arc;

use bpi_rs::{
    BpiClient, BpiResult,
    ids::Mid,
    opus::{OpusSpaceFeedKind, OpusSpaceFeedParams},
    user::{UserCardParams, UserUploadedVideoOrder, UserUploadedVideosParams},
};
use serde_json::Value;

use super::{search::format_count, spawn_on_api_runtime};

/// 投稿每页条数。`with_page_size` 只拒绝 0（`src/user/params.rs:589-597`），
/// 30 是 bpi-rs 的默认值，也和 B 站网页端空间页一致。
pub const VIDEO_PAGE_SIZE: u32 = 30;

/// 排序口径（页面不碰 bpi 的类型）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoOrder {
    /// 最新发布。
    Latest,
    /// 最多播放。
    MostPlay,
    /// 最多人收藏。
    MostFavorite,
}

impl VideoOrder {
    pub fn label(self) -> &'static str {
        match self {
            Self::Latest => "最新发布",
            Self::MostPlay => "最多播放",
            Self::MostFavorite => "最多人收藏",
        }
    }

    fn to_bpi(self) -> UserUploadedVideoOrder {
        match self {
            Self::Latest => UserUploadedVideoOrder::Pubdate,
            Self::MostPlay => UserUploadedVideoOrder::Click,
            Self::MostFavorite => UserUploadedVideoOrder::Stow,
        }
    }
}

/// 空间页的三块内容。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpaceSection {
    /// 投稿视频。
    Videos,
    /// 图文（opus）。
    Posts,
    /// 收藏夹列表。
    Folders,
}

impl SpaceSection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Videos => "投稿",
            Self::Posts => "图文",
            Self::Folders => "收藏",
        }
    }
}

/// 资料卡（一次 `card()` 给的头像/昵称/签名/数字）。
#[derive(Clone, Debug, Default)]
pub struct SpaceProfile {
    pub mid: u64,
    pub name: String,
    pub face: String,
    pub sign: String,
    /// 粉丝数（已中文化）。
    pub fans_text: String,
    /// 关注数（已中文化）。
    pub follow_text: String,
    /// 投稿数（接口给的是原始数字，界面自己带单位）。
    pub archive_count: u64,
    /// 专栏/图文数。
    pub article_count: u64,
    /// 是否已关注 TA。
    pub followed: bool,
}

/// 一个投稿。字段已经是展示形态。
#[derive(Clone, Debug, Default)]
pub struct SpaceVideo {
    pub bvid: String,
    pub title: String,
    /// 封面（`pic`，显示前还要过 `ui::cover`）。
    pub cover_url: String,
    /// 时长：接口给的就是 `"mm:ss"` 文本，不要再格式化。
    pub duration: String,
    /// 播放量中文读法。
    pub play_text: String,
    /// 弹幕数（`video_review`）。
    pub danmaku_text: String,
    /// 所属分区 ID（chips 选中态与它对齐）。
    pub tid: u64,
}

/// 一条图文（opus）。
#[derive(Clone, Debug, Default)]
pub struct SpacePost {
    /// 图文 id（列表项的稳定 key）。
    pub opus_id: String,
    /// 正文（接口不给标题，正文就是全部）。
    pub text: String,
    /// 封面（`cover.url`，可能没有）。
    pub cover_url: String,
    /// 封面宽高比，画的时候用它定档高（见 [`cover_ratio`]）。
    pub cover_ratio: f32,
    /// 点赞数：接口给的就是**字符串**，原样用。
    pub like_text: String,
}

/// 封面比例（宽除以高）。
///
/// 图文的图**有横有竖**，接口给了 `cover.width/height`，所以必须按它摆——写死 16:9 会把竖图
/// 裁掉半张脸。缺数据（0 或没给封面）时退回 16:9，宁可得比例不对也不画出一条 0 高的缝。
pub fn cover_ratio(width: u32, height: u32) -> f32 {
    if width == 0 || height == 0 {
        return 16. / 9.;
    }
    width as f32 / height as f32
}

/// 一个分区筛选项（`tlist` 里的一格）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoFilter {
    pub tid: u64,
    pub name: String,
    pub count: u64,
}

/// 把 `tlist` 这块 `Value` 折成筛选条目。
///
/// 只认"`{tid: {name, count}}`"这一层结构：键或值不对就跳过那一格。
/// 认不出来时界面少几个 chips，而不是整页报错——这是故意的，
/// B 站这个字段没有类型定义可follow（bpi-rs 给的也是 `Value`）。
pub fn parse_tlist(tlist: &Value) -> Vec<VideoFilter> {
    let Some(map) = tlist.as_object() else {
        return Vec::new();
    };
    let mut out: Vec<VideoFilter> = map
        .iter()
        .filter_map(|(key, value)| {
            let item = value.as_object()?;
            let name = item.get("name")?.as_str()?.to_string();
            if name.is_empty() {
                return None;
            }
            // tid 优先取对象里的 `tid`，退化成键（B 站两处都给，取到一个就够）。
            let tid = item
                .get("tid")
                .and_then(Value::as_u64)
                .or_else(|| key.parse::<u64>().ok())?;
            let count = item.get("count").and_then(Value::as_u64).unwrap_or_default();
            Some(VideoFilter { tid, name, count })
        })
        .collect();
    // 键值对是无序的（serde_json 默认不保序）：按投稿数从多到少排，同数按 tid 排，
    // 这样 chips 的位置不会每次刷新都跳。
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.tid.cmp(&b.tid)));
    out
}

/// 取资料卡。
pub fn fetch_profile(
    client: Arc<BpiClient>,
    mid: u64,
) -> tokio::task::JoinHandle<BpiResult<SpaceProfile>> {
    log::debug!("提交 空间资料卡 到 API runtime：mid={mid}");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let card = client.user().card(UserCardParams::new(Mid::new(mid)?)).await?;
        log::debug!("空间资料卡返回 {:?}，用时 {:?}", card.card.name, started.elapsed());
        Ok(SpaceProfile {
            mid,
            name: card.card.name,
            face: card.card.face,
            sign: card.card.sign,
            fans_text: format_count(card.card.fans),
            follow_text: format_count(card.card.attention),
            archive_count: card.archive_count,
            article_count: card.article_count,
            followed: card.following,
        })
    })
}

/// 取投稿的一页。`tid = 0` 表示不限分区。
pub fn fetch_videos(
    client: Arc<BpiClient>,
    mid: u64,
    page: u32,
    tid: u64,
    order: VideoOrder,
) -> tokio::task::JoinHandle<BpiResult<(Vec<SpaceVideo>, Vec<VideoFilter>, u64)>> {
    log::debug!(
        "提交 空间投稿 到 API runtime：mid={mid} pn={page} tid={tid} order={}",
        order.label()
    );
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let params = UserUploadedVideosParams::new(Mid::new(mid)?)
            .with_order(order.to_bpi())
            .with_tid(tid)
            .with_page(page)?
            .with_page_size(VIDEO_PAGE_SIZE)?;
        let data = client.user().uploaded_videos(params).await?;
        let filters = parse_tlist(&data.list.tlist);
        let total = data.page.count;
        let videos = data
            .list
            .videos
            .into_iter()
            .map(|item| SpaceVideo {
                bvid: item.bvid.as_str().to_string(),
                title: item.title,
                cover_url: item.pic,
                duration: item.length,
                play_text: format_count(item.play),
                danmaku_text: format_count(item.video_review),
                tid: item.typeid,
            })
            .collect();
        log::debug!(
            "空间投稿返回 {} 条（总数 {total}、筛选 {} 个），用时 {:?}",
            videos.len(),
            filters.len(),
            started.elapsed()
        );
        Ok((videos, filters, total))
    })
}

/// 取图文的一页。`offset` 是上一页回传的游标，第一页给 `None`。
pub fn fetch_posts(
    client: Arc<BpiClient>,
    mid: u64,
    page: u32,
    offset: Option<String>,
) -> tokio::task::JoinHandle<BpiResult<(Vec<SpacePost>, bool, String)>> {
    log::debug!("提交 空间图文 到 API runtime：mid={mid} page={page}");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let mut params = OpusSpaceFeedParams::new(Mid::new(mid)?)
            .with_page(page)
            .with_kind(OpusSpaceFeedKind::All);
        if let Some(offset) = offset.filter(|value| !value.is_empty()) {
            params = params.with_offset(offset)?;
        }
        let data = client.opus().space_feed(params).await?;
        let posts = data
            .items
            .into_iter()
            .map(|item| {
                let (cover_url, ratio) = match item.cover {
                    Some(cover) => (cover.url, cover_ratio(cover.width, cover.height)),
                    None => (String::new(), cover_ratio(0, 0)),
                };
                SpacePost {
                    opus_id: item.opus_id,
                    text: item.content,
                    cover_url,
                    cover_ratio: ratio,
                    like_text: item.stat.like,
                }
            })
            .collect();
        log::debug!(
            "空间图文返回 {} 条（has_more={}），用时 {:?}",
            posts.len(),
            data.has_more,
            started.elapsed()
        );
        Ok((posts, data.has_more, data.offset))
    })
}
