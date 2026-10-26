//! 收藏页：我创建的 + 我收藏的 收藏夹列表。
//!
//! ## 这一版只列，不下钻
//!
//! 2026-10-26 用户定稿（"只列收藏夹，不下钻"）。点进去看内容要再压一层页
//! （`fav().list_detail` 已经在接口层封装过，见 [`favorites::fetch_folders`] 同文件），
//! 差的是"那一层页长什么样、在栈里怎么回退"这个结构决定，等他真机看过这一版再定。
//!
//! ## 与"我的"页的差别
//!
//! P2b 时期收藏与历史挤在同一页里当两个 tab；现在它们是侧栏上两个平级入口，
//! 各自一份加载状态（`seq`/`note`），互不影响——所以这一页失败不会连带历史。
//!
//! ## `mid` 从哪来
//!
//! 凭据里的 `DedeUserID`（字符串）。转不成数字时**一路请求都不发**，直接把话写在页面上：
//! 那种情况只有"凭据坏了"一种解释，让用户重新扫码比发三个必失败的请求有用。

use std::sync::Arc;

use gpui_kit::component::button::Button;
use gpui_kit::*;

use crate::api::favorites::{self, FavFolder};
use crate::pages::loading;
use crate::ui::{layout, palette};

/// 收藏页根视图。
pub struct FavoritesPage {
    client: Arc<bpi_rs::BpiClient>,
    /// 凭据里的 mid（字符串形式）。
    mid_raw: String,
    folders: Vec<FavFolder>,
    note: SharedString,
    seq: u32,
    loaded: bool,
}

impl FavoritesPage {
    /// 建页但不发请求：随外壳一起构造，第一次被看到才取（懒加载）。
    pub fn new(client: Arc<bpi_rs::BpiClient>, mid_raw: String) -> Self {
        Self {
            client,
            mid_raw,
            folders: Vec::new(),
            note: "".into(),
            seq: 0,
            loaded: false,
        }
    }

    /// 第一次被看到时取一次。
    pub fn ensure_loaded(&mut self, cx: &mut Context<Self>) {
        if self.loaded {
            return;
        }
        self.reload(cx);
    }

    /// 重新拉收藏夹列表。
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.loaded = true;
        self.seq += 1;
        let seq = self.seq;
        self.note = "收藏夹加载中…".into();
        cx.notify();

        let Ok(mid) = self.mid_raw.trim().parse::<u64>() else {
            log::error!("收藏页：凭据里的 mid 不是数字: {:?}", self.mid_raw);
            self.note = "凭据里的用户 ID 无法解析，请重新扫码登录".into();
            return;
        };
        let join = favorites::fetch_folders(Arc::clone(&self.client), mid);
        loading::spawn_into(cx, seq, "收藏夹", join, |page| page.seq, move |page, outcome| {
            match outcome {
                Ok(list) => {
                    page.note = if list.is_empty() {
                        "还没有公开的收藏夹".into()
                    } else {
                        "".into()
                    };
                    page.folders = list;
                }
                Err(message) => {
                    page.folders.clear();
                    page.note = format!("取收藏夹失败：{message}").into();
                }
            }
        });
    }

    /// 页头右侧那枚"重新加载"。
    fn reload_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("favorites-reload")
            .label("重新加载")
            .on_click(cx.listener(|this, _event, _window, cx| this.reload(cx)))
            .into_any_element()
    }

    fn body(&self) -> AnyElement {
        let mut list = div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(palette::LIST_GAP))
            .px(px(palette::CARD_TEXT_PADDING_X))
            .py(px(palette::BODY_PADDING_Y));
        if !self.note.is_empty() {
            list = list.child(
                div()
                    .w_full()
                    .text_size(px(palette::FONT_BODY))
                    .text_color(palette::text_secondary())
                    .child(self.note.clone()),
            );
        }
        list.children(self.folders.iter().map(|folder| {
            // "TA 创建/TA 收藏"这套说法在空间页用过，这里是自己看自己，直接写"我创建/我收藏"。
            let kind = if folder.created { "我创建" } else { "我收藏" };
            let upper = if folder.upper.is_empty() {
                String::new()
            } else {
                format!(" · {}", folder.upper)
            };
            layout::list_row(
                folder.title.clone(),
                format!("{kind} · {} 个内容{upper}", folder.media_count),
            )
        }))
        .into_any_element()
    }
}

impl Render for FavoritesPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = if self.folders.is_empty() {
            String::new()
        } else {
            format!("{} 个收藏夹", self.folders.len())
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(layout::header(
                "收藏",
                (!count.is_empty()).then(|| count.into()),
                Some(self.reload_button(cx)),
            ))
            .child(layout::divider())
            .child(
                div()
                    .id("favorites-body")
                    .flex_1()
                    .w_full()
                    .min_w(px(0.))
                    .overflow_y_scroll()
                    .child(self.body()),
            )
    }
}
