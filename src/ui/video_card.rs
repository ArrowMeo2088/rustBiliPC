//! 视频卡片：搜索/历史/相关推荐共用的展示单元，度量全部照 `ref/Bili.Copilot`。
//!
//! ## 尺寸从哪来
//!
//! - 网格与卡片：`App.xaml:70-76` 的 `VideoLayout` 是 `UniformGridLayout`，
//!   项 **232×238**、列距 **20**、行距 **20**，容器外边距 `0,16`（`PopularMainBody.xaml:37`）。
//!   gpui 没有 `UniformGridLayout`，等价做法是 `flex_wrap` + 固定宽度的项 + `gap`（见 [`grid`]）；
//!   宽度固定后换行位置就由容器宽度决定，和 UniformGrid 的观感一致。
//! - 封面：`VideoCardControl.xaml:16-19` 的行是 `*` + 文字区 **116**，也就是封面 **232×122**
//!   （比例约 1.90:1，**不是 16:9**——照 16:9 画会显得图小、下面空）。
//!   圆角只有上面两个角（`:21` 的 `CornerRadius=4,4,0,0`）：这里用"外层圆角 + overflow_hidden"
//!   达到同样效果，少一个 API 依赖。
//! - 文字区：`padding 16,12,16,16`、行距 8，三行是 `Auto`/`*`/`Auto`，
//!   **顺序是 UP 主（12px Tertiary）→ 标题（Body 14，两行）→ 统计（12px Tertiary，项距 12）**
//!   （`:60-64`、`:84-101`）。
//! - 时长角标：黑 60% 底、白字 12、离边 8、内边距 4（`:37-50`）。
//! - hover：把整个仓库翻遍也没有缩放/阴影动画，只有换底色（`Overrides.xaml` 的 VisualState）
//!   ⇒ 我们只做**底色加深**，不做缩放、不做描边。
//!
//! ## 侧栏用的横向卡
//!
//! 详情页右栏那种"图在左、字在右"的小卡是另一套度量（`VideoCardControl.xaml:1057-1087` 的
//! `PlayerRecommendVideoCardStyle`：高 **104**、缩略图列 **148**、封面 `Margin=6`、圆角 4、
//! 文字列 `Padding=12,4,12,8`、行距 6），所以单独给一个 [`horizontal`]，
//! 不要拿竖卡硬塞进 360 宽的栏里。
//!
//! ## 封面为什么不需要我们自己写下载器
//!
//! gpui 0.3.9 的 `ImageAssetLoader` 见到 URI 就走 `App::http_client()` 抓取并自带缓存与在途合并
//! （`src/elements/img.rs:619-656`）。前提是启动时装了真 HttpClient（`src/main.rs`），
//! 且地址必须先过 [`cover::for_display`](crate::ui::cover) —— 接口给的往往是 `//i0.hdslb.com/…`，
//! gpui 判 URL 用 `Url::parse`，那种写法会被当成内置资源，图永远不出来（第 4.5 节）。

// `gpui_kit::*` 的 glob 已经把 `Styled`/`StyledImage` 带进来（第三轮 CI 证明 `.object_fit`
// 无需额外导入即可编译），所以这里不再单独引入 trait。
// 但 `when` 来自 `FluentBuilder`，它只在 gpui 的 prelude 里，glob 带不进来，要显式导入。
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::ui::cover;
use crate::ui::palette;

/// 卡片数据。字段都是已格式化的展示文本，渲染路径里不做网络请求或数字格式化。
#[derive(Clone, Debug, Default)]
pub struct VideoCardData {
    /// 稿件 bvid，用作列表项的稳定 key 与详情页入参。
    pub bvid: SharedString,
    /// 标题（已在 api 层剥掉高亮标签）。
    pub title: SharedString,
    /// UP 主名。
    pub author: SharedString,
    /// 时长，B 站给的就是 `mm:ss`。
    pub duration: SharedString,
    /// 播放量的中文化文本（如 "12.3万"）。
    pub play_text: SharedString,
    /// 封面 URL；为空时画占位块，避免向 gpui 提交一个必然失败的资源请求。
    pub cover_url: SharedString,
    /// 第三行（历史用"今天看过"、收藏用文件夹名等）。空则不占行。
    pub extra: SharedString,
}

/// 逻辑 232×122 的封面在 2× 缩放下该向图床要的尺寸。
///
/// 参考项目那边写的是 `DecodeWidth="600"`（`VideoCardControl.xaml:22`），我们按 2× 取 464
/// 已经够清晰又省一半字节：原图一张 541KB，一页 30 张能把"有没有图"拖成"图迟迟不来"（第 4.5 节）。
const COVER_DEVICE_WIDTH: u32 = 464;
const COVER_DEVICE_HEIGHT: u32 = 244;

