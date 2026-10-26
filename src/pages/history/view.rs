//! 历史页：最近观看的条目，卡片式，点卡片进详情页。
//!
//! ## 只取一页，不做"加载更多"
//!
//! 这条接口是**游标**分页（`HistoryCursor { max, view_at, business, ps }`，
//! `ref/bpi-rs/src/historytoview/history.rs:14-23`），要往回翻得把上一页最后一条的
//! `max`/`view_at` 带回去。这一版只拉第一页 + 一个"重新加载"：
//! `ps` 在库里只拒绝 0（`params.rs:136-143`），30 与 B 站网页端一屏给的数量一致，够用。
//! 真要连续翻历史，得先决定"游标放哪儿、翻回去算不算同一层页"，那是结构问题，不顺手做。
//!
//! ## 卡片的第三行放的是"看到哪儿"
//!
//! `HistoryItem` 不给播放量，给的是 `progress`（上次看到的秒数，`history.rs:94`）与
//! `tag_name`（子分区名）。历史里用户认的是"我看到哪儿了"，所以左格放分区、右格放进度；
//! 进度文案的口径与待看页共用 [`progress_text`](crate::api::progress_text)，两页说法必须一样。

use std::sync::Arc;

use gpui_kit::component::button::Button;
use gpui_kit::*;

use crate::api::history::{self, HistoryItem};
use crate::pages::{loading, OpenVideo};
use crate::ui::video_card::{self, VideoCardData};
use crate::ui::layout;

/// 一次取多少条历史。见文件头那条：这条接口的 `ps` 只拒绝 0。
const HISTORY_LIMIT: u32 = 30;

/// 历史页根视图。
pub struct HistoryPage {
    client: Arc<bpi_rs::BpiClient>,
    items: Vec<HistoryItem>,
    note: SharedString,
    seq: u32,
    loaded: bool,
}

impl HistoryPage {
    /// 建页但不发请求（懒加载）。
    pub fn new(client: Arc<bpi_rs::BpiClient>) -> Self {
        Self {
            client,
            items: Vec::new(),
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

    /// 重新拉一页历史。
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.loaded = true;
        self.seq += 1;
        let seq = self.seq;
        self.note = "历史加载中…".into();
        cx.notify();

        let join = history::fetch_history(Arc::clone(&self.client), HISTORY_LIMIT);
        loading::spawn_into(cx, seq, "观看历史", join, |page| page.seq, |page, outcome| {
            match outcome {
                Ok(list) => {
                    page.note = if list.is_empty() {
                        "还没有观看记录".into()
                    } else {
                        "".into()
                    };
                    page.items = list;
                }
                Err(message) => {
                    page.items.clear();
                    page.note = format!("取历史失败：{message}").into();
                }
            }
        });
    }

    fn reload_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("history-reload")
            .label("重新加载")
            .on_click(cx.listener(|this, _event, _window, cx| this.reload(cx)))
            .into_any_element()
    }
}

/// 点历史卡片 = 请外壳压一层详情页，与搜索/主页/空间页同一个信号。
impl EventEmitter<OpenVideo> for HistoryPage {}

impl Render for HistoryPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = if self.items.is_empty() {
            String::new()
        } else {
            format!("最近 {} 条", self.items.len())
        };
        let note = self.note.clone();
        let cards: Vec<AnyElement> = self
            .items
            .iter()
            .map(|item| {
                let card = VideoCardData {
                    bvid: item.bvid.clone().into(),
                    title: item.title.clone().into(),
                    author: item.author.clone().into(),
                    duration: item.duration_text.clone().into(),
                    play_text: item.tag.clone().into(),
                    cover_url: item.cover.clone().into(),
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
            .collect();

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(layout::header(
                "历史",
                (!count.is_empty()).then(|| count.into()),
                Some(self.reload_button(cx)),
            ))
            .child(layout::divider())
            .child(
                div()
                    .id("history-body")
                    .flex_1()
                    .w_full()
                    .min_w(px(0.))
                    .overflow_y_scroll()
                    .child(
                        // 请求出问题时先把话说清楚，再给一片空网格——只写提示不画列表是
                        // 第 4.7 节定的做法（错误必须看得见）。
                        if note.is_empty() {
                            video_card::grid(cards)
                        } else {
                            div()
                                .flex()
                                .flex_col()
                                .w_full()
                                .child(layout::note(note))
                                .child(video_card::grid(cards))
                                .into_any_element()
                        },
                    ),
            )
    }
}
