//! 详情页（P3）。
//!
//! ## 这一页只做到"看得清 + 选得了 P"
//!
//! 主体（标题/封面/UP 主/统计/简介/标签/分P/相关推荐）来自 `api/detail.rs`，字幕列表来自
//! `api/detail::fetch_subtitles`。**播放不在这一轮**：P4 才接 mpv，所以选中的 P 现在只是
//! 高亮 + 记下 cid，界面写明"播放接下一阶段"，不假装能播。
//!
//! ## 为什么字幕要等主体回来才去取
//!
//! `player_info_v2` 的入参需要 `cid`，而 `cid` 在 `video().view()` 的响应里
//! （`VideoView.cid` = 默认分 P）。所以是"主体 → 用它的 cid 再发一路"这条依赖链，不是并发。
//! 字幕失败只影响字幕那一块（原因见 `api/detail.rs` 头注释：`PlayerInfoResponseData`
//! 字段极多且无 `#[serde(default)]`）。
//!
//! ## 导航
//!
//! 相关推荐的卡片复用搜索页那套 `video_card::clickable`，发的还是同一个
//! [`OpenVideo`](crate::pages::OpenVideo) 事件——外壳收到就往栈里再压一层详情页，
//! 所以"相关推荐 → 进去 → 返回"是连续的，页面自己不做导航决定。
//!
//! ## gpui 写法上的两个约束（本文件因此的样子）
//!
//! - 交互方法（`on_click`）只在 `Stateful<Div>` 上有，所以链式顺序固定为
//!   "样式 → child → `.id(..)` → `.on_click(..)`"，与 `gpui/src/window/prompts.rs:139-146` 一致。
//! - `impl IntoElement` 只能有一个返回类型，所以"没拿到主体"与"拿到主体"两条分支
//!   各自转成 `AnyElement`，再交给同一个外层 div。

use std::sync::Arc;

use gpui_kit::*;

use crate::api::detail::{self, SubtitleBrief, VideoDetailInfo};
use crate::pages::OpenVideo;
use crate::ui::cover;
use crate::ui::video_card::{self, VideoCardData};

/// 详情页大图向图床要的尺寸（逻辑 320×180，按 2× 缩放）。
const HERO_DEVICE_WIDTH: u32 = 640;
const HERO_DEVICE_HEIGHT: u32 = 360;

/// 选中的分 P 那一行的底色。
const SELECTED_BG: u32 = 0x232323;

/// 详情页根视图。
pub struct DetailPage {
    client: Arc<bpi_rs::BpiClient>,
    bvid: SharedString,
    detail: Option<VideoDetailInfo>,
    /// 主体的状态：加载中 / 失败原因。空字符串表示一切正常。
    note: SharedString,
    subtitles: Vec<SubtitleBrief>,
    subtitle_note: SharedString,
    /// 当前选中的分 P（`pages` 里的下标）。P4 播放时用它取 cid 与标题。
    selected: usize,
    seq: u32,
}

