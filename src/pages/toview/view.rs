//! 待看页：B 站的"稍后再看"列表，卡片式，点卡片进详情页。
//!
//! ## 这一页没有翻页，只有"重新加载"
//!
//! 这条接口一次把整个列表给完（零参数，`ref/bpi-rs/src/historytoview/toview.rs:212` 断言过），
//! 所以界面上不该出现分页控件——没有的东西不做一排按钮。
//!
//! ## 页头上那两个数
//!
//! 接口自己报 `count`，我们另外只保留**有 `bvid`** 的条目（下钻只认 bvid，见
//! [`fetch_toview`](crate::api::toview)）。两个数不一致时把差额写在明面上而不是藏起来：
//! 用户看见"共 40 条 · 2 条打不开"才知道不是我们吞了他的条目。
//!
//! ## 严格结构坏掉时的表现
//!
//! `ToViewVideoItem` 那一串嵌套结构全没有 `#[serde(default)]`，B 站少给任一字段就整次解析失败。
//! 症状是这一页写一行原因文本、别处照常（与主页第 4.16 节、风险表第 20 条同一路）。

use std::sync::Arc;

use gpui_kit::component::button::Button;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::api::toview::{self, ToViewItem};
use crate::pages::{loading, OpenVideo};
use crate::ui::video_card::{self, VideoCardData};
use crate::ui::{layout, palette};

/// 待看页根视图。
pub struct ToViewPage {
    client: Arc<bpi_rs::BpiClient>,
    items: Vec<ToViewItem>,
    /// 接口报的总条数（`count`），与 [`items`](Self::items) 的长度的差值要显示给用户看。
    total: u32,
    note: SharedString,
    seq: u32,
    loaded: bool,
}

impl ToViewPage {
    /// 建页但不发请求（懒加载）。
    pub fn new(client: Arc<bpi_rs::BpiClient>) -> Self {
        Self {
            client,
            items: Vec::new(),
            total: 0,
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

    /// 重新拉整个列表。
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.loaded = true;
        self.seq += 1;
        let seq = self.seq;
        self.note = "待看加载中…".into();
        cx.notify();

        let join = toview::fetch_toview(Arc::clone(&self.client));
        loading::spawn_into(cx, seq, "待看列表", join, |page| page.seq, |page, outcome| {
            match outcome {
                Ok(list) => {
                    page.total = list.count;
                    page.note = if list.items.is_empty() {
                        "待看列表是空的".into()
                    } else {
                        "".into()
                    };
                    // 一次给全，所以只留最新这一份，不做累加。
                    page.items = list.items;
                }
                Err(message) => {
                    page.items.clear();
                    page.total = 0;
                    page.note = format!("取待看失败：{message}").into();
                }
            }
        });
    }

    /// 页头副标题：总数 + （有的话）打不开的差额。
    fn summary(&self) -> String {
        if self.items.is_empty() {
            return String::new();
        }
        let skipped = self.total.saturating_sub(self.items.len() as u32);
        if skipped == 0 {
            format!("共 {} 条", self.total)
        } else {
            format!("共 {} 条 · {skipped} 条打不开", self.total)
        }
    }

    fn reload_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("toview-reload")
            .label("重新加载")
            .on_click(cx.listener(|this, _event, _window, cx| this.reload(cx)))
            .into_any_element()
    }

    fn cards(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.items
            .iter()
            .map(|item| {
                let card = VideoCardData {
                    bvid: item.bvid.clone().into(),
                    title: item.title.clone().into(),
                    author: item.author.clone().into(),
                    duration: item.duration.clone().into(),
                    // 第三行左格是子分区、右格是"看到哪儿"，与历史页同一档信息。
                    play_text: item.tname.clone().into(),
                    cover_url: item.cover_url.clone().into(),
                    extra: item.progress_text.clone().into(),
                };
                let bvid = card.bvid.as_str().to_string();
                video_card::clickable(
                    card,
                    cx.listener(move |_this, _event, _window, cx| {
                        cx.emit(OpenVideo { bvid: bvid.clone() });
                    }),
                )
                .into_any_element()
            })
            .collect()
    }
}

/// 点待看卡片 = 请外壳压一层详情页。
impl EventEmitter<OpenVideo> for ToViewPage {}

impl Render for ToViewPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let summary = self.summary();
        let note = self.note.clone();
        let cards = self.cards(cx);

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(layout::header(
                "待看",
                (!summary.is_empty()).then(|| summary.into()),
                Some(self.reload_button(cx)),
            ))
            .child(layout::divider())
            .child(
                div()
                    .id("toview-body")
                    .flex_1()
                    .w_full()
                    .min_w(px(0.))
                    .overflow_y_scroll()
                    .child(
                        // 提示在前、列表在后：解析失败时列表本来就是空的，两件事都不藏。
                        div()
                            .flex()
                            .flex_col()
                            .w_full()
                            .when(!note.is_empty(), |this| {
                                this.child(
                                    div()
                                        .w_full()
                                        .px(px(palette::CARD_TEXT_PADDING_X))
                                        .pt(px(palette::BODY_PADDING_Y))
                                        .text_size(px(palette::FONT_BODY))
                                        .text_color(palette::text_secondary())
                                        .child(note),
                                )
                            })
                            .child(video_card::grid(cards))
                            .into_any_element(),
                    ),
            )
    }
}
