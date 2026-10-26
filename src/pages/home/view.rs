//! 主页（P6 第一步）：B 站首页推荐流。
//!
//! ## 这一页的位置
//!
//! 用户定的清单里"主页"排在最前，所以它也是外壳的**默认页**（登录后进来第一眼就是这里）。
//! 内容只做首页推荐流一条（`api/home.rs` 有接口的形状与两条硬约束）：
//! 卡片网格 + "换一批"，不做分页条——这个接口没有"总页数"，只能一批批换。
//!
//! ## 排版照 Bili.Copilot 的主内容区
//!
//! 页头 56 + 1px 分隔线、内容区用同一套 232×238 卡片网格（`App.xaml:70-76` 的 `VideoLayout`）、
//! 上下外边距 16（`PopularMainBody.xaml:37`）。页头右侧的操作件对应它的
//! `PopularMainHeader.xaml:38-49`（那颗"刷新"按钮）——我们把它做成"换一批"，
//! 因为推荐流的刷新语义就是换下一批而不是重取同一批。
//!
//! ## 懒加载
//!
//! 与个人中心同一套做法：外壳构造时就建好实体，但**第一次被看到才发请求**
//! （[`HomePage::ensure_loaded`]），返回来时只换列表，不重建实体。
//! 请求序号 `seq` 保证"连点两次换一批"时旧结果自己让位（第 4.9 节）。

use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;

use crate::api::home::{self, HomeHit};
use crate::pages::relogin;
use crate::pages::OpenVideo;
use crate::ui::scroll::DragScroll;
use crate::ui::{layout, palette};
use crate::ui::video_card::{self, VideoCardData};

/// 主页根视图。
pub struct HomePage {
    /// 复用一份客户端；请求要 clone 成 `Arc` 投给 tokio。
    client: Arc<bpi_rs::BpiClient>,
    /// 单指/按住拖动的滚动状态（`ui::scroll::DragScroll`，与下面那个滚动容器一一对应）。
    scroll: DragScroll,
    hits: Vec<HomeHit>,
    /// 加载状态与失败原因（空串 = 正常）。错误必须上界面，不许只进日志（第 4.7 节）。
    note: SharedString,
    /// 请求序号：只有最新那次的结果允许写回界面。
    seq: u32,
    /// 当前是第几批（接口的 `fresh_idx`/`fetch_row`，从 1 开始）。
    page: u32,
    /// 是否已经发起过首轮加载（懒加载用）。
    loaded: bool,
}

impl HomePage {
    /// 建页，但**不**发请求：这一页随外壳一起构造，首屏请求由 `ensure_loaded` 触发。
    pub fn new(client: Arc<bpi_rs::BpiClient>) -> Self {
        Self {
            scroll: DragScroll::default(),
            client,
            hits: Vec::new(),
            note: "".into(),
            seq: 0,
            page: 1,
            loaded: false,
        }
    }

    /// 第一次被看到时加载一次；之后保留已有列表，要更新点"换一批"。
    pub fn ensure_loaded(&mut self, cx: &mut Context<Self>) {
        if self.loaded {
            return;
        }
        self.load(cx);
    }

    /// 重取当前这一批（不推进批次号）。
    ///
    /// 给重登之后刷新用（[`MainShell::reload_after_relogin`](crate::pages::shell::MainShell)）：
    /// 换一批是"用户想看新的"，重登是"刚才那批根本不该失败"，两者不能混用同一个入口。
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    /// 换一批：批次号 +1 后重新取。这一页没有"总页数"，所以永远可以往下换。
    fn next_batch(&mut self, cx: &mut Context<Self>) {
        self.page += 1;
        self.load(cx);
    }

