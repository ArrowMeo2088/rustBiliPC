//! mpv 命名管道 IPC：命令构造（纯数据）+ 连接读写（tokio）。
//!
//! ## Windows 上的形态与坑（都有源码/文档出处）
//!
//! - mpv 用 `--input-ipc-server=\\.\pipe\<name>` 建**命名管道**，服务端句柄是
//!   `PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED` + `PIPE_TYPE_MESSAGE | PIPE_READMODE_BYTE`，
//!   并且 `PIPE_UNLIMITED_INSTANCES`、`PIPE_REJECT_REMOTE_CLIENTS`
//!   （`ref/mpv/input/ipc-win.c:352-359`）。多条连接同时存在是合法的。
//! - 协议是"一行一条 minified JSON、值里不得出现 `\n`"（`DOCS/man/ipc.rst:95-99`）。
//!   我们只在行尾加一个 `\n`，其余交给 `serde_json` 转义。
//! - **客户端必须用 overlapped I/O 才能同时读写**（`DOCS/man/ipc.rst:69-73`）。
//!   std 的同步管道句柄做不到这一点（同一个句柄上阻塞读会把写一起挡住），所以这里用
//!   `tokio::net::windows::named_pipe`：它的 `try_read`/`try_write`/`ready` 都取 `&self`，
//!   同一个任务里读写可以交错进行，overlapped 的细节在 tokio 内部处理。
//! - `observe_property` 的注册是**按连接**算的：连接一断注册即失效
//!   （`DOCS/man/ipc.rst:280-289` 的 warning，还专门提醒"用两次 socat 分别收发"会看起来不工作）。
//!   所以订阅和后续命令都走这同一条连接，且这条连接必须一直开着。
//! - 回包形如 `{"error":"success","data":…,"request_id":n}`；不写 `request_id` 时它固定是 0
//!   （`DOCS/man/ipc.rst:120-139`）。我们把回包也变成事件交给界面，非 `success` 的会显示出来。
//! - `loadfile` 的回复**早于**真正切完文件：文档明说"替换当前文件的 loadfile 会在当前文件
//!   还没停、新文件还没开始加载时就返回"（`DOCS/man/input.rst:542-548`），
//!   所以不能用它的回复判断"流打没打开"，那要靠 `idle-active`/`eof-reached` 这类属性。
//!
//! ## 只有 Windows
//!
//! 本项目按定稿只编 Windows x64，`tokio::net::windows` 也只在那儿存在，所以这里不加
//! `#[cfg(windows)]`；真要移植再说，别为此留一条假的兼容路径。

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};
use tokio::io::Interest;
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use tokio::sync::mpsc;

/// 为本进程的某一次起播生成管道名。
///
/// 要带**序号**而不只是 pid：mpv 服务端用 `PIPE_UNLIMITED_INSTANCES` 建管道
/// （`ref/mpv/input/ipc-win.c:387`），同名**不会报错**，于是"刚被 kill 的上一个实例"和
/// 新实例会同时听同一条管道，我们的命令就可能发给错的窗口。
pub fn pipe_name_for(pid: u32, seq: u32) -> String {
    format!(r"\\.\pipe\rustbilipc-{pid}-{seq}")
}

/// 构造一条 mpv IPC 命令：`{"command": [...], "request_id": n}`。
///
/// `async: true` 不加：我们用 `request_id` 匹配回包，够用且更好读。
pub fn command(args: &[Value], request_id: u64) -> Value {
    json!({
        "command": args,
        "request_id": request_id,
    })
}

/// 订阅属性变化：`observe_property <id> <name>`。
///
/// 之后 mpv 会推 `property-change` 事件；连接一断开，订阅即失效（mpv 文档明确警告）。
pub fn observe_property(name: &str, observe_id: u64, request_id: u64) -> Value {
    command(&[json!("observe_property"), json!(observe_id), json!(name)], request_id)
}

