//! 搜索页（阶段一）。
//!
//! 这一页要跑通的是整条链路：输入 → 回车/按钮 → tokio 上的 bpi-rs 请求 →
//! 回到 gpui 更新状态 → 卡片与分页呈现。排版走 `ui/palette` + `ui/layout` 那套度量（P5），
//! 页面自己不留任何颜色与尺寸字面量。
//!
//! 三个刻意的设计选择：
//! - 触发方式同时给"回车"和"搜索"按钮：gpui 的 `InputEvent::PressEnter` 在中文 IME
//!   组词期间的回车语义由 kit 的 InputState 处理，双入口能避免"按了没反应"的误判。
//! - 结果区用普通滚动 + 分页按钮，不上虚拟列表：B 站每页固定几十条，一次渲染的代价
//!   远小于把 `virtual_list` 的 render 回调接错的风险。真正的长列表（历史/收藏）到了
//!   P2/P3 再引虚拟列表。
//! - 封面直接 `img(url)`：gpui 自带 URI 图片加载与缓存，前提见 `docs/architecture.md` 第 4.5 节。

use std::sync::Arc;

// `Button::primary()` 定义在 `ButtonVariants` trait 上（gpui-component 0.8.0 的
// src/button/button.rs:52），不把这个 trait 一起导入就报 "no method named `primary`"。
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::*;

use crate::api::search::{self, PageInfo, SearchSort};
use crate::pages::OpenVideo;
use crate::ui::scroll::DragScroll;
use crate::ui::{layout, palette};
use crate::ui::video_card::{self, VideoCardData};

/// 搜索页根视图。
pub struct SearchPage {
    /// 关键词输入框状态。kit 的 `InputState` 内部是 Rope，中文 IME 合成已可用。
    query: Entity<InputState>,
    /// 单指/按住拖动的滚动状态（`ui::scroll::DragScroll`，与下面那个滚动容器一一对应）。
    scroll: DragScroll,
    /// 复用一份客户端；请求要 clone 成 `Arc` 投给 tokio。
    client: Arc<bpi_rs::BpiClient>,
    /// 当前展示的卡片。
    cards: Vec<VideoCardData>,
    /// 分页信息，由最近一次成功搜索写入。
    info: PageInfo,
    /// 最近一次已提交的关键词（分页时沿用，不再读输入框，避免用户改了一半翻页）。
    submitted: SharedString,
    /// 当前页码，从 1 开始。
    page: u32,
    /// 排序口径。
    sort: SearchSort,
    /// 请求在途标记，防止连点回车并发多次搜索。
    loading: bool,
    /// 请求序号：只有序号仍为最新的那次结果才允许写回界面。
    request_seq: u32,
    /// 状态/错误提示，显示在筛选行中间（异步失败一定要有地方说，见第 4.7 节）。
    feedback: SharedString,
    /// 订阅要随实体存活。
    _subscriptions: Vec<Subscription>,
}

