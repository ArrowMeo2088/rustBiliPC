//! 颜色与度量令牌：把 `ref/Bili.Copilot`（WinUI3）那套视觉翻译成 gpui 能给的固定值。
//!
//! ## 为什么自己定色板，不用 kit 的语义色
//!
//! Bili.Copilot 全项目几乎不硬写颜色，它走 WinUI 的 `ThemeResource` 键
//! （`TextFillColorPrimary/Secondary/Tertiary`、`CardBackgroundFillColorDefault`、
//! `DividerStrokeColorDefault`、`SolidBackgroundFillColorBase`）。gpui 没有主题键的等价物，
//! 而 gpui-kit 的语义色也不完全对齐这套层级。所以这里**把那几个键在暗色主题下的实际取值
//! 抄成常量**，全项目只从这里出，避免每个页面各写一个 `0x2A2A2A`。
//!
//! 明确只做暗色：底 `#202020` 配半透明白文字，这正是 Bili.Copilot 暗色下的观感。
//! 想要亮色时再补一组值，别在页面里加 `if is_dark` 的分支。
//!
//! ## WinUI 拿不到、我们也不装的东西
//!
//! Acrylic/Mica 材质、`ItemContainer` 的选中视觉、`RevealBrush`、图标字库
//! （`Segoe Fluent Icons`）都不存在。参考项目自己给出的替代是：**用不透明纯色 + 1px 描边 +
//! 圆角 + 字号差 + 三档灰**表达同样的层级——例如它的时长角标就是一块黑 60% 的纯色
//! （`VideoCardControl.xaml:44`），hover 只换底色不做缩放。
use gpui_kit::{Rgba, rgb, rgba};

/// B 站品牌粉。Bili.Copilot 用的是**系统 accent**（全仓搜不到 `FB7299`），
/// 所以这是我们的选择，不是从它那里抄的。
pub const ACCENT: u32 = 0xFB7299;

/// 主按钮的悬停/按下两档。
///
/// **不是抄来的数字**：WinUI 的 accent 有三档派生色，我们手上只有一支粉，
/// 就按"悬停提亮一档、按下压暗一档"各取一档明度变化。
/// 亮度过渡本身不重要，重要的是**别拿同一支粉当 hover**——那样鼠标移动看不出反馈。
pub const ACCENT_HOVER: u32 = 0xFC8AA9;
pub const ACCENT_ACTIVE: u32 = 0xE4607F;

/// 页面底色 = WinUI `SolidBackgroundFillColorBase`（暗色 `#202020`）。
pub fn page_background() -> Rgba {
    rgb(0x202020)
}

/// 卡片/容器底 = `CardBackgroundFillColorDefault`（暗色约等于白 6% 叠在页面底上）。
///
/// 注意 `rgba` 的字节序是 **RRGGBBAA**（`ref/gpui/gpui/src/color.rs:20-23`），
/// 半透明白写成 `0xFFFFFF10`，不是 `0x10FFFFFF`。
pub fn card_background() -> Rgba {
    rgba(0xFFFFFF10)
}

/// 再高一层（选中行、输入框底）。
pub fn card_background_elevated() -> Rgba {
    rgba(0xFFFFFF1A)
}

/// hover：只加深底色，不做缩放/阴影（参考项目的 hover 就是换底色）。
pub fn card_background_hover() -> Rgba {
    rgba(0xFFFFFF24)
}

/// 分隔线 = `DividerStrokeColorDefault`，**高度固定 1px**。
pub fn divider() -> Rgba {
    rgba(0xFFFFFF14)
}

/// 主文本 = `TextFillColorPrimary`（暗色白 90%）。
pub fn text_primary() -> Rgba {
    rgba(0xFFFFFFE6)
}

/// 次要文本 = `TextFillColorSecondary`（白 60%）。
pub fn text_secondary() -> Rgba {
    rgba(0xFFFFFF99)
}

/// 辅助文本 = `TextFillColorTertiary`（白 40%）。UP 主名、时长、播放量这类都用它。
pub fn text_tertiary() -> Rgba {
    rgba(0xFFFFFF68)
}

/// 强调色上的文字。`#FB7299` 亮度约 0.72，配白字对比度不足 4.5:1，所以用近黑。
pub fn text_on_accent() -> Rgba {
    rgb(0x1F1F1F)
}

