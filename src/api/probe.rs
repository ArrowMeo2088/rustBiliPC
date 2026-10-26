//! 起播前的地址探测：**主地址不通就换 `backupUrl`**（P13，用户点名的三件事之一）。
//!
//! ## 为什么必须"先探再给"，而不是"失败之后再换"
//!
//! 播放这一路我们是**把 URL 交给 mpv**，读字节、重试、报错全在它自己肚子里
//! （见 `src/player/mod.rs` 文件头：起播之后客户端不做任何播放期控制）。
//! 它内部用的地址我们既看不到也换不了 —— 所以"主地址失败换备用域名"这件事
//! 只能在**交给它之前**完成：我们自己发一个极小的 Range 请求问一句"这个地址通吗"，
//! 通的第一个才交给 mpv。
//!
//! 下载那条路不需要这么绕（[`crate::api::download`] 是我们自己读字节，
//! 失败可以当场换下一个地址），这两处的差别就来自"谁在读字节"。
//!
//! ## 探测的口径
//!
//! - 请求头与交给 mpv 的那套**一致**（Referer + UA + 凭据 Cookie，见 [`cdn_headers`]）：
//!   不然会出现"我们探到 403、mpv 却能播"或反过来的假结论。
//! - `Range: bytes=0-0`：只问可达性，不拉流。**200 也算通**（有些 CDN 不认 Range 就直接给全量，
//!   我们立刻把响应丢掉，不落盘也不占带宽）。
//! - 只探**音视频各一条轨**：主地址通就不再探它的备用域名（happy path 一共 2 个小请求）。
//! - 超时 [`PROBE_TIMEOUT`]：探测不能把"点了播放却没窗口"变成"点了播放永远没反应"。
//!
//! ## 探测全不通的时候为什么不报错
//!
//! [`choose`] 返回 `None` 时我们**仍然把主地址交给 mpv**，只在界面那一行说明"每个地址都探不通"。
//! 理由：探测本身可能因为代理/防火墙/CDN 对 Range 的怪脾气而误判，
//! 而 mpv 自己还有一次机会；宁可多等一次"确实播不出来"，也不要因为我们的误判彻底不给播。

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::http_client::{AsyncBody, Builder, HttpClient, HttpRequestExt as _, RedirectPolicy};
use gpui_kit::BackgroundExecutor;

use crate::player::{BILIBILI_REFERER, MPV_USER_AGENT};
use crate::store::auth::Auth;

/// 单个地址的探测超时（含连接与首字节）。
///
/// 6 秒太宽容：真不通的节点会让人干等。四秒 + 最多三个候选 = 一条轨最坏 12 秒，
/// 而且**音视频两轨是并行探的**（见 [`probe_plan`]），happy path 就是一个 RTT。
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(4000);

/// 一条轨最多探几个候选地址（主地址 + 这么多备用域名）。
///
/// B 站一般给 2–3 个。全探一遍的收益递减、代价是线性叠加的等待，
/// 所以到上限就停：**剩下的地址仍然会交给 mpv 自己重试**（它内部有重试），
/// 我们只是不再替它提前问。
pub const MAX_PROBE_CANDIDATES: usize = 3;

/// 一条轨的候选地址：主地址在前，B 站给的 `backupUrl` 按原顺序跟在后面。
///
/// 去掉空串与"和主地址重复"的那几条：B 站偶尔把同一个域名同时写进 `baseUrl` 和 `backupUrl`，
/// 照原样探就是把同一个地址问两遍。
pub fn ordered_urls(primary: &str, backups: &[String]) -> Vec<String> {
    let mut urls: Vec<String> = Vec::with_capacity(backups.len() + 1);
    if !primary.is_empty() {
        urls.push(primary.to_string());
    }
    urls.extend(
        backups
            .iter()
            .filter(|url| !url.is_empty() && url.as_str() != primary)
            .cloned(),
    );
    urls
}

