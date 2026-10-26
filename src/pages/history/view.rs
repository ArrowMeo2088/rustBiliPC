//! 历史页：最近观看的条目，卡片式，点卡片进详情页；带**本地筛选**与**再加载 30 条**。
//!
//! ## 搜索为什么是本地的，而且要说清范围
//!
//! 这条接口**没有** `keyword` 参数：`HistoryListParams` 只有 `max/business/view_at/typ/ps`
//! （`ref/bpi-rs/src/historytoview/params.rs:84-90`），整个库里也没有 `history/search` 端点。
//! 所以这一页的过滤只能盖住**已经加载到本机**的那些条目。界面不藏这件事：
//! 输入框的提示写"筛选已加载的历史"，页头写"筛出 N / 已加载 M 条"，
//! 想要更大范围就用"再加载 30 条"把它扩出去（用户 2026-10-26 定稿就是这个方案）。
//!
//! ## "再加载"靠游标三件套，不靠页码
//!
//! B 站这条是游标分页：把上一页 `cursor` 的 `max`/`view_at`/`business` 原样回传
//! （见 [`fetch_history`](crate::api::history::fetch_history)）。取完的判据用**我们自己那条**：
//! 这次没拿到条目就停按钮，不看接口的 cursor（它就算空列表也照样回，照它判断会出现
//! "点了再加载、什么也没多、按钮也不消失"的死圈）。
//!
//! ## 卡片的第三行放的是"看到哪儿"
//!
//! `HistoryItem` 不给播放量，给的是 `progress`（上次看到的秒数）与 `tag_name`（子分区名）。
//! 历史里用户认的是"我看到哪儿了"，所以左格放分区、右格放进度；进度文案与待看同一份口径
//! （[`progress_text`](crate::api::progress_text)：`0` 与 `-1` 都给空串）。

use std::sync::Arc;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use crate::api::history::{self, HistoryCursor, HistoryItem};
use crate::pages::{loading, matches_keyword, OpenVideo};
use crate::ui::video_card::{self, VideoCardData};
use crate::ui::scroll::{should_auto_page, DragScroll};
use crate::ui::{layout, palette};

/// 一次取多少条历史。库里的 `ps` 只拒绝 0（`params.rs:136-143`），
/// 30 与 B 站网页端一屏给的数量一致；"再加载"每次也是这个数。
const HISTORY_LIMIT: u32 = 30;

/// 历史页根视图。
pub struct HistoryPage {
    client: Arc<bpi_rs::BpiClient>,
    /// 单指/按住拖动的滚动状态（与本页那个滚动容器一一对应，见 `ui/scroll.rs`）。
    scroll: DragScroll,
    items: Vec<HistoryItem>,
    /// 下一页的起点；`None` = 翻到头了（或还没取过）。
    cursor: Option<HistoryCursor>,
    input: Entity<InputState>,
    filter: SharedString,
    note: SharedString,
    seq: u32,
    loaded: bool,
    /// 有没有在途的历史请求。自动连续翻页靠它挡"一帧连发五个下一页"（见
    /// [`should_auto_page`](crate::ui::scroll::should_auto_page)），所以**开始置真、回来置假**。
    loading: bool,
    _subscriptions: Vec<Subscription>,
}

