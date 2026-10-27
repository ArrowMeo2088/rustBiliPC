//! 下载：把选定的一条 DASH 流写成磁盘上的**两个文件**（视频 + 音频）。
//!
//! ## 这是唯一允许写到运行目录之外的功能
//!
//! 2026-10-26 用户的原话："下载功能要做一下，**仅这个能下载到外部文件里**。"
//! 其余落盘仍然只有设置目录（`%APPDATA%/ArrowMeo/rustBiliPC`）里的
//! `debug.log` / `auth.json` / `config.json`（口径见 QODER.md"运行期数据"）。
//! 所以这一层的目录**只来自系统弹框选出来的那个**，不猜、不用默认目录、也不在运行目录里开一个 downloads/。
//!
//! ## 为什么是两个文件
//!
//! B 站普通稿件是 DASH 音视频分开两条流（`src/player/mod.rs` 文件头同一条事实），
//! 在线播时 mpv 用 `--audio-file=` 把两条凑成一次播放。落盘没有"凑"这一步：
//! 下出来就是 `xxx.m4v` + `xxx.m4a`，用户定的形态就是这个（"双文件 + 高画质"）。
//! 合成一个 mp4 需要混流器，我们零浏览器之外**也不为此引 ffmpeg 依赖**，不做。
//!
//! ## 三条硬规矩（都在下面的代码里）
//!
//! 1. **先写 `.part`，成了再改名**。改名的意义：中断留下的半成品不会长得像成品，
//!    而 Windows 上"边写边被 rename"这种竞争也不存在——只有写它的那个任务会 rename 它。
//! 2. **取消是标志位，不是 `abort()`**。`JoinHandle::abort()` 会在下一个 await 点把任务
//!    直接丢掉，那个任务手里的 `File` 与 `BufWriter` 还没 flush 就被析构，而界面这边已经
//!    在 `remove_file` 了——Windows 上"文件还开着就删"会失败或留下孤儿。
//!    现在的方式：界面置标志 → 任务在下一个分块处看见 → 自己收尾（flush、删 `.part`）→ 发 `Cancelled`。
//! 3. **主地址失败就换 `backupUrl`**（`baseUrl` 之外 B 站给一组备用域名）。
//!    候选地址的排法与 CDN 要的那条 Cookie 都在 [`probe`](super::probe) 里，
//!    在线播那一路也共用同一组（[`ordered_urls`] / [`cdn_headers`]）——
//!    两条路的差别只在"谁读字节"：这里能当场换下一个域名，
//!    播那一路只能在交给 mpv 之前先探好（mpv 内部的地址我们替换不了）。
//!
//! ## 不做的事
//!
//! - **断点续传**：没有 Range 请求与 `.part` 长度对齐的逻辑，重来就是从头下。
//! - **并发**：一次任务内部视频写完再写音频；多个任务之间靠 tokio runtime 的多线程自然并行，
//!   但没有队列上限——点了五个就是五个在跑。
//! - **durl（分段直链）**：`durl[]` 是多段文件，要按 `order` 拼接才是完整一片，
//!   这一版直接拒绝下载并给一句实话（在线播不受影响）。

use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::AsyncReadExt as _;
use gpui_kit::http_client::{AsyncBody, Builder, HttpClient, HttpRequestExt as _, RedirectPolicy};

use super::probe::{cdn_headers, ordered_urls};
use super::spawn_on_api_runtime;
use super::stream::StreamChoice;
use crate::player::{BILIBILI_REFERER, MPV_USER_AGENT};

/// 一次读多少字节。256 KiB：太小会让 `tokio` 任务被大量 poll 占住，太大会让"取消"响应变慢。
const CHUNK_SIZE: usize = 256 * 1024;
/// 累计写够这么多字节才回报一次进度（约 1 MiB）。
///
/// 不节流的话一个 1 GB 的文件会往界面通道里塞四千多条消息，而界面一次也只画得出一个百分比。
const REPORT_STEP: u64 = 1024 * 1024;
/// 文件名主体的最长字符数（按**字符**数，不按字节：标题里全是中文与 emoji，
/// 按字节截会把一个多字节字符切成半个，Windows 上就变成乱码名或非法名）。
const MAX_STEM_CHARS: usize = 60;