impl SearchPage {
    /// 创建搜索页。需要 `window` 是因为 kit 的 `InputState::new` 与 `subscribe_in` 都要用它。
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        client: Arc<bpi_rs::BpiClient>,
    ) -> Self {
        let query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("输入关键词，回车搜索（支持中文输入法）")
        });

        // 回车即搜索。回调里同时拿到本页实体与输入状态，不需要额外字段。
        // 事件类型显式标注：kit 自身也是这么写的，省略时闭包的泛型参数推断会卡住。
        let _subscriptions = vec![cx.subscribe_in(
            &query,
            window,
            move |this, state, event: &InputEvent, _window, cx| {
                // PressEnter 是带字段的 struct 变体（secondary / shift），必须写 `{ .. }`。
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let keyword = state.read(cx).value().trim().to_string();
                    this.submit(keyword, 1, this.sort, cx);
                }
            },
        )];

        Self {
            scroll: DragScroll::default(),
            query,
            client,
            cards: Vec::new(),
            info: PageInfo::default(),
            submitted: SharedString::default(),
            page: 1,
            sort: SearchSort::Comprehensive,
            loading: false,
            request_seq: 0,
            feedback: "输入关键词后回车或点搜索".into(),
            _subscriptions,
        }
    }

    /// 提交一次搜索。`page` 从 1 开始；空关键词与在途请求直接忽略。
    fn submit(
        &mut self,
        keyword: String,
        page: u32,
        sort: SearchSort,
        cx: &mut Context<Self>,
    ) {
        if keyword.is_empty() || self.loading {
            return;
        }

        self.submitted = keyword.clone().into();
        self.page = page;
        self.sort = sort;
        self.loading = true;
        self.request_seq += 1;
        let seq = self.request_seq;
        self.feedback = format!("搜索中… {keyword}（{}）", sort.label()).into();
        cx.notify();

        let client = Arc::clone(&self.client);
        let join = search::search_videos(client, keyword, page, sort);
        log::debug!("搜索页：等待搜索结果（请求 #{seq}）");

        // 请求在 tokio 上跑，回到 gpui 时用 kit 自身的写法：upgrade 后 update。
        // 末尾的 `.detach()` 不能省：`Task` 被 drop 就等于取消任务
        // （`gpui-pre-scheduler/src/executor.rs:396-402`），那样结果永远回不来。
        // 用 `request_seq` 挡住"翻页时旧请求后到"覆盖新结果。
        cx.spawn(async move |this, cx| {
            let outcome = match join.await {
                Ok(inner) => {
                    log::debug!("搜索页：结果已回到前台执行器（请求 #{seq}）");
                    inner.map_err(|error| error.to_string())
                }
                Err(join_error) => Err(crate::app::describe_join_error(join_error)),
            };
            let Some(this) = this.upgrade() else {
                log::warn!("搜索页：等待期间实体已释放，丢弃结果");
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.request_seq != seq {
                    log::debug!("搜索页：请求 #{seq} 已被更新的请求取代，丢弃结果");
                    return;
                }
                this.loading = false;
                match outcome {
                    Ok(outcome) => {
                        this.info = outcome.info.clone();
                        this.cards = video_card::cards_from(outcome.hits);
                        this.feedback = if this.cards.is_empty() {
                            "没有匹配的结果".into()
                        } else {
                            format!(
                                "共 {} 条，第 {}/{} 页",
                                this.info.num_results, this.info.page, this.info.num_pages
                            )
                            .into()
                        };
                    }
                    Err(message) => {
                        this.cards.clear();
                        this.feedback = format!("搜索失败: {message}").into();
                    }
                }
                log::debug!("搜索页：状态已写入，卡片 {} 张", this.cards.len());
                cx.notify();
            });
        })
        .detach();
    }

    /// 翻页：沿用已提交的关键词，只改页码。
    fn goto_page(&mut self, page: u32, cx: &mut Context<Self>) {
        let keyword = self.submitted.to_string();
        self.submit(keyword, page, self.sort, cx);
    }

    /// 切换排序并回到第 1 页。
    fn change_sort(&mut self, sort: SearchSort, cx: &mut Context<Self>) {
        let keyword = self.submitted.to_string();
        self.submit(keyword, 1, sort, cx);
    }

    /// 分页按钮组。加载中一律不响应，避免同一页并发请求（判断写在回调里）。
    fn pager(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.page;

        div()
            .flex()
            .gap_2()
            .items_center()
            .flex_shrink_0()
            .text_size(px(palette::FONT_CAPTION))
            .child(
                Button::new("prev-page")
                    .label("上一页")
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        if this.page > 1 && !this.loading {
                            let page = this.page - 1;
                            this.goto_page(page, cx);
                        }
                    })),
            )
            .child(
                div()
                    .text_size(px(palette::FONT_CAPTION))
                    .child(format!("第 {current} / 共 {} 页", self.info.num_pages.max(0))),
            )
            .child(
                Button::new("next-page")
                    .label("下一页")
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        if !this.loading && (this.page as i64) < this.info.num_pages.max(0) {
                            let page = this.page + 1;
                            this.goto_page(page, cx);
                        }
                    })),
            )
    }

    /// 排序按钮组。
    fn sort_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut bar = div().flex().gap(px(palette::CARD_TEXT_ROW_GAP)).flex_shrink_0();
        for sort in [
            SearchSort::Comprehensive,
            SearchSort::PlayCount,
            SearchSort::PublishDate,
        ] {
            let active = sort == self.sort;
            let button = Button::new(format!("sort-{}", sort.label()))
                .label(sort.label())
                .on_click(cx.listener(move |this, _, _window, cx| {
                    if this.sort != sort {
                        this.change_sort(sort, cx);
                    }
                }));
            bar = bar.child(if active { button.primary() } else { button });
        }
        bar
    }
}

