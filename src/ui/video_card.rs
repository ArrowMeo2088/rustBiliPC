//! 视频卡片：搜索结果列表的基本单元。
//!
//! 尺寸参考 `ref/Bili.Copilot`（`App.xaml` 的 `VideoLayout`：卡片 232×238、间距 20；
//! `VideoCardControl.xaml`：封面 `CornerRadius=4`、时长角标黑底 60% 不透明度 + `Margin=8 Padding=4`）。
//! 那是 WinUI/XAML，不能直接搬，这里只按同一套度量在 gpui 上重画；
//! 现阶段用横向列表而不是网格，网格留到 P5 打磨阶段。
//!
//! 封面**不需要我们自己写下载器**：gpui 0.3.9 的 `ImageAssetLoader` 见到 URI 就走
//! `App::http_client()` 抓取并自带缓存与在途合并（`src/elements/img.rs:619-656`）。
//! 前提是启动时装了真的 HttpClient —— 见 `src/main.rs` 与 `docs/architecture.md` 第 4.5 节。

// `gpui_kit::*` 的 glob 已经把 `Styled`/`StyledImage` 带进来（第三轮 CI 证明 `.object_fit`
// 无需额外导入即可编译），所以这里不再单独引入 trait。
use gpui_kit::*;

use crate::ui::cover;

/// 卡片数据。字段都是已格式化的展示文本，渲染路径里不做网络请求或数字格式化。
#[derive(Clone, Debug, Default)]
pub struct VideoCardData {
    /// 稿件 bvid，用作列表项的稳定 key 与详情页入参。
    pub bvid: SharedString,
    /// 标题（已在 api 层剥掉高亮标签）。
    pub title: SharedString,
    /// UP 主名。
    pub author: SharedString,
    /// 时长，B 站返回的就是 `mm:ss`。
    pub duration: SharedString,
    /// 播放量的中文化文本（如 "12.3万"）。
    pub play_text: SharedString,
    /// 封面 URL；为空时画占位块，避免向 gpui 提交一个必然失败的资源请求。
    pub cover_url: SharedString,
}

/// 封面尺寸（16:9）。
const COVER_WIDTH: f32 = 160.;
const COVER_HEIGHT: f32 = 90.;

/// 向图床要的那份封面的**设备像素**尺寸：逻辑 160×90 在 2× 缩放下就是 320×180，
/// 再大就是白下载（原图一张 541KB，一页 30 张能把"有没有图"拖成"图迟迟不来"）。
const COVER_DEVICE_WIDTH: u32 = 320;
const COVER_DEVICE_HEIGHT: u32 = 180;

/// 渲染一张横向卡片。
///
/// 用 `div().flex()`（默认行方向）而不是 `h_flex()`：已发布的 gpui-kit 0.8.0 没有那个自由函数。
pub fn render(data: VideoCardData) -> impl IntoElement {
    div()
        .flex()
        .w_full()
        .flex_shrink_0()
        .gap_2()
        .p_2()
        .items_start()
        .child(cover_box(&data.cover_url, &data.duration))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .gap_1()
                .child(div().child(data.title))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .text_sm()
                        .child(data.author)
                        .child(data.play_text),
                ),
        )
}

/// 封面区域：有 URL 就走 gpui 的 URI 图片加载，没有就画占位块。
///
/// 时长角标用绝对定位压在封面右下角，这是 Bili.Copilot 的读法。
fn cover_box(cover_url: &SharedString, duration: &SharedString) -> impl IntoElement {
    let box_base = div()
        .w(px(COVER_WIDTH))
        .h(px(COVER_HEIGHT))
        .flex_shrink_0()
        .overflow_hidden()
        .relative();

    if cover_url.is_empty() {
        return box_base
            .bg(rgb(0x2A2A2A))
            .child(badge(duration))
            .into_any_element();
    }

    // 交给图床的是"补全协议 + 按这个框的尺寸缩放"之后的地址：接口给的往往是
    // `//i0.hdslb.com/…`，gpui 判 URL 靠 `Url::parse`，那种写法会被当成内置资源，
    // 图永远不出来（第十五轮 debug.log 里满屏 `Embedded resource not found` 就是这个）。
    // 详见 `ui/cover.rs`。
    let source = cover::for_display(cover_url.as_str(), COVER_DEVICE_WIDTH, COVER_DEVICE_HEIGHT);
    box_base
        .child(
            img(SharedString::from(source))
                .size_full()
                .object_fit(ObjectFit::Cover)
                // 加载失败时 gpui 只调用这里的兜底视图，界面不会自己画占位；
                // 给一块深色底，"图挂了"和"还没加载完"在真机上就能一眼分开。
                .with_fallback(|| div().size_full().bg(rgb(0x1A1A1A)).into_any_element()),
        )
        .child(badge(duration))
        .into_any_element()
}

/// 右下角的时长角标。
fn badge(duration: &SharedString) -> impl IntoElement {
    div()
        .absolute()
        // 用 1（4px）而不是 0p5：gpui 没有生成 `right_0p5`/`bottom_0p5`。
        .right_1()
        .bottom_1()
        .px_1()
        .text_xs()
        .bg(rgb(0x000000).opacity(0.6))
        .child(duration.clone())
        .into_any_element()
}

/// 用于无结果/未搜索状态的提示元素。
pub fn notice(text: impl Into<SharedString>) -> impl IntoElement {
    div().p_4().text_sm().child(text.into()).into_any_element()
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
        })
        .collect()
}
