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
/// `pipe_name` 由 [`ipc::pipe_name_for`] 生成；`extra_headers` 用于追加 `Cookie: …`
/// （**必须带**：B 站的 m4s 直链校验 Referer，而 1080P 还要校验登录 cookie）。
///
/// 参数取舍说明：
/// - `--no-config`：不读用户的 `mpv.conf`，避免别人配的 `vo`/`hwdec` 影响我们；
/// - `--force-window`：无文件时也保有窗口，便于 `--idle` 场景；
/// - `--keep-open=yes`：播完不退出，进程生命周期由我们控制；
/// - `--hwdec=auto`：优先硬解，选不到自动软解（HEVC 在 6 代酷睿上可硬解 8bit）；
/// - `--rebase-start-time=yes`：双 demuxer 的时间轴对齐主要靠它（默认即 yes，写明以免误解）。
///
/// `audio_url` 是 `Option`：B 站给 DASH 时才有单独的音频轨；万一回退成 `durl`
/// （FLV/MP4 一段文件、里面已含音频），多一个空的 `--audio-file=` 反而会让 mpv 报错。
pub fn build_arguments(
    video_url: &str,
    audio_url: Option<&str>,
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
    ];
    // 外部音频轨：B 站 DASH 的音频单独一路 URL。
    if let Some(audio_url) = audio_url {
        args.push(format!("--audio-file={audio_url}"));
    }
    // 主文件必须是最后一个位置参数。
    args.push(video_url.to_string());
    args
}

/// 拉起 mpv（独立进程、独立窗口），返回子进程句柄与这条实例的管道名。
///
/// 子进程句柄要留着：留着才不会在 Windows 上泄漏进程句柄，也才有办法在用户再次起播时
/// 收掉上一个实例。丢掉它不会杀掉 mpv（`std::process::Child` 的 drop 不管子进程），
/// 所以"关掉详情页但视频还在放"这种事要避免——见 `pages/detail/view.rs` 的处理。
pub fn launch(
    video_url: &str,
    audio_url: Option<&str>,
    extra_headers: &[String],
) -> anyhow::Result<(std::process::Child, String)> {
    let exe = find_mpv_in_path()
        .ok_or_else(|| anyhow::anyhow!("PATH 里找不到 mpv，请先安装 mpv 并把它加入 PATH"))?;
    let pipe_name = ipc::pipe_name_for(std::process::id());
    let args = build_arguments(video_url, audio_url, &pipe_name, extra_headers);

    let mut command = std::process::Command::new(&exe);
    command.args(&args);
    #[cfg(windows)]
    {
        // 万一用户 PATH 里是 mpv.com（命令行启动器），也别给我们闪一个控制台窗口。
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let child = command
        .spawn()
        .map_err(|error| anyhow::anyhow!("拉起 {} 失败: {error}", exe.display()))?;
    log::info!(
        "已拉起 mpv：pid={} 管道={pipe_name} 音频={} 附加头={} 条",
        child.id(),
        audio_url.is_some(),
        extra_headers.len()
    );
    Ok((child, pipe_name))
}

/// 在 PATH 中查找 mpv 可执行文件。
///
/// 找不到是常态（多数用户没装 mpv），返回 `None` 让上层去提示用户，而不是直接失败。
pub fn find_mpv_in_path() -> Option<std::path::PathBuf> {
    which::which("mpv").ok()
}