/// "没有描边"的那层透明色。
///
/// 选中行必须**平时就占住 2px 边框**：参考项目的 `PART_SelectionVisual` 是
/// `BorderThickness=2` + `BorderBrush=Transparent`（`Overrides.xaml:195-196`），
/// 选中时只把 Brush 换成选中色（`:233-247`）。这样点选不会让内容跳 2px——
/// 我们如果只在选中那一项加边框，列表一行行抖起来比丑更难受。
pub fn border_none() -> Rgba {
    rgba(0x00000000)
}

/// 时长角标的底：黑 60%，和 `VideoCardControl.xaml:44` 的 `SolidColorBrush Black Opacity .6` 一致。
pub fn badge_background() -> Rgba {
    rgba(0x00000099)
}

/// 角标上的字：那边直接写 `Foreground="White"`（`VideoCardControl.xaml:47`），不是三档灰之一。
pub fn text_on_badge() -> Rgba {
    rgb(0xFFFFFF)
}

/// 品牌粉。
pub fn accent() -> Rgba {
    rgb(ACCENT)
}

/// 点赞这类"红"的语义色 = `SystemFillColorCritical` 的近似。
pub fn critical() -> Rgba {
    rgb(0xF25C6C)
}

// ---- 度量：这些数字全部来自 ref/Bili.Copilot，改之前先去那边看原文（文件:行号写在注释里）----

/// 标题栏高度（`RootLayout.xaml:33` 的第一行是 48）。gpui 这边标题栏归系统，
/// 我们把这个值用在侧栏顶部的应用名块上。
pub const TITLE_BAR_HEIGHT: f32 = 48.;

/// 页头高度（`PopularPage.xaml:18` 是 `RowDefinition Height="56"`，`:30-33` 那条 Border 是 1px 分隔线）。
pub const HEADER_HEIGHT: f32 = 56.;

/// 左侧列表栏宽度（`LayoutPageViewModelBase.cs:41` = 240）。
pub const SIDE_COLUMN_WIDTH: f32 = 240.;

/// 详情页右侧信息栏宽度（`VideoPlayerPanel.xaml:11` 的 `<Grid Width="360">`）。
pub const DETAIL_SIDE_WIDTH: f32 = 360.;

/// 视频卡片：`App.xaml:70-76` 的 `VideoLayout` 是 232×238，列距/行距都是 20。
pub const CARD_WIDTH: f32 = 232.;
pub const CARD_HEIGHT: f32 = 238.;
pub const CARD_GAP: f32 = 20.;

/// 封面高：卡片减掉文字区 116（`VideoCardControl.xaml:16-19` 的行定义 `*` + `116`），
/// 也就是 232×122，**不是 16:9**（比例约 1.90:1）。
pub const CARD_COVER_HEIGHT: f32 = 122.;

/// 卡片文字区内边距（`VideoCardControl.xaml:51-54`：`16,12,16,16`，行距 8）。
pub const CARD_TEXT_PADDING_X: f32 = 16.;
pub const CARD_TEXT_PADDING_TOP: f32 = 12.;
pub const CARD_TEXT_ROW_GAP: f32 = 8.;

/// 侧栏横卡（`VideoCardControl.xaml:1057-1087` 的 `PlayerRecommendVideoCardStyle`）：
/// 整卡高 **104**、缩略图列宽 **148**、封面在该列里再留 `Margin=6`（于是图 136×92）、
/// 文字列 `Padding=12,4,12,8`、行距 **6**。列内顺序也是"UP 主 / 标题 / 统计"，
/// 与竖卡同一套读法。
pub const SIDE_CARD_HEIGHT: f32 = 104.;
pub const SIDE_THUMB_COLUMN_WIDTH: f32 = 148.;
pub const SIDE_THUMB_MARGIN: f32 = 6.;
pub const SIDE_COVER_WIDTH: f32 = 136.;
pub const SIDE_COVER_HEIGHT: f32 = 92.;
pub const SIDE_TEXT_PADDING_X: f32 = 12.;
pub const SIDE_TEXT_PADDING_TOP: f32 = 4.;
pub const SIDE_TEXT_PADDING_BOTTOM: f32 = 8.;
pub const SIDE_TEXT_ROW_GAP: f32 = 6.;

