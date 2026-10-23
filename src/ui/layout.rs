//! 跨页面复用的"页面外壳"件：页头、分隔线、区块标题、侧栏项、头像、统计行。
//!
//! ## 为什么要有这一层
//!
//! 度量与颜色只在 [`palette`](super::palette) 里出现，排版骨架又要在四个页面里一模一样，
//! 所以做成组件；否则"页头 56 + 1px 线 + 侧栏 240"这套东西会被抄四遍，
//! 下次改度量就得同步四处，一定会有人漏掉其中一处。
//!
//! ## 结构对齐的是 `ref/Bili.Copilot`
//!
//! - 页头 + 分隔线：`PopularPage.xaml:17-35` 的行是"头 **56** + 1px `DividerStrokeColor` + 体"，
//!   页头内左右内边距 16（`PopularMainHeader.xaml:27`）。
//! - 侧栏：同一批页面用 `240` 宽的列 + `ColumnSplitter`；列表项高 **40**、
//!   左右外边距 8、内容内边距 `12,0`、行距 8、底色是卡片底
//!   （`PopularSideBody.xaml:31-52`）。选中态是 WinUI `ItemContainer` 的
//!   `PART_SelectionVisual`：一条 **2px** 的边框 + 换选中底色，
//!   而且这条边框**平时就在，只是透明**（`Overrides.xaml:190-197` 与 `:233-247`），
//!   所以点选不会让内容位移。我们照做：`border_2` + 透明/主色。
//! - 圆头像：它一律用"直径即圆角"（头像 44/48/40，`CornerRadius = w/2`），
//!   所以 [`avatar`] 直接把圆角设成尺寸的一半，不需要单独的"圆形"控件。
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::ui::cover;
use crate::ui::palette;

/// 页头：56 高、左右 16 内边距、垂直居中，右边可选放操作件。
///
/// `trailing` 用 `Option<AnyElement>`：gpui 的一个元素只能有一种子元素类型，
/// 有/无操作件两条分支在这里统一成 `AnyElement`。
pub fn header(
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    trailing: Option<AnyElement>,
) -> AnyElement {
    let head = div()
        .flex()
        .w_full()
        .h(px(palette::HEADER_HEIGHT))
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .px(px(palette::LIST_ROW_PADDING_X))
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .justify_center()
                .child(
                    div()
                        .w_full()
                        .text_size(px(palette::FONT_TITLE))
                        .text_color(palette::text_primary())
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .overflow_hidden()
                        .child(title.into()),
                )
                .when_some(subtitle, |this, text| {
                    this.child(
                        div()
                            .w_full()
                            .text_size(px(palette::FONT_CAPTION))
                            .text_color(palette::text_tertiary())
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .overflow_hidden()
                            .child(text),
                    )
                }),
        );
    let head = match trailing {
        Some(element) => head.child(element),
        None => head,
    };
    // 1px 分隔线：参考项目所有页头下面都是这一条。
    div()
        .flex()
        .flex_col()
        .w_full()
        .flex_shrink_0()
        .child(head)
        .child(divider())
        .into_any_element()
}

/// 一条 1px 的分隔线。
pub fn divider() -> AnyElement {
    div()
        .w_full()
        .h(px(palette::DIVIDER_HEIGHT))
        .flex_shrink_0()
        .bg(palette::divider())
        .into_any_element()
}

/// 区块标题：12px 辅助灰，上下留呼吸。
pub fn section_title(text: impl Into<SharedString>) -> AnyElement {
    div()
        .w_full()
        .pt_4()
        .pb_2()
        .px(px(palette::CARD_TEXT_PADDING_X))
        .text_size(px(palette::FONT_CAPTION))
        .text_color(palette::text_tertiary())
        .child(text.into())
        .into_any_element()
}

