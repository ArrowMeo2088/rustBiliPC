//! 下载页：这一排任务，加上每条任务能做的三个动作。
//!
//! ## 这一页不再只是入口
//!
//! 2026-10-26 之前它是占位（只有侧栏那一格）。P11 把它做成真的：**下载是唯一能把文件写到
//! 运行目录之外的功能**（用户原话"仅这个能下载到外部文件里"），所以这一页也就是全客户端
//! 唯一会碰用户磁盘的界面——写哪儿、写成了什么、失败了为什么，都必须在这一页看得见。
//!
//! ## 三个动作的边界
//!
//! - **取消**：只置标志（机制与理由见 [`queue`](crate::pages::download::queue) 文件头）。
//!   界面上立刻写"正在取消…"，等任务把当前这块写完再删临时文件。
//! - **打开所在目录**：交给系统的 `reveal_path`。我们不自己拼 `explorer.exe` 命令行——
//!   参数里那种 `/select,` 语法在 gpui 已经有实现（`gpui_windows/src/platform.rs` 的 `reveal_path`）。
//! - **用 mpv 打开**：把本地两个文件当参数交出去（`--audio-file=` 那条老路），
//!   这样下完不必再去文件夹里手挑。子进程句柄**必须留着**，否则换一条视频打开时
//!   上一个实例就成了关不掉的"声音还在"（QODER.md"子进程必须有归宿"）。
//! - **移除记录**：只从列表里去掉这一行，磁盘上的成品不动。
//!
//! ## 为什么进度条要接受"没有分母"
//!
//! B 站的 `dash.video[].size` 是 `Option`（`ref/bpi-rs/src/video/videostream_url.rs:63`），
//! 拿不到大小时画不出百分比。这时走 `Progress::loading(true)` 的不定进度 + 已下字节数，
//! **不猜**一个总数出来——猜出来的 100% 比没有进度更容易骗人。

use std::path::PathBuf;

use gpui_kit::component::button::Button;
use gpui_kit::component::progress::Progress;
use gpui_kit::*;

use crate::api::download;
use crate::pages::download::queue::{
    DownloadQueue, DownloadTask, DownloadsChanged, TaskState, MAX_RUNNING,
};
use crate::ui::scroll::DragScroll;
use crate::ui::{layout, palette};

/// 渲染用的一行快照。
///
/// 先把队列读成这个再建元素：`cx.listener` 要 `&mut Context`，而 `queue.read(cx)` 还借着
/// 同一个 `cx`——直接在里面建按钮会 borrowck。别的页面（收藏夹内容、历史）也是这个套路。
#[derive(Clone)]
struct TaskRow {
    id: u64,
    label: SharedString,
    state: TaskState,
    done: u64,
    total: Option<u64>,
    note: SharedString,
    dir: PathBuf,
    /// 只有"已完成"才带着两个成品路径（其余状态下面是"将要写的路径"，不能当可用文件打开）。
    files: Vec<PathBuf>,
}

impl TaskRow {
    fn of(task: &DownloadTask) -> Self {
        Self {
            id: task.id,
            label: task.label.clone(),
            state: task.state,
            done: task.done,
            total: task.total,
            note: task.note.clone(),
            dir: task.dir.clone(),
            files: task.finished_files().to_vec(),
        }
    }

    /// 一行右侧的读数：`3.4 MB / 86.2 MB`，没分母就只写已下多少。
    fn readout(&self) -> String {
        let done = download::format_bytes(self.done);
        match self.total {
            Some(total) => format!("{done} / {}", download::format_bytes(total)),
            None => done,
        }
    }
}

/// 下载页根视图。
pub struct DownloadPage {
    queue: Entity<DownloadQueue>,
    scroll: DragScroll,
    /// "用 mpv 打开"拉起的实例（同详情页的规矩：留着才 kill 得掉）。
    player: Option<std::process::Child>,
    /// 这一页自己那一路动作的提示（打开成品成功/失败）。空串表示没话要说。
    ///
    /// 为什么要有它：`launch()` 失败只会写进日志，而**日志是给人查的、不是给人看的**
    /// （第 4.7 节"失败必须看得见"），点完按钮界面上没反应就等于没告诉用户为什么没窗口。
    note: SharedString,
    _subscription: Subscription,
}

/// 页面退出时收掉自己拉起的 mpv（与 `DetailPage` 同一条理由）。
impl Drop for DownloadPage {
    fn drop(&mut self) {
        if let Some(child) = self.player.as_mut() {
            log::info!("下载页退出，收掉 mpv pid={}", child.id());
            let _ = child.kill();
        }
    }
}

