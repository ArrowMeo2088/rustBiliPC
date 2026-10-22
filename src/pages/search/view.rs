//! 搜索页（阶段一）。
//!
//! P0 只提供骨架：顶栏输入框 + 假数据卡片列表 + 底部环境自检。
//! 目的是让"框架能不能跑起来"这件事在接入真实接口之前就有结论，
//! 所以这里刻意**不**发任何网络请求。
//!
//! P1 要补的三件事，都记在 `docs/architecture.md` 的阶段表里：
//! 1. `InputEvent::Enter` / 搜索按钮 → [`crate::api::spawn_on_api_runtime`] 发请求；
//! 2. `bpi_rs::search` 的 `Video` → [`VideoCardData`] 的转换（标题要剥 `<em>` 标签）；
//! 3. 封面网络图 + 缓存（gpui-kit 不提供，需自研）。

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::*;

use crate::ui::video_card::{self, VideoCardData};

/// 搜索页根视图。
pub struct SearchPage {
    /// 关键词输入框的状态。kit 的 `InputState` 内部是 Rope，中文 IME 合成已可用。
    query: Entity<InputState>,
    /// 当前展示的结果。P0 是假数据，P1 换成接口返回。
    results: Vec<VideoCardData>,
    /// 底部环境自检文本，构造时算一次，避免每帧读盘。
    status_line: SharedString,
    /// 订阅要随实体存活，命名以 `_` 开头表示"只需保活，不需读取"。
    _subscriptions: Vec<Subscription>,
}

impl SearchPage {
    /// 创建搜索页。需要 `window` 是因为 kit 的 `InputState::new` 要读窗口级状态。
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("搜索视频（P0 骨架：搜索接线在 P1）")
        });

        Self {
            query,
            results: video_card::placeholder_cards(),
            status_line: Self::build_status_line(),
            _subscriptions: Vec::new(),
        }
    }

    /// 组装底部自检信息：凭据文件在不在、mpv 找不找得到。
    ///
    /// 这两项决定了后面阶段能不能立刻跑起来，所以从 P0 就摆在明面上。
    fn build_status_line() -> SharedString {
        let credential = if crate::store::credentials::SavedCredential::load().is_some() {
            "已保存"
        } else {
            "未登录"
        };
        let mpv = if crate::player::find_mpv_in_path().is_some() {
            "已找到"
        } else {
            "PATH 中未找到"
        };

        format!(
            "rustBiliPC v{} · 凭据: {credential} ({}) · mpv: {mpv}",
            crate::VERSION,
            crate::store::credentials::SavedCredential::path().display()
        )
        .into()
    }
}

impl Render for SearchPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // 注意：已发布的 gpui-kit 0.8.0 没有 `v_flex()`/`h_flex()` 自由函数（那是 ref/gpui-kit
        // 本地 main 才有的），所以这里用 `div().flex()` + `.flex_col()` 表达同一件事。
        div()
            .flex()
            .flex_col()
            .size_full()
            // 顶栏：输入框占满剩余宽度，右侧留给以后的排序/分页控件。
            .child(
                div()
                    .flex()
                    .w_full()
                    .gap_2()
                    .p_4()
                    .items_center()
                    .child(div().flex_1().child(Input::new(&self.query))),
            )
            // 结果区：滚动容器必须先 `.id()`——`overflow_*` 是 `Stateful<Div>` 上的方法。
            // P1 换成虚拟列表（结果可达上千条）。
            .child(
                div()
                    .id("search-results")
                    .flex_1()
                    .w_full()
                    .overflow_y_scroll()
                    .children(self.results.iter().cloned().map(video_card::render)),
            )
            .child(
                div()
                    .w_full()
                    .p_2()
                    .text_sm()
                    .child(self.status_line.clone()),
            )
    }
}
