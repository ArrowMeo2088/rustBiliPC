//! 设置落盘：**明文 JSON**，写在 `%APPDATA%/ArrowMeo/rustBiliPC/config.json`
//! （Linux 是 `~/.config/ArrowMeo/rustBiliPC/config.json`）。
//!
//! ## 这一版起它是"真设置页"
//!
//! 2026-10-26 两次改口：上午定稿"设置保存到运行同目录的 config.json、先只做关于页"，
//! 下午（P13）改成"**放到 `%APPDATA%/ArrowMeo/rustBiliPC`**"并且设置页要真的有四项：
//! MPV 播放器位置、默认播放画质、默认播放编码、调试模式。
//! 旧位置（exe 同目录与更早的 `data/cookies.json`）**一行兼容代码都不留**，也不做迁移。
//!
//! 仍然守着的红线：**只放代码真的会读的字段**，不做"显示了但不生效的假开关"。
//! 现在这四个字段每一个都有读取方：`max_quality` → 请求 `qn` 与选流封顶（`api::stream`），
//! `codec` → 选流优先级（`api::stream_select`），`mpv_path` → `player::resolve`，
//! `debug_log` → `logging::init` 与 `main.rs`。
//!
//! ## 画质为什么只有三档可选
//!
//! 他要的是"1080P、1080P 60帧、2K、4K"。但 B 站的 `qn` 是**固定档位代码**
//! （`ref/bpi-rs/src/models/stream.rs:8-38` 的 `VideoQuality`：80/112/116/120/125/126/127），
//! **没有"2K"这一档**——1440p 的内容是在 120 这一档里给的。
//! 所以这里给 1080P(80) / 1080P60(116) / 4K(120) 三个真档，并在设置页把这句话写在下面：
//! 摆一个"2K"按钮其实等于 4K，就是假开关。要加 112（1080P 高码率）或 127（8K）随时能加，
//! 但那是"接口有的档"，不是"我们造的名"。
//!
//! ## 上限不再是硬红线
//!
//! 之前 116 是**硬上限**（目标机 i5-6200U 解不动 4K/10bit HEVC、4K 还要大会员）。
//! 现在他要在设置里能选到 4K，所以红线改成**默认值**：默认仍 116，上限放到接口里最高的 127。
//! 代价照旧写在界面上——选了 4K/8K 之后目标机大概率软解丢帧，且需要大会员。
//!
//! ## 读不到怎么办
//!
//! 文件不存在 → 写一份默认的出去；JSON 坏了 → 也用默认值，**并且这一轮强制开日志**
//! （见 [`LoadOutcome::RecoveredFromBroken`]）：不然"配置坏了 + 默认不写日志"= 他既看不到错误、
//! 也没法打开调试模式，因为调试模式就在那个坏文件里。

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// 画质档上限的下界（360P）。B 站还有 6(240P)，当"上限"没有意义。
pub const MIN_QUALITY_ID: u64 = 16;
/// 画质档上限的上界：**接口里最高的那一档**（127=8K，`models/stream.rs:36`）。
/// 不再是"我们替目标机做的 116 硬红线"——那条现在是默认值。
pub const MAX_QUALITY_ID: u64 = 127;
/// 默认上限：1080P 60帧。目标机（i5-6200U）能硬解的最高档，也是老红线的位置。
pub const DEFAULT_QUALITY_ID: u64 = 116;

/// 4K 档的 `qn`（`models/stream.rs:31` 的 `P4K`）。
///
/// 单独给个名字是因为有两个地方要用同一个数：设置页的选项表，和
/// `play_url` 请求里"要不要开 `fourk` 这一位"（`api::stream::fetch_streams`）。
pub const QUALITY_4K: u64 = 120;

/// 设置页给选的画质档。**只有这三档是 B 站真的有的**（理由见文件头"画质为什么只有三档可选"）。
pub const QUALITY_CHOICES: [(u64, &str); 3] = [
    (80, "1080P"),
    (116, "1080P 60帧"),
    (QUALITY_4K, "4K 超清"),
];

/// 编码偏好。B 站的 `codecs` 是 RFC 6381 字符串（`hev1.*`/`avc1.*`/`av01.*`），
/// 这三个名字与参考项目的 `PreferCodecType{H265,H264,Av1}` 同一套口径。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodecPreference {
    /// HEVC/H.265（默认：目标机能硬解 8bit，且同画质下码率最低）。
    #[default]
    Hevc,
    /// AVC/H.264（兼容性最好，硬解一定够，但同画质更占带宽）。
    Avc,
    /// AV1（**Skylake 没有 AV1 硬解**，选它大概率软解丢帧）。
    Av1,
}

impl CodecPreference {
    /// 设置页那一排按钮的标签。
    pub fn label(self) -> &'static str {
        match self {
            Self::Hevc => "HEVC",
            Self::Avc => "AVC",
            Self::Av1 => "AV1",
        }
    }

    /// 这个偏好在目标机上的代价，写在设置页下面一行（不选完才发现卡）。
    pub fn caveat(self) -> &'static str {
        match self {
            Self::Hevc => "默认。i5-6200U 能硬解 8bit HEVC，同画质码率最低。",
            Self::Avc => "兼容性最好、硬解一定够，但同画质更占带宽。",
            Self::Av1 => "注意：Skylake 没有 AV1 硬解，选它大概率软解丢帧。",
        }
    }
}

