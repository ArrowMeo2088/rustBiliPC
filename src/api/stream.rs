//! 播控：拿某一 P 的可播地址，并选出要交给 mpv 的那一对流。
//!
//! ## 一条 DASH 还是两段
//!
//! B 站普通稿件是**音视频分开的两条 DASH 流**，所以 `StreamChoice` 里 `audio_url` 是
//! `Option`：正常情况下有单独的音频轨（mpv 用 `--audio-file=` 附加）；只有当 B 站回退成
//! `durl`（FLV/MP4 直链，一段文件里已经含音频）时才是 `None`，这时 mpv 只需要主文件。
//! 这两条路的差别必须留到界面/日志上说得清，否则"没声音"和"只有声音"两种坏法长得很像。
//!
//! ## fnval
//!
//! `16 | 2048` = DASH + AV1。**没有单独的 HEVC 位**：开了 DASH 之后 `dash.video[]` 里
//! HEVC/AVC/AV1 一起给，靠 `codecs` 区分（`docs/api-notes.md` 第 5 节）。
//! 4K/HDR/杜比那几位（128/64/256/512/1024）里，**只有 4K 这一位跟着设置走**：
//! `.fourk(封顶 >= 120)`（P13 起设置页能选 4K）。HDR(125)/杜比(126)/8K(127) 仍然不开——
//! 目标机解不动，而且那三档在 B 站的 `support_formats` 里也不是靠 `fourk` 一位放出来的。
//! 请求 `qn`（[`request_qn`]，值来自 `config.json` 的 `max_quality`）与 `.fourk` 一起
//! 决定拿到的流在不在封顶以内；编码优先级由 [`preferred_codec`] 决定（同档内谁先）。
//!
//! ## 域名
//!
//! `baseUrl` 之外 B 站还给一组 `backupUrl`。这里原样带出去（视频、音频各一组），
//! **由谁用、什么时候换**不在这一层决定——那需要真实网络结果：
//! 下载是 [`crate::api::download`] 在主地址失败后按顺序换下一个；在线播不用它，
//! 因为 mpv 自己对同一 URL 有重试逻辑，我们替换不了它内部的地址。

use std::sync::Arc;

use bpi_rs::{
    BpiClient, BpiError, BpiResult,
    ids::{Bvid, Cid},
    video::VideoPlayUrlParams,
    video::videostream_url::SupportFormat,
};

use super::spawn_on_api_runtime;
use super::stream_select;

/// 请求位掩码：DASH + AV1。
const FNVAL_DASH_AV1: u64 = 16 | 2048;

/// 起播时想要的画质档：**读进程级设置**（`config.json` 的 `max_quality`，
/// 默认 116=1080P60，P13 起用户能在设置页选到 4K）。
///
/// 页面不需要知道 `stream_select` 与 config 的存在（分层规则），起播一律用这个函数；
/// 它同时也是选流的封顶值——见 [`fetch_streams`] 里把同一个数传给
/// [`pick_video`](crate::api::stream_select::pick_video)，请求档与选择档不会各说一套。
pub fn request_qn() -> u64 {
    crate::app::config().max_quality
}

/// 编码偏好（`config.json` 的 `codec`）。同画质档内谁优先由它决定。
pub fn preferred_codec() -> crate::store::config::CodecPreference {
    crate::app::config().codec
}

/// 一个可选清晰度。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QualityOption {
    /// 画质档（B 站的 qn）。
    pub qn: u64,
    /// 中文描述，如"1080P 高清"。
    pub label: String,
}

/// 选定的一对流 + 换清晰度要用的信息。
#[derive(Clone, Debug)]
pub struct StreamChoice {
    /// 这份地址是为哪个 `cid` 取的。详情页用它判断"手里这份计划还能不能给当前选中的 P 用"
    /// （播放与下载都要这个：换了 P 就不能拿旧地址凑）。
    pub cid: u64,
    /// 视频（或 durl 回退时的唯一直链）。
    pub video_url: String,
    /// 单独的音频轨；`durl` 回退时是 `None`。
    pub audio_url: Option<String>,
    /// 视频轨的备用域名（下载时主地址失败就按顺序换）。
    pub backup_urls: Vec<String>,
    /// 音频轨的备用域名。
    pub audio_backup_urls: Vec<String>,
    /// 视频轨字节数（`dash.video[].size`，B 站不给就是 `None`）。
    pub video_size: Option<u64>,
    /// 音频轨字节数（`dash.audio[].size`）。
    pub audio_size: Option<u64>,
    /// 视频落盘用的后缀（见 [`file_extension`]）。
    pub video_ext: &'static str,
    /// 音频落盘用的后缀。
    pub audio_ext: &'static str,
    /// 实际拿到的画质档（B 站可能给得比我们要求的低）。
    pub quality: u64,
    /// 画质的中文描述。
    pub quality_label: String,
    /// 编码字符串（`hev1.1.6.L150.90` 之类），界面与日志都要看它。
    pub codecs: String,
    /// 这个 P 能切的清晰度（来自 `accept_quality`，已按封顶筛过）。
    pub available: Vec<QualityOption>,
    /// true = DASH 双流；false = durl 单文件。
    pub is_dash: bool,
    /// 全片长度，**毫秒**（`timelength` 的单位，别和 DASH 里的秒混）。
    pub timelength_ms: u64,
}

