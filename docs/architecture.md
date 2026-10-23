# rustBiliPC 架构

本文是维护契约：改代码必须同步改这里，违反就把这条当 bug 修。
接口层面的细节见 [api-notes.md](api-notes.md)。

## 1. 目标与硬约束

Bilibili 桌面客户端（Windows x64），功能顺序 **搜索 → 个人 → 详情 → 播放**。

硬约束（不可协商，历史上的备选都已被否决）：

- **零浏览器技术**：不使用 Electron / WebView / JS / TS。gpui 走原生 D3D11 + DirectComposition + DirectWrite。
- **本机不装 Rust 工具链**：只有 GitHub Actions 编译，验证以 Actions 日志为准。
- **访问 GitHub 走 `http://127.0.0.1:7887` 代理**。
- **视频播放交给外置 mpv 独立进程 + 独立窗口**，不做窗口嵌入、不自研解码渲染。

## 2. 选型与版本锁

| 项 | 选择 | 理由 |
| --- | --- | --- |
| UI 框架 | `gpui-kit = "0.8.0"`（umbrella，`default = ["component","assets"]`） | 60+ 组件、Lucide 图标、JSON 主题令牌；实测默认 features **不含 webview** |
| gpui 实际版本 | `gpui-pre = "=0.3.9"`（Zed@93d7456 的重发布快照），由 kit 携带精确 pin | 0.2.2 与 0.3.x 的 `App/Window/Entity` 类型互不兼容，不能混 |
| 放弃 | `ref/gpui-component`（0.2.2）与 Ely 作依赖 | 分别因为：拉 tree-sitter + `lb-wry`（WebView2）且与 0.3.9 类型不兼容；`publish=false` 只能 git 依赖 + Rust 1.95 |
| API | `bpi-rs = "0.3.1"`，`default-features = false` + 6 个 feature | 关掉 `full` 的 27 个模块，只开 login/search/video/user/fav/historytoview |
| 异步 | 自建 tokio runtime（`src/app.rs`） | gpui 执行器不是 tokio，reqwest 需要 reactor |
| 编译配置 | **release**，`lto = "off"`、`strip = false`、`debug = true` | 见下节 |
| 依赖锁定 | **提交 `Cargo.lock`** 作为版本种子，CI **不加** `--locked` | 挡住 gpui-pre 每周快照的破坏性变更；本机无 cargo，加 `--locked` 会让任何新增依赖直接卡死 CI |

### 为什么必须 release

`gpui-pre-windows 0.3.9` 的 `directx_renderer.rs` 在 `cfg(debug_assertions)` 分支里用
`D3DCompileFromFile(env!("CARGO_MANIFEST_DIR")/src/shaders.hlsl)` **在运行时**编译 HLSL。
这个路径在编译期被烧进二进制，CI 产物拿到别的机器上必然找不到该文件，窗口起不来。
release 分支（`build.rs` 里 `#[cfg(not(debug_assertions))] compile_shaders()`）用 `fxc.exe`
在编译期把字节码 `include_bytes!` 内嵌，产物才可移植。

`gpui_platform` 的 `runtime_shaders` 特性只映射到 `gpui_macos`，**Windows 上没有开关**。

### 参考仓库 ≠ 我们在用的版本（写代码前必读）

`ref/gpui-kit` 是 kit 的**本地 main**，比 crates.io 的 `gpui-kit 0.8.0` 新，它的 `examples/` 里
有些写法在已发布版本里不存在。已踩到的两条（首轮 CI 报错，均以日志为证）：

- `v_flex()` / `h_flex()` 自由函数**不可用** → 用 `div().flex().flex_col()`（行方向就是 `div().flex()`）。
- `overflow_y_scroll()` 是 `Stateful<Div>` 的方法 → 必须先 `.id("...")`，否则报 E0599
  并提示一个不相干的 `overflow_y_scrollbar`。
- `Button::primary()` / `.secondary()` 等定义在 **`ButtonVariants` trait** 上
  （`gpui-component 0.8.0` 的 `src/button/button.rs:52`）→ 必须
  `use gpui_kit::component::button::{Button, ButtonVariants};`，只导入 `Button` 会报 no method。
- `InputEvent::PressEnter` 是**带字段的 struct 变体**（`{ secondary, shift }`）→ 模式要写
  `InputEvent::PressEnter { .. }`，裸写报 E0533。
- 反过来，`Styled` / `StyledImage`（`.object_fit()`）确实由 `gpui_kit::*` 的 glob 提供，
  **不要**再为它们加显式导入：第三轮 CI 里 `video_card.rs` 无额外导入就编译通过了。
- `rustls::crypto::aws_lc_rs::default_provider().install_default()` 的 `Err` 变体是
  **已经装上的那个 `CryptoProvider`**，它不实现 `Display` → 不能写成 `"{error}"`（第六轮 CI 实测）。

规则：拿不准的调用，先按下面的顺序核对，再动手（一轮冷 CI 约 6–14 分钟，猜错就要重跑）：

1. `ref/gpui/<crate>/src` —— 我们真正依赖的 gpui 0.3.9 发布源码（渲染、元素、`Styled`/`StyledImage`、
   `ImageAssetLoader`、`.hlsl` 都在这里）。