/// Windows 不允许出现在文件名里的字符（`:` `/` `\` `*` `?` `"` `<` `>` `|`）。
/// 命中不整条拒绝，换成空格——标题里这些符号很常见（"预告：第 3 集"）。
const ILLEGAL_FILE_CHARS: [char; 9] = ['<', '>', '"', '/', '\\', '|', '?', '*', ':'];

/// 一次下载要写出去的东西。**全部由调用方在点按钮那一刻算好**，
/// 这样任务跑起来之后不再回头看界面状态（用户可能已经翻页、切 P 了）。
#[derive(Clone, Debug)]
pub struct DownloadRequest {
    /// 列表上给人看的一行字（标题 + 画质）。
    pub label: String,
    /// 清洗过、且确认不重名的文件名主体（不含点与后缀）。
    pub file_stem: String,
    pub video_url: String,
    pub video_backup: Vec<String>,
    pub video_ext: &'static str,
    pub video_size: Option<u64>,
    pub audio_url: Option<String>,
    pub audio_backup: Vec<String>,
    pub audio_ext: &'static str,
    pub audio_size: Option<u64>,
}

impl DownloadRequest {
    /// 两条轨的后缀。重名判定要看**两个**：只看视频的话会出现"视频改了新名字、音频盖掉旧音频"。
    pub fn extensions(&self) -> Vec<&'static str> {
        let mut extensions = vec![self.video_ext];
        if self.audio_url.is_some() {
            extensions.push(self.audio_ext);
        }
        extensions
    }

    /// 换成实际要用的文件名主体（目录里重名时由队列加 `(2)`）。
    pub fn with_stem(mut self, stem: String) -> Self {
        self.file_stem = stem;
        self
    }

    /// 视频与音频各一个落盘路径。
    pub fn targets(&self, dir: &Path) -> Vec<PathBuf> {
        let mut paths = vec![dir.join(format!("{}.{}", self.file_stem, self.video_ext))];
        if self.audio_url.is_some() {
            paths.push(dir.join(format!("{}.{}", self.file_stem, self.audio_ext)));
        }
        paths
    }

    /// 界面进度条的分母。**只有全部要下的轨都知道大小才给**：
    /// 给一半的数会让"91%"这种读数说谎。
    pub fn total_bytes(&self) -> Option<u64> {
        let video = self.video_size?;
        match self.audio_url.as_ref() {
            Some(_) => Some(video + self.audio_size?),
            None => Some(video),
        }
    }
}

/// 从一次起播计划拼出下载请求。`Err` 里是给界面看的一句话（不发无意义的任务）。
pub fn request_for(
    plan: &StreamChoice,
    label: String,
    file_stem: String,
) -> Result<DownloadRequest, String> {
    if !plan.is_dash {
        return Err("这条稿件给的是分段直链（durl），要按 order 拼接才是完整一片，这一版只支持在线播".into());
    }
    if plan.video_url.is_empty() {
        return Err("播控地址里没有可用的视频轨".into());
    }
    Ok(DownloadRequest {
        label,
        file_stem,
        video_url: plan.video_url.clone(),
        video_backup: plan.backup_urls.clone(),
        video_ext: plan.video_ext,
        video_size: plan.video_size,
        audio_url: plan.audio_url.clone(),
        audio_backup: plan.audio_backup_urls.clone(),
        audio_ext: plan.audio_ext,
        audio_size: plan.audio_size,
    })
}

/// 下载过程中的回报。`Progress` 只给**累计**已写字节（两条轨加在一起）。
#[derive(Clone, Debug)]
pub enum DownloadEvent {
    Progress { done: u64 },
    Finished { files: Vec<PathBuf> },
    Failed { message: String },
    Cancelled,
}

/// 一路轨（视频或音频）的下载目标。
struct Track {
    /// 界面用词："视频" / "音频"。
    label: &'static str,
    /// 按顺序试的地址（主地址 + 备用域名）。
    urls: Vec<String>,
    /// 成品路径。
    path: PathBuf,
    /// 中途写的临时路径（成品名 + `.part`）。
    part_path: PathBuf,
}

impl Track {
    /// 由成品路径推出临时路径：`片名.m4v` → `片名.m4v.part`。
    ///
    /// 用 `set_extension` 拼"原后缀 + .part"而不是 `PathBuf::push`（push 会当成子路径插一个
    /// 分隔符，得到 `片名.m4v/.part`），也不直接写死 `set_extension("part")`
    /// （那样 `a.m4v` 与 `a.m4a` 的临时文件会同名，两条轨互相踩）。
    fn new(label: &'static str, urls: Vec<String>, path: PathBuf) -> Self {
        let mut part_path = path.clone();
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("part");
        part_path.set_extension(format!("{extension}.part"));
        Self {
            label,
            urls,
            path,
            part_path,
        }
    }
}

