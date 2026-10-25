//! 主界面外壳：左侧栏、详情页栈、以及把它们串起来的事件。
//!
//! ## 这一层的职责
//!
//! 定稿是"同一个窗口内切换视图，不开第二个窗口"（第 4.6 节）。从 P3 起窗口里不止两个页面
//! （搜索、个人、详情），所以 `root.rs` 只管"登录 vs 主界面"，外壳管"哪一页在最上面"。
//!
//! ## 骨架照 `ref/Bili.Copilot` 的框架页
//!
//! `RootLayout.xaml:32-35` 是"48 的自绘标题栏 + 内容"，内容再分成"240 的侧栏 + 分隔条 + 主区"
//! （`LayoutPageViewModelBase.cs:41` 的 240）。gpui 这边标题栏归系统画，所以我们只抄第二层：
//! **左侧 240 的导航栏 + 右边整块主区**。每页自己的 56 页头与 1px 分隔线在
//! [`crate::ui::layout::header`] 里，不在这里重复。
//!
//! ## 详情页是一个栈，不是一个槽
//!
//! 详情页里有"相关推荐"，点进去还能再来一层，所以用 `detail_stack: Vec<Entity<DetailPage>>`：
//! 打开 = push，返回 = pop。栈空时显示 `tab` 选中的那一页。
//! 三个页面发的都是同一个 [`OpenVideo`](crate::pages::OpenVideo) 事件，外壳只订阅一种信号；
//! 导航决定（压栈/出栈）全部收在这里，页面不知道自己在栈里。
//! "返回"做成侧栏顶部的一项而不是再加一条横栏：再加一条就会和详情页自己的 56 页头叠成两层，
//! 看着像两个标题栏。
//!
//! ## 订阅的生命周期
//!
//! `Subscription` 一被 drop 就取消订阅（它是 `#[must_use]`，`gpui/src/subscription.rs:149`），
//! 所以每条订阅都存进 `_subscriptions`。从栈里 pop 掉的详情页会留下一条指向已释放实体的订阅，
//! gpui 随实体释放让它自动失效，不会累积回调数量之外的成本。
//!
//! ## 三个入口页面都一直持有
//!
//! 切回来时不重新请求：主页那一批推荐、搜索结果和已加载的收藏/历史都还在。
//! 主页是默认页（构造时就取第一批），个人中心与"搜索后才有内容"保持懒加载
//! （见 `pages/home/view.rs`、`pages/mine/view.rs`）。

use std::sync::Arc;

use gpui_kit::*;

use crate::pages::OpenVideo;
use crate::pages::detail::DetailPage;
use crate::pages::home::HomePage;
use crate::pages::mine::MinePage;
use crate::pages::search::SearchPage;
use crate::ui::{layout, palette};

/// 栈空时显示哪一页。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellTab {
    /// 主页（P6：首页推荐流）。登录后第一眼就是它。
    Home,
    /// 搜索页（P1）。
    Search,
    /// 个人中心页（P2b）。
    Mine,
}

/// 主界面外壳视图。
pub struct MainShell {
    client: Arc<bpi_rs::BpiClient>,
    home: Entity<HomePage>,
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
        // 个人中心与主页都随外壳一起构造，但第一次被看到才发请求（懒加载见各自 ensure_loaded）。
        let mine = cx.new(|_cx| MinePage::new(Arc::clone(&client), mid_raw));
        let home = cx.new(|_cx| HomePage::new(Arc::clone(&client)));

