//! 主界面外壳：左侧窄栏、下钻栈、以及把它们串起来的事件。
//!
//! ## 这一层的职责
//!
//! 窗口这一层已经分出去了（登录窗与主窗各开各的，见 [`windows`](crate::pages::windows)），
//! 外壳只管"主区此刻显示哪一页"：七个平级入口 + 一个下钻栈（详情页 / 空间页）。
//!
//! ## 侧栏：80 宽、图标 + 两字
//!
//! 2026-10-26 用户定稿四条，都落在这里：
//! - **不要"我的"这一格**，改成 `收藏 / 历史 / 待看 / 下载 / 设置` 五个平级入口
//!   （连同原有的 主页 / 搜索 共七项）；原来 P2b 那个"三块挤在一页里"的个人中心拆掉了。
//! - **栏宽收到 1/3**（240 → 80），所以排法改成竖排图标 + 两字（[`side_item`](crate::ui::layout::side_item)）。
//! - **左上角不再写应用名，改成登录账号的头像**，而且**只做显示、不可点**（他的原话）。
//!   头像要一次网络请求才回来，请求没回来之前那块由组件画同尺寸的暗色底，尺寸不跳。
//! - **左下角那行 mpv 自检删掉**：信息没丢，挪进设置页（`pages/about/view.rs`）。
//!
//! ## 头像那一路请求失败了怎么办
//!
//! 只在日志里留话，界面上不解释——头像是装饰，功能一页都不缺。真正需要看见的账户错误
//! 由设置页自己那一路请求负责显示（它有自己的提示位），这里不摆第二份同样的话。
//!
//! ## 栈里可以放两种页
//!
//! "点 UP 主 → 空间页"和"点卡片 → 详情页"是同一类下钻，所以栈里存的是
//! [`StackPage`]（详情 / 空间）而不是裸的 `Entity<DetailPage>`：
//! 打开 = push，返回 = pop，栈空时显示选中的那一页。
//! 两种页发的都是同一对事件（[`OpenVideo`](crate::pages::OpenVideo) 与
//! [`OpenUser`](crate::pages::OpenUser)），新压进来的一页两条订阅都装上；
//! 导航决定（压哪一层、弹回哪一层）全部收在这里，页面不知道自己在栈里。
//! "返回"做成侧栏第一项而不是再加一条横栏：再加一条就会和详情页自己的 56 页头叠成两层，
//! 看着像两个标题栏。文案也从"← 返回上一层"缩成"返回"——80 宽放不下那句。
//!
//! ## 七个入口的实体都一直持有
//!
//! 切回来时不重新请求：主页那一批推荐、搜索结果、已加载的收藏/历史/待看都还在。
//! 主页是默认页（构造时就取第一批），其余六个第一次被看到才发请求（各自的 `ensure_loaded`）。
//!
//! ## 订阅的生命周期
//!
//! `Subscription` 一被 drop 就取消订阅（它是 `#[must_use]`，`gpui/src/subscription.rs:149`），
//! 所以每条订阅都存进 `_subscriptions`。从栈里 pop 掉的页会留下一条指向已释放实体的订阅，
//! gpui 随实体释放让它自动失效，不会累积回调数量之外的成本。

use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::*;

use crate::api::profile;
use crate::pages::{
    OpenUser, OpenVideo, about::AboutPage, detail::DetailPage, download::DownloadPage,
    favorites::FavoritesPage, history::HistoryPage, home::HomePage, loading, search::SearchPage,
    space::UserSpacePage, toview::ToViewPage,
};
use crate::ui::{layout, palette};

/// 栈里的一层。"点 UP 主"和"点卡片"是两类下钻、会交错发生，所以必须做成一个枚举而不是两个栈：
/// 两个栈的话，"返回"就不知道该弹哪一个了。
enum StackPage {
    Detail(Entity<DetailPage>),
    Space(Entity<UserSpacePage>),
}

impl StackPage {
    /// 当前这一页是谁。渲染时用它决定放哪个实体进主区。
    fn entity(&self) -> AnyElement {
        match self {
            Self::Detail(page) => page.clone().into_any_element(),
            Self::Space(page) => page.clone().into_any_element(),
        }
    }
}