/// 直链要的凭据头：登录了才有 `Cookie`。
///
/// 这是 CDN 那一侧要的（与交给 mpv 的是同一份凭据），不是 API 的登录判定；
/// 没登录时返回空列表，1080P 会被 CDN 拒成 403 —— 那种失败会以"HTTP 403"原话出现在界面上，
/// 不静默降级成"能播的低画质"。
pub fn cdn_headers() -> Vec<(String, String)> {
    match Auth::load().filter(|auth| auth.is_complete()) {
        Some(auth) => vec![("Cookie".to_string(), auth.cookie_header())],
        None => Vec::new(),
    }
}

/// 状态码能不能当作"这一路能播"。
///
/// 2xx/3xx 全算通（重定向已由 `follow_redirects(FollowAll)` 跟到底，这里看到的是终态；
/// 有些 CDN 对 `Range: bytes=0-0` 直接回 200 而不是 206，那也是通）。
/// 403/404/416/5xx 都不算——403 是凭据问题，416 是 Range 不被接受，两者都不该换来换去地重试，
/// 但**探到 403 就换下一个域名**恰恰是 B 站 CDN 的常见解法（某个边缘节点抽风）。
pub fn is_playable_status(status: u16) -> bool {
    (200..=399).contains(&status)
}

/// 一条轨的探测结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeOutcome {
    /// 要用的地址；`None` = 探到的每一个都不通（调用方自己决定退不退，见文件头最后一条）。
    pub url: Option<String>,
    /// 每个失败地址一句话（带主机名，不带完整 URL：里面是签名令牌，不该显示到界面上）。
    pub failures: Vec<String>,
    /// 一共探了几个地址。`0` 表示压根没有候选（比如 durl 回退且没给备用域名）。
    pub attempted: usize,
}

impl ProbeOutcome {
    /// 没探过（候选列表是空的）——界面上不该因此多打一行字。
    pub fn untouched() -> Self {
        Self {
            url: None,
            failures: Vec::new(),
            attempted: 0,
        }
    }

    /// 有没有真的换过地址（第一个就用上 = 没换）。
    pub fn fell_back(&self) -> bool {
        self.url.is_some() && !self.failures.is_empty()
    }
}

/// 从"每个地址各自的结果"里挑第一个能用的（纯函数，边界进单测）。
///
/// `results` 的顺序就是候选地址的顺序；一旦挑中就停在这里，所以 `attempted` 是"用掉第几个"，
/// 而不是列表长度。
pub fn choose(results: &[(String, Result<u16, String>)]) -> ProbeOutcome {
    let mut failures: Vec<String> = Vec::new();
    for (index, (url, outcome)) in results.iter().enumerate() {
        match outcome {
            Ok(status) if is_playable_status(*status) => {
                return ProbeOutcome {
                    url: Some(url.clone()),
                    failures,
                    attempted: index + 1,
                }
            }
            Ok(status) => failures.push(format!("{}：HTTP {status}", host_of(url))),
            Err(message) => failures.push(format!("{}：{message}", host_of(url))),
        }
    }
    ProbeOutcome {
        url: None,
        failures,
        attempted: results.len(),
    }
}

/// 从直链里取主机名，只用于给人看。
///
/// 不用 `url` crate：我们只需要"第三个 `/` 之前那一段"，而为这一行去引一个新依赖
/// （bpi-rs 里那份 `url` 不在我们的直接依赖里）不值得。
pub fn host_of(url: &str) -> String {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host = after_scheme.split('/').next().unwrap_or("");
    if host.is_empty() {
        // 认不出 scheme 的（比如相对路径这种不该出现的东西）原样给出，不编一个空主机名。
        url.to_string()
    } else {
        host.to_string()
    }
}

/// 界面那一行的说法（纯函数）。`label` 是"视频"/"音频"。
pub fn describe(label: &str, outcome: &ProbeOutcome) -> String {
    if outcome.attempted == 0 || outcome.failures.is_empty() {
        return String::new();
    }
    let detail = outcome.failures.join("；");
    match outcome.url.as_ref() {
        Some(url) => format!(
            "{label}轨主地址不通，已换 {}（{detail}）",
            host_of(url)
        ),
        None => format!("{label}轨每个地址都探不通（{detail}），仍按主地址交给 mpv 试一次"),
    }
}

