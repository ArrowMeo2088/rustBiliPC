//! 下载队列：界面与 API runtime 之间唯一的那条缝。
//!
//! ## 为什么队列是一个实体而不是下载页的字段
//!
//! 任务是从**详情页**点出来的，列表在**下载页**看，两边都归外壳持有。把队列塞进下载页，
//! 详情页就得知道另一个页面；做成外壳建的 `Entity<DownloadQueue>` 注入两边，
//! 页面之间还是互不认识（外壳本来就是唯一知道全部页面的那一层）。
//!
//! ## 状态只有一个出处
//!
//! tokio 任务只往 `mpsc::UnboundedReceiver<DownloadEvent>` 里塞进度与终态，
//! **不共享任何可变状态**（没有 `Arc<Mutex<Vec<..>>>`，也没有给界面读的原子计数）：
//! 所有字段都在主线程的 [`DownloadTask`] 里，改它们的路径只有 [`apply`](Self::apply) 一处。
//! 这么定的理由：一边是 tokio 线程、一边是 gpui 主线程，只要有一张表两边都写，
//! 就必然出现"界面读到写了一半的任务"这种查不动的问题。
//!
//! ## 取消为什么不能 `abort()`
//!
//! 见 [`crate::api::download`] 文件头第 2 条：`abort` 会在下一个 await 点丢掉任务，
//! 它手里的 `File`/`BufWriter` 还没 flush 就被析构，而主线程已经在删同一个文件了
//! （Windows 上"文件还开着就删"会失败）。所以这里只置标志，等任务自己收尾。
//!
//! ## 队列不持久化，但**退出时会叫停在跑的任务**
//!
//! 关掉客户端，这一排记录就没了（磁盘上的成品还在）。要做"下次打开还在"就得再写一份状态文件，
//! 而那与"只有下载能写到用户目录之外"这条口径是两回事——没让他点头之前不做。
//! 不过"进程走了、`.part` 留在别人盘上"不是持久化问题而是**收尾**问题：
//! 所以 [`crate::api::download::cancel_all_active`] 会在外壳实体被释放时、以及 `main.rs` 里
//! 我们主动 `cx.quit()` 之前各叫一次（置标志，让任务自己 flush → 关句柄 → 删临时文件）。
//! **诚实的限制**：gpui 没有"最后一条窗口关闭 / 即将退出"的回调（只有 `App::on_window_closed`，
//! 而它要 `WindowId` 且得有人长期持有 `Subscription`），所以"用户点 X 退出"这一路
//! 只能靠 `DownloadQueue` 的 `Drop` 兜住；进程被任务管理器杀掉时仍然会留 `.part`。
//!
//! ## 同时在跑的条数有上限（P14）
//!
//! 第一版是"点五条就是五条在跑"。现在是 **[`MAX_RUNNING`] = 3**：多出来的排在队列里
//! （界面上写"排队中"，也能直接取消），有任务进终态就自动补位。
//! 限的是**同时开写的任务数**，不是"同时存在的记录数"——记录想留多少留多少。
//! 为什么排在队列而不是干脆拒绝：他从详情页点下去的那一下已经付过一次弹框的代价，
//! 告诉他"太挤了没排上"比让他等一下更没用。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use gpui_kit::http_client::HttpClient;
use gpui_kit::*;

use crate::api::download::{self, DownloadEvent, DownloadRequest};

/// 一条任务走到哪一步了。文字原因另存在 [`DownloadTask::note`]，这两个枚举不承担文案。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    /// 还没轮到它（同时在跑的已经有 [`MAX_RUNNING`] 条）。
    Pending,
    Running,
    Finished,
    Failed,
    Cancelled,
}

impl TaskState {
    /// 列表上那一行的状态词。
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "排队中",
            Self::Running => "下载中",
            Self::Finished => "已完成",
            Self::Failed => "失败",
            Self::Cancelled => "已取消",
        }
    }

    /// 还在占着名额或等着占名额的状态（页头那句"进行中/排队"要用，也用来挡住"移除记录"）。
    pub fn active(self) -> bool {
        matches!(self, Self::Pending | Self::Running)
    }
}

/// 同时在跑几条下载。
///
/// 为什么是 3 而不是"想点几条点几条"：每条任务都是 256 KiB 一块地读+写，
/// 四条一起跑就把带宽和磁盘寻道切成碎片，**总耗时不变而每条都更慢**，
/// 而且界面上一排"都卡在半途"比"三条在动、两条排队"更难读。
pub const MAX_RUNNING: usize = 3;

