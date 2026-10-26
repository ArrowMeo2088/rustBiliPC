//! 收藏夹**内容**页：点收藏列表里的一行展开到这一层（P9）。
//!
//! ## 三个能力各走各的路子
//!
//! | 能力 | 走哪条 | 为什么 |
//! | --- | --- | --- |
//! | 展开 | 外壳把 `FavFolderPage` 压进下钻栈（[`OpenFolder`](crate::pages::OpenFolder)） | 用户定稿"下钻一层页"，与详情/空间同一条"返回"路径 |
//! | 搜索 | **服务端**：`fav().list_detail` 有 `keyword`（`ref/bpi-rs/src/fav/list.rs:140`、`:196`） | 夹子可能上百条，只有接口给得起关键词；这条也是历史/待看不有的（那两页只能本地过滤） |
//! | 进视频 | 卡片发 [`OpenVideo`](crate::pages::OpenVideo)，再压一层详情页 | 与主页/搜索/历史/待看同一个信号 |
//!
//! ## 分页是页码 + `has_more`
//!
//! `ps`/`pn`（`list.rs:162`、`:174`，两者只拒绝 0），所以底部一个"加载更多"按钮，
//! `page+1` 把新一页**累加**在旧条目后面（换关键词时回第 1 页并清空，不能累加）。
//!
//! ## 页头那两个数
//!
//! `info.media_count` 是夹子**总数**，本页长度是已经拿到的，`skipped` 是这一页里
//! **拿不到稿件号**被跳过的行（专栏/番剧，或者 B 站改了键名）。三个数分开写，
//! 因为"我收藏夹 400 个视频，怎么只有 30 条"和"有 3 条我打不开"是两种不同的疑问。
//!
//! ## 严格结构的风险
//!
//! `FavListDetailData` 的 `info`/`has_more`/`ttl` 与 `FavListMedia` 的十几个字段都没有
//! `#[serde(default)]`（整个 fav 模块只有 `medias` 有）。坏的时候只有这一页显示原因文本，
//! 别处照常（风险表第 23 条）。真机若稳定复现，改成自己解析 `serde_json::Value`（主页同法）。

use std::sync::Arc;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use crate::api::favorites::{self, FavItem};
use crate::pages::{loading, OpenVideo};
use crate::ui::video_card::{self, VideoCardData};
use crate::ui::{layout, palette};

/// 收藏夹内容页根视图。
pub struct FavFolderPage {
    client: Arc<bpi_rs::BpiClient>,
    /// 接口那边叫 `media_id`。
    media_id: u64,
    /// 页头标题：先用列表带过来的名字，回来后以接口的 `info.title` 为准。
    title: SharedString,
    input: Entity<InputState>,
    /// **已经提交**的关键词。与输入框里的文本分开：这条是服务端搜索，
    /// 没按回车之前不许改变列表（否则每敲一个字打一次接口）。
    submitted: SharedString,
    items: Vec<FavItem>,
    total: u32,
    page: u32,
    has_more: bool,
    skipped: u32,
    note: SharedString,
    seq: u32,
    loaded: bool,
    _subscriptions: Vec<Subscription>,
}