/// 任务失败或被取消的原因。
enum Stop {
    Cancelled,
    Failed(String),
}

/// 把一次下载投到 API runtime 上跑，返回 `JoinHandle` 与进度接收端。
///
/// `cancel` 由界面持有：置 `true` 之后任务在下一个分块处收尾。
/// `JoinHandle` 这一版**不用于取消**（原因见文件头第 2 条），界面收下它只是为了
/// 在日志里知道任务还在不在；tokio 的 `JoinHandle` 被 drop 不会取消任务，
/// 所以调用方丢弃它是安全的。
pub fn spawn_download(
    request: DownloadRequest,
    dir: PathBuf,
    client: Arc<dyn HttpClient>,
    cancel: Arc<AtomicBool>,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::mpsc::UnboundedReceiver<DownloadEvent>,
) {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    log::info!(
        "提交下载到 API runtime：{} → {:?}（大小={}）",
        request.file_stem,
        dir,
        request
            .total_bytes()
            .map(|bytes| bytes.to_string())
            .unwrap_or_else(|| "未知".to_string())
    );
    let join = spawn_on_api_runtime(async move {
        // 注册进"还在跑"的名册：退场时（`cancel_all_active`）才知道该给哪些任务置取消标志。
        // 放在任务里而不是 `spawn_download` 里，是为了**名册里有的那一支一定已经在跑**，
        // 排队中的任务（P14 的队列上限）不该被"退出收尾"叫到。
        register(&cancel);
        run(request, dir, client, cancel.clone(), sender).await;
        unregister(&cancel);
    });
    (join, receiver)
}

/// 在跑任务的取消标志名册。
///
/// 为什么是一张全局表而不是"队列自己记得"：置标志这件事发生在**进程要退的时候**，
/// 那一刻队列实体正在被 drop（`DownloadQueue::drop` 就是主要调用方），
/// 从 `Drop` 里再回头借 `&self.tasks` 是行得通的，但另一条路（`main.rs` 主动 `cx.quit()`）
/// 手上根本没有队列实体 —— 一张表能让两个入口共用，也不会漏下"队列已被清掉但任务还在写"的那种。
static ACTIVE: Mutex<Vec<Arc<AtomicBool>>> = Mutex::new(Vec::new());

fn register(flag: &Arc<AtomicBool>) {
    if let Ok(mut guard) = ACTIVE.lock() {
        guard.push(Arc::clone(flag));
    }
}

fn unregister(flag: &Arc<AtomicBool>) {
    if let Ok(mut guard) = ACTIVE.lock() {
        guard.retain(|one| !Arc::ptr_eq(one, flag));
    }
}

/// 叫所有在跑的任务收尾（置取消标志），返回叫了几条。
///
/// **只置标志、不删文件**：删 `.part` 由任务自己做（它先 flush、再关句柄、最后删）。
/// 主线程直接删会有两个问题：Windows 上"文件还开着就删"会失败，
/// 以及"任务刚好改名成成品"时把成品当临时文件删掉（先后顺序谁也不知道）。
///
/// 诚实的限制：置完标志到进程真的退出之间可能只剩几毫秒，任务不一定来得及跑到下一个分块，
/// 那时磁盘上仍会留一个 `.part`。要治那条得让退出路径等一等任务，
/// 而"退出时卡两秒等下载收尾"是不是他要的，得先问（见风险表新增那条）。
pub fn cancel_all_active() -> usize {
    let Ok(mut guard) = ACTIVE.lock() else {
        return 0;
    };
    // 顺手清掉已经结束的（正常路径上任务自己摘了，这里是防泄漏的兜底）。
    guard.retain(|flag| !flag.load(Ordering::Relaxed));
    let count = guard.len();
    for flag in guard.iter() {
        flag.store(true, Ordering::Relaxed);
    }
    if count > 0 {
        log::info!("下载：退出收尾请求了 {count} 条在跑的任务");
    }
    count
}