/// 给一个画质档找中文描述。
///
/// 优先 `support_formats`（新接口，带 `new_description`），退回 `accept_quality` 与
/// `accept_description` 按下标配对；两处都没有就把档号本身写出来，至少日志能看懂。
pub fn quality_label(
    support_formats: &[SupportFormat],
    accept_quality: &[u64],
    accept_description: &[String],
    qn: u64,
) -> String {
    if let Some(found) = support_formats.iter().find(|format| format.quality == qn) {
        if !found.new_description.is_empty() {
            return found.new_description.clone();
        }
        if !found.display_desc.is_empty() {
            return found.display_desc.clone();
        }
    }
    if let Some(index) = accept_quality.iter().position(|one| *one == qn) {
        if let Some(text) = accept_description.get(index) {
            return text.clone();
        }
    }
    format!("{qn}P")
}

/// 把 `accept_quality` 与 `accept_description` 拼成可点的一排清晰度按钮的数据。
///
/// `cap` 是进程封顶（`config.json` 的 `max_quality`）：**高于它的档不给按钮**。
/// 这一条在 P11 之前是缺的——界面上能点出"4K/HDR"，点下去 `pick_video` 就按那个档位选流，
/// 等于我们自己的封顶说没就没。下载更直接："高画质下载"必须真的是封顶以内的高画质。
/// 过滤到空列表时退回原样（宁可给一个超封顶的选项，也不要一排点不动的按钮），
/// 这与 [`pick_video`](crate::api::stream_select::pick_video) 的退让口径一致。
pub fn quality_options(
    support_formats: &[SupportFormat],
    accept_quality: &[u64],
    accept_description: &[String],
    cap: u64,
) -> Vec<QualityOption> {
    // 有 support_formats 时以它为准（它就是"可切"的那份列表，顺序也是 B 站给的顺序）。
    let candidates: Vec<u64> = if !support_formats.is_empty() {
        support_formats.iter().map(|format| format.quality).collect()
    } else {
        accept_quality.to_vec()
    };
    let allowed: Vec<u64> = candidates
        .iter()
        .copied()
        .filter(|qn| *qn <= cap)
        .collect();
    let picked = if allowed.is_empty() { candidates } else { allowed };

    picked
        .into_iter()
        .map(|qn| QualityOption {
            qn,
            label: quality_label(support_formats, accept_quality, accept_description, qn),
        })
        .collect()
}

/// 一条 DASH 流落盘该用的后缀。
///
/// B 站普通稿件的 DASH 是 fragmented MP4：`mime_type` 一般是 `video/mp4` / `audio/mp4`。
/// 视频用 `.m4v`、音频用 `.m4a`（用户 2026-10-26 定的"双文件"形态就是这两个后缀）；
/// 万一给的是 webm 就照实用——**不硬把 webm 改名成 m4v**，那只会让播放器猜错容器。
pub fn file_extension(mime_type: &str, is_audio: bool) -> &'static str {
    if mime_type.contains("webm") {
        "webm"
    } else if is_audio {
        "m4a"
    } else {
        "m4v"
    }
}

/// 日志里"可能没有大小"的写法：不知道就写短横，不写 0（0 会被读成"真是个空文件"）。
fn bytes_text(bytes: Option<u64>) -> String {
    bytes
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// `durl` 回退路径的后缀（B 站给的 `format` 原样小写：`mp4`/`flv`/`mpg`…）。
///
/// 认不出来的一律当 `mp4` 写：这条路这一版**不允许下载**（分段直链要按 `order` 拼接，
/// 见 [`crate::api::download`]），后缀只是给日志与播放行一个说法。
pub fn durl_extension(format: &str) -> &'static str {
    match format {
        "flv" => "flv",
        "mpg" => "mpg",
        _ => "mp4",
    }
}