/// 设置结构。字段名即 JSON 键名，新增字段一律带 `#[serde(default)]`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// 请求与选流时的画质档上限（B 站的 `qn` 代码）。
    #[serde(default = "default_quality")]
    pub max_quality: u64,
    /// 编码偏好（同画质档内的优先级）。
    #[serde(default)]
    pub codec: CodecPreference,
    /// mpv 可执行文件的**显式路径**。空串 = 自动：先查 PATH 里的 `mpv.exe`，再退回运行目录。
    #[serde(default)]
    pub mpv_path: String,
    /// 调试模式：`true` 才创建 `debug.log` 并输出日志。默认关闭。
    #[serde(default)]
    pub debug_log: bool,
}

fn default_quality() -> u64 {
    DEFAULT_QUALITY_ID
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_quality: default_quality(),
            codec: CodecPreference::default(),
            mpv_path: String::new(),
            debug_log: false,
        }
    }
}

impl Config {
    /// `static RwLock<Config>` 的初始化要用 const fn，`Default::default()` 不是 const，
    /// 所以这里另给一个只给默认值的 const 构造。
    pub const fn const_default() -> Self {
        Self {
            max_quality: DEFAULT_QUALITY_ID,
            codec: CodecPreference::Hevc,
            mpv_path: String::new(),
            debug_log: false,
        }
    }

    /// 配置文件路径：`<设置目录>/config.json`（目录见 [`crate::app::settings_dir`]）。
    pub fn path() -> PathBuf {
        crate::app::settings_dir().join("config.json")
    }

    /// 从 JSON 文本解析并收一次边界。坏文本返回 `None`（调用方用默认值）。
    ///
    /// 单独做成函数是为了让这段逻辑能进单测（不碰文件系统）。
    pub fn from_json(raw: &str) -> Option<Self> {
        match serde_json::from_str::<Config>(raw) {
            Ok(config) => {
                let clamped = clamp_quality(config.max_quality);
                if clamped != config.max_quality {
                    log::warn!(
                        "config.json 里的 max_quality={} 超出允许范围，本次按 {clamped} 处理",
                        config.max_quality
                    );
                }
                Some(Config {
                    max_quality: clamped,
                    ..config
                })
            }
            Err(error) => {
                log::warn!("配置文件无法解析，本次用默认设置: {error}");
                None
            }
        }
    }

    /// 写出（覆盖）。目录不存在会先建——设置页每次改动都走这里。
    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("创建设置目录失败: {}", dir.display()))?;
        }
        let json = serde_json::to_string_pretty(self).context("序列化设置失败")?;
        std::fs::write(&path, json).with_context(|| format!("写入设置失败: {}", path.display()))?;
        Ok(path)
    }

    /// 读配置；读不到就写一份默认的出去再用默认值（结果见 [`LoadOutcome`]）。
    ///
    /// 做成关联函数是为了和另外两个落盘点对称（`Auth::load`、`logging::init` 都是各自模块的一个入口）。
    pub fn load_or_create() -> (Config, LoadOutcome) {
        let config = Config::default();
        match std::fs::read_to_string(Config::path()) {
            Ok(raw) => match Config::from_json(&raw) {
                Some(loaded) => (loaded, LoadOutcome::Loaded),
                // 文件在、但读不出来（坏了/权限）：按"重建"处理，并让调用方强制开日志。
                None => {
                    let outcome = match config.save() {
                        Ok(_) => LoadOutcome::RecoveredFromBroken,
                        Err(error) => {
                            log::error!("重建 config.json 失败: {error}");
                            LoadOutcome::RecoveredFromBroken
                        }
                    };
                    (config, outcome)
                }
            },
            Err(_) => {
                let outcome = match config.save() {
                    Ok(path) => {
                        log::info!("没有可用的 config.json，已写入默认设置：{}", path.display());
                        LoadOutcome::Created
                    }
                    Err(error) => {
                        log::warn!("写入默认设置失败，本次仍用默认值: {error}");
                        LoadOutcome::Created
                    }
                };
                (config, outcome)
            }
        }
    }
}

/// 把任意写法收回 `[MIN_QUALITY_ID, MAX_QUALITY_ID]`。单独做成函数是为了进单测。
pub fn clamp_quality(raw: u64) -> u64 {
    raw.clamp(MIN_QUALITY_ID, MAX_QUALITY_ID)
}

/// 读设置的结果。`main.rs` 用它决定"这一轮要不要强制开日志"。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadOutcome {
    /// 读到了可用的 `config.json`。
    Loaded,
    /// 文件本来不存在，我们写了一份默认的出去。
    Created,
    /// 文件在、但读不出来（坏了/权限），本次用默认值并顺手覆盖写回。
    ///
    /// 这一档**强制开日志**：调试模式就写在那个坏文件里，不开日志他就永远看不到为什么没日志。
    RecoveredFromBroken,
}

/// 画质档 → 中文读法。认得出的档用 B 站自己的叫法（`models/stream.rs` 的注释），
/// 认不出的原样带上数字，界面绝不编一个"应该是 1080P"。
pub fn quality_name(qn: u64) -> String {
    let label = match qn {
        6 => "240P",
        16 => "360P",
        32 => "480P",
        64 => "720P",
        74 => "720P 60帧",
        80 => "1080P",
        112 => "1080P 高码率",
        116 => "1080P 60帧",
        120 => "4K 超清",
        125 => "HDR 真彩色",
        126 => "杜比视界",
        127 => "8K 超高清",
        other => return format!("未知档（qn={other}）"),
    };
    format!("{label}（qn={qn}）")
}