/// 侧栏横卡的缩略图（逻辑 136×92 = 148 列减掉左右 margin 6、104 高减掉上下 margin 6）。
const SIDE_DEVICE_WIDTH: u32 = 272;
const SIDE_DEVICE_HEIGHT: u32 = 184;

/// 竖卡网格：`flex_wrap` + 20 的间距 + 容器上下 16 的外边距。
///
/// 收的是**已经装配好的项**（页面用 [`clickable`] 或 [`render`] 包好再交进来）：
/// 能不能点、点了发给谁，是页面的事，组件层不猜。每项的宽度由 [`container`] 固定，
/// 所以 `flex_wrap` 换行后的观感就和 `UniformGridLayout` 一致。
pub fn grid(items: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_wrap()
        .gap(px(palette::CARD_GAP))
        .py(px(palette::BODY_PADDING_Y))
        .children(items)
        .into_any_element()
}

/// 渲染一张竖卡（232×238）。不可点的场合（例如还没接上点击的历史项）走这个。
pub fn render(data: VideoCardData) -> impl IntoElement {
    container().bg(palette::card_background()).child(inner(data))
}

/// 卡片的容器：只放尺寸与"外框"性质（圆角 + 裁切），底色由调用方给。
///
/// 为什么单独拆出来：**hover 必须打在画底色的那个元素上**。gpui 的 hover 改的是自己的 style，
/// 如果底色在外层容器上、hover 加在最外层，被子元素铺满的那层颜色会把它盖住，
/// 于是"鼠标变深"这件事在真机上完全看不见。
fn container() -> Div {
    div()
        .w(px(palette::CARD_WIDTH))
        .h(px(palette::CARD_HEIGHT))
        .flex()
        .flex_col()
        .overflow_hidden()
        .rounded(px(palette::RADIUS))
        // 网格是 flex 行，不给这条就会被压到不足 232 宽（封面比例立刻难看）。
        .flex_shrink_0()
}

/// 卡片内容：封面（232×122）+ 文字区（116）。
///
/// **行序是"UP 主 → 标题 → 统计"**，这不是随手排的：`VideoCardControl.xaml:53-64` 那一列
/// 三行是 `Auto`/`*`/`Auto`，第一行就是 12px Tertiary 的 UP 主，标题在中间那行占满剩余高度，
/// 播放量贴在最后一行。把标题放最上面会一眼就不像参考项目。
fn inner(data: VideoCardData) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(cover_box(
            &data.cover_url,
            &data.duration,
            px(palette::CARD_COVER_HEIGHT),
            COVER_DEVICE_WIDTH,
            COVER_DEVICE_HEIGHT,
        ))
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(palette::CARD_TEXT_ROW_GAP))
                .pt(px(palette::CARD_TEXT_PADDING_TOP))
                .px(px(palette::CARD_TEXT_PADDING_X))
                .pb(px(palette::CARD_TEXT_PADDING_X))
                .child(meta_line(data.author))
                // 标题：Body(14) 两行，占掉中间那一档（`RowDefinition Height="*"`）。
                .child(
                    div()
                        .flex_1()
                        .w_full()
                        .text_size(px(palette::FONT_BODY))
                        .line_height(px(palette::LINE_HEIGHT_BODY))
                        .text_color(palette::text_primary())
                        .line_clamp(2)
                        .child(data.title),
                )
                // 统计行：12px Tertiary，项间距 12（`StackPanel Spacing="12"`）。
                .child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .gap(px(palette::STAT_ITEM_GAP))
                        .text_size(px(palette::FONT_CAPTION))
                        .text_color(palette::text_tertiary())
                        .child(data.play_text)
                        .when(!data.extra.is_empty(), |this| {
                            this.child(div().child(data.extra))
                        }),
                ),
        )
}

/// 侧栏用的横向卡（104 高、缩略图列 148），相关推荐与播放列表都是这个形状。
pub fn horizontal(data: VideoCardData) -> impl IntoElement {
    horizontal_shell()
        .bg(palette::card_background())
        .child(horizontal_inner(data))
}

/// 横向卡容器：高 104、圆角 4（`VideoCardControl.xaml:1057-1060` 的
/// `CardControl Height=104` 与 148 宽的缩略图列）。
fn horizontal_shell() -> Div {
    div()
        .flex()
        .w_full()
        .h(px(palette::SIDE_CARD_HEIGHT))
        .flex_shrink_0()
        .overflow_hidden()
        .rounded(px(palette::RADIUS))
        .cursor_pointer()
}

