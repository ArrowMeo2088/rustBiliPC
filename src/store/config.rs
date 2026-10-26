//! 设置落盘：**明文 JSON**，写在可执行文件同目录的 `config.json`。
//!
//! ## 为什么只有一个字段
//!
//! 2026-10-26 用户定稿："设置保存到运行同目录的 config.json"，而设置页**这一版只做关于页**
//! （他的原选项："先只做关于页"）。所以这里**不放任何界面开关**，只放**代码真的会读**的那一个值：
//! `max_quality`。它是硬解能力与会员档位的天花板，改它要靠手改文件，关于页只把当前值显示出来。
//! 以后要加项，必须先有真的读取方，再加字段和界面——**不做只显示不生效的假设置**。
//!
//! ## 上下限为什么是 16 与 116
//!
//! - 116 = 1080P 60帧。再往上的 120(4K)/125(HDR)/126(Dolby Vision)/127(8K) 目标机
//!   （i5-6200U，Skylake：没有 AV1 硬解、HEVC 只到 8bit）解不动，而且 4K 起要大会员；
//!   这条与 [`stream_select::MAX_QUALITY_ID`](crate::api::stream_select) 是同一个红线。
//! - 16 = 360P。B 站还给了 6(240P)，但那是网络极差时的兜底，当"上限"没有意义。
//! - 落在两个数之外的写法（比如手滑写了 4K 的 125 或 0）由 [`clamp_quality`] 收回来，
//!   并在日志里说明原始值——不静默改用户的文件内容之外的东西。
//!
//! ## 读不到怎么办
//!
//! 文件不存在、JSON 坏了、字段缺，一律用默认值，并**顺手写出一份完整的默认配置**：
//! 这样用户能立刻看到"有这么个文件、里面长什么样"，比让他去猜路径有用（关于页也显示这个路径）。

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::api::stream_select;

/// 画质档上限的下界（360P）。
pub const MIN_QUALITY_ID: u64 = 16;

/// 设置结构。字段名即 JSON 键名，新增字段一律带 `#[serde(default)]`（旧文件要能读）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// 请求与选流时的画质档上限（B 站的 `qn` 代码）。
    #[serde(default = "default_quality")]
    pub max_quality: u64,
}

fn default_quality() -> u64 {
    stream_select::MAX_QUALITY_ID
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_quality: default_quality(),
        }
    }
}

/// 把任意写法收回 `[MIN_QUALITY_ID, MAX_QUALITY_ID]`。单独做成函数是为了进单测。
pub fn clamp_quality(raw: u64) -> u64 {
    raw.clamp(MIN_QUALITY_ID, stream_select::MAX_QUALITY_ID)
}

/// 画质档 → 中文读法。给设置页显示当前上限用。
///
/// B 站的 `qn` 是固定档位代码，不是任意数：认得的给名字，认不出的原样带上数字，
/// 界面绝不编一个"应该是 1080P"出来骗人。
pub fn quality_name(qn: u64) -> String {
    let label = match qn {
        16 => "360P",
        32 => "480P",
        64 => "720P",
        80 => "1080P",
        112 => "1080P 高码率",
        stream_select::MAX_QUALITY_ID => "1080P 60帧",
        other => return format!("未知档（qn={other}）"),
    };
    format!("{label}（qn={qn}）")
}

impl Config {
    /// 配置文件路径：`<exe 所在目录>/config.json`。
    pub fn path() -> PathBuf {
        crate::app::exe_dir().join("config.json")
    }

    /// 从 JSON 文本解析并收一次边界。坏文本返回 `None`（调用方用默认值）。
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
                })
            }
            Err(error) => {
                log::warn!("配置文件无法解析，本次用默认设置: {error}");
                None
            }
        }
    }

    /// 写出（覆盖）。关于页不提供修改入口，所以只有"首次启动补一份默认值"会用到它。
    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::path();
        let json = serde_json::to_string_pretty(self).context("序列化设置失败")?;
        std::fs::write(&path, json).with_context(|| format!("写入设置失败: {}", path.display()))?;
        Ok(path)
    }

    /// 读配置；读不到就写一份默认的出来再用默认值。
    pub fn load_or_create() -> Self {
        let path = Self::path();
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Some(config) = Self::from_json(&raw) {
                return config;
            }
        }
        let config = Self::default();
        match config.save() {
            Ok(path) => log::info!("没有可用的 config.json，已写入默认设置：{}", path.display()),
            Err(error) => log::warn!("写入默认设置失败，本次仍用默认值: {error}"),
        }
        config
    }
}