/// 一条任务的主体：先把视频写完再写音频，中途只认取消标志。
async fn run(
    request: DownloadRequest,
    dir: PathBuf,
    client: Arc<dyn HttpClient>,
    cancel: Arc<AtomicBool>,
    sender: tokio::sync::mpsc::UnboundedSender<DownloadEvent>,
) {
    let headers = cdn_headers();
    let mut tracks = vec![Track::new(
        "视频",
        ordered_urls(&request.video_url, &request.video_backup),
        dir.join(format!("{}.{}", request.file_stem, request.video_ext)),
    )];
    if let Some(audio_url) = request.audio_url.clone() {
        tracks.push(Track::new(
            "音频",
            ordered_urls(&audio_url, &request.audio_backup),
            dir.join(format!("{}.{}", request.file_stem, request.audio_ext)),
        ));
    }

    let mut written: Vec<PathBuf> = Vec::new();
    let mut done: u64 = 0;

    for track in tracks {
        if cancel.load(Ordering::Relaxed) {
            clean_up(&written, &track.part_path);
            let _ = sender.send(DownloadEvent::Cancelled);
            return;
        }
        match pull_track(&client, &track, &headers, &cancel, &sender, &mut done).await {
            Ok(()) => written.push(track.path.clone()),
            Err(stop) => {
                // 一对文件要么都留下、要么都不留：这条轨失败时把本任务已经写好的成品也删掉
                // （理由见 [`clean_up`]）。正在写的临时文件一定在 `track.part_path`。
                clean_up(&written, &track.part_path);
                match stop {
                    Stop::Cancelled => {
                        log::info!("下载已取消：{}", track.path.display());
                        let _ = sender.send(DownloadEvent::Cancelled);
                    }
                    Stop::Failed(message) => {
                        log::error!("下载失败：{}（{:?}）", message, track.path);
                        let _ = sender.send(DownloadEvent::Failed { message });
                    }
                }
                return;
            }
        }
    }

    log::info!("下载完成：{:?}", written);
    let _ = sender.send(DownloadEvent::Finished { files: written });
}

/// 写一条轨：按顺序试每个地址，成功就改名成成品。
///
/// `done` 是"已经算进进度的字节"，**包含换地址时丢掉的那一段**：
/// 分母是整个文件的大小，如果丢掉的不计进去，一个 900 MB 的地址在下到 40% 时失败，
/// 进度就会从 36% 一路掉回 0% 再往上爬 —— 那看起来像"下载逻辑坏了"，其实是口径没定对。
/// 丢掉的那段确实没留在磁盘上，所以日志里另外写了一句"其中 N 字节是换地址前下的"。
async fn pull_track(
    client: &Arc<dyn HttpClient>,
    track: &Track,
    headers: &[(String, String)],
    cancel: &AtomicBool,
    sender: &tokio::sync::mpsc::UnboundedSender<DownloadEvent>,
    done: &mut u64,
) -> Result<(), Stop> {
    let mut last_error = String::from("地址列表是空的");

    for url in &track.urls {
        let base = *done;
        match attempt(client, track, url, headers, cancel, sender, base).await {
            Ok(bytes) => {
                *done = base + bytes;
                return Ok(());
            }
            // 取消是全局意图，不换域名、不重试；那一段字节已经不计了（任务本身就废了）。
            Err(AttemptStop::Cancelled { bytes }) => {
                *done = base + bytes;
                let _ = sender.send(DownloadEvent::Progress { done: *done });
                return Err(Stop::Cancelled);
            }
            Err(AttemptStop::Failed { message, bytes }) => {
                // 只写**主机名**：完整直链里带着 `deadline`/`gen` 这些签名参数，
                // 谁拿到日志就能在有效期内替他把这条流下走。日志要能定位问题不假，
                // 但没有理由把可用凭证写进一个会被发回来的文件（`probe::host_of` 同一条口径）。
                *done = base + bytes;
                log::warn!(
                    "下载：{}轨的这个地址失败，换下一个（{}）：{message}（丢弃 {bytes} 字节，进度里仍算已下）",
                    track.label,
                    super::probe::host_of(url),
                );
                let _ = sender.send(DownloadEvent::Progress { done: *done });
                last_error = message;
            }
        }
    }

    Err(Stop::Failed(format!(
        "{}轨的每个地址都没下成：{last_error}",
        track.label
    )))
}

/// 一次"用一个地址试"的失败形状：除了原因，还要带回**已经下到手的字节数**。
enum AttemptStop {
    Cancelled { bytes: u64 },
    Failed { message: String, bytes: u64 },
}

