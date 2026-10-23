//! rustBiliPC — Bilibili 桌面客户端（Rust + GPUI，Windows x64）。
//!
//! 模块布局（决策来源见 `docs/architecture.md`）：
//!
//! - [`app`]：进程级基础设施（tokio runtime、exe 同目录路径）。
//! - [`theme`]：把 gpui-kit 的强调色改成 B 站粉（颜色取自 [`ui::palette`]，不另定义一份）。
//! - [`api`]：bpi-rs 的调用封装 + DASH 码流选择（纯函数，可单测）。
//! - [`store`]：凭据/配置的落盘读写（明文 JSON，见其模块注释里的取舍）。
//! - [`player`]：外置 mpv 的启动参数与拉起。**播放期（进度/暂停/音量/字幕显示）归 mpv 自己的 OSC，
//!   这一层不做遥控**；命名管道客户端曾写过一版又按范围删掉，机制留在
//!   `docs/architecture.md` 第 4.14 节。
//! - [`pages`]：按功能阶段划分的页面（登录 → 搜索 → 个人 → 详情）。
//! - [`ui`]：跨页面复用的视图组件（`palette` 是唯一颜色/度量出处，`layout` 是复用件）。

pub mod api;
pub mod app;
pub mod logging;
pub mod pages;
pub mod player;
pub mod store;
pub mod theme;
pub mod ui;

/// 单元测试模块（约定：单元测试集中在 `src/tests/`，见根目录 `test/README.md`）。
pub mod tests;

/// 脚手架版本号，与 Cargo.toml 保持一致。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
