//! 我们自己那份**只嵌用到的图标**的资源表（P14）。
//!
//! ## 为什么要自己写一份，而不用 kit 的 `AllAssets`
//!
//! `main.rs` 以前挂的是 `gpui_kit::assets::AllAssets`：它把 kit 那套**完整 Lucide 目录**
//! （1830 个 svg，磁盘上 2.8 MB）用 rust-embed 整个塞进 exe。当时是被迫的：
//! kit 默认的 `Assets` 只有 106 个，侧栏要的 House/Clock/Bookmark/Download 不在里面，
//! 而**图标名找不到不会报错、只会画出一片空白**（`component/src/svg.rs` 那边 `log_err()` 就完了）。
//! 也就是说那 2.8 MB 里有 94% 是永远不会被请求的文件。
//!
//! 现在这张表是"**kit 默认那 106 个**（它自己的组件要用哪些，全在里面）
//! **+ 我们侧栏/页头多用的那 4 个**"，一共 110 个、171 KB，
//! 用 `include_bytes!` 直接嵌（**不引 rust-embed 依赖**，那张表就是编译期常量）。
//! 省下的是 exe 里 ~2.6 MB 的 `.rdata`（UPX 对这种文本型 blob 效果本来就好，
//! 但日常那份不压缩，所以省的是实打实的下载与内存映射）。
//!
//! ## 加图标时要做什么
//!
//! 新增一个 `IconName::X` 用到而表里没有 → **界面那片是空的，不会编译失败**。
//! 所以加图标 = 两件事：把 `ref/gpui-kit/crates/assets/assets/icons/<kebab>.svg` 拷进
//! `src/ui/icons/`，再在下面这张表里加一行。文件名用 `IconName` 的 kebab-case
//! （kit 的 build.rs 就是 `icons/<file-stem>.svg`，`House` → `house.svg`、`ArrowLeft` → `arrow-left.svg`）。
//!
//! [`AppAssets::MISSING_HINT`] 那句是给排查用的：真怀疑少图标时，在 `ref/gpui-kit` 里
//! grep 一下 `default-icons.txt` 有没有它，比猜快。

use std::borrow::Cow;

use gpui_kit::*;