impl DownloadPage {
    /// 建页。`queue` 由外壳注入——任务是从详情页点出来的，队列不归这一页所有。
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        queue: Entity<DownloadQueue>,
    ) -> Self {
        let subscription = cx.subscribe_in(
            &queue,
            window,
            |_this, _queue, _event: &DownloadsChanged, _window, cx| {
                // 队列变了就重绘：这一页不缓存任何进度数字，读的就是队列本身。
                cx.notify();
            },
        );
        Self {
            queue,
            scroll: DragScroll::default(),
            player: None,
            note: "".into(),
            _subscription: subscription,
        }
    }

    /// 用 mpv 打开一条已完成的任务（视频 + 可选音频）。
    ///
    /// 走的是在线播那同一个 `player::launch`：本地路径对它来说就是"最后一个位置参数"，
    /// 多给的那几个 HTTP 头对本地文件无效也无害，不值得为它开一条新的参数拼装路。
    fn open_files(&mut self, files: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(video) = files.first() else {
            return;
        };
        let video_text = video.to_string_lossy().to_string();
        let audio_text = files
            .get(1)
            .map(|path| path.to_string_lossy().to_string());

        if let Some(mut previous) = self.player.take() {
            log::info!("再次打开成品，收掉上一个 mpv pid={}", previous.id());
            let _ = previous.kill();
        }

        match crate::player::launch(&video_text, audio_text.as_deref(), &[]) {
            Ok(child) => {
                let pid = child.id();
                log::info!("已用 mpv 打开成品：{video_text}");
                // 句柄必须留下：`Child` 的 drop 不杀子进程，留下的就是"声音还在、没人能关"。
                self.player = Some(child);
                self.note = format!("已交给 mpv（pid {pid}）：{video_text}").into();
            }
            Err(error) => {
                log::error!("用 mpv 打开失败: {error}");
                self.note = format!("打开失败：{error}").into();
            }
        }
        cx.notify();
    }

    /// 一行任务。
    fn task_row(&self, row: TaskRow, cx: &mut Context<Self>) -> AnyElement {
        let id = row.id;
        let files = row.files.clone();

        // 进度条：跑动用百分比（拿不到大小就退成不定进度）；"已完成"画满；
        // **排队中**、失败与取消都不画线——那三行要人读的是"还没轮到"或原因文字，
        // 不是一条停在 0% 的线（0% 的线看起来像"卡住了"）。
        let bar = match row.state {
            TaskState::Running => {
                let bar = Progress::new(format!("download-bar-{id}"))
                    .w_full()
                    .h(px(palette::PROGRESS_BAR_HEIGHT))
                    .color(palette::accent());
                match download::percent(row.done, row.total) {
                    Some(value) => bar.value(value),
                    None => bar.loading(true),
                }
                .into_any_element()
            }
            TaskState::Finished => Progress::new(format!("download-bar-{id}"))
                .w_full()
                .h(px(palette::PROGRESS_BAR_HEIGHT))
                .value(100.)
                .color(palette::accent())
                .into_any_element(),
            _ => div().w_full().into_any_element(),
        };

        // 按钮：按状态给，不给点了没反应的按钮。
        let mut actions: Vec<AnyElement> = Vec::new();
        // 排队中的也要给"取消"：不然那条记录唯一的入口就是"等它开始下"，
        // 而人家明显不想下第五条（点了才发现要排队，此刻能反悔才有意义）。
        if row.state.active() {
            actions.push(
                Button::new(format!("download-cancel-{id}"))
                    .label("取消")
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.queue.update(cx, |queue, cx| queue.cancel(id, cx));
                    }))
                    .into_any_element(),
            );
        }
        if row.state == TaskState::Finished && !files.is_empty() {
            let reveal_dir = row.dir.clone();
            actions.push(
                Button::new(format!("download-reveal-{id}"))
                    .label("打开所在目录")
                    .on_click(move |_event, _window, cx| {
                        cx.reveal_path(&reveal_dir);
                    })
                    .into_any_element(),
            );
            let open_files = files.clone();
            actions.push(
                Button::new(format!("download-mpv-{id}"))
                    .label("用 mpv 打开")
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        // 路径来自这一行的快照，不再回队列里取：记录可能已经被别的回调挪走了。
                        this.open_files(open_files.clone(), cx);
                    }))
                    .into_any_element(),
            );
        }
        // 还在跑或还排着的**不给"移除记录"**：那是这条任务唯一的入口，
        // 摘掉之后取消按钮就没了（排队的尤其危险——它会一直等到有名额时自己开跑）。
        if !row.state.active() {
            actions.push(
                Button::new(format!("download-forget-{id}"))
                    .label("移除记录")
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.queue.update(cx, |queue, cx| queue.forget(id, cx));
                    }))
                    .into_any_element(),
            );
        }

        let detail_text = if row.state == TaskState::Finished {
            // 成品名直接写出来：用户要的是"到底落成了哪两个文件"。
            row.files
                .iter()
                .map(|path| {
                    path.file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.to_string_lossy().to_string())
                })
                .collect::<Vec<String>>()
                .join(" + ")
        } else if row.state == TaskState::Pending {
            // 排队中的那一行必须自己解释自己：否则它和"卡住的下载"在界面上长得一样。
            format!("等前面的下完就轮到（同时最多 {MAX_RUNNING} 条）")
        } else {
            row.readout()
        };

        let mut lines: Vec<AnyElement> = vec![
            div()
                .flex()
                .w_full()
                .items_center()
                .gap(px(palette::CARD_TEXT_ROW_GAP))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(palette::FONT_BODY))
                        .text_color(palette::text_primary())
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .overflow_hidden()
                        .child(row.label.clone()),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(palette::FONT_CAPTION))
                        .text_color(if row.state == TaskState::Failed {
                            palette::critical()
                        } else {
                            palette::text_tertiary()
                        })
                        .child(row.state.label()),
                )
                .into_any_element(),
            bar,
            div()
                .w_full()
                .text_size(px(palette::FONT_CAPTION))
                .text_color(palette::text_tertiary())
                .child(detail_text)
                .into_any_element(),
        ];
        if !row.note.is_empty() {
            lines.push(
                div()
                    .w_full()
                    .text_size(px(palette::FONT_CAPTION))
                    .line_height(px(palette::LINE_HEIGHT_BODY))
                    .text_color(if row.state == TaskState::Failed {
                        palette::critical()
                    } else {
                        palette::text_secondary()
                    })
                    .child(row.note.clone())
                    .into_any_element(),
            );
        }
        lines.push(
            div()
                .flex()
                .w_full()
                .gap(px(palette::CARD_TEXT_ROW_GAP))
                .children(actions)
                .into_any_element(),
        );

        layout::card(lines)
    }
}

