//! 文件日志：写**运行同目录**的 `debug.log`（与凭据同处），每次启动清空重写，级别 debug。
//!
//! 为什么不直接用 env_logger：双击启动的 GUI 程序没有控制台，stderr 一律看不见，
//! 而排查"请求发出去了、界面却不动"必须把每一跳（tokio 任务内部完成、结果回到 gpui 前台、
//! 实体是否还活着、状态写入是否成功）都落盘才可能定位。
//!
//! 位置按定稿放在 exe 同目录：跟程序一起被看见、被发回来，不用教用户找 `%LOCALAPPDATA%`。
//! 代价是它落在构建输出目录 `publish/` 里，所以**取产物时只覆盖 exe，绝不整目录清空**
//! （`gh run download` 直接往 `publish/` 解包会因文件已存在而失败，正确做法是解到临时目录再搬）。
//!
//! 实现上零依赖：自己实现 `log::Log`，每行写完立刻 flush（进程崩溃也不丢最后几行），
//! 同时往 stderr 复制一份，方便从命令行启动时直接看。

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// 日志文件；`OnceLock` 保证 `init()` 重复调用不会开出第二个句柄。
static LOG_FILE: OnceLock<Mutex<File>> = OnceLock::new();

/// 实际使用的日志路径，供界面提示用户"出问题时把这个文件发回来"。
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// 日志文件名（放在 [`log_dir`] 里）。
pub const FILE_NAME: &str = "debug.log";

/// 日志目录：可执行文件所在目录（与 `data/cookies.json` 同一层，用户一眼就能看到）。
fn log_dir() -> PathBuf {
    crate::app::exe_dir()
}

/// 日志文件路径（`init()` 之前也能安全调用）。
pub fn log_path() -> PathBuf {
    LOG_PATH
        .get()
        .cloned()
        .unwrap_or_else(|| log_dir().join(FILE_NAME))
}

struct FileLogger;

impl log::Log for FileLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        // 级别由 `set_max_level` 统一管，这里一律放行。
        true
    }

    fn log(&self, record: &log::Record) {
        // 不需要在这里再判级别：`log` 门面已按 `set_max_level` 过滤后才会调到本函数
        // （`log::Level` 也没有 `is_enabled` 方法，CI 实测）。
        let line = format!(
            "{} {:>5} {}: {}\n",
            timestamp(),
            record.level(),
            record.target(),
            record.args()
        );

        if let Some(file) = LOG_FILE.get() {
            if let Ok(mut guard) = file.lock() {
                // 写盘失败只能放弃这一行：日志本身不能再产生日志。
                let _ = guard.write_all(line.as_bytes());
                let _ = guard.flush();
            }
        }

        // 命令行启动时同样可见。
        let _ = std::io::stderr().write_all(line.as_bytes());
    }

    fn flush(&self) {
        if let Some(file) = LOG_FILE.get() {
            if let Ok(mut guard) = file.lock() {
                let _ = guard.flush();
            }
        }
    }
}

/// `HH:MM:SS.mmm` 的 UTC 时间戳，够对齐一次登录流程里的所有事件。
fn timestamp() -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let total_ms = elapsed.as_millis() % 1000;
    let total_secs = elapsed.as_secs() % 86_400;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        total_secs / 3600,
        (total_secs / 60) % 60,
        total_secs % 60,
        total_ms
    )
}

/// 初始化日志：创建（并清空）**exe 同目录**的 `debug.log`，装上 logger，级别 `Debug`。
///
/// 返回日志文件路径，调用方可以把它显示到界面上，用户就知道该把哪个文件发回来。
pub fn init() -> PathBuf {
    let dir = log_dir();
    // 目录可能还不存在（首次运行）；建不出来就退化成"只有 stderr"。
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(FILE_NAME);

    match File::create(&path) {
        Ok(file) => {
            let _ = LOG_FILE.set(Mutex::new(file));
        }
        Err(error) => {
            // 目录不可写（比如装在 Program Files）时仍要能跑，只是没日志。
            let _ = std::io::stderr().write_all(format!("无法创建日志文件 {path:?}: {error}\n").as_bytes());
        }
    }

    let _ = LOG_PATH.set(path.clone());

    // 已经装过 logger 时 `set_logger` 会返回 Err，这里忽略：重复 init 不该 panic。
    if log::set_boxed_logger(Box::new(FileLogger)).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }

    path
}
