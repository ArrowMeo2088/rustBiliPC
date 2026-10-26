//! 跨页面复用的"页面外壳"件：页头、分隔线、区块标题、侧栏项、头像、统计行。
//!
//! ## 为什么要有这一层
//!
//! 度量与颜色只在 [`palette`](super::palette) 里出现，排版骨架又要在这些页面里一模一样，
//! 所以做成组件；否则"页头 56 + 1px 线 + 侧栏 80"这套东西会被抄七遍，
//! 下次改度量就得同步七处，一定会有人漏掉其中一处。
//!
//! ## 结构对齐的是 `ref/Bili.Copilot`
//!
//! - 页头 + 分隔线：`PopularPage.xaml:17-35` 的行是"头 **56** + 1px `DividerStrokeColor` + 体"，
//!   页头内左右内边距 16（`PopularMainHeader.xaml:27`）。
//! - 侧栏：参考项目那一栏是 `240` 宽的列 + `ColumnSplitter`，列表项高 **40**、
//!   左右外边距 8、内容内边距 `12,0`、行距 8、底色是卡片底
//!   （`PopularSideBody.xaml:31-52`）。我们这一版按用户要求收到 80，只保留"底色 +
//!   行距 + 2px 选中边框"这三条口径，排法改成图标在上、文字在下（见 [`side_item`]）。
//!   选中态是 WinUI `ItemContainer` 的
//!   `PART_SelectionVisual`：一条 **2px** 的边框 + 换选中底色，
//!   而且这条边框**平时就在，只是透明**（`Overrides.xaml:190-197` 与 `:233-247`），
//!   所以点选不会让内容位移。我们照做：`border_2` + 透明/主色。
//! - 圆头像：它一律用"直径即圆角"（头像 44/48/40，`CornerRadius = w/2`），
//!   所以 [`avatar`] 直接把圆角设成尺寸的一半，不需要单独的"圆形"控件。
//! - 字重：WinUI 那边"要强一档的字"用的是 `*Strong` 样式（`AccountCard.xaml:44` 的
//!   `BodyStrongTextBlockStyle`），对应到 gpui 就是 `font_weight(FontWeight::SEMIBOLD)`（600）；
//!   正文不加粗。层级仍然是"字号 + 三档灰 + 这一档字重"，不靠别的。
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Input, InputState};
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
                        .font_weight(FontWeight::SEMIBOLD)
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

