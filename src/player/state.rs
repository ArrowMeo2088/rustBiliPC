//! mpv 回传的属性 → 播放状态（纯数据，无 gpui，可单测）。
//!
//! ## 为什么单独一层
//!
//! 界面只该问"现在什么状态"，不该自己解析 `property-change`。这里把
//! [`crate::player::ipc::IpcEvent`] 折成几个字段，并且**把进度文案也管在这里**：
//! "没进度"该显示 `--:--` 而不是 `00:00`，这种口径写散到页面里就一定会有人写错。
//!
//! ## 为什么状态只信 mpv
//!
//! 点下暂停按钮时我们**不**本地翻转 `paused`：mpv 是真相来源，回包与属性变化会把它带回来。
//! 本地猜一次就会在"命令失败"时留下一个和实际不符的按钮，比少一次反馈难受得多。
//! 属性值也可能是 `null`（`seek` 到未加载完的流时 `time-pos` 就是 null），
//! 所以数值一律 `Option`，`as_f64()`/`as_bool()` 拿不到就当没这回事。

use serde_json::Value;

use super::ipc::IpcEvent;

/// 一次起播的播放状态。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayState {
    /// 管道连上了（连上才谈得上"播放中/已暂停"）。
    pub connected: bool,
    /// `time-pos`，秒。未加载完时 mpv 会给 `null`。
    pub position: Option<f64>,
    /// `duration`，秒。
    pub duration: Option<f64>,
    /// `pause`。
    pub paused: bool,
    /// `paused-for-cache`：在缓冲，不是用户按的暂停。
    pub buffering: bool,
    /// `eof-reached`：放过一遍了（我们配了 `--keep-open=yes`，所以进程还在）。
    pub ended: bool,
    /// `idle-active`：mpv 手上没有可播的流（刚起、或地址打不开）。
    pub idle: bool,
    /// 最近一条非 `success` 的命令回包，成功过一次就清掉。
    pub last_error: Option<String>,
}

impl PlayState {
    /// 把一个事件并进状态。返回 `Some(文案)` 表示这条值得直接显示给用户。
    pub fn apply(&mut self, event: IpcEvent) -> Option<String> {
        match event {
            IpcEvent::Connected => {
                self.connected = true;
                None
            }
            IpcEvent::ConnectFailed(message) => {
                self.connected = false;
                Some(format!("连上 mpv 管道失败：{message}"))
            }
            IpcEvent::Property { name, data } => {
                self.apply_property(&name, &data);
                None
            }
            IpcEvent::Reply { request_id, error } => {
                if error == "success" {
                    self.last_error = None;
                    return None;
                }
                let text = format!("命令失败（请求 {request_id}）：{error}");
                self.last_error = Some(text.clone());
                Some(text)
            }
            IpcEvent::Closed => {
                self.connected = false;
                Some("mpv 已退出（管道断开）".to_string())
            }
        }
    }

    fn apply_property(&mut self, name: &str, data: &Value) {
        match name {
            "time-pos" => self.position = data.as_f64(),
            "duration" => self.duration = data.as_f64(),
            "pause" => self.paused = bool_of(data),
            "paused-for-cache" => self.buffering = bool_of(data),
            "eof-reached" => self.ended = bool_of(data),
            "idle-active" => self.idle = bool_of(data),
            // 订阅表以外的属性不认，但也别当错误：mpv 会推很多我们没订的东西。
            _ => {}
        }
    }

    /// 当前状态的一句话说明（不含进度）。缓冲优先于暂停显示：用户最想知道的是"它卡住了还是我按的"。
    pub fn mode_text(&self) -> &'static str {
        if self.buffering {
            "缓冲中"
        } else if self.ended {
            "已结束"
        } else if self.paused {
            "已暂停"
        } else if self.idle {
            "等待载入"
        } else {
            "播放中"
        }
    }

    /// 进度那一截（`已播 / 总长`）。四种组合各不相同：**只有位置时不硬造总时长**，
    /// 没进度一律 `--:--` 而不是 `00:00`（"还没开始"和"从 0 秒开始"是两件事）。
    pub fn progress_text(&self) -> String {
        match (self.position, self.duration) {
            (Some(position), Some(duration)) => {
                format!("{} / {}", seconds_text(position), seconds_text(duration))
            }
            (Some(position), None) => seconds_text(position),
            (None, Some(duration)) => format!("{MISSING_TIME} / {}", seconds_text(duration)),
            (None, None) => format!("{MISSING_TIME} / {MISSING_TIME}"),
        }
    }

    /// 界面那一行播放信息。**纯函数**，页面只负责把它塞进元素树（所以它能被单测覆盖）。
    ///
    /// 没连上遥控时显示 `disconnected_note`（可能是"还没起播"、"起播失败：…"、
    /// "连上 mpv 管道失败：…"、"mpv 已退出（管道断开）"）；连上之后显示 `进度 · 状态`，
    /// 有命令失败原因时附在最后——**失败要看得见**，不能只写进日志（第十八轮的教训）。
    pub fn line(&self, disconnected_note: &str) -> String {
        if !self.connected {
            return if disconnected_note.is_empty() {
                "还没起播。".to_string()
            } else {
                disconnected_note.to_string()
            };
        }

        match self.last_error.as_ref() {
            Some(error) => format!("{} · {} · {error}", self.progress_text(), self.mode_text()),
            None => format!("{} · {}", self.progress_text(), self.mode_text()),
        }
    }
}

/// 进度缺失时画面上写的东西。**不能编一个 `00:00` 出来**：流还没载入时 `time-pos` 就是 `null`，
/// 写 `00:00` 会被读成"已经播到了开头"，而"还没开始"和"从 0 秒开始"是两件事。
pub const MISSING_TIME: &str = "--:--";

/// 秒 → 中文读法。mpv 给的是浮点秒，先夹到 0（真出现负值时画面也不该写 `-00:03`），
/// 再交给 api 层那个共用的整秒格式化（历史/详情/播控都用它，见 `api/mod.rs`）。
fn seconds_text(seconds: f64) -> String {
    crate::api::format_seconds(crate::api::seconds_to_u32(seconds.max(0.0) as u64))
}

/// mpv 的属性值可能是 `null`（`observe_property` 给的是原生 JSON 值，布尔就是 `true`/`false`），
/// 拿不到就当 false——UI 不需要"未知"这个第三态。
fn bool_of(data: &Value) -> bool {
    data.as_bool().unwrap_or_default()
}