2. `ref/gpui-kit/crates/{base,component}/src` —— 组件层真实用法（**看 src，不看 examples**）。
3. 需要更精确时直接下发布包核对：`curl -L https://static.crates.io/crates/<name>/<name>-<ver>.crate`
   （已用过这招确认 `ButtonVariants` trait 与 `gpui-pre-windows` 的着色器分支）。
4. 都拿不到就等 CI 日志里 rustc 的建议。

## 3. 模块布局

```
src/
├── main.rs            启动序列：logger → runtime → with_assets → init → theme → set_http_client → 开窗
├── lib.rs             模块树 + VERSION
├── app.rs             tokio runtime（OnceLock）、install_tls_provider()、describe_join_error()、exe_dir()/data_dir()
├── logging.rs         debug.log 文件日志（写 exe 同目录，每次启动清空、Debug 级、零依赖、同时写 stderr）
├── theme.rs           全局主题覆盖：accent = #FB7299，accent_foreground = #1F1F1F
├── api/
│   ├── mod.rs         spawn_on_api_runtime()：gpui ↔ tokio 的调用桥
│   ├── login.rs       扫码登录：取码/轮询 + 码值翻译 + cookie→凭据 + 缺 buvid3 时补取（纯函数可测）
│   ├── profile.rs     个人中心头部：user().space_info()（自动 WBI）+ login().stat()（关注/粉丝）
│   ├── favorites.rs   收藏夹：fav().created_list() + collected_list()，合并成一个 DTO
│   ├── history.rs     观看历史：historytoview().history_list()（游标分页）+ 秒数格式化纯函数
│   ├── search.rs      搜索封装：DTO（SearchHit/PageInfo/SearchSort）+ 标签清洗 + 播放量格式化
│   └── stream_select.rs  DASH 选择纯函数（HEVC 优先、画质档封顶 116）
├── store/
│   └── credentials.rs 明文 JSON 凭据落盘（<exe>/data/cookies.json）
├── player/
│   ├── mod.rs         mpv 参数构造 + Referer/UA 常量 + PATH 查找
│   └── ipc.rs         mpv JSON IPC 的命令构造（纯数据，管道 I/O 在 P4）
├── pages/
│   ├── root.rs        窗口根视图：无凭据→登录页，有凭据→主界面外壳；登录成功后原地切换
│   ├── shell/         主界面外壳：顶部"搜索 / 我的"两个入口，两页实体都常驻（切回不重刷）
│   ├── login/         扫码登录页（二维码 + 1.5s 轮询 + 过期自动重载）
│   ├── mine/          个人中心页（资料 + 收藏夹 + 历史，三路并发各自显示错误）
│   └── search/        阶段一页面（view.rs 为根视图）
├── ui/
│   ├── qr.rs          二维码：qrcode-generator 出布尔矩阵，div 网格绘制
│   └── video_card.rs  卡片视图（封面走 gpui URI 图片加载 + 时长角标）
└── tests/             单元测试集中地（纯逻辑）
```

分层规则（越界要在评审时拦住）：

- `pages/*` 只做装配与渲染，网络调用一律经 `api/`，不直接 `use bpi_rs`。
- `api/`、`store/`、`player/` 是**无 UI 依赖**的层，必须能在 `cargo test` 里独立跑。
- `ui/` 只放跨页面复用组件；页面私有的视图留在该页面目录内。
- gpui 的类型统一从 `gpui_kit::*` 取，不单独 `use gpui::...`（kit 已把 gpui 放在根上）。

## 4. 关键机制

### 4.1 异步调用桥

`cx.spawn(client.search().video(..))` 会 panic（no reactor running）。规定写法（与
`ref/gpui-kit/crates/component/src/carousel/state.rs:1004` 同款）：

```rust
let join = crate::api::search::search_videos(Arc::clone(&self.client), keyword, page, sort);
cx.spawn(async move |this, cx| {
    let outcome = join.await;                       // await JoinHandle 不需要 reactor
    if let Some(this) = this.upgrade() {
        let _ = this.update(cx, |this, cx| {
            /* 写回状态 */
            cx.notify();
        });
    }
})
// 末尾的 detach() 或 `let _task = …` 二选一，缺了任务会被立即取消（第 4.9 节）。
.detach();
```

`cx.spawn` 交给闭包的是 `&mut AsyncApp`（`gpui/src/app/context.rs:230`），不是
`AsyncWindowContext`；两者都有 `background_executor()`，但要写窗口相关操作时得改用
`Context::spawn_in(window, …)`。gpui 的前台任务不要求 future 是 `Send`（`executor.rs:66-72` 走
`spawn_local`），所以可以直接 await tokio 的 `JoinHandle`；唤醒来自任何线程都会被调度回主线程。

### 4.2 码流选择