/// 侧栏的一项（40 高、圆角 4、常态卡片底；选中换更亮的底 + 2px 强调色边框；hover 加深）。
///
/// 回调是 gpui 原样的 `Fn(&ClickEvent, &mut Window, &mut App)`，由调用方用 `cx.listener` 造，
/// 组件层不认识任何页面类型。
pub fn side_item(
    id: impl Into<ElementId>,
    label: &'static str,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let base = div()
        .flex()
        .w_full()
        .h(px(palette::LIST_ROW_MIN_HEIGHT))
        .items_center()
        .px(px(palette::LIST_ROW_PADDING_X))
        .rounded(px(palette::RADIUS))
        .cursor_pointer()
        // 参考项目每一项都带卡片底色（`PopularSideBody.xaml:41` 的
        // `Background=CardBackgroundFillColorDefaultBrush`），选中只是换更亮的一层 + 出那条 2px 边框。
        .bg(palette::card_background())
        // 边框平时就在，只是透明（`Overrides.xaml:190-197` 的 PART_SelectionVisual）。
        .border_2()
        .border_color(if selected {
            palette::accent()
        } else {
            palette::border_none()
        })
        .text_size(px(palette::FONT_BODY))
        // 选中的那一项用主文本色，未选中是次要灰：层级靠"底色 + 边框 + 字色"，不靠花哨颜色。
        .text_color(if selected {
            palette::text_primary()
        } else {
            palette::text_secondary()
        })
        .when(selected, |this| this.bg(palette::card_background_elevated()))
        .child(label);
    base.id(id)
        .on_click(on_click)
        .hover(|style| style.bg(palette::card_background_hover()))
        .into_any_element()
}

/// 圆头像。`size` 是直径，圆角取一半（参考项目就是这么做的）。
///
/// `url` 用 `&str`：DTO 那边的头像字段（`api::detail::VideoDetailInfo::owner_face`、
/// `api::profile` 的 `face`）都是 `String`，收 `&SharedString` 会让调用点全部多写一次转换。
pub fn avatar(url: &str, size: f32) -> AnyElement {
    let box_base = div()
        .w(px(size))
        .h(px(size))
        .flex_shrink_0()
        .rounded(px(size / 2.))
        .overflow_hidden()
        .bg(palette::card_background_elevated());

    if url.is_empty() {
        return box_base.into_any_element();
    }

    // 设备像素按 2× 向图床要：那边 48 的头像写的是 `DecodeWidth="100"`（`AccountCard.xaml:26`），
    // 我们取整两代（48→96）避免拿 812KB 的原图去铺一个小圆。
    let device = (size * 2.) as u32;
    let source = cover::for_display(url, device, device);
    box_base
        .child(
            img(SharedString::from(source))
                .size_full()
                .object_fit(ObjectFit::Cover)
                .with_fallback(|| {
                    div()
                        .size_full()
                        .bg(palette::card_background_elevated())
                        .into_any_element()
                }),
        )
        .into_any_element()
}

/// 一行统计（12px 辅助灰、组内间距 12）。参考项目所有"数字行"都是这个规格。
pub fn stat_line(items: Vec<SharedString>) -> AnyElement {
    div()
        .flex()
        .flex_wrap()
        .w_full()
        .gap(px(palette::STAT_ITEM_GAP))
        .text_size(px(palette::FONT_CAPTION))
        .text_color(palette::text_tertiary())
        .children(items)
        .into_any_element()
}

/// 一行说明/占位文本（14px 次要灰）。
pub fn note(text: impl Into<SharedString>) -> AnyElement {
    div()
        .w_full()
        .px(px(palette::CARD_TEXT_PADDING_X))
        .py(px(palette::CARD_TEXT_ROW_GAP))
        .text_size(px(palette::FONT_BODY))
        .text_color(palette::text_secondary())
        .child(text.into())
        .into_any_element()
}

/// 卡片式的容器（底色 + 圆角 + 1px 描边），详情页的信息块与评论区都套它。
pub fn card(items: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .w_full()
        .gap(px(palette::CARD_TEXT_ROW_GAP))
        .p(px(palette::LIST_ROW_PADDING_X))
        .overflow_hidden()
        .rounded(px(palette::RADIUS))
        .bg(palette::card_background())
        .border_1()
        .border_color(palette::divider())
        .children(items)
        .into_any_element()
}