impl Render for DownloadPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.scroll.watch(window, cx.entity_id());

        let (rows, running, waiting, finished) = {
            let queue = self.queue.read(cx);
            let rows: Vec<TaskRow> = queue.tasks().iter().map(TaskRow::of).collect();
            let running = queue.running();
            let waiting = queue.waiting();
            let finished = rows
                .iter()
                .filter(|row| matches!(row.state, TaskState::Finished | TaskState::Cancelled))
                .count();
            (rows, running, waiting, finished)
        };

        // 显式标类型：这一条 `if/else` 的 `None` 分支要靠后面的 `header` 才推得出来，
        // 不写就是 E0282（第三十轮那两条歧义 collect 是同一类错）。
        // 排队中的数目一定要写出来：不说"为什么这条停在排队中"，它就长得像"下载坏了"。
        let subtitle: Option<SharedString> = if rows.is_empty() {
            None
        } else if waiting > 0 {
            Some(
                format!(
                    "共 {} 条 · 进行中 {running} · 排队中 {waiting}（同时最多下 {MAX_RUNNING} 条）",
                    rows.len()
                )
                .into(),
            )
        } else {
            Some(format!("共 {} 条 · 进行中 {running}", rows.len()).into())
        };
        let trailing: Option<AnyElement> = if finished > 0 {
            Some(
                Button::new("download-clear")
                    .label("清掉已完成")
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        this.queue.update(cx, |queue, cx| queue.clear_completed(cx));
                    }))
                    .into_any_element(),
            )
        } else {
            None
        };

        let body: AnyElement = if rows.is_empty() {
            div()
                .flex()
                .flex_col()
                .w_full()
                .gap(px(palette::CARD_TEXT_ROW_GAP))
                .px(px(palette::CARD_TEXT_PADDING_X))
                .py(px(palette::BODY_PADDING_Y))
                .child(
                    div()
                        .text_size(px(palette::FONT_BODY))
                        .text_color(palette::text_primary())
                        .child("还没有下载任务。"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .w_full()
                        .gap(px(palette::CARD_TEXT_ROW_GAP))
                        .text_size(px(palette::FONT_CAPTION))
                        .text_color(palette::text_secondary())
                        .children(
                            [
                                "进视频详情页，点「下载这一 P」，在弹出来的窗口里选一个目录就开始。",
                                "下出来是两个文件（视频 .m4v + 音频 .m4a），画质取到设置里那个封顶档以内；只下当前选中的那一 P。",
                                "这一版不支持断点续传：中途取消就是把已经写的删掉，下次从头下。",
                            ]
                            .into_iter()
                            .map(|line| {
                                div()
                                    .w_full()
                                    .line_height(px(palette::LINE_HEIGHT_BODY))
                                    .child(line)
                            }),
                        ),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .w_full()
                .gap(px(palette::LIST_GAP))
                .px(px(palette::CARD_TEXT_PADDING_X))
                .py(px(palette::BODY_PADDING_Y))
                .children(rows.into_iter().map(|row| self.task_row(row, cx)))
                .into_any_element()
        };

        // 提示那一行：没话就画一个空 div（少一个 `when_some` 的导入，也让布局层数恒定）。
        let note_line = if self.note.is_empty() {
            div().into_any_element()
        } else {
            div()
                .w_full()
                .px(px(palette::CARD_TEXT_PADDING_X))
                .py(px(palette::CARD_TEXT_ROW_GAP))
                .text_size(px(palette::FONT_CAPTION))
                .text_color(palette::text_secondary())
                .child(self.note.clone())
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(layout::header("下载", subtitle, trailing))
            .child(note_line)
            .child(
                div()
                    .id("download-body")
                    .flex_1()
                    .w_full()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(self.scroll.handle())
                    .on_mouse_down(MouseButton::Left, self.scroll.on_press())
                    .child(body),
            )
    }
}