`bpi-rs` 视频链路的 `DashInfo` **没有** `best_video()`（那助手挂在番剧/课程的
`models::stream::VideoStreamData` 上，且第一关键字是分辨率，有大会员会直接选 4K）。
`src/api/stream_select.rs` 自实现的排序键是 `(画质档 id, 编码优先级 HEVC>AV1>AVC, 带宽)`，
默认只接受 `id <= 116`（1080P60），排除 120/125/126/127（4K/HDR/DV/8K）——目标机 i5-6200U
硬解不到 4K 与 10bit HEVC。封顶范围内无可用流时退回全集，宁可不降级也要能播。

### 4.3 播放：外置 mpv

- 音视频是两条 DASH URL：视频当主文件，音频走 `--audio-file=`（可接受远程 URL）。
- `--http-header-fields` / `--user-agent` / `--referrer` 是**实例级全局**（`mp_network_conf`），
  对同实例的所有 HTTP 流生效，所以 URL 必须与这套头成对出现，否则 403。
- 控制走命名管道 `\\.\pipe\rustbilipc-<pid>`，JSON 行协议 + `observe_property`；
  连接断开即视为 mpv 已退出。
- `--no-config` 隔离用户自己的 `mpv.conf`，避免别人的 `vo`/`hwdec` 设置干扰。
- mpv 不在 PATH 是常态：底部状态条从 P0 起就显示查找结果，登录/播放阶段要给明确提示。

### 4.4 凭据

`<exe 同目录>/data/cookies.json`，明文（定稿选择，风险已在 `store/credentials.rs` 顶部写明）。
`bpi_rs::session::Account` 只 derive `Deserialize`，所以用 `SavedCredential` 做可序列化镜像，
读回时构造 `Account` 交给 `BpiClient::set_account`。四字段（DedeUserID/SESSDATA/bili_jct/buvid3）
必须齐全，缺 `buvid3` 会被判定不完整。

### 4.5 网络封面图（不自己写下载器）

gpui 0.3.9 的 `ImageAssetLoader`（`src/elements/img.rs:619-656`）对 `Resource::Uri` 会走
`App::http_client()` 抓字节、解码、缓存，并在 `img(source)` 的 `From<&str/String/SharedString>`
里自动判定"是 URI 还是嵌入式资源"（`is_uri` 用 `url::Url::parse`）。所以：

- 展示封面 = `img(url).size_full().object_fit(ObjectFit::Cover)`，下载/去重/缓存都不用我们写。
- **前提**：gpui 默认的 HttpClient 是 `NullHttpClient`（`src/app.rs:183` 直接构造），
  不装真客户端时所有网络图片静默失败。启动时必须
  `cx.set_http_client(Arc::new(reqwest_client::ReqwestClient::user_agent("rustbilipc")?))`，
  依赖用 `gpui-pre-reqwest-client =0.3.9`（与 kit 同一快照，避免多份 gpui）。
- 封面 CDN（`*.hdslb.com`）是公开域，不需要 Referer；这点与**播放用的 m4s 直链不同**，后者必须带头。
- `cover_url` 为空时画占位块而不是提交空 URI，避免必然失败的请求刷日志。

### 4.6 扫码登录与启动判定（定稿：登录先行，不做免登录搜索）

- **启动判定**在 `pages/root.rs`：`SavedCredential::load()` 且四字段完整 → 直接把凭据
  `set_account` 进共享的 `BpiClient` 并进主界面；否则进登录页。登录成功后由登录页发
  `LoginSucceeded` 事件，根视图**在同一窗口内**换成搜索页。
  之所以不开第二个窗口：`AsyncWindowContext::open_window` 可用，但"关掉登录窗口"要先拿到窗口
  句柄再走 `Window::remove_window`，其中取句柄这一步在参考仓库里没有可照抄的先例，风险不值得。
- **二维码不用图片管线**：`qrcode-generator`（只开 `std`+`qr` feature）出 `Vec<Vec<bool>>`，
  一格一个 div 铺出来。走 `RenderImage` 要构造 Zed 自己那个 `image` 分支的 `Frame`
  （`gpui/src/svg_renderer.rs:5`），而且会把 pixels 版 `image` 拖进同一棵依赖树。
- **轮询节奏与状态机**在 `pages/login/view.rs`：每 1.5 秒一次；`86101` 继续等、`86038` 过期自动
  重新取码并**退出旧循环**（避免两条轮询并存）、`code==0` 组装凭据；网络错误只记日志、下一轮重试。
- 码值翻译与 cookie→凭据是纯函数（`api/login.rs` 的 `QrPollState::from_code` /
  `credential_from_cookies`），单测覆盖——真机登录时才碰得到的分支，先在 CI 里锁住。

### 4.7 异步任务的失败必须可见（真机教训）

真机上出现过"界面一直停在正在获取二维码/搜索中"，但没有错误文案。定位路径：
bpi-rs 的 builder 默认 `no_proxy = true`、`timeout = 10s`、`connect_timeout = 10s`
（`src/client.rs:113-115`），**网络再差也会在 10 秒内回一个 `BpiError`**。既然界面停在加载态，
说明结果根本没回到 gpui 侧 —— 唯一会这样的是 tokio 任务 **panic**：`join.await` 返回
`Err(JoinError)`，而我们的分支只写了 `log`。

