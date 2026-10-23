//! 视频详情（P3）的接口封装。
//!
//! ## 为什么是"四次请求 + 三块容错"而不是一次 `detail()`
//!
//! `video().detail()` 看起来最省事（`View + Tags + Related + Card + Reply`），但它的 `Tags`
//! 用的是 `model::VideoTag`，那个结构的 `tag_id: u64` **既不是 `Option` 也没有 `#[serde(default)]`**
//! （`ref/bpi-rs/src/video/model.rs:100-108`）——B 站给 BGM 类标签时经常省略 `tag_id`，
//! 于是整个 `detail()` 会反序列化失败，连带把标题/封面一起弄没。所以这里拆开：
//!
//! | 块 | 调用 | 失败时 |
//! | --- | --- | --- |
//! | 主体（标题/封面/UP/统计/分P） | `video().view(VideoViewParams)` | 整页失败（没主体就没意义），错误上界面 |
//! | 简介 | `video().desc(VideoDescParams)` | 记 warn，显示空 |
//! | 标签 | `video().tags(VideoTagsParams)`（返回 `tags::VideoTag`，字段是 `Option` 的另一个类型） | 记 warn，标签区不显示 |
//! | 相关推荐 | `video().related_videos(VideoRelatedParams)` | 记 warn，该区显示"没取到" |
//!
//! 四个都不做 WBI 签名（`ref/bpi-rs/src/video/client.rs:65/92/206/243`），cookie 由库自动带上，
//! 都不强制登录。
//!
//! ## 三个字段口径上的坑（都已核实）
//!
//! - **live 的 `VideoView` 没有 `desc`/`pubdate`/`tname`/总时长**（`model.rs:6-26`）。
//!   简介只能另走 `desc()`；总时长用 `pages[].duration`（单位=秒）累加；
//!   发布时间与分区在当前可用类型里**拿不到**——`src/video/info/` 那一整块虽然有这些字段，
//!   但没有任何方法返回它（死代码），不许 import。
//! - `VideoStat` 的收藏数有**两个**字段：`favorite`（新载荷）与 `fav`（别名），都是
//!   `Option<u64>` + `#[serde(default)]`；只读一个会显示 0，所以取先出现的非空者。
//! - `Aid/Cid/Mid/Bvid` 是 newtype：`Aid::new(0)` 报错、`Bvid` 要求 `BV` 前缀且 ≥12 位
//!   ASCII 字母数字（`src/ids.rs:161-183`），所以坏 bvid 会在我们这层就返回
//!   `InvalidParameter`，不必去打网络。

use std::sync::Arc;

use bpi_rs::{
    BpiClient, BpiResult,
    ids::{Bvid, Cid},
    video::{VideoDescParams, VideoPlayerInfoParams, VideoRelatedParams, VideoTagsParams, VideoViewParams},
};

use super::{format_seconds, seconds_to_u32, spawn_on_api_runtime, search::format_count};

/// 一个分 P。
#[derive(Clone, Debug)]
pub struct PageInfo {
    /// 该 P 的内容 ID（播控/播放地址要按 P 取 cid）。
    pub cid: u64,
    /// 从 1 开始的序号。
    pub page: u32,
    /// 分 P 标题。
    pub part: String,
    /// 时长文本 `mm:ss` 或 `h:mm:ss`。
    pub duration_text: String,
}

/// 统计数据，已全部是中文读法的文本。
#[derive(Clone, Debug, Default)]
pub struct StatInfo {
    pub play_text: String,
    pub danmaku_text: String,
    pub reply_text: String,
    pub coin_text: String,
    pub share_text: String,
    pub like_text: String,
    pub favorite_text: String,
}

/// 相关推荐的一条（只留展示与跳转要用的字段）。
#[derive(Clone, Debug)]
pub struct RelatedBrief {
    pub bvid: String,
    pub title: String,
    pub author: String,
    /// 封面（接口原样地址）。
    pub cover: String,
    /// 时长文本：`RelatedVideo.duration` 是**数字秒**，与搜索接口的 `"mm:ss"` 字符串不同。
    pub duration_text: String,
    pub play_text: String,
}

/// 一条字幕。P3 只列出来，P4b-2 才把 `url` 喂给 mpv 的 `sub-add`。
#[derive(Clone, Debug)]
pub struct SubtitleBrief {
    /// 语言显示名（`lan_doc`）。
    pub label: String,
    /// 语言代码（`lan`），用来去重与默认选择。
    pub lan: String,
    /// 字幕 JSON 的地址，**已补成绝对地址**。
    pub url: String,
    /// 是否 AI 生成（B 站的 `ai_status` 非 0）。
    pub ai: bool,
}

/// 详情页需要的全部内容。
#[derive(Clone, Debug, Default)]
pub struct VideoDetailInfo {
    pub bvid: String,
    /// AV 号，播控接口要用（`play_url` 的 aid）。
    pub aid: u64,
    /// 默认分 P 的 cid。
    pub first_cid: u64,
    /// 分 P 总数。
    pub videos: u32,
    pub title: String,
    /// 封面（接口原样地址，显示前交给 `ui::cover::for_display`）。
    pub cover: String,
    pub owner_mid: u64,
    pub owner_name: String,
    /// UP 主头像（同上，显示前要过 `ui::cover`）。
    pub owner_face: String,
    /// 简介；`desc()` 失败时是空字符串。
    pub desc: String,
    pub stat: StatInfo,
    pub pages: Vec<PageInfo>,
    /// 总时长文本，由 `pages` 的秒数累加而来（live 类型没有总时长字段）。
    pub total_duration_text: String,
    pub tags: Vec<String>,
    pub related: Vec<RelatedBrief>,
    /// 简介/标签/推荐这三块各自失败时的说明，主体照常显示。
    pub notes: Vec<String>,
}

