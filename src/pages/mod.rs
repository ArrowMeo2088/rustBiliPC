//! 页面层：按功能阶段划分，一个阶段一个目录；`root.rs` 是窗口根视图。
//!
//! 层次是 `root.rs`（登录 vs 主界面）→ `shell/`（搜索 vs 我的）→ 具体页面，
//! 两层各管各的切换，互不污染。顺序为 登录/搜索 → 个人 → 详情 → 播放
//! （定稿：没有凭据时启动即进登录页）。
//! 播放阶段的 UI 只是一层控制壳，真正的画面在外部 mpv 窗口里。

pub mod detail;
pub mod login;
pub mod mine;
pub mod root;
pub mod search;
pub mod shell;

pub use root::RootView;

/// 页面之间共用的导航事件：卡片被点开 → 打开该稿件的详情页。
///
/// 定义在这一层（而不是各页面各自一个）是因为三个地方发同一个信号：搜索结果的卡片、
/// 个人中心的"最近观看"、详情页的"相关推荐"。外壳只需订阅这一种事件。
#[derive(Clone, Debug)]
pub struct OpenVideo {
    /// 稿件号。
    pub bvid: String,
}