/// 取某一 P 的播放地址并选好流。
pub fn fetch_streams(
    client: Arc<BpiClient>,
    bvid: String,
    cid: u64,
    want_qn: u64,
) -> tokio::task::JoinHandle<BpiResult<StreamChoice>> {
    log::debug!("提交 playurl 到 API runtime：{bvid} cid={cid} 想要 qn={want_qn}");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let preferred = preferred_codec();
        let params = VideoPlayUrlParams::from_bvid(Bvid::new(bvid.clone())?, Cid::new(cid)?)
            .quality(want_qn)
            .format_flags(FNVAL_DASH_AV1)
            .high_quality(true)
            // 封顶在 4K 以上才把 4K 那位打开：没这个偏好时 B 站压根不给 120 的流，
            // 打开了也不会让我们要到超出 `want_qn` 的东西（选流那一步仍然按 `want_qn` 夹）。
            .fourk(want_qn >= crate::store::config::QUALITY_4K);
        let data = client.video().play_url(params).await?;
        let available = quality_options(
            &data.support_formats,
            &data.accept_quality,
            &data.accept_description,
            want_qn,
        );

        // DASH 优先：音视频两条流。
        if let Some(dash) = data.dash.as_ref() {
            // 封顶值就用本次请求的 `qn`：请求档与选择档是同一个数，
            // 不会出现"要 720P 却选了 1080P"或反过来的两套说法。
            // 编码偏好只影响**同一画质档内**选哪一路（HEVC/AVC/AV1），不会越档。
            let video = stream_select::pick_video(&dash.video, want_qn, preferred).ok_or_else(|| {
                BpiError::unsupported_response("playurl 的 dash.video 里没有封顶范围内的可用流")
            })?;
            let audio = stream_select::pick_audio(&dash.audio);
            let choice = StreamChoice {
                cid,
                video_url: video.base_url.clone(),
                audio_url: audio.map(|stream| stream.base_url.clone()),
                backup_urls: video.backup_url.clone(),
                audio_backup_urls: audio.map(|stream| stream.backup_url.clone()).unwrap_or_default(),
                video_size: video.size,
                audio_size: audio.and_then(|stream| stream.size),
                video_ext: file_extension(&video.mime_type, false),
                audio_ext: audio
                    .map(|stream| file_extension(&stream.mime_type, true))
                    .unwrap_or("m4a"),
                quality: video.id,
                quality_label: quality_label(
                    &data.support_formats,
                    &data.accept_quality,
                    &data.accept_description,
                    video.id,
                ),
                codecs: video.codecs.clone(),
                available,
                is_dash: true,
                timelength_ms: data.timelength,
            };
            log::debug!(
                "playurl 选好流：qn={} codecs={} 音频={} 视频={}B 音频={}B 用时={:?}",
                choice.quality,
                choice.codecs,
                choice.audio_url.is_some(),
                bytes_text(choice.video_size),
                bytes_text(choice.audio_size),
                started.elapsed()
            );
            return Ok(choice);
        }

        // 回退：durl 是已经合成好的一路文件，里面带音频。
        if let Some(first) = data.durl.as_ref().and_then(|list| list.first()) {
            log::warn!("playurl 没给 dash，回退用 durl 单文件（qn={})", data.quality);
            return Ok(StreamChoice {
                cid,
                video_url: first.url.clone(),
                audio_url: None,
                backup_urls: first.backup_url.clone(),
                audio_backup_urls: Vec::new(),
                // 分段直链的总长要按 `order` 相加才有意义，而这一版不下载它（见 api/download.rs），
                // 所以这里**不给** size：给了就会被当成"已知大小"画进进度条。
                video_size: None,
                audio_size: None,
                video_ext: durl_extension(&data.format),
                audio_ext: "m4a",
                quality: data.quality,
                quality_label: quality_label(
                    &data.support_formats,
                    &data.accept_quality,
                    &data.accept_description,
                    data.quality,
                ),
                codecs: data.accept_format.clone(),
                available,
                is_dash: false,
                timelength_ms: data.timelength,
            });
        }

        Err(BpiError::unsupported_response(
            "playurl 既没有 dash 也没有 durl，无法起播",
        ))
    })
}