/// 侧栏的七个入口（栈空时主区显示哪一个）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellTab {
    /// 主页（首页推荐流）。登录后第一眼就是它。
    Home,
    /// 搜索页。
    Search,
    /// 收藏：收藏夹列表。
    Favorites,
    /// 历史：最近观看。
    History,
    /// 待看：稍后再看列表。
    ToView,
    /// 下载：这一版是占位页。
    Download,
    /// 设置：关于本机（版本 / 账户 / 文件位置 / 画质上限）。
    About,
}

impl ShellTab {
    /// 侧栏上显示的两字标签。
    fn label(self) -> &'static str {
        match self {
            Self::Home => "主页",
            Self::Search => "搜索",
            Self::Favorites => "收藏",
            Self::History => "历史",
            Self::ToView => "待看",
            Self::Download => "下载",
            Self::About => "设置",
        }
    }

    /// 图标名。**这些变体必须真实存在于 `ref/gpui-kit/crates/assets/assets/icons/`**：
    /// 名字写错既不会编译失败也不会 panic，只会画出一片空白（见
    /// [`side_item`](crate::ui::layout::side_item) 的说明）。
    /// 完整目录里的 `House`/`Clock`/`Bookmark`/`Download` 不在 kit 默认那 106 个图标里，
    /// 所以 `main.rs` 挂的是 `assets::AllAssets`。
    fn icon(self) -> IconName {
        match self {
            Self::Home => IconName::House,
            Self::Search => IconName::Search,
            Self::Favorites => IconName::Star,
            Self::History => IconName::Clock,
            Self::ToView => IconName::Bookmark,
            Self::Download => IconName::Download,
            Self::About => IconName::Settings,
        }
    }
}

/// 主界面外壳视图。
pub struct MainShell {
    client: Arc<bpi_rs::BpiClient>,
    home: Entity<HomePage>,
    search: Entity<SearchPage>,
    favorites: Entity<FavoritesPage>,
    history: Entity<HistoryPage>,
    toview: Entity<ToViewPage>,
    download: Entity<DownloadPage>,
    about: Entity<AboutPage>,
    tab: ShellTab,
    /// 左上角头像的 URL；空串表示还没拿到（或取失败），组件会画那块暗色底。
    face: SharedString,
    /// 头像这一路的请求序号：只有最新那次的结果允许写回（第 4.9 节）。
    account_seq: u32,
    /// 下钻栈；最后一个在最上面（详情页或空间页）。
    stack: Vec<StackPage>,
    _subscriptions: Vec<Subscription>,
}

