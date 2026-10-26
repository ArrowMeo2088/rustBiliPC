//! 页面层：按功能划分，一个功能一个目录。
//!
//! ## 2026-10-26 之后这一层的形状
//!
//! `root.rs`（"同一窗口里换登录/主界面"）退役了，改成 [`windows`]：**没凭据就只开一个
//! 小登录窗（[`gate`] 装 [`login`]），有凭据就直接开主窗口（[`shell`] 是它的根视图）**。
//! 于是这一层的职责变成三件事：
//! - [`windows`]：窗口尺寸、开关顺序（先开主窗再关登录窗，反了进程会自己退出）；
//! - [`gate`]：登录窗的根视图，只负责"显示二维码 + 成功后换窗口"；
//! - [`shell`]：主窗内的七个入口（主页 / 搜索 / 收藏 / 历史 / 待看 / 下载 / 设置）
//!   与下钻栈（详情页 / 空间页）。
//! 原来的 `mine/`（个人中心一页装三块）拆成了 `favorites/`、`history/`，新增 `toview/`、
//! `download/`、`about/`。
//!
//! [`loading`] 是各入口页共用的"一次请求回到实体"样板（序号 + upgrade + update）。
//!
//! 播放期不在这里——进度、暂停、音量、字幕显示都归外置 mpv 自己的 OSC（第 4.3 节）。

pub mod about;
pub mod detail;
pub mod download;
pub mod favorites;
pub mod gate;
pub mod history;
pub mod home;
pub mod loading;
pub mod login;
pub mod search;
pub mod shell;
pub mod space;
pub mod toview;
pub mod windows;

/// 页面之间共用的导航事件：卡片被点开 → 打开该稿件的详情页。
///
/// 定义在这一层（而不是各页面各自一个）是因为五个地方发同一个信号：
/// 主页的推荐卡片、搜索结果的卡片、历史与待看的卡片、详情页的"相关推荐"，
/// 以及空间页的投稿卡片。只发这一种事件的入口页由外壳的 `watch_videos` 订阅，
/// 两种都发的（详情页 / 空间页）用 `watch_page`——两个都是
/// [`MainShell`](crate::pages::shell::MainShell) 的私有方法，见那个文件的说明。
#[derive(Clone, Debug)]
pub struct OpenVideo {
    /// 稿件号。
    pub bvid: String,
}

/// 另一个共用导航事件：点开 UP 主 → 打开 TA 的空间页。
///
/// `name` 是**从上一页带过来的昵称**：空间页的资料卡要一次网络请求才回来，
/// 没有这个名字的话，那一瞬间标题只能空着或者写一串 mid。资料卡回来后页面会用接口值覆盖它。
#[derive(Clone, Debug)]
pub struct OpenUser {
    /// 用户 ID。
    pub mid: u64,
    /// 昵称（用于首屏占位）。
    pub name: String,
}

/// 第三个共用导航事件：点开收藏夹 → 压一层"收藏夹内容"页（P9）。
///
/// `title` 与 [`OpenUser::name`] 同理：内容页要先一次请求才知道夹子名，
/// 先拿列表里那个名字占住页头，回来后用接口的 `info.title` 覆盖（改名了也以接口为准）。
#[derive(Clone, Debug)]
pub struct OpenFolder {
    /// 收藏夹 ID（接口那边叫 `media_id`）。
    pub id: u64,
    /// 夹子名（用于首屏占位）。
    pub title: String,
}

/// 本地关键词过滤：标题或 UP 主名命中即可（2026-10-26 用户定稿的匹配字段）。
///
/// 为什么要有这个函数、而且只做小写化：历史与待看**没有服务端搜索参数**
/// （库里 `HistoryListParams` 只有 `max/business/view_at/type/ps`，
/// `toview_list()` 干脆零参数），所以这两页只能过滤已经拿到手的条目。
/// 空关键词一律放行（"还没输入"和"输入了又删干净"是同一种状态，不许出现空列表）。
/// 大小写只对拉丁字母有意义，中文 `contains` 天然按字面匹配。
pub fn matches_keyword(title: &str, author: &str, keyword: &str) -> bool {
    let needle = keyword.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    title.to_lowercase().contains(&needle) || author.to_lowercase().contains(&needle)
}