规则：`cx.spawn` 里的 `JoinHandle` 错误**必须同时写日志和上界面**。`src/app.rs::describe_join_error`
用 `is_panic()` + `try_into_panic()` + downcast 把 panic payload 取出来，因为 `JoinError` 的
`Display` 不含现场信息。

已定位的最可能 panic 源：这张依赖图里 `rustls 0.23` 同时启用了 `aws-lc-rs` 与 `ring`
（首轮 Cargo.lock 里两者都在），未装进程级默认 crypto provider 时 TLS 握手会抛
"Could not automatically determine a process-level default crypto provider"。
修法是启动时 `app::install_tls_provider()` 显式装 `aws-lc-rs`（它已随 CI 的 NASM 编好）；
`install_default` 只能装一次，重复装返回 Err 时降级为 warn。

**第七轮之后的新事实**：装好 provider 后请求确实发出去了（用户 Wireshark 抓到包），但界面仍停在
加载态且没有错误文案 —— 说明"结果回到 UI"这一跳丢了。为把它拆开看，加了 `debug.log`（第 4.8 节）
与登录页心跳；同时排除了另一类猜测：bpi-rs 内部没有 `block_on`、也没有跨 await 持有的 `std Mutex`，
不会出现"后台任务自己卡死"。
（这一节原先还写过"`gpui::Task` 的 `Drop` 不取消任务"——**那句是错的**，我的 grep 漏了文件，
真正的结论见第 4.9 节。）

### 4.8 日志：debug.log 是唯一可依赖的现场

GUI 程序双击启动时没有控制台，`stderr` 一律看不见，所以 `src/logging.rs` 自实现 `log::Log`：

- 路径 **exe 同目录**的 `debug.log`（与 `data/cookies.json` 同一层，用户不用找系统目录），
  **每次启动 `File::create` 清空重写**，级别 `Debug`。因为它落在构建输出目录 `publish/` 里，
  取产物时**只搬 exe、绝不整目录清空**（`gh run download -D publish` 会因同名文件失败，
  正确做法是解到临时目录再 copy——上一轮我们因此差点连现场一起删掉）。
- 每行 `HH:MM:SS.mmm LEVEL target: message`，写完立即 `flush`（崩溃也不丢最后几行）；
- 同时把同一行复制到 stderr，便于从命令行启动时直接看；
- 零新依赖（原来的 `env_logger` 已移除）。

约定：**异步链路的每一跳都要有日志**，否则无法区分"没发出/没回来/回来了但没写进界面"：
api 层在提交前、tokio 任务内请求前后（带 `Instant` 耗时与返回码）各一条；视图层在 `await` 前后、
`upgrade()` 失败、`update()` 闭包内部各一条。界面上会显示日志文件路径。

诊断脚手架用完就删：登录页原先挂着一个"每 2 秒一行、共 5 行"的心跳任务，只为回答
"前台队列到底会不会被后台事件唤醒"这一个问题（第 4.9 节）。根因确认并已修好后它不再说明任何东西，
第十二轮删除，改由轮询循环自身的"轮询结果已回到前台（代 N）"作常驻证据——
**心跳不是登录的驱动机制**，扫码在心跳跑完之后照样能登录成功，这一点真机已验证。

### 4.9 "请求发出去了但界面不动"的真实根因：Task 被丢弃即取消

`debug.log` 的现场是：tokio 侧 `qr_generate` 190ms 正常返回，而视图层 `cx.spawn` 闭包的**第一行日志
从未出现**，心跳也一行没有 ⇒ 那段 future 从没被推进过。原因在我们自己的写法。gpui 执行器的
`Task` 文档写得很明白（`gpui-pre-scheduler/src/executor.rs:396-402`）：

> Task is a primitive that allows work to happen in the background. …
> **If you drop a task it will be cancelled immediately.** Calling `Task::detach` allows the task
> to continue running, but with no way to return a value.
> `#[must_use] pub struct Task<T>`

我们每处都写成裸 `cx.spawn(...);`，返回的 `Task` 当场 drop ⇒ 任务立即取消。请求仍然出现在 Wireshark
里，是因为 `fetch_qr(...)` 在 `cx.spawn` **之前**就把 tokio 任务发出去了；丢掉的只是"等结果并写回
界面"那一段。rustls 双 provider 那条修复是必要的（不装连请求都发不出去），但它不是这个卡死的原因。

**修复与防复发**：

- 每个 `cx.spawn` 必须要么 `let _task = ...` 存进实体字段（实体析构即取消，适合长驻循环），
  要么末尾 `.detach()`（适合一次性任务）。
- **不要从任务自己的 future 里丢弃它的 `Task`**：`Task::cancel` 会等任务停止，自己等自己 = 主线程
  死锁。所以登录流程用 `generation: u32` 序号，旧循环发现自己已被取代就自行 `return`；搜索页用
  `request_seq` 同样处理（顺带解决"翻页时旧请求后到覆盖新结果"）。
- `Cargo.toml` 加 `[lints.rust] unused_must_use = "deny"`：这类写法今后直接编译失败，
  不再依赖"谁去翻 CI 日志里的 warning"——第十轮之前那条 `unused 'Task' that must be used` 一直躺在那里。