/// 圆角：它用的控件默认是 4（封面是 `4,4,0,0`，只有上面两个角；角标用
/// `{StaticResource ControlCornerRadius}`，WinUI 默认也是 4，见 `VideoCardControl.xaml:42`）。
pub const RADIUS: f32 = 4.;

/// 分隔线厚度（`PopularPage.xaml:30-33` 那条 Border 就是 1px）。
pub const DIVIDER_HEIGHT: f32 = 1.;

/// 标题这类多行文本的行高。
///
/// 参考项目没写行高（它用 WinUI 的默认），gpui 的默认行距对 14px 中文偏紧、两行标题会挤在一起，
/// 所以这里定一个固定值 20，**不是抄来的数字**，真要调就在这一处调。
pub const LINE_HEIGHT_BODY: f32 = 20.;

/// 时长角标离封面边缘 8、内边距 4（`VideoCardControl.xaml:37-50`）。
pub const BADGE_INSET: f32 = 8.;
pub const BADGE_PADDING_X: f32 = 4.;

/// 头像：UP 主行 44（`UpSectionControl.xaml:21-33`）、资料卡 48（`AccountCard.xaml:13-51`）。
/// 那边写的是 `Width/Height=48` + `CornerRadius="24"` + `DecodeWidth="100"`
/// ——即"直径即圆角"，而且**取图按 2× 边长要**（48 的头像解 100 宽），
/// 所以 [`avatar`](crate::ui::layout::avatar) 直接圆角=尺寸/2、向图床要 2× 设备像素。
pub const AVATAR_UP: f32 = 44.;
pub const AVATAR_ACCOUNT: f32 = 48.;

/// 评论区一行的度量（`CommentItemControl.xaml:21-46`）：
/// 整行是 `Margin="0,0,0,4"` + `Padding="12"` + 卡片底 + 1px 同色描边 + `ControlCornerRadius`，
/// 头像与文字列的列距 12，头像 **40**（`CornerRadius="20"`、`DecodeWidth="80"`）。
pub const AVATAR_COMMENT: f32 = 40.;
pub const COMMENT_ROW_GAP: f32 = 4.;
pub const COMMENT_ROW_PADDING: f32 = 12.;
pub const COMMENT_COLUMN_GAP: f32 = 12.;

/// 分 P / 侧栏列表项的行高与内边距（`VideoPartsSection.xaml:18-21`：`MinHeight 40`、`12,8`、
/// 列距 8，每行都带卡片底色 `CardBackgroundFillColorDefaultBrush`）。
pub const LIST_ROW_MIN_HEIGHT: f32 = 40.;
pub const LIST_ROW_PADDING_X: f32 = 12.;
pub const LIST_ROW_PADDING_Y: f32 = 8.;

/// 列表项之间的间距（`VideoPartsSection.xaml:51` 与 `PopularSideBody.xaml:52` 都是 `Spacing="8"`）。
pub const LIST_GAP: f32 = 8.;

/// 网格/列表容器的上下外边距（`PopularMainBody.xaml:37` 的 `Margin="0,16"`）。
pub const BODY_PADDING_Y: f32 = 16.;

/// 页头里搜索框的宽度。参考项目给的是 `MinWidth=320 / MaxWidth=500`
/// （`RootLayout.xaml:74-80`），gpui 这边没有"区间宽度"的写法，取 460 落在那个带里。
pub const SEARCH_BOX_WIDTH: f32 = 460.;

/// 一行里若干项的间距（`AccountCard.xaml:14` 的 `ColumnSpacing="12"`）。
pub const STAT_ITEM_GAP: f32 = 12.;

/// 卡内竖排各行之间的间距（`AccountCard.xaml:39` 的 `StackPanel Spacing="4"`）。
pub const CARD_LINE_GAP: f32 = 4.;

/// 字号层级：它基本只用 10/11/12/14/16 五档（Caption=12、Body=14、Subtitle=16），
/// 层级差靠 **字号 + 三档灰 + Bold** 表达，不靠颜色种类。
pub const FONT_CAPTION: f32 = 12.;
pub const FONT_BODY: f32 = 14.;
pub const FONT_TITLE: f32 = 16.;
