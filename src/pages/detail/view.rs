//! 详情页（P3 起，P4 加起播）。
//!
//! ## 这一页做什么
//!
//! 主体（标题/封面/UP 主/统计/简介/标签/分P/相关推荐）来自 `api/detail.rs`，字幕列表来自
//! `api/detail::fetch_subtitles`，评论区来自 `api/comments.rs`（它要的是 **aid**，所以只能挂在主体
//! 回来之后），"播放"只负责**把选中的那一路流交给 mpv 起进程**——
//! 进度、暂停、音量、字幕显示一律在 mpv 那个窗口里，不在这一页（范围定稿见
//! `docs/architecture.md` 第 4.3 节）。
//!
//! 字幕这块到 P6 才接到 mpv 的 `sub-add`；现在只是把清单列出来。
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
//! 相关推荐的卡片复用搜索页那套 `video_card::horizontal_clickable`，发的还是同一个
//! [`OpenVideo`](crate::pages::OpenVideo) 事件——外壳收到就往栈里再压一层详情页，
//! 所以"相关推荐 → 进去 → 返回"是连续的，页面自己不做导航决定。
//!
//! ## 排版照的是 Bili.Copilot 的"信息栏"，不是"上播放器下双栏"
//!
//! 参考项目的详情其实是一个播放器窗口 + 贴在左侧的 **360 宽信息面板**
//! （`Forms/PlayerWindow.cs:19-20`、`VideoPlayerPanel.xaml:11-31`），页签是
//! 信息/分P/合集/播放列表/推荐/评论。我们把"播放器"整个交给外置 mpv 了，所以这里沿用
//! 它的**分栏比例与组件度量**：主列放信息，右栏 360 放"选集 + 相关推荐"，
//! 分 P 行最小高 40、内边距 `12,8`、列距 8、每行都带卡片底色，选中态是"更亮的底色 + 2px 边框"
//! （`VideoPartsSection.xaml:18-21`、`Overrides.xaml:190-197` 与 `:233-247`）。
//!
//! ## gpui 写法上的三个约束（本文件因此的样子）
//!
//! - 交互方法（`on_click`/`hover`）只在 `Stateful<Div>` 上有，链式顺序固定为
//!   "样式 → child → `.id(..)` → `.on_click(..)`"，与 `gpui/src/window/prompts.rs:139-146` 一致。
//! - `overflow_y_scroll()` 同样是 `Stateful<Div>` 的方法，滚动容器必须先给 id。
//! - `impl IntoElement` 只能有一个返回类型，所以"没拿到主体"与"拿到主体"两条分支
//!   各自转成 `AnyElement`，再交给同一个外层 div。

use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::api::comments::{self, CommentItem, CommentSortKind};
use crate::api::detail::{self, PageInfo, SubtitleBrief, VideoDetailInfo};
use crate::api::stream::{self, StreamChoice};
use crate::pages::{OpenUser, OpenVideo};
use crate::store::auth::Auth;
use crate::ui::scroll::DragScroll;
use crate::ui::{cover, layout, palette};
use crate::ui::video_card::{self, VideoCardData};

/// 详情页大图向图床要的尺寸（逻辑上宽自适应，按 16:9 裁，先要一张 640×360 的缩略）。
const HERO_DEVICE_WIDTH: u32 = 640;
const HERO_DEVICE_HEIGHT: u32 = 360;

/// 详情页根视图。
pub struct DetailPage {
    client: Arc<bpi_rs::BpiClient>,
    bvid: SharedString,
    detail: Option<VideoDetailInfo>,
    /// 主体的状态：加载中 / 失败原因。空字符串表示一切正常。
    note: SharedString,
    subtitles: Vec<SubtitleBrief>,
    subtitle_note: SharedString,
    /// 当前选中的分 P（`pages` 里的下标）。起播时用它取 cid 与标题。
    selected: usize,
    seq: u32,
    /// 播放那一块：已取到的流地址（None = 还没取或取失败）。
    plan: Option<StreamChoice>,
    /// 播放块的提示：取地址中 / 失败原因 / 已拉起 mpv。
    player_note: SharedString,
    /// 我们为这个详情页拉起的 mpv。留着它是为了：再次起播时收掉上一个实例、
    /// 以及本实体被弹出详情页栈时把子进程一起收掉（见下面的 `Drop`）。
    player: Option<std::process::Child>,
    /// 正在为哪个 cid 取地址；取回来时不是它就丢掉（用户可能又点了别的 P）。
    planning_cid: Option<u64>,
    /// 评论区当前这一页（None = 还没取到）。总数与页码都在里面，"还有下一页"由它自己算。
    comment: Option<comments::CommentPage>,
    /// 评论区的排序口径（决定"最热/最新"哪颗按钮亮着）。
    comment_sort: CommentSortKind,
    /// 评论区的提示位：加载中 / 失败原因 / 空评论。
    comment_note: SharedString,
    /// 评论自己的请求序号：换排序或翻页时，旧结果回来要自己让位（第 4.9 节）。
    comment_seq: u32,
    /// 主列那个滚动容器的拖动状态（`ui/scroll.rs`）。
    scroll: DragScroll,
    /// 右栏（选集与相关推荐）是第二个容器，必须单独一个：共用会让两边互相把对方的偏移抢走。
    side_scroll: DragScroll,
}

