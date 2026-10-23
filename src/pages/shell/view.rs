//! 主界面外壳：顶部入口（搜索 / 我的）、详情页栈、以及把它们串起来的事件。
//!
//! 为什么要有这一层：定稿是"同一个窗口内切换视图，不开第二个窗口"（第 4.6 节），
//! 而从 P3 起窗口里不止两个页面（搜索、个人、详情）。`root.rs` 只管"登录 vs 主界面"，
//! 外壳管"哪一页在最上面"，两层的职责不互相污染。
//!
//! ## 详情页是一个栈，不是一个槽
//!
//! 详情页里有"相关推荐"，点进去还能再来一层，所以用 `detail_stack: Vec<Entity<DetailPage>>`：
//! 打开 = push，返回 = pop。栈空时显示 `tab` 选中的那一页。
//! 三个页面发的都是同一个 [`OpenVideo`](crate::pages::OpenVideo) 事件，外壳只订阅一种信号；
//! 导航决定（压栈/出栈）全部收在这里，页面不知道自己在栈里。
//!
//! ## 订阅的生命周期
//!
//! `Subscription` 一被 drop 就取消订阅（它是 `#[must_use]`，`gpui/src/subscription.rs:149`），
//! 所以每条订阅都存进 `_subscriptions`。从栈里 pop 掉的详情页会留下一条指向已释放实体的订阅，
//! gpui 随实体释放让它自动失效，不会累积回调数量之外的成本。
//!
//! ## 两个入口页面都一直持有
//!
//! 切回来时不重新请求：搜索结果和已加载的收藏/历史都还在。个人中心是懒加载
//! （第一次切到它才 `ensure_loaded`，见 `pages/mine/view.rs`）。

use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;

use crate::pages::OpenVideo;
use crate::pages::detail::DetailPage;
use crate::pages::mine::MinePage;
use crate::pages::search::SearchPage;

/// 栈空时显示哪一页。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellTab {
    /// 搜索页（P1）。
    Search,
    /// 个人中心页（P2b）。
    Mine,
}

/// 主界面外壳视图。
pub struct MainShell {
    client: Arc<bpi_rs::BpiClient>,
    search: Entity<SearchPage>,
    mine: Entity<MinePage>,
    tab: ShellTab,
    /// 详情页栈；最后一个在最上面。
    detail_stack: Vec<Entity<DetailPage>>,
    _subscriptions: Vec<Subscription>,
}

impl MainShell {
    /// 建外壳。`mid_raw` 是凭据里的 `DedeUserID`，交给个人中心页去要自己的数据。
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        client: Arc<bpi_rs::BpiClient>,
        mid_raw: String,
    ) -> Self {
        let search = cx.new(|cx| SearchPage::new(window, cx, Arc::clone(&client)));
        // 个人中心随外壳一起构造，但第一次切到它才发请求（懒加载见 MinePage::ensure_loaded）。
        let mine = cx.new(|_cx| MinePage::new(Arc::clone(&client), mid_raw));

        // 两个入口页面的卡片点击都走同一个入口。`subscribe_in` 的回调要 window，
        // 因为压栈时 `cx.new` 建视图实体要用它。
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(
            &search,
            window,
            |this, _search, event: &OpenVideo, window, cx| {
                this.open_detail(&event.bvid, window, cx);
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &mine,
            window,
            |this, _mine, event: &OpenVideo, window, cx| {
                this.open_detail(&event.bvid, window, cx);
            },
        ));

        Self {
            client,
            search,
            mine,
            tab: ShellTab::Search,
            detail_stack: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    /// 压一层详情页。它自己的 `OpenVideo` 也订阅上，所以相关推荐能一路点下去。
    fn open_detail(&mut self, bvid: &str, window: &mut Window, cx: &mut Context<Self>) {
        log::debug!(
            "外壳：打开详情页 {bvid}（当前栈深 {}）",
            self.detail_stack.len()
        );
        let page = cx.new(|cx| {
            DetailPage::new(Arc::clone(&self.client), bvid.to_string(), cx)
        });
        let subscription =
            cx.subscribe_in(&page, window, |this, _page, event: &OpenVideo, window, cx| {
                this.open_detail(&event.bvid, window, cx);
            });
        self._subscriptions.push(subscription);
        self.detail_stack.push(page);
        cx.notify();
    }

    /// 顶部入口按钮。
    fn tab_button(
        &self,
        tab: ShellTab,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let button = Button::new(format!("shell-tab-{label}"))
            .label(label)
            .on_click(cx.listener(move |this, _, _window, cx| {
                if this.tab != tab {
                    this.tab = tab;
                    // 第一次进"我的"时触发它自己的首轮加载；已加载过就只是换个显示。
                    if tab == ShellTab::Mine {
                        this.mine.update(cx, |mine, cx| mine.ensure_loaded(cx));
                    }
                    cx.notify();
                }
            }));
        if self.tab == tab { button.primary() } else { button }
    }

    /// 顶栏：详情页在最上面时只有"返回"，否则是入口切换。
    fn top_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut bar = div().flex().gap_2().px_4().p_2().items_center();

        if !self.detail_stack.is_empty() {
            bar = bar
                .child(
                    Button::new("detail-back")
                        .label("← 返回")
                        .on_click(cx.listener(|this, _, _window, cx| {
                            this.detail_stack.pop();
                            cx.notify();
                        })),
                )
                .child(div().text_sm().child(format!(
                    "后面还压着 {} 层详情",
                    self.detail_stack.len().saturating_sub(1)
                )));
            return bar.into_any_element();
        }

        bar = bar
            .child(div().text_base().child("rustBiliPC"))
            .child(self.tab_button(ShellTab::Search, "搜索", cx))
            .child(self.tab_button(ShellTab::Mine, "我的", cx));
        bar.into_any_element()
    }
}

impl Render for MainShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 最上面是详情页就显示详情页，否则显示 tab 选中的那一页。
        let page: AnyElement = match self.detail_stack.last() {
            Some(top) => top.clone().into_any_element(),
            None => match self.tab {
                ShellTab::Search => self.search.clone().into_any_element(),
                ShellTab::Mine => self.mine.clone().into_any_element(),
            },
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.top_bar(cx))
            .child(div().flex_1().w_full().child(page))
    }
}