顺带把这条链路的另一半也照着源码确认过，免得以后再怀疑"await 别人的 runtime 到底能不能醒"：
gpui 的前台任务由 `async_task` 驱动（`gpui-pre-scheduler/src/executor.rs:2`），
`LocalExecutor::spawn` 只要求 `F: Future + 'static`（**不要求 `Send`**，同文件 `:66-72`），而每个
runnable 的 schedule 回调是 `Fn(Runnable) + Send + Sync`（同文件 `:190-194`），唤醒来自哪个线程都会被
`dispatch` 送回主线程。所以"tokio 上跑请求、gpui 前台 await `JoinHandle`"这个模式本身是成立的，
之前不成立只是因为任务被 drop 掉了。`Task::detach` 的实现就是 `async_task::Task::detach`
（同文件 `:572-579`），detach 后无法取返回值，但会跑完。

**撤回一个错误结论**：本节上一版把根因写成"gpui Windows 的 `wake_posted` 唤醒竞态"，**那是错的**：
唤醒用的 `PostMessageW` 目标是平台隐藏窗口 `self.handle`，它在 `WindowsPlatform::new`
（`gpui_windows/src/platform.rs:224` 的 `CreateWindowExW`）里就创建好了，与业务窗口注册时机无关；
`RootView::startup_pending` 那套"首帧再派发"的规避已随之删除。教训：框架源码要读到底，
但没验证过的推断不要写成结论。

### 4.10 登录成功的最后一步：buvid3 不在扫码响应里

`Account::validate_complete` 要求 `DedeUserID / SESSDATA / bili_jct / buvid3` 四个字段全部非空
（`ref/bpi-rs/src/session/account.rs:110-115`），而 `qr_poll` 成功时 bpi-rs 只是把响应的
Set-Cookie 原样收集进 `data.cookies`（`ref/bpi-rs/src/login/client.rs:211-223`，只在 `code==0`
时才填）。B 站的轮询响应给的是登录三件套，`buvid3` 是**设备标识**、由匿名接口
`/x/frontend/finger/spi` 下发（`ref/bpi-rs/src/misc/client.rs:11`，`misc().buvid() ->
BuvidData { buvid3, buvid4 }`）。所以 `api/login.rs::poll_qr` 在 `code==0` 且响应里没有 buvid3 时
补取一次再交给凭据构造函数；补取失败就让界面显示"凭据不完整"，同时日志里列出这次响应到底带了
哪些 cookie 名——真机上一看就知道是 B 站改了口径还是我们漏了字段。

为此 `Cargo.toml` 的 bpi-rs features 多了 `misc`。

### 4.11 个人中心（P2b）：外壳 + 三路并发

- **窗口层次变成三层**：`RootView`（登录 vs 主界面）→ `MainShell`（搜索 vs 我的，顶部两个入口）
  → 具体页面。外壳把两个页面实体**都常驻持有**，切回来不重新请求；这样"搜完看一眼收藏再回来看结果"
  不会把列表刷没。mid 由 `LoginSucceeded { mid }` 事件或已保存的凭据提供，一路传到 `MinePage`。
- **个人中心是懒加载**：`MinePage::new` 只构造状态，`ensure_loaded` 由外壳在**第一次切到"我的"**时调用。
  理由是这一页随外壳一起构造，而用户可能全程只用搜索；启动就打三个接口既浪费，又会把三块
  与本意无关的错误行摆进首屏日志。刷新走页内"重新加载"按钮。
- **三块内容 = 三路并发**（资料 / 收藏夹 / 历史），共用一个 `seq`：一次 `reload` 只自增一次，
  三路任务写回前都检查 `view.seq == seq`，被取代的整路丢弃。每路各自 `cx.spawn(...).detach()`
  （第 4.9 节的规矩），各自把自己的错误写进自己的提示位——**一块挂了不能把整页变空白**，
  这是第 4.7 节"失败必须可见"在并发场景下的形态。
- **接口选择上的两个坑**（详细证据在 `docs/api-notes.md` 第 8 节）：
  `misc::NavData` 只是两个 WBI key，不是用户资料；真正的资料在 `user().space_info()`（自动 WBI），
  关注/粉丝在 `login().stat()`（在 `login` feature 里，不必为此开 `web_widget`）。
  收藏接口没有"取自己"的变体，`created_list` / `collected_list` 都要显式传 `up_mid`。
- **历史只要稿件**（`HistoryListType::Archive`）：专栏/直播条目那里没有 `bvid`
  （bvid 在 `item.history.bvid: Option<String>`，不是顶层），而我们下一页要进详情页。
- 历史的 `duration` / `progress` 是**数字秒**（搜索接口给的是 `"mm:ss"` 字符串，两边别混），
  格式化用 `api/history.rs::format_seconds`（纯函数，已进单测）。
  `view_at` 暂时不显示：本项目没有日期库，也不打算为一条次要文案引入 chrono——
  真要做的时候优先加依赖，不要手写日历换算。

## 5. 阶段进度

