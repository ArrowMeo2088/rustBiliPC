//! 详情页（P3 起，P4 加起播）。
//!
//! ## 这一页做什么
//!
//! 主体（标题/封面/UP 主/统计/简介/标签/分P/相关推荐）来自 `api/detail.rs`，字幕列表来自
//! `api/detail::fetch_subtitles`，"播放"只负责**把选中的那一路流交给 mpv 起进程**——
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

use crate::api::detail::{self, PageInfo, SubtitleBrief, VideoDetailInfo};
use crate::api::stream::{self, StreamChoice};
use crate::pages::OpenVideo;
use crate::store::credentials::SavedCredential;
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

    /// 当前选中的分 P。
    fn selected_page(&self) -> Option<PageInfo> {
        let info = self.detail.as_ref()?;
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
        self.request_plan(page.cid, stream::DEFAULT_REQUEST_QN, cx);
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
        // 所以由我们从凭据里拼一条，和 Referer/UA 一起给 mpv（见 player 模块注释）。
        let extra_headers = match SavedCredential::load() {
            Some(credential) => vec![format!("Cookie: {}", credential.cookie_header())],
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
                div()
                    .flex()
                    .w_full()
                    .gap(px(palette::STAT_ITEM_GAP))
                    .items_center()
                    .child(layout::avatar(&info.owner_face, palette::AVATAR_UP))
                    .child(
                        div()
                            .text_size(px(palette::FONT_BODY))
                            .text_color(palette::text_primary())
                            .child(info.owner_name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(palette::FONT_CAPTION))
                            .text_color(palette::text_tertiary())
                            .child(format!("{} · {} P", info.bvid, info.videos)),
                    ),
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

impl Render for DetailPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