impl HistoryPage {
    /// 建页但不发请求（懒加载）。要 `window` 是为了 `InputState::new` 与 `subscribe_in`。
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        client: Arc<bpi_rs::BpiClient>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("筛选已加载的历史（标题或 UP 主）")
        });
        let subscriptions = vec![cx.subscribe_in(
            &input,
            window,
            |this, state, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.filter = state.read(cx).value().trim().to_string().into();
                    cx.notify();
                }
            },
        )];

        Self {
            scroll: DragScroll::default(),
            client,
            items: Vec::new(),
            cursor: None,
            input,
            filter: "".into(),
            note: "".into(),
            seq: 0,
            loaded: false,
            loading: false,
            _subscriptions: subscriptions,
        }
    }

    /// 第一次被看到时取一次。
    pub fn ensure_loaded(&mut self, cx: &mut Context<Self>) {
        if self.loaded {
            return;
        }
        self.fetch(None, cx);
    }

    /// 重新取第一页（清空已加载的，游标从头）。
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.items.clear();
        self.cursor = None;
        self.fetch(None, cx);
    }

    /// 往后翻一页并**累加**。翻到头时这个按钮不会再出现（`cursor` 是 `None`）。
    fn load_more(&mut self, cx: &mut Context<Self>) {
        let Some(cursor) = self.cursor.clone() else {
            return;
        };
        self.fetch(Some(cursor), cx);
    }

    /// 取一段历史。`after` 是 `None` 时替换列表，`Some` 时往后加一页。
    fn fetch(&mut self, after: Option<HistoryCursor>, cx: &mut Context<Self>) {
        self.loaded = true;
        self.loading = true;
        self.seq += 1;
        let seq = self.seq;
        let resuming = after.is_some();
        self.note = if resuming {
            "正在加载更早的记录…".into()
        } else {
            "历史加载中…".into()
        };
        cx.notify();

        let join = history::fetch_history(Arc::clone(&self.client), HISTORY_LIMIT, after);
        // `move`：这个闭包要跨 `await` 存活（`'static`），所以 `resuming` 必须搬进去而不是借。
        loading::spawn_into(cx, seq, "观看历史", join, |page| page.seq, move |page, outcome| {
            page.loading = false;
            match outcome {
                Ok(batch) => {
                    if !resuming {
                        page.items = batch.items;
                    } else {
                        page.items.extend(batch.items);
                    }
                    page.cursor = batch.next;
                    page.note = if page.items.is_empty() {
                        "还没有观看记录".into()
                    } else {
                        "".into()
                    };
                }
                Err(message) => {
                    if !resuming {
                        page.items.clear();
                    }
                    page.note = format!("取历史失败：{message}").into();
                }
            }
        });
    }

    /// 当前要画出来的那些条（本地过滤后的结果）。
    fn shown(&self) -> Vec<&HistoryItem> {
        self.items
            .iter()
            .filter(|item| matches_keyword(&item.title, &item.author, self.filter.as_str()))
            .collect()
    }

    /// 页头副标题：筛选时说明盖住了多少条已加载记录。
    fn summary(&self) -> String {
        if self.items.is_empty() {
            return String::new();
        }
        if self.filter.is_empty() {
            return format!("已加载 {} 条", self.items.len());
        }
        format!("筛出 {} / 已加载 {} 条", self.shown().len(), self.items.len())
    }

    fn reload_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("history-reload")
            .label("重新加载")
            .on_click(cx.listener(|this, _event, _window, cx| this.reload(cx)))
            .into_any_element()
    }

    fn load_more_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.cursor.is_none() {
            return None;
        }
        Some(
            div()
                .id("history-more")
                .flex()
                .w_full()
                .justify_center()
                .py(px(palette::CARD_TEXT_ROW_GAP))
                .child(
                    Button::new("history-more-btn")
                        .label(format!("再加载 {HISTORY_LIMIT} 条"))
                        .on_click(cx.listener(|this, _event, _window, cx| this.load_more(cx))),
                )
                .into_any_element(),
        )
    }

    fn cards(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.shown()
            .into_iter()
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
            .collect()
    }
}

/// 点历史卡片 = 请外壳压一层详情页，与搜索/主页/空间页同一个信号。
impl EventEmitter<OpenVideo> for HistoryPage {}

impl Render for HistoryPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 每帧登记一次移动/抬起监听（`Window::on_mouse_event` 只覆盖下一帧，见 `ui/scroll.rs`）。
        self.scroll.watch(window, cx.entity_id());
        // 滚到底就自动取下一页（P13 的"自动连续翻页"）。判据四条，分工写在
        // [`should_auto_page`](crate::ui::scroll::should_auto_page)；那个"再加载"按钮仍然留着，
        // 自动那一路万一没触发（比如窗口很高、一屏装完），手点还能走。
        if should_auto_page(&self.scroll, self.cursor.is_some(), self.items.len(), self.loading) {
            log::debug!("历史页：滚到底，自动取下一页");
            self.load_more(cx);
        }
        let summary = self.summary();
        let note = self.note.clone();
        let empty_hint = !note.is_empty() && self.shown().is_empty();
        let cards = self.cards(cx);
        let more = self.load_more_button(cx);

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(layout::header(
                "历史",
                (!summary.is_empty()).then(|| summary.into()),
                Some(self.reload_button(cx)),
            ))
            .child(layout::search_box(&self.input))
            .child(layout::divider())
            .child(
                div()
                    .id("history-body")
                    .flex_1()
                    .w_full()
                    .min_w(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(self.scroll.handle())
                    .on_mouse_down(MouseButton::Left, self.scroll.on_press())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w_full()
                            // 请求出问题时先把话说清楚再摆列表（第 4.7 节：错误必须看得见）。
                            .child(if empty_hint {
                                layout::note(note)
                            } else {
                                div().into_any_element()
                            })
                            .child(video_card::grid(cards))
                            .children(more),
                    ),
            )
    }
}
