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
//! ## 遥控（P4b）
//!
//! 起播成功后我们**再连一条 mpv 的命名管道**（`player::ipc::open`），订阅
//! `time-pos`/`duration`/`pause`/`eof-reached`/`idle-active`/`paused-for-cache`，
//! 于是界面能显示进度与状态，也能发暂停、±10 s、`quit`。
//! 三个写法上的定稿：
//! - **状态只信 mpv**：按暂停时不本地翻转 `paused`，等 `property-change` 回来再改；
//!   否则命令失败时按钮会和实际不符（见 `player/state.rs`）。
//! - **一条常驻 watch 任务 + 代数守卫**：`player_generation` 每次起播加一，
//!   旧任务发现自己被取代就直接丢事件。**不要**用 drop Task 来停旧循环（第 4.9 节）。
//! - **管道断开就当 mpv 已经走了**：`IpcEvent::Closed` 时把 `Child` 与链路一起放下，
//!   否则用户自己关掉 mpv 窗口之后，我们 `Drop` 里那句 `kill` 会打到别人的 pid 上
//!   （pid 会被复用，这不是洁癖）。
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
use crate::player::ipc::{self, EventStream, IpcEvent, PlayerLink};
use crate::player::state::PlayState;
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
    /// 与 mpv 之间的那条命名管道（起播成功后才有）。drop 它 = 结束对面那个管道任务。
    link: Option<PlayerLink>,
    /// mpv 回传属性折出来的播放状态。**只信它**，不按按钮时不本地翻转。
    state: PlayState,
    /// 第几次起播：上一轮的 watch 任务靠它发现自己已过时（第 4.9 节的规矩：不能用 drop Task 停）。
    player_generation: u32,
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
            link: None,
            state: PlayState::default(),
            player_generation: 0,
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
                        view.start_playback(cx);
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

    /// 用已取到的地址拉起 mpv（独立窗口）并接上遥控管道。不 notify，调用方负责。
    fn start_playback(&mut self, cx: &mut Context<Self>) {
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
                // 旧链路的发送端一 drop，对面那个管道任务就自己结束了（pump 里 select 到 None）。
                self.link = None;
                self.state = PlayState::default();
                self.player_generation += 1;
                let mut link = ipc::open(pipe_name.clone());
                if let Some(events) = link.take_events() {
                    self.watch_ipc(events, self.player_generation, cx);
                }
                self.link = Some(link);

                let track = match (plan.is_dash, plan.audio_url.as_ref()) {
                    (true, Some(_)) => "DASH 双流",
                    (true, None) => "DASH 只有视频轨（音频接口没给流）",
                    (false, _) => "durl 单文件",
                };
                self.player_note = format!(
                    "已拉起 mpv（pid {pid}）：{} · {} · {} · 备用地址 {} 个（P4b-2 才自动换）",
                    plan.quality_label, plan.codecs, track, plan.backup_urls.len()
                )
                .into();
                log::info!("详情页：起播后去连遥控管道 {pipe_name}");
            }
            Err(error) => {
                self.player_note = format!("起播失败：{error}").into();
            }
        }
    }

    /// 收 mpv 的事件并写进状态。
    ///
    /// 接收端 move 进这条任务，所以一个 `PlayerLink` 只能被 watch 一次（`take_events` 的原因）。
    /// `generation` 不匹配就什么也不做——那是上一次起播留下的任务在送旧事件。
    fn watch_ipc(&mut self, mut events: EventStream, generation: u32, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                let closed = matches!(event, IpcEvent::Closed);
                let Some(view) = this.upgrade() else {
                    log::debug!("详情页：实体已释放，结束遥控任务");
                    return;
                };
                view.update(cx, move |view, cx| {
                    if view.player_generation != generation {
                        log::debug!("详情页：遥控事件属于代 {generation}，已被取代，丢弃");
                        return;
                    }
                    if let Some(note) = view.state.apply(event) {
                        view.player_note = note.into();
                    }
                    if closed {
                        // mpv 自己走了（用户点 X，或 quit 生效）。句柄必须放下：
                        // pid 会被系统复用，留着它到 `Drop` 里 kill 就打的是别人的进程。
                        view.player = None;
                        view.link = None;
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// 往 mpv 发一条命令。链路不在（或已经断了）就把原因写进状态：按钮按下去必须有反馈，
    /// 否则用户只会觉得"这按钮没反应"。
    fn with_link(
        &mut self,
        cx: &mut Context<Self>,
        what: &'static str,
        send: impl FnOnce(&PlayerLink) -> bool,
    ) {
        let sent = match self.link.as_ref() {
            Some(link) => send(link),
            None => false,
        };
        if !sent {
            self.player_note = format!("遥控没连上，{what}没发出去。").into();
        }
        cx.notify();
    }

    /// 暂停 / 继续。取反的是 **mpv 报回来的** `paused`，本地不翻转（见 `player/state.rs`）。
    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        let want = !self.state.paused;
        let what = if want { "暂停" } else { "继续" };
        self.with_link(cx, what, move |link| link.set_pause(want));
    }

    /// 相对跳转，秒（负数是倒退）。
    fn nudge(&mut self, seconds: f64, cx: &mut Context<Self>) {
        let what = if seconds >= 0.0 { "快进" } else { "倒退" };
        self.with_link(cx, what, move |link| link.seek_by(seconds));
    }

    /// 请 mpv 自己退出（比 `kill` 干净；`Closed` 回来后我们就不再有句柄）。
    fn quit_player(&mut self, cx: &mut Context<Self>) {
        self.with_link(cx, "关闭播放器", |link| link.quit());
    }

    /// 播放区：起播按钮 + 状态行 + 遥控按钮一排 + 清晰度一排。
    fn player_block(&self, cx: &mut Context<Self>) -> AnyElement {
        // 口径都在 `player::state::PlayState::line`：连上遥控就显示 mpv 报回来的进度与状态，
        // 没连上才退回起播那句提示（"起播失败：…"、"mpv 已退出（管道断开）"…）。
        let note = if self.state.connected {
            ""
        } else {
            self.player_note.as_str()
        };
        let status = SharedString::from(self.state.line(note));
        let pause_label = if self.state.paused { "继续" } else { "暂停" };

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
            .child(div().text_sm().child(status))
            // 遥控这一排在"连上管道"之前也应该在：按下去会写明"遥控没连上"，
            // 比按钮凭空消失好排查（第十八轮的教训就是界面不给原因）。
            .child(
                div()
                    .flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("detail-pause")
                            .label(pause_label)
                            .on_click(cx.listener(|this, _, _window, cx| this.toggle_pause(cx))),
                    )
                    .child(
                        Button::new("detail-back")
                            .label("-10 秒")
                            .on_click(cx.listener(|this, _, _window, cx| this.nudge(-10.0, cx))),
                    )
                    .child(
                        Button::new("detail-forward")
                            .label("+10 秒")
                            .on_click(cx.listener(|this, _, _window, cx| this.nudge(10.0, cx))),
                    )
                    .child(
                        Button::new("detail-quit")
                            .label("关闭播放器")
                            .on_click(cx.listener(|this, _, _window, cx| this.quit_player(cx))),
                    ),
            );

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