/// 一条下载记录（界面只看这些字段）。
pub struct DownloadTask {
    pub id: u64,
    /// 给人看的一行字（标题 + 画质）。
    pub label: SharedString,
    /// 这次写进的那个目录（弹框选的）。
    pub dir: PathBuf,
    /// 成品路径。运行中是"将要写的路径"，完成后是确实存在的文件。
    pub files: Vec<PathBuf>,
    /// 分母；`None` 表示 B 站没给大小，进度条走不定进度。
    pub total: Option<u64>,
    pub done: u64,
    pub state: TaskState,
    /// 失败原因或"正在取消…"这类临时说明；空串表示没有要补充的。
    pub note: SharedString,
    /// 取消标志（任务在下一个分块处看见它）。
    cancel: Arc<AtomicBool>,
    /// 排队时要靠这两个字段把任务真正 spawn 出去（点下去那一刻只登记，不开跑）。
    /// 已开跑的任务也留着它们：`spawn_download` 收的是克隆，代价是几百字节，
    /// 换来的是"排队与立刻开跑"两条路共用同一个 `start`。
    request: DownloadRequest,
    client: Arc<dyn HttpClient>,
    /// 防重入：`pump` 会被多处调用，一条任务只能被 spawn 一次。
    started: bool,
}

impl DownloadTask {
    /// 已经写完的成品（只有"已完成"才给，避免把半成品当可用文件打开）。
    pub fn finished_files(&self) -> &[PathBuf] {
        if self.state == TaskState::Finished {
            &self.files
        } else {
            &[]
        }
    }
}

/// 队列变了（新任务、进度、终态、移除）——下载页订阅它来重绘。
#[derive(Clone, Debug)]
pub struct DownloadsChanged;

/// 进程内的一排下载任务。
pub struct DownloadQueue {
    tasks: Vec<DownloadTask>,
    next_id: u64,
}

