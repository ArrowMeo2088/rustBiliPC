//! 页面层：按功能阶段划分，一个阶段一个目录；`root.rs` 是窗口根视图。
//!
//! 层次是 `root.rs`（登录 vs 主界面）→ `shell/`（主页 / 搜索 / 我的，外加详情页栈）→ 具体页面，
//! 两层各管各的切换，互不污染。页面顺序按用户定的清单走：
//! **主页、搜索、个人、视频详情**（评论与清晰度呈现继续在 P6 补）。
//! 播放期不在这里——进度、暂停、音量、字幕显示都归外置 mpv 自己的 OSC（第 4.3 节）。

pub mod detail;
pub mod home;
pub mod login;
pub mod mine;
pub mod root;
pub mod search;
pub mod shell;

pub use root::RootView;

/// 页面之间共用的导航事件：卡片被点开 → 打开该稿件的详情页。
///
/// 定义在这一层（而不是各页面各自一个）是因为四个地方发同一个信号：
/// 主页的推荐卡片、搜索结果的卡片、个人中心的"最近观看"、详情页的"相关推荐"。
/// 外壳只需订阅这一种事件。
#[derive(Clone, Debug)]
pub struct OpenVideo {
    /// 稿件号。
    pub bvid: String,
}
