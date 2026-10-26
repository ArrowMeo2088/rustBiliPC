//! 收藏夹列表：我创建的 + 我收藏的。
//!
//! ## 已核实的接口形状
//!
//! - `fav().created_list(FavCreatedListParams::new(Mid)) -> BpiResult<CreatedFolderListData>`
//!   （`src/fav/client.rs:69`、`src/fav/params.rs:30`，构造不校验、不会失败）
//! - `fav().collected_list(FavCollectedListParams::new(Mid)) -> BpiResult<CollectedFolderListData>`
//!   （`src/fav/client.rs:81`、`src/fav/params.rs:79`）
//! - 两个端点都**要求调用方显式传自己的 mid**，bpi-rs 没有"mine"变体，所以 mid 由页面传进来
//!   （来源是凭据里的 `DedeUserID`）。
//!
//! ## 收藏夹**内容**（点进去那一层）的证据
//!
//! - `fav().list_detail(FavListDetailParams) -> BpiResult<FavListDetailData>`
//!   （`ref/bpi-rs/src/fav/client.rs:105`，GET `x/v3/fav/resource/list`，`:15`）
//! - 参数在 `src/fav/list.rs:109-210`：`media_id`（**是 `MediaId`，不要 `Mid`**）、
//!   `keyword`（`:140`/`:196`，**服务端搜索在这里，历史和待看都没有这个能力**）、
//!   `ps`/`pn`（`:162`/`:174`，两者都只拒绝 0）、`tid`/`order`/`content_type` 我们不用。
//! - 返回 `FavListDetailData { info, medias, has_more, ttl }`（`:86-93`），
//!   整个 fav 模块**只有 `medias` 带 `#[serde(default)]`**（null→空表，`:89`），
//!   `info`/`has_more`/`ttl` 与条目里十几个字段都是必填 ⇒ 严格结构与待看同一风险档
//!   （风险表第 23 条）。
//! - 分页是**页码**（`pn`/`ps` + `has_more`），不是游标，所以"加载更多"直接 `page+1` 累加。
//!
//! ## 条目类型没有可信的取值表
//!
//! `FavListMedia.type_name`（JSON 里的 `type`，`:66-67`）是 `u8`，但库里**没有**这个字段的
//! 取值文档或常量（`src/fav/list.rs:437` 那个 `"type": 2` 是**夹子 info** 的，不是条目）。
//! 所以这里不猜"2=视频"，改成看**拿不拿得到稿件号**：
//! `bvid`/`bv_id` 两个键都空就是点不开的行（专栏、番剧、或 B 站哪天换写法），
//! 一律收进"打不开"的差额里，界面上如实写"另有 M 条打不开"，不假装我们知道它是什么类型。
//!
//! ## 为什么不合并成一个列表
//!
//! `CreatedFolderItem`（`src/fav/info.rs:52`）没有封面和 UP 主，`CollectedFolderItem`
//! （`src/fav/info.rs:81`）有。两类在 B 站本身就是两个接口、两种语义（自己的夹子 vs 别人分享的夹子），
//! 这里用 `created` 标记区分，界面分两段展示，不去假装它们是同一种东西。
//!
//! ## 解析风险（已知）
//!
//! 这两个响应结构都没有 `#[serde(default)]`，B 站少给任何一个字段都会变成 `BpiError::parse`。
//! 处置口径与第 4.7 节一致：错误必须显示到界面上，同时日志里有原文。

use std::sync::Arc;

use bpi_rs::{
    BpiClient, BpiResult,
    fav::{FavCollectedListParams, FavCreatedListParams, FavListDetailParams},
    ids::{MediaId, Mid},
};

use super::spawn_on_api_runtime;
use super::{format_seconds, search::format_count, search::strip_keyword_tags};

/// 一个收藏夹。
#[derive(Clone, Debug)]
pub struct FavFolder {
    /// 收藏夹 ID（`list_detail` 的 `media_id` 入参）。
    pub id: u64,
    /// 标题。
    pub title: String,
    /// 内容条数。
    pub media_count: u32,
    /// 封面 URL；"我创建的"接口不给封面，这里为空字符串。
    pub cover: String,
    /// 创建者昵称；"我创建的"接口同样不给，这里为空字符串。
    pub upper: String,
    /// true = 我创建的，false = 我收藏的。
    pub created: bool,
}

/// 取当前账号的收藏夹（创建的 + 收藏的，两段按顺序拼在一起）。
pub fn fetch_folders(
    client: Arc<BpiClient>,
    mid: u64,
) -> tokio::task::JoinHandle<BpiResult<Vec<FavFolder>>> {
    log::debug!("提交 收藏夹列表 到 API runtime：mid={mid}");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let mid = Mid::new(mid)?;

        let created = client.fav().created_list(FavCreatedListParams::new(mid)).await?;
        let collected = client
            .fav()
            .collected_list(FavCollectedListParams::new(mid))
            .await?;
        log::debug!(
            "收藏夹返回 创建={} 收藏={} 用时={:?}",
            created.list.len(),
            collected.list.len(),
            started.elapsed()
        );

        let mut folders: Vec<FavFolder> = created
            .list
            .into_iter()
            .map(|item| FavFolder {
                id: item.id,
                title: item.title,
                media_count: item.media_count,
                cover: String::new(),
                upper: String::new(),
                created: true,
            })
            .collect();

        folders.extend(collected.list.into_iter().map(|item| FavFolder {
            id: item.id,
            title: item.title,
            media_count: item.media_count,
            cover: item.cover,
            upper: item.upper.name,
            created: false,
        }));

        Ok(folders)
    })
}