/// 详情页退出时必须带走自己拉起的 mpv：外壳把这一层 pop 掉（或整个窗口关掉）时，
/// 留下一条在放视频、又没人再能关掉它的 mpv 是最难查的"声音还在"问题。
/// `std::process::Child` 的 drop 不会杀子进程，所以这里显式 kill。
impl Drop for DetailPage {
    fn drop(&mut self) {
        if let Some(child) = self.player.as_mut() {
            log::info!("详情页退出，收掉 mpv pid={}", child.id());
            let _ = child.kill();
        }
    }
}

impl DetailPage {
    /// 建页并发起主体请求。
    pub fn new(client: Arc<bpi_rs::BpiClient>, bvid: String, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            scroll: DragScroll::default(),
            side_scroll: DragScroll::default(),
            client,
            bvid: bvid.into(),
            detail: None,
            note: "加载中…".into(),
            subtitles: Vec::new(),
            subtitle_note: "字幕加载中…".into(),
            selected: 0,
            seq: 0,
            plan: None,
            player_note: "".into(),
            player: None,
            planning_cid: None,
            comment: None,
            comment_sort: CommentSortKind::Hot,
            comment_note: "".into(),
            comment_seq: 0,
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
            view.update(cx, |view, cx| {
                // 评论区的 `oid` 是**数字稿件号**（不是 bvid，见 `api/comments.rs` 文件头），
                // 所以它只能挂在主体之后：这里顺手把 aid 取出来。
                let aid = view.detail.as_ref().map(|info| info.aid);
                let sort = view.comment_sort;
                view.load_subtitles(cid, seq, cx);
                match aid {
                    Some(aid) => view.load_comments(aid, 1, sort, cx),
                    None => view.comment_note = "主体里没有可用的稿件号，取不了评论".into(),
                }
            });
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

    /// 取评论区的一页。`aid` 必须由调用方给（评论区的 `oid` 是数字稿件号，只能等主体回来才有）。
    ///
    /// 失败**不清空已有列表**：翻下一页失败时把旧的一页留着 + 显示原因，比把整块抹成空白好查。
    fn load_comments(&mut self, aid: u64, page: u32, sort: CommentSortKind, cx: &mut Context<Self>) {
        self.comment_seq += 1;
        let seq = self.comment_seq;
        self.comment_sort = sort;
        self.comment_note = format!("评论加载中…（第 {page} 页）").into();
        cx.notify();

        let join = comments::fetch_comments(Arc::clone(&self.client), aid, page, sort);
        cx.spawn(async move |this, cx| {
            let outcome = match join.await {
                Ok(inner) => inner.map_err(|error| error.to_string()),
                Err(join_error) => Err(crate::app::describe_join_error(join_error)),
            };
            log::debug!("详情页：评论结果已回到前台（#{seq}）");

            let Some(view) = this.upgrade() else {
                log::warn!("详情页：等待期间实体已释放，丢弃评论结果");
                return;
            };
            view.update(cx, |view, cx| {
                if view.comment_seq != seq {
                    log::debug!("详情页：评论请求 #{seq} 已被取代，丢弃");
                    return;
                }
                match outcome {
                    Ok(page_data) => {
                        view.comment_note = if page_data.items.is_empty() {
                            "这条稿件没有评论"
                        } else {
                            ""
                        }
                        .into();
                        view.comment = Some(page_data);
                    }
                    Err(message) => {
                        view.comment_note = format!("取评论失败：{message}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 翻页/换排序的入口：自己取 aid，取不到就只写提示（不发无意义的请求）。
    fn request_comments(&mut self, page: u32, sort: CommentSortKind, cx: &mut Context<Self>) {
        let Some(aid) = self.detail.as_ref().map(|info| info.aid) else {
            self.comment_note = "还没有主体信息，取不了评论".into();
            cx.notify();
            return;
        };
        self.load_comments(aid, page, sort, cx);
    }

    /// 当前选中的分 P。
    fn selected_page(&self) -> Option<PageInfo> {        let info = self.detail.as_ref()?;
        info.pages
            .get(self.selected)
            .cloned()
            .or_else(|| info.pages.first().cloned())
    }

    /// 播放选中的那一 P（想要封顶档，B 站会给到它能给的最高）。
    fn play_selected(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.selected_page() else {
            self.player_note = "这条稿件没有可播的分 P".into();
            cx.notify();
            return;
        };
        self.request_plan(page.cid, stream::request_qn(), cx);
    }

    /// 换清晰度。
    ///
    /// 现在的做法是**重新取地址并换一个 mpv 实例**（窗口会回到开头）。同实例用 `loadfile` +
    /// `audio-add` 换流才能保住窗口与进度，那是 P6 的事（管道机制已核实并留档在
    /// `docs/architecture.md` 第 4.14 节）。先把按钮做出来是有意义的：
    /// 它能立刻暴露"哪个档位其实拿不到流"这类问题。
    fn play_with_quality(&mut self, qn: u64, cx: &mut Context<Self>) {
        let Some(page) = self.selected_page() else {
            self.player_note = "这条稿件没有可播的分 P".into();
            cx.notify();
            return;
        };
        self.request_plan(page.cid, qn, cx);
    }

    /// 为一个 cid 取播控地址。结果回来时如果用户已经改点了别的 P，这份结果就丢掉。
    fn request_plan(&mut self, cid: u64, qn: u64, cx: &mut Context<Self>) {
        let Some(bvid) = self.detail.as_ref().map(|info| info.bvid.clone()) else {
            return;
        };
        self.planning_cid = Some(cid);
        self.player_note = format!("取播控地址…（想要 qn={qn}）").into();
        cx.notify();

        let join = stream::fetch_streams(Arc::clone(&self.client), bvid, cid, qn);
        cx.spawn(async move |this, cx| {
            let outcome = match join.await {
                Ok(inner) => inner.map_err(|error| error.to_string()),
                Err(join_error) => Err(crate::app::describe_join_error(join_error)),
            };
            log::debug!("详情页：播控地址已回到前台（cid={cid}）");

            let Some(view) = this.upgrade() else { return };
            view.update(cx, |view, cx| {
                if view.planning_cid != Some(cid) {
                    log::debug!("详情页：cid={cid} 的地址已过时，丢弃");
                    return;
                }
                match outcome {
                    Ok(plan) => {
                        view.plan = Some(plan);
                        view.start_playback();
                    }
                    Err(message) => {
                        view.plan = None;
                        view.player_note = format!("取播控地址失败：{message}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 用已取到的地址拉起 mpv（独立窗口）。不 notify，调用方负责。
    fn start_playback(&mut self) {
        let Some(plan) = self.plan.clone() else { return };

        // 上一次也是我们拉起的实例：先收掉，否则点几次就堆几个窗口。
        if let Some(mut previous) = self.player.take() {
            log::info!("再次起播，收掉上一个 mpv pid={}", previous.id());
            let _ = previous.kill();
        }

        // 1080P/高码率的 m4s 直链要校验登录 cookie；bpi-rs 不把它的 cookie jar 交出来，
        // 所以由我们从凭据（运行同目录的 auth.json）里拼一条，和 Referer/UA 一起给 mpv。
        let extra_headers = match Auth::load() {
            Some(auth) => vec![format!("Cookie: {}", auth.cookie_header())],
            None => Vec::new(),
        };

        match crate::player::launch(&plan.video_url, plan.audio_url.as_deref(), &extra_headers) {
            Ok(child) => {
                let pid = child.id();
                self.player = Some(child);
                let track = match (plan.is_dash, plan.audio_url.as_ref()) {
                    (true, Some(_)) => "DASH 双流",
                    (true, None) => "DASH 只有视频轨（音频接口没给流）",
                    (false, _) => "durl 单文件",
                };
                self.player_note = format!(
                    "已交给 mpv（pid {pid}）：{} · {} · {}。暂停/进度/字幕都在播放窗口里操作。",
                    plan.quality_label, plan.codecs, track
                )
                .into();
            }
            Err(error) => {
                self.player_note = format!("起播失败：{error}").into();
            }
        }
    }

    /// 播放那一块：起播按钮 + 提示 + 清晰度一排。放在主列顶部，紧挨着标题。
    fn player_block(&self, cx: &mut Context<Self>) -> AnyElement {
        let status = SharedString::from(if self.player_note.is_empty() {
            "选好分 P 后点\"播放这一 P\"；进度与音量在 mpv 窗口里控制。".to_string()
        } else {
            self.player_note.to_string()
        });

        let mut rows: Vec<AnyElement> = vec![
            div()
                .flex()
                .gap_2()
                .items_center()
                .child(
                    Button::new("detail-play")
                        .label("播放这一 P")
                        .primary()
                        .on_click(cx.listener(|this, _, _window, cx| this.play_selected(cx))),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(palette::FONT_CAPTION))
                        .text_color(palette::text_secondary())
                        .child(status),
                )
                .into_any_element(),
        ];

        if let Some(plan) = self.plan.as_ref() {
            let options = plan.available.clone();
            rows.push(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(palette::CARD_TEXT_ROW_GAP))
                    .children(options.into_iter().map(|option| {
                        let qn = option.qn;
                        Button::new(format!("detail-qn-{qn}"))
                            .label(option.label.clone())
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.play_with_quality(qn, cx);
                            }))
                    }))
                    .into_any_element(),
            );
        }
        layout::card(rows)
    }

    /// 相关推荐的卡片：点它 = 让外壳再压一层详情页。
    fn related_cards(&self, cx: &mut Context<Self>) -> AnyElement {
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
                        extra: SharedString::default(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        if cards.is_empty() {
            return layout::note("（没有相关推荐）");
        }

        div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(palette::LIST_GAP))
            .px(px(palette::LIST_ROW_PADDING_X))
            .py(px(palette::CARD_TEXT_ROW_GAP))
            .children(cards.into_iter().map(|card| {
                let bvid = card.bvid.as_str().to_string();
                video_card::horizontal_clickable(
                    card,
                    cx.listener(move |_this, _event, _window, cx| {
                        cx.emit(OpenVideo { bvid: bvid.clone() });
                    }),
                )
            }))
            .into_any_element()
    }

    /// 分 P（选集）列表；点一行就是选中它。
    fn pages_body(&self, cx: &mut Context<Self>) -> AnyElement {
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
            .gap(px(palette::LIST_GAP))
            .px(px(palette::LIST_ROW_PADDING_X))
            .py(px(palette::CARD_TEXT_ROW_GAP))
            .children(pages.into_iter().map(|(index, label, duration, cid)| {
                let selected = index == self.selected;
                // id 用 cid：同一稿件里它唯一，切 P 也不会让两行共用一个 id。
                div()
                    .flex()
                    .w_full()
                    .min_h(px(palette::LIST_ROW_MIN_HEIGHT))
                    .gap(px(palette::LIST_ROW_PADDING_Y))
                    .px(px(palette::LIST_ROW_PADDING_X))
                    .py(px(palette::LIST_ROW_PADDING_Y))
                    .items_center()
                    .overflow_hidden()
                    .rounded(px(palette::RADIUS))
                    .cursor_pointer()
                    // 行底色与 2px 边框都常驻（`VideoPartsSection.xaml:18-21` 每行都有卡片底；
                    // 边框见 `Overrides.xaml:190-197`），选中只换颜色，不产生位移。
                    .bg(if selected {
                        palette::card_background_elevated()
                    } else {
                        palette::card_background()
                    })
                    .border_2()
                    .border_color(if selected {
                        palette::accent()
                    } else {
                        palette::border_none()
                    })
                    .text_color(if selected {
                        palette::text_primary()
                    } else {
                        palette::text_secondary()
                    })
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(palette::FONT_BODY))
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .overflow_hidden()
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(palette::FONT_CAPTION))
                            .text_color(palette::text_tertiary())
                            .child(duration),
                    )
                    .id(format!("detail-page-{cid}"))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.selected = index;
                        cx.notify();
                    }))
                    .hover(|style| style.bg(palette::card_background_hover()))
            }))
            .into_any_element()
    }

    /// 字幕列表（P6 会把选中的那条喂给 mpv 的 `sub-add`）。
    fn subtitles_body(&self) -> AnyElement {
        if !self.subtitle_note.is_empty() {
            return layout::note(self.subtitle_note.clone());
        }
        div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(palette::LIST_GAP))
            .px(px(palette::LIST_ROW_PADDING_X))
            .py(px(palette::CARD_TEXT_ROW_GAP))
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
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(palette::FONT_BODY))
                            .text_color(palette::text_primary())
                            .child(label),
                    )
                    .child(
                        div()
                            .text_size(px(palette::FONT_CAPTION))
                            .text_color(palette::text_tertiary())
                            .child(sub.lan.clone()),
                    )
            }))
            .into_any_element()
    }

