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
//! 1. 画质档 `id`（即 B 站 qn 代码）降序，且默认只允许 `<= MAX_QUALITY_ID`；
//! 2. 同档内编码优先级：HEVC(`hev1`/`hvc1`) > AV1(`av01`) > 其它(主要是 `avc1`)；
//! 3. 仍并列时取带宽更高的一路。
//!
//! 若过滤后为空（例如响应只给了 4K），退回全集再选一次，保证总能拿到一路。

use bpi_rs::video::videostream_url::DashStream;

/// 画质档上限：116 = 1080P 60帧。允许 16/32/64/80/112/116，
/// 排除 120(4K)/125(HDR)/126(Dolby Vision)/127(8K)。
pub const MAX_QUALITY_ID: u64 = 116;

/// 编码优先级，数值越大越优先。
///
/// `codecs` 是 B 站原样返回的 RFC 6381 字符串，例如 `hev1.1.6.L150.90`、`avc1.64.00`。
fn codec_score(codecs: &str) -> u8 {
    if codecs.starts_with("hev1") || codecs.starts_with("hvc1") {
        2
    } else if codecs.starts_with("av01") {
        1
    } else {
        0
    }
}

/// 画质档是否落在允许的封顶范围内。单独抽出来便于单元测试。
pub fn is_quality_allowed(id: u64) -> bool {
    id <= MAX_QUALITY_ID
}

/// 三路比较键：(画质档, 编码优先级, 带宽)。
fn rank(stream: &DashStream) -> (u64, u8, u64) {
    (stream.id, codec_score(&stream.codecs), stream.bandwidth)
}

/// 选出要交给 mpv 的视频流。
///
/// 只用于 `dash.video`；`durl`（FLV/MP4 直链回退路径）现阶段不处理——
/// 请求里带 DASH 标志后 B 站基本不会再返回 durl，真遇到再补。
pub fn pick_video(streams: &[DashStream]) -> Option<&DashStream> {
    let best = streams
        .iter()
        .filter(|stream| is_quality_allowed(stream.id))
        .max_by(|a, b| rank(a).cmp(&rank(b)));

    // 封顶范围内没有可用流时退回全集，宁可不降级也别播不了。
    best.or_else(|| streams.iter().max_by(|a, b| rank(a).cmp(&rank(b))))
}

/// 选出要交给 mpv 的音频流。
///
/// B 站的 `dash.audio` 用 `id` 表示码率档（30251/30280 之类最高），
/// 音频不涉及硬解能力问题，直接取带宽最大的一路。
pub fn pick_audio(streams: &[DashStream]) -> Option<&DashStream> {
    streams.iter().max_by_key(|stream| stream.bandwidth)
}
