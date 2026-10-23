//! 详情页（P3）。
//!
//! ## 这一页做到"看得清 + 选得了 P + 起得了播"
//!
//! 主体（标题/封面/UP 主/统计/简介/标签/分P/相关推荐）来自 `api/detail.rs`，字幕列表来自
//! `api/detail::fetch_subtitles`。播放是 P4a：**取播控地址 → 拉起一个独立的 mpv 进程**，
//! 我们只发起、不遥控（暂停/进度/换清晰度保住进度都要等命名管道 IPC，即 P4b）。
//! 因此清晰度按钮现在是"重新取地址 + 重开一个 mpv"，界面与 `player_note` 都照实这么写。
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

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;

use crate::api::detail::{self, PageInfo, SubtitleBrief, VideoDetailInfo};
use crate::api::stream::{self, StreamChoice};
use crate::pages::OpenVideo;
use crate::store::credentials::SavedCredential;
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
    /// 播放那一块：已取到的流地址（None = 还没取或取失败）。
    plan: Option<StreamChoice>,
    /// 播放块的提示：取地址中 / 失败原因 / 已拉起 mpv。
    player_note: SharedString,
    /// 我们为这个详情页拉起的 mpv。留着它是为了：再次起播时收掉上一个实例、
    /// 以及本实体被弹出详情页栈时把子进程一起收掉（见本文件的 `Drop`）。
    player: Option<std::process::Child>,
    /// 正在为哪个 cid 取地址；取回来时不是它就丢掉（用户可能又点了别的 P）。
    planning_cid: Option<u64>,
}

/// 详情页退出时必须带走自己拉起的 mpv：外壳把这一层 pop 掉（或整个窗口关掉）时，
/// 留下一条在放视频、又没人再能遥控它的 mpv 是最难查的"声音还在"问题。
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
        info.pages.get(self.selected).cloned().or_else(|| info.pages.first().cloned())
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
    /// 现在（P4a）的做法是**重新取地址并换一个 mpv 实例**；等管道 IPC 通了（P4b）再改成
    /// 在同一实例里 `loadfile` + `audio-remove`/`audio-add`，那样窗口和进度都能保住。
    /// 先把按钮做出来是有意义的：它能立刻暴露"哪个档位其实拿不到流"这类问题。
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
            Ok((child, pipe_name)) => {
                let pid = child.id();
                self.player = Some(child);
                let track = match (plan.is_dash, plan.audio_url.as_ref()) {
                    (true, Some(_)) => "DASH 双流",
                    (true, None) => "DASH 只有视频轨（音频接口没给流）",
                    (false, _) => "durl 单文件",
                };
                log::info!("详情页：mpv 管道名 {pipe_name}");
                self.player_note = format!(
                    "已拉起 mpv（pid {pid}）：{} · {} · {} · 备用地址 {} 个（P4b 才自动换）· mpv 自己的输出逐行进 debug.log",
                    plan.quality_label,
                    plan.codecs,
                    track,
                    plan.backup_urls.len()
                )
                .into();
            }
            Err(error) => {
                self.player_note = format!("起播失败：{error}").into();
            }
        }
    }

    /// 播放区：起播按钮 + 状态行 + 一排清晰度。
    fn player_block(&self, cx: &mut Context<Self>) -> AnyElement {
        // SharedString 而不是 &str：`IntoElement` 那条实现是按 `SharedString`/`String` 来的，
        // 借来的 &str 生命周期不保证能进元素树。
        let status: SharedString = if self.player_note.is_empty() {
            "还没起播。".into()
        } else {
            self.player_note.clone()
        };

        let mut block = div()
            .flex()
            .flex_col()
            .w_full()
            .gap_1()
            .px_4()
            .p_2()
            .child(
                div().flex().gap_2().items_center().child(
                    Button::new("detail-play")
                        .label("播放选中分P")
                        .primary()
                        .on_click(cx.listener(|this, _, _window, cx| this.play_selected(cx))),
                ),
            )
            .child(div().text_sm().child(status));

        if let Some(plan) = self.plan.as_ref() {
            let options = plan.available.clone();
            block = block.child(
                div().flex().gap_1().children(options.into_iter().map(|option| {
                    let qn = option.qn;
                    Button::new(format!("detail-qn-{qn}"))
                        .label(option.label.clone())
                        .on_click(cx.listener(move |this, _, _window, cx| {
                            this.play_with_quality(qn, cx);
                        }))
                })),
            );
        }
        block.into_any_element()
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

    /// 字幕列表（P4b 会把选中的那条喂给 mpv 的 `sub-add`）。
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
                "共 {} P · 总时长 {}",
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
            .child(Self::section("播放"))
            .child(self.player_block(cx))
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
