# src — 项目源代码

Rust crate 根目录（crate 名 `rustbilipc`）。职责划分与维护契约见
[docs/architecture.md](../docs/architecture.md)。

## 当前结构

| 路径 | 职责 | 备注 |
| --- | --- | --- |
| `main.rs` | 启动序列 | **`logging::init()`** → tokio runtime → TLS provider → `BpiClient` → `with_assets` → `gpui_kit::init` → 主题 → `set_http_client` → 开窗，顺序有意义 |
| `lib.rs` | 模块树 + `VERSION` | 逻辑都收敛在 lib crate 里便于 `cargo test` |
| `logging.rs` | 文件日志 | **exe 同目录**的 `debug.log`（与 `data/cookies.json` 同一层），每次启动清空、级别 Debug、每行 flush，同时复制到 stderr；自实现 `log::Log`，零依赖 |
| `app.rs` | 进程级基础设施 | 全局 tokio runtime（`OnceLock`）、`install_tls_provider()`（rustls 双 provider 会 panic）、`describe_join_error()`（把任务 panic 变成可读文案）、`exe_dir()`/`data_dir()` |
| `theme.rs` | 全局主题覆盖 | accent `#FB7299`，强调色前景改深色（对比度不足 4.5:1） |
| `api/` | bpi-rs 封装 + DASH 选择 | `spawn_on_api_runtime` 是唯一的 gpui↔tokio 桥；`login.rs`（取码/轮询 + 码值翻译 + cookie→凭据 + **缺 buvid3 时向 `misc().buvid()` 补取**）、`search.rs` 出 DTO（`SearchHit`/`PageInfo`/`SearchSort`），bpi 类型不外泄；`stream_select` 是纯函数 |
| `store/` | 落盘 | `credentials.rs` 明文 JSON，见文件头的风险说明 |
| `player/` | 外置 mpv | `build_arguments` 纯函数 + Referer/UA 常量；`ipc.rs` 只做命令构造，管道 I/O 在 P4 |
| `pages/` | 按阶段划分的页面 | `root.rs` 是窗口根视图（无凭据→登录页，有→主界面，登录成功原地切换）；`login/` 扫码 + 1.5 s 轮询 + 过期自动重载；`search/`：输入 → 回车/按钮/排序/分页 → tokio 请求 → `upgrade()+update()` 回填 |
| `ui/` | 跨页面复用组件 | `video_card.rs`（封面用 `img(url)` + `ObjectFit::Cover`，时长角标绝对定位）、`qr.rs`（qrcode-generator 出 `Vec<Vec<bool>>`，一格一个 div 画出来） |
| `tests/` | 单元测试 | 只测纯逻辑：码流边界、mpv 参数形状、凭据往返、搜索标签清洗与播放量格式化、登录码值翻译 / cookie 完整性 / buvid3 补取判定、二维码矩阵 |

## 分层规则

- `pages/*` 不直接调接口，网络调用一律经 `api/`。唯一允许的例外是持有
  `Arc<bpi_rs::BpiClient>`（在 `main.rs` 构造后传入，只为 clone 给 `api/`），
  这样页面里不会出现任何 bpi 的请求/响应类型。
- `api/`、`store/`、`player/` 不得依赖 gpui 类型，必须能在无窗口环境下测试。
- gpui 类型统一从 `gpui_kit::*` 取，不单独依赖 gpui（kit 的 umbrella 已把它放在根上）。

## 约定

修改本目录任何代码时，同步更新本 README、`docs/architecture.md` 相应小节，并提交 git。
注释写"为什么"（机制、坑、定稿理由），不写"是什么"。

**异步链路的每一跳都要写日志**（`debug.log`，见 `logging.rs`）：api 层在提交前与 tokio 任务内
请求前后各一条（带耗时与返回码），视图层在 `await` 前后、`upgrade()` 失败、`update()` 闭包内各一条。
GUI 程序没有控制台，只有落盘的日志能区分"没发出 / 没回来 / 回来了但没写进界面"。

**每个 `cx.spawn` 返回的 `Task` 必须被持有或 `.detach()`**：gpui 的 Task 一被 drop 就立刻取消任务
（`gpui-pre-scheduler/src/executor.rs:396-402`，`Task` 还是 `#[must_use]`），写成裸 `cx.spawn(...);`
的后果是"请求发出去了、界面永远停在加载态"。`Cargo.toml` 已把 `unused_must_use` 设为 deny，
这类写法直接编译失败。不要用 `.cancel()`/丢弃 Task 来停旧循环（`Task::cancel` 会等任务停止，
从任务自己的 future 里等自己 = 主线程死锁），改用序号字段（登录页 `generation`、搜索页 `request_seq`）
让旧循环发现自己被取代后 `return`。见 `docs/architecture.md` 第 4.9 节。

**写 gpui/kit 调用前先确认该写法在已发布版本里存在**：`ref/gpui-kit/examples/` 用的是 kit 本地
main，比 crates.io 的 0.8.0 新。已踩过两条：没有 `v_flex()`/`h_flex()` 自由函数（用
`div().flex()` + `.flex_col()`）；`overflow_y_scroll()` 属于 `Stateful<Div>`，必须先 `.id("...")`。
详见 `docs/architecture.md` 第 2 节与第 8 节。
