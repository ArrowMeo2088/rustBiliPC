//! 播放层：外置 mpv 进程（独立窗口，不做窗口嵌入）。
//!
//! ## 设计定稿：播放期的一切交给 mpv，我们只做"播放之外"
//!
//! 视频不由 gpui 渲染。我们只负责**选好流、拼好启动参数、拉起 mpv 进程、管住它的生死**。
//! 进度条、时间、音量、全屏、暂停、倍速、字幕显示、弹幕渲染全部由 mpv 自带的 OSC 与
//! 快捷键负责——**客户端里不做播放时间轴，也不做暂停/跳转按钮**（2026-10-23 用户定稿：
//! "视频播放相关的交给 mpv，rustBiliPC 做的是播放之外的：主页、搜索、个人、详情、评论、
//! 分集/分P、清晰度"）。
//!
//! 也正因为这样，这一层**不接 mpv 的命名管道**：曾经做过一版 `ipc.rs` + `state.rs`
//! （订阅 `time-pos` 之类画进度行），按上面那条范围划掉删干净了，别照着 git 历史又捡回来。
//! 唯一还需要"播放状态"的场合是**换清晰度**——现在的做法是重新取一次地址、重新起一个实例，
//! 不试图去遥控旧实例。
//!
//! ## B 站是音视频双 DASH 流，而且是在线播不是下载
//!
//! 一路视频 URL + 一路音频 URL。mpv 的 `--audio-file=` 是 `--audio-files-append` 的
//! 命令行别名，接受远程 URL（`player/loadfile.c` 里走 `mp_is_url` 分支），
//! 所以是"视频当主文件、音频当外部轨"这一条路。**我们只把 URL 交给它**，
//! 读字节、切片、缓存、seek 全归 mpv 自己的 demuxer，绝不先把片子下到磁盘。
//!
//! ## 请求头是实例级全局的
//!
//! `--http-header-fields` / `--user-agent` / `--referrer` 都属于 mpv 的
//! `mp_network_conf` 组（`ref/mpv/stream/network.c:34-40`），对同一个实例里的
//! **所有** HTTP 流生效（视频、外部音频、字幕）。所以 **URL 必须和这套头一起给出**，否则 403。
//!
//! ## 为什么头是用逗号拼成一个参数
//!
//! `--http-header-fields` 是 string list（`stream/network.c:37` 的 `OPT_STRINGLIST`），
//! mpv 用 `,` 切分参数（`options/m_option.c:1403-1440` 的 `separate_input_param` 配合
//! `m_option.h:644` 的 `OPTION_LIST_SEPARATOR`），而**在命令行上重复给同一个列表选项
//! 是"替换"而非"追加"**（默认操作 `OP_NONE` 直接 `VAL(dst) = res`）。
//! 所以多条头必须拼成一次给出，别写两个 `--http-header-fields`。
//! 副作用是值里不能出现裸逗号——已对着现场凭据核过：四个 cookie 字段（含 222 字符的
//! `SESSDATA`）都不含逗号与空格，所以拼起来不会被切坏。
//!
//! ## mpv 可执行文件的位置：运行同目录，不用 PATH
//!
//! 定稿（2026-10-23）：**只认 `<exe 同目录>/mpv.exe`**。原因不是"找不到"，而是找到的是错的东西：
//! PATH 上排第一的是 `mpv.com`（`PATHEXT` 把 `.COM` 排在 `.EXE` 前面），而控制台版 mpv 是个
//! 会**再派生一个 mpv.exe** 的启动器——我们 `kill` 的是外层那个，真正的播放器不归我们管。
//! 用户机器上那份还属于 mpv-lazy 懒人包（带 `umpv.conf` 的单实例启动器与 `portable_config`），
//! 拿它当"我们的播放进程"更不可控。原版 mpv 放进运行目录后，进程树是我们自己的一条。
//!
//! ## 参数必须写成 `--opt=value`，而且只给所有构建都存在的选项
//!
//! mpv 的命令行**不接受** `--opt value`：文档只额外承认单连字符的 legacy 形式
//! `-opt value`（`ref/mpv/DOCS/man/mpv.rst:440-444`），而 `--opt value` 里 mpv 会把值当成
//! **文件名**并报 `Error parsing commandline option user-agent: option requires parameter` →
//! `Exiting... (Fatal error)`。这是**建窗口之前**的致命错误，症状就是"点了播放什么都没弹出来"
//! （第十八轮真机踩的就是它，原文记在 `docs/architecture.md` 第 8 节）。
//!
//! 推论：别给"某些构建不认"的选项。`--no-ytdl` 看着合理（我们给的是解析好的直链，
//! 不需要 URL 提取器），但 `{"ytdl", …}` 整块在 `#if HAVE_LUA` 里
//! （`ref/mpv/options/options.c:570-574`），不带 Lua 的构建会把它当未知选项，症状又回到
//! "静默不出现窗口"。参数表里留下的这些是无条件的：`http-header-fields`/`user-agent`/`referrer`
//! （`stream/network.c:34-40`）。
//!
//! ## 不接 mpv 的文本输出
//!
//! GUI 宿主没有控制台，很容易误以为得把 mpv 的 stdout/stderr 抄进 `debug.log`。定稿是不接，
//! 但**必须显式 `Stdio::null()`**：`piped()` 之后没人抽水，缓冲区写满会把 mpv 的写阻塞住，
//! 症状是"播着播着卡住、又不报错"。顺带记一条事实免得再绕回去：mpv 的消息默认写 **stdout**，
//! 只有 `force_stderr` 才走 stderr（`ref/mpv/common/msg.c:190-202`；本机实测一次失败的打开
//! stdout 613 字节、stderr 0 字节）。要查 mpv 为什么不播，就在本机把同一套参数复跑一遍——
//! 一次本地复现比一轮编译便宜。

