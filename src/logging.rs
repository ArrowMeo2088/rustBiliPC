//! 文件日志：写 `%APPDATA%/ArrowMeo/rustBiliPC/debug.log`，每次启动清空重写，级别 debug。
//!
//! ## 它现在是可关的（P13）
//!
//! 设置页新增"调试模式"，默认**关**。关掉时 [`init`] 不建文件、不装 logger，
//! 于是全项目那些 `log::info!/debug!/warn!` 都是空转（`log` 门面在没装 logger 时直接丢弃记录）。
//! 两条必须记住的连带事实：
//! 1. **exe 现在是 GUI 子系统**（`main.rs` 上的 `#![windows_subsystem = "windows"]`），
//!    双击不再弹那个黑色控制台窗口——它与调试模式无关，两种模式下都没有控制台。
//!    想要"调试时弹控制台"得另外做 `AttachConsole`，那没做。
//! 2. 配置**坏了**时（`config.json` 读不出来）这一轮会**强制开日志**：
//!    调试模式就写在那个坏文件里，不开日志他就既看不到错误也没法打开它。
//!    判据在 [`crate::store::config::LoadOutcome::RecoveredFromBroken`]。
//!
//! ## 为什么还要自己实现，而不用 env_logger
//!
//! 排查"请求发出去了、界面却不动"必须把每一跳（tokio 任务内部完成、结果回到 gpui 前台、
//! 实体是否还活着、状态写入是否成功）都落盘才可能定位。
//!
//! 位置在 2026-10-26 从"exe 同目录"改到 `%APPDATA%` 下（用户定稿，见 `store/auth.rs` 文件头）。
//! 这顺带把一条要靠人记的红线消掉了：以前取 CI 产物时"只许覆盖 exe、绝不整目录清空"，
//! 因为 `debug.log`/`auth.json` 就躺在 `publish/` 里；现在它们不在产物目录了。
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

/// 日志目录：`%APPDATA%/ArrowMeo/rustBiliPC`（与 `auth.json`、`config.json` 同一处）。
fn log_dir() -> PathBuf {
    crate::app::settings_dir()
}

/// 本次启动**实际**写出来的日志文件路径。
///
/// `None` = 调试模式关闭（不建文件、也没装 logger），或 `init()` 还没跑。
/// 界面用它如实回答"到底有没有日志可发回来"——以前这里会造一个"看起来有"的路径，
/// 关闭调试模式之后就变成假信息了。
pub fn log_path() -> Option<PathBuf> {
    LOG_PATH.get().cloned()
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

/// 初始化日志：`enabled` 为真才创建（并清空）`<设置目录>/debug.log`、装上 logger、级别 `Debug`。
///
/// `enabled=false`（设置页"调试模式"关着，也是默认）时**什么都不做**：不建目录、不开文件、
/// 不装 logger，全项目那些 `log!` 调用都是空转。返回 `None`，界面那行就显示"调试模式关闭"。
///
/// 返回启用时的日志路径，调用方可以把它显示出来，用户就知道该把哪个文件发回来。
pub fn init(enabled: bool) -> Option<PathBuf> {
    if !enabled {
        // 显式设成 Off：万一有依赖库自己装了 logger，关闭时也不该漏出日志。
        log::set_max_level(log::LevelFilter::Off);
        return None;
    }

    let dir = log_dir();
    // 目录可能还不存在（首次运行）；建不出来就退化成"只有 stderr"。
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(FILE_NAME);

    let mut opened = None;
    match File::create(&path) {
        Ok(file) => {
            let _ = LOG_FILE.set(Mutex::new(file));
            opened = Some(path.clone());
        }
        Err(error) => {
            // 目录不可写（比如权限被改过）时仍要能跑，只是没日志。
            let _ = std::io::stderr().write_all(format!("无法创建日志文件 {path:?}: {error}\n").as_bytes());
        }
    }

    // 只在**真的开出了文件**时记下路径：`log_path()` 是给界面说的实话，
    // 建失败还说"日志在这个文件里"就会让人去发一个不存在的文件。
    if opened.is_some() {
        let _ = LOG_PATH.set(path);
    }

    // 已经装过 logger 时 `set_logger` 会返回 Err，这里忽略：重复 init 不该 panic。
    if log::set_boxed_logger(Box::new(FileLogger)).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }

    opened
}
