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
| `api/` | bpi-rs 封装 + DASH 选择 | `spawn_on_api_runtime` 是唯一的 gpui↔tokio 桥，`format_seconds`/`seconds_to_u32` 放在 `mod.rs`（历史与详情共用）；`login.rs`（取码/轮询 + 码值翻译 + cookie→凭据 + **缺 buvid3 时向 `misc().buvid()` 补取**）、`profile.rs`（`space_info` + `login().stat()`）、`favorites.rs`（`created_list` + `collected_list`，DTO 带 `created` 标记）、`history.rs`（`history_list` 只要 Archive）、`detail.rs`（`view` 主体 + `desc`/`tags`/`related_videos`/字幕**四路各自容错**，原因见文件头）、`stream.rs`（`play_url` 取地址 → `StreamChoice`：DASH 双流或 durl 单文件、清晰度中文档、备用地址；页面不起播细节都从这里出）、`search.rs` 出 DTO（`SearchHit`/`PageInfo`/`SearchSort`），bpi 类型不外泄；`stream_select` 是纯函数 |
| `store/` | 落盘 | `credentials.rs` 明文 JSON，见文件头的风险说明；`cookie_header()` 把四条 cookie 拼成 mpv 要的那一行（1080P 直链校验登录态） |
| `player/` | 外置 mpv | `build_arguments` 纯函数（音频 URL 是 `Option`：durl 回退时**不能**给空 `--audio-file=`）+ Referer/UA 常量 + `launch()` 拉起进程（PATH 找不到就返回可读错误，不 panic）；`ipc.rs` 只做命令构造与管道命名，**管道 I/O 在 P4b** |
| `pages/` | 按阶段划分的页面 | `mod.rs` 里有三个页面共用的导航事件 `OpenVideo{bvid}`；`root.rs` 是窗口根视图（无凭据→登录页，有→主界面外壳，登录成功原地切换，事件带 mid）；`shell/` 顶部"搜索 / 我的" + **详情页栈**（打开 push、返回 pop），两页实体常驻不重刷，**切到"我的"才第一次发请求**；`login/` 扫码 + 1.5 s 轮询 + 过期自动重载；`mine/` 资料 + 收藏夹 + 历史**三路并发**、各写各的提示位；`detail/` 主体 + 选集 + **播放**（起播/清晰度，持 `Child` 并在 `Drop` 里 kill，见 `docs/architecture.md` 第 4.13 节）+ 字幕 + 相关推荐；`search/`：输入 → 回车/按钮/排序/分页 → tokio 请求 → `upgrade()+update()` 回填 |
| `ui/` | 跨页面复用组件 | `video_card.rs`（封面 `img(url)` + `ObjectFit::Cover` + `with_fallback` 兜底，时长角标绝对定位；`clickable(data, on_click)` 是要能点的那一层）、`cover.rs`（**显示前必过**：补 `//`→`https://` + 拼图床 `@Ww_Hh.jpg` 缩略图后缀）、`qr.rs`（qrcode-generator 出 `Vec<Vec<bool>>`，一格一个 div 画出来） |
| `tests/` | 单元测试 | 只测纯逻辑：码流边界、mpv 参数形状（含"单文件不加 `--audio-file`"）与管道名、凭据往返与 `cookie_header()` 只列有的字段、搜索标签清洗与播放量格式化、登录码值翻译 / cookie 完整性 / buvid3 补取判定、二维码矩阵、秒数格式化与会员文案、封面 URL 补协议与缩放、详情的 `favorite`/`fav` 双字段与字幕地址补协议、清晰度中文档的四级兜底顺序 |

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

**我们拉起的子进程必须有归宿**：`std::process::Child` 的 drop **不会**杀子进程，
所以持有它的实体要在自己的 `Drop` 里显式 `kill()`（`DetailPage` 就是这么做的），
再次起播时也先收掉上一个实例。否则就是"关掉详情页、视频还在放、没人能遥控它"。

**写 gpui/kit 调用前先确认该写法在已发布版本里存在**：`ref/gpui-kit/examples/` 用的是 kit 本地
main，比 crates.io 的 0.8.0 新。已踩过两条：没有 `v_flex()`/`h_flex()` 自由函数（用
`div().flex()` + `.flex_col()`）；`overflow_y_scroll()` 属于 `Stateful<Div>`，必须先 `.id("...")`。
详见 `docs/architecture.md` 第 2 节与第 8 节。
