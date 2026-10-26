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
//! ## 队列不持久化
//!
//! 关掉客户端，这一排记录就没了（磁盘上的成品还在）。要做"下次打开还在"就得往运行目录
//! 写第三份状态文件，而那与"只有下载能写到运行目录之外"这条口径是两回事——没让他点头之前不做。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use gpui_kit::http_client::HttpClient;
use gpui_kit::*;

use crate::api::download::{self, DownloadEvent, DownloadRequest};

/// 一条任务走到哪一步了。文字原因另存在 [`DownloadTask::note`]，这两个枚举不承担文案。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    Running,
    Finished,
    Failed,
    Cancelled,
}

impl TaskState {
    /// 列表上那一行的状态词。
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "下载中",
            Self::Finished => "已完成",
            Self::Failed => "失败",
            Self::Cancelled => "已取消",
        }
    }
}

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
        self.tasks
            .iter()
            .filter(|task| task.state == TaskState::Running)
            .count()
    }

    /// 把一次下载排进队列并开跑。`Err` 里是给界面的一句话（不产生记录）。
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
        let (join, mut receiver) =
            download::spawn_download(request, dir.clone(), client, Arc::clone(&cancel));

        let id = self.next_id;
        self.next_id += 1;
        self.tasks.push(DownloadTask {
            id,
            label,
            dir,
            files,
            total,
            done: 0,
            state: TaskState::Running,
            note: "".into(),
            cancel,
        });

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

        cx.emit(DownloadsChanged);
        cx.notify();
        // 把最终落盘的文件名给调用方：界面上要写的就是"开始下 xxx.m4v"，
        // 而不是用户以为的那个名字（重名时我们改过）。
        Ok(video_name.into())
    }

    /// 请求取消：置标志 + 立刻在界面上写"正在取消…"，终态等任务自己发回来。
    pub fn cancel(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) else {
            return;
        };
        if task.state != TaskState::Running {
            return;
        }
        task.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        task.note = "正在取消…（等它把手里的这块写完）".into();
        log::info!("下载：已请求取消任务 #{id}");
        cx.emit(DownloadsChanged);
        cx.notify();
    }

    /// 从列表里去掉一条记录（**不动磁盘**）。还在跑的不许去：那会丢掉唯一的取消入口。
    pub fn forget(&mut self, id: u64, cx: &mut Context<Self>) {
        let before = self.tasks.len();
        self.tasks
            .retain(|task| !(task.id == id && task.state != TaskState::Running));
        if self.tasks.len() != before {
            cx.emit(DownloadsChanged);
            cx.notify();
        }
    }

    /// 去掉所有"已完成"与"已取消"的记录（磁盘上的成品一个都不动）。
    ///
    /// 失败与进行中的留着：前者那一行是用户唯一能看到原因的地方，一键清掉等于把问题抹掉；
    /// 后者还没结束，去掉就再没地方点取消。
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
            // 换备用域名时会回一个**较小**的数（那一次失败写的字节不算），
            // 所以界面上的百分比偶尔会退一点——那是真的退，不是画错。
            DownloadEvent::Progress { done } if task.state == TaskState::Running => {
                task.done = done;
            }
            DownloadEvent::Progress { .. } => {}
            DownloadEvent::Finished { files } => {
                // 最后一个不足 1 MiB 的分块不会单独回报，这里按分母补满。
                if let Some(total) = task.total {
                    task.done = total;
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
        cx.emit(DownloadsChanged);
        cx.notify();
    }
}

impl EventEmitter<DownloadsChanged> for DownloadQueue {}