impl DownloadQueue {
    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            next_id: 0,
        }
    }

    /// 给下载页渲染用（最新一条在最下面，与"任务列表"的直觉一致，不反转）。
    pub fn tasks(&self) -> &[DownloadTask] {
        &self.tasks
    }

    /// 还在跑的任务数（页头副标题用它写"下载 · 进行中 2"）。
    pub fn running(&self) -> usize {
        self.count(TaskState::Running)
    }

    /// 排队中的任务数（名额满了才会有）。
    pub fn waiting(&self) -> usize {
        self.count(TaskState::Pending)
    }

    fn count(&self, state: TaskState) -> usize {
        self.tasks.iter().filter(|task| task.state == state).count()
    }

    /// 把一次下载登记进队列（可能立刻开跑，也可能排队）。`Err` 里是给界面的一句话（不产生记录）。
    ///
    /// 重名要在这里处理：目录是弹框选出来的，拿到目录才能看有什么文件已经存在，
    /// 而文件名一旦定下就不能再变（任务与界面都按它找文件）。
    pub fn enqueue(
        &mut self,
        request: DownloadRequest,
        dir: PathBuf,
        client: Arc<dyn HttpClient>,
        cx: &mut Context<Self>,
    ) -> Result<SharedString, String> {
        if request.file_stem.is_empty() {
            return Err("这条稿件的标题清洗之后不能当文件名，换一个来源名".to_string());
        }
        let stem = download::available_stem(&dir, &request.file_stem, &request.extensions());
        if stem != request.file_stem {
            log::info!("下载：目标目录里已有同名文件，改名为 {stem}");
        }
        let video_name = format!("{stem}.{}", request.video_ext);
        let request = request.with_stem(stem);
        let files = request.targets(&dir);
        let total = request.total_bytes();
        let label = SharedString::from(request.label.clone());
        let cancel = Arc::new(AtomicBool::new(false));

        let id = self.next_id;
        self.next_id += 1;
        self.tasks.push(DownloadTask {
            id,
            label,
            dir,
            files,
            total,
            done: 0,
            // 先进 `Pending`，由 `pump` 决定是立刻开跑还是排队：这样"名额满了"这件事
            // 只有一个地方能改状态，不会出现两处都以为自己是开跑的那次。
            state: TaskState::Pending,
            note: "".into(),
            cancel,
            request,
            client,
            started: false,
        });
        self.pump(cx);

        cx.emit(DownloadsChanged);
        cx.notify();
        // 把最终落盘的文件名给调用方：界面上要写的就是"开始下 xxx.m4v"，
        // 而不是用户以为的那个名字（重名时我们改过）。
        Ok(video_name.into())
    }

    /// 有空名额就开跑排队的任务（按点下去的先后，也就是 `tasks` 里的顺序）。
    fn pump(&mut self, cx: &mut Context<Self>) {
        while self.running() < MAX_RUNNING {
            let Some(id) = self
                .tasks
                .iter()
                .find(|task| task.state == TaskState::Pending)
                .map(|task| task.id)
            else {
                break;
            };
            self.start(id, cx);
        }
    }

    /// 真正把一条任务 spawn 到 API runtime 上。只认没开跑过的那条。
    fn start(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) else {
            return;
        };
        if task.started || task.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            // 第二种情况：还在排队时用户就点了取消（见 [`cancel`](Self::cancel) 那条分支）。
            return;
        }
        task.started = true;
        task.state = TaskState::Running;
        let (join, mut receiver) = download::spawn_download(
            task.request.clone(),
            task.dir.clone(),
            Arc::clone(&task.client),
            Arc::clone(&task.cancel),
        );

        // 进度与终态回到主线程：`recv()` 返回 `None` 表示任务那头把发送端放下了（跑完了），
        // 这个循环自己结束，不留悬着的任务。
        cx.spawn(async move |this, cx| {
            while let Some(event) = receiver.recv().await {
                let Some(queue) = this.upgrade() else { break };
                queue.update(cx, |queue, cx| queue.apply(id, event, cx));
            }
        })
        .detach();
        // tokio 的 `JoinHandle` 被 drop **不会**取消任务（那是 gpui 的 `Task` 的语义），
        // 取消靠的是 `cancel` 那个标志，所以这里可以安心丢掉它。
        let _join = join;
    }

    /// 请求取消：在跑的置标志 + 写"正在取消…"，等任务自己发回终态；
    /// **还排着的直接改成已取消**（它压根没开跑，置标志没有意义，而且那样会留一条永远不动的记录）。
    pub fn cancel(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) else {
            return;
        };
        match task.state {
            TaskState::Pending => {
                task.state = TaskState::Cancelled;
                task.note = "已取消（还没开始下，没有写任何文件）".into();
                log::info!("下载：任务 #{id} 在排队时被取消");
            }
            TaskState::Running => {
                task.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                task.note = "正在取消…（等它把手里的这块写完）".into();
                log::info!("下载：已请求取消任务 #{id}");
            }
            other => {
                log::debug!("下载：任务 #{id} 已经是 {:?}，取消请求忽略", other);
                return;
            }
        }
        // 排队的腾出了位置（虽然它不占名额，但"取消排队"之后可能有别的还排着 —— 统一泵一次不亏）。
        self.pump(cx);
        cx.emit(DownloadsChanged);
        cx.notify();
    }

    /// 从列表里去掉一条记录（**不动磁盘**）。还在跑或还排着的不许去：
    /// 那两条都是"这条任务唯一的入口"，去掉就再也关不掉它（排队的至少还能看到"排队中"）。
    pub fn forget(&mut self, id: u64, cx: &mut Context<Self>) {
        let before = self.tasks.len();
        self.tasks
            .retain(|task| !(task.id == id && !task.state.active()));
        if self.tasks.len() != before {
            cx.emit(DownloadsChanged);
            cx.notify();
        }
    }

    /// 去掉所有"已完成"与"已取消"的记录（磁盘上的成品一个都不动）。
    ///
    /// 失败、排队中与进行中的留着：失败那一行是用户唯一能看到原因的地方，一键清掉等于把问题抹掉；
    /// 另两种还没结束，去掉就再没地方点取消。
    pub fn clear_completed(&mut self, cx: &mut Context<Self>) {
        let before = self.tasks.len();
        self.tasks.retain(|task| {
            !matches!(
                task.state,
                TaskState::Finished | TaskState::Cancelled
            )
        });
        if self.tasks.len() != before {
            cx.emit(DownloadsChanged);
            cx.notify();
        }
    }

    /// 把任务发回来的一条消息写进对应记录。找不到就丢掉（记录已被移除是正常情况）。
    fn apply(&mut self, id: u64, event: DownloadEvent, cx: &mut Context<Self>) {
        let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) else {
            return;
        };
        match event {
            // 进度只往前走：换备用域名时那一段丢掉的字节也已经计进 `done`
            // （第四十九轮改的，理由见 `api/download.rs` 的 `pull_track`），
            // 这里再挡一次乱序到达的旧进度，免得百分比抖回去。
            DownloadEvent::Progress { done } if task.state == TaskState::Running => {
                if done > task.done {
                    task.done = done;
                }
            }
            DownloadEvent::Progress { .. } => {}
            DownloadEvent::Finished { files } => {
                // 最后一个不足 1 MiB 的分块不会单独回报，这里按分母补满。
                if let Some(total) = task.total {
                    task.done = task.done.max(total);
                }
                task.files = files;
                task.state = TaskState::Finished;
                task.note = "".into();
            }
            DownloadEvent::Failed { message } => {
                task.state = TaskState::Failed;
                task.note = message.into();
            }
            DownloadEvent::Cancelled => {
                task.state = TaskState::Cancelled;
                task.note = "已取消，这一对没下完的文件已经删掉了".into();
            }
        }
        // 空出一个名额就叫下一条上来（放在终态分支之后：`running()` 已经是减过的值）。
        self.pump(cx);
        cx.emit(DownloadsChanged);
        cx.notify();
    }
}

impl EventEmitter<DownloadsChanged> for DownloadQueue {}

/// 队列被释放（主窗口关掉 → 外壳 drop → 它持有的队列 drop）时叫停在跑的任务，
/// 让它们自己 flush、关句柄、删 `.part`。
///
/// 为什么不能在这里直接删文件：那会儿任务线程可能还开着同一个句柄，Windows 上删不掉，
/// 而且会出现"任务刚改名成成品、主线程把成品当临时文件删了"的先后歧义。
/// 置标志是唯一没有竞态的做法。
impl Drop for DownloadQueue {
    fn drop(&mut self) {
        let stopped = download::cancel_all_active();
        if stopped > 0 {
            log::info!("下载：进程要退了，已请求 {stopped} 条任务收尾（它们会删掉 .part）");
        }
    }
}