/// 用**一个**地址把一条轨写到 `.part` 再改名，返回这次写的字节数。
async fn attempt(
    client: &Arc<dyn HttpClient>,
    track: &Track,
    url: &str,
    headers: &[(String, String)],
    cancel: &AtomicBool,
    sender: &tokio::sync::mpsc::UnboundedSender<DownloadEvent>,
    base: u64,
) -> Result<u64, AttemptStop> {
    let request = Builder::new()
        .uri(url)
        .follow_redirects(RedirectPolicy::FollowAll)
        // Referer 与 UA 是 B 站 CDN 的检查项（在线播时同样要，见 src/player/mod.rs）；
        // 1080P 还要登录 cookie，所以缺了 Cookie 头会拿到 403 而不是流。
        .header("Referer", BILIBILI_REFERER)
        .header("User-Agent", MPV_USER_AGENT);
    let request = headers
        .iter()
        .fold(request, |builder, (name, value)| {
            builder.header(name.as_str(), value.as_str())
        })
        .body(AsyncBody::empty())
        .map_err(|error| AttemptStop::Failed { message: format!("请求拼不出来: {error}"), bytes: 0 })?;

    let mut response = client
        .send(request)
        .await
        .map_err(|error| AttemptStop::Failed { message: format!("请求失败: {error}"), bytes: 0 })?;
    // `Response::status()` 返回的是 `StatusCode` 本身（它 `Copy`），不是引用：
    // 先拿走状态码再看响应体，后面 `body_mut()` 要独占这个响应。
    let status = response.status();
    if !status.is_success() {
        return Err(AttemptStop::Failed { message: format!("HTTP {status}"), bytes: 0 });
    }

    let file = std::fs::File::create(&track.part_path).map_err(|error| AttemptStop::Failed {
        message: format!("建临时文件失败: {error}"),
        bytes: 0,
    })?;
    let mut writer = BufWriter::with_capacity(1024 * 1024, file);
    let mut buffer = vec![0u8; CHUNK_SIZE];
    // `body` 本身是 `&mut AsyncBody`，方法调用时会自己再借用，所以这里不需要 `mut`。
    let body = response.body_mut();
    let mut written: u64 = 0;
    let mut since_report: u64 = 0;

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = writer.flush();
            // **先把句柄关掉再交回给上层删**：`BufWriter` 里面装着那个 `File`，
            // 没 drop 就去 `remove_file` 在 Windows 上会失败（文件正开着），
            // 症状是"取消了，但目录里还是留了个 .part"。
            drop(writer);
            return Err(AttemptStop::Cancelled { bytes: written });
        }
        let read = body
            .read(&mut buffer)
            .await
            .map_err(|error| AttemptStop::Failed {
                message: format!("读流失败: {error}"),
                bytes: written,
            })?;
        if read == 0 {
            break;
        }
        writer
            .write_all(&buffer[..read])
            .map_err(|error| AttemptStop::Failed {
                message: format!("写盘失败: {error}"),
                bytes: written,
            })?;
        written += read as u64;
        since_report += read as u64;
        if since_report >= REPORT_STEP {
            since_report = 0;
            let _ = sender.send(DownloadEvent::Progress {
                done: base + written,
            });
        }
    }

    writer
        .flush()
        .map_err(|error| AttemptStop::Failed { message: format!("落盘失败: {error}"), bytes: written })?;
    // 只有整条轨读完了才改名：这一步之后文件在磁盘上就是"下好的"。
    std::fs::rename(&track.part_path, &track.path).map_err(|error| AttemptStop::Failed {
        message: format!("改名成成品失败: {error}"),
        bytes: written,
    })?;
    Ok(written)
}

/// 删掉**还没下完**的那条轨的临时文件，以及本任务已经写好的成品。
///
/// 为什么连成品也删：一次下载是"视频 + 音频"一对，只有两个都在才算成一个可用的稿件；
/// 留下一个孤零零的 `xxx.m4v`（没有声音）比全删掉更容易被当成可用的文件用。
/// 已改名的成品在 `written` 里，正在写的临时文件在 `part_path` 里。
fn clean_up(written: &[PathBuf], part_path: &Path) {
    let mut paths: Vec<&Path> = written.iter().map(PathBuf::as_path).collect();
    paths.push(part_path);
    for path in paths {
        match std::fs::remove_file(path) {
            Ok(()) => log::debug!("下载收尾：删掉 {:?}", path),
            // 本来就不存在（第一个分块都没写成就失败）不是错误。
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => log::warn!("下载收尾：删不掉 {:?}（{error}）", path),
        }
    }
}