impl DetailPage {
    /// 建页并发起主体请求。
    pub fn new(client: Arc<bpi_rs::BpiClient>, bvid: String, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            client,
            bvid: bvid.into(),
            detail: None,
            note: "加载中…".into(),
            subtitles: Vec::new(),
            subtitle_note: "字幕加载中…".into(),
            selected: 0,
            seq: 0,
        };
        this.load(cx);
        this
    }

    /// 取主体；主体回来后立刻用它的 cid 追一路字幕列表。
    fn load(&mut self, cx: &mut Context<Self>) {
        self.seq += 1;
        let seq = self.seq;
        self.note = "加载中…".into();
        self.subtitle_note = "字幕加载中…".into();
        self.subtitles.clear();
        cx.notify();

        let join = detail::fetch_detail(Arc::clone(&self.client), self.bvid.to_string());
        cx.spawn(async move |this, cx| {
            let outcome = match join.await {
                Ok(inner) => inner.map_err(|error| error.to_string()),
                Err(join_error) => Err(crate::app::describe_join_error(join_error)),
            };
            log::debug!("详情页：主体结果已回到前台（请求 #{seq}）");

            let Some(view) = this.upgrade() else {
                log::warn!("详情页：等待期间实体已释放，丢弃主体结果");
                return;
            };
            // update 只把"要不要接着取字幕"这个决定传出来，不在里面嵌套 spawn。
            let cid = view.update(cx, |view, cx| {
                if view.seq != seq {
                    log::debug!("详情页：请求 #{seq} 已被取代，丢弃主体结果");
                    return None;
                }
                match outcome {
                    Ok(info) => {
                        let cid = info.first_cid;
                        view.note = "".into();
                        view.selected = 0;
                        view.detail = Some(info);
                        cx.notify();
                        Some(cid)
                    }
                    Err(message) => {
                        view.note = message.into();
                        cx.notify();
                        None
                    }
                }
            });

            let Some(cid) = cid else { return };
            let Some(view) = this.upgrade() else { return };
            view.update(cx, |view, cx| view.load_subtitles(cid, seq, cx));
        })
        .detach();
    }

    /// 取这一 P 的字幕列表。失败只写字幕那一块。
    fn load_subtitles(&mut self, cid: u64, seq: u32, cx: &mut Context<Self>) {
        log::debug!("详情页：发起字幕列表请求（#{seq}）cid={cid}");
        let join = detail::fetch_subtitles(Arc::clone(&self.client), self.bvid.to_string(), cid);
        cx.spawn(async move |this, cx| {
            let outcome = match join.await {
                Ok(inner) => inner.map_err(|error| error.to_string()),
                Err(join_error) => Err(crate::app::describe_join_error(join_error)),
            };
            log::debug!("详情页：字幕结果已回到前台（请求 #{seq}）");

            let Some(view) = this.upgrade() else { return };
            view.update(cx, |view, cx| {
                // 还得是同一轮加载：用户可能已经切到别的稿件了。
                if view.seq != seq {
                    return;
                }
                match outcome {
                    Ok(list) => {
                        view.subtitle_note = if list.is_empty() {
                            "这条稿件没有字幕"
                        } else {
                            ""
                        }
                        .into();
                        view.subtitles = list;
                    }
                    Err(message) => view.subtitle_note = message.into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 相关推荐的卡片：点它 = 让外壳再压一层详情页。
    fn related_cards(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let cards: Vec<VideoCardData> = self
            .detail
            .as_ref()
            .map(|info| {
                info.related
                    .iter()
                    .map(|item| VideoCardData {
                        bvid: item.bvid.clone().into(),
                        title: item.title.clone().into(),
                        author: item.author.clone().into(),
                        duration: item.duration_text.clone().into(),
                        play_text: item.play_text.clone().into(),
                        cover_url: item.cover.clone().into(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        div().flex().flex_col().w_full().children(
            cards.into_iter().map(|card| {
                let bvid = card.bvid.as_str().to_string();
                video_card::clickable(
                    card,
                    cx.listener(move |_this, _event, _window, cx| {
                        cx.emit(OpenVideo { bvid: bvid.clone() });
                    }),
                )
            }),
        )
    }

    /// 分 P（选集）列表；点一行就是选中它。
    fn pages_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pages: Vec<(usize, String, String, u64)> = self
            .detail
            .as_ref()
            .map(|info| {
                info.pages
                    .iter()
                    .enumerate()
                    .map(|(index, page)| {
                        (
                            index,
                            format!("P{} · {}", page.page, page.part),
                            page.duration_text.clone(),
                            page.cid,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        div()
            .flex()
            .flex_col()
            .w_full()
            .children(pages.into_iter().map(|(index, label, duration, cid)| {
                let base = div().flex().w_full().gap_2().px_4().p_2().items_center();
                let base = if index == self.selected {
                    base.bg(rgb(SELECTED_BG))
                } else {
                    base
                };
                base.child(div().flex_1().child(label))
                    .child(div().text_sm().child(duration))
                    // id 用 cid：同一稿件里它唯一，切 P 也不会让两行共用一个 id。
                    .id(format!("detail-page-{cid}"))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.selected = index;
                        cx.notify();
                    }))
            }))
    }

    /// 字幕列表（P4 会把选中的那条喂给 mpv 的 `sub-add`）。
    fn subtitles_body(&self) -> impl IntoElement {
        if !self.subtitle_note.is_empty() {
            return div().px_4().p_2().text_sm().child(self.subtitle_note.clone());
        }
        div()
            .flex()
            .flex_col()
            .w_full()
            .children(self.subtitles.iter().map(|sub| {
                let label = if sub.ai {
                    format!("{}（AI）", sub.label)
                } else {
                    sub.label.clone()
                };
                div()
                    .flex()
                    .w_full()
                    .gap_2()
                    .px_4()
                    .p_2()
                    .items_center()
                    .child(div().flex_1().child(label))
                    .child(div().text_sm().child(sub.lan.clone()))
            }))
    }

    /// 区块标题。
    fn section(title: &'static str) -> impl IntoElement {
        div().px_4().pt_4().text_sm().child(title)
    }

    /// 主体内容（拿到了详情才走这里）。
    fn content(&self, info: &VideoDetailInfo, cx: &mut Context<Self>) -> impl IntoElement {
        let hero = if info.cover.is_empty() {
            div().w(px(320.)).h(px(180.)).bg(rgb(0x2A2A2A)).into_any_element()
        } else {
            div()
                .w(px(320.))
                .h(px(180.))
                .overflow_hidden()
                .child(
                    img(SharedString::from(cover::for_display(
                        &info.cover,
                        HERO_DEVICE_WIDTH,
                        HERO_DEVICE_HEIGHT,
                    )))
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .with_fallback(|| div().size_full().bg(rgb(0x1A1A1A)).into_any_element()),
                )
                .into_any_element()
        };

        let tags = info.tags.join(" / ");
        let notes = info.notes.join(" · ");

        div()
            .flex()
            .flex_col()
            .w_full()
            .child(hero)
            .child(div().text_base().child(info.title.clone()))
            .child(div().text_sm().child(format!(
                "UP：{} · {}",
                info.owner_name, info.bvid
            )))
            .child(div().text_sm().child(format!(
                "播放 {} · 弹幕 {} · 评论 {} · 硬币 {} · 收藏 {} · 分享 {} · 点赞 {}",
                info.stat.play_text,
                info.stat.danmaku_text,
                info.stat.reply_text,
                info.stat.coin_text,
                info.stat.favorite_text,
                info.stat.share_text,
                info.stat.like_text,
            )))
            .child(div().text_sm().child(format!(
                "共 {} P · 总时长 {}（播放接到下一阶段）",
                info.videos, info.total_duration_text
            )))
            .child(if tags.is_empty() {
                div().text_sm().child("（没有标签）")
            } else {
                div().text_sm().child(tags)
            })
            .child(if info.desc.is_empty() {
                div().text_sm().child("（没有简介）")
            } else {
                div().text_sm().child(info.desc.clone())
            })
            // 降级说明：主体成功但某一块失败时，原因显示在主体下面，那一块自己留空位。
            .child(if notes.is_empty() {
                div()
            } else {
                div().text_sm().child(format!("部分信息缺失：{notes}"))
            })
            .child(Self::section("选集"))
            .child(self.pages_body(cx))
            .child(Self::section("字幕"))
            .child(self.subtitles_body())
            .child(Self::section("相关推荐"))
            .child(self.related_cards(cx))
    }
}

/// 相关推荐点开来是同一层导航：发同一个事件给外壳。
impl EventEmitter<OpenVideo> for DetailPage {}

impl Render for DetailPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 两条分支的元素类型不同，各自转成 AnyElement 后交给同一个外层 div。
        let body: AnyElement = match self.detail.clone() {
            Some(info) => self.content(&info, cx).into_any_element(),
            None => div()
                .p_4()
                .text_sm()
                .child(format!("{}：{}", self.bvid.as_str(), self.note.as_str()))
                .into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .id("detail-scroll")
                    .flex_1()
                    .w_full()
                    .overflow_y_scroll()
                    .child(body),
            )
    }
}
