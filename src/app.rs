//! 进程级基础设施：tokio runtime 桥接、exe 同目录约定、进程级设置快照。
//!
//! ## 为什么需要这一层
//!
//! gpui 的执行器不是 tokio，而 bpi-rs 底层是 reqwest（依赖 tokio 的 reactor 与
//! timer）。把 bpi-rs 的 future 直接丢进 `cx.spawn` 会在 poll 时 panic
//! （"there is no reactor running"）。所以本模块持有一个进程唯一的 runtime，
//! [`crate::api`] 把请求 spawn 到它上面，gpui 侧再 await 返回的 `JoinHandle`
//! （await 它只需要 waker，不需要 reactor，唤醒会经调度器送回主线程）。

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

/// 进程级设置快照。启动时 [`init_config`] 一次（来源是运行目录的 `config.json`），
/// 之后各处只读 [`config`]。
///
/// 为什么做成进程级而不是把 `Arc<Config>` 一路传进每个页面：真正读它的只有起播那一条路
/// （请求 `qn` 与选流上限），传下去要动六个页面的构造函数；但**纯函数仍然收显式参数**
/// （见 [`crate::api::stream_select`]），这样单测不用碰全局状态。
static CONFIG: OnceLock<crate::store::config::Config> = OnceLock::new();

/// 装好进程级设置（重复调用无效果，第一次的值算数）。
pub fn init_config(config: crate::store::config::Config) {
    let _ = CONFIG.set(config);
}

/// 取当前设置。没装过时返回默认值——单测与"设置读不到"两种情况都走这条路。
pub fn config() -> crate::store::config::Config {
    CONFIG.get().copied().unwrap_or_default()
}

/// 显式安装 rustls 的进程级默认 crypto provider。
///
/// 必须做，否则**任何 HTTPS 请求都会在 TLS 握手处 panic**：这张依赖图里
/// `rustls 0.23` 同时启用了 `aws-lc-rs`（reqwest 的默认 provider）与 `ring`
/// （首轮 CI 的 Cargo.lock 里两者都在），rustls 无法自行二选一，会抛
/// "Could not automatically determine a process-level default crypto provider"。
/// panic 发生在 tokio 任务里，表现为 `JoinHandle` 返回 Err，界面就停在加载态。
///
/// 选 `aws-lc-rs`：它已经在依赖图里编译好了（CI 为此装了 NASM），换 `ring` 不会更省事。
/// `install_default` 只允许装一次，重复调用返回 Err，这里降级成一条 warn。
pub fn install_tls_provider() {
    match rustls::crypto::aws_lc_rs::default_provider().install_default() {
        Ok(()) => log::debug!("已安装 rustls 默认 provider: aws-lc-rs"),
        // 注意：这个版本的 `install_default()` 的 Err 里装的是"已经被装上那个 provider"，
        // 它不实现 Display，所以只能记事件、不能格式化内容（CI 实测踩过）。
        Err(_) => log::warn!("rustls 默认 provider 已被安装，沿用现有配置"),
    }
}

/// 把 `JoinHandle` 的错误翻译成能给人看的一句话。
///
/// 关键在 `is_panic()` 分支：任务 panic 时 `JoinError` 的 `Display` 只有
/// "task panicked: ..."，真正的现场在 panic payload 里，必须 `try_into_panic()`
/// 下来再 downcast 才能拿到（bpi-rs/rustls 的 panic 消息就是靠这个暴露的）。
pub fn describe_join_error(error: tokio::task::JoinError) -> String {
    if error.is_panic() {
        let payload = error
            .try_into_panic()
            .ok()
            .and_then(|panic| {
                panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_string()))
            });

        match payload {
            Some(message) => format!("请求任务 panic: {message}"),
            None => "请求任务 panic（未提供消息）".to_string(),
        }
    } else {
        format!("请求任务被取消: {error}")
    }
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
