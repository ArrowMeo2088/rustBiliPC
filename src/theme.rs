//! 主题令牌覆盖：把 gpui-kit 的中性强调色换成 B 站品牌色。
//!
//! 视觉规格来源是 `ref/Bili.Copilot`（WinUI3 项目），这里只移植"可翻译"的部分：
//! 强调色与前景色。它实际用的是系统 accent，品牌粉 #FB7299 是我们自己的选择。
//! 布局尺寸（卡片 232×238、间距 20、封面圆角 4）在 [`crate::ui`] 里按令牌落地。

use gpui_kit::component::Theme;
use gpui_kit::{rgb, App};

/// B 站品牌粉。
pub const BRAND_PINK: u32 = 0xFB7299;

/// 强调色上的前景色。
///
/// #FB7299 的亮度约 0.72，配白字对比度不足 4.5:1，所以这里用近黑，
/// 而不是沿用 kit 默认的白字。
pub const BRAND_PINK_FOREGROUND: u32 = 0x1F1F1F;

/// 应用品牌色到全局主题。必须在 `gpui_kit::init(cx)` 之后、开窗之前调用。
pub fn apply(cx: &mut App) {
    Theme::update(cx, |theme| {
        theme.colors.accent = rgb(BRAND_PINK).into();
        theme.colors.accent_foreground = rgb(BRAND_PINK_FOREGROUND).into();
    });
}
