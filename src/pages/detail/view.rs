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
//! ## P11 加的两件事：文字可选中、这一 P 能下载
//!
//! **选中与复制**用 kit 的 `SelectableText`（gpui 0.3.9 自己没有文本选择，
//! `ref/gpui/gpui/src` 里搜不到 `Selectable`/`text_selection`）：标题、简介、评论昵称与正文
//! 换成这个叶子元素，拖动就能选中，`Ctrl+C` 由 `base::Root` 装的复制动作直接送进剪贴板。
//! 触摸屏没有 Ctrl+C，所以页头右侧给了一颗「复制选中」，读的是同一个窗口级选区
//! （`TextSelection::selected_text`）。
//! 代价写在 [`DragScroll`](crate::ui::scroll::DragScroll) 里：**按在可选中文字上的那一下
//! 不再用来拖滚动**，否则手指划一段文字会同时"选中 + 翻页"。要滚就在卡片、空白处拖。
//!
//! **下载**只加一颗按钮，其余都在 [`DownloadQueue`](crate::pages::download::DownloadQueue) 与
//! [`api::download`](crate::api::download) 里：先按当前选中的 P 取一次播控地址
//! （和播放同一条路，档位仍然吃 `config.json` 的封顶），再弹系统对话框选目录。
//! 取地址的**目的**要记下来（`PlanPurpose`）：同一份地址，播放是拉起 mpv，下载是入队，
//! 结果回来时如果用户已经改了主意（换了 P、或又点了播放），这份计划就丢掉。
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

use gpui_kit::base::{SelectableText, TextSelection};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::api::comments::{self, CommentItem, CommentSortKind};
use crate::api::detail::{
    self, PageInfo, SeasonSectionBrief, SubtitleBrief, VideoDetailInfo,
};
use crate::api::download;
use crate::api::probe::{self, ProbeOutcome};
use crate::api::stream::{self, StreamChoice};
use crate::pages::download::DownloadQueue;
use crate::pages::relogin;
use crate::pages::{OpenUser, OpenVideo};
use crate::ui::scroll::DragScroll;
use crate::ui::video_card::{self, VideoCardData};
use crate::ui::{cover, layout, palette};

/// 详情页大图向图床要的尺寸（逻辑上宽自适应，按 16:9 裁，先要一张 640×360 的缩略）。
const HERO_DEVICE_WIDTH: u32 = 640;
const HERO_DEVICE_HEIGHT: u32 = 360;

/// 取播控地址是为了干什么。同一份地址，播放与下载的下一步完全不同，
/// 所以这个"意图"必须跟着请求一起走，结果回来时才知道该往哪条路交。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlanPurpose {
    Play,
    Download,
}

