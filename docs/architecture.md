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
├── app.rs             tokio runtime（OnceLock）、exe_dir()/data_dir()
├── theme.rs           全局主题覆盖：accent = #FB7299，accent_foreground = #1F1F1F
├── api/
│   ├── mod.rs         spawn_on_api_runtime()：gpui ↔ tokio 的调用桥
│   ├── login.rs       扫码登录：取码/轮询 + 码值翻译 + cookie→凭据（纯函数可测）
│   ├── search.rs      搜索封装：DTO（SearchHit/PageInfo/SearchSort）+ 标签清洗 + 播放量格式化
│   └── stream_select.rs  DASH 选择纯函数（HEVC 优先、画质档封顶 116）
├── store/
│   └── credentials.rs 明文 JSON 凭据落盘（<exe>/data/cookies.json）
├── player/
│   ├── mod.rs         mpv 参数构造 + Referer/UA 常量 + PATH 查找
│   └── ipc.rs         mpv JSON IPC 的命令构造（纯数据，管道 I/O 在 P4）
├── pages/
│   ├── root.rs        窗口根视图：无凭据→登录页，有凭据→主界面；登录成功后原地切换
│   ├── login/         扫码登录页（二维码 + 1.5s 轮询 + 过期自动重载）
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
});
```

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

## 5. 阶段进度

| 阶段 | 内容 | 状态 |
| --- | --- | --- |
| P0 | 依赖骨架、release CI、空窗口 + 中文输入框 + 假数据列表、`api/store/player` 纯逻辑层 | **完成**（run 39787252294 全绿） |
| P1 | 搜索接线：`search().video()`、结果转换（剥 `<em>`）、封面网络图（gpui 原生 URI 加载）、分页/排序 | 代码与 CI 均完成（run 39795360593）；**验证并入登录之后**（定稿不做免登录搜索） |
| P2a | 登录：扫码（取码/轮询/过期重载）+ 凭据落盘 + 启动判定 + 登录后进主界面 | 代码完成，待真机验证 |
| P2b | 个人：我的信息 / 收藏 / 历史 入口与页面 | 未开始 |
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

## 7. 维护规则

- 每个目录都有 `README.md`；改了这个目录的东西，就改它的 README，并更新本文相应小节，然后提交 git。
- 代码注释写"为什么"，不写"是什么"：尤其是上面 4.x 的机制，注释与本文互为索引。
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