impl MainShell {
    /// 建外壳。`mid_raw` 是凭据里的 `DedeUserID`，入口页各自拿它去要自己的数据。
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        client: Arc<bpi_rs::BpiClient>,
        mid_raw: String,
    ) -> Self {
        let search = cx.new(|cx| SearchPage::new(window, cx, Arc::clone(&client)));
        let home = cx.new(|_cx| HomePage::new(Arc::clone(&client)));
        let favorites = cx.new(|_cx| FavoritesPage::new(Arc::clone(&client), mid_raw.clone()));
        let history = cx.new(|_cx| HistoryPage::new(Arc::clone(&client)));
        let toview = cx.new(|_cx| ToViewPage::new(Arc::clone(&client)));
        let download = cx.new(|_cx| DownloadPage::new());
        let about = cx.new(|_cx| AboutPage::new(Arc::clone(&client), mid_raw.clone()));

        let mut shell = Self {
            client,
            home,
            search,
            favorites,
            history,
            toview,
            download,
            about,
            tab: ShellTab::Home,
            face: "".into(),
            account_seq: 0,
            stack: Vec::new(),
            _subscriptions: Vec::new(),
        };

        // 实体已经进了 `shell`，订阅用的是"另一份句柄"，所以先各自 clone 成本地变量再传引用：
        // 直接写 `shell.watch_videos(&shell.home.clone(), ..)` 会同时借到 `shell` 的可变与不可变引用。
        // 五个句柄类型各不相同（`Entity<HomePage>` / `Entity<SearchPage>`…），也不能塞进同一个数组，
        // 所以这里是一行一次调用——`watch_videos` 是泛型的，加新入口页只要多一行。
        let home = shell.home.clone();
        let search = shell.search.clone();
        let favorites = shell.favorites.clone();
        let history = shell.history.clone();
        let toview = shell.toview.clone();
        shell.watch_videos(&home, window, cx);
        shell.watch_videos(&search, window, cx);
        shell.watch_videos(&favorites, window, cx);
        shell.watch_videos(&history, window, cx);
        shell.watch_videos(&toview, window, cx);

        // 主页是默认页：构造完就把第一批推荐取回来，登录后第一眼不该是一片空白。
        shell.home.update(cx, |page, cx| page.ensure_loaded(cx));
        // 头像是外壳自己的事（不属于任何一页），所以这一路请求在这里发。
        shell.load_account(mid_raw, cx);
        shell
    }

    /// 取账户资料，只为左上角那张头像。
    fn load_account(&mut self, mid_raw: String, cx: &mut Context<Self>) {
        self.account_seq += 1;
        let seq = self.account_seq;
        let Ok(mid) = mid_raw.trim().parse::<u64>() else {
            log::error!("外壳：凭据里的 mid 不是数字，头像这一栏留空: {mid_raw:?}");
            return;
        };
        let join = profile::fetch_profile(Arc::clone(&self.client), mid);
        loading::spawn_into(
            cx,
            seq,
            "外壳头像",
            join,
            |shell| shell.account_seq,
            |shell, outcome| match outcome {
                Ok(account) => shell.face = account.face.into(),
                // 头像拿不到不影响任何一页的功能：只在日志里留话。要看账户错误去设置页，
                // 那一页有自己的提示位（文件头写过这条分工）。
                Err(message) => log::warn!("外壳：头像请求失败，左上角留空: {message}"),
            },
        );
    }

    /// 只订"点卡片 → 详情页"这一种信号的入口页。
    fn watch_videos<T>(&mut self, page: &Entity<T>, window: &mut Window, cx: &mut Context<Self>)
    where
        T: 'static + Render + EventEmitter<OpenVideo>,
    {
        let subscription =
            cx.subscribe_in(page, window, |this, _page, event: &OpenVideo, window, cx| {
                this.open_detail(&event.bvid, window, cx);
            });
        self._subscriptions.push(subscription);
    }

    /// 压一层详情页。它自己发的两种事件都订阅上，所以"相关推荐一路点下去"和
    /// "点 UP 主进空间、空间里再点卡片"都是连续的。
    fn open_detail(&mut self, bvid: &str, window: &mut Window, cx: &mut Context<Self>) {
        log::debug!("外壳：打开详情页 {bvid}（当前栈深 {}）", self.stack.len());
        let page = cx.new(|cx| DetailPage::new(Arc::clone(&self.client), bvid.to_string(), cx));
        self.watch_page(&page, window, cx);
        self.stack.push(StackPage::Detail(page));
        cx.notify();
    }

    /// 压一层空间页。它和详情页一样是"被点进来的"，所以进来就触发首轮加载，
    /// 不给用户一片空白去猜。
    fn open_space(&mut self, user: &OpenUser, window: &mut Window, cx: &mut Context<Self>) {
        log::debug!(
            "外壳：打开空间页 mid={}（{}，栈深 {}）",
            user.mid,
            user.name,
            self.stack.len()
        );
        let page = cx.new(|_cx| {
            UserSpacePage::new(Arc::clone(&self.client), user.mid, user.name.clone())
        });
        self.watch_page(&page, window, cx);
        page.update(cx, |page, cx| page.ensure_loaded(cx));
        self.stack.push(StackPage::Space(page));
        cx.notify();
    }

    /// 新压进来的一页要同时听两种下钻信号：点卡片进详情、点 UP 主进空间。
    ///
    /// 是两条订阅而不是"一个联合事件"：gpui 的 `EventEmitter` 一个事件一次订阅
    /// （`ref/gpui/gpui/src/app/context.rs:341-350`）。两种页都实现了这两个事件，
    /// 所以这里对泛型 `T` 提两个约束就够，不必再为页面做一层适配 trait。
    fn watch_page<T>(&mut self, page: &Entity<T>, window: &mut Window, cx: &mut Context<Self>)
    where
        T: 'static + Render + EventEmitter<OpenVideo> + EventEmitter<OpenUser>,
    {
        self.watch_videos(page, window, cx);
        let on_user = cx.subscribe_in(
            page,
            window,
            |this, _page, event: &OpenUser, window, cx| {
                this.open_space(event, window, cx);
            },
        );
        self._subscriptions.push(on_user);
    }

    /// 切入口页：没取过的块在这里补取。
    fn select_tab(&mut self, tab: ShellTab, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        self.tab = tab;
        match tab {
            ShellTab::Home => self.home.update(cx, |page, cx| page.ensure_loaded(cx)),
            ShellTab::Favorites => {
                self.favorites.update(cx, |page, cx| page.ensure_loaded(cx))
            }
            ShellTab::History => self.history.update(cx, |page, cx| page.ensure_loaded(cx)),
            ShellTab::ToView => self.toview.update(cx, |page, cx| page.ensure_loaded(cx)),
            ShellTab::About => self.about.update(cx, |page, cx| page.ensure_loaded(cx)),
            // 搜索页自己管输入与请求；下载页这一版没有网络请求。
            ShellTab::Search | ShellTab::Download => {}
        }
        cx.notify();
    }

    /// 侧栏：头像块 + 返回（栈非空时）+ 七个入口。
    fn side_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut items: Vec<AnyElement> = Vec::new();

        if !self.stack.is_empty() {
            items.push(layout::side_item(
                "shell-back".to_string(),
                IconName::ArrowLeft,
                "返回",
                false,
                cx.listener(|this, _event, _window, cx| {
                    this.stack.pop();
                    cx.notify();
                }),
            ));
        }

        for tab in [
            ShellTab::Home,
            ShellTab::Search,
            ShellTab::Favorites,
            ShellTab::History,
            ShellTab::ToView,
            ShellTab::Download,
            ShellTab::About,
        ] {
            items.push(layout::side_item(
                format!("shell-tab-{}", tab.label()),
                tab.icon(),
                tab.label(),
                self.tab == tab,
                cx.listener(move |this, _event, _window, cx| {
                    this.select_tab(tab, cx);
                }),
            ));
        }

        div()
            .w(px(palette::SIDE_COLUMN_WIDTH))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(palette::divider())
            .child(
                // 左上角：登录账号的头像，**只做显示、不可点**（2026-10-26 用户定稿）。
                // 这一格的高度沿用参考项目自绘标题栏的 48（`RootLayout.xaml:33`），
                // 里面摆 44 的圆头像（`AVATAR_UP`，与详情页 UP 主那一行同一尺寸）。
                div()
                    .flex()
                    .w_full()
                    .h(px(palette::TITLE_BAR_HEIGHT))
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .child(layout::avatar(self.face.as_str(), palette::AVATAR_UP)),
            )
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(palette::LIST_GAP))
                    .p(px(palette::LIST_ROW_PADDING_Y))
                    .children(items),
            )
            .into_any_element()
    }
}

impl Render for MainShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 最上面是下钻页（详情或空间）就显示它，否则显示侧栏选中的那一页。侧栏一直留着，
        // 这样"进详情 → 一路点下去 → 返回"时上下文不丢（参考项目也是这个结构）。
        let page: AnyElement = match self.stack.last() {
            Some(top) => top.entity(),
            None => match self.tab {
                ShellTab::Home => self.home.clone().into_any_element(),
                ShellTab::Search => self.search.clone().into_any_element(),
                ShellTab::Favorites => self.favorites.clone().into_any_element(),
                ShellTab::History => self.history.clone().into_any_element(),
                ShellTab::ToView => self.toview.clone().into_any_element(),
                ShellTab::Download => self.download.clone().into_any_element(),
                ShellTab::About => self.about.clone().into_any_element(),
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
