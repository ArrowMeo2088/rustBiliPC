//! 主题令牌覆盖：把 gpui-kit 的中性强调色换成 B 站品牌色。
//!
//! 视觉规格来源是 `ref/Bili.Copilot`（WinUI3 项目），这里只移植"可翻译"的部分：
//! 强调色与前景色。它实际用的是系统 accent，品牌粉 #FB7299 是我们自己的选择。
//! 这里**不重复定义颜色**：品牌色与"强调色上的前景"就是 [`crate::ui::palette`] 里那两个，
//! 否则同一个粉会在两处漂移（kit 的按钮/输入框读主题，页面读 palette，必须同源）。
use gpui_kit::component::Theme;
use gpui_kit::{App, rgb};

use crate::ui::palette;

/// 应用品牌色到全局主题。必须在 `gpui_kit::init(cx)` 之后、开窗之前调用。
pub fn apply(cx: &mut App) {
    Theme::update(cx, |theme| {
        theme.colors.accent = rgb(palette::ACCENT).into();
        // 强调色上的字用近黑而不是白：#FB7299 亮度约 0.72，白字对比度不足 4.5:1。
        theme.colors.accent_foreground = palette::text_on_accent().into();
    });
}