    /// 取当前这一批推荐。
    fn load(&mut self, cx: &mut Context<Self>) {
        self.loaded = true;
        self.seq += 1;
        let seq = self.seq;
        self.note = format!("加载中…（第 {} 批）", self.page).into();
        cx.notify();

        let join = home::fetch_home(Arc::clone(&self.client), self.page);
        // `spawn` 要求整个 future `'static`，所以写回状态用 `upgrade + update`，
        // 闭包里只捕获副本，不借用 self。
        cx.spawn(async move |this, cx| {
            let outcome = match join.await {
                Ok(inner) => inner.map_err(|error| relogin::message(&error)),
                Err(join_error) => Err(crate::app::describe_join_error(join_error)),
            };
            log::debug!("主页：推荐结果已回到前台（请求 #{seq}）");

            let Some(view) = this.upgrade() else {
                log::warn!("主页：等待期间实体已释放，丢弃结果");
                return;
            };
            view.update(cx, |view, cx| {
                if view.seq != seq {
                    log::debug!("主页：请求 #{seq} 已被取代，丢弃结果");
                    return;
                }
                match outcome {
                    Ok(hits) => {
                        let count = hits.len();
                        view.hits = hits;
                        // 接口成功但一条稿件都没有：这句必须写出来，
                        // 否则界面只剩一片空白，看不出是"没内容"还是"没加载"。
                        view.note = if count == 0 {
                            "这一批没有可展示的稿件".into()
                        } else {
                            "".into()
                        };
                    }
                    Err(message) => {
                        view.hits.clear();
                        view.note = format!("取首页推荐失败：{message}").into();
                    }
                }
                cx.notify();
            });
            relogin::flush(cx);
        })
        .detach();
    }

    /// 卡片列表（点一张 = 发同一个 `OpenVideo`，导航由外壳决定）。
    fn cards(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.hits
            .iter()
            .cloned()
            .map(|hit| {
                let card = VideoCardData {
                    bvid: hit.bvid.into(),
                    title: hit.title.into(),
                    author: hit.author.into(),
                    duration: hit.duration.into(),
                    play_text: hit.play_text.into(),
                    cover_url: hit.cover_url.into(),
                    // 推荐理由占第三格：B 站没给就是空串，卡片那一格自动不出现。
                    extra: hit.reason.into(),
                };
                let bvid = card.bvid.as_str().to_string();
                video_card::clickable(
                    card,
                    cx.listener(move |_this, _event, _window, cx| {
                        cx.emit(OpenVideo { bvid: bvid.clone() });
                    }),
                )
                .into_any_element()
            })
            .collect()
    }
}

/// 点主页卡片时通知外壳打开详情页。
impl EventEmitter<OpenVideo> for HomePage {}

impl Render for HomePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 每帧登记一次移动/抬起监听（`Window::on_mouse_event` 只覆盖下一帧，见 `ui/scroll.rs`）。
        self.scroll.watch(window, cx.entity_id());
        // 页头右侧那颗按钮对应参考项目的"刷新"位（`PopularMainHeader.xaml:38-49`）。
        let trailing = div()
            .flex()
            .flex_shrink_0()
            .gap(px(palette::CARD_TEXT_ROW_GAP))
            .child(
                Button::new("home-next")
                    .primary()
                    .label("换一批")
                    .on_click(cx.listener(|this, _event, _window, cx| this.next_batch(cx))),
            )
            .into_any_element();

        let subtitle = if self.hits.is_empty() {
            None
        } else {
            Some(format!("第 {} 批 · {} 条", self.page, self.hits.len()).into())
        };

        let body: AnyElement = if self.note.is_empty() {
            video_card::grid(self.cards(cx))
        } else if self.hits.is_empty() {
            layout::note(self.note.clone())
        } else {
            // 已有列表 + 新的提示（例如换一批失败）：两者都要看得见，不许把列表清空藏起来。
            div()
                .flex()
                .flex_col()
                .w_full()
                .child(
                    div()
                        .w_full()
                        .px(px(palette::CARD_TEXT_PADDING_X))
                        .pt(px(palette::CARD_TEXT_ROW_GAP))
                        .text_size(px(palette::FONT_CAPTION))
                        .text_color(palette::text_secondary())
                        .child(self.note.clone()),
                )
                .child(video_card::grid(self.cards(cx)))
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(layout::header("主页", subtitle, Some(trailing)))
            .child(
                div()
                    .id("home-body")
                    .flex_1()
                    .w_full()
                    .min_w(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(self.scroll.handle())
                    .on_mouse_down(MouseButton::Left, self.scroll.on_press())
                    .px(px(palette::CARD_TEXT_PADDING_X))
                    .child(body),
            )
    }
}