    /// 一条评论。度量照 `CommentItemControl.xaml:21-46`：整行是卡片底 + 1px **同色**描边、
    /// `Padding=12`、头像列与文字列距 12、头像 40；行内三行间距 8（`:76-118`）。
    ///
    /// 参考项目把评论做成播放器页签里的单独一页，这里排成主列的一节：
    /// `NavigationView` 那种页签控件在 gpui 侧没有等价物，硬造一个只会更难看（风险表第 16 条）。
    fn comment_row(&self, item: &CommentItem) -> AnyElement {
        // 点赞与"共 N 条回复"挤在同一行：这一行只是元信息，不值得再开一档高度。
        let meta = if item.reply_count > 0 {
            format!("{} 赞 · 共 {} 条回复", item.like, item.reply_count)
        } else {
            format!("{} 赞", item.like)
        };
        div()
            .flex()
            .w_full()
            .items_start()
            .gap(px(palette::COMMENT_COLUMN_GAP))
            .p(px(palette::COMMENT_ROW_PADDING))
            .rounded(px(palette::RADIUS))
            .bg(palette::card_background())
            .border_1()
            .border_color(palette::card_background())
            .child(layout::avatar(&item.face, palette::AVATAR_COMMENT))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(palette::CARD_TEXT_ROW_GAP))
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .w_full()
                            .items_center()
                            .gap(px(palette::CARD_TEXT_ROW_GAP))
                            .child(
                                div()
                                    .text_size(px(palette::FONT_BODY))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(palette::text_primary())
                                    .child(item.author.clone()),
                            )
                            // 昵称与时间分居两端：中间这一档把宽度吃掉。
                            .child(div().flex_1())
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(palette::FONT_CAPTION))
                                    .text_color(palette::text_tertiary())
                                    .child(item.time_text.clone()),
                            ),
                    )
                    // 正文不折限行数：评论就是给人读完的，砍掉尾巴比整块难看。
                    .child(
                        div()
                            .w_full()
                            .text_size(px(palette::FONT_BODY))
                            .text_color(palette::text_primary())
                            .child(item.message.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(palette::FONT_CAPTION))
                            .text_color(palette::text_tertiary())
                            .child(meta),
                    ),
            )
            .into_any_element()
    }

    /// 评论区那一整块：区块标题 + 排序与翻页 + 一行行评论。
    ///
    /// 加载中/失败/没有评论都走 `comment_note` 这一行（第 4.7 节"失败必须看得见"）；
    /// 翻页失败时上一批还留着，不会把已经读到的东西抹掉。
    fn comments_block(&self, cx: &mut Context<Self>) -> AnyElement {
        let current_page = self
            .comment
            .as_ref()
            .map(|page| page.page)
            .unwrap_or(1);
        let total_text = self
            .comment
            .as_ref()
            .and_then(|page| page.total)
            .map(|total| format!("共 {total} 条"))
            .unwrap_or_default();
        let rows: Vec<AnyElement> = self
            .comment
            .as_ref()
            .map(|page| {
                page.items
                    .iter()
                    .map(|item| self.comment_row(item))
                    .collect()
            })
            .unwrap_or_default();

        let mut head = div()
            .flex()
            .w_full()
            .items_center()
            .gap(px(palette::CARD_TEXT_ROW_GAP))
            .px(px(palette::LIST_ROW_PADDING_X));
        for kind in [CommentSortKind::Hot, CommentSortKind::Latest] {
            let button = Button::new(format!("comment-sort-{}", kind.label()))
                .label(kind.label())
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    // 换排序回到第 1 页：停在第 3 页换口径读起来像是"数据跳了"。
                    if this.comment_sort != kind {
                        this.request_comments(1, kind, cx);
                    }
                }));
            head = head.child(if self.comment_sort == kind {
                button.primary()
            } else {
                button
            });
        }
        let head = head
            .child(
                div()
                    .flex_1()
                    .text_size(px(palette::FONT_CAPTION))
                    .text_color(palette::text_tertiary())
                    .child(total_text),
            )
            .child(
                Button::new("comment-prev")
                    .label("上一页")
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        let page = this
                            .comment
                            .as_ref()
                            .map(|page| page.page)
                            .unwrap_or(1);
                        if page > 1 {
                            let sort = this.comment_sort;
                            this.request_comments(page - 1, sort, cx);
                        }
                    })),
            )
            .child(
                Button::new("comment-next")
                    .label("下一页")
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        // `has_more` 自己会处理"接口没给总数"那种情况（满一页就认为还能翻）。
                        if this.comment.as_ref().is_some_and(|page| page.has_more()) {
                            let page = this
                                .comment
                                .as_ref()
                                .map(|page| page.page)
                                .unwrap_or(1);
                            let sort = this.comment_sort;
                            this.request_comments(page + 1, sort, cx);
                        }
                    })),
            );

        let mut block = div()
            .flex()
            .flex_col()
            .w_full()
            .child(layout::section_title(format!("评论 · 第 {current_page} 页")))
            .child(head);
        if !self.comment_note.is_empty() {
            block = block.child(
                div()
                    .w_full()
                    .px(px(palette::LIST_ROW_PADDING_X))
                    .py(px(palette::CARD_TEXT_ROW_GAP))
                    .text_size(px(palette::FONT_BODY))
                    .text_color(if rows.is_empty() {
                        palette::text_secondary()
                    } else {
                        // 已经有内容，说明这是"翻页/换排序失败"，按错误色提示。
                        palette::critical()
                    })
                    .child(self.comment_note.clone()),
            );
        }
        block
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .gap(px(palette::COMMENT_ROW_GAP))
                    .px(px(palette::LIST_ROW_PADDING_X))
                    .pb(px(palette::CARD_TEXT_PADDING_TOP))
                    .children(rows),
            )
            .into_any_element()
    }

    /// 主列的内容（拿到了详情才走这里）。
    fn content(&self, info: &VideoDetailInfo, cx: &mut Context<Self>) -> AnyElement {
        let hero = if info.cover.is_empty() {
            div()
                .w_full()
                .aspect_ratio(16. / 9.)
                .bg(palette::card_background_elevated())
                .into_any_element()
        } else {
            div()
                .w_full()
                .aspect_ratio(16. / 9.)
                .overflow_hidden()
                .rounded(px(palette::RADIUS))
                .child(
                    img(SharedString::from(cover::for_display(
                        &info.cover,
                        HERO_DEVICE_WIDTH,
                        HERO_DEVICE_HEIGHT,
                    )))
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
        };

        let tags = info.tags.join(" / ");
        let notes = info.notes.join(" · ");

        div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(palette::CARD_TEXT_ROW_GAP))
            .px(px(palette::CARD_TEXT_PADDING_X))
            .py(px(palette::CARD_TEXT_PADDING_TOP))
            .child(hero)
            .child(
                div()
                    .w_full()
                    .text_size(px(palette::FONT_TITLE))
                    .text_color(palette::text_primary())
                    .child(info.title.clone()),
            )
            .child(
                // UP 主那一行：44 的圆头像 + 名字 + 稿件号（`UpSectionControl.xaml:21-33`）。
                // **整行可点**，点进去是 TA 的空间页（P7）——参考项目里这个位置也是按钮
                // （`UpSectionControl` 的头像/昵称都挂了 `ShowUserSpaceCommand`）。
                div()
                    .id("detail-open-up")
                    .flex()
                    .w_full()
                    .items_center()
                    .gap(px(palette::STAT_ITEM_GAP))
                    .px(px(palette::CARD_TEXT_PADDING_X))
                    .py(px(palette::CARD_LINE_GAP))
                    .rounded(px(palette::RADIUS))
                    .cursor_pointer()
                    .child(layout::avatar(&info.owner_face, palette::AVATAR_UP))
                    .child(
                        div()
                            .text_size(px(palette::FONT_BODY))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(palette::text_primary())
                            .child(info.owner_name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(palette::FONT_CAPTION))
                            .text_color(palette::text_tertiary())
                            .child(format!("{} · {} P", info.bvid, info.videos)),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(palette::FONT_CAPTION))
                            .text_color(palette::text_tertiary())
                            .child("TA 的空间 ›"),
                    )
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        let (mid, name) = match this.detail.as_ref() {
                            Some(info) => (info.owner_mid, info.owner_name.clone()),
                            None => return,
                        };
                        cx.emit(OpenUser { mid, name });
                    }))
                    .hover(|style| style.bg(palette::card_background_hover())),
            )
            .child(layout::stat_line(vec![
                format!("播放 {}", info.stat.play_text).into(),
                format!("弹幕 {}", info.stat.danmaku_text).into(),
                format!("评论 {}", info.stat.reply_text).into(),
                format!("硬币 {}", info.stat.coin_text).into(),
                format!("收藏 {}", info.stat.favorite_text).into(),
                format!("分享 {}", info.stat.share_text).into(),
                format!("点赞 {}", info.stat.like_text).into(),
            ]))
            .child(self.player_block(cx))
            .child(if tags.is_empty() {
                layout::note("（没有标签）")
            } else {
                layout::note(tags)
            })
            .child(if info.desc.is_empty() {
                layout::note("（没有简介）")
            } else {
                layout::note(info.desc.clone())
            })
            // 降级说明：主体成功但某一块失败时，原因显示在主体下面，那一块自己留空位。
            .when(!notes.is_empty(), |this| {
                this.child(layout::note(format!("部分信息缺失：{notes}")))
            })
            .child(layout::section_title("字幕"))
            .child(self.subtitles_body())
            // 评论区挂在主列最下面：它依赖主体的 aid，且是"读完简介往下翻"的自然位置。
            .child(self.comments_block(cx))
            // 这一串链子的终点是 `Div`，函数签名要的是 `AnyElement`（两条分支都得是同一种元素）。
            .into_any_element()
    }

    /// 右栏（360）：选集 + 相关推荐。度量来自参考项目信息面板的宽度。
    fn side_column(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .w(px(palette::DETAIL_SIDE_WIDTH))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(palette::divider())
            .child(
                div()
                    .id("detail-side-scroll")
                    .flex_1()
                    .w_full()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(self.side_scroll.handle())
                    .on_mouse_down(MouseButton::Left, self.side_scroll.on_press())
                    .child(layout::section_title("选集"))
                    .child(self.pages_body(cx))
                    .child(layout::section_title("相关推荐"))
                    .child(self.related_cards(cx)),
            )
            .into_any_element()
    }
}