/// 横向卡内容：左列是 148 宽、内边距 6 的封面位（于是图 136×92），右列同样"UP 主/标题/统计"。
fn horizontal_inner(data: VideoCardData) -> impl IntoElement {
    div()
        .flex()
        .w_full()
        .h_full()
        .child(
            div()
                .w(px(palette::SIDE_THUMB_COLUMN_WIDTH))
                .h_full()
                .flex_shrink_0()
                .p(px(palette::SIDE_THUMB_MARGIN))
                .child(cover_box(
                    &data.cover_url,
                    &data.duration,
                    px(palette::SIDE_COVER_HEIGHT),
                    SIDE_DEVICE_WIDTH,
                    SIDE_DEVICE_HEIGHT,
                )),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(palette::SIDE_TEXT_ROW_GAP))
                .pt(px(palette::SIDE_TEXT_PADDING_TOP))
                .px(px(palette::SIDE_TEXT_PADDING_X))
                .pb(px(palette::SIDE_TEXT_PADDING_BOTTOM))
                .overflow_hidden()
                .child(meta_line(data.author))
                .child(
                    div()
                        .flex_1()
                        .w_full()
                        .text_size(px(palette::FONT_BODY))
                        .line_height(px(palette::LINE_HEIGHT_BODY))
                        .text_color(palette::text_primary())
                        .line_clamp(2)
                        .child(data.title),
                )
                .child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .text_size(px(palette::FONT_CAPTION))
                        .text_color(palette::text_tertiary())
                        .child(data.play_text),
                ),
        )
}

/// 图文（opus）条目：**竖排一张卡**——封面、正文、点赞。空间页的"图文"页签用。
///
/// 为什么不复用 [`horizontal`] 或 [`render`]：接口**不给标题、也不给时间**，`stat.like`
/// 已经是字符串（`ref/bpi-rs/src/opus/space.rs:29-40`）。硬套视频卡那三行（UP 主/标题/播放量）
/// 会留下两个空档，看着像坏了。形状照 `PersonalVideoMomentPresenter.xaml:14-20`：
/// `Padding=12`、`RowSpacing=12`，行是"发布者 / 封面 `*` / 正文 / 操作"，我们只有三档。
///
/// `ratio` 是封面宽除以高（接口自己给了 `cover.width/height`，见
/// [`crate::api::space::cover_ratio`]）：**图文的图有横有竖**，写死 16:9 会把竖图裁掉一半。
pub fn post_card(
    text: SharedString,
    like: SharedString,
    cover_url: SharedString,
    ratio: f32,
) -> AnyElement {
    let card = div()
        .flex()
        .flex_col()
        .w_full()
        .gap(px(palette::STAT_ITEM_GAP))
        .p(px(palette::COMMENT_ROW_PADDING))
        .rounded(px(palette::RADIUS))
        .bg(palette::card_background())
        .overflow_hidden();

    // 没封面就整档不占位（接口给的是 Option，画一个空灰块比少一张图更难看）。
    let card = if cover_url.is_empty() {
        card
    } else {
        card.child(
            div()
                .w_full()
                .aspect_ratio(ratio)
                .overflow_hidden()
                .rounded(px(palette::RADIUS))
                .child(
                    img(SharedString::from(cover::for_display(
                        cover_url.as_str(),
                        SIDE_DEVICE_WIDTH,
                        SIDE_DEVICE_HEIGHT,
                    )))
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .with_fallback(|| {
                        div()
                            .size_full()
                            .bg(palette::card_background_elevated())
                            .into_any_element()
                    }),
                ),
        )
    };

    card.child(
        div()
            .w_full()
            .text_size(px(palette::FONT_BODY))
            .line_height(px(palette::LINE_HEIGHT_BODY))
            .text_color(palette::text_primary())
            .child(text),
    )
    .when(!like.is_empty(), |this| {
        this.child(
            div()
                .text_size(px(palette::FONT_CAPTION))
                .text_color(palette::text_tertiary())
                .child(format!("{like} 点赞")),
        )
    })
    .into_any_element()
}

/// 只有一行的元信息（UP 主名这种），12px + Tertiary 灰 + 单行省略。
fn meta_line(text: SharedString) -> impl IntoElement {
    div()
        .w_full()
        .text_size(px(palette::FONT_CAPTION))
        .text_color(palette::text_tertiary())
        .whitespace_nowrap()
        .text_ellipsis()
        .overflow_hidden()
        .child(text)
}

