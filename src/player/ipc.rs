//! mpv 命名管道 IPC 的协议层（纯数据部分）。
//!
//! ## Windows 上的形态
//!
//! mpv 在 Windows 用命名管道而非 unix socket：`--input-ipc-server=\\.\pipe\<name>`，
//! 协议是"每行一条 minified JSON、禁止值里出现换行"。多个客户端可以同时连
//! （mpv 用 `PIPE_UNLIMITED_INSTANCES` 建管道），所以查询和 `observe_property`
//! 可以各走一条连接。
//!
//! ## P4 才接的部分
//!
//! 真正的管道读写需要 overlapped I/O（mpv 文档明确要求），到播放阶段再接；
//! 现在只提供命令构造与管道命名，方便先写单测、先把参数形状定下来。
//!
//! 断连即代表 mpv 退出——我们用 `idle-active`/`eof-reached` 之外的"连接失败"来判断存活。

use serde_json::{json, Value};

/// 为本进程的某一次起播生成一个不与其他实例冲突的管道名。
///
/// 要带**序号**而不只是 pid：再次起播时我们先 kill 上一个 mpv，但那只是十几毫秒，
/// 上一个实例可能还持有同名管道句柄；mpv 用 `PIPE_UNLIMITED_INSTANCES` 建管道
/// （`ref/mpv/input/ipc-win.c:387`），同名不会报错，结果是两个 mpv 都在听——
/// 我们的命令会随机发给其中一个。
pub fn pipe_name_for(pid: u32, seq: u32) -> String {
    format!(r"\\.\pipe\rustbilipc-{pid}-{seq}")
}

/// 构造一条 mpv IPC 命令：`{"command": [...], "request_id": n}`。
///
/// `async: true` 不加，因为我们用 `request_id` 匹配回包，够用且更好读。
pub fn command(args: &[&str], request_id: u64) -> Value {
    json!({
        "command": args,
        "request_id": request_id,
    })
}

/// 订阅属性变化：`observe_property <id> <name>`。
///
/// 之后 mpv 会推 `property-change` 事件；连接一断开，订阅即失效（mpv 文档明确警告）。
pub fn observe_property(name: &str, observe_id: u64, request_id: u64) -> Value {
    json!({
        "command": ["observe_property", observe_id, name],
        "request_id": request_id,
    })
}

/// 设置属性，例如音量与暂停：`set_property <name> <value>`。
pub fn set_property(name: &str, value: Value, request_id: u64) -> Value {
    json!({
        "command": ["set_property", name, value],
        "request_id": request_id,
    })
}

/// 播放控制类页面的常用订阅集合，集中一处便于以后增减。
pub const OBSERVED_PROPERTIES: &[&str] = &[
    "time-pos",
    "duration",
    "pause",
    "volume",
    "eof-reached",
    "idle-active",
    "paused-for-cache",
];