/// 相关推荐点开来是同一层导航：发同一个事件给外壳。
impl EventEmitter<OpenVideo> for DetailPage {}
/// 点 UP 主那一行 = 请外壳压一层空间页（P7）。名字也一起带上，
/// 空间页的资料卡要一次请求才回来，先拿这个名字占住标题。
impl EventEmitter<OpenUser> for DetailPage {}

impl Render for DetailPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 两个容器各自登记一次（`Window::on_mouse_event` 只覆盖下一帧，见 `ui/scroll.rs`）。
        let view = cx.entity_id();
        self.scroll.watch(window, view);
        self.side_scroll.watch(window, view);
        // 两条分支的元素类型不同，各自转成 AnyElement 后交给同一个外层 div。
        let (head_subtitle, body): (Option<SharedString>, AnyElement) = match self.detail.clone() {
            Some(info) => {
                let subtitle = format!("UP：{}", info.owner_name).into();
                let main = div()
                    .flex()
                    .flex_row()
                    .w_full()
                    .h_full()
                    .child(
                        div()
                            .id("detail-scroll")
                            .flex_1()
                            .h_full()
                            .min_w(px(0.))
                            .overflow_y_scroll()
                            .track_scroll(self.scroll.handle())
                            .on_mouse_down(MouseButton::Left, self.scroll.on_press())
                            .child(self.content(&info, cx)),
                    )
                    .child(self.side_column(cx))
                    .into_any_element();
                (Some(subtitle), main)
            }
            None => (
                None,
                layout::note(format!("{}：{}", self.bvid.as_str(), self.note.as_str())),
            ),
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(layout::header("视频详情", head_subtitle, None))
            .child(div().flex_1().w_full().min_h(px(0.)).child(body))
    }
}
