//! 进程级基础设施：tokio runtime 桥接、exe 同目录约定。
//!
//! ## 为什么需要这一层
//!
//! gpui 的执行器不是 tokio，而 bpi-rs 底层是 reqwest（依赖 tokio 的 reactor 与
//! timer）。把 bpi-rs 的 future 直接丢进 `cx.spawn` 会在 poll 时 panic
//! （"there is no reactor running"）。所以本模块持有一个进程唯一的 runtime，
//! [`crate::api`] 把请求 spawn 到它上面，再通过 channel 把结果送回 gpui 主线程。

use std::path::PathBuf;
use std::sync::OnceLock;

/// 进程唯一的 tokio runtime。用 `OnceLock` 而不是全局 `#[tokio::main]`，
/// 因为 gpui 的 `Application::run` 会阻塞主线程，两者不能嵌套。
static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// 取得（并在首次调用时惰性创建）API 用的 tokio runtime。
///
/// `enable_all` 必须开：reqwest 的 DNS 解析走 tokio 的 net 驱动，
/// 超时与 QR 轮询节奏走 tokio 的 time 驱动。
pub fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("rustbilipc-api")
            .build()
            .expect("初始化 tokio runtime 失败")
    })
}

/// 可执行文件所在目录。凭据与配置要"保存到运行同目录"，一切路径以它为根。
///
/// 拿不到 `current_exe` 时退回当前工作目录，不 panic——这只在异常启动方式下发生。
pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 运行目录下的数据目录 `./data/`（凭据、缓存、将来的下载记录都放这里）。
pub fn data_dir() -> PathBuf {
    exe_dir().join("data")
}