/// 收藏数：B 站新旧载荷字段名不同，取先出现的非空者。
pub fn favorite_count(favorite: Option<u64>, fav: Option<u64>) -> u64 {
    favorite.or(fav).unwrap_or_default()
}

/// 把字幕地址补成绝对地址。
///
/// B 站的 `SubtitleItem.subtitle_url` 也是协议相对的 `//comment.bilibili.com/…`
/// （`ref/bpi-rs/src/video/player.rs:103-117`），而 mpv 的 `sub-add` 需要一个完整 URL；
/// 规则与封面一样，但这里不依赖 UI 层，播放层能单独用。
pub fn normalize_subtitle_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        trimmed.to_string()
    }
}

/// 取视频详情（主体失败即整体失败，其余三块降级）。
pub fn fetch_detail(
    client: Arc<BpiClient>,
    bvid: String,
) -> tokio::task::JoinHandle<BpiResult<VideoDetailInfo>> {
    log::debug!("提交 视频详情 到 API runtime：{bvid}");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let parsed = Bvid::new(bvid.clone())?;

        let view = client
            .video()
            .view(VideoViewParams::from_bvid(parsed.clone()))
            .await?;
        log::debug!(
            "view 返回 title={} pages={} 用时={:?}",
            view.title,
            view.pages.len(),
            started.elapsed()
        );

        let mut notes: Vec<String> = Vec::new();

        // 简介：live 的 VideoView 里没有这个字段，必须单独问一次。
        let desc = match client
            .video()
            .desc(VideoDescParams::from_bvid(parsed.clone()))
            .await
        {
            Ok(text) => text,
            Err(error) => {
                log::warn!("取简介失败（详情页照常显示）: {error}");
                notes.push(format!("简介没取到：{error}"));
                String::new()
            }
        };

        // 标签：用 tags()，它的类型是 video::tags::VideoTag（字段是 Option，容错更好）。
        let tags = match client
            .video()
            .tags(VideoTagsParams::from_bvid(parsed.clone()))
            .await
        {
            Ok(list) => list.into_iter().map(|tag| tag.tag_name).collect(),
            Err(error) => {
                log::warn!("取标签失败（详情页照常显示）: {error}");
                notes.push(format!("标签没取到：{error}"));
                Vec::new()
            }
        };

        // 相关推荐：这条接口给的字段比 detail().related 更啰嗦，只留四个。
        let related = match client
            .video()
            .related_videos(VideoRelatedParams::from_bvid(parsed))
            .await
        {
            Ok(list) => list
                .into_iter()
                .map(|item| RelatedBrief {
                    bvid: item.bvid,
                    title: item.title,
                    author: item.owner.name,
                    cover: item.pic,
                    duration_text: format_seconds(seconds_to_u32(item.duration)),
                    play_text: format_count(item.stat.view),
                })
                .collect(),
            Err(error) => {
                log::warn!("取相关推荐失败（详情页照常显示）: {error}");
                notes.push(format!("相关推荐没取到：{error}"));
                Vec::new()
            }
        };

        let total_seconds: u64 = view.pages.iter().map(|page| page.duration).sum();
        let pages: Vec<PageInfo> = view
            .pages
            .iter()
            .map(|page| PageInfo {
                cid: page.cid.get(),
                page: page.page,
                part: page.part.clone(),
                duration_text: format_seconds(seconds_to_u32(page.duration)),
            })
            .collect();

        let stat = &view.stat;
        Ok(VideoDetailInfo {
            bvid,
            aid: view.aid.get(),
            first_cid: view.cid.get(),
            videos: view.videos,
            title: view.title,
            cover: view.pic,
            owner_mid: view.owner.mid.get(),
            owner_name: view.owner.name.clone(),
            owner_face: view.owner.face.clone(),
            desc,
            stat: StatInfo {
                play_text: format_count(stat.view),
                danmaku_text: format_count(stat.danmaku),
                reply_text: format_count(stat.reply),
                coin_text: format_count(stat.coin),
                share_text: format_count(stat.share),
                like_text: format_count(stat.like),
                favorite_text: format_count(favorite_count(stat.favorite, stat.fav)),
            },
            pages,
            total_duration_text: format_seconds(seconds_to_u32(total_seconds)),
            tags,
            related,
            notes,
        })
    })
}

/// 取某一 P 的字幕列表。
///
/// 单独一个请求、单独容错：`player_info_v2` 走的 `PlayerInfoResponseData` 字段极多且
/// **都没有 `#[serde(default)]`**（`ref/bpi-rs/src/video/player.rs:12`），B 站少给任何一个
/// 就整份响应解析失败。字幕在详情页只是"有就列出来"，不该连累主体，所以页面侧失败只记一条说明。
pub fn fetch_subtitles(
    client: Arc<BpiClient>,
    bvid: String,
    cid: u64,
) -> tokio::task::JoinHandle<BpiResult<Vec<SubtitleBrief>>> {
    log::debug!("提交 字幕列表 到 API runtime：{bvid} cid={cid}");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let bvid = Bvid::new(bvid)?;
        let params = VideoPlayerInfoParams::from_bvid(bvid, Cid::new(cid)?);
        let info = client.video().player_info_v2(params).await?;
        let items = info.subtitle.map(|sub| sub.subtitles).unwrap_or_default();
        log::debug!("字幕返回 {} 条，用时 {:?}", items.len(), started.elapsed());

        Ok(items
            .into_iter()
            .map(|item| SubtitleBrief {
                label: item.lan_doc,
                lan: item.lan,
                url: normalize_subtitle_url(&item.subtitle_url),
                ai: item.ai_status != 0,
            })
            .collect())
    })
}
