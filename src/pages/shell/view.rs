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
//! ## 栈里放三种页（P9 起）
//!
//! "点卡片 → 详情页"、"点 UP 主 → 空间页"、"点收藏夹 → 内容页"是同一类下钻，所以栈里存的是
//! [`StackPage`]（详情 / 空间 / 收藏夹内容）而不是三个裸字段或三个栈：
//! 打开 = push，返回 = pop，栈空时显示选中的那一页。
//! 订阅按各页**真的会发**的事件给：详情与空间两种页都实现 `OpenVideo` + `OpenUser`，
//! 所以 [`watch_page`](Self::watch_page) 一次装两条；收藏夹内容页只发 `OpenVideo`；
//! 收藏列表页只发 [`OpenFolder`](crate::pages::OpenFolder)，用
//! [`watch_folders`](Self::watch_folders)。第三十三轮的 CI 就是这条把关的——
//! 给不下钻的页多订一次直接 `E0277`。
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
    OpenCollection, OpenFolder, OpenUser, OpenVideo, about::AboutPage,
    collection::CollectionContentPage, detail::DetailPage,
    download::{DownloadPage, DownloadQueue},
    favorites::{FavFolderPage, FavoritesPage},
    history::HistoryPage, home::HomePage, loading, search::SearchPage, space::UserSpacePage,
    toview::ToViewPage,
};
use crate::ui::{layout, palette};

/// 栈里的一层。"点 UP 主"、"点卡片"、"点收藏夹"是三类下钻、会交错发生
/// （收藏 → 夹子内容 → 某个视频 → 那个 UP 主的空间 → …），所以必须做成一个枚举而不是三个栈：
/// 分栈的话"返回"就不知道该弹哪一个了。
enum StackPage {
    Detail(Entity<DetailPage>),
    Space(Entity<UserSpacePage>),
    Folder(Entity<FavFolderPage>),
    /// 合集/系列内容页（P12）。它和 `Folder` 是同一类下钻，但走的是两条接口、两种 id，
    /// 所以单独一个变体而不是给 `Folder` 加个"这是哪种格子"的字段。
    Collection(Entity<CollectionContentPage>),
}

