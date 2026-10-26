//! 下载页：**占位**。这一版只有入口和一句实话。
//!
//! ## 为什么先把入口摆上
//!
//! 2026-10-26 用户定稿：侧栏要有"下载"这一格，功能之后做。入口先占住位置，
//! 界面结构（侧栏七项）就定型了，做功能时不用再动外壳。
//!
//! ## 为什么页面里什么都没有
//!
//! 下载真正要做的是三件事，每一件都还没定：**存到哪个目录**（现在运行目录只放
//! exe / mpv / debug.log / auth.json / config.json，加一个下载目录要他点头）、
//! **哪些稿件允许下**（1080P 直链有登录校验与时效，见风险表第 13 条）、
//! **并发与断点**（现在整个客户端只有 tokio 的 API runtime，没有下载队列）。
//! 所以这一页不放"开始下载"这种点了没反应的按钮，只把没做的事写出来。

use gpui_kit::*;

use crate::ui::{layout, palette};

/// 下载页根视图（无状态）。
pub struct DownloadPage;

impl DownloadPage {
    pub fn new() -> Self {
        Self
    }
}

impl Render for DownloadPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(layout::header("下载", None, None))
            .child(layout::divider())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .gap(px(palette::CARD_TEXT_ROW_GAP))
                    .px(px(palette::CARD_TEXT_PADDING_X))
                    .py(px(palette::BODY_PADDING_Y))
                    .child(
                        div()
                            .text_size(px(palette::FONT_BODY))
                            .text_color(palette::text_primary())
                            .child("下载还没做，这一版只是把入口摆在这里。"),
                    )
                    .child(
                        div()
                            .text_size(px(palette::FONT_CAPTION))
                            .line_height(px(palette::LINE_HEIGHT_BODY))
                            .text_color(palette::text_secondary())
                            .child(
                                "开工前要先定三件事：下载目录放哪儿、哪些稿件允许下（1080P 直链有登录校验与时效）、\
                                 并发与断点怎么管（客户端目前只有 API 用的 tokio runtime，没有下载队列）。",
                            ),
                    ),
            )
    }
}