/// 收藏夹内容一页取多少条。库里 `page_size` 只拒绝 0（`src/fav/list.rs:162`），
/// 30 与历史页那一档保持一致，界面"加载更多"的节奏也统一。
pub const FOLDER_PAGE_SIZE: u32 = 30;

/// 收藏夹里的一条可播内容。
#[derive(Clone, Debug, Default)]
pub struct FavItem {
    pub bvid: String,
    pub title: String,
    /// 封面（`cover`，显示前要过 `ui::cover`）。
    pub cover_url: String,
    /// UP 主名（`upper.name`）。
    pub author: String,
    /// 时长 `mm:ss`（接口的 `duration` 是**秒**）。
    pub duration: String,
    /// 播放量文案：优先用接口给的 `cnt_info.view_text_1`（已经是"12.3万"这种读法），
    /// 没有就把 `cnt_info.play` 自己格式化，两条路都比写 0 诚实。
    pub play_text: String,
    /// 用户自己写的备注（`intro`），没有是空串。
    pub note: String,
}

/// 一个收藏夹的内容（一页 + 它周边的口径）。
#[derive(Clone, Debug, Default)]
pub struct FavItemList {
    /// 夹子名，**以接口给的 `info.title` 为准**（列表里带过来的那个可能是改名前的）。
    pub title: String,
    /// 夹子总条数（`info.media_count`，不是本页长度）。
    pub total: u32,
    pub items: Vec<FavItem>,
    /// 本页被跳过的行数（拿不到稿件号），见文件头那条。
    pub skipped: u32,
    pub has_more: bool,
    /// 这是第几页（`pn` 从 1 起）。
    pub page: u32,
}

/// 从 `bvid` / `bv_id` 两种写法里挑一个稿件号。
///
/// B 站这个字段历史上换过名（库里两个键都留着，`src/fav/list.rs:80-81`），
/// 所以按"`bvid` 优先、`bv_id` 兜底、都空就是空串"来取；空串的行不进列表。
/// 做成纯函数是为了进单测（三种键位组合都要钉住）。
pub fn pick_bvid(bvid: Option<String>, bv_id: Option<String>) -> String {
    bvid.filter(|value| !value.is_empty())
        .or_else(|| bv_id.filter(|value| !value.is_empty()))
        .unwrap_or_default()
}

/// 取收藏夹的一页内容。`keyword` 非空时走**服务端搜索**（这条接口给得起，
/// 历史与待看给不起，所以那两页是本地过滤）。
pub fn fetch_items(
    client: Arc<BpiClient>,
    media_id: u64,
    page: u32,
    keyword: &str,
) -> tokio::task::JoinHandle<BpiResult<FavItemList>> {
    let keyword = keyword.trim().to_string();
    log::debug!(
        "提交 收藏夹内容 到 API runtime：media_id={media_id} pn={page} keyword={:?}",
        if keyword.is_empty() { "" } else { &keyword }
    );
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let mut params = FavListDetailParams::new(MediaId::new(media_id)?)
            .page(page)?
            .page_size(FOLDER_PAGE_SIZE)?;
        if !keyword.is_empty() {
            // 库里 `keyword` 会拒绝空白串（`src/fav/list.rs:140`），所以只在非空时才设。
            params = params.keyword(keyword.clone())?;
        }
        let data = client.fav().list_detail(params).await?;
        let total = data.info.media_count;
        let mut skipped = 0u32;
        let mut items: Vec<FavItem> = Vec::new();
        for media in data.medias {
            let bvid = pick_bvid(media.bvid, media.bv_id);
            if bvid.is_empty() {
                skipped += 1;
                continue;
            }
            // 服务端搜索回来的标题带 `<em class="keyword">…</em>` 高亮标签，
            // 与搜索结果同一份清洗（`api/search.rs`）。
            let play_text = media
                .cnt_info
                .view_text_1
                .clone()
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| format_count(media.cnt_info.play));
            items.push(FavItem {
                bvid,
                title: strip_keyword_tags(&media.title),
                cover_url: media.cover,
                author: media.upper.name,
                duration: format_seconds(media.duration),
                play_text,
                note: media.intro,
            });
        }
        log::debug!(
            "收藏夹内容返回 {} 条（跳过 {skipped}、总数 {total}、has_more={}），用时 {:?}",
            items.len(),
            data.has_more,
            started.elapsed()
        );
        Ok(FavItemList {
            title: data.info.title,
            total,
            items,
            skipped,
            has_more: data.has_more,
            page,
        })
    })
}
