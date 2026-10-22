//! 页面层：按功能阶段划分，一个阶段一个目录；`root.rs` 是窗口根视图。
//!
//! 顺序为 登录/搜索 → 个人 → 详情 → 播放（定稿：没有凭据时启动即进登录页）。
//! 播放阶段的 UI 只是一层控制壳，真正的画面在外部 mpv 窗口里。

pub mod login;
pub mod root;
pub mod search;

pub use root::RootView;
