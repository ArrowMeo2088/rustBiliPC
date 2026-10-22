//! 视频卡片：搜索结果列表的基本单元。
//!
//! 尺寸参考 `ref/Bili.Copilot`（`App.xaml` 的 `VideoLayout`：卡片 232×238、间距 20；
//! `VideoCardControl.xaml`：封面圆角 4、时长角标黑底 60% 不透明度 + `Margin=8 Padding=4`）。
//! 那是 WinUI/XAML，不能直接搬，这里只按同一套度量在 gpui 上重画。
//!
//! P0 阶段封面是纯色占位块——网络图片加载器还没有（gpui-kit 不提供，需自研），
//! 放到 P1 与真实搜索接口一起做。

use gpui_kit::*;

/// 卡片数据。字段都是已格式化的展示文本，避免在渲染路径里做网络请求或数字格式化。
#[derive(Clone, Debug)]
pub struct VideoCardData {
    /// 稿件 bvid，用作列表项的稳定 key 与详情页入参。
    pub bvid: SharedString,
    /// 标题（B 站返回的标题带 `<em class="keyword">` 高亮标签，入库前要剥掉，见 P1）。
    pub title: SharedString,
    /// UP 主名。
    pub author: SharedString,
    /// 时长，bpi-rs 返回的就是 `mm:ss` 字符串，直接显示。
    pub duration: SharedString,
    /// 播放量的中文化文本（如 "12.3万"）。
    pub play_text: SharedString,
}

/// 封面占位块的宽高（16:9）。
const COVER_WIDTH: f32 = 160.;
const COVER_HEIGHT: f32 = 90.;

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
        .items_center()
        // 封面占位：P1 换成网络图 + 右下角时长角标。
        .child(
            div()
                .w(px(COVER_WIDTH))
                .h(px(COVER_HEIGHT))
                .flex_shrink_0()
                .bg(rgb(0x2A2A2A)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .gap_1()
                .child(div().child(data.title))
                .child(div().text_sm().child(data.author)),
        )
        .child(div().flex_shrink_0().text_sm().child(data.duration))
        .child(div().flex_shrink_0().text_sm().child(data.play_text))
}

/// 用于骨架阶段的一组假数据，方便在没有网络的情况下验证布局与滚动。
pub fn placeholder_cards() -> Vec<VideoCardData> {
    (1..=8)
        .map(|index| VideoCardData {
            bvid: format!("BV00000000{index:02}").into(),
            title: format!("P0 骨架假数据卡片 {index}（尚未接入搜索接口）").into(),
            author: "rustBiliPC".into(),
            duration: "12:34".into(),
            play_text: "1.0万".into(),
        })
        .collect()
}
