//! 全局主题：强制暗色 + 把 kit 的语义色对到我们的色板。
//!
//! ## 为什么必须显式切暗色
//!
//! `gpui_kit::init` 里写的是 `Theme::change(ThemeMode::Light, None, cx)`
//! （gpui-component 0.8.0 的 `src/theme/mod.rs:40`），**默认就是亮色**。
//! 我们自己的页面 div 是硬编码暗色，于是"页面是黑的、kit 组件是白的"——
//! 真机上右上角那块搜索框就是白底黑字（用户 2026-10-25 指出）。
//! 所以这一层要做的是**把 kit 组件也拉进同一套颜色**，而不是在页面里给某个控件打补丁：
//! 打补丁只能修好这一个控件，下次换个组件又是白的。
//!
//! ## 顺序不能反
//!
//! `Theme::change` 会**整体换上**该模式的调色板（`theme/mod.rs:389` 的注释写明"loads the
//! registered theme for mode"），所以先 `change(Dark)` 再 `update` 覆盖我们在乎的几项；
//! 反过来写就被它盖掉。
//!
//! ## 为什么亮色不做
//!
//! Bili.Copilot 也是只做暗色那一套观感，`ui/palette` 里的三档灰是"半透明白叠深底"，
//! 拿到亮色底上全部失效。要做亮色得整组色值另写，等真有需要再说。
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, px, rgb};

use crate::ui::palette;

/// 应用暗色与品牌色。必须在 `gpui_kit::init(cx)` 之后、开窗之前调用。
pub fn apply(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        // ---- 基础三档：底、字、描边，与 ui/palette 同源 ----
        theme.colors.background = palette::page_background().into();
        theme.colors.foreground = palette::text_primary().into();
        theme.colors.border = palette::divider().into();
        // `input` 是输入框的**描边**色；暗色下输入框的底是 `input` 与透明色混合而来
        // （`theme/mod.rs:485` 的 `input_background()`），所以这一项同时决定"黑底"有多黑，
        // 取我们卡片"抬起"那一层，正好是页底上更亮一档的黑。
        theme.colors.input = palette::card_background_elevated().into();
        // 占位文案与次要文字：三级灰。
        theme.colors.muted_foreground = palette::text_tertiary().into();

        // ---- 品牌色 ----
        // Bili.Copilot 用的是系统 accent，`#FB7299` 是我们自己定的（见 palette::ACCENT）。
        theme.colors.accent = rgb(palette::ACCENT).into();
        // 强调色上的字用近黑：#FB7299 亮度约 0.72，白字对比度不足 4.5:1。
        theme.colors.accent_foreground = palette::text_on_accent().into();
        // 聚焦描边（环）用主色：参考项目的输入框获焦时也是露出 accent。
        theme.colors.ring = rgb(palette::ACCENT).into();

        // ---- 按钮 ----
        // `Button::primary()` 读的是 `button_primary` 这一组，**不是 `accent`**
        // （gpui-component 0.8.0 `src/button/button.rs:1002`），所以只改 accent 的话
        // 主按钮还是暗色默认的白底黑字。
        theme.colors.button = palette::card_background().into();
        theme.colors.button_hover = palette::card_background_hover().into();
        theme.colors.button_active = palette::card_background_elevated().into();
        theme.colors.button_foreground = palette::text_primary().into();
        theme.colors.button_primary = rgb(palette::ACCENT).into();
        theme.colors.button_primary_hover = rgb(palette::ACCENT_HOVER).into();
        theme.colors.button_primary_active = rgb(palette::ACCENT_ACTIVE).into();
        theme.colors.button_primary_foreground = palette::text_on_accent().into();

        // ---- 列表/滚动条 ----
        theme.colors.list_hover = palette::card_background_hover().into();
        // 轨道不画、只画滑块：Bili.Copilot 的滚动条就是浮在内容上的一条细灰。
        theme.colors.scrollbar = palette::border_none().into();
        theme.colors.scrollbar_thumb = palette::card_background_hover().into();

        // ---- 圆角与字号 ----
        // WinUI 的 `ControlCornerRadius` 是 4（参考项目的卡片、角标、按钮都用它），
        // Body 字号是 14——kit 默认分别是 6 与 16，不改成这两处，组件和我们画的卡片会明显差一档。
        theme.radius = px(palette::RADIUS);
        theme.font_size = px(palette::FONT_BODY);
    });
}
