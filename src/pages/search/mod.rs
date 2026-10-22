//! 搜索页模块。后续阶段同构：`pages/<阶段>/{mod.rs, view.rs}`，
//! 需要独立状态时再加 `state.rs`。

mod view;

pub use view::SearchPage;