/// 表里每一项：`(gpui 要的路径, 文件字节)`。路径必须与 `IconName::path()` 给的完全一致。
const ICONS: &[(&str, &[u8])] = &[
    ("icons/a-large-small.svg", include_bytes!("icons/a-large-small.svg")),
    ("icons/arrow-down.svg", include_bytes!("icons/arrow-down.svg")),
    ("icons/arrow-left.svg", include_bytes!("icons/arrow-left.svg")),
    ("icons/arrow-right.svg", include_bytes!("icons/arrow-right.svg")),
    ("icons/arrow-up.svg", include_bytes!("icons/arrow-up.svg")),
    ("icons/asterisk.svg", include_bytes!("icons/asterisk.svg")),
    ("icons/ban.svg", include_bytes!("icons/ban.svg")),
    ("icons/battery-charging.svg", include_bytes!("icons/battery-charging.svg")),
    ("icons/battery-full.svg", include_bytes!("icons/battery-full.svg")),
    ("icons/battery-low.svg", include_bytes!("icons/battery-low.svg")),
    ("icons/battery-medium.svg", include_bytes!("icons/battery-medium.svg")),
    ("icons/battery-warning.svg", include_bytes!("icons/battery-warning.svg")),
    ("icons/battery.svg", include_bytes!("icons/battery.svg")),
    ("icons/bell.svg", include_bytes!("icons/bell.svg")),
    ("icons/book-open.svg", include_bytes!("icons/book-open.svg")),
    ("icons/bookmark.svg", include_bytes!("icons/bookmark.svg")),
    ("icons/bot.svg", include_bytes!("icons/bot.svg")),
    ("icons/building-2.svg", include_bytes!("icons/building-2.svg")),
    ("icons/calendar.svg", include_bytes!("icons/calendar.svg")),
    ("icons/case-sensitive.svg", include_bytes!("icons/case-sensitive.svg")),
    ("icons/chart-pie.svg", include_bytes!("icons/chart-pie.svg")),
    ("icons/check.svg", include_bytes!("icons/check.svg")),
    ("icons/chevron-down.svg", include_bytes!("icons/chevron-down.svg")),
    ("icons/chevron-left.svg", include_bytes!("icons/chevron-left.svg")),
    ("icons/chevron-right.svg", include_bytes!("icons/chevron-right.svg")),
    ("icons/chevron-up.svg", include_bytes!("icons/chevron-up.svg")),
    ("icons/chevrons-up-down.svg", include_bytes!("icons/chevrons-up-down.svg")),
    ("icons/circle-alert.svg", include_bytes!("icons/circle-alert.svg")),
    ("icons/circle-check.svg", include_bytes!("icons/circle-check.svg")),
    ("icons/circle-user.svg", include_bytes!("icons/circle-user.svg")),
    ("icons/circle-x.svg", include_bytes!("icons/circle-x.svg")),
    ("icons/clock.svg", include_bytes!("icons/clock.svg")),
    ("icons/close.svg", include_bytes!("icons/close.svg")),
    ("icons/copy.svg", include_bytes!("icons/copy.svg")),
    ("icons/cpu.svg", include_bytes!("icons/cpu.svg")),
    ("icons/dash.svg", include_bytes!("icons/dash.svg")),
    ("icons/delete.svg", include_bytes!("icons/delete.svg")),
    ("icons/download.svg", include_bytes!("icons/download.svg")),
    ("icons/ellipsis-vertical.svg", include_bytes!("icons/ellipsis-vertical.svg")),
    ("icons/ellipsis.svg", include_bytes!("icons/ellipsis.svg")),
    ("icons/external-link.svg", include_bytes!("icons/external-link.svg")),
    ("icons/eye-off.svg", include_bytes!("icons/eye-off.svg")),
    ("icons/eye.svg", include_bytes!("icons/eye.svg")),
    ("icons/file-text.svg", include_bytes!("icons/file-text.svg")),
    ("icons/file.svg", include_bytes!("icons/file.svg")),
    ("icons/folder-closed.svg", include_bytes!("icons/folder-closed.svg")),
    ("icons/folder-open.svg", include_bytes!("icons/folder-open.svg")),
    ("icons/folder.svg", include_bytes!("icons/folder.svg")),
    ("icons/frame.svg", include_bytes!("icons/frame.svg")),
    ("icons/gallery-vertical-end.svg", include_bytes!("icons/gallery-vertical-end.svg")),
    ("icons/github.svg", include_bytes!("icons/github.svg")),
    ("icons/globe.svg", include_bytes!("icons/globe.svg")),
    ("icons/hard-drive.svg", include_bytes!("icons/hard-drive.svg")),
    ("icons/heart-off.svg", include_bytes!("icons/heart-off.svg")),
    ("icons/heart.svg", include_bytes!("icons/heart.svg")),
    ("icons/house.svg", include_bytes!("icons/house.svg")),
    ("icons/inbox.svg", include_bytes!("icons/inbox.svg")),
    ("icons/info.svg", include_bytes!("icons/info.svg")),
    ("icons/inspector.svg", include_bytes!("icons/inspector.svg")),
    ("icons/layout-dashboard.svg", include_bytes!("icons/layout-dashboard.svg")),
    ("icons/loader-circle.svg", include_bytes!("icons/loader-circle.svg")),
    ("icons/loader.svg", include_bytes!("icons/loader.svg")),
    ("icons/map.svg", include_bytes!("icons/map.svg")),
    ("icons/maximize.svg", include_bytes!("icons/maximize.svg")),
    ("icons/memory-stick.svg", include_bytes!("icons/memory-stick.svg")),
    ("icons/menu.svg", include_bytes!("icons/menu.svg")),
    ("icons/mic.svg", include_bytes!("icons/mic.svg")),
    ("icons/minimize.svg", include_bytes!("icons/minimize.svg")),
    ("icons/minus.svg", include_bytes!("icons/minus.svg")),
    ("icons/moon.svg", include_bytes!("icons/moon.svg")),
    ("icons/network.svg", include_bytes!("icons/network.svg")),
    ("icons/palette.svg", include_bytes!("icons/palette.svg")),
    ("icons/panel-bottom-open.svg", include_bytes!("icons/panel-bottom-open.svg")),
    ("icons/panel-bottom.svg", include_bytes!("icons/panel-bottom.svg")),
    ("icons/panel-left-close.svg", include_bytes!("icons/panel-left-close.svg")),
    ("icons/panel-left-open.svg", include_bytes!("icons/panel-left-open.svg")),
    ("icons/panel-left.svg", include_bytes!("icons/panel-left.svg")),
    ("icons/panel-right-close.svg", include_bytes!("icons/panel-right-close.svg")),
    ("icons/panel-right-open.svg", include_bytes!("icons/panel-right-open.svg")),
    ("icons/panel-right.svg", include_bytes!("icons/panel-right.svg")),
    ("icons/pause.svg", include_bytes!("icons/pause.svg")),
    ("icons/play.svg", include_bytes!("icons/play.svg")),
    ("icons/plus.svg", include_bytes!("icons/plus.svg")),
    ("icons/redo-2.svg", include_bytes!("icons/redo-2.svg")),
    ("icons/redo.svg", include_bytes!("icons/redo.svg")),
    ("icons/refresh-cw.svg", include_bytes!("icons/refresh-cw.svg")),
    ("icons/replace.svg", include_bytes!("icons/replace.svg")),
    ("icons/resize-corner.svg", include_bytes!("icons/resize-corner.svg")),
    ("icons/rotate-cw.svg", include_bytes!("icons/rotate-cw.svg")),
    ("icons/search.svg", include_bytes!("icons/search.svg")),
    ("icons/settings-2.svg", include_bytes!("icons/settings-2.svg")),
    ("icons/settings.svg", include_bytes!("icons/settings.svg")),
    ("icons/sort-ascending.svg", include_bytes!("icons/sort-ascending.svg")),
    ("icons/sort-descending.svg", include_bytes!("icons/sort-descending.svg")),
    ("icons/square-terminal.svg", include_bytes!("icons/square-terminal.svg")),
    ("icons/square.svg", include_bytes!("icons/square.svg")),
    ("icons/star-fill.svg", include_bytes!("icons/star-fill.svg")),
    ("icons/star-off.svg", include_bytes!("icons/star-off.svg")),
    ("icons/star.svg", include_bytes!("icons/star.svg")),
    ("icons/sun.svg", include_bytes!("icons/sun.svg")),
    ("icons/thumbs-down.svg", include_bytes!("icons/thumbs-down.svg")),
    ("icons/thumbs-up.svg", include_bytes!("icons/thumbs-up.svg")),
    ("icons/triangle-alert.svg", include_bytes!("icons/triangle-alert.svg")),
    ("icons/undo-2.svg", include_bytes!("icons/undo-2.svg")),
    ("icons/undo.svg", include_bytes!("icons/undo.svg")),
    ("icons/user.svg", include_bytes!("icons/user.svg")),
    ("icons/window-close.svg", include_bytes!("icons/window-close.svg")),
    ("icons/window-maximize.svg", include_bytes!("icons/window-maximize.svg")),
    ("icons/window-minimize.svg", include_bytes!("icons/window-minimize.svg")),
    ("icons/window-restore.svg", include_bytes!("icons/window-restore.svg")),
];