/// 把竖卡包成可点击的一项（点搜索结果、最近观看、相关推荐都走它）。
///
/// 三个必须注意的点，都是 gpui 的机制：
/// - **要有 id**：交互状态挂在元素 id 上，没有 `.id()` 就调不到 `on_click`，也调不到 `hover`
///   （和 `overflow_y_scroll()` 同一类坑，见 docs/architecture.md 第 2 节）；
///   id 用 bvid，列表里不会出现两项同 id 把点击串到别处。
/// - **hover 与底色必须在同一个元素上**：gpui 的 hover 改的是自己的 style，被子元素铺满的
///   那层颜色会把它盖住，"鼠标变深"在真机上就完全看不见了。所以这里不加外层包装，
///   直接给卡片容器加 `.id()/.on_click()/.hover()`。
/// - **回调用 gpui 原本的形状**（`&ClickEvent, &mut Window, &mut App`），由调用方用
///   `cx.listener(...)` 生成。这样组件层不需要知道任何页面类型。
pub fn clickable(
    data: VideoCardData,
    on_open: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id = format!("video-card-{}", data.bvid.as_str());
    let card = inner(data);
    // 顺序照 gpui 自己的写法（`window/prompts.rs:139-146`）：先样式与 children，再 `.id()`，
    // 最后 `.on_click()`/`.hover()`——它们只存在于 `Stateful<Div>` 上。
    container()
        .bg(palette::card_background())
        .cursor_pointer()
        .child(card)
        .id(id)
        .on_click(on_open)
        // 只加深底色：Bili.Copilot 的 hover 就是换底色，不做缩放、也不做阴影（它没有阴影可用）。
        .hover(|style| style.bg(palette::card_background_hover()))
}

/// 横向卡的可点击版本（详情页右栏的相关推荐用）。
pub fn horizontal_clickable(
    data: VideoCardData,
    on_open: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    // id 里带标题哈希不了，就用"前缀 + bvid + 时长"：同一稿件的相关推荐里不会出现两项完全同名。
    let id = format!(
        "related-card-{}-{}",
        data.bvid.as_str(),
        data.duration.as_str()
    );
    let content = horizontal_inner(data);
    horizontal_shell()
        .bg(palette::card_background())
        .child(content)
        .id(id)
        .on_click(on_open)
        .hover(|style| style.bg(palette::card_background_hover()))
}

/// 封面区域：有 URL 就走 gpui 的 URI 图片加载，没有就画占位块。
///
/// 时长角标用绝对定位压在封面右下角，这是 Bili.Copilot 的读法（离边 8、内边距 4）。
fn cover_box(
    cover_url: &SharedString,
    duration: &SharedString,
    height: Pixels,
    device_width: u32,
    device_height: u32,
) -> impl IntoElement {
    let box_base = div()
        .w_full()
        .h(height)
        .flex_shrink_0()
        .overflow_hidden()
        .relative();

    if cover_url.is_empty() {
        return box_base
            .bg(palette::card_background_elevated())
            .child(badge_or_nothing(duration))
            .into_any_element();
    }

    let source = cover::for_display(cover_url.as_str(), device_width, device_height);
    box_base
        .child(
            img(SharedString::from(source))
                .size_full()
                .object_fit(ObjectFit::Cover)
                // 加载失败时 gpui 只调用这里的兜底视图，界面不会自己画占位；
                // 给一块深色底，"图挂了"和"还没加载完"在真机上就能一眼分开。
                .with_fallback(|| {
                    div()
                        .size_full()
                        .bg(palette::card_background_elevated())
                        .into_any_element()
                }),
        )
        .child(badge_or_nothing(duration))
        .into_any_element()
}

/// 时长角标：没有时长就不画（图文条目没有时长，画一个空黑块会像个 bug）。
fn badge_or_nothing(duration: &SharedString) -> AnyElement {
    if duration.is_empty() {
        return div().into_any_element();
    }
    badge(duration)
}

/// 右下角的时长角标（`VideoCardControl.xaml:37-50`：`Margin=8`、`Padding=4`、黑 60%、白字 12）。
///
/// 返回类型写 `AnyElement` 而不是 `impl IntoElement`：上面那个 `badge_or_nothing` 要在
/// "空 div" 和"角标"之间二选一，opaque 类型是没法当 `AnyElement` 直接返回的。
fn badge(duration: &SharedString) -> AnyElement {
    div()
        .absolute()
        .right(px(palette::BADGE_INSET))
        .bottom(px(palette::BADGE_INSET))
        .px(px(palette::BADGE_PADDING_X))
        .text_size(px(palette::FONT_CAPTION))
        .text_color(palette::text_on_badge())
        .rounded(px(palette::RADIUS))
        .bg(palette::badge_background())
        .child(duration.clone())
        .into_any_element()
}

/// 卡片列表的便捷构造：把 api 层的 DTO 变成带 gpui 类型的展示数据。
///
/// 放在这里是为了让 `pages/` 只做装配，不承担字段搬运的工作。
pub fn cards_from(hits: Vec<crate::api::search::SearchHit>) -> Vec<VideoCardData> {
    hits
        .into_iter()
        .map(|hit| VideoCardData {
            bvid: hit.bvid.into(),
            title: hit.title.into(),
            author: hit.author.into(),
            duration: hit.duration.into(),
            play_text: hit.play_text.into(),
            cover_url: hit.cover_url.into(),
            extra: SharedString::default(),
        })
        .collect()
}
