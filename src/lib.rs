//! rustBiliPC 库入口：可测试的逻辑收敛在这里，`src/main.rs` 只做启动。

pub mod tests;

/// 当前脚手架版本号，后续与 Cargo.toml 保持同步。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