| 阶段 | 内容 | 状态 |
| --- | --- | --- |
| P0 | 依赖骨架、release CI、空窗口 + 中文输入框 + 假数据列表、`api/store/player` 纯逻辑层 | **完成**（run 39787252294 全绿） |
| P1 | 搜索接线：`search().video()`、结果转换（剥 `<em>`）、封面网络图（gpui 原生 URI 加载）、分页/排序 | 代码与 CI 均完成（run 39795360593）；**验证并入登录之后**（定稿不做免登录搜索） |
| P2a | 登录：扫码（取码/轮询/过期重载）+ 凭据落盘 + 启动判定 + 登录后进主界面 | **完成**（第十一轮产物真机扫码登录通过，凭据已写出） |
| P2b | 个人：我的信息 / 收藏 / 历史 入口与页面 | 代码完成（外壳 + 三路并发，机制见第 4.11 节），待编译与真机复验 |
| P3 | 详情：`video().detail()`、多 P、tags/related、字幕列表 | 未开始 |
| P4 | 播放：拉起 mpv、管道 IPC 读写、换清晰度（`loadfile` + `audio-remove`/`audio-add`）、`sub-add` | 未开始 |
| P5 | 打磨：详情页覆盖式布局、骨架屏、快捷键、续播进度 | 未开始 |

## 6. 已知风险与待验证

| # | 风险 | 处置 |
| --- | --- | --- |
| 1 | release 需要 `fxc.exe`（gpui `build.rs` 找不到就 panic） | **已解除**：CI 探测到 Windows SDK 的 fxc 并设 `GPUI_FXC_PATH`，该步通过 |
| 2 | `rustls 0.23` 默认 provider 拉 `aws-lc-sys`，MSVC 需 CMake + NASM | **已解除**：CI 的 `choco install nasm` 成功，aws-lc 编译通过 |
| 3 | 构建内有两份 reqwest（kit 的 `gpui-pre-reqwest 0.12.15` + bpi-rs 的 `0.13.5`） | 只影响编译时长，不动 |
| 4 | gpui-pre 快照每周变动、API 破坏性 | lock 入库作种子 + `=0.3.9` 精确 pin；升级要连带检查 kit 的 pin |
| 5 | gpui Windows 侧 `WindowDecorations::Client` 是空实现、不支持原生锚定弹窗、无托盘 API | 依赖 kit 组件自绘弹层；托盘如需，自调 `Shell_NotifyIconW` |
| 6 | 无 Mica/Acrylic 等价物（Bili.Copilot 的观感依赖它） | 用实色底近似，不追求一致 |
| 7 | mpv 对 DASH/fmp4 双流 seek 无任何文档 | P4 实测；失败备选：合并下载后本地播放 |
| 8 | 1080P 需登录、HDR/4K/8K 需大会员 | 按 `accept_quality` 降级 + `requires_vip` 提示 |
| 9 | rustls 同时启用 aws-lc-rs 与 ring → 未装默认 provider 时 TLS 握手 panic（症状：加载态永不结束） | 启动时 `app::install_tls_provider()`；所有 `JoinHandle` 错误都上界面（第 4.7 节） |
| 10 | **`cx.spawn` 的 `Task` 被丢弃 = 任务立即取消**（症状：请求发出去了、界面永远停在加载态，日志里连闭包第一行都没有） | 已修：所有 spawn 一律 `.detach()` 或存进实体字段，并用 `[lints.rust] unused_must_use = "deny"` 让这类写法编译失败，见第 4.9 节 |
| 11 | 扫码成功的响应 cookie 里没有 `buvid3`，而 `Account::validate_complete` 要它 → 登录在最后一步失败 | `poll_qr` 在 `code==0` 且缺 buvid3 时调 `misc().buvid()`（`/x/frontend/finger/spi`）补一个，见第 4.10 节 |
| 12 | 收藏/历史那几个响应结构**没有 `#[serde(default)]`**（`HistoryListData.cursor/tab/list`、`FavListDetailData.info/has_more/ttl` 等），B 站少给任何一个字段就变成 `BpiError::parse` | 症状可控：那一块显示自己的错误位而不是整页空白（第 4.11 节），日志里有原始错误文本可判断缺哪个字段 |

## 7. 维护规则

- 每个目录都有 `README.md`；改了这个目录的东西，就改它的 README，并更新本文相应小节，然后提交 git。
- **注释是硬要求**（项目规定，覆盖 Agent 默认的"少写注释"取向）：每个模块文件头用 `//!` 写清职责与
  定稿理由，`pub` 项用 `///`；机制、坑、被否决的方案必须写明，并尽量带上证据（参考仓库的文件行号或
  CI run 号）。注释解释"为什么"，不复述代码在做什么。
- 测试只覆盖纯逻辑（`src/tests/`）。UI 与网络的验证方式是：CI 产物在真机上跑起来看。
- 快速开发阶段不设 clippy/rustfmt 门禁，不追求零 warning；但编译必须无 error。

## 8. CI 实测记录（2026-10-22）

### 第一轮（run 39785338951）：依赖图全绿，我们自己的代码报错

- 前 6 步全绿（toolchain、NASM、fxc 探测、缓存），**整个依赖图 release 约 8 分钟编完**——
  远快于预估的 25–45 分钟（不 LTO + 不 strip 省下的）。