/// 点卡片时通知外壳打开详情页（事件定义在 `pages/mod.rs`，三个页面共用同一个信号）。
impl EventEmitter<OpenVideo> for SearchPage {}

impl Render for SearchPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 每帧登记一次移动/抬起监听（`Window::on_mouse_event` 只覆盖下一帧，见 `ui/scroll.rs`）。
        self.scroll.watch(window, cx.entity_id());
        // 页头右侧那条就是搜索框 + 按钮。宽度取自 palette，落在参考项目 320~500 的区间里
        // （`RootLayout.xaml:74-80` 的 MinWidth/MaxWidth）。
        let search_box = div()
            .flex()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .w(px(palette::SEARCH_BOX_WIDTH))
            .child(div().flex_1().child(Input::new(&self.query)))
            .child(
                Button::new("do-search")
                    .primary()
                    .label("搜索")
                    .on_click(cx.listener(|this, _, _window, cx| {
                        let keyword = this.query.read(cx).value().trim().to_string();
                        this.submit(keyword, 1, this.sort, cx);
                    })),
            );

        // 副标题就是"当前已提交的关键词"，让用户知道分页/排序作用在哪句话上。
        let keyword = if self.submitted.is_empty() {
            None
        } else {
            Some(self.submitted.clone())
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(layout::header("搜索", keyword, Some(search_box.into_any_element())))
            // 筛选行：排序在左、反馈居中、分页在右。
            .child(
                div()
                    .flex()
                    .w_full()
                    .h(px(palette::LIST_ROW_MIN_HEIGHT))
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .px(px(palette::CARD_TEXT_PADDING_X))
                    .child(self.sort_bar(cx))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(palette::FONT_CAPTION))
                            .text_color(palette::text_tertiary())
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .overflow_hidden()
                            .child(self.feedback.clone()),
                    )
                    .child(self.pager(cx)),
            )
            .child(layout::divider())
            // 结果区。两个必需项：滚动容器要先 `.id()`（`overflow_*` 是 `Stateful<Div>` 的方法），
            // 以及 `min_w(px(0.))`——不给的话这个 flex 子项会被内容撑开，横向能滚出一段空白。
            .child(
                div()
                    .id("search-results")
                    .flex_1()
                    .w_full()
                    .min_w(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(self.scroll.handle())
                    .on_mouse_down(MouseButton::Left, self.scroll.on_press())
                    .px(px(palette::CARD_TEXT_PADDING_X))
                    .child(video_card::grid(
                        self.cards
                            .iter()
                            .cloned()
                            .map(|card| {
                                // 点卡片 → 发 OpenVideo，由外壳负责打开详情页（页面自己不做导航）。
                                let bvid = card.bvid.as_str().to_string();
                                video_card::clickable(
                                    card,
                                    cx.listener(move |_this, _event, _window, cx| {
                                        cx.emit(OpenVideo { bvid: bvid.clone() });
                                    }),
                                )
                                .into_any_element()
                            })
                            .collect(),
                    )),
            )
    }
}
