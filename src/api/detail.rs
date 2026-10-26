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

//! ## 分集（`ugc_season`）为什么要自己发一次 `view`
//!
//! `video().view()` 返回的是 `video::model::VideoView`，它只有
//! `aid/bvid/videos/title/pic/owner/stat/cid/pages` 九个字段（`ref/bpi-rs/src/video/model.rs:6-27`），
//! **没有 `ugc_season`**。带 `ugc_season` 的那份类型在 `src/video/info/view.rs:250`（`UgcSeason`），
//! 但库里没有任何方法返回它——它是死代码。而"这一集属于哪个合集、这个合集还有哪些集、
//! 分段怎么切"全在 `ugc_season` 里，B 站网页端那个分集面板读的就是同一个字段。
//!
//! 所以这里**再发一次同一个端点**，只取 `data.ugc_season` 这一块：
//! `BpiClient::get` 是公开的（`ref/bpi-rs/src/client.rs:367`），它带上库自己那套
//! Referer/UA/cookie，我们不必另装一份 HTTP 栈；响应按 `serde_json::Value` 读，
//! 交给 [`parse_ugc_season`] 宽容解析（**每个字段都可能没有**，缺就退化成"这一集少一个信息"，
//! 而不是整块失败）。这一步失败**只少分集那一节**，主体照常——与简介/标签/推荐同一待遇。
//!
//! 顺手说清两件这路上**不做**的：`data.pubdate`/`data.tname` 也在这份原始响应里
//! （第 4.12 节"发布时间与分区拿不到"那条理由其实已经不成立了），但那是他明确划掉的项，
//! 要加得他先点头；`redirect_url`（番剧/影视那条路）我们不看，普通稿件合集用不到。

use std::sync::Arc;

use bpi_rs::{
    BpiClient, BpiResult,
    ids::{Bvid, Cid},
    video::{VideoDescParams, VideoPlayerInfoParams, VideoRelatedParams, VideoTagsParams, VideoViewParams},
};
use serde_json::Value;

use super::{format_seconds, seconds_to_u32, spawn_on_api_runtime, search::format_count};

/// 我们自己再读一次 `view` 时用的端点（与库里 `VideoViewParams` 走的是同一个地址）。
const VIEW_ENDPOINT: &str = "https://api.bilibili.com/x/web-interface/view";

/// 一集（分集面板里的一行）。
#[derive(Clone, Debug)]
pub struct SeasonEpisodeBrief {
    /// 空 bvid 的条目在解析时就被丢掉了（跳过的条数记在 [`SeasonBrief::skipped`]），
    /// 所以到这里一定有值——界面那一行才敢直接画成可点的。
    pub bvid: String,
    /// 这一集**默认那一 P** 的 cid（我们只用它做去重与日志；点进去还是走详情页重新取主体）。
    pub cid: u64,
    pub title: String,
    pub duration_text: String,
    /// 这一集自己的分 P 数（`arc.videos`）。**大于 1 就是他说的那种复杂情况**：
    /// 分集里的一集本身还是一个多 P 的稿件。1 表示接口没给或就是单 P，界面上不标。
    pub videos: u32,
}

/// 一个分段（`ugc_season.sections[]`）。B 站给的分段标题就是"电动/猞狸"那种。
#[derive(Clone, Debug, Default)]
pub struct SeasonSectionBrief {
    pub id: u64,
    pub title: String,
    pub episodes: Vec<SeasonEpisodeBrief>,
}

/// 当前稿件所在的合集/系列。
#[derive(Clone, Debug, Default)]
pub struct SeasonBrief {
    pub season_id: u64,
    pub title: String,
    /// 接口声明的总集数（`ep_count`）。**0 = 没给**。
    pub ep_count: u64,
    /// 我们实际列出来的集数（跨分段累加）。与 `ep_count` 不等时界面要说这句话。
    pub listed: usize,
    /// 因为拿不到 bvid 而跳过的条数（口径与收藏夹内容那一页一致：说差额，不假装知道是什么）。
    pub skipped: usize,
    /// 当前这一集在扁平列表里的序号（1 起）。0 = 这一集不在这份列表里（接口偶尔不给全）。
    pub current_index: usize,
    pub sections: Vec<SeasonSectionBrief>,
}

impl SeasonBrief {
    /// 有几个分段。0/1 个分段时界面不给分段 chips（没必要，也省一排按钮）。
    pub fn has_sections(&self) -> bool {
        self.sections.len() > 1
    }
}