/// 文件名主体：标题，多 P 稿件再带上 `P<页码> <分P标题>`；清洗完是空串就退回稿件号。
///
/// 为什么要带页码：只下当前那一 P（用户定稿），而下出来的两个文件都只按标题命名时，
/// 同一稿件的 20 个 P 会全叫同一个名字，靠 `(2)`、`(3)` 后缀分不清谁是谁。
/// 兜底用 `bvid` 而不是报错：标题全是 `***` 这种稿件真存在，用户要的是"下下来"，
/// 不是一个弹窗。
pub fn file_stem(title: &str, bvid: &str, part: Option<(u32, &str)>) -> String {
    let raw = match part {
        Some((index, name)) => format!("{title} P{index} {name}"),
        None => title.to_string(),
    };
    let stem = sanitize_file_stem(&raw);
    if stem.is_empty() {
        return bvid.to_string();
    }
    stem
}

/// 把标题清洗成能当文件名的主体：非法字符换空格、空白折叠、去掉首尾空格与点、按字符截断。
///
/// 结果可能是空串（标题全是 `***` 这种），调用方要用稿件号兜底。
pub fn sanitize_file_stem(raw: &str) -> String {
    let mut out = String::new();
    let mut pending_space = false;

    for ch in raw.chars() {
        let ch = if ILLEGAL_FILE_CHARS.contains(&ch) || (ch as u32) < 0x20 {
            ' '
        } else {
            ch
        };
        if ch.is_whitespace() {
            if !out.is_empty() {
                pending_space = true;
            }
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(ch);
        if out.chars().count() >= MAX_STEM_CHARS {
            break;
        }
    }

    // 截断可能刚好停在"要写空格"之后，收尾的空格与点都要去掉（Windows 不允许结尾是空格或点）。
    while out.ends_with(' ') || out.ends_with('.') {
        out.pop();
    }
    out
}

/// 重名时的候选主体：第 0 次原名，之后是 `原名 (2)`、`原名 (3)`……
///
/// 用半角括号加空格是 Windows 自己"copy of"的排法，也是资源管理器里最容易认的一种。
pub fn numbered_stem(stem: &str, index: usize) -> String {
    if index == 0 {
        return stem.to_string();
    }
    format!("{stem} ({})", index + 1)
}

/// 在 `dir` 里找一个不占用现有文件的文件名主体。
///
/// 判定看**两条轨的后缀**：只有两个都不存在才算空位，否则会出现"视频是新名字、音频盖掉旧音频"。
/// 一直试下去（现实中不会有几百个同名稿件），不返回失败。
pub fn available_stem(dir: &Path, stem: &str, extensions: &[&str]) -> String {
    let mut index = 0usize;
    loop {
        let candidate = numbered_stem(stem, index);
        let taken = extensions
            .iter()
            .any(|ext| dir.join(format!("{candidate}.{ext}")).exists());
        if !taken {
            return candidate;
        }
        index += 1;
    }
}

/// 字节数 → 给人看的一句话（`1.4 GB` / `86.2 MB`）。
///
/// 单位按 1024 进位但写 KB/MB/GB（Windows 资源管理器的口径），不写 KiB——
/// 这行字是给人估"下完要多大"的，不是给运维算容量的。
pub fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.;
    const MIB: f64 = 1024. * 1024.;
    const GIB: f64 = 1024. * 1024. * 1024.;
    let value = bytes as f64;
    if value < KIB {
        return format!("{bytes} B");
    }
    if value < MIB {
        return format!("{:.1} KB", value / KIB);
    }
    if value < GIB {
        return format!("{:.1} MB", value / MIB);
    }
    format!("{:.2} GB", value / GIB)
}

/// 进度百分比（`Progress` 要 0~100 的浮点）。分母缺失或为 0 时给 `None`，界面改用不定进度。
///
/// 已写超过分母（B 站的 `size` 偶尔与实际不符）时**夹到 99.9 而不是 100**：
/// 100% 在界面上的意思是"下完了"，那个状态只由 `Finished` 给。
pub fn percent(done: u64, total: Option<u64>) -> Option<f32> {
    let total = total.filter(|value| *value > 0)?;
    let raw = done as f32 * 100. / total as f32;
    Some(raw.clamp(0., 99.9))
}