        // 三个入口页面的卡片点击都走同一个事件。`subscribe_in` 的回调要 window，
        // 因为压栈时 `cx.new` 建视图实体要用它。
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(
            &home,
            window,
            |this, _home, event: &OpenVideo, window, cx| {
                this.open_detail(&event.bvid, window, cx);
            },
        ));
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

        // 主页是默认页：构造完就把第一批推荐取回来，登录后第一眼不该是一片空白。
        // （"我的"仍然懒加载——用户可能根本不点它。）
        home.update(cx, |page, cx| page.ensure_loaded(cx));

        Self {
            client,
            home,
            search,
            mine,
            tab: ShellTab::Home,
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

    /// 侧栏：应用名 + 返回（栈非空时）+ 三个入口（主页 / 搜索 / 我的）。
    fn side_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut items: Vec<AnyElement> = Vec::new();

        if !self.detail_stack.is_empty() {
            items.push(layout::side_item(
                "detail-back".to_string(),
                "← 返回上一层",
                false,
                cx.listener(|this, _event, _window, cx| {
                    this.detail_stack.pop();
                    cx.notify();
                }),
            ));
        }

        items.push(self.side_tab(ShellTab::Home, "主页", cx));
        items.push(self.side_tab(ShellTab::Search, "搜索", cx));
        items.push(self.side_tab(ShellTab::Mine, "我的", cx));

        div()
            .w(px(palette::SIDE_COLUMN_WIDTH))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(palette::divider())
            .child(
                // 应用名占 48：那是参考项目自绘标题栏的高度，我们放在侧栏顶部对齐它。
                div()
                    .flex()
                    .w_full()
                    .h(px(palette::TITLE_BAR_HEIGHT))
                    .flex_shrink_0()
                    .items_center()
                    .px(px(palette::CARD_TEXT_PADDING_X))
                    .child(
                        div()
                            .text_size(px(palette::FONT_TITLE))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(palette::text_primary())
                            .child("rustBiliPC"),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(palette::CARD_TEXT_ROW_GAP))
                    .p(px(palette::CARD_TEXT_ROW_GAP))
                    .children(items),
            )
            .child(self.status_note(cx))
            .into_any_element()
    }

    /// 侧栏上的一个入口。选中态由 `layout::side_item` 表达（更亮的底色 + 2px 强调色边框）。
    fn side_tab(&self, tab: ShellTab, label: &'static str, cx: &mut Context<Self>) -> AnyElement {
        layout::side_item(
            format!("shell-tab-{label}"),
            label,
            self.tab == tab,
            cx.listener(move |this, _event, _window, cx| {
                if this.tab != tab {
                    this.tab = tab;
                    // 第一次进"我的"时触发它自己的首轮加载；已加载过就只是换个显示。
                    // 主页在构造时已经取过第一批，这里的 ensure_loaded 只是同一条规则的写法统一。
                    match tab {
                        ShellTab::Home => this.home.update(cx, |home, cx| home.ensure_loaded(cx)),
                        ShellTab::Mine => this.mine.update(cx, |mine, cx| mine.ensure_loaded(cx)),
                        ShellTab::Search => {}
                    }
                    cx.notify();
                }
            }),
        )
    }

    /// 侧栏底部那行自检信息（凭据在不在、mpv 在不在）。
    ///
    /// 留着它是因为这两件事一旦不对，症状会出现在别的页面上（图不显示、播不了），
    /// 有一行写在明处的自检比让人猜快得多。
    fn status_note(&self, _cx: &mut Context<Self>) -> AnyElement {
        let mpv = if crate::player::find_mpv().is_some() {
            "mpv：有"
        } else {
            "mpv：缺"
        };
        div()
            .w_full()
            .flex_shrink_0()
            .px(px(palette::CARD_TEXT_PADDING_X))
            .py(px(palette::CARD_TEXT_ROW_GAP))
            .text_size(px(palette::FONT_CAPTION))
            .text_color(palette::text_tertiary())
            .child(format!("{mpv} · {}", crate::player::mpv_path().display()))
            .into_any_element()
    }
}

impl Render for MainShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 最上面是详情页就显示详情页，否则显示侧栏选中的那一页。侧栏一直留着，
        // 这样"进详情 → 一路点相关推荐 → 返回"时上下文不丢（参考项目也是这个结构）。
        let page: AnyElement = match self.detail_stack.last() {
            Some(top) => top.clone().into_any_element(),
            None => match self.tab {
                ShellTab::Home => self.home.clone().into_any_element(),
                ShellTab::Search => self.search.clone().into_any_element(),
                ShellTab::Mine => self.mine.clone().into_any_element(),
            },
        };

        div()
            .size_full()
            .flex()
            .bg(palette::page_background())
            .text_color(palette::text_primary())
            .child(self.side_bar(cx))
            .child(div().flex_1().h_full().min_w(px(0.)).child(page))
    }
}