/// 按顺序探一条轨的所有候选地址，第一个通就停。
///
/// `executor` 是 gpui 的后台执行器：超时靠它的 `timer()`，**不用 tokio 的定时器**——
/// 这一段跑在 gpui 前台任务里（`cx.spawn`），而 HttpClient 是 gpui 装的那个 reqwest 客户端，
/// 与下载读字节是同一条 HTTP 路（不引第二套 HTTP 栈）。
pub async fn probe_track(
    http: Arc<dyn HttpClient>,
    urls: Vec<String>,
    headers: Vec<(String, String)>,
    executor: BackgroundExecutor,
) -> ProbeOutcome {
    let mut results: Vec<(String, Result<u16, String>)> = Vec::new();
    // 到 [`MAX_PROBE_CANDIDATES`] 就不问了：剩下的地址 mpv 自己会重试，
    // 我们只是不再替它提前一个个问（每个不通的地址都要付一整段超时）。
    for url in urls.into_iter().take(MAX_PROBE_CANDIDATES) {
        let outcome = probe_one(&http, &url, &headers, &executor).await;
        let playable = matches!(&outcome, Ok(status) if is_playable_status(*status));
        results.push((url, outcome));
        if playable {
            break;
        }
    }
    choose(&results)
}

/// 一次把**两轨都探完**，两条并行等。
///
/// 为什么要并行：串行就是"视频的 RTT + 音频的 RTT"，而这两件事毫无依赖关系——
/// 起播按钮点下去多等两百毫秒是能感觉到的。
/// `plan.audio_url` 是 `None`（durl 单文件）时返回 `None` 而**不是**一个空结果：
/// 上层要分得清"这条轨不存在"与"探了一圈没探出来"。
pub async fn probe_plan(
    http: Arc<dyn HttpClient>,
    plan: &StreamChoice,
    headers: Vec<(String, String)>,
    executor: BackgroundExecutor,
) -> (ProbeOutcome, Option<ProbeOutcome>) {
    use futures::future::Either;

    let video = probe_track(
        Arc::clone(&http),
        ordered_urls(&plan.video_url, &plan.backup_urls),
        headers.clone(),
        executor.clone(),
    );
    let audio = match plan.audio_url.as_ref() {
        // 两条分支的输出类型必须一致（`Either` 要求 `B: Future<Output = A::Output>`），
        // 所以探的那一支也包成 `Option`。
        Some(url) => Either::Left(async move {
            Some(probe_track(http, ordered_urls(url, &plan.audio_backup_urls), headers, executor).await)
        }),
        None => Either::Right(async move { None }),
    };
    // `Either` 在这里不是为了好看：`join` 要求两个 future 都摆在那儿，
    // 而"没有音频轨"这一支压根不该发请求。
    let (video, audio) = futures::future::join(video, audio).await;
    (video, audio)
}

/// 发一个 `Range: bytes=0-0` 的 GET，只看状态码，响应体立刻丢掉。
async fn probe_one(
    http: &Arc<dyn HttpClient>,
    url: &str,
    headers: &[(String, String)],
    executor: &BackgroundExecutor,
) -> Result<u16, String> {
    let request = Builder::new()
        .uri(url)
        .follow_redirects(RedirectPolicy::FollowAll)
        // 与交给 mpv、交给下载的同一条 Referer/UA（B 站 CDN 会校验）。
        .header("Referer", BILIBILI_REFERER)
        .header("User-Agent", MPV_USER_AGENT)
        .header("Range", "bytes=0-0");
    let request = headers
        .iter()
        .fold(request, |builder, (name, value)| {
            builder.header(name.as_str(), value.as_str())
        })
        .body(AsyncBody::empty())
        .map_err(|error| format!("请求拼不出来: {error}"))?;

    let send = Box::pin(http.send(request));
    let timeout = Box::pin(executor.timer(PROBE_TIMEOUT));
    match futures::future::select(send, timeout).await {
        futures::future::Either::Left((result, _timer)) => {
            let response = result.map_err(|error| format!("请求失败: {error}"))?;
            // `status()` 返回 `StatusCode` 本身（`Copy`），先拿走码再看响应体。
            Ok(response.status().as_u16())
        }
        futures::future::Either::Right((_done, _request)) => {
            Err(format!("探测超时（{} 秒）", PROBE_TIMEOUT.as_secs()))
        }
    }
}
