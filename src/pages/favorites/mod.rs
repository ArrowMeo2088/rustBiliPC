//! 收藏这一格的两层页：列表（[`FavoritesPage`]）与点进去的内容（[`FavFolderPage`]）。

mod folder;
mod view;

pub use folder::FavFolderPage;
pub use view::FavoritesPage;
