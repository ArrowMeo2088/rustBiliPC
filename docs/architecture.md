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
| 放弃 | `ref/gpui-component`（0.2.2）、Ely、`ref/zed` 作依赖 | 分别因为：拉 tree-sitter + `lb-wry`（WebView2）；`publish=false` 只能 git 依赖 + Rust 1.95；是 0.3.9 之后的未发布主干 |
| API | `bpi-rs = "0.3.1"`，`default-features = false` + 6 个 feature | 关掉 `full` 的 27 个模块，只开 login/search/video/user/fav/historytoview |
| 异步 | 自建 tokio runtime（`src/app.rs`） | gpui 执行器不是 tokio，reqwest 需要 reactor |
| 编译配置 | **release**，`lto = "off"`、`strip = false`、`debug = true` | 见下节 |
| 依赖锁定 | **提交 `Cargo.lock`**，CI 用 `--locked` | 挡住 gpui-pre 每周快照的破坏性变更 |

### 为什么必须 release

`gpui-pre-windows 0.3.9` 的 `directx_renderer.rs` 在 `cfg(debug_assertions)` 分支里用
`D3DCompileFromFile(env!("CARGO_MANIFEST_DIR")/src/shaders.hlsl)` **在运行时**编译 HLSL。
这个路径在编译期被烧进二进制，CI 产物拿到别的机器上必然找不到该文件，窗口起不来。
release 分支（`build.rs` 里 `#[cfg(not(debug_assertions))] compile_shaders()`）用 `fxc.exe`
在编译期把字节码 `include_bytes!` 内嵌，产物才可移植。

`gpui_platform` 的 `runtime_shaders` 特性只映射到 `gpui_macos`，**Windows 上没有开关**。

## 3. 模块布局

```
src/
├── main.rs            启动序列：logger → runtime → with_assets → init → theme → open_window
├── lib.rs             模块树 + VERSION
├── app.rs             tokio runtime（OnceLock）、exe_dir()/data_dir()
├── theme.rs           全局主题覆盖：accent = #FB7299，accent_foreground = #1F1F1F
├── api/
│   ├── mod.rs         spawn_on_api_runtime()：gpui ↔ tokio 的调用桥
│   └── stream_select.rs  DASH 选择纯函数（HEVC 优先、画质档封顶 116）
├── store/
│   └── credentials.rs 明文 JSON 凭据落盘（<exe>/data/cookies.json）
├── player/
│   ├── mod.rs         mpv 参数构造 + Referer/UA 常量 + PATH 查找
│   └── ipc.rs         mpv JSON IPC 的命令构造（纯数据，管道 I/O 在 P4）
├── pages/
│   └── search/        阶段一页面（view.rs 为根视图）
├── ui/
│   └── video_card.rs  卡片视图 + 骨架期假数据
└── tests/             单元测试集中地（纯逻辑）
```

分层规则（越界要在评审时拦住）：

- `pages/*` 只做装配与渲染，网络调用一律经 `api/`，不直接 `use bpi_rs`。
- `api/`、`store/`、`player/` 是**无 UI 依赖**的层，必须能在 `cargo test` 里独立跑。
- `ui/` 只放跨页面复用组件；页面私有的视图留在该页面目录内。
- gpui 的类型统一从 `gpui_kit::*` 取，不单独 `use gpui::...`（kit 已把 gpui 放在根上）。

## 4. 关键机制

### 4.1 异步调用桥

`cx.spawn(client.search().video(..))` 会 panic（no reactor running）。规定写法：

```rust
let join = crate::api::spawn_on_api_runtime(async move { client.search().video(params).await });
cx.spawn(async move |_, cx| {
    let result = join.await;              // await JoinHandle 不需要 reactor
    cx.update(|_, cx| { /* 写回页面状态 */ }).ok();
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

## 5. 阶段进度

| 阶段 | 内容 | 状态 |
| --- | --- | --- |
| P0 | 依赖骨架、release CI、空窗口 + 中文输入框 + 假数据列表、`api/store/player` 纯逻辑层 | 进行中 |
| P1 | 搜索接线：`search().video()`、结果转换（剥 `<em>`）、封面网络图 + 缓存、虚拟列表、分页 | 未开始 |
| P2 | 个人：扫码登录（`qr_generate`/`qr_poll` 轮询 + `qrcode` 自绘）、凭据读写与失效处理、我的信息/收藏/历史 | 未开始 |
| P3 | 详情：`video().detail()`、多 P、tags/related、字幕列表 | 未开始 |
| P4 | 播放：拉起 mpv、管道 IPC 读写、换清晰度（`loadfile` + `audio-remove`/`audio-add`）、`sub-add` | 未开始 |
| P5 | 打磨：详情页覆盖式布局、骨架屏、快捷键、续播进度 | 未开始 |

## 6. 已知风险与待验证

| # | 风险 | 处置 |
| --- | --- | --- |
| 1 | release 需要 `fxc.exe`（gpui `build.rs` 找不到就 panic） | CI 先探测 SDK 路径，必要时设 `GPUI_FXC_PATH`；**首次 release 构建必须确认** |
| 2 | `rustls 0.23` 默认 provider 拉 `aws-lc-sys`，MSVC 需 CMake + NASM | CI 加 `choco install nasm`；不做砍 http3 的清理 |
| 3 | 构建内有两份 reqwest（kit 的 `gpui-pre-reqwest 0.12.15` + bpi-rs 的 `0.13.5`） | 只影响编译时长，不动 |
| 4 | gpui-pre 快照每周变动、API 破坏性 | lock 入库 + `--locked`；升级要连带检查 kit 的 pin |
| 5 | gpui Windows 侧 `WindowDecorations::Client` 是空实现、不支持原生锚定弹窗、无托盘 API | 依赖 kit 组件自绘弹层；托盘如需，自调 `Shell_NotifyIconW` |
| 6 | 无 Mica/Acrylic 等价物（Bili.Copilot 的观感依赖它） | 用实色底近似，不追求一致 |
| 7 | mpv 对 DASH/fmp4 双流 seek 无任何文档 | P4 实测；失败备选：合并下载后本地播放 |
| 8 | 1080P 需登录、HDR/4K/8K 需大会员 | 按 `accept_quality` 降级 + `requires_vip` 提示 |

## 7. 维护规则

- 每个目录都有 `README.md`；改了这个目录的东西，就改它的 README，并更新本文相应小节，然后提交 git。
- 代码注释写"为什么"，不写"是什么"：尤其是上面 4.x 的机制，注释与本文互为索引。
- 测试只覆盖纯逻辑（`src/tests/`）。UI 与网络的验证方式是：CI 产物在真机上跑起来看。
- 快速开发阶段不设 clippy/rustfmt 门禁，不追求零 warning；但编译必须无 error。
