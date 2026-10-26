//! 下载模块：**任务队列**（`queue`，被外壳创建、被详情页与下载页共用）+ **列表界面**（`view`）。
//!
//! 分成两个文件是因为下载有两家：发起方在详情页，看结果的地方在下载页。
//! 把它们都指向同一个 `Entity<DownloadQueue>`，页面之间就不需要认识彼此。

mod queue;
mod view;

pub use queue::{DownloadQueue, DownloadTask, DownloadsChanged, TaskState};
pub use view::DownloadPage;