/// 侧栏的一项：**竖排**的"图标在上、两字在下"，高 48（复用 `TITLE_BAR_HEIGHT`）、
/// 圆角 4、常态卡片底；选中换更亮的底 + 让那条常驻 2px 边框变主色；hover 加深底色。
///
/// 为什么换形状：2026-10-26 用户把侧栏收到 80 宽（"左边的菜单的长度也收小到原来的1/3"），
/// 240 那一版的"一行文字"排不下七个入口了。图标 + 两字是这个宽度里唯一还能读的排法；
/// 参考项目的紧凑态也是这个形状，只是它交给系统控件（WinUI `NavigationView`）画，我们没有等价物。
///
/// 图标来自 kit 的 Lucide 集，用的是**完整目录** `gpui_kit::assets::IconName`（1830 个变体，
/// 变体名是文件名的 PascalCase）。两个坑记在这里：
/// - kit 默认那份 `Assets` 只嵌了 106 个图标，`House`/`Clock`/`Bookmark`/`Download` 都不在，
///   所以 `main.rs` 挂的是 `assets::AllAssets`；
/// - 名字写错**不会编译失败也不会 panic**：找不到资源时 svg 那边 `log_err()` 后直接不画
///   （`component/src/svg.rs:166-171`），界面上就是一片空白——加新入口时先确认 `icons/*.svg` 在不在。
///
/// 回调是 gpui 原样的 `Fn(&ClickEvent, &mut Window, &mut App)`，由调用方用 `cx.listener` 造，
/// 组件层不认识任何页面类型。
pub fn side_item(
    id: impl Into<ElementId>,
    icon: IconName,
    label: &'static str,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    // 图标与文字必须同一档灰：只把文字调灰、图标跟着全局色走，选中态就会一边亮一边不亮。
    let color = if selected {
        palette::text_primary()
    } else {
        palette::text_secondary()
    };
    let base = div()
        .flex()
        .flex_col()
        .w_full()
        .h(px(palette::TITLE_BAR_HEIGHT))
        .items_center()
        .justify_center()
        .gap(px(palette::CARD_LINE_GAP))
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
        .text_size(px(palette::FONT_CAPTION))
        // 选中的那一项用主文本色，未选中是次要灰：层级靠"底色 + 边框 + 字色"，不靠花哨颜色。
        .text_color(color)
        .when(selected, |this| {
            this.bg(palette::card_background_elevated())
        })
        .child(
            Icon::new(icon)
                .size(px(palette::SIDEBAR_ICON_SIZE))
                .text_color(color),
        )
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

/// 一条"标题 + 右侧说明"的列表行（收藏夹、分区这类）。
///
/// 度量照 `VideoPartsSection.xaml:18-21`：`MinHeight=40`、`Padding=12,8`、列距 8、
/// 每行都带卡片底色。个人中心与用户空间两处都是这个形状，所以做成复用件——
/// 抄两遍的话下次改度量一定漏一处。
pub fn list_row(label: impl Into<SharedString>, meta: impl Into<SharedString>) -> AnyElement {
    div()
        .flex()
        .w_full()
        .min_h(px(palette::LIST_ROW_MIN_HEIGHT))
        .gap(px(palette::LIST_ROW_PADDING_Y))
        .items_center()
        .px(px(palette::LIST_ROW_PADDING_X))
        .py(px(palette::LIST_ROW_PADDING_Y))
        .overflow_hidden()
        .rounded(px(palette::RADIUS))
        .bg(palette::card_background())
        .child(
            div()
                .flex_1()
                .text_size(px(palette::FONT_BODY))
                .text_color(palette::text_primary())
                .whitespace_nowrap()
                .text_ellipsis()
                .overflow_hidden()
                .child(label.into()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(palette::FONT_CAPTION))
                .text_color(palette::text_tertiary())
                .child(meta.into()),
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

/// 一行筛选/搜索输入框。`input` 由页面自己 `cx.new(|_| InputState::new(window, cx).placeholder(..))`
/// 造好再传进来：占位文案每页不同（"筛选标题或 UP 主" / "在收藏夹内搜索"），
/// 而"什么时候算一次搜索"是页面的事（本地过滤听 `Change`，服务端搜索听回车），
/// 组件只负责把它摆成一行。
///
/// 宽度交给外层 `flex_1()`：这一行既出现在 80 宽侧栏之外的主区顶部，
/// 也可能被塞进页头右侧，所以不写死数字。kit 的 `Input` 底色与字色走全局主题
/// （第二十六轮把主题切成暗色之后，输入框才不再是"页面黑、框白"）。
pub fn search_box(input: &Entity<InputState>) -> AnyElement {
    div()
        .flex()
        .w_full()
        .items_center()
        .gap(px(palette::CARD_TEXT_ROW_GAP))
        .px(px(palette::CARD_TEXT_PADDING_X))
        .pt(px(palette::CARD_TEXT_ROW_GAP))
        .child(div().flex_1().min_w(px(0.)).child(Input::new(input)))
        .into_any_element()
}

/// 可点的一行"标题 + 说明"：与 [`list_row`] 同一套度量（40 高、`12,8` 内边距、卡片底），
/// 差别只在这一行**是交互元素**——收藏列表点进去是收藏夹内容，
/// 所以它要 `id` + `on_click` + hover。
///
/// 为什么不复用 `list_row` 再加个可选回调：gpui 里 `hover`/`on_click` 只存在于
/// `Stateful<Div>`（要先 `.id()`），把它做成可选参数会把复用件变成两种形状；
/// 两个函数共用同一组常量，度量仍然只有一个出处。
pub fn clickable_row(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    meta: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let base = div()
        .flex()
        .w_full()
        .min_h(px(palette::LIST_ROW_MIN_HEIGHT))
        .gap(px(palette::LIST_ROW_PADDING_Y))
        .items_center()
        .px(px(palette::LIST_ROW_PADDING_X))
        .py(px(palette::LIST_ROW_PADDING_Y))
        .rounded(px(palette::RADIUS))
        .cursor_pointer()
        .bg(palette::card_background())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(palette::FONT_BODY))
                .text_color(palette::text_primary())
                .whitespace_nowrap()
                .text_ellipsis()
                .overflow_hidden()
                // `.into()` 不能省：`label` 这个泛型参数本身不满足 `IntoElement`
                // （第三十六轮的两条 E0277 就在这个位置），先转成 `SharedString` 才当得了子元素。
                .child(label.into()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(palette::FONT_CAPTION))
                .text_color(palette::text_tertiary())
                .child(meta.into()),
        );
    base.id(id)
        .on_click(on_click)
        .hover(|style| style.bg(palette::card_background_hover()))
        .into_any_element()
}
