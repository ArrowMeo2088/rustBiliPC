//! DASH 码流选择：HEVC 优先 + 默认封顶 1080P。
//!
//! ## 为什么不是调库里的 `best_video()`
//!
//! bpi-rs 的 `best_video()` 挂在 `models::stream::VideoStreamData` 上（番剧/课程响应），
//! 普通影视的 `play_url()` 返回的是 `video::videostream_url::PlayUrlResponseData`，
//! 它的 `DashInfo`/`DashStream` 没有任何选择助手，只有裸字段。而且 `best_video()`
//! 的第一关键字是**分辨率**，有大会员时会直接选中 4K/8K HEVC——目标机（i5-6200U）
//! 硬解不到 4K/10bit HEVC，会掉软解丢帧。所以这里自己实现，并加画质档上限。
//!
//! ## 排序规则
//!
//! 1. 画质档 `id`（即 B 站 qn 代码）降序，且只允许 `<= cap`；
//! 2. 同档内**先按用户设置的编码偏好**（`config.json` 的 `codec`），
//!    偏好那一档没有时才退回默认优先级：HEVC(`hev1`/`hvc1`) > AV1(`av01`) > 其它(主要是 `avc1`)；
//! 3. 仍并列时取带宽更高的一路。
//!
//! `cap` 由调用方给（起播那条路传的就是本次请求的 `qn`，见 [`crate::api::stream::request_qn`]，
//! 它的值来自 `config.json` 的 `max_quality`）。[`DEFAULT_QUALITY_CAP`] 是那个设置的**默认值**；
//! 能选到的最高档由 [`crate::store::config::MAX_QUALITY_ID`] 管（P13 起用户能在设置页选到 4K，
//! "116"不再是红线，只是替目标机做的默认选择）。
//!
//! 若过滤后为空（例如响应只给了 4K），退回全集再选一次，保证总能拿到一路。

use bpi_rs::video::videostream_url::DashStream;

use crate::store::config::CodecPreference;

/// 画质档的**默认**上限：116 = 1080P 60帧。
///
/// 目标机（i5-6200U，Skylake）硬解只到这里：再往上的 120(4K)/125(HDR)/126(杜比)/127(8K)
/// 要么解不动、要么要大会员。P13 之前它是硬上限，现在只是默认值——用户能在设置页改。
pub const DEFAULT_QUALITY_CAP: u64 = 116;

/// 编码前缀 → 家族。`codecs` 是 B 站原样返回的 RFC 6381 字符串，例如
/// `hev1.1.6.L150.90`、`avc1.64.00`、`av01.0.08M.08`。
fn is_hevc(codecs: &str) -> bool {
    codecs.starts_with("hev1") || codecs.starts_with("hvc1")
}

fn is_av1(codecs: &str) -> bool {
    codecs.starts_with("av01")
}

fn is_avc(codecs: &str) -> bool {
    codecs.starts_with("avc1") || codecs.starts_with("avc3")
}

/// 编码优先级，数值越大越优先。
///
/// **用户偏好那一档拿 3，其它一律按"省带宽优先"排**（HEVC 2 > AV1 1 > AVC 0）。
/// 为什么偏好要压过默认顺序：他选 AV1 时就是想看 AV1（哪怕更吃 CPU），
/// 不该在"同档"里被我们偷偷换成 HEVC，然后界面上还说他用的是 AV1。
fn codec_score(codecs: &str, preferred: CodecPreference) -> u8 {
    let matches_preference = match preferred {
        CodecPreference::Hevc => is_hevc(codecs),
        CodecPreference::Avc => is_avc(codecs),
        CodecPreference::Av1 => is_av1(codecs),
    };
    if matches_preference {
        return 3;
    }
    if is_hevc(codecs) {
        2
    } else if is_av1(codecs) {
        1
    } else {
        0
    }
}

/// 画质档是否落在封顶范围内。`cap` 由调用方给（来自 `config.json` 的 `max_quality`），
/// 单独做成函数是为了进单测——边界都在这一行上。
pub fn is_quality_allowed(id: u64, cap: u64) -> bool {
    id <= cap
}

/// 三路比较键：(画质档, 编码优先级, 带宽)。
fn rank(stream: &DashStream, preferred: CodecPreference) -> (u64, u8, u64) {
    (
        stream.id,
        codec_score(&stream.codecs, preferred),
        stream.bandwidth,
    )
}

/// 选出要交给 mpv（或下载）的视频流。
///
/// 只用于 `dash.video`；`durl`（FLV/MP4 直链回退路径）现阶段不处理——
/// 请求里带 DASH 标志后 B 站基本不会再返回 durl，真遇到再补。
pub fn pick_video(
    streams: &[DashStream],
    cap: u64,
    preferred: CodecPreference,
) -> Option<&DashStream> {
    let best = streams
        .iter()
        .filter(|stream| is_quality_allowed(stream.id, cap))
        .max_by(|a, b| rank(a, preferred).cmp(&rank(b, preferred)));

    // 封顶范围内没有可用流时退回全集，宁可不降级也别播不了。
    best.or_else(|| {
        streams
            .iter()
            .max_by(|a, b| rank(a, preferred).cmp(&rank(b, preferred)))
    })
}

/// 选出要交给 mpv 的音频流。
///
/// B 站的 `dash.audio` 用 `id` 表示码率档（30251/30280 之类最高），
/// 音频不涉及硬解能力问题，直接取带宽最大的一路。
pub fn pick_audio(streams: &[DashStream]) -> Option<&DashStream> {
    streams.iter().max_by_key(|stream| stream.bandwidth)
}