/// 交给 mpv 的 Referer。值必须是 bilibili 域，末尾不带斜杠更稳。
pub const BILIBILI_REFERER: &str = "https://www.bilibili.com";

/// 交给 mpv 的 User-Agent。
///
/// 用桌面 Chrome UA：部分 CDN 对空 UA 或非浏览器 UA 会限速或拒绝。
pub const MPV_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// mpv 可执行文件的约定位置：`<exe 同目录>/mpv.exe`。
pub fn mpv_path() -> std::path::PathBuf {
    crate::app::exe_dir().join("mpv.exe")
}

/// mpv 是否就放在运行目录。
///
/// 找不到是常态（用户还没放），返回 `None` 让上层去提示该放哪，而不是直接失败。
pub fn find_mpv() -> Option<std::path::PathBuf> {
    let path = mpv_path();
    path.is_file().then_some(path)
}

/// 构造一次播放所需的 mpv 命令行参数（不含可执行文件名）。
///
/// `extra_headers` 用于追加 `Cookie: …`（**必须带**：B 站的 m4s 直链校验 Referer，
/// 1080P 还要校验登录 cookie）。
///
/// **带值的选项一律写成 `--opt=value`**，见文件头那条：`--opt value` 是致命错误，
/// mpv 会在建窗口前退出。由单测
/// `options_never_take_their_value_as_a_separate_argument` 把这条形状钉住。
///
/// 参数取舍说明：
/// - `--no-config`：不读用户的 `mpv.conf`，避免别人配的 `vo`/`hwdec` 影响我们；
/// - `--force-window`：无文件时也保有窗口，便于 `--idle` 场景；
/// - `--keep-open=yes`：播完不退出，进程生命周期由我们控制；
/// - `--idle=once`：起播失败时不要一直挂着（配合上面一条，播完仍留着窗口给我们 kill）；
/// - `--hwdec=auto`：优先硬解，选不到自动软解（HEVC 在 6 代酷睿上可硬解 8bit）；
/// - `--rebase-start-time=yes`：双 demuxer 的时间轴对齐主要靠它（默认即 yes，写明以免误解）。
///
/// `audio_url` 是 `Option`：B 站给 DASH 时才有单独的音频轨；万一回退成 `durl`
/// （FLV/MP4 一段文件、里面已含音频），多一个空的 `--audio-file=` 反而会让 mpv 报错。
pub fn build_arguments(
    video_url: &str,
    audio_url: Option<&str>,
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
        format!("--user-agent={MPV_USER_AGENT}"),
        format!("--referrer={BILIBILI_REFERER}"),
        format!("--http-header-fields={}", header_fields.join(",")),
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

/// 拉起 mpv（独立进程、独立窗口），返回子进程句柄。
///
/// 子进程句柄要留着：留着才不会在 Windows 上泄漏进程句柄，也才有办法在用户再次起播时
/// 收掉上一个实例。丢掉它不会杀掉 mpv（`std::process::Child` 的 drop 不管子进程），
/// 所以"关掉详情页但视频还在放"这种事要避免——见 `pages/detail/view.rs` 的 `Drop`。
pub fn launch(
    video_url: &str,
    audio_url: Option<&str>,
    extra_headers: &[String],
) -> anyhow::Result<std::process::Child> {
    let exe = find_mpv().ok_or_else(|| {
        anyhow::anyhow!(
            "运行目录里没有 mpv.exe（应放在 {}）",
            mpv_path().display()
        )
    })?;
    let args = build_arguments(video_url, audio_url, extra_headers);

    let mut command = std::process::Command::new(&exe);
    command.args(&args);
    // 见文件头"不接 mpv 的文本输出"那条：一定要 `null()`，不能 `piped()`。
    command.stdout(std::process::Stdio::null());
    command.stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        // mpv.exe 是 GUI 子系统程序，正常不需要这个；留着是为了万一放进来的是控制台版构建。
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let child = command
        .spawn()
        .map_err(|error| anyhow::anyhow!("拉起 {} 失败: {error}", exe.display()))?;
    log::info!(
        "已拉起 mpv：pid={} exe={} 音频={} 附加头={} 条",
        child.id(),
        exe.display(),
        audio_url.is_some(),
        extra_headers.len()
    );
    Ok(child)
}