- 唯一失败点是我们自己的代码：5 个错误、两类原因，见第 2 节"参考仓库 ≠ 我们在用的版本"。
- `Cargo.lock`（881 个包；`gpui-pre 0.3.9` / `gpui-pre-windows 0.3.9` / `gpui-kit 0.8.0` /
  `bpi-rs 0.3.1`）已由 CI 的 `cargo-lock` artifact 产出并入库；P1 起改为**不加 `--locked`**
  （新增依赖时本机无法刷新 lock，加了必卡），lock 只作版本种子，每轮从 artifact 取回更新。
- **缓存教训**：`actions/cache` 在 post 步骤写盘，首轮失败时 post 不执行 → 缓存从未保存，
  第二轮仍是全量重编（>9 分钟）。已给该步骤加 `save-always: true`，并把 key 收窄为只按
  `Cargo.lock` 哈希（原先混入 workflow 与 Cargo.toml，改一行 CI 就整体失效）。

### 第二轮：P0 通过（run 39787252294）

- 全步骤 success，含 `cargo test --release`：**15 passed / 0 failed**，另有 1 个 doctest 被
  `ignore` 标记跳过（`api/mod.rs` 里的示例代码块）。
- 无缓存可命中时约 14 分钟编完全图（`debug=true`、不 LTO、不 strip）。
- 产物 `rustbilipc-win-x64-release`：zip 8.7 MB，解出的 `publish/release/rustbilipc.exe` 24 MB。
- 待真机验证（UI 只能这样验）：窗口能否打开、中文 IME 能否输入、底部自检行显示的
  凭据/mpv 状态是否符合预期。
- **真机已验证**（2026-10-22）：窗口正常打开、输入框可打中英混排、自检行显示"未登录 +
  mpv 查找结果"符合预期；点击输入框时 gpui 会打一条 a11y INFO 日志（元素有 id 无 role），
  属噪音，后续给可聚焦元素补 role 时消掉。

### 第三轮（run 39792866994）：P1 代码 3 个错误

- `Button::primary()` 缺 trait 导入、`InputEvent::PressEnter` 是带字段的变体——细节都记在
  第 2 节的版本差异清单里。缓存已命中，从触发到我们的 crate 报错只用约 6 分钟。

### 第四轮（run 39795360593）：P1 通过

- 全部步骤 success，含 `cargo test --release`；产物 zip 解出的 `publish/release/rustbilipc.exe`
  约 31.6 MB（比 P0 的 24 MB 大，因为链进了 gpui-component 组件层与 bpi-rs）。
- lock 从 artifact 取回更新：新增 3 个包（`gpui-pre-reqwest-client`、`gpui-pre-http-client-tls`、
  `webpki-roots`，ReqwestClient 走的是打包根证书而非系统根），总数 881 → 884。
- 待真机验证：真实搜索结果与分页/排序、封面图是否显示（这条同时验 `set_http_client` 是否生效）。
- **真机日志噪音（不是故障）**：点击输入框时可能打出
  `ERROR unable to get cursor position: 拒绝访问。(0x80070005)`——来自
  `gpui_windows/src/window.rs:693-703` 的 `GetCursorPos`，它用 `.log_err()` 记录后照常返回，
  不影响功能；之前那条 a11y INFO 同理。看到别当报错排查。

### 第五轮（run 39802276345）：登录功能一次通过

- 全步骤 success，一次过、零返工；`cargo test --release` **28 passed / 0 failed**
  （P0 的 15 + 搜索格式化 6 + 登录 5 + 二维码 2）。
- 缓存命中后从触发到结束 **7 分 11 秒**（15:10:17 → 15:17:28），这是后续迭代的常态耗时。
- lock 再次从 artifact 取回入库：新增 `qrcode-generator` 及其两个依赖（`atomic-write-file`、
  `nix`），881 → 887 个包。
- 产物按新约定放在 `publish/rustbilipc.exe`（约 32.0 MB），artifact zip 根目录同样直接是 exe。

### 第六、七轮（run 39806973376）：TLS provider 修复

- 第六轮唯一错误是 `install_default()` 的 `Err` 变体不实现 `Display`（见第 2 节）；第七轮修好后
  全步骤 success，**28 passed / 0 failed**，耗时约 5 分钟（缓存全命中）。
- 背景是第 4.7 节那次真机诊断：加载态永不结束 → 任务 panic 被 `JoinHandle::Err` 吞掉。
  现在两处（登录取码、轮询）都会把 `describe_join_error()` 的文案显示到界面上。
- lock 只是本包依赖列表变化（新增直接依赖 `rustls`），包数仍 887。

### 第八、九轮（run 39812160559 失败 / run 39883479417 通过）：日志埋点上机

- 第八轮失败是我的两个流程错误叠加：上一轮没跑完就推新提交（白烧一轮），以及 `log::Level`
  上用了不存在的方法 `is_enabled`（`log` 门面在调用 `log()` 前已按 `set_max_level` 过滤，
  这层判断本就多余）。已把"一轮只烧一次"写进 QODER.md。
