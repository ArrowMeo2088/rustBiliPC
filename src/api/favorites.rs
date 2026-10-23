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
    fav::{FavCollectedListParams, FavCreatedListParams},
    ids::Mid,
};

use super::spawn_on_api_runtime;

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