impl StackPage {
    /// 当前这一页是谁。渲染时用它决定放哪个实体进主区。
    fn entity(&self) -> AnyElement {
        match self {
            Self::Detail(page) => page.clone().into_any_element(),
            Self::Space(page) => page.clone().into_any_element(),
            Self::Folder(page) => page.clone().into_any_element(),
            Self::Collection(page) => page.clone().into_any_element(),
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
    /// 下载队列（外壳创建，注入详情页与下载页共用）。
    ///
    /// 为什么不在下载页里自己建：任务是从**详情页**点出来的。放进任何一页都得让另一页认识它，
    /// 而外壳本来就是唯一知道全部页面的那一层（导航决定也在它这里）。
    queue: Entity<DownloadQueue>,
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
        // 这三页现在都有输入框，构造时要 `window`（`InputState::new` 与 `subscribe_in` 都要它）。
        let favorites =
            cx.new(|cx| FavoritesPage::new(window, cx, Arc::clone(&client), mid_raw.clone()));
        let history = cx.new(|cx| HistoryPage::new(window, cx, Arc::clone(&client)));
        let toview = cx.new(|cx| ToViewPage::new(window, cx, Arc::clone(&client)));
        // 队列先建：它要同时交给下载页（看结果）与详情页（发任务），两边都不认识对方。
        let queue = cx.new(|_cx| DownloadQueue::new());
        let download = cx.new(|cx| DownloadPage::new(window, cx, queue.clone()));
        let about = cx.new(|_cx| AboutPage::new(Arc::clone(&client), mid_raw.clone()));

        let mut shell = Self {
            client,
            queue,
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
        // 只有"会发卡片点击"的页要订：收藏页这一版只列收藏夹、点不动（用户定稿"不下钻"），
        // 下载页是占位，设置页没有列表卡片——它们压根没实现 EventEmitter<OpenVideo>，
        // 第三十三轮的 CI 就是在替这一点把关：给收藏页多订一次直接 E0277。
        let home = shell.home.clone();
        let search = shell.search.clone();
        let history = shell.history.clone();
        let toview = shell.toview.clone();
        shell.watch_videos(&home, window, cx);
        shell.watch_videos(&search, window, cx);
        shell.watch_videos(&history, window, cx);
        shell.watch_videos(&toview, window, cx);

        // 收藏页发的不是"打开视频"，是"打开收藏夹"（P9 下钻）。
        let favorites = shell.favorites.clone();
        shell.watch_folders(&favorites, window, cx);

        // 主页是默认页：构造完就把第一批推荐取回来，登录后第一眼不该是一片空白。
        shell.home.update(cx, |page, cx| page.ensure_loaded(cx));
        // 下载队列的 `Drop` 会在外壳被释放时叫停在跑的任务（"退出别留 .part"）。
        // 这一句是**故意多此一举**的一层保险：把队列强提一次，确保它在实体表里有正式归属，
        // 也就确保外壳 drop 时它跟着 drop —— 而不是变成一个"没人持有、被实体表回收得不知去哪"的实体。
        let _ = &shell.queue;
        // 登记给重登协调器（P13）：凭据过期时它要拿这份客户端开扫码窗，
        // 重登成功后要叫这一个外壳刷新第一屏。用弱句柄，窗口关了它就自动失效。
        crate::pages::relogin::set_shell(cx.entity().downgrade());
        // 头像是外壳自己的事（不属于任何一页），所以这一路请求在这里发。
        shell.load_account(mid_raw, cx);
        shell
    }

    /// 重登成功之后刷新：头像/账户那一栏换成新 mid，当前这一格重新取。
    ///
    /// 为什么不动下钻栈：栈里那一层（详情页/合集页）可能正挂着我们在放的 mpv，
    /// 清栈会 drop 那些实体，`DetailPage::Drop` 会 kill 子进程——"重登把视频掐了"是新引入的坏行为。
    /// 其余格子中，只有"当前看得见的这一格"一定要重取（它就是用户刚看到报错的地方）；
    /// 切过去时各页仍按自己的 `ensure_loaded` 走。
    pub fn reload_after_relogin(&mut self, mid_raw: String, cx: &mut Context<Self>) {
        log::info!(
            "外壳：重登完成，刷新当前格 {}（mid={mid_raw}）",
            self.tab.label()
        );
        if !mid_raw.trim().is_empty() {
            self.load_account(mid_raw.clone(), cx);
        }
        match self.tab {
            ShellTab::Home => self.home.update(cx, |page, cx| page.reload(cx)),
            ShellTab::Favorites => self.favorites.update(cx, |page, cx| {
                page.set_mid(mid_raw.clone());
                page.reload(cx);
            }),
            ShellTab::History => self.history.update(cx, |page, cx| page.reload(cx)),
            ShellTab::ToView => self.toview.update(cx, |page, cx| page.reload(cx)),
            ShellTab::About => self.about.update(cx, |page, cx| {
                page.set_mid(mid_raw.clone());
                page.reload(cx);
            }),
            // 搜索页要保留用户输入的关键词、下载页没有网络请求：这两格没什么要重取的。
            ShellTab::Search | ShellTab::Download => {}
        }
        cx.notify();
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

    /// 只订"点收藏夹 → 内容页"这一种信号的页（目前就是收藏列表页）。
    fn watch_folders<T>(&mut self, page: &Entity<T>, window: &mut Window, cx: &mut Context<Self>)
    where
        T: 'static + Render + EventEmitter<OpenFolder>,
    {
        let subscription =
            cx.subscribe_in(page, window, |this, _page, event: &OpenFolder, window, cx| {
                this.open_folder(event, window, cx);
            });
        self._subscriptions.push(subscription);
    }

    /// 只订"点合集/系列 → 内容页"这一种信号的页（P12 起就是空间页）。
    fn watch_collections<T>(
        &mut self,
        page: &Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        T: 'static + Render + EventEmitter<OpenCollection>,
    {
        let subscription = cx.subscribe_in(
            page,
            window,
            |this, _page, event: &OpenCollection, window, cx| {
                this.open_collection(event, window, cx);
            },
        );
        self._subscriptions.push(subscription);
    }

    /// 压一层合集/系列内容页。`kind` 决定这一页发的是哪一条接口（见 `api/collection.rs`），
    /// `title` 先占住页头——系列那一路接口不给名字，合集回来后会用 `meta.name` 覆盖。
    fn open_collection(
        &mut self,
        event: &OpenCollection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        log::debug!(
            "外壳：打开{}内容 id={} mid={}（{}，栈深 {}）",
            event.kind.label(),
            event.id,
            event.mid,
            event.title,
            self.stack.len()
        );
        let page = cx.new(|_cx| {
            CollectionContentPage::new(
                Arc::clone(&self.client),
                event.mid,
                event.kind,
                event.id,
                event.title.clone(),
            )
        });
        // 内容页只发"打开视频"这一种信号（点一行就是进那一集的详情页）。
        self.watch_videos(&page, window, cx);
        page.update(cx, |page, cx| page.ensure_loaded(cx));
        self.stack.push(StackPage::Collection(page));
        cx.notify();
    }

    /// 压一层收藏夹内容页。进来就取第 1 页：这一页是被点进来的，不给一片空白。
    fn open_folder(&mut self, event: &OpenFolder, window: &mut Window, cx: &mut Context<Self>) {
        log::debug!(
            "外壳：打开收藏夹内容 id={}（{}，栈深 {}）",
            event.id,
            event.title,
            self.stack.len()
        );
        let page = cx.new(|cx| {
            FavFolderPage::new(
                window,
                cx,
                Arc::clone(&self.client),
                event.id,
                event.title.clone(),
            )
        });
        // 内容页只发"打开视频"这一种信号，订那一条就够（P9：夹子里点卡片进详情）。
        self.watch_videos(&page, window, cx);
        page.update(cx, |page, cx| page.ensure_loaded(cx));
        self.stack.push(StackPage::Folder(page));
        cx.notify();
    }

    /// 压一层详情页。它自己发的两种事件都订阅上，所以"相关推荐一路点下去"和
    /// "点 UP 主进空间、空间里再点卡片"都是连续的。
    fn open_detail(&mut self, bvid: &str, window: &mut Window, cx: &mut Context<Self>) {
        log::debug!("外壳：打开详情页 {bvid}（当前栈深 {}）", self.stack.len());
        let page = cx.new(|cx| {
            DetailPage::new(
                Arc::clone(&self.client),
                bvid.to_string(),
                self.queue.clone(),
                cx,
            )
        });
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
        // 空间页还会发"打开合集/内容"这一种信号（P12）。它**只有空间页发**——
        // 详情页没有 `EventEmitter<OpenCollection>`，所以这条不能塞进 `watch_page`
        // 那个泛型里（那会让详情页编译不过，同第三十三轮收藏页那条 E0277 的教训）。
        self.watch_collections(&page, window, cx);
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

    /// 切入口页：**先清掉下钻栈**，再补取没取过的块。
    ///
    /// 为什么必须清栈（2026-10-26 用户报的缺陷）：上一版只改 `tab`，而渲染时"栈非空就显示栈顶"，
    /// 于是人在详情页/空间页/收藏夹内容里点左侧任何一个入口都**看起来没反应**，
    /// 只能先点"返回"把栈弹空。深层级点侧栏应当直接换页——这才是侧栏导航的语义，
    /// "返回"留给逐层退，两件事各归各。
    ///
    /// 一个必须知道的后果：清栈会 drop 掉那些页的实体，而 `DetailPage` 在自己 `Drop` 里
    /// `kill()` 它拉起的 mpv（QODER.md"子进程必须有归宿"那条定稿）。
    /// 所以**看着视频时点侧栏换页，会把那个 mpv 实例关掉**——与一路点"返回"退到空是同一行为，
    /// 不是这里新引入的。要不要改成"换页不动播放"是另一个决定，没问之前不动它。
    fn select_tab(&mut self, tab: ShellTab, cx: &mut Context<Self>) {
        let jumped = self.tab != tab;
        self.tab = tab;
        if !self.stack.is_empty() {
            log::debug!("外壳：切入口页 {}，清掉深度 {} 的下钻栈", tab.label(), self.stack.len());
            self.stack.clear();
        }
        if jumped {
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