- 第九轮全绿（28 passed），产物 31.9 MB；lock 因移除 `env_logger` 从 887 → 877 个包。
- **重要观测（排除整类猜测）**：在同一台机器上用 `curl --noproxy '*'` 直连
  `passport.bilibili.com/x/passport-login/web/qrcode/generate` 得到 `http=200`、TLS 0.19s、
  总耗时 0.22s，并返回合法 `qrcode_key`；`/x/web-interface/nav` 同样正常（`-101 未登录` 是
  预期响应，`wbi_img` 在里面）。所以"界面停在加载态"与网络、DNS、系统代理无关，
  只可能是进程内的问题：任务没被 tokio 调度，或结果回到 gpui 前台的那一跳丢了。
  对照第 4.8 节的埋点位置读 `debug.log` 即可分辨是哪一种。

### 第十轮（run 39886233646）：一个错误的绕法

- 全绿（28 passed），产物 31.9 MB；但这一轮按"`wake_posted` 唤醒竞态"的猜测加了"首帧 render 里
  才派发启动异步"的绕法（`RootView::startup_pending`），**方向是错的**，真机仍然停在加载态。
- 读到底才看清两件事：唤醒用的 `PostMessageW` 目标是 `WindowsPlatform::new` 里早就创建好的平台
  隐藏窗口，与业务窗口注册时机无关；而 `debug.log` 里"闭包第一行都没有"的真正原因是我们自己把
  `cx.spawn` 返回的 `Task` 当场丢掉了 —— CI 从第七轮起就在报
  `unused 'Task' that must be used`，是我按"快速开发阶段先不管 warning"把它跳过了。
  第 4.9 节记录了根因、绕法的撤回和防复发措施。

### debug.log 的实际结论

日志显示 tokio 侧 190ms 正常返回，但视图层 `cx.spawn` 闭包的第一行与 10 行心跳**全部缺失**
⇒ 属于第二类，但不是"前台队列不被唤醒"，而是**那段 future 从来没被 spawn 成功过**：
返回的 `Task` 被当场 drop，任务在第一次 poll 之前就取消了（第 4.9 节）。
`install_tls_provider` 那个修复也是必要的（不装 provider 连请求都发不出去），但单靠它不够——
两处都要成立才能跑通。第十一轮同时补上第 4.10 节的 buvid3 补齐，否则登录会在写凭据那一步失败。

### 第十一轮（run 39890864644）：Task 修复 + buvid3 补齐，一次通过

- 全步骤 success，**29 passed / 0 failed**（新增 `only_a_successful_poll_without_buvid3_needs_backfill`）。
- 依赖图缓存全命中：`bpi-rs`（因新增 `misc` feature 重编）+ 我们的 crate 共 **47 秒**编完，
  整轮约 5 分钟。`unused_must_use = "deny"` 没有绊倒任何东西——说明四处 `cx.spawn` 都持有了。
- 唯一 warning 是登录页多余的 `ButtonVariants` 导入（这一页没用到 `.primary()`），已删。
- **真机已验证通过**（2026-10-23，第十一轮产物）：二维码正常显示、扫码后状态推进、
  登录成功并写出 `publish/data/cookies.json`（四个字段齐全，`buvid3` 由 finger/spi 补取），
  之后进入搜索页可用。心跳打满 5 次以后再扫码照样成功——证实心跳只是诊断脚手架、不参与登录。

### 第十二轮（run 39893441663）：日志回运行同目录 + 删心跳 —— 我自己写坏了一处匹配

- 这一轮的改动本身：`logging::log_dir()` 改回 `exe_dir()`，按定稿日志与凭据同层
  （`publish/debug.log`），每次启动 `File::create` 清空；同时把"取产物只搬 exe、不整目录清空"
  写成规矩（第 4.8 节）。删掉登录页的心跳任务与 `HEARTBEAT_TICKS`，改为轮询循环里一条常驻的
  "轮询结果已回到前台（代 N）"，证据力更强且不刷无用行。
- 失败：5 个 E0308，全在登录页那个 `match state`。原因是我把 `Ok(Ok(state))` 改成了 `Ok(inner)`
  并把日志行插进去，于是被匹配的值变成了 `Result<QrPollState, BpiError>`。
- 教训不是"少改"，而是**改完要把整段读一遍**：这一处只是给一条日志挪位置，
  却把匹配的形状改了；一行 review 就能拦住，不用等 5 分钟的 CI。

### 第十三轮（run 39895258094）：P2b 个人中心 —— 只有一个错，还是生命周期

- 失败：1 个 E0310，`MinePage::spawn_load<T>` 的 `T` 要跨 `cx.spawn` 的 `'static` future，
  缺 `T: 'static`；rustc 直接把该加什么写在 help 里（`<T: 'static>`）。
- 有价值的信息：**整个 lib 只报了这一个错**，说明 P2b 那三个 api 封装（profile/favorites/history）、
  外壳、根视图的类型全部核对到位——`space_info` / `created_list` / `collected_list` /
  `history_list` 的签名与字段名都没猜错（逐个查 `ref/bpi-rs` 起作用了）。