impl FavFolderPage {
    /// 建页。要 `window`：`InputState::new` 与 `subscribe_in` 都要它。
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        client: Arc<bpi_rs::BpiClient>,
        media_id: u64,
        title: String,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("在这个收藏夹里搜索，回车提交")
        });
        let subscriptions = vec![cx.subscribe_in(
            &input,
            window,
            |this, state, event: &InputEvent, _window, cx| {
                // 只认回车：服务端搜索不能跟着 `Change` 跑（见 `submitted` 那条注释）。
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let keyword = state.read(cx).value().trim().to_string();
                    this.submit(keyword, cx);
                }
            },
        )];

        Self {
            client,
            media_id,
            title: title.into(),
            input,
            submitted: "".into(),
            items: Vec::new(),
            total: 0,
            page: 0,
            has_more: false,
            skipped: 0,
            note: "".into(),
            seq: 0,
            loaded: false,
            _subscriptions: subscriptions,
        }
    }

    /// 第一次被看到时取第 1 页（这一页是点进来的，不该先给一片空白）。
    pub fn ensure_loaded(&mut self, cx: &mut Context<Self>) {
        if self.loaded {
            return;
        }
        self.request(1, cx);
    }

    /// 提交一个关键词：回到第 1 页、清空已加载的条目。
    fn submit(&mut self, keyword: String, cx: &mut Context<Self>) {
        self.submitted = keyword.into();
        self.items.clear();
        self.total = 0;
        self.page = 0;
        self.has_more = false;
        self.skipped = 0;
        self.request(1, cx);
    }

    /// 取某一页。`page == 1` 是替换，大于 1 是往后**累加**。
    fn request(&mut self, page: u32, cx: &mut Context<Self>) {
        self.loaded = true;
        self.seq += 1;
        let seq = self.seq;
        self.note = if page == 1 {
            "加载中…".into()
        } else {
            "正在加载下一页…".into()
        };
        cx.notify();

        let join = favorites::fetch_items(
            Arc::clone(&self.client),
            self.media_id,
            page,
            self.submitted.as_str(),
        );
        loading::spawn_into(
            cx,
            seq,
            "收藏夹内容",
            join,
            |page_state| page_state.seq,
            // `move`：闭包要跨 await 存活（'static），所以 `page` 是搬进来而不是借。
            move |page_state, outcome| match outcome {
                Ok(batch) => {
                    // 名字以接口为准：列表里那个可能是改名前抓的。
                    if !batch.title.is_empty() {
                        page_state.title = batch.title.into();
                    }
                    page_state.total = batch.total;
                    page_state.skipped = batch.skipped;
                    page_state.has_more = batch.has_more;
                    page_state.page = batch.page;
                    if batch.page == 1 {
                        page_state.items = batch.items;
                    } else {
                        page_state.items.extend(batch.items);
                    }
                    page_state.note = if page_state.items.is_empty() {
                        if page_state.submitted.is_empty() {
                            "这个收藏夹还没有能播的内容".into()
                        } else {
                            "这个关键词在夹子里没搜到东西".into()
                        }
                    } else {
                        "".into()
                    };
                }
                Err(message) => {
                    // 只有第一页失败才清空：翻第 N 页失败时，已经看到的第 1..N-1 页
                    // 仍然有效，跟着一起清等于把用户的浏览记录砸了（历史页同一条口径）。
                    if page == 1 {
                        page_state.items.clear();
                        page_state.total = 0;
                        page_state.skipped = 0;
                    }
                    page_state.note = format!("取收藏夹内容失败：{message}").into();
                }
            },
        );
    }

    /// 页头副标题：总数 / 已加载 / 打不开的那几行。
    fn summary(&self) -> String {
        if self.items.is_empty() {
            return String::new();
        }
        let mut text = format!("共 {} 条 · 已加载 {}", self.total, self.items.len());
        if self.skipped > 0 {
            text.push_str(&format!(" · 另有 {} 条打不开", self.skipped));
        }
        if !self.submitted.is_empty() {
            text = format!("搜索「{}」· {}", self.submitted.as_str(), text);
        }
        text
    }

    fn search_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("fav-folder-search")
            .label("搜索")
            .on_click(cx.listener(|this, _event, _window, cx| {
                let keyword = this.input.read(cx).value().trim().to_string();
                this.submit(keyword, cx);
            }))
            .into_any_element()
    }

    fn load_more_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.has_more {
            return None;
        }
        let next = self.page + 1;
        Some(
            div()
                .id("fav-folder-more")
                .flex()
                .w_full()
                .justify_center()
                .py(px(palette::CARD_TEXT_ROW_GAP))
                .child(
                    Button::new("fav-folder-more-btn")
                        .label("加载更多")
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.request(next, cx);
                        })),
                )
                .into_any_element(),
        )
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
                    play_text: item.play_text.clone().into(),
                    cover_url: item.cover_url.clone().into(),
                    // 右格放**用户自己写的备注**：这是收藏夹里唯一属于他本人的信息，
                    // 比再摆一个时间戳有用（备注为空时那一格自己不显示）。
                    extra: item.note.clone().into(),
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

/// 点内容卡片 = 请外壳压一层详情页。
impl EventEmitter<OpenVideo> for FavFolderPage {}

impl Render for FavFolderPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let summary = self.summary();
        let title = self.title.clone();
        let note = self.note.clone();
        let cards = self.cards(cx);
        let more = self.load_more_button(cx);

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(layout::header(
                title,
                (!summary.is_empty()).then(|| summary.into()),
                Some(self.search_button(cx)),
            ))
            .child(layout::search_box(&self.input))
            .child(layout::divider())
            .child(
                div()
                    .id("fav-folder-body")
                    .flex_1()
                    .w_full()
                    .min_w(px(0.))
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w_full()
                            .child(if note.is_empty() {
                                div().into_any_element()
                            } else {
                                layout::note(note)
                            })
                            .child(video_card::grid(cards))
                            .children(more),
                    ),
            )
    }
}