/// 设置属性，例如音量与暂停：`set_property <name> <value>`。
pub fn set_property(name: &str, value: Value, request_id: u64) -> Value {
    command(&[json!("set_property"), json!(name), value], request_id)
}

/// 相对跳转：`seek <秒> relative`。
///
/// `seek` 的默认模式本来就是 relative（`DOCS/man/input.rst:300-308`），
/// 这里写明是为了让读代码的人不必去查默认值。
pub fn seek_relative(seconds: f64, request_id: u64) -> Value {
    command(&[json!("seek"), json!(seconds), json!("relative")], request_id)
}

/// 让 mpv 自己退出：`quit`。
///
/// 走这条路是为了让 OSC 与 mpv 自己收尾（关窗、写 watch-later 之类），
/// 比 `Child::kill` 干净；我们仍在 `Drop` 里留 `kill` 兜底。
pub fn quit(request_id: u64) -> Value {
    command(&[json!("quit")], request_id)
}

/// 界面要盯的属性，集中一处便于增减。
///
/// 每个名字都对应 mpv 的属性表（`ref/mpv/player/command.c:4708-4743`：
/// `time-pos`、`eof-reached`、`paused-for-cache`、`idle-active` 都在）。
/// 没列 `volume`：音量交给 mpv 自己的 OSC 与快捷键，我们不做。
pub const OBSERVED_PROPERTIES: &[&str] = &[
    "time-pos",
    "duration",
    "pause",
    "eof-reached",
    "idle-active",
    "paused-for-cache",
];

/// 为每个要盯的属性生成一条订阅命令，`observe_property` 的 id 用下标 + 1。
///
/// id 在**这条连接内**唯一即可（不同连接各自独立）；这里从 1 开始并连号，方便日志对照。
pub fn observe_batch(properties: &[&str]) -> Vec<Value> {
    properties
        .iter()
        // `copied()` 把元素从 `&&str` 收成 `&str`，下面的 `(index, name)` 解构才是
        // `(usize, &str)`；少了这一步就会拿 `&&str` 去填 `&str` 的位（第二十轮就是这么报的）。
        .copied()
        .enumerate()
        .map(|(index, name)| {
            let id = index as u64 + 1;
            observe_property(name, id, id)
        })
        .collect()
}

/// 把一条命令压成 mpv 要的那一行（minified JSON，不带换行）。
pub fn to_line(value: &Value) -> String {
    value.to_string()
}

/// 管道那一侧发生的事，全部已经翻译成界面能直接用的形状。
#[derive(Clone, Debug, PartialEq)]
pub enum IpcEvent {
    /// 连上管道并发出订阅。
    Connected,
    /// 连不上（mpv 起太慢或已经退了）。
    ConnectFailed(String),
    /// 订阅的属性变了。`data` 原样留着：数字、布尔、`null` 都可能出现。
    Property { name: String, data: Value },
    /// 一条命令的回包。`error` 正常是 `"success"`。
    Reply { request_id: u64, error: String },
    /// 管道断了：多数就是 mpv 退出了。
    Closed,
}

/// 解析 mpv 发来的一行。不是我们认识的东西就返回 `None`（不报错，日志里也懒得记）。
pub fn parse_line(raw: &str) -> Option<IpcEvent> {
    let value: Value = serde_json::from_str(raw).ok()?;
    match value.get("event").and_then(Value::as_str) {
        Some("property-change") => {
            let name = value.get("name").and_then(Value::as_str)?.to_string();
            let data = value.get("data").cloned().unwrap_or(Value::Null);
            Some(IpcEvent::Property { name, data })
        }
        // 其它事件（start-file / file-loaded / log-message…）这一轮不消费：
        // 进度与状态一律走 property-change，它带名字与值，够界面用。
        Some(_) => None,
        // 命令回包认 `error` 字段；没有就当作不认识。
        None => {
            let error = value.get("error").and_then(Value::as_str)?.to_string();
            let request_id = value
                .get("request_id")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            Some(IpcEvent::Reply { request_id, error })
        }
    }
}