/// 从 `view` 的原始 `data` 里挑出 `ugc_season` 并转成界面要的形状。
///
/// 三条判据都进单测：① 没有 `ugc_season` 或它不是对象 → `None`（这视频不在合集里，正常情况）；
/// ② `sections` 缺失或不是数组 → `None`（一集都列不出来就别显示这一节）；
/// ③ 单条 episode 缺字段 → 那一行少那一格信息，**不影响整块**。
pub fn parse_ugc_season(data: &Value, current_bvid: &str) -> Option<SeasonBrief> {
    let season = data.get("ugc_season")?.as_object()?;
    let sections_raw = season.get("sections")?.as_array()?;
    let mut brief = SeasonBrief {
        season_id: number_of(season.get("id")).unwrap_or_default(),
        title: text_of(season.get("title")),
        ep_count: number_of(season.get("ep_count")).unwrap_or_default(),
        ..SeasonBrief::default()
    };

    for section in sections_raw {
        let mut one = SeasonSectionBrief {
            id: number_of(section.get("id")).unwrap_or_default(),
            title: text_of(section.get("title")),
            episodes: Vec::new(),
        };
        let episodes = section
            .get("episodes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for episode in episodes {
            let arc = episode.get("arc");
            let bvid = text_of(episode.get("bvid"));
            if bvid.is_empty() {
                // 跳过的条数要看得见：那一行点了没反应比少一行更难查。
                brief.skipped += 1;
                continue;
            }
            let title = {
                let direct = text_of(episode.get("title"));
                if direct.is_empty() {
                    text_of(arc.and_then(|arc| arc.get("title")))
                } else {
                    direct
                }
            };
            let duration = number_of(arc.and_then(|arc| arc.get("duration")))
                .or_else(|| number_of(episode.get("page").and_then(|page| page.get("duration"))))
                .unwrap_or_default();
            let videos = number_of(arc.and_then(|arc| arc.get("videos")))
                .or_else(|| {
                    episode
                        .get("pages")
                        .and_then(Value::as_array)
                        .map(|pages| pages.len() as u64)
                })
                .unwrap_or(1);

            one.episodes.push(SeasonEpisodeBrief {
                bvid,
                cid: number_of(episode.get("cid")).unwrap_or_default(),
                title,
                // 时长缺失与"时长 0 秒"是两件事：前者留空，后者才写 `00:00`
                // （与 `progress_text` 那条同一口径，别显示一个假读数）。
                duration_text: if duration == 0 {
                    String::new()
                } else {
                    format_seconds(seconds_to_u32(duration))
                },
                videos: videos.min(u32::MAX as u64) as u32,
            });
        }
        brief.listed += one.episodes.len();
        brief.sections.push(one);
    }

    if brief.listed == 0 {
        return None;
    }
    // 当前集在扁平列表里的位置：分段内顺序 + 分段之间顺序都按接口给的来，不自己排。
    // 先把 bvid 抄成一份 owned 的序列再找——直接在 `&brief.sections` 的迭代里写
    // `brief.current_index` 会被借用检查拦下（同一时刻既借又写）。
    let order: Vec<String> = brief
        .sections
        .iter()
        .flat_map(|section| section.episodes.iter().map(|episode| episode.bvid.clone()))
        .collect();
    brief.current_index = order
        .iter()
        .position(|bvid| bvid == current_bvid)
        .map(|index| index + 1)
        .unwrap_or(0);
    Some(brief)
}

/// `Value` → `String`：只认字符串，其它一律当没有（不当 `"0"`、不当 `null` 的字面量）。
fn text_of(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        _ => String::new(),
    }
}

/// `Value` → `u64`：B 站这些字段是整数，偶尔给成负数或字符串时一律当没有。
fn number_of(value: Option<&Value>) -> Option<u64> {
    match value {
        Some(Value::Number(number)) => number.as_u64(),
        _ => None,
    }
}

/// 自己发一次 `view`，只要 `data.ugc_season` 那块。
///
/// `Err` 是给界面看的一句话（与简介/标签/推荐那三块同一待遇：失败只写提示位）。
/// bvid 在调用前已经过 `Bvid::new` 校验（BV 前缀 + 12 位字母数字，`src/ids.rs:161-183`），
/// 所以拼进 query 不需要再转义。
async fn fetch_season(client: &BpiClient, bvid: &str) -> Result<Option<SeasonBrief>, String> {
    let url = format!("{VIEW_ENDPOINT}?bvid={bvid}");
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let body: Value = response.json().await.map_err(|error| error.to_string())?;
    let code = body.get("code").and_then(Value::as_i64).unwrap_or(0);
    if code != 0 {
        return Err(format!("接口返回 code={code}"));
    }
    Ok(parse_ugc_season(
        body.get("data").unwrap_or(&Value::Null),
        bvid,
    ))
}

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

/// 一条字幕。现在只列出来，P6 才把 `url` 喂给 mpv 的 `sub-add`。
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
    /// 分集：这一集所在的合集/系列（含分段）。不在合集里、或那一路失败时是 `None`。
    pub season: Option<SeasonBrief>,
    /// 简介/标签/推荐/分集这几块各自失败时的说明，主体照常显示。
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

        // 分集：库里 `VideoView` 没带 `ugc_season`，所以自己再读一次同一个端点（文件头有理由）。
        // 这一步失败只少右栏那一节，主体、评论、字幕都不受牵连。
        let season = match fetch_season(&client, &bvid).await {
            Ok(season) => {
                log::debug!(
                    "分集：{}（{} 集，本次列出 {} 集）",
                    season.as_ref().map(|brief| brief.title.clone()).unwrap_or_default(),
                    season.as_ref().map(|brief| brief.ep_count).unwrap_or_default(),
                    season.as_ref().map(|brief| brief.listed).unwrap_or_default()
                );
                season
            }
            Err(error) => {
                log::warn!("取分集信息失败（详情页照常显示）: {error}");
                notes.push(format!("分集信息没取到：{error}"));
                None
            }
        };

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
            season,
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
