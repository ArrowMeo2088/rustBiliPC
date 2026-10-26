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
//! 4K/HDR/杜比那几位（128/64/256/512/1024）不开：目标机 i5-6200U 硬解不到，
//! 而且 `.fourk(false)` 与请求 `qn`（[`request_qn`]，值来自 `config.json` 的 `max_quality`，
//! 默认与硬上限 116=1080P60）一起把画质封在封顶档以内。
//!
//! ## 域名
//!
//! `baseUrl` 之外 B 站还给一组 `backupUrl`。这里原样带出去，由播放层在主地址失败时换用；
//! 我们**不**在这里决定用哪个——那需要真实网络结果。

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

/// 起播时想要的画质档：**读进程级设置**（运行目录 `config.json` 的 `max_quality`，
/// 默认与硬上限都是 [`stream_select::MAX_QUALITY_ID`]）。
///
/// 页面不需要知道 `stream_select` 与 config 的存在（分层规则），起播一律用这个函数；
/// 它同时也是选流的封顶值——见 [`fetch_streams`] 里把同一个数传给
/// [`pick_video`](crate::api::stream_select::pick_video)，请求档与选择档不会各说一套。
pub fn request_qn() -> u64 {
    crate::app::config().max_quality
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
    /// 视频（或 durl 回退时的唯一直链）。
    pub video_url: String,
    /// 单独的音频轨；`durl` 回退时是 `None`。
    pub audio_url: Option<String>,
    /// 实际拿到的画质档（B 站可能给得比我们要求的低）。
    pub quality: u64,
    /// 画质的中文描述。
    pub quality_label: String,
    /// 编码字符串（`hev1.1.6.L150.90` 之类），界面与日志都要看它。
    pub codecs: String,
    /// 备用域名，主地址失败时按顺序换。
    pub backup_urls: Vec<String>,
    /// 这个 P 能切的清晰度（来自 `accept_quality`）。
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
pub fn quality_options(
    support_formats: &[SupportFormat],
    accept_quality: &[u64],
    accept_description: &[String],
) -> Vec<QualityOption> {
    // 有 support_formats 时以它为准（它就是"可切"的那份列表，顺序也是 B 站给的顺序）。
    if !support_formats.is_empty() {
        return support_formats
            .iter()
            .map(|format| QualityOption {
                qn: format.quality,
                label: quality_label(support_formats, accept_quality, accept_description, format.quality),
            })
            .collect();
    }
    accept_quality
        .iter()
        .map(|qn| QualityOption {
            qn: *qn,
            label: quality_label(support_formats, accept_quality, accept_description, *qn),
        })
        .collect()
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
        let params = VideoPlayUrlParams::from_bvid(Bvid::new(bvid)?, Cid::new(cid)?)
            .quality(want_qn)
            .format_flags(FNVAL_DASH_AV1)
            .high_quality(true)
            .fourk(false);
        let data = client.video().play_url(params).await?;
        let available =
            quality_options(&data.support_formats, &data.accept_quality, &data.accept_description);

        // DASH 优先：音视频两条流。
        if let Some(dash) = data.dash.as_ref() {
            // 封顶值就用本次请求的 `qn`：请求档与选择档是同一个数，
            // 不会出现"要 720P 却选了 1080P"或反过来的两套说法。
            let video = stream_select::pick_video(&dash.video, want_qn).ok_or_else(|| {
                BpiError::unsupported_response("playurl 的 dash.video 里没有封顶范围内的可用流")
            })?;
            let audio = stream_select::pick_audio(&dash.audio);
            let choice = StreamChoice {
                video_url: video.base_url.clone(),
                audio_url: audio.map(|stream| stream.base_url.clone()),
                quality: video.id,
                quality_label: quality_label(
                    &data.support_formats,
                    &data.accept_quality,
                    &data.accept_description,
                    video.id,
                ),
                codecs: video.codecs.clone(),
                backup_urls: video.backup_url.clone(),
                available,
                is_dash: true,
                timelength_ms: data.timelength,
            };
            log::debug!(
                "playurl 选好流：qn={} codecs={} 音频={} 用时={:?}",
                choice.quality,
                choice.codecs,
                choice.audio_url.is_some(),
                started.elapsed()
            );
            return Ok(choice);
        }

        // 回退：durl 是已经合成好的一路文件，里面带音频。
        if let Some(first) = data.durl.as_ref().and_then(|list| list.first()) {
            log::warn!("playurl 没给 dash，回退用 durl 单文件（qn={})", data.quality);
            return Ok(StreamChoice {
                video_url: first.url.clone(),
                audio_url: None,
                quality: data.quality,
                quality_label: quality_label(
                    &data.support_formats,
                    &data.accept_quality,
                    &data.accept_description,
                    data.quality,
                ),
                codecs: data.accept_format.clone(),
                backup_urls: first.backup_url.clone(),
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
