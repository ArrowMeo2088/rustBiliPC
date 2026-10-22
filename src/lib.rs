//! rustBiliPC — Bilibili 桌面客户端（Rust + GPUI，Windows x64）。
//!
//! 模块布局（决策来源见 `docs/architecture.md`）：
//!
//! - [`app`]：进程级基础设施（tokio runtime、exe 同目录路径）。
//! - [`theme`]：把 gpui-kit 的强调色改成 B 站粉。
//! - [`api`]：bpi-rs 的调用封装 + DASH 码流选择（纯函数，可单测）。
//! - [`store`]：凭据/配置的落盘读写（明文 JSON，见其模块注释里的取舍）。
//! - [`player`]：外置 mpv 进程的启动参数与命名管道 IPC。
//! - [`pages`]：按功能阶段划分的页面（搜索 → 个人 → 详情 → 播放）。
//! - [`ui`]：跨页面复用的视图组件。

pub mod api;
pub mod app;
pub mod pages;
pub mod player;
pub mod store;
pub mod theme;
pub mod ui;

/// 单元测试模块（约定：单元测试集中在 `src/tests/`，见根目录 `test/README.md`）。
pub mod tests;

/// 脚手架版本号，与 Cargo.toml 保持一致。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