/// 一段会换行、又允许选中复制的正文（简介那一块）。
///
/// 为什么不直接给 [`layout::note`](crate::ui::layout::note) 加个"可选"参数：
/// `layout::note` 收的是 `impl Into<SharedString>`，而 `SelectableText` 是**元素**不是文本，
/// 塞进去会让复用件的签名变成两种形状。度量（16 内边距 / 14 字 / 次要灰）在这里抄一遍，
/// 只有"要不要能选中"这一处不同。
fn selectable_note(text: String) -> AnyElement {
    div()
        .w_full()
        .px(px(palette::CARD_TEXT_PADDING_X))
        .py(px(palette::CARD_TEXT_ROW_GAP))
        .text_size(px(palette::FONT_BODY))
        .text_color(palette::text_secondary())
        .line_height(px(palette::LINE_HEIGHT_BODY))
        .child(SelectableText::new("detail-desc", text))
        .into_any_element()
}

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
    /// 播放块的提示：取地址中 / 失败原因 / 已拉起 mpv / 已开始下载。
    player_note: SharedString,
    /// 我们为这个详情页拉起的 mpv。留着它是为了：再次起播时收掉上一个实例、
    /// 以及本实体被弹出详情页栈时把子进程一起收掉（见下面的 `Drop`）。
    player: Option<std::process::Child>,
    /// 正在为哪个 cid、为了什么取地址；取回来时不是它就丢掉（用户可能又点了别的 P）。
    planning: Option<(u64, PlanPurpose)>,
    /// 起播探测（`api::probe`）的序号：探测期间用户又点了一次播放/换了 P，旧结果作废。
    probe_seq: u32,
    /// 外壳注入的下载队列：这一页只负责"把一次下载排进去"，列表在下载页看。
    queue: Entity<DownloadQueue>,
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
    /// 分集面板当前展开的是第几个**分段**（`ugc_season.sections` 的下标）。
    ///
    /// 只有多分段的合集才用得到它（像"物理er的本科专业合集"那种：电动/猞狸、量力/度规…）。
    /// 换稿件（重新取主体）时归 0，不然会指着上一份合集的某一段。
    season_section: usize,
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
    /// 建页并发起主体请求。`queue` 是外壳那份下载队列（点"下载这一 P"时往里排任务）。
    pub fn new(
        client: Arc<bpi_rs::BpiClient>,
        bvid: String,
        queue: Entity<DownloadQueue>,
        cx: &mut Context<Self>,
    ) -> Self {
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
            planning: None,
            probe_seq: 0,
            queue,
            season_section: 0,
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
                Ok(inner) => inner.map_err(|error| relogin::message(&error)),
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
                        // 分段选择跟着稿件走：换了一份合集还指着上一份的第 3 段是错的。
                        view.season_section = 0;
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
            relogin::flush(cx);
        })
        .detach();
    }

    /// 取这一 P 的字幕列表。失败只写字幕那一块。
    fn load_subtitles(&mut self, cid: u64, seq: u32, cx: &mut Context<Self>) {
        log::debug!("详情页：发起字幕列表请求（#{seq}）cid={cid}");
        let join = detail::fetch_subtitles(Arc::clone(&self.client), self.bvid.to_string(), cid);
        cx.spawn(async move |this, cx| {
            let outcome = match join.await {
                Ok(inner) => inner.map_err(|error| relogin::message(&error)),
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
            relogin::flush(cx);
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
                Ok(inner) => inner.map_err(|error| relogin::message(&error)),
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
            relogin::flush(cx);
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
        self.request_plan(page.cid, stream::request_qn(), PlanPurpose::Play, cx);
    }

    /// 下载选中的那一 P：**每次都要重新取一次地址**，不复用手里那份 `plan`。
    ///
    /// 为什么不省这一次请求：B 站的 m4s 直链是**有时效**的（库里那条注释写的是 120 分钟，
    /// `ref/bpi-rs/src/models/stream.rs:202-215`），而"先播一会儿、想起要下载"完全可能隔更久。
    /// 拿一份过期的地址去下，症状是下到一半变 403/404，比多等 200 ms 难查得多
    /// （取址与播放同一条路：同一个 `request_qn()`、同一条选流规则，
    /// 所以"下到的画质"与"刚才播的画质"仍然同一口径）。
    /// `durl`（分段直链）由 [`download::request_for`] 拒绝，这里不重复判断。
    fn download_selected(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.selected_page() else {
            self.player_note = "这条稿件没有可下载的分 P".into();
            cx.notify();
            return;
        };
        self.request_plan(page.cid, stream::request_qn(), PlanPurpose::Download, cx);
    }

    /// 弹系统对话框问一次目录，然后把这一对文件排进队列。
    ///
    /// `prompt_for_paths` 返回一个 oneshot 的 `Receiver`，所以这里必须 `cx.spawn` 去等它
    /// （gpui 的文档要求：这类平台提示在异步上下文里要放在 `cx.update`/任务里，
    /// 不要塞进别的实体的 `update`）。用户按"取消"拿到的是 `None`——那不是错误，
    /// 界面上只写"没有选目录"。
    fn ask_destination(&mut self, plan: StreamChoice, page: &PageInfo, cx: &mut Context<Self>) {
        let Some(info) = self.detail.clone() else {
            return;
        };
        // 只有多 P 稿件才在文件名里带页码：单 P 的标题本身就是完整的名字。
        let part = if info.pages.len() > 1 {
            Some((page.page, page.part.as_str()))
        } else {
            None
        };
        let request = match download::request_for(
            &plan,
            format!("{} · {}", info.title, plan.quality_label),
            download::file_stem(&info.title, &info.bvid, part),
        ) {
            Ok(request) => request,
            Err(message) => {
                self.player_note = message.into();
                cx.notify();
                return;
            }
        };
        let http = cx.http_client();
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            // 只要目录：文件名的形态（`标题 P2.m4v` + `.m4a`）是我们定的，
            // 让用户在保存对话框里改扩展名反而会写出对不上号的文件。
            directories: true,
            multiple: false,
            prompt: Some("存到这里".into()),
        });

        let queue = self.queue.clone();
        self.player_note = format!("等待选目录…（{}）", request.file_stem).into();
        cx.spawn(async move |this, cx| {
            // 这一串剥的是三层壳：`await` 给 `Result<平台结果, RecvError>`，
            // 平台结果是 `Result<Option<Vec<PathBuf>>, anyhow::Error>`，
            // 最里面的 `None` 才是"用户按了取消"。任何一层失败都当"没选目录"处理。
            let picked = receiver
                .await
                .ok()
                .and_then(|inner| inner.ok())
                .flatten()
                .and_then(|mut paths| paths.pop());
            let Some(dir) = picked else {
                let Some(view) = this.upgrade() else { return };
                view.update(cx, |view, cx| {
                    view.player_note = "没有选目录，这次下载没开始".into();
                    cx.notify();
                });
                return;
            };

            let Some(view) = this.upgrade() else { return };
            // `enqueue` 里会做重名检查并 spawn 任务，所以要在主线程这次 update 里完成；
            // 失败原因（比如清洗后名字为空）直接写回播放那一行。
            let outcome = view.update(cx, |_view, cx| {
                queue.update(cx, |queue, cx| queue.enqueue(request, dir, http, cx))
            });
            let Some(view) = this.upgrade() else { return };
            view.update(cx, |view, cx| {
                view.player_note = match outcome {
                    Ok(name) => format!("已开始下载：{name}（进度在左侧「下载」那一页）").into(),
                    Err(message) => message.into(),
                };
                cx.notify();
            });
            relogin::flush(cx);
        })
        .detach();
    }

    /// 换清晰度。
    ///
    /// 现在的做法是**重新取地址并换一个 mpv 实例**（窗口会回到开头）。同实例用 `loadfile` +
    /// `audio-add` 换流才能保住窗口与进度，那是 P6 的事（管道机制已核实并留档在
    /// `docs/architecture.md` 第 4.14 节）。先把按钮做出来是有意义的：
    /// 它能立刻暴露"哪个档位其实拿不到流"这类问题。
    ///
    /// 这一排按钮里的档位**已经过 `config.json` 的封顶过滤**（见
    /// [`quality_options`](crate::api::stream::quality_options)）：以前能给 4K/HDR 的按钮，
    /// 点下去就把我们自己的 1080P 上限顶穿了。
    fn play_with_quality(&mut self, qn: u64, cx: &mut Context<Self>) {
        let Some(page) = self.selected_page() else {
            self.player_note = "这条稿件没有可播的分 P".into();
            cx.notify();
            return;
        };
        self.request_plan(page.cid, qn, PlanPurpose::Play, cx);
    }

    /// 为一个 cid 取播控地址。结果回来时如果用户已经改点了别的 P（或改了目的），这份结果就丢掉。
    fn request_plan(&mut self, cid: u64, qn: u64, purpose: PlanPurpose, cx: &mut Context<Self>) {
        let Some(bvid) = self.detail.as_ref().map(|info| info.bvid.clone()) else {
            return;
        };
        self.planning = Some((cid, purpose));
        self.player_note = format!(
            "取播控地址…（想要 qn={qn}，{}）",
            match purpose {
                PlanPurpose::Play => "为了播",
                PlanPurpose::Download => "为了下载",
            }
        )
        .into();
        cx.notify();

        let join = stream::fetch_streams(Arc::clone(&self.client), bvid, cid, qn);
        cx.spawn(async move |this, cx| {
            let outcome = match join.await {
                Ok(inner) => inner.map_err(|error| relogin::message(&error)),
                Err(join_error) => Err(crate::app::describe_join_error(join_error)),
            };
            log::debug!("详情页：播控地址已回到前台（cid={cid}）");

            let Some(view) = this.upgrade() else { return };
            view.update(cx, |view, cx| {
                if view.planning != Some((cid, purpose)) {
                    log::debug!("详情页：cid={cid} 的地址已过时或目的已改变，丢弃");
                    return;
                }
                view.planning = None;
                match outcome {
                    Ok(plan) => {
                        let page = view.pages_of(plan.cid);
                        view.plan = Some(plan);
                        match purpose {
                            PlanPurpose::Play => view.start_playback(cx),
                            PlanPurpose::Download => {
                                if let Some(page) = page {
                                    // `ask_destination` 会自己 spawn 等对话框，
                                    // 所以这里可以在这一次 update 里调它。
                                    view.download_with_plan(&page, cx);
                                } else {
                                    view.player_note = "这个分 P 已经不在列表里了".into();
                                }
                            }
                        }
                    }
                    Err(message) => {
                        view.plan = None;
                        view.player_note = format!("取播控地址失败：{message}").into();
                    }
                }
                cx.notify();
            });
            relogin::flush(cx);
        })
        .detach();
    }

    /// 按 cid 找回那一 P（下载要用它的页码与标题拼文件名）。
    fn pages_of(&self, cid: u64) -> Option<PageInfo> {
        let info = self.detail.as_ref()?;
        info.pages.iter().find(|page| page.cid == cid).cloned()
    }

    /// 地址已经在手上了，直接进"问目录"那一步。
    fn download_with_plan(&mut self, page: &PageInfo, cx: &mut Context<Self>) {
        let Some(plan) = self.plan.clone() else {
            return;
        };
        self.ask_destination(plan, page, cx);
    }

    /// 用已取到的地址拉起 mpv（独立窗口）。
    ///
    /// **交给它之前先探一次地址**（P13）：主地址不通就按顺序换 `backupUrl`。
    /// 为什么必须先探：mpv 内部用的地址我们看不到也换不了，"失败之后再换"这一路做不到，
    /// 机制与口径见 [`api::probe`](crate::api::probe)。探测跑在 gpui 前台任务里，
    /// 用的就是加载封面图那个 `HttpClient`（不引第二套 HTTP 栈）。
    ///
    /// 探测期间用户又点了播放或换了 P：`probe_seq` 变了，这一份结果直接丢掉。
    fn start_playback(&mut self, cx: &mut Context<Self>) {
        let Some(plan) = self.plan.clone() else { return };
        let http = cx.http_client();
        let executor = cx.background_executor();
        let headers = probe::cdn_headers();
        let video_urls = probe::ordered_urls(&plan.video_url, &plan.backup_urls);
        let audio_urls = plan
            .audio_url
            .as_ref()
            .map(|url| probe::ordered_urls(url, &plan.audio_backup_urls));
        self.probe_seq += 1;
        let seq = self.probe_seq;
        self.player_note = "探测 CDN 地址…".into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let video = probe::probe_track(
                Arc::clone(&http),
                video_urls,
                headers.clone(),
                executor.clone(),
            )
            .await;
            let audio = match audio_urls {
                Some(urls) => Some(
                    probe::probe_track(Arc::clone(&http), urls, headers.clone(), executor.clone())
                        .await,
                ),
                None => None,
            };
            log::debug!(
                "详情页：CDN 探测完成 视频={} 音频={:?}",
                video.attempted,
                audio.as_ref().map(|one| one.attempted)
            );
            let Some(view) = this.upgrade() else { return };
            view.update(cx, |view, cx| {
                if view.probe_seq != seq {
                    log::debug!("详情页：探测结果已过时（用户又点了一次），丢弃");
                    return;
                }
                view.launch(plan, headers, video, audio);
                cx.notify();
            });
            relogin::flush(cx);
        })
        .detach();
    }

    /// 地址定了，真正拉进程。不 notify，调用方负责。
    ///
    /// `video`/`audio` 是探测结果：探到可用地址就用它，**全不通就退回计划里的主地址**
    /// （理由见 [`probe`] 文件头最后一条：探测本身可能误判，不能因为它彻底不给播）。
    /// 探测换来的每一句说明都拼在提示最前面——用户要能看出"这次播的是备用域名"。
    fn launch(
        &mut self,
        plan: StreamChoice,
        headers: Vec<(String, String)>,
        video: ProbeOutcome,
        audio: Option<ProbeOutcome>,
    ) {
        let video_url = video.url.clone().unwrap_or_else(|| plan.video_url.clone());
        let audio_url = match (plan.audio_url.clone(), audio.as_ref()) {
            (Some(primary), Some(outcome)) => outcome.url.clone().unwrap_or(primary),
            (primary, _) => primary,
        };
        let mut notes: Vec<String> = Vec::new();
        for (label, outcome) in [
            ("视频", Some(video)),
            ("音频", audio),
        ] {
            if let Some(one) = outcome {
                let text = probe::describe(label, &one);
                if !text.is_empty() {
                    notes.push(text);
                }
            }
        }
        let prefix = if notes.is_empty() {
            String::new()
        } else {
            format!("{}。", notes.join("；"))
        };

        // 上一次也是我们拉起的实例：先收掉，否则点几次就堆几个窗口。
        if let Some(mut previous) = self.player.take() {
            log::info!("再次起播，收掉上一个 mpv pid={}", previous.id());
            let _ = previous.kill();
        }

        // 1080P/高码率的 m4s 直链要校验登录 cookie；bpi-rs 不把它的 cookie jar 交出来，
        // 所以由我们从凭据里拼，和 Referer/UA 一起给 mpv（探测用的就是这一组头，两边口径一致）。
        let extra_headers: Vec<String> = headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect();

        match crate::player::launch(&video_url, audio_url.as_deref(), &extra_headers) {
            Ok(child) => {
                let pid = child.id();
                self.player = Some(child);
                let track = match (plan.is_dash, plan.audio_url.as_ref()) {
                    (true, Some(_)) => "DASH 双流",
                    (true, None) => "DASH 只有视频轨（音频接口没给流）",
                    (false, _) => "durl 单文件",
                };
                self.player_note = format!(
                    "{prefix}已交给 mpv（pid {pid}）：{} · {} · {}。暂停/进度/字幕都在播放窗口里操作。",
                    plan.quality_label, plan.codecs, track
                )
                .into();
            }
            Err(error) => {
                self.player_note = format!("{prefix}起播失败：{error}").into();
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
                    // 下载走的是同一条取址路，只是下一步不同（见 `download_selected`）。
                    // 这一颗按钮是**全客户端唯一会往用户磁盘写文件**的入口。
                    Button::new("detail-download")
                        .label("下载这一 P")
                        .on_click(cx.listener(|this, _, _window, cx| this.download_selected(cx))),
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
        // 元素 id 用 `rpid`：它在一次列表里唯一，翻页/换排序也不会让两行共用一个选中区
        // （用行号的话，翻页后第 1 行会继承上一页第 1 行的选区状态）。
        let author_id = format!("detail-comment-author-{}", item.rpid);
        let message_id = format!("detail-comment-{}", item.rpid);
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
                                    .child(SelectableText::new(author_id, item.author.clone())),
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
                    // 做成可选中的（P11）：要复制一段评论时，"选中它"比"整条评论进剪贴板"更符合直觉。
                    .child(
                        div()
                            .w_full()
                            .text_size(px(palette::FONT_BODY))
                            .text_color(palette::text_primary())
                            .line_height(px(palette::LINE_HEIGHT_BODY))
                            .child(SelectableText::new(message_id, item.message.clone())),
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
                // 标题也做成可选中：读详情页的人第一个想复制的就是它。
                div()
                    .w_full()
                    .text_size(px(palette::FONT_TITLE))
                    .text_color(palette::text_primary())
                    .child(SelectableText::new("detail-title", info.title.clone())),
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
                selectable_note(info.desc.clone())
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

    /// 分集那一节（右栏最上面）。没有 `ugc_season` 时返回空列表，右栏就什么都不摆——
    /// **不摆一个"本稿件不在合集里"的空标题**，那对单发稿件是噪音。
    fn season_block(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(brief) = self.detail.as_ref().and_then(|info| info.season.as_ref()) else {
            return Vec::new();
        };
        let mut out: Vec<AnyElement> = vec![layout::section_title(format!("分集 · {}", brief.title))];

        // 那一行读数：`第 168 / 189 集`。三个数字各有出处，缺一个就少说一段：
        // `current_index` 是我们在这份列表里找到的位置（0 = 没找到，说明接口没把这一集列进来），
        // `ep_count` 是接口声明的总集数，`listed` 是本次真正拿到的条数。
        // 后两个不等时把差额写出来，别让用户以为这合集只有这些。
        let mut readout: Vec<String> = Vec::new();
        if brief.current_index > 0 {
            readout.push(format!("第 {} 集", brief.current_index));
        } else {
            readout.push("当前这一集不在列出的名单里".to_string());
        }
        if brief.ep_count > 0 {
            readout.push(format!("共 {} 集", brief.ep_count));
        }
        if brief.ep_count as usize != brief.listed {
            readout.push(format!("本次列出 {} 集", brief.listed));
        }
        if brief.skipped > 0 {
            readout.push(format!("另有 {} 条打不开", brief.skipped));
        }
        out.push(
            div()
                .w_full()
                .px(px(palette::LIST_ROW_PADDING_X))
                .pb(px(palette::CARD_TEXT_ROW_GAP))
                .text_size(px(palette::FONT_CAPTION))
                .text_color(palette::text_tertiary())
                .child(readout.join(" · "))
                .into_any_element(),
        );

        // 分段 chips：一个分段时不给（那一排按钮只会占掉 360 宽里的一行，什么也没选）。
        if brief.has_sections() {
            let mut chips: Vec<AnyElement> = Vec::new();
            for (index, section) in brief.sections.iter().enumerate() {
                let selected = index == self.season_section;
                let label = if section.title.is_empty() {
                    format!("分段 {}", index + 1)
                } else {
                    section.title.clone()
                };
                let button = Button::new(format!("season-section-{index}"))
                    .label(label)
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.season_section = index;
                        cx.notify();
                    }));
                chips.push(if selected { button.primary() } else { button }.into_any_element());
            }
            out.push(
                div()
                    .flex()
                    .flex_wrap()
                    .w_full()
                    .gap(px(palette::CARD_TEXT_ROW_GAP))
                    .px(px(palette::LIST_ROW_PADDING_X))
                    .pb(px(palette::CARD_TEXT_ROW_GAP))
                    .children(chips)
                    .into_any_element(),
            );
        }

        // 用户可能点了分段后又换了稿件：下标夹在有效范围内再取（`saturating_sub` 防 0 减 1 下溢）。
        let index = self
            .season_section
            .min(brief.sections.len().saturating_sub(1));
        let Some(section) = brief.sections.get(index) else {
            return out;
        };
        // 行号在整个合集里连续（不是每个分段从 1 重新开始）：这样"第 168 集"这种读数
        // 与左边那一列数字对得上，用户找下一集不用心算。
        let offset: usize = brief
            .sections
            .iter()
            .take(index)
            .map(|one| one.episodes.len())
            .sum();
        out.extend(self.season_rows(section, offset, cx));
        out
    }

    /// 一个分段里的每一集。点一行 = 请外壳再压一层详情页（与卡片同一条路）。
    fn season_rows(
        &self,
        section: &SeasonSectionBrief,
        offset: usize,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        section
            .episodes
            .iter()
            .enumerate()
            .map(|(index, episode)| {
                let current = episode.bvid == self.bvid.as_str();
                let meta = {
                    let mut parts: Vec<String> = vec![format!("{:02}", offset + index + 1)];
                    // `arc.videos > 1` 就是他说的"情况比较复杂"：合集里的一集自己还是个多 P 稿件。
                    // 这一格必须显出来，否则用户会以为点进去就是整集。
                    if episode.videos > 1 {
                        parts.push(format!("{} P", episode.videos));
                    }
                    if !episode.duration_text.is_empty() {
                        parts.push(episode.duration_text.clone());
                    }
                    parts.join(" · ")
                };
                let bvid = episode.bvid.clone();
                let base = div()
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
                    // 与"选集"那些行同一套选中口径：底色 + 那条常驻 2px 边框换色，不产生位移。
                    .bg(if current {
                        palette::card_background_elevated()
                    } else {
                        palette::card_background()
                    })
                    .border_2()
                    .border_color(if current {
                        palette::accent()
                    } else {
                        palette::border_none()
                    })
                    .text_color(if current {
                        palette::text_primary()
                    } else {
                        palette::text_secondary()
                    })
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(palette::FONT_CAPTION))
                            .text_color(palette::text_tertiary())
                            .child(meta),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_size(px(palette::FONT_BODY))
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .overflow_hidden()
                            .child(episode.title.clone()),
                    );
                base.id(format!("season-{}", episode.bvid))
                    .on_click(cx.listener(move |_this, _event, _window, cx| {
                        cx.emit(OpenVideo { bvid: bvid.clone() });
                    }))
                    .hover(|style| style.bg(palette::card_background_hover()))
                    .into_any_element()
            })
            .collect()
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
                    // 分集在最上面：这一集"属于哪个合集、上一集下一集是谁"是进详情页第一眼要的答案，
                    // 而"这一集自己分几个 P"（下面那节"选集"）是第二层信息。
                    .children(self.season_block(cx))
                    // 有分集时这一节的标题要说清是"本集的分 P"：上面那节已经叫"分集"了，
                    // 两节都叫"选集"会让人以为下面是同一份列表的另一半。
                    .child(layout::section_title(if self
                        .detail
                        .as_ref()
                        .is_some_and(|info| info.season.is_some())
                    {
                        "本集的分 P"
                    } else {
                        "选集"
                    }))
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

        // 复制：键盘用户按 Ctrl+C（`base::Root` 装了那个动作），触摸屏没有键盘，
        // 所以页头右侧给一颗读同一个窗口选区的按钮。选区是**窗口级**的，
        // 于是它复制的是这一段里任何一处选中的文字，不需要页面自己再存一份。
        let copy_button = Button::new("detail-copy-selection")
            .label("复制选中")
            .on_click(|_event, window, cx| {
                let text = TextSelection::selected_text(window, cx);
                if text.is_empty() {
                    log::debug!("复制选中：当前窗口没有选区（先用手指或鼠标选中一段文字）");
                    return;
                }
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            })
            .into_any_element();

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(layout::header("视频详情", head_subtitle, Some(copy_button)))
            .child(div().flex_1().w_full().min_h(px(0.)).child(body))
    }
}
