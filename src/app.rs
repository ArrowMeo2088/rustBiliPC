//! 进程级基础设施：tokio runtime 桥接、用户数据目录（[`settings_dir`]）、进程级设置快照。
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

/// 进程级设置快照。启动时 [`init_config`] 一次（来源是 [`settings_dir`] 里的 `config.json`），
/// 之后各处只读 [`config`]；设置页改一项就走 [`update_config`]（改内存 + 落盘一起做，不留两份真相）。
///
/// 为什么做成进程级而不是把 `Arc<Config>` 一路传进每个页面：真正读它的只有起播/下载那两条路
/// （请求 `qn`、选流上限、编码偏好、mpv 位置），传下去要动七个页面的构造函数；
/// 但**纯函数仍然收显式参数**（见 [`crate::api::stream_select`]），这样单测不用碰全局状态。
///
/// 用 `RwLock` 而不是 `OnceLock`：设置页现在真的能改它了（画质上限、编码偏好、mpv 路径、调试模式），
/// `OnceLock` 装一次就再也换不掉，那"改了要重启才生效"就成了假设置。
static CONFIG: std::sync::RwLock<crate::store::config::Config> =
    std::sync::RwLock::new(crate::store::config::Config::const_default());

/// 装好进程级设置（启动时用一次）。
pub fn init_config(config: crate::store::config::Config) {
    if let Ok(mut guard) = CONFIG.write() {
        *guard = config;
    }
}

/// 取当前设置。锁中毒（别处 panic 过）时退回默认值，不让一次 panic 拖死整个界面。
pub fn config() -> crate::store::config::Config {
    CONFIG
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

/// 改设置：先改内存快照，再写盘。返回写盘结果（界面上要把失败说给用户听）。
///
/// 顺序是"先内存后磁盘"：写盘失败时内存里已经是新值，界面显示的与实际生效的一致；
/// 反过来（先写盘）会出现"界面显示旧值、实际按新值跑"这种更难受的分裂。
pub fn update_config(
    change: impl FnOnce(&mut crate::store::config::Config),
) -> anyhow::Result<std::path::PathBuf> {
    let next = {
        let mut guard = CONFIG
            .write()
            .map_err(|_| anyhow::anyhow!("设置快照被别处的 panic 弄坏了"))?;
        change(&mut guard);
        guard.clone()
    };
    next.save()
}

/// 显式安装 rustls 的进程级默认 crypto provider。
///
/// 必须做，否则**任何 HTTPS 请求都会在 TLS 握手处 panic**：rustls 0.23 的依赖图里
/// `aws-lc-rs` 与 `ring` 可能同时被启用（reqwest/bpi-rs 与我们各自带到），rustls 无法自行二选一，
/// 会抛 "Could not automatically determine a process-level default crypto provider"。
/// panic 发生在 tokio 任务里，表现为 `JoinHandle` 返回 Err，界面就停在加载态。
///
/// **选 `ring`（P14 改）**：`ring` 是纯 Rust + 少量 C，而 `aws-lc-sys` 要 NASM/CMake、
/// 编译慢、代码大。我们直接依赖的 `rustls` 已经 `default-features = false` + 只开 `ring`，
/// 这里再把它显式装上，两边口径一致。
/// `install_default` 只允许装一次，重复调用返回 Err，这里降级成一条 warn。
pub fn install_tls_provider() {
    match rustls::crypto::ring::default_provider().install_default() {
        Ok(()) => log::debug!("已安装 rustls 默认 provider: ring"),
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

/// 可执行文件所在目录。现在只用于两件事：日志里报"程序装在哪"，以及 [`settings_dir`]
/// 拿不到 `APPDATA`/`HOME` 时的退路；mpv 也会顺带看一眼同目录（见 [`crate::player`]）。
///
/// 拿不到 `current_exe` 时退回当前工作目录，不 panic——这只在异常启动方式下发生。
pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 用户数据目录：**`%APPDATA%/ArrowMeo/rustBiliPC`**（Windows）或 `$XDG_CONFIG_HOME`/`~/.config`
/// 下的 `ArrowMeo/rustBiliPC`（Linux）。`auth.json`、`config.json`、`debug.log` 三样都放这里。
///
/// 2026-10-26 用户改口定稿（原话："config.json/auth.json/debug.json改为存储到
/// %APPDATA%/ArrowMeo/rustBiliPC里，不要保留任何支持旧位置的json文件的代码"）——
/// 之前的"exe 同目录"与更早的 `data/cookies.json` 两条兼容路径**全部删掉**，
/// 代码里不再去看老位置，也不做迁移。
/// 换目录的实际好处正好对上交付红线：CI 产物解到 `publish/` 不会碰到用户数据，
/// 我们也不用再担心"取产物时把别人的登录态清掉"（见 QODER.md 运行期数据那条）。
///
/// 拿不到 `APPDATA`/`HOME` 时退回 exe 同目录，不 panic——真到那一步界面上也有路径可看。
pub fn settings_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));

    let vendor = PathBuf::from("ArrowMeo").join("rustBiliPC");
    match base {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(vendor),
        _ => exe_dir().join(vendor),
    }
}

/// 确保 [`settings_dir`] 存在。三个落盘点（凭据、设置、日志）都靠它先建目录。
pub fn ensure_settings_dir() -> std::io::Result<PathBuf> {
    let dir = settings_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