/// 界面用来收 mpv 事件的那一半（`recv()` 可以在 gpui 的任务里 await）。
pub type EventStream = mpsc::UnboundedReceiver<IpcEvent>;

/// 一条已发起连接的 mpv 遥控通道。
///
/// `commands` 是发命令的入口（非阻塞），`events` 只能被界面**取走一次**
/// （`take_events`），因为接收端要 move 进那条常驻的 watch 任务里。
pub struct PlayerLink {
    commands: mpsc::UnboundedSender<String>,
    events: Option<EventStream>,
    /// 这条实例的管道名，写进日志便于对照（一个客户端进程会有多次起播）。
    pub pipe_name: String,
    next_request_id: AtomicU64,
}

impl PlayerLink {
    /// 取出事件接收端（只能成功一次）。
    pub fn take_events(&mut self) -> Option<EventStream> {
        self.events.take()
    }

    fn send(&self, value: &Value) -> bool {
        // 发送失败只有一种可能：管道任务已经结束了（`events` 里会同时给出 Closed）。
        self.commands.send(to_line(value)).is_ok()
    }

    /// 暂停 / 继续。
    pub fn set_pause(&self, paused: bool) -> bool {
        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        self.send(&set_property("pause", json!(paused), id))
    }

    /// 相对跳转，单位秒（负数倒退）。
    pub fn seek_by(&self, seconds: f64) -> bool {
        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        self.send(&seek_relative(seconds, id))
    }

    /// 请 mpv 自己退出。
    pub fn quit(&self) -> bool {
        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        self.send(&quit(id))
    }
}

/// 打开这条管道的遥控通道并把读写任务投递到 API runtime 上。
///
/// 调用点应该在 mpv **已经拉起**之后：mpv 建管道是启动早期的一步，但仍然晚于我们的
/// `spawn()` 返回，所以 [`connect`] 里带重试。
pub fn open(pipe_name: String) -> PlayerLink {
    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let link = PlayerLink {
        commands: command_tx,
        events: Some(event_rx),
        pipe_name: pipe_name.clone(),
        next_request_id: AtomicU64::new(1000),
    };
    // 任务要活到我们主动结束为止：`JoinHandle` 是 `#[must_use]`，
    // 绑定它只是让 lint 闭嘴，**不 join**（join = 等 mpv 退出，会把调用方挂住）。
    let _join = crate::app::runtime().spawn(pump(pipe_name, command_rx, event_tx));
    link
}

/// 建连重试的上限：mpv 从 spawn 到建好管道通常几十毫秒，40 次 ×120 ms 足够宽；
/// 真超了就说明 mpv 没起来，界面要显示原因。
const CONNECT_ATTEMPTS: u32 = 40;
const CONNECT_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(120);

/// 连到 mpv 的管道。
///
/// `ClientOptions::open` 底层是 `CreateFileW`：服务端还没建好实例时它**立即返回错误**
/// （不会阻塞），所以这里用短睡重试，而不是把它当成致命失败。
async fn connect(path: &str) -> Result<NamedPipeClient, String> {
    let mut last = String::from("没有试过");
    // 绑一份给格式化用（`format!` 的内联参数走变量捕获，别拿常量的名字去赌）。
    let attempts = CONNECT_ATTEMPTS;
    for _attempt in 0..attempts {
        match ClientOptions::new().read(true).write(true).open(path) {
            Ok(client) => return Ok(client),
            Err(error) => last = error.to_string(),
        }
        tokio::time::sleep(CONNECT_RETRY_DELAY).await;
    }
    Err(format!(
        "试了 {attempts} 次仍连不上 {path}（最后一次：{last}）"
    ))
}

