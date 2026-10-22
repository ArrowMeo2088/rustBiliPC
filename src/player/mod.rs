//! 播放层：外置 mpv 进程（独立窗口，不做窗口嵌入）。
//!
//! ## 设计定稿
//!
//! 视频不由 gpui 渲染。我们只负责**拼好启动参数、拉起 mpv 进程、通过命名管道遥控**。
//! 好处是进度条/音量/全屏/字幕显示全部由 mpv 自带的 OSC 负责，我们不用造播放器的轮子；
//! 代价是播放器是另一个窗口，详情页只做"发起 + 观测状态"。
//!
//! ## B 站是音视频双 DASH 流
//!
//! 一路视频 URL + 一路音频 URL。mpv 的 `--audio-file=` 是 `--audio-files-append` 的
//! 命令行别名，接受远程 URL（`player/loadfile.c` 里走 `mp_is_url` 分支），
//! 所以是"视频当主文件、音频当外部轨"这一条路。
//!
//! ## 请求头是实例级全局的
//!
//! `--http-header-fields` / `--user-agent` / `--referrer` 都属于 mpv 的
//! `mp_network_conf` 组，对同一个实例里的**所有** HTTP 流生效（视频、外部音频、字幕）。
//! 因此换清晰度时用 `loadfile` + `audio-add` 复用同一实例是安全的；
//! 反过来，**URL 必须和这套头一起给出**，否则 403。
//!
//! ## 为什么必须带 Referer
//!
//! B 站的 m4s 直链校验 Referer。bpi-rs 自己请求时会加，但它不外发这份头，
//! 所以下面这两个常量要手动与 B 站的要求保持一致。

pub mod ipc;

/// 交给 mpv 的 Referer。值必须是 bilibili 域，末尾不带斜杠更稳。
pub const BILIBILI_REFERER: &str = "https://www.bilibili.com";

/// 交给 mpv 的 User-Agent。
///
/// 用桌面 Chrome UA：部分 CDN 对空 UA 或非浏览器 UA 会限速或拒绝。
pub const MPV_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// 构造一次播放所需的 mpv 命令行参数（不含可执行文件名）。
///
/// `pipe_name` 由 [`ipc::pipe_name_for`] 生成；`extra_headers` 用于以后追加 Cookie。
///
/// 参数取舍说明：
/// - `--no-config`：不读用户的 `mpv.conf`，避免别人配的 `vo`/`hwdec` 影响我们；
/// - `--force-window`：无文件时也保有窗口，便于 `--idle` 场景；
/// - `--keep-open=yes`：播完不退出，进程生命周期由我们控制；
/// - `--hwdec=auto`：优先硬解，选不到自动软解（HEVC 在 6 代酷睿上可硬解 8bit）；
/// - `--rebase-start-time=yes`：双 demuxer 的时间轴对齐主要靠它（默认即 yes，写明以免误解）。
pub fn build_arguments(
    video_url: &str,
    audio_url: &str,
    pipe_name: &str,
    extra_headers: &[String],
) -> Vec<String> {
    let mut header_fields = vec![format!("Referer: {BILIBILI_REFERER}")];
    header_fields.extend(extra_headers.iter().cloned());

    let mut args = vec![
        "--no-config".to_string(),
        "--force-window".to_string(),
        "--keep-open=yes".to_string(),
        "--idle=once".to_string(),
        "--hwdec=auto".to_string(),
        // Windows 上 mpv 会自动补 `\\.\pipe\` 前缀，但我们还是写全，避免歧义。
        format!("--input-ipc-server={pipe_name}"),
        "--user-agent".to_string(),
        MPV_USER_AGENT.to_string(),
        "--referrer".to_string(),
        BILIBILI_REFERER.to_string(),
        "--http-header-fields".to_string(),
        header_fields.join(","),
        "--rebase-start-time=yes".to_string(),
        // 外部音频轨：B 站 DASH 的音频单独一路 URL。
        format!("--audio-file={audio_url}"),
        // 主文件必须是最后一个位置参数。
        video_url.to_string(),
    ];
    args.shrink_to_fit();
    args
}

/// 在 PATH 中查找 mpv 可执行文件。
///
/// 找不到是常态（多数用户没装 mpv），返回 `None` 让上层去提示用户，而不是直接失败。
pub fn find_mpv_in_path() -> Option<std::path::PathBuf> {
    which::which("mpv").ok()
}
