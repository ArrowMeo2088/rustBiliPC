//! 主界面外壳：顶部两个入口（搜索 / 我的），下面放选中的那一页。
//!
//! 为什么要有这一层：定稿是"同一个窗口内切换视图，不开第二个窗口"（第 4.6 节），
//! 而 P2b 之后窗口里至少有两个页面。把切换收在外壳里，`root.rs` 就只管"登录 vs 主界面"，
//! 外壳只管"搜索 vs 我的"，两层的职责不互相污染。
//!
//! 两个页面实体**都一直持有**，切回来时不会重新发请求：搜索结果和已加载的收藏/历史都还在。

use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;

use crate::pages::mine::MinePage;
use crate::pages::search::SearchPage;

/// 外壳当前显示哪一页。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellTab {
    /// 搜索页（P1）。
    Search,
    /// 个人中心页（P2b）。
    Mine,
}

/// 主界面外壳视图。
pub struct MainShell {
    search: Entity<SearchPage>,
    mine: Entity<MinePage>,
    tab: ShellTab,
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
        // 个人中心随外壳一起构造，但**第一次切到它才发请求**（懒加载见 MinePage::ensure_loaded）。
        let mine = cx.new(|_cx| MinePage::new(Arc::clone(&client), mid_raw));
        Self {
            search,
            mine,
            tab: ShellTab::Search,
        }
    }

    /// 顶部入口按钮。
    fn tab_button(&self, tab: ShellTab, label: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
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
}

impl Render for MainShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match self.tab {
            ShellTab::Search => self.search.clone().into_any_element(),
            ShellTab::Mine => self.mine.clone().into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .px_4()
                    .p_2()
                    .items_center()
                    .child(div().text_base().child("rustBiliPC"))
                    .child(self.tab_button(ShellTab::Search, "搜索", cx))
                    .child(self.tab_button(ShellTab::Mine, "我的", cx))
                    .child(div().flex_1()),
            )
            .child(div().flex_1().w_full().child(page))
    }
}