/// 只认上面这张表的资源源。
pub struct AppAssets;

/// 表里没有时的提示语（写在 `list` 的返回值与日志里，方便排查"为什么这片是空的"）。
#[allow(dead_code)]
impl AppAssets {
    /// 加图标该做的事，一句话版（详细版见模块头）。
    pub const MISSING_HINT: &'static str =
        "把 svg 拷进 src/ui/icons/ 并在 ICONS 表里加一行（文件名 = IconName 的 kebab-case）";

    /// 这张表里有多少个图标（单测用它钉住"我们确实只嵌了该嵌的那批"）。
    pub const COUNT: usize = ICONS.len();

    /// 有没有某个路径在这张表里（纯函数，进单测）。
    pub fn has(path: &str) -> bool {
        ICONS.iter().any(|(key, _)| *key == path)
    }

    /// 这张表往 exe 里塞了多少字节（体积回归闸用它；省下来的就是这里）。
    pub fn bytes() -> usize {
        ICONS.iter().map(|(_, one)| one.len()).sum()
    }
}

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        // 找不到时给 `Ok(None)`：kit 那份实现是抛 Err（然后被 `log_err()` 吞掉，画一片空白），
        // 两者表现一样，但 `None` 不会在日志里造一条噪音——我们连日志默认都不开。
        Ok(ICONS
            .iter()
            .find(|(key, _)| *key == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(key, _)| key.starts_with(path))
            .map(|(key, _)| SharedString::from(*key))
            .collect())
    }
}