/// 管道任务的主体：一个任务同时负责读与写（靠 `&self` 的 try_* 接口，不需要拆分句柄）。
async fn pump(
    path: String,
    mut commands: mpsc::UnboundedReceiver<String>,
    events: mpsc::UnboundedSender<IpcEvent>,
) {
    let client = match connect(&path).await {
        Ok(client) => client,
        Err(message) => {
            log::warn!("IPC：{message}");
            let _sent = events.send(IpcEvent::ConnectFailed(message));
            return;
        }
    };
    log::info!("IPC：连上 {path}");

    // 待写字节与未凑满一行的已读字节。写不出去时 pending 会攒着，下一轮再试。
    let mut pending: Vec<u8> = Vec::new();
    let mut incoming: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];

    for value in observe_batch(OBSERVED_PROPERTIES) {
        push_line(&mut pending, &to_line(&value));
    }
    let _sent = events.send(IpcEvent::Connected);

    loop {
        // 1) 先把待发命令写干净（一次写不完就留着，下轮接着写）。
        while !pending.is_empty() {
            match client.try_write(&pending) {
                // 写进去多少就丢掉多少。这里用 `split_off` 再赋值而不是 `drain(..n)`：
                // `drain` 返回的迭代器不消费会撞上本项目的 `unused_must_use = "deny"`
                // （第 4.9 节那条规矩的副作用），而 std 的 `Vec` **没有** `replace_range`
                // （那是 `bstr::ByteVec` 的方法，第二十一轮就是这么报的 E0599）。
                // 只有"一次没写完"这种少见的情况会多一次分配。
                Ok(written) if written > 0 => pending = pending.split_off(written),
                // **一次都没写进去就退出这一轮**，去等下一次可写。少了这个分支就是在这条
                // runtime 线程上死转：`pending` 永远非空、`written` 永远是 0。
                Ok(_zero) => break,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    log::debug!("IPC：写管道失败：{error}");
                    let _sent = events.send(IpcEvent::Closed);
                    return;
                }
            }
        }

        // 2) 把可读的字节吸干。读到 0 字节就是 EOF（mpv 退了或连接被关）。
        loop {
            match client.try_read(&mut chunk) {
                Ok(0) => {
                    log::info!("IPC：管道已关闭（mpv 退出了？）");
                    let _sent = events.send(IpcEvent::Closed);
                    return;
                }
                Ok(read) => incoming.extend_from_slice(&chunk[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    log::debug!("IPC：读管道失败：{error}");
                    let _sent = events.send(IpcEvent::Closed);
                    return;
                }
            }
        }

        // 3) 按 `\n` 切行交给界面；`send` 失败说明页面已经不要这些事件了。
        while let Some(index) = incoming.iter().position(|byte| *byte == b'\n') {
            let raw: Vec<u8> = incoming.drain(..=index).collect();
            let text = String::from_utf8_lossy(&raw);
            if let Some(event) = parse_line(text.trim_end()) {
                if events.send(event).is_err() {
                    log::debug!("IPC：界面已释放接收端，结束管道任务");
                    return;
                }
            }
        }

        // 4) 等下一步：可读（+ 有待发时可写），或者界面塞了新命令。
        //    没待发时故意只等 READABLE，免得被"一直可写"唤醒空转。
        let interest = if pending.is_empty() {
            Interest::READABLE
        } else {
            Interest::READABLE | Interest::WRITABLE
        };
        tokio::select! {
            command = commands.recv() => match command {
                Some(line) => push_line(&mut pending, &line),
                // 界面放下了发送端 = 这个详情页/这次起播结束了，任务没有存在意义。
                None => {
                    log::debug!("IPC：发送端已关闭，结束管道任务");
                    return;
                }
            },
            ready = client.ready(interest) => {
                if let Err(error) = ready {
                    log::debug!("IPC：等待管道就绪失败：{error}");
                    let _sent = events.send(IpcEvent::Closed);
                    return;
                }
            }
        }
    }
}

/// 往待发缓冲里追加一行（mpv 要求每条消息以 `\n` 结束，且行内不得出现 `\n`）。
fn push_line(pending: &mut Vec<u8>, line: &str) {
    pending.extend_from_slice(line.as_bytes());
    pending.push(b'\n');
}
