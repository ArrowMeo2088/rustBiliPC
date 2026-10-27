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
| API | `bpi-rs = "0.3.1"`，`default-features = false` + 8 个 feature | 关掉 `full` 的 27 个模块，只开 login/search/video/user/fav/historytoview/misc/comment（最后一个是 P6b 评论区；它在 bpi-rs 里写作 `comment = []`，纯 cfg 开关、不引入新 crate） |
| 异步 | 自建 tokio runtime（`src/app.rs`） | gpui 执行器不是 tokio，reqwest 需要 reactor |
| 编译配置 | 日常（`ci.yml`）用 **release**：`lto = "off"`、`strip = false`、`debug = true`；发布（`cd.yml`）用 **`release-dist`**：`lto = "fat"`、`codegen-units = 1`、`strip = "symbols"`、`debug = false` + UPX | 见下节；两份配置的分工在第 4.28 节第 ④ 条 |
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
  并提示一个不相干的 `overflow_y_scrollbar`。**`on_click()` 同一条规则**（P3 又踩一次）：
  它也在 `Stateful<Div>` 上（`gpui/src/elements/div.rs:1590`），`Div` 上只有 imperative 版
  （`:591`），所以写 `div().on_click(..)` 一样不通过。
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
├── main.rs            启动序列：**读设置 → 按"调试模式"装日志** → runtime → with_assets → init → theme → set_http_client → 读凭据 → 开窗（GUI 子系统：`#![windows_subsystem]`，双击不弹控制台）
├── lib.rs             模块树 + VERSION
├── app.rs             tokio runtime（OnceLock）、install_tls_provider()、describe_join_error()、exe_dir()/**settings_dir()**、**进程级 CONFIG（RwLock）：config()/update_config()**
├── logging.rs         debug.log 文件日志（写**用户数据目录**，每次启动清空、Debug 级、零依赖、同时写 stderr）；`init(enabled)` **可整个关掉**，`log_path()` 返回 `Option`
├── theme.rs           全局主题覆盖：accent = #FB7299，accent_foreground = #1F1F1F
├── api/
│   ├── mod.rs         spawn_on_api_runtime()：gpui ↔ tokio 的调用桥
│   ├── login.rs       扫码登录：取码/轮询 + 码值翻译 + cookie→凭据 + 缺 buvid3 时补取（纯函数可测）
│   ├── profile.rs     个人中心头部：user().space_info()（自动 WBI）+ login().stat()（关注/粉丝）
│   ├── collection.rs  **合集与系列（P12）**：空间页那一格（`seasons_series_list`，一次给两类）+ 内容页（`seasons_archives_list` / `series_archives`）+ `date_text`（无 chrono 的日期换算）
│   ├── detail.rs      视频详情：view() 主体 + desc()/tags()/related_videos() 三块各自容错 + 字幕列表 + **自己再发一次 view 读 `ugc_season`（分集）**
│   ├── favorites.rs   收藏夹：fav().created_list() + collected_list()，合并成一个 DTO；P9 起还有 list_detail 内容页（页码 + 服务端 keyword + 挑 bvid）
│   ├── history.rs     观看历史：historytoview().history_list()（游标分页）+ 秒数格式化纯函数
│   ├── comments.rs    评论区：comment().list()（type=1、oid=aid、ps≤20）+ 相对时间与翻页判断纯函数
│   ├── home.rs        主页：video().homepage_recommendations()（WBI 由库里签）+ **按 goto 过滤**
│   ├── space.rs       用户空间：card()（不签 WBI）+ uploaded_videos()（签 WBI、带分区筛选）+ opus 图文（游标）
│   ├── search.rs      搜索封装：DTO（SearchHit/PageInfo/SearchSort）+ 标签清洗 + 播放量格式化
│   ├── toview.rs      待看：historytoview().toview_list()（零参数、没有分页、要登录、不签 WBI）
│   ├── stream.rs      播控：video().play_url() 取地址 + 选流成 StreamChoice（DASH 双流 / durl 单文件、两轨大小与备用域名、落盘后缀）；请求 qn 与封顶档都取自 `app::config()`，**清晰度列表也按它过滤**
│   ├── download.rs    **下载（P11）**：StreamChoice → DownloadRequest，用 `App::http_client()` 分块读流写盘（`.part` + 改名、备用域名、标志取消、durl 拒绝）；文件名清洗与进度读数都是纯函数
│   ├── probe.rs       **起播前的地址探测（P13）**：`Range: bytes=0-0` 问一句"这个地址通吗"，主地址不通就按顺序换 `backupUrl`；`ordered_urls`/`cdn_headers` 与下载共用，`choose`/`describe`/`host_of` 是纯函数
│   └── stream_select.rs  DASH 选择纯函数（画质档优先，同档内**先按设置里的编码偏好**、再按 HEVC > AV1 > AVC；封顶值由调用方给）
├── store/
│   ├── auth.rs        明文 JSON 凭据落盘（`<用户数据目录>/auth.json`）+ cookie_header() 给 mpv；**旧位置（exe 同目录 / data/cookies.json）的读取与迁移代码已全部删除**
│   └── config.rs      明文 JSON 设置落盘（`<用户数据目录>/config.json`）：**四项**（`max_quality`/`codec`/`mpv_path`/`debug_log`）都有读取方；`clamp_quality` 收边界、`LoadOutcome` 让"配置坏了"那一轮强制开日志
├── player/
│   └── mod.rs         mpv 参数构造（一律 `--opt=value`）+ launch() 拉起进程 + Referer/UA 常量 + **resolve_mpv()：设置指定 → PATH → 运行目录（只拼 mpv.exe，绝不问 .com）**
                      **不做播放期 UI，也不接它的命名管道/文本输出**（范围定稿见第 4.3 节）
├── pages/
│   ├── mod.rs         页面层出口 + 四个共用导航事件 `OpenVideo` / `OpenUser` / `OpenFolder` / `OpenCollection` + 本地关键词过滤 `matches_keyword`
│   ├── windows.rs     窗口这一层：主窗 1120×760 与登录窗 320×420；**先开主窗再关登录窗**（Windows 默认 QuitMode 会在最后一个窗口关闭时退进程）
│   ├── gate.rs        登录窗根视图：摆登录页 + 收到 LoginSucceeded 后在**任务里**收尾；`GateMode::{Startup, Relogin}`（P13）——启动是"开主窗再关自己"，重登是"只关自己再刷新外壳"，走错会出现两个主窗口
│   ├── loading.rs     各入口页共用的"投请求 → 序号判定 → 写回实体"样板（Task 必 detach）；P13 起顺手记 `-101` 并在实体更新之外那层调 `relogin::flush`
│   ├── relogin.rs     **凭据过期的统一处置（P13）**：进程级协调器（客户端 / 外壳弱句柄 / 登录窗句柄），判定用库里的 `requires_login()`，删凭据 + 去重开小窗
│   ├── shell/         外壳：**80 宽窄侧栏**（左上是账号头像、只显示不可点；Lucide 图标 + 两字的七入口 主页/搜索/收藏/历史/待看/下载/设置；栈非空时第一项是"返回"，点入口**先清栈再换页**）+ **下钻栈**（`StackPage::{Detail, Space, Folder}`）+ **创建下载队列实体并注入两页**
│   ├── home/          主页（P6 第一步，默认页）：首页推荐流网格 + "换一批"
│   ├── login/         扫码登录页（二维码 + 1.5s 轮询 + 过期自动重载），住在自己的小窗里
│   ├── favorites/     收藏页两层：列表（本地筛选 + 点进去发 `OpenFolder`）与内容页 `folder.rs`（卡片网格 + 服务端 keyword + 加载更多；**P13：滚到底自动接下一页**）
│   ├── toview/        待看页：稍后再看列表（接口一次给全，所以没有分页控件；本地筛选）
│   ├── download/      下载：**`queue.rs` = 任务队列实体**（重名编号、进度事件回传、按标志取消、只清已结束的记录）+ **`view.rs` = 任务列表**（进度、取消、打开所在目录、用 mpv 打开并管好子进程、移除记录）
│   ├── about/         **设置页（P13 起是真设置页）**：账户 + 本机路径，加**四个能改的块**（默认播放画质 / 默认播放编码 / MPV 播放器位置（含"选择…"系统对话框）/ 调试模式），改一项就 `update_config` 落盘一次、失败原因写在页面上
│   ├── history/       历史页：最近观看卡片网格（本地筛选 + 游标"再加载 30 条"；**P13：滚到底自动接下一页**）
│   ├── detail/        详情页（主列信息 + 播放块【播放（**P13：起播前先探地址、不通就换 `backupUrl`**）/ **下载这一 P**】+ 字幕 + 评论区；右栏 360 = **分集（分段 chips + 一集一行 + `N P` 角标）** + 选集/本集的分 P + 相关推荐；持 mpv 子进程并在 Drop 时收掉；**标题/简介/评论可选中复制**）
│   ├── space/         用户空间页（P7：资料卡 + 投稿/图文/收藏三块，投稿带分区筛选 chips；**P12 加第四格"合集"**：一次请求同时列合集与系列，点一行发 `OpenCollection`；**P13：图文与合集两格滚到底自动接下一页**——投稿那一格是整屏换页，不自动）；
│   ├── collection/    **合集/系列内容页（P12）**：一列分集（跨页累加 + "再加载一页"；**P13：滚到底自动接下一页**），接口没有 `keyword` 所以**不放搜索框**；
│   └── search/        阶段一页面（view.rs 为根视图；**P13：滚到底把下一页"接到"列表尾巴**，手动上/下页仍整屏换）
├── ui/
│   ├── palette.rs     **颜色与度量的唯一出处**：把 Bili.Copilot 的 WinUI 主题键与 xaml 数字翻译成 gpui 的值
│   ├── layout.rs      页头/分隔线/区块标题/侧栏项/圆头像/统计行/卡片容器
│   ├── qr.rs          二维码：qrcode-generator 出布尔矩阵，div 网格绘制
│   ├── cover.rs       封面显示前必过：补协议 + hdslb 图床 `@{w}w_{h}h.jpg` 缩略图后缀
│   ├── scroll.rs      滚动容器的"按住拖动滚动"（Windows 上 gpui 没有触摸管线，靠注入的鼠标消息；见第 4.23 节）；**P11 起按下时先问 `TextSelection::is_selectable_at`，落在可选中文字上就让位给选字**（第 4.25 节）；**P13 加自动翻页判据**：`is_at_bottom`/`should_auto_page`/`DragScroll::scrollable`
│   └── video_card.rs  竖卡 232×238 与侧栏横卡 104（flex_wrap 当 UniformGrid 用、时长角标、hover、可点击）
└── tests/             单元测试集中地（纯逻辑）
```

分层规则（越界要在评审时拦住）：

- `pages/*` 只做装配与渲染，网络调用一律经 `api/`，不直接 `use bpi_rs`。
- `api/`、`store/`、`player/` 是**无 UI 依赖**的层，必须能在 `cargo test` 里独立跑。
  **一条已定稿的例外**（P11）：`api/download.rs` 收 `Arc<dyn gpui::http_client::HttpClient>`。
  它是**传输抽象**而不是界面类型（`App::http_client()`，与封面图共用那份 reqwest 客户端），
  而 bpi-rs 每个接口都把响应整个读进内存——一部片子不能这么读，只能用它把响应体当 `AsyncRead` 分块搬。
  代价：这一段没法在 `src/tests/` 里跑，所以那里只测命名/大小/状态这些纯逻辑，
  搬运与写盘靠真机验。
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

- **只把直链 URL 交给 mpv，在线流式播**。B 站是 DASH 音视频两条流，读字节、切片、缓存、seek
  都归 mpv 自己的 demuxer；我们绝不先把片子下载到磁盘。
- 音视频是两条 DASH URL：视频当主文件，音频走 `--audio-file=`（可接受远程 URL）。
- `--http-header-fields` / `--user-agent` / `--referrer` 是**实例级全局**（`mp_network_conf`），
  对同实例的所有 HTTP 流生效，所以 URL 必须与这套头成对出现，否则 403。
- 多条头拼成**一个**逗号分隔的 `--http-header-fields=`：列表选项按 `,` 切分
  （`ref/mpv/options/m_option.c:1403-1440`），而命令行上重复给同一列表选项是**替换**不是追加。
- **带值的选项一律 `--opt=value`**。mpv 的 CLI 只额外承认单连字符的 legacy 形式
  `-opt value`（`ref/mpv/DOCS/man/mpv.rst:440-444`），`--opt value` 会被当成文件名并报
  `Error parsing commandline option user-agent: option requires parameter` 然后
  `Exiting... (Fatal error)`——**在建窗口之前**就退出。第十八轮"mpv 窗口没出现"就是它，
  详见第 8 节末与第 4.13 节。
- **mpv 的位置：`<exe 同目录>/mpv.exe`（不走 PATH）**。PATH 上 `PATHEXT` 先命中 `mpv.com`，
  那是控制台版启动器，会再派生一个 mpv.exe：我们 `kill` 的只是外层，真正的播放器脱管。
  懒人包（mpv-lazy）还带自己的 `portable_config` 和单实例管道，更不可控。
- **不接 mpv 的文本输出**（用户定稿，2026-10-23）：`launch()` 把 stdout 与 stderr 都设 `null()`。
  播放状态一律以 IPC 为准，日志里再抄一份 mpv 自己的话没有意义；而且 `piped()` 之后没人抽水会
  **把 mpv 的写阻塞住**，那是更坏的沉默。要查 mpv 为什么不动，就在本机把同一套参数复跑一遍——
  第十八轮那条 `--opt value` 致命错误就是这样定位的。
  顺带记一条源码事实：**mpv 的消息默认写 stdout**，只有 `force_stderr` 才走 stderr
  （`ref/mpv/common/msg.c:190-202` 的 `term_msg_fileno`；本机实测一次失败的打开，
  stdout 613 字节、stderr 0 字节）。所以"只接 stderr"其实等于什么都没接。
- **播放期的 UI 与控制一律归 mpv**（范围定稿，2026-10-23）：进度条、音量、全屏、暂停、字幕显示
  都由 mpv 自带的 OSC 负责，rustBiliPC **不做**播放控制条，也不把播放时间显示到详情页。
  我们只负责"取地址 + 拼参数 + 拉起进程"。曾按"遥控"做过一版命名管道客户端
  （见第 4.14 节），已按这条范围删净——**连 `--input-ipc-server` 这个参数也一并去掉了**，
  不给人"是不是还有人在听这条管道"的错觉。P6 要做"同实例换清晰度"和 `sub-add` 时再加回来，
  机制细节全在第 4.14 节备查，不必重新查证。
- `--no-config` 隔离用户自己的 `mpv.conf`，避免别人的 `vo`/`hwdec` 设置干扰。
- **参数表里只放所有构建都存在的选项**：`--no-ytdl` 看着很合理（我们给的是解析好的直链，不需要
  URL 提取器，还能少一句没用的 `youtube-dl failed` 噪音），但 `{"ytdl", …}` 整块在
  `#if HAVE_LUA` 里（`ref/mpv/options/options.c:570-574`）——不带 Lua 的构建会把它当未知选项，
  症状又退回"静默不出现窗口"。而 `http-header-fields`/`user-agent`/`referrer`
  （`stream/network.c:34-40`）与 `input-ipc-server`（`options/options.c:922`）是无条件的，可以留。
- mpv 不在运行目录是常态：搜索页底部状态行显示该目录路径，详情页起播失败会写明该放哪儿。

### 4.4 凭据

`<用户数据目录>/auth.json`（Windows `%APPDATA%\ArrowMeo\rustBiliPC`，见第 4.27 节），明文
（定稿选择，风险已在 `store/auth.rs` 顶部写明）。位置换过两次：`data/cookies.json` →
exe 同目录（2026-10-26 上午）→ `%APPDATA%`（2026-10-26 下午 P13，**旧位置的读取与迁移代码已全部删除**）。
`bpi_rs::session::Account` 只 derive `Deserialize`，所以用 `Auth` 做可序列化镜像，
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
  （实测四种组合——`http`/`https` × 自报家门 UA/浏览器 UA、带不带 Referer——全部 200。）
- `cover_url` 为空时画占位块而不是提交空 URI，避免必然失败的请求刷日志。
- **但第十五轮真机上封面是空的，原因不在网络也不在 HttpClient**：B 站搜索接口给的 `pic` 是
  **省略协议的** `//i2.hdslb.com/bfs/archive/xxx.jpg`，而 `is_uri` 靠 `url::Url::parse` 判断，
  `//host/path` 没有 base 必然解析失败 ⇒ 走 `Resource::Embedded` 分支去 asset 里找，
  一次网络都没发。`debug.log` 里的现场（23 条同一句）：

  ```text
  ERROR gpui::asset_cache: Failed to load asset: Asset("Embedded resource not found: //i2.hdslb.com/…jpg")
  ```

  同一份日志里个人中心头像正常连上了 `https://i2.hdslb.com`，因为 `space_info` 的 `face`
  本来就是绝对地址——这条对照把"CDN 拦我们 / HttpClient 没装"两种猜测一次排除。
- **显示前必须过 `ui/cover.rs`**：`for_display(url, 宽, 高)` = 补协议 + 拼图床的
  `@{w}w_{h}h.jpg` 缩略图后缀。尺寸是真量出来的：稿件封面原图 **541,549 字节** →
  `@320w_180h.jpg` **14,832 字节**；头像原图 **812,735 字节** → `@112w_112h.jpg` **2,740 字节**。
  一页 30 条就是 16MB 与 0.44MB 的差别，gpui 的图片加载没有并发上限，在 i5-6200U 上
  同时拉 30 张原图必然表现为"图迟迟不来"。非 `hdslb.com` 的域名与 GIF 一律不动
  （`@…` 是图床私有语法，实测写错返回 400）。
- 加载失败时界面要有兜底视图（`with_fallback`，`img.rs:166`）：gpui 只会调它，不会自己画占位，
  所以卡片给一块深色底，用来把"图挂了"和"还没加载完"分开。日志不用另写——
  asset 管线自己 `log::error!("Failed to load asset: {e:?}")`（`asset_cache.rs:134`），
  我们的文件日志收得到。

### 4.6 扫码登录与启动判定（定稿：登录先行，不做免登录搜索）

- **启动判定**：`Auth::load()` 且四字段完整 → 把凭据 `set_account` 进共享的 `BpiClient`
  并**只开主窗口**；否则**只开一个小的扫码登录窗**。判定的位置从 `pages/root.rs` 移到了
  `src/main.rs` + [`pages/windows.rs`](../src/pages/windows.rs)。
  **这一条在 2026-10-26 被用户推翻并重来**：当初不开第二个窗口的理由是"取窗口句柄没有可抄的先例"，
  现在核实到位了（`Window::window_handle()` 在 `gpui/src/window.rs:2371`，
  关窗口是 `Window::remove_window()` `:2384`，kit 的 `open_window` 返回
  `(AnyWindowHandle, Entity<V>)` `crates/kit/src/lib.rs:181-193`），
  于是按他的要求做成独立登录窗。**新方式的两条硬约束记在第 4.19 节**，`root.rs` 已删除，别改回去。
- **二维码不用图片管线**：`qrcode-generator`（只开 `std`+`qr` feature）出 `Vec<Vec<bool>>`，
  一格一个 div 铺出来。走 `RenderImage` 要构造 Zed 自己那个 `image` 分支的 `Frame`
  （`gpui/src/svg_renderer.rs:5`），而且会把 pixels 版 `image` 拖进同一棵依赖树。
- **轮询节奏与状态机**在 `pages/login/view.rs`：每 1.5 秒一次；`86101` 继续等、`86038` 过期自动
  重新取码并**退出旧循环**（避免两条轮询并存）、`code==0` 组装凭据；网络错误只记日志、下一轮重试。
- 码值翻译与 cookie→凭据是纯函数（`api/login.rs` 的 `QrPollState::from_code` /
  `auth_from_cookies`），单测覆盖——真机登录时才碰得到的分支，先在 CI 里锁住。

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

- 路径 **exe 同目录**的 `debug.log`（与 `auth.json`、`config.json` 同一层，用户不用找系统目录），
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
  格式化用 `api/mod.rs::format_seconds`（纯函数，已进单测；历史和详情都用它，所以放在 api 层公共处）。
  `view_at` 暂时不显示：本项目没有日期库，也不打算为一条次要文案引入 chrono——
  真要做的时候优先加依赖，不要手写日历换算。

### 4.12 详情页（P3）：栈式导航 + 四次请求

- **导航只有一个事件**：`pages/mod.rs` 里的 `OpenVideo { bvid }`。搜索结果的卡片、个人中心的
  "最近观看"、详情页的"相关推荐"发的都是它，外壳只订阅这一种信号——页面不知道自己会被压进栈里。
- **外壳里详情页是一个栈**（`detail_stack: Vec<Entity<DetailPage>>`）：打开就 push、返回就 pop，
  栈空才显示"搜索/我的"。这样"相关推荐 → 进去 → 再推荐 → 返回"是连续的。
  每压一层都给它订一条 `subscribe_in`（存进 `_subscriptions`，`Subscription` 一 drop 就退订，
  `gpui/src/subscription.rs:149`）；pop 掉的那层留下的订阅随实体释放自动失效。
- **点卡片这件事在 gpui 里也要先 `.id()`**：`on_click` 是 `Stateful<Div>` 的方法，
  和 `overflow_y_scroll()` 是同一类坑（第 2 节的清单）。链式顺序照 `gpui/src/window/prompts.rs:139-146`：
  样式 → `child(...)` → `.id(..)` → `.on_click(cx.listener(..))`。`ElementId` 有
  `From<String>`（`gpui/src/window.rs:7676`），所以 id 直接用 bvid/cid 拼出来的字符串。
  组件层不认页面：`video_card::clickable(data, on_open)` 收的就是 gpui 原样的
  `Fn(&ClickEvent, &mut Window, &mut App)`，调用方用 `cx.listener` 造它。
- **详情为什么不是一次 `detail()` 搞定**：`VideoDetail` 的 `Tags` 用的是
  `model::VideoTag`，它的 `tag_id: u64` 既不是 `Option` 也没有 `#[serde(default)]`
  （`ref/bpi-rs/src/video/model.rs:100-108`），而 B 站给 BGM/话题标签时常省掉 `tag_id`
  —— 一旦省掉，整个 `detail()` 连标题一起解析失败。所以拆成
  `view()`（主体，失败才算整页失败）+ `desc()` + `tags()`（返回的是另一个容错的
  `video::tags::VideoTag`）+ `related_videos()`，后三者各自失败只记一条说明。
- **字幕必须等主体**：`player_info_v2` 的入参要 `cid`，而 `cid` 来自 `view()` 的响应，
  所以这条依赖链是"主体回来后追一发"，不是并发。`PlayerInfoResponseData` 全无
  `#[serde(default)]`，失败概率不低，因此字幕只是详情页的一小块。
- **当前可用的字段就这些**：live 的 `VideoView` **没有** 简介/发布时间/分区/总时长
  （`model.rs:6-26`）；简介另问 `desc()`，总时长用 `pages[].duration`（数字秒）累加。
  `ref/bpi-rs/src/video/info/` 那一整块虽然有 `desc/pubdate/tname/duration`，但
  **没有任何方法返回它**（死代码）——编译得过、永远拿不到数据，别 import。
- **播放相关的准备**：选中哪个 P 记在 `selected`（cid 在 `PageInfo.cid`），起播就是拿这个 cid 去问播控。
  注意 `PlayUrlResponseData.timelength` 是**毫秒**，而 `VideoPage.duration` 是**秒**（第 4.13 节）。

### 4.13 起播（P4a）：取地址 + 拉起 mpv，先不遥控

三段各管一件事，`StreamChoice` 是它们之间唯一的交接物：

| 段 | 文件 | 只管 |
| --- | --- | --- |
| 取地址、选流 | `src/api/stream.rs` | playurl 请求参数、DASH/durl 两条路、清晰度中文档 |
| 拼参数、开进程 | `src/player/mod.rs` 的 `launch()` | 定位 `mpv.exe`（运行同目录）、请求头、`CREATE_NO_WINDOW`、stdout/stderr 一律 `null()`（第 4.3 节） |
| 发起、显示状态、收进程 | `src/pages/detail/view.rs` 的播放块 | 按钮/清晰度/`player_note`、`Child` 的生命周期（**只有"交给 mpv"这一句，没有进度/暂停控件**） |

- **fnval 是 `16 | 2048`（DASH + AV1）**：**没有单独的 HEVC 位**——开了 DASH 之后
  `dash.video[]` 把 HEVC/AVC/AV1 一起给，靠 `codecs` 前缀区分（选流规则见第 4.2 节）。
  4K/HDR/杜比那几位（128/64/256/512/1024）不开，再叠 `.fourk(false)`，与
  `MAX_QUALITY_ID = 116` 一起把画质封在 1080P。
- **请求的档 ≠ 拿到的档**：B 站按账号权限回落到实际能给的 `quality`，所以界面报的是
  选中那条流的 `video.id`（`StreamChoice.quality`），不是我们请求的 `want_qn`。
- **`dash` 与 `durl` 是两条不同的路**：`durl`（FLV/MP4 单文件）里已经含音频，
  此时 `audio_url = None`，`build_arguments` **不能**再加 `--audio-file=`——
  空值会让 mpv 直接报错退出。这条由单测 `single_file_playback_adds_no_audio_argument` 守住。
- **直链必须和头成对出现**：Referer + UA + **登录 Cookie**。bpi-rs 不会把它自己那份 cookie jar
  交出来（`jar` 是私有字段），所以由 `Auth::cookie_header()` 现拼一条
  `Cookie: DedeUserID=…; SESSDATA=…; bili_jct=…; buvid3=…` 塞进 `--http-header-fields`；
  缺它就是 403/-101，而症状与"地址过期"长得一样，所以 `player_note` 把档位/编码/轨数都写出来。
- **`Child` 的 drop 不杀子进程**（`std::process::Child` 文档明确）。"关掉详情页但声音还在放"
  是这类播放器最难查的 bug，所以两处显式 `kill()`：再次起播先收掉上一个实例，
  以及 `impl Drop for DetailPage` 在实体被弹出栈（或整个窗口关闭）时收掉。
  mpv 侧配 `--keep-open=yes`，播完不自己退，生命周期完全归我们。
- **mpv 走运行目录、不走 PATH**：`mpv_path()` = `<exe 同目录>/mpv.exe`（真机定稿，见第 4.3 节）。
  找不到就在 `player_note` 与日志里写明**该放哪儿的哪个文件名**，不 panic、不静默；
  搜索页底部状态行也把这个路径显示出来，省一轮来回问。
- **起播"没窗口"靠本机复跑那条命令行定位，不靠 mpv 的文本日志**：第十八轮的现场是 playurl 成功、
  `已拉起 mpv（pid …）`也写了，却查不到进程为什么秒退。当时加过一版"stderr 逐行进日志"，
  后来按用户定稿撤掉了（第 4.3 节：**文本输出与命名管道都不接**），而且 mpv 的消息默认写
  **stdout**（`common/msg.c:190-202`），只接 stderr 等于什么都没接。
  留住的手法是：拿真 mpv 在本机把同一套参数跑一遍，让它自己把 `Exiting... (Fatal error)` 喊出来。
- **换清晰度现在是"重开一个实例"**（当前做法）：重新 playurl + 重新 `launch`，
  代价是窗口回到开头。改成同实例 `loadfile` + `audio-remove`/`audio-add` 属于 P6，
  它要求把第 4.14 节那套管道的客户端重新写回来——这条**不是**播放期 UI（用户仍在客户端里点档位），
  所以不违反"播放归 mpv"的范围。先把按钮做出来是有意义的：它能立刻暴露"哪个档位其实取不到流"这类真机问题。
- **多条头只能拼成一个逗号分隔的参数**：`--http-header-fields` 是 string list 选项，
  mpv 按 `,` 切分（`ref/mpv/options/m_option.c:1403-1440` + `m_option.h:644`），
  而**命令行上重复给同一个列表选项是"替换"不是"追加"**（默认 `OP_NONE` 直接
  `VAL(dst) = res`）。所以 `build_arguments` 把 `Referer` 与 `Cookie` 用 `,` 连起来一次给出；
  代价是值里不能有裸逗号——对着现场凭据核过，四个 cookie 字段（含 222 字符的 `SESSDATA`）
  都不含逗号与空格，拼接是安全的。
- **结果只认最近一次点击的 cid**：`planning_cid` 守卫——用户点了 P3 又点 P5，
  P3 的地址回来时直接丢，否则会起一个播错分 P 的窗口。

### 4.14 【技术档案，代码已删】mpv 命名管道遥控：机制核实过，等 P6 再用

**这一节写的东西当前不存在于代码里。**第二十二轮我按"遥控"做了 `player/ipc.rs` + `player/state.rs`
（连管道、订阅属性、进度行、暂停/±10 秒），第二十三轮 57 passed 全绿、真机也能用，
但用户定稿范围是"**视频播放相关的交给 MPV，rustBiliPC 做播放之外**"（第 4.3 节），
所以这套 UI 与客户端已整体删除，tokio 的 `net` feature 也一并收回。
留下这一节，是因为里面每一条都是**下过源码/文档核实**的：P6 要做"同实例换清晰度"和
`sub-add` 时必须连这条管道，重造机制等于把这轮学费再交一遍。

- **一条连接同时负责读与写**：`observe_property` 的注册是按连接算的，连接一断注册即失效
  （`ref/mpv/DOCS/man/ipc.rst:280-289` 的 warning，还专门说"用两次 socat 分别收发"会看起来不工作）。
  订阅、命令、事件全走同一条管道，且这条连接必须一直开着。
- **为什么用 tokio 而不是 std**：mpv 文档明说 Windows 上的客户端要"同时读写这条管道"必须用
  overlapped I/O（`DOCS/man/ipc.rst:69-73`）。std 的同步句柄做不到——同一个句柄上阻塞读会把写
  一起挡住。`tokio::net::windows::named_pipe` 的 `try_read`/`try_write`/`ready` **都取 `&self`**（docs.rs 核过），
  所以一个任务里就能读写交错，overlapped 的细节在 tokio 内部处理。要恢复就先给 tokio 加回 `net`
  （`mio`/`socket2`/`windows-sys` 本来就在依赖图里，Cargo.lock 不用动）。
- **pump 任务的形状**：一个循环，`select!` 同时等"管道可读"与"界面塞了新命令"；读到的字节攒进
  `incoming` 按 `\n` 切行、逐行解析，待发的字节攒进 `pending` 用 `try_write` 抽干
  （一次写不完就下轮接着写，见下面两条坑）。没待发时**故意只等 `READABLE`**：
  连着等 `WRITABLE` 会被"一直可写"叫醒空转。
- **两个把自己转死的坑**（都是当场抓出来的，不是编译器给的）：
  `try_write` 返回 `Ok(0)` 时必须退出那一轮，否则在共享 runtime 线程上**死转**；
  已写出的那一段用 `pending = pending.split_off(written)` 赋值丢弃，
  **没有 `Vec::replace_range` 这个东西**（`replace_range` 是 `String` 的；Vec 那个来自 `bstr::ByteVec`），
  第二十/二十一轮的 E0308、E0599 就分别是这两处。
- **建连要重试**：mpv 建管道晚于我们 `spawn()` 返回，`ClientOptions::open`（底层 `CreateFileW`）
  在服务端还没实例时**立即返回错误**而不是阻塞，所以重试 40 次 ×120 ms。
- **状态只信 mpv 报回来的**：按暂停时**不**本地翻转 `paused`，等 `property-change` 回来再改——
  本地猜一次，命令失败时就留下一个和实际不符的按钮。`time-pos`/`duration` 可能是 `null`
  （流还没载入），所以一律 `Option`，没进度就写 `--:--`，**不编一个 `00:00` 出来**
  （"还没开始"与"从 0 秒开始"是两件事）。
- **管道名带实例序号**：再次起播时先 `kill` 上一个 mpv，但那只等十几毫秒，上一个实例可能还持有
  同名管道句柄。mpv 建管道用的是 `PIPE_UNLIMITED_INSTANCES`（`ref/mpv/input/ipc-win.c:387`），
  同名不报错，后果是**两个 mpv 都在听**、命令发给任意一个——"按了暂停的是上一个窗口"。
- **`loadfile` 的回复早于真正切完文件**（`DOCS/man/input.rst:542-548`：替换当前文件的 `loadfile`
  会在当前文件还没停、新文件还没开始加载时就返回），所以**不能拿它的回包判断"流开没开成"**；
  P6 的同实例换清晰度要靠 `idle-active`/`eof-reached` 属性或 `file-loaded` 事件确认。
- 属性名与命令形状都对着源码核过：`duration`/`time-pos`/`eof-reached`/`paused-for-cache`/`idle-active`
  在 `ref/mpv/player/command.c:4697-4743`，`seek <target> [relative]` 在 `input.rst:300-308`
  （默认就是 relative，仍要写明），`loadfile`/`audio-add` 在 `input.rst:542/767`。

### 4.15 视觉（P5）：照 `ref/Bili.Copilot` 的度量重做，不是凭感觉调

起因是用户一句"我说了 GUI 样式要参考 ref/Bili.Copilot，你这个太丑了"。
"参考"不能是形容词，所以这一轮的做法是：**先把参考项目的度量从 XAML 里抄成数字**，
再让代码用这些数字。下面每一条后面就是它来自哪个文件哪几行。

- **两层结构**：`ui/palette.rs` 只放**颜色与尺寸常量**（唯一出处），`ui/layout.rs` 只放
  **由这些常量组成的复用件**（页头、分隔线、侧栏项、头像、统计行、提示、卡片壳）。
  页面不许再自己写 `rgba(0xFFFFFF99)` 或 `px(56.)`；这也是"改一处就全体一致"的前提。
- **`rgba(hex)` 是 RRGGBBAA**（`ref/gpui/gpui/src/color.rs:20-23`），不是 CSS 的 RGBA。
  这轮差点把透明度写成前两位：`card_background = rgba(0xFFFFFF10)` 才是"白 6% 不透明"。
- **色板**：主色 `#FB7299`（B 站粉），页面底 `#202020`，卡片底白 6%、抬起白 10%、hover 白 14%，
  分隔线白 8%，文字主/次/三 `E6`/`99`/`68`，徽章黑 60%，危险色 `#F25C6C`。
- **视频卡 232×238**：`App.xaml:70-76` 的 `VideoLayout` 是 `MinItemWidth=232` /
  `MinItemHeight=238` / 列距行距各 20；`VideoCardControl.xaml:16-19` 那一行是"封面 `*` + 文字区 116"，
  即封面 232×122（≈1.90:1，**不是 16:9**）；`:21` 圆角只有上面两个角（`4,4,0,0`）；
  `:53-54` 文字区内边距 16,12,16,16、行距 8；`:38-45` 时长徽章是外边距 8、内边距 4 的黑 60%（`Opacity=0.6`）；
  `:60-64` 与 `:84-101` 的 UP 主、播放量都是 12px Tertiary。
  网格靠 `flex_wrap` + `gap(20)`，卡片一律 `flex_shrink_0`，否则窄窗口下会被压扁。
- **文字行的顺序也是抄的**：三行是 `Auto`/`*`/`Auto`，**UP 主在最上、标题居中占满、统计在底**
  （`VideoCardControl.xaml:53-64`）。标题放最上面是我上一版的直觉排法，一眼就不像参考项目。
- **侧栏横卡**：`VideoCardControl.xaml:1057-1087` 的 `PlayerRecommendVideoCardStyle`——整卡高 **104**、
  缩略图列宽 **148**、封面在列内再 `Margin=6`（于是图 136×92）、圆角 4、
  文字列 `Padding=12,4,12,8`、行距 **6**，同样是"UP 主/标题/统计"三行。
- **骨架尺寸**：标题栏 48（`RootLayout.xaml:33`）、页头 56 + 1px 分隔线
  （`PopularPage.xaml:18` 与 `:30-33`）、侧栏 240（`ViewModels/LayoutPageViewModelBase.cs:41`）、
  详情页右栏 360（`VideoPlayerPanel.xaml:11`）、搜索框 320–500（`RootLayout.xaml:74-80`，取了 460）、
  列表行 `MinHeight=40` + `Padding=12,8` + 列距 8 + **每行都带卡片底色**（`VideoPartsSection.xaml:18-21`）、
  行间距 8（同文件 `:51`）、UP 头像 44（`UpSectionControl.xaml:28-31`，`CornerRadius=22`）、
  账号头像 48（`AccountCard.xaml:20-26`，`CornerRadius=24`、`DecodeWidth=100`）。
- **选中态：边框平时就在，只是透明**。WinUI 的 `ItemContainer` 用 `PART_SelectionVisual`
  （`Overrides.xaml:190-197`：`BorderThickness=2` + `BorderBrush=Transparent` + `Opacity=0`），
  选中时才把 brush 换成选中色（`:233-247`）。所以这里也是 `border_2()` 常驻 +
  `border_color(选中 ? accent : 透明)`——**只在选中那项加边框会让整列每行抖 2px**，比丑更难受。
- **hover 只换底色**：参考项目的卡片 hover 没有位移、没有阴影、没有缩放，所以这里也只是
  `card_background` → `card_background_hover`。要 hover 生效得把 `bg` 和 `cursor_pointer`
  放在**同一个 div** 上（`hover` 定义在 `InteractiveElement`，`elements/div.rs:842`，
  而 `InteractiveElement` 同时给 `Div` 与 `Stateful<Div>`：`:1877` 与 `:4087`）。
- **登录页一起改了**：它是用户看到的第一个画面，`rgb(0x2A2A2A)` 那种散色也收进了 palette，
  否则"登录成功后窗口闪一下换个世界"。
- **API 坑（这轮踩实的四条）**：`FluentBuilder` 的 `when/when_some` **不在 kit 的 re-export 里**
  （`gpui_kit` 只 `pub use ::gpui::*`，没有 prelude glob），要单独 `use gpui_kit::prelude::FluentBuilder;`；
  `on_click`/`overflow_y_scroll`/`hover` 都要求先 `.id(...)`，而 `ElementId` 有 `From<String>`
  （`window.rs:7676`）；`text_size`/`line_height` 收 `impl Into<AbsoluteLength/DefiniteLength>`，
  所以 `px(FONT_BODY)` 而不是裸数字；`cx.listener(...)` 返回的就是
  `impl Fn(&E, &mut Window, &mut App) + 'static`（`app/context.rs:246`），
  所以组件层可以把自己参数写成这个形状、由页面传 `cx.listener` 进来。
- **还有一条是关于"怎么查 API"的**：`rounded`/`border_2`/`gap`/`min_h`/`h_full`/`p_4` 这些
  **在 `styled.rs` 里 grep 不到**——它们是 `gpui-pre-macros` 的 `styles.rs` 按
  "前缀表 × 后缀表"生成的（`border`+`_2`、`w`+`_full`…）。上一轮我差点因为"源码里没有"
  就判定这些方法不存在；正确做法是把 `gpui-pre-macros-0.3.9.crate` 下下来读
  `border_prefixes()/box_prefixes()/corner_prefixes()` 那几张表。
  `From<Rgba> for Hsla`（`color.rs:677`）与 `From<Rgba> for Background`（`:960`）也核过了，
  所以 palette 统一返回 `Rgba` 是安全的。
- **不模仿的部分**：Acrylic/Mica 毛玻璃、`ItemsView`、icon font。gpui 没有等价物，
  硬做会得到"像但更糟"的结果（风险表第 6 条）。`ItemContainer` 的**选中视觉**这次是照做了的
  （上面那条），剩下的差异是它带圆角动画，我们没有。
- **kit 组件的颜色不读我们的 palette，读全局主题**（2026-10-25 用户指出"右上角搜索框还是白底黑字"）：
  `gpui_kit::init` 里写死了 `Theme::change(ThemeMode::Light, …)`（gpui-component 0.8.0
  `src/theme/mod.rs:40`），所以 Input/Button/滚动条一律按亮色画，而我们的页面 div 是硬编码暗色——
  于是"页面黑、控件白"。修法是**在 `theme::apply` 里先切暗色再覆盖语义色**，
  不是给某个控件单独打补丁（打补丁只修好这一个控件，下一个组件又是白的）。顺序不能反：
  `Theme::change` 会整体换上该模式的调色板。这一层现在映射了
  `background/foreground/border/input/muted_foreground/accent(±foreground)/ring/button*/list_hover/scrollbar*`
  以及 `radius=4`、`font_size=14`（WinUI 的 `ControlCornerRadius` 与 Body 字号）。
  两个容易踩的点：**主按钮读的是 `button_primary` 而不是 `accent`**
  （gpui-component 0.8.0 `src/button/button.rs:1002`），只改 accent 主按钮还是白的；
  暗色下输入框的底是 `input` 混 30% 透明来的（`theme/mod.rs:485` 的 `input_background()`），
  所以"黑底"这件事是靠 `colors.input` 控制的。
- **字重也归这一套**：参考项目"要强一档的字"用 `*Strong` 样式（`AccountCard.xaml:44` 的
  `BodyStrongTextBlockStyle`），这里对应 `font_weight(FontWeight::SEMIBOLD)`（600），
  用在页头标题、侧栏应用名、资料卡昵称上；正文与卡片标题不加粗。
- **本轮的验收方式改不了**：本地没有工具链，我只能保证**数字来自 XAML、方法名来自宏的生成表**，
  不能保证"看着对"。所以产物出来后请他重点看四件事：卡片网格的节奏与留白、
  卡片文字三行的顺序（UP 主在上）、页头/侧栏的比例、详情页右栏的高度分配。
  视觉问题要靠眼睛，不靠编译器。

### 4.16 主页（P6 第一步）：首页推荐流当默认页，"换一批"而不是分页

接口与字段口径全部核在 `docs/api-notes.md` 第 10 节，这里只记结构性决定。

- **为什么它当默认页**：用户清单里"主页"排第一，登录后第一眼是一片空白不对；
  所以 `MainShell::new` 里 `tab = ShellTab::Home`，并**在构造末尾就把第一批推荐取回来**
  （`home.update(cx, |page, cx| page.ensure_loaded(cx))`）。
  "我的"仍然懒加载（用户可能根本不点），差别就在这里：默认页不该懒。
- **内容选的是"首页推荐"而不是"热门"**：`video().homepage_recommendations()` 在**已启用的 `video`
  feature** 里、WBI 由库里签（`ref/bpi-rs/src/video/client.rs:218`）、载荷是**typed** 的 `RcmdItem`。
  而 `video_ranking().popular_list()` 属于另一个 feature，且它的 `list` 是 `Vec<serde_json::Value>`
  （`src/video_ranking/popular.rs:9-15`），得我们自己挖字段。少加一个 feature、少写一堆 Value 解析，
  是当前更稳的一条。分区侧栏（Bili.Copilot 那种"分区 + 该区内容"）**没做**：它要先定侧栏怎么摆、
  点分区走哪条接口，是结构问题，等他拍板。
- **只有 `goto == "av"` 能进卡片**：推荐流混着 `live`/`ogv` 项，它们的 `bvid` 不是稿件号，
  做成可点卡片等于"点了必然报错"。过滤规则单独做成 `is_archive_goto` 并进单测。
- **分页做成"换一批"**：这个接口没有 `num_pages`（`recommend.rs:165-172` 只有 `item/mid/两个百分比`），
  所以页头右侧是一颗"换一批"按钮（对应参考项目 `PopularMainHeader.xaml:38-49` 那颗"刷新"位），
  批次号 `page` 同时喂给 `fresh_idx` 与 `fetch_row`。**这条口径没有官方文档**，
  真机看"换一批是否真的换出不同内容"，不行就改成只递增 `fresh_idx`。
- **载荷是严格结构，缺字段就整批失败**：`RcmdItem` 除四个 `Option` 外都没有 `#[serde(default)]`，
  `preload_expose_pct` 这类外层字段也是必填。风险与收藏/历史同一类（第 6 节第 12 条），
  处置也一样：**原因写在这一页的提示位**，不许只进日志；已有列表 + 新失败时两者都显示，
  不清空列表藏起来。
- **时长单位是秒**：`RcmdItem.duration: u64` 是数字秒（搜索接口给的是 `"mm:ss"` 字符串），
  所以走 `api::format_seconds(seconds_to_u32(..))`，与历史/详情同一口径。
- **卡片第三格给"推荐理由"**：`rcmd_reason.content`（B 站的 0 无 / 1 已关注 / 3 高点赞量）
  填进 `VideoCardData.extra`，空串时那一格自动不出现（`video_card` 里用 `when` 控制）。

### 4.17 评论区（P6b）：挂在主列最下面，翻页失败不许把已读到的评论抹掉

接口证据在 `docs/api-notes.md` 第 11 节，这里记结构性决定。

- **为什么在详情页里、而不是单独一页**：评论区的 `oid` 是 **aid**，必须先有主体响应才有它
  （`api/comments.rs` 文件头），所以它的生命周期天然属于详情页；参考项目把它做成播放器窗口左侧的
  页签（`VideoPlayerPanel`），而页签控件在 gpui 侧没有等价物，我们把它排成主列最下面的一节。
- **它排在字幕之后**：`content()` 的最后一节是"评论 · 第 N 页"，顺序是
  信息 → 播放块 → 标签/简介 → 字幕 → 评论（"读完简介往下翻"的自然位置）。
- **类型代码 1 不是 11**：`bilibili-API-collect` 的类型代码表里 **1=视频、11=相簿、12=专栏、17=动态**，
  网上大量帖子把"视频"写成 11 是混了编号；bpi-rs 自己的 `CommentType::Video = 1`
  （`src/comment/action.rs:22-27`）和它 live 测试用的 `TEST_TYPE = 1`（`src/comment/list.rs:322`）
  相互印证。写错的症状是"评论永远是空的"而不是报错，所以钉了单测
  `video_comment_target_type_is_one`（另见风险表第 19 条）。
- **每页 20 条是库的上限**：`with_page_size` 把 `ps` 卡在 1–20（`src/comment/list.rs:94-101`，
  接口本身允许到 49）。
- **"还有下一页"有两种口径**：有 `page.count` 就按总数算；没给（那一层是 `Option`）就退成
  "这一页取满了 20 条就认为还有"。两者都写在 `CommentPage::has_more` 里并进单测——
  按钮点下去没反应比转圈更难查。
- **时间只做减法**：接口只有 `ctime`（unix 秒），相对时间（"3天前"）与时区无关，
  所以既不引 `chrono`、也不手搓公历换算；参考项目显示的也是 `RelativeTime`
  （`CommentItemControl.xaml:97-104`），绝对时间只进 ToolTip。
  `relative_time(ctime, now)` 收两个参数就是为了单测，读时钟只发生在 `fetch_comments` 里。
- **失败保留旧内容**：翻页或换排序失败时，`comment` 里还是上一批，提示行按错误色显示；
  只有首批失败或"这条稿件没有评论"才只剩一行说明。
- **一行评论的度量**：整行卡片底 + 1px **同色**描边、`Padding=12`、头像列与文字列距 12、
  头像 **40**（`CornerRadius=20`、`DecodeWidth=80`）、行距 8、行间 `Margin=0,0,0,4`
  （`CommentItemControl.xaml:21-46` 与 `:76-118`）。昵称走 SEMIBOLD（那边写的是 `FontWeight=Bold`）。
- **没接的东西**：楼中楼展开（`comment().replies()`，证据在 api-notes 第 11 节末尾）、
  点赞与发评论（要 csrf，且属于写操作）。展开子回复要先定"插在哪"这个结构问题，不在这一轮顺手加。

### 4.18 用户空间（P7）：点详情页的 UP 主就压一层空间页

接口与行号在 `docs/api-notes.md` 第 12 节，这里记结构与口径。

- **入口**：详情页 UP 主那一行整行可点（`div().id("detail-open-up").on_click(..).hover(..)`），
  发的是新事件 [`OpenUser{mid,name}`](../src/pages/mod.rs)。带 `name` 是因为资料卡要一次请求才回来，
  没有它的话那一瞬间标题只能空着。参考项目里这个位置也是按钮
  （`UpSectionControl` 的头像与昵称都挂 `ShowUserSpaceCommand`）。
- **栈改成一个枚举而不是两个栈**：`StackPage::{Detail, Space}`。
  从 P7 起下钻是交错的（详情 → 空间 → 点卡片又进详情 → 再点 UP 主…），
  两个栈的话"返回上一层"就不知道该弹哪一个。`MainShell::watch` 对泛型 `T` 提
  `EventEmitter<OpenVideo> + EventEmitter<OpenUser>` 两个约束，新压进来的一页两条订阅都装上。
- **三块内容的分页口径不一样，所以各一个序号**：投稿是页码（`pn`/`ps` + 总数，界面给"上一页/下一页"）、
  图文是 **offset 游标**（只能往后，界面是"加载更多"并累加）、收藏一次给全。
  共用一个 `seq` 会出现"图文在途时点了投稿的下一页，两边互相把对方的结果判成过期"，
  所以 `video_seq / post_seq / folder_seq` 各一套，提示位也各一套（与第 4.11 节同一个道理）。
- **标签（分区）筛选不额外打接口**：投稿响应里 `list.tlist` 就是"这个 UP 用过的分区 + 条数"，
  第一页回来就画成 chips，点一个带 `tid` 重取第 1 页。"全部"那一档是我们自己加的（`tid = 0`）。
  两个口径要分开：**chips 上"全部"的数字是 `tlist` 各分区之和**（`tlist` 的计数与筛选无关，
  用 `page.count` 的话点进分区后会出现"全部 12 / 游戏 12 / 单机 3"这种自相矛盾的一排）；
  **页头右侧与翻页用的是 `page.count`**，因为那是本页真正的总数，文字因此写成"当前分区 N 个投稿"。
  `tlist` 在库里是 `serde_json::Value`，**没有类型可跟**，所以
  [`parse_tlist`](../src/api/space.rs) 只认"`{tid: {name,count}}`"，认不出来就少一个 chip，
  绝不让整块投稿失败——这条有单测（含畸形输入与"键当 tid"的回退）。
- **chips 排序是必要的，不是美观**：`serde_json` 的对象不保序，不排序的话每次刷新 chips 都会换位置，
  用户刚点过的那一格就跑到了别处。
- **图文条目只有"正文 + 封面 + 点赞"**：`SpaceItem` 不给标题也不给发布时间，`stat.like` 已经是字符串
  （`ref/bpi-rs/src/opus/space.rs:29-40`）。所以另给了一个 [`video_card::post_card`](../src/ui/video_card.rs)，
  不硬套视频卡那三行——套上去会留下两个空档，看着像坏了。封面是**按接口给的宽高比**摆的
  （[`space::cover_ratio`](../src/api/space.rs)：图文有横有竖，写死 16:9 会把竖图裁掉半张脸；
  没给封面就整档不占位）。
- **这一轮没做**：收藏夹**只列不钻**（点进去看内容是 `fav().list_detail`，我们已经在"我的"那页用过，
  差的是"再压一层什么页"这个结构决定）；关注/取关是写操作，不在客户端清单里。

### 4.19 登录改成独立小窗口：两条不能反的顺序（2026-10-26）

用户定稿：「登录单独做一个窗口……启动时若没有 auth.json，先弹窗让用户扫码登录，
注意这个扫码登录的窗口做小一点，只要能显示 rustBiliPC 的 logo 和登录二维码即可」。
形状：320×420、`is_resizable: false`、保留系统标题栏（无边框要自己处理拖动与关闭按钮，
为一个二维码引一套 `TitleBar` 不值得）；页面上只剩**应用名 + 二维码 + 一行状态 + 重新获取按钮**，
原来那两行说明（解锁 1080P、日志路径）收掉了——日志路径现在写在设置页。

换窗口有两条**顺序性**约束，写反就是「点完登录程序自己没了」：

1. **先开主窗口，再关登录窗口**。gpui 在 Windows 上的默认 `QuitMode` 是最后一个窗口关闭就退进程
   （`ref/gpui/gpui/src/app.rs:458` 的 `Default` 取 `cfg!(not(target_os = "macos"))`，
   判定在 `:3180-3188`）。反过来的话中间出现一个零窗口的瞬间，进程当场退出。
2. **不要在别的窗口的 update 里开关窗口**。`App::open_window` 会强制画一帧才返回（`app.rs:1520`），
   而 `LoginSucceeded` 的事件回调正跑在登录窗自己的一次更新里。所以
   [`LoginGate`](../src/pages/gate.rs) 只 `cx.spawn` 一个任务，在任务里
   `AsyncApp::update(|app| …)` 再开关（kit 文档同一姿势：`crates/kit/src/lib.rs:180`）。

`Subscription` 的归属也是这一节决定的：原本想让 `main.rs` 在应用层订阅登录事件，
但 run 闭包里的局部 `Subscription` 一析构就取消订阅（`subscription.rs:147-152`），
所以订阅挂在登录窗的真实视图 `LoginGate` 的字段上——与本项目「订阅跟着实体走」的既有做法一致。

### 4.20 侧栏：80 宽的七个入口 + 头像占住左上角（2026-10-26）

用户四条要求逐条落地：

| 要求 | 落点 |
| --- | --- |
| 不要「我的」这一格 | `pages/mine/` 删除，拆成 `favorites/`、`history/`，新增 `toview/`、`download/`、`about/`；`ShellTab` 从 3 个变 7 个 |
| 栏宽收到 1/3 | `palette::SIDE_COLUMN_WIDTH` 240 → **80**（数是怎么反推的写在 `palette.rs` 那条注释里，**不是抄来的**） |
| 左上角换成登录账号头像、不做菜单 | 48 那一格（`TITLE_BAR_HEIGHT`）里摆 44 的圆头像（`AVATAR_UP`），**不可点**；头像来自外壳自己那一路 `profile::fetch_profile` |
| 左下角 mpv 信息删除 | `status_note` 整块删掉；**信息没丢**，挪进设置页（播放内核那一行连路径一起给） |

导航项的形状改成「Lucide 图标在上、两字在下」（`layout::side_item`，高 48 =
上下各 8 + 图标 16 + 间距 4 + 12 号字，全是既有常量）。图标用**完整目录**
`gpui_kit::assets::IconName`，因此 `main.rs` 的 `with_assets` 从 kit 默认的 `Assets`
换成 `AllAssets`：默认那份只嵌 106 个 svg，`House`/`Clock`/`Bookmark`/`Download` 都不在，
而**名字写错既不编译失败也不 panic，只画出一片空白**（`component/src/svg.rs:166-171`
`log_err()` 后直接 return）——这是这一节最需要记住的坑。

「← 返回上一层」缩成「返回」：80 宽放不下那句，含义没变（还是弹下钻栈的一层）。

### 4.21 落盘改名：`auth.json` 与 `config.json`（2026-10-26）

- **凭据**：`store/credentials.rs` → `store/auth.rs`，类型 `SavedCredential` → `Auth`，
  路径 `<exe>/data/cookies.json` → `<exe>/auth.json`。**旧文件不删**：`Auth::load()` 只在
  `auth.json` 读不到时去读它一次，字段合格才迁移并写新文件；不合格就当没登录。
  这样他原来登录好的现场不会因为我改路径就掉登录态，而删用户文件这件事是不可逆的，我们不做。
- **设置**：新增 `store/config.rs`，`<exe>/config.json`，**只有 `max_quality` 一个字段**，
  默认与硬上限都是 `stream_select::MAX_QUALITY_ID`(116)，下限 16=360P，超出范围的写法由
  `clamp_quality` 收回并留一条日志（不静默改用户的值）。它**真的被读走**：
  `stream::request_qn()` 起播时取它，同一个数既当请求 `qn` 又当选流封顶（`pick_video(streams, cap)`
  因此从常量改成参数）。设置页只把它显示出来，**不做假开关**（用户定稿「先只做关于页」）。
- **为什么 `max_quality` 做成进程级快照**：`app::init_config` 在 `main.rs` 装一次，
  各处 `app::config()` 只读。传 `Arc<Config>` 一路要动七个页面的构造函数；
  而**纯函数仍然收显式参数**（`is_quality_allowed(id, cap)`），单测因此不碰全局状态。
- 明文落盘的选择没变（风险与理由见 `store/auth.rs` 文件头），只是位置与文件名变了。

### 4.22 收藏夹下钻与三种搜索（P9，2026-10-26）

同一句需求里的三个能力各走各的路子，差别来自接口给不给得起：

| 能力 | 做法 | 依据 |
| --- | --- | --- |
| 展开 | 点收藏列表的一行发 `OpenFolder{id,title}` → 外壳把 `FavFolderPage` 压进**同一条下钻栈**（第三种 `StackPage`），"返回"逐层退 | 用户定稿"下钻一层页"；与详情/空间同一条路径，不再造第二套返回逻辑 |
| 收藏内容搜索 | **服务端** `keyword`（`fav/list.rs:140`/`:196`）：输入框只认回车与"搜索"按钮，**不认 `Change`** | 夹子可能上百条，只有这条接口给得起关键词 |
| 历史/待看搜索 | **本地过滤**（`pages::matches_keyword`：标题或 UP 主，忽略大小写，空串一律放行） | 历史与待看**都没有** `keyword` 参数（api-notes 第 14 节的三条 grep 结论） |

三条定稿细节：

- **`Change` 与 `InputEvent::PressEnter` 的分工是刻意的**：本地过滤不花钱，敲一个字就该少几条；
  服务端搜索跟着 `Change` 跑就是每敲一个字打一次接口。所以同一层里两种页用同一个复用件
  （`layout::search_box`）但订阅不同的事件，这个差别写进两个页面的文件头。
- **历史"再加载 30 条"用游标三件套**（`with_max`/`with_view_at`/`with_raw_business`），
  "翻到头"的判据用**我们自己的**"这次没拿到条目就停按钮"，不看接口回的 `cursor`——
  那条 cursor 就算 `list` 为空也照样回，照它判断会出现"点了再加载、什么也没多、按钮也不消失"的死圈。
- **不猜 `FavListMedia.type` 的取值**：库里没有这个字段的枚举或常量（自带的 `"type": 2` 是夹子 info 的），
  所以准入条件是"拿不拿得到稿件号"（`bvid`/`bv_id` 两个键，`pick_bvid` 有单测），
  跳过的行数在页头写成"另有 M 条打不开"。**这一条是"不编造解释"的原则落地**：
  与其赌一个数字然后悄悄少列几类内容，不如按"能不能点开"筛并把差额说出来。
- 收藏列表页与内容页的**行/卡片都是交互元素**，所以新增了 `layout::clickable_row`
  （与 `list_row` 共用同一组度量常量，差别只在 `id`/`on_click`/hover：gpui 的 hover 只存在于 `Stateful<Div>`）。

### 4.23 触摸屏：单指滚动与侧栏换页（P10，2026-10-26）

**这一版没有任何"触摸事件"可用**。核实结果（`ref/gpui/gpui_windows/src` 全文 grep）：
没有 `WM_TOUCH`、没有 `WM_POINTERDOWN/UP/UPDATE`、没有 `RegisterPointerInputTarget`/`EnableMouseInPointer`，
窗口过程只处理 `WM_MOUSEMOVE`（`events.rs:112`）、`WM_LBUTTONDOWN/UP`（`:136/:142`）、`WM_MOUSEWHEEL`（`:148`），
其余交给 `DefWindowProcW`。gpui 核心的触摸通路（`TouchEvent`/`TouchPhase`/`dispatch_touch_event`
`window.rs:5810`、pan→`ScrollWheelEvent` `:5851`）都在，但喂它的是 macOS 后端，
Windows 上这条管线**永久闲置**（`PlatformGestures` 的默认实现是 `None`，`platform.rs:484`）。

**那手指为什么还能动**：Windows 会把触点注入成鼠标消息——按住拖动是一条带 `MK_LBUTTON` 的
`WM_MOUSEMOVE`，落到 gpui 是 `MouseMoveEvent { pressed_button: Some(Left), .. }` 且
`event.dragging()` 为真。所以"单指上下滚"只能做成**按住把内容拖着走**（`ui/scroll.rs`），
顺带给了鼠标用户"按住左键拖列表"。三个必须记住的实现事实：

1. **偏移要走 `ScrollHandle`**：`.track_scroll(&handle)` 之后 div 的 paint 偏移与句柄是同一个
   `Rc<RefCell<..>>`；`ScrollHandle` 上**没有** `scroll_by`（那个只在 `ListState` 上），只能
   `set_offset`；`offset()` 是负值、顶是 `0`、底是 `0 - max_offset()`。方向与夹取抽成
   纯函数 `target_offset` 进单测——这两处写反都不会编译失败。
2. **监听每帧重登记**：`Window::on_mouse_event` 的文档原文是"register … for the next frame …
   the listener will be cleared"（`window.rs:5427-5445`），所以 `scroll.watch(window, view)`
   必须在 `render` 里调；挪进 `new()` 只有头一帧收得到移动。改完偏移用 `cx.notify(entity_id)`
   催重绘，与 kit 滚动条的 `notify_drag` 同一做法（`base/src/scrollbar.rs:517-533`）。
3. **移动里 `cx.stop_propagation()`**：不停会顺带开始选中文本，看到的是"拖出一段高亮"而不是内容在动
   （kit 滚动条在同一处也停）。按下只记起点、不吞事件，所以点击与按钮照旧；
   从一张卡片起步拖到别处也不会误触——gpui 的点击要求同一个元素自己的 `pending_mouse_down` 配对
   （`elements/div.rs:3609-3622`）。

**已知做不到的**：手指按下后**没有 hover**（触摸不产生"悬停"），所以凡是只靠 hover 表达的层级
（例如导航项 hover 加深底色）在触摸屏上看不见——选中态本来就另有底色+边框，不影响可用性。
`ScrollBounce`（kit 的橡皮筋回弹）在 Windows 上是关的（`base/src/scroll_bounce.rs:172` 的
`enabled_by_default()` 只在 ios/android/macos 为真），所以拖到边界是硬停，不回弹。

**侧栏换页必须先清栈**（同一轮修的缺陷）：上一版 `select_tab` 只改 `tab`，而渲染规则是
"栈非空就显示栈顶"，于是人在详情页/空间页/收藏夹内容里点侧栏**看起来没反应**，只能先点"返回"。
现在 `select_tab` 先 `stack.clear()` 再切页——"返回"退回只负责逐层退。代价写进代码注释了：
清栈会 drop 详情页，而它在 `Drop` 里 kill 自己拉起的 mpv，所以**看着视频点侧栏换页会关掉那个实例**
（与一路点"返回"退到空是同一行为，要不要改成"换页不动播放"是另一个决定）。

### 4.24 下载：双文件、每次弹框选目录、可取消（P11，2026-10-26）

**用户的口子只开了一个**："下载功能要做一下，**仅这个能下载到外部文件里**"。
其余落盘口径（`debug.log`/`auth.json`/`config.json` 在 exe 同目录）一个字没动。

**四条已定稿的形态**（AskUserQuestion 当场选的）：文件形态=**双文件 + 高画质**、
保存位置=**每次弹框选目录**、多分 P=**只下当前 P**。第三条推论是"没有默认目录、
没有 `config.json.download_dir`、运行目录里也不开 `downloads/`"——目录只来自那一次弹框。

**弹框是真的**（不是自绘列表）：`App::prompt_for_paths(PathPromptOptions{directories:true,
files:false, multiple:false, prompt:Some("存到这里")})` → Windows 后端用 `IFileOpenDialog`
+ `FOS_PICKFOLDERS`（`ref/gpui/gpui_windows/src/platform.rs:804` 与 `:1531`）。
`files` 与 `directories` **不能同时为真**：`can_select_mixed_files_and_dirs()` 在 Windows 返回
`false`（`:833`）。返回的是 futures oneshot 的 `Receiver`，所以要在 `cx.spawn` 里等
（`kit/crates/story/src/stories/data_table_story.rs:1210` 是这个用法的现场样本）；
用户按取消拿到 `None`，界面写"没有选目录，这次下载没开始"，不当错误处理。

**取址复用播放那条路**：详情页 `PlanPurpose::{Play, Download}` 跟着请求走，
结果回来时 `planning != Some((cid, purpose))` 就丢弃——"换了 P 又点下载"不会用到上一个 P 的地址。
但**下载每次都重新取一次地址**，不复用手里那份 `plan`：直链有时效（库里那条注释是 120 分钟，
`ref/bpi-rs/src/models/stream.rs:202-215`），"播了一会儿再想起下载"完全可能已经过期，
而过期的症状是下到一半变 403/404，比多等 200 ms 难查得多。
下载与播放共用 `stream::request_qn()`（= `config.json` 的封顶），所以"下到的画质"与"刚才播的画质"
同一口径。前提是先修掉那条缺陷：**清晰度按钮以前给的是 `support_formats` 全集**，
点 4K 会把我们自己的 1080P 上限顶穿（现在 `quality_options(.., cap)` 过滤，见第 8 节该轮记录）。

**字节是谁读的**：bpi-rs 每个接口都是 `response.bytes().await`（整个响应进内存），
一部片子不能这么读；库里的 `stream` feature 也没把流暴露出来。所以这一条走 gpui 的传输抽象：
`App::http_client()`（我们在 `main.rs` 装的 `reqwest_client::ReqwestClient`，与封面图共用一份）
→ `HttpClient::send(http::Request<AsyncBody>)` → `Response<AsyncBody>`，
`AsyncBody` 实现 `futures::AsyncRead`（`gpui-pre-http-client 0.3.9` 的 `async_body.rs:113`），
于是可以 256 KiB 一块地 `read` 进 `BufWriter<File>`。**`http` 不用加进 `Cargo.toml`**：
`gpui::http_client` 自己 `pub use http::{self, ...}`；`futures` 只多一条依赖边（crate 本来就在图里）。
这是 `api/` 层第一次碰 gpui 的类型，理由与代价写在 `src/README.md` 的分层规则里。

**四条实现上的硬规矩**（都在 `src/api/download.rs`）：
1. **先写 `成品名.part`，整条轨读完才 `rename`**。好处：中断留下的东西不会长得像成品。
2. **取消是 `AtomicBool` 标志，不是 `JoinHandle::abort()`**。`abort` 在下一个 await 点把任务丢掉，
   任务手里的 `File`/`BufWriter` 还没 flush 就析构，而主线程同时在删同一个文件——
   Windows 上"文件还开着就删"会失败。现在由任务自己收尾（flush → 删 `.part` → 发 `Cancelled`）。
   顺带一条：tokio 的 `JoinHandle` 被 drop **不**取消任务（那是 gpui `Task` 的语义），
   所以队列丢掉它是安全的，与第 4.9 节那条"必须持有或 detach"不冲突。
3. **主地址失败按顺序换 `backupUrl`**（roadmap 里 R0 第 3 条：接口给了 `backup_url` 但我们从没用过）。
   这正是只有下载能做的事——mpv 内部的地址我们替换不了。换地址时那一次写的字节**不计入总数**，
   所以进度偶尔会退一点，那不是画错。
4. **一对文件要么都留、要么都不留**。音频轨失败时把已改名的视频也删掉：
   留下一个没声音的 `xxx.m4v` 比全删更容易被当成可用文件用。

**大小与进度**：`dash.*[].size` 是 `Option`（`videostream_url.rs:63`），
所以 `total_bytes()` **只在全部要下的轨都知道大小时才给分母**；拿不到就走
`Progress::loading(true)` 的不定进度 + 已下字节数。`percent()` 把读数夹在 99.9 以内——
"100%" 在这个界面里只属于 `Finished`。回报按 1 MiB 节流（一个 1 GB 的文件不该往界面通道塞四千条消息）。

**durl 明确拒绝**：分段直链要按 `order` 拼接才是完整一片，这一版不实现拼接，
`request_for()` 直接返回一句实话而不是"看起来能下、下出来只有第一段"。
**断点续传也不做**（没有 Range 逻辑），取消就是删掉重来。

**状态只有一个出处**：`DownloadQueue` 是外壳创建的实体，注入详情页（发起）与下载页（查看），
页面之间因此互不认识。tokio 任务只往 `mpsc::UnboundedReceiver<DownloadEvent>` 里塞进度与终态，
**不共享任何可变状态**（没有 `Arc<Mutex<Vec<..>>>`，也没有给界面读的原子计数）：
所有字段都在主线程的 `DownloadTask` 里，改它们的路径只有 `apply()` 一处。
下载页用 `TaskRow` 快照再建按钮，是因为 `cx.listener` 要 `&mut Context` 而 `queue.read(cx)` 还借着同一个 `cx`。

**成品怎么听**：下载页那颗「用 mpv 打开」把两个本地路径交给同一个 `player::launch`
（`--audio-file=` 对本地文件与远程 URL 是同一条代码路，参数表一个字不用改），
子进程句柄照旧**持有并在 `Drop` 里 kill**——多开一条就收掉上一条，否则就是"声音还在、没人能关"。
「打开所在目录」用 `App::reveal_path`（Windows 后端有实现，不自己拼 `explorer.exe /select,`）。

**已知做不到/没做**：没有下载队列上限（点五个就是五个在跑）；没有并发混流成一个 mp4（要引 ffmpeg）；
任务记录不持久化（关程序就没了，磁盘成品不动）；退出时还在写的任务会被硬杀，留下 `.part`（这一层不管）。

### 4.25 详情页与评论区的文字可选中、可复制（P11，2026-10-26）

**gpui 0.3.9 本体没有文本选择**：`ref/gpui/gpui/src` 全文搜不到 `Selectable`/`text_selectable`/
`SelectedText`/`text_selection.rs`，`elements/div.rs` 与 `elements/text.rs` 里连 "selection" 这个词都没有，
所以 `div().child("中文")` **默认不可选中**，也没有 `.text_selectable()` 这种开关。
机制在 kit 的 base 层：`gpui_kit::base::{SelectableText, TextSelection}`
（`ref/gpui-kit/crates/base/src/selectable_text.rs:19`、`text_selection.rs`）。

**接线几乎不用写**：`gpui_kit::open_window` 把内容包进 `base::Root`（`crates/kit/src/lib.rs:181-194`），
Root 已经挂了 `TextSelectionLayer`（`base/root.rs:254`）、装了 `Ctrl+C`/`cmd+C` → `Copy` 动作并把
选区写进剪贴板（`root.rs:225-234`）。所以我们现有的 `gpui_kit::init(cx)` + `open_window` 这条路
**天然带选中与复制**，只需要把要复制的那段文字换成叶子元素：

```rust
div().text_size(..).text_color(..)                       // 样式仍留在父 div 上
   .child(SelectableText::new("detail-title", info.title.clone()))
```

`SelectableText::new` 自己用 `window.with_element_state` 保留一个 handle（`:157-166`），
页面不必存字段；`with_handle(..) + .document_order(n)` 是给"多段拼成一个连续文档"用的，
这一版每段各管各的（划一段选中一段，不做跨段连选）。元素 id 要稳定：
评论用 `rpid` 而不是行号，翻页后第 1 行不会继承上一页第 1 行的选区。

**触摸屏那条**：没有键盘就没有 Ctrl+C，所以页头右侧加了一颗「复制选中」，
读的是同一个窗口级选区（`TextSelection::selected_text(window, cx)`），
空选区就什么都不做（只在日志里留一句）。剪贴板只有 `App::write_to_clipboard(ClipboardItem::new_string(String))`
一条路，没有 `arboard`。

**与 P10 拖动滚动的冲突必须选一边**：手指与鼠标到这一层是同一条 `WM_MOUSEMOVE`（第 4.23 节），
**没有来源可辨**。同一个"按住左键拖"既是"划文字"又是"拖内容"，所以按下时问一句
`TextSelection::is_selectable_at(event.position, window, cx)`（它读上一帧 paint 登记的 `text_bounds`，
所以只能在按下时问）：落在可选中文字上就不记拖动起点，这一路拖动只是选字；落在卡片、封面、空白处照旧拖着走。
`DragScroll` 的 `stop_propagation()` 因此只在"落点不是文字"时发出，不会再打断选择。
**副作用要知道**：详情页那几大段文字区域只能选字，要滚就得在卡片之间或空白处拖——
真机如果觉得这条不划算，改的是这个判据，不是选择功能本身（这是风险表第 24 条）。

### 4.26 合集与系列、分集与分 P（P12，2026-10-26）

**这一轮是他先推翻自己的旧决定**：早先"合集列表不做"在他那批否决里，2026-10-26 他改口
"继续完成UP主的主页内容…我需要其『合集与系列』内容，并且确保支持好了『分P』/『分集』，
尤其是分集…情况比较复杂也要能支持"，并附了 B 站网页端那张 168/189 集的分集列表截图。

**三处形状，两个入口**：

| 位置 | 数据 | 分页口径 |
| --- | --- | --- |
| 空间页第四格"合集" | `video().seasons_series_list`（**一次同时给 `seasons_list` 与 `series_list`**，`ref/bpi-rs/src/video/client.rs:142`） | 页码；`page.total` 语义没文档 → **不显示**，"还能不能翻"按"这一页给满了"判 |
| 点进去的内容页 | 合集走 `seasons_archives_list`（`meta.total` 权威）；系列走 `series_archives`（**不签 WBI**、**响应里不给系列名**，名字从上一页带） | 页码 + 已加载条数比 `total` |
| 详情页右栏最上一节"分集" | `view` 原始 JSON 里的 `data.ugc_season`（**库里那份类型没这个字段**，见 QODER.md 那条边界） | 无分页：接口在这份响应里给的就是它给的那些集 |

**"复杂"那部分具体指两件事，都得支持**：

1. **一个合集里再分段**（截图里的"电动/猞狸 / 量力/度规 / 量力/扑通"）。`ugc_season.sections[]`
   每段有自己的 `title` 与 `episodes[]`，所以界面上是"分段 chips + 段内一列"；
   只有一个分段时**不给 chips**（那一排按钮只占一行、什么也没选）。
   行号**跨分段连续**（第二段的第一集是"第 3 集"），这样"第 168 / 189 集"那个读数与左边那列数字对得上。
2. **分集里的一集自己还是多 P 稿件**。`episodes[].arc.videos` 就是它的分 P 数，
   所以那一行标 `4 P` 这种角标，并且下面那节标题从"选集"改成**"本集的分 P"**——
   两节都叫"选集"会让人以为是同一份列表的另一半。点那一集 = 走 `OpenVideo` 压一层详情页，
   进去后它自己的 `view.pages` 照常列 P，播放/下载都按那一集自己的 P 走（不需要新代码）。

**三个数字各说各的话**（`SeasonBrief`）：`ep_count` 是接口声明的总集数、`listed` 是本次真拿到的条数、
`skipped` 是因为没有 `bvid` 而跳过的条数、`current_index` 是当前集在扁平列表里的位置（0 = 不在里面）。
四者不等时界面就照实说（"共 189 集 · 本次列出 100 集"、"当前这一集不在列出的名单里"），
**不藏差额**——与收藏夹内容那一页"另有 M 条打不开"同一口径。

**为什么不引 `VideoData`**：`ref/bpi-rs/src/video/info/view.rs:250` 那个 `UgcSeason` 类型
**没有任何方法返回它**（死代码），而且它的字段一个 `#[serde(default)]` 都没有——
真按它反序列化，B 站少给任一字段就是整个详情页失败。所以我们只读 `Value`、只取要的那几格，
`text_of`/`number_of` 两个小函数把"类型不对"一律收成"没有"（`videos` 给成字符串就当没有，
`ep_count` 给成小数就当没有），这些判据全进单测（`season_tests` 五条）。

**内容页不放搜索框**：`seasons_archives_list` 与 `series_archives` 的参数表里都没有 `keyword`
（`ref/bpi-rs/src/video/collection/params.rs:24-76`、`:182-249`），所以这一页只有翻页，
不做"看起来能搜、其实只搜了本页"的输入框（QODER.md"接口没给的能力不编"）。

**`Archive` 里没有 `videos`**：合集内容页那一行**不猜**"这一集有几个 P"（那个字段不存在，
`collection/info.rs:26-55`）。想知道只有两条路：点进去看它自己的 `view`，或在详情页那节分集里看
`arc.videos`。这是两条接口给料能力的真实差别，界面上不抹平。

### 4.27 落盘目录搬到 `%APPDATA%` + 设置页四项真能改（P13，2026-10-26）

**这一节改的是 4.21 的落盘结论**（那份"exe 同目录"只活了半天）。用户原话：
"`config.json`/`auth.json`/`debug.json` 改为存储到 `%APPDATA%/ArrowMeo/rustBiliPC` 里，
**不要保留任何支持旧位置的 json 文件的代码**"。

- 唯一入口是 `src/app.rs::settings_dir()`：Windows 取 `APPDATA`，非 Windows 取
  `XDG_CONFIG_HOME`/`~/.config`，两个环境变量都拿不到才退回 `<exe 目录>/ArrowMeo/rustBiliPC`
  （服务/计划任务那种启动方式会走到这一支；不 panic，设置页那行照旧把真实路径写出来）。
  三个写盘点（凭据、设置、日志）都先 `ensure_settings_dir()` 再落。
- **旧位置一行代码都不剩**：`Auth::legacy_path()`、`load()` 里"读不到就去看一眼
  `data/cookies.json` 并迁移"那一整块、`app::data_dir()` 全部删除。
  **磁盘上那些旧文件没删**（`publish/auth.json`、`publish/data/cookies.json`）：那是他某一次登录的现场，
  删掉不可逆，而现在代码根本不会再读它们，留着不影响任何东西。
- 附带收益：以前"取 CI 产物只许覆盖 exe、绝不整目录清空 `publish/`"这条要靠人记的红线，
  现在结构上安全了——用户数据不在产物目录里。规矩本身仍留在 QODER.md 第 5 节
  （`publish/` 里还有他自己放的 `mpv.exe` 和旧日志）。
- 启动顺序因此变了：**先读设置，再装日志**（"要不要写 `debug.log`"本身就是设置里的一项）。
  `main.rs` 文件头那 1–5 步就是这份顺序的权威说明，改启动流程先改那里。

设置页从"只读关于页"变成四项能改（`src/pages/about/view.rs`）。四项与它们的读取方：

| 设置项 | 落在 `config.json` | 真的读它的代码 | 生效时机 |
| --- | --- | --- | --- |
| 默认播放画质 | `max_quality` | `api::stream::request_qn()` + `stream_select::pick_video` 的封顶 + `quality_options` 过滤 | 下一次起播/下载 |
| 默认播放编码 | `codec` | `api::stream::preferred_codec()` → `stream_select::codec_score` | 下一次起播/下载 |
| MPV 播放器位置 | `mpv_path`（空串=自动） | `player::resolve_mpv` → `find_mpv` → `launch` | 下一次起播 |
| 调试模式 | `debug_log` | `logging::init(enabled)`（`main.rs` 只在启动时调一次） | **下一次启动** |

三处**照字面做会做错**的地方：

1. **没有"2K"这一档**。他要的清单里写了 2K，但 B 站的 `qn` 是固定档位代码
   （`ref/bpi-rs/src/models/stream.rs:8-38`：80/112/116/120/125/126/127），
   **1440p 的内容是在 120(4K) 这一档给的**。摆一个"2K"按钮实际发的就是 4K 的请求，
   属于 QODER.md 禁的"界面说的和发出去的不是一回事"。所以按钮是 1080P / 1080P 60帧 / 4K，
   这句话写在下面一行说明里。
2. **116 从硬上限降为默认值**。以前 `stream_select::MAX_QUALITY_ID = 116` 同时是"默认"和"天花板"，
   现在拆开：`DEFAULT_QUALITY_CAP = 116`（默认，注释说明它为什么对目标机是好选择）
   与 `config::MAX_QUALITY_ID = 127`（允许写到的最高档）。`.fourk(want_qn >= 120)` 跟着设置走，
   所以没选 4K 时请求里那位仍然是 0。**代价没有藏**：4K 要大会员、目标机大概率软解丢帧，写在按钮下面。
3. **编码偏好必须真的压过默认优先级**。`codec_score(codecs, preferred)`：命中偏好给 3，
   否则按"省带宽"排 HEVC 2 > AV1 1 > AVC 0。他选 AV1 时不该在同档里被偷偷换成 HEVC
   ——那界面就成了假话。那一档 B 站没给时才退回默认顺序（`Avc` 偏好下遇到只有 HEVC/AV1 的流
   会选 HEVC，单测 `explicit_codec_preference_beats_the_default_order` 钉住这两半）。
   画质档仍永远排在编码之前（`quality_still_outranks_codec_preference`）：不会为了 HEVC 放弃更高档。

**mpv 位置这一项推翻了他自己 2026-10-23 那条"只认运行目录、不走 PATH"**（新原话："默认在 PATH，
支持打开资源管理器选择 mpv.exe"）。当年那个坑没消失，是用两条规则绕开的：
查找顺序"设置指定 → PATH → 运行目录"，但每一步都自己拼 `目录/mpv.exe` 再 `is_file()`，
**不经过 `PATHEXT`/shell 的 `where` 语义**（那会先命中 `mpv.com`，而它会再派生一个真正的 mpv.exe，
我们 `kill` 的只是外层启动器 —— 第十八轮真机实测），设置里写死的路径必须是真实文件，
否则跳过它继续找并把原因显示在设置页那一行。"选择…"用 `App::prompt_for_paths{ files: true, .. }`
（`PathPromptOptions` **没有扩展名过滤**这一项，能给的只有 `prompt` 那句文案）；
"恢复自动"只在真指定过才出现（否则是一颗什么都不做的按钮）。
`resolve_mpv`/`path_directories` 是纯函数、注入 `exists` 判定，三条顺序都有单测。

**调试模式**：`logging::init(enabled)`，关着时不建目录、不开文件、不装 logger、`set_max_level(Off)`，
全项目那些 `log!` 都是空转。`log_path()` 改成 `Option<PathBuf>` 并且**只在真开出文件时**记路径
（以前它会造一个"看起来有"的路径，关掉调试模式后那就是假信息）。
两条连带事实要记住：① 控制台的有无是**链接期**子系统（`main.rs` 的
`#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]`），所以两种模式下都不弹窗口，
"调试模式开着也不弹控制台"不是 bug，真要做要另写 `AttachConsole`（没做）；
② 配置**坏了**的那一轮强制开日志（`LoadOutcome::RecoveredFromBroken`）——
调试模式就写在那个坏文件里，不开日志他就既看不到错误也没法打开它。
`CONFIG` 从 `OnceLock` 换成 `RwLock`（`OnceLock` 装一次就换不掉，"能改"就成了假设置），
写盘走 `app::update_config(闭包)`：**先改内存快照、再写文件**，失败把原因显示在页面上
（顺序反过来会出现"界面显示旧值、实际按新值跑"这种更难受的分裂）。

### 4.28 起播换备用域名、`-101` 自动重登、自动连续翻页（P13，2026-10-26）

他点的三件，都落在"客户端能真正控制的那一层"，没有一件是靠换技术栈绕开的。

**① 播放的 `backupUrl` 只能在"交给 mpv 之前"用**（`src/api/probe.rs`）。
下载是我们自己读字节，失败可以当场换下一个地址（4.24）；播放是 `launch()` 把 URL 交给 mpv，
**它内部用的地址我们既看不到也换不了**。所以起播前给每条候选地址发一个 `Range: bytes=0-0`
的 GET，第一个"能用的"才交给 mpv。四条口径：
- 请求头与交给 mpv / 交给下载的是**同一组**：`probe::cdn_headers()`（凭据 Cookie）+
  `player::BILIBILI_REFERER` + `MPV_USER_AGENT`，`ordered_urls()`（主地址在前、去空去重）也是两边共用，
  `download.rs` 里那两份私有副本删掉了。头不一致就会出现"我们探到 403、mpv 却能播"这种假结论。
- 200/206/3xx 都算通（有的 CDN 不认 Range 直接回 200；我们不读体、立刻丢弃），
  403/404/416/5xx 不算。**403 也换域名**：B 站某个边缘节点抽风是常见现象。
- 超时用 `BackgroundExecutor::timer()` + `futures::future::select`（6 秒）夹在外面。
  这一段跑在 gpui 前台任务里、用 gpui 的 `HttpClient`（与封面图、下载同一条 HTTP 路），
  **不引第二套 HTTP 栈**，也不用 tokio 定时器（那是 bpi-rs 那一侧的事）。
- **全探不通时仍然把主地址交给 mpv**，界面写"每个地址都探不通（…），仍按主地址交给 mpv 试一次"：
  探测可能被代理/防火墙/CDN 对 Range 的怪脾气误判，而 mpv 自己还有一次机会。
  换址成功时那句"视频轨主地址不通，已换 <主机名>（a：HTTP 403）"写在提示最前面。
  只给**主机名**：完整直链里是签名令牌（`deadline`/`gen`），不该显示到界面上。
- 探测期间用户又点播放/换 P：`probe_seq` 一变，旧结果丢（4.9 那条老规矩）。
  happy path 只多两个小请求（视频、音频各一），探到通就不再问它的备用域名。

**② 凭据过期自动重登**（`src/pages/relogin.rs`）。判定只用库里的 `BpiError::requires_login()`
（`code ∈ {-101, -401, 800501007}` 或 HTTP 401），**不在我们这边再抄一遍错误码**。
处置 = 删掉 `auth.json` + 开一个扫码登录小窗。四条规矩：
- **判断与处置分两步**：`relogin::message(&error)` 翻文案并记一个进程级原子布尔，
  真正开窗在 `relogin::flush(&mut AsyncApp)` 里做——`App::open_window` 会强制画一帧，
  不能塞进某个实体的 `update`（4.11/4.19 那条）。走 `loading::spawn_into` 的页面自动有这两步；
  detail/space/search/home 那 12 处手写 `cx.spawn` 是同一对调用。
- **去重记的是窗口句柄而不是布尔**：协调器存 `Option<AnyWindowHandle>`，
  "还在不在"用 `handle.update(..).is_ok()` 问一句。用户直接关掉小窗（不扫了）时那句会失败，
  登记自动清掉，下一次 `-101` 就还能再开一次。这样就不需要 `on_window_closed` 观察者
  （它要 `WindowId`，还得有人长期持有 `Subscription`）。
- **没登记客户端就不动**：启动时本机就没凭据的那一窗由 `main.rs` 开，它同时把句柄登记进来，
  所以跑起来后的第一次 `-101` 不会在同一个现场再开第二个窗。
- **重登成功不等于再开一个主窗口**：登录窗现在有两种收尾（`pages::gate::GateMode`）——
  `Startup` 仍是"先开主窗、再关自己"，`Relogin` **只关自己**，然后叫外壳
  `reload_after_relogin(mid)` 重取当前那一格（顺带把收藏/设置两页记住的 mid 换掉）。
  走错模式的症状很具体：桌面上出现两个一模一样的主窗口。
  下钻栈**不动**：清栈会 drop 详情页实体，`DetailPage::Drop` 会 kill 我们在放的 mpv
  （"子进程必须有归宿"），那等于"重登把视频掐了"。
  其余格子仍按各自 `ensure_loaded`（它们此刻的状态是"失败"，`ensure_loaded` 认为加载过就不再取）——
  **这是这一版的取舍**：只保证用户看得见的那一格是新数据。

**③ 自动连续翻页**。判据集中成一个纯函数 + 两个小方法：
`ui::scroll::should_auto_page(scroll, has_more, items, loading)`，四条都必要——
有下一页、列表非空（空列表的"到底"是还没加载，不是滚到底）、**没有在途请求**
（少了这条，游标类分页会在同一个游标上连发同一个请求）、这一帧真的有可滚的行程
（`is_at_bottom` 在"零行程"时按定义给 `true`，一屏装得下的列表不该"打开页面就自动捞三页"）。
提前量 `AUTO_PAGE_MARGIN = 240.`（约两行卡片），把等待藏在滚动里。
`render` 里读到的偏移来自**上一帧**布局，所以刚加长列表那一帧可能读到旧行程——
效果只是"晚一帧触发"，不会连发。接上的页面：**观看历史**（游标）、**收藏夹内容**（页码+`has_more`）、
**合集与系列内容**（页码，`has_more` 是我们按已加载条数算的那份）、**搜索**（页码 + `num_pages`）、
**空间页的图文与合集**两格。原来那排"加载更多/下一页"按钮全部留着——
判据万一没命中（窗口很高、一屏装完），手点还能走。

两处**故意没做**，都写在代码注释里：搜索的自动翻页是**接到列表尾巴**（`append = true`），
而点"上一页/下一页"仍然整屏换成那一页（人在看第 2 页时列表突然变成第 3 页是最难受的；
追加失败也**不清列表**，与历史/收藏内容页同一条口径）；
空间页**投稿**那一格是"整屏换一页"（还有分区筛选与上下页），自动续页要先改成追加模式，
那是另一件事；**评论区**不自动翻——一页 20 条、翻页是随机访问，自动全捞会把"看看评论"变成几十次请求。

**④ CI/CD 拆开**（`.github/workflows/`，`build.yml` 删除）：
`ci.yml` = push/PR 自动跑、只 win x64 release + `cargo test`（本机没工具链，这一条是编译证据通道，
矩阵会让人不敢推）；`cd.yml` = 手动、win/linux × x64/arm64 四个目标、
`[profile.release-dist]`（LTO + `codegen-units=1` + `strip`）+ UPX（`-9 --lzma`，压失败就发未压缩那份）
+ `gh release create`，**资产只有可执行文件**。`release-dist` 不写 `panic = "abort"`：
`app::describe_join_error` 靠 tokio 把任务 panic 变成 `JoinError` 才能把崩溃原因说给人看。
四个目标里 win-x64 是长期验证过的，另三个是这一版**新配上、还没跑过**的（风险表第 31 条）。

### 4.29 全面审之后的优化：图标表、crypto provider、下载排队与退出收尾（P14，2026-10-27）

P13 交完之后按"设计缺陷 / 依赖 / 性能 / 占用 / 体积 / 稳定"六项过了一遍代码，
先修掉八条（第四十九~五十一轮，账在第 8 节），再做了下面三件要动体积与依赖的。

**① 图标资源从"整份 Lucide"换成我们自己那张表**（`src/ui/assets.rs` + `src/ui/icons/`）
- 起因：`main.rs` 一直挂 `gpui_kit::assets::AllAssets`，那是 rust-embed 把
  `assets/icons/**/*.svg` **1830 个文件全塞进 exe**（`.rdata` 里的常量表，
  运行时不读也在二进制里；磁盘上这批文件 732 KB，但嵌进去比原始字节更贵——每个条目要对齐）。
  当初换它是被迫的：kit 默认的 `Assets` 只有 106 个，
  侧栏的 `House`/`Clock`/`Bookmark`/`Download` 不在里面，而**找不到图标不会报错、只画一片空白**。
- 现在的表 = **kit 默认那 106 个**（它自己的组件要用哪些，那 106 个就是答案）
  **+ 我们多用的 4 个** = 110 项、磁盘 **48 KB**。
  用 `include_bytes!` 写死成常量表，**不为此引 rust-embed**（少一个 proc-macro 依赖）。
- **实测收益（第五十一轮，同一台 CI 机、`--release`）**：exe 从 **36,980,736**（第四十七轮）
  降到 **36,053,504** 字节，**小了 927,232 字节（0.88 MB）**。这一版中间还加了探测、重登、
  设置页与图标表之外的代码，所以图标这一条单独的收益只会比这个数大。
  参照物：当年把侧栏图标挂上 `AllAssets` 那一轮 exe **涨了 1.33 MB**（第 8 节第三十三轮那行）。
  唯一的失效模式还是那片空白，所以拿测试堵：`assets_tests` 三条 ——
  用到的 8 个名字逐个 `IconName::path()` 查表、表里每项真能 `load` 且像 svg、
  条目数与总字节当体积闸（**加图标要三处一起改**：拷 svg、表里加一行、测试数组加一项，
  `src/ui/icons/README.md` 与 `ui::layout::side_item` 的注释都写了这句）。

**② crypto provider 从 `aws-lc-rs` 换成 `ring`**
- `rustls = "0.23"` 的默认特征带 `aws-lc-rs` → 依赖图里进来一个 `aws-lc-sys`
  （C + 汇编，CI 那步 `choco install nasm` 就是为它装的，编译时间与代码体积都吃它）。
  我们只需要"有一个 provider 被显式装上"，`ring` 已经在图里（`app.rs` 文件头那条事实），
  所以直接依赖改成 `default-features = false` + 只开 `ring/std/logging/tls12`，
  `install_tls_provider()` 装 `rustls::crypto::ring::default_provider()`。
- **能不能真把 `aws-lc-sys` 从图里去掉要看 CI 日志**（reqwest 那条链如果自己点名要它，
  这一改就只是"说清我们用哪个"，不减依赖）。**第五十一轮的结论：不减**。
  CI 上传回来的 `Cargo.lock` 与本地零差异，里面 `aws-lc-rs 1.18.2` + `aws-lc-sys` 仍在，
  点名要它的是 **`quinn-proto 0.11.19`**（HTTP/3，随 `reqwest 0.13.5` 进来，而 reqwest 是
  gpui 与 bpi-rs 带的，我们改不了它的特征）；`rustls` 那一条 `aws_lc_rs` 特征也因此还开着。
  所以这一改**只买到两件事**：① 我们这一侧的口径写死了（不再依赖"图上只有一个 provider"这种巧合），
  ② **运行时走的是 ring**——启动时 `install_default()` 装上 ring 之后，
  全进程（含 gpui 的 HTTPS）都用它，`aws-lc-sys` 那块代码只是躺在二进制里没被调用。
  **`choco install nasm` 那一步继续留着**（它还在编译），等哪天 reqwest/quinn 那条链断了再删。

**③ 下载：同时在跑限 3 条，退出时叫停在跑的任务**
- 第一版"点五条就是五条在跑"。现在 `TaskState::Pending`（界面"排队中"，页头写清
  "进行中 N · 排队中 M（同时最多 3 条）"），`enqueue` 只登记、`pump` 决定开跑还是排队、
  `apply` 进终态后再泵一次补位；`started` 标志防同一条被 spawn 两次。
  排队的**也能取消**（直接置终态，不 spawn），排队/在跑的**都不给"移除记录"**
  ——那是这条任务唯一的入口。详情页那句提示也跟着从"已开始下载"改成"已排进下载队列"。
- 退出收尾：`api::download` 记一张"在跑任务"的名册（`ACTIVE`），
  `cancel_all_active()` 只置标志；`DownloadQueue::Drop` 与 `main.rs` 主动退出前各叫一次。
  **不在主线程直接删 `.part`**：那会儿任务可能还开着句柄（Windows 删不掉），
  还可能撞上"任务刚改名成成品、主线程把成品当临时文件删了"的先后歧义。
  顺带修掉取消路径本身的一个 bug：原来 `attempt` 取消时没先 `drop(writer)` 就返回，
  上层删 `.part` 时句柄还开着 —— 这正是"取消了但目录里留个半成品"的原因。
- **诚实的限制**（写进代码注释，别当已解决）：gpui 没有"即将退出"的回调
  （只有 `App::on_window_closed`，它要 `WindowId` 且要有人长期持有 `Subscription`），
  所以"用户点 X 退出"这一路只能靠 `Drop` 兜；置完标志到进程真退可能只剩几毫秒，
  任务不一定来得及跑到下一个分块，那种情况下磁盘上仍会留一个 `.part`。
  要治它就得让退出路径等一等任务，而"退出卡两秒"是不是他要的，得先问。

**没做的两件与理由**：列表虚拟化（gpui 的 `List` 才是虚拟的，我们整页都是 `div` 堆卡片；
自动翻页因此加了"240 条就停"的软闸，改成虚拟化是一次真正的重构，不动）；
`profile` 细分（`opt-level`/`panic` 这些先按现两份配置走，等 `cd.yml` 真跑一次量出数字再说）。

## 5. 阶段进度


| 阶段 | 内容 | 状态 |
| --- | --- | --- |
| P0 | 依赖骨架、release CI、空窗口 + 中文输入框 + 假数据列表、`api/store/player` 纯逻辑层 | **完成**（run 39787252294 全绿） |
| P1 | 搜索接线：`search().video()`、结果转换（剥 `<em>`）、封面网络图（gpui 原生 URI 加载）、分页/排序 | **完成**（第十六轮找到封面真因——协议相对地址被当成内置资源，第 4.5 节——修好后真机确认列表/分页/封面都正常） |
| P2a | 登录：扫码（取码/轮询/过期重载）+ 凭据落盘 + 启动判定 + 登录后进主界面 | **完成**（第十一轮产物真机扫码登录通过，凭据已写出） |
| P2b | 个人：我的信息 / 收藏 / 历史 入口与页面 | **完成**（第十五轮产物真机通过；外壳 + 懒加载 + 三路并发，第 4.11 节） |
| P3 | 详情：`video().view()`、多 P、tags/related、字幕列表 | **完成**（第十七轮产物真机通过；机制见第 4.12 节。发布时间/分区按用户决定**不做**——live 的 `VideoView` 没有这两个字段） |
| P4a | 起播：playurl 取地址 + 选流 + 拉起外置 mpv（不遥控） | **完成**（第十八轮真机失败：mpv 窗口没出现，根因是 `--opt value` 写法 + mpv 该从运行目录取；第十九轮修好，43 passed、零 warning，真机确认能起播出声） |
| P4b | 曾按"遥控"理解做过一版（连管道、订阅属性、进度行、暂停/±10 秒）——第二十三轮 57 passed 全绿，但**方向错了**：播放期 UI 归 mpv。已按范围删净，机制留在第 4.14 节 |
| P5 | 视觉：照 `ref/Bili.Copilot` 的度量重做（色板/字号层级/卡片 232×238/网格/页头 56 + 侧栏 240/右栏 360/hover 换底色） | **完成**（他真机看过说"没什么问题"，唯一剩下的是右上角搜索框——根因是 kit 一直用亮色主题，第二十六轮已修，第 4.15 节最后两条） |
| P6a | 主页：首页推荐流当默认页 + "换一批"（第 4.16 节） | **编译与测试已绿**（第二十七轮 43 passed、零 Rust warning，产物 `dd2bf73`）；待真机看：能不能"换一批"、`goto` 过滤后有没有内容 |
| P6b | 评论区（顶层列表 + 翻页 + 最热/最新） | **编译与测试已绿**（第二十九轮 48 passed、零 Rust warning，产物 `edd553e`）；待真机看：评论有没有内容、翻页与排序（第 4.17 节） |
| P6c | 曾计划做"字幕 `sub-add` + 同实例换清晰度保住进度" | **用户 2026-10-25 判定不做**（"我认为P6c两个都没必要做"）：换清晰度现在就重开一个实例、字幕只列清单。第 4.14 节那套管道机制仍然留档，将来要捡起来照那份做 |
| P7 | 用户空间：点详情页 UP 主 → 资料卡 + 投稿（带分区筛选）/ 图文 / 收藏（第 4.18 节） | **编译与测试已绿**（第三十二轮 54 passed、零 Rust warning，产物 `40f556d`，md5 `86297ed0…`）；待真机看：第 4.18 节那 6 条 |
| P8 | 外壳重构（2026-10-26 用户定稿）：登录改独立小窗 + 侧栏 80 宽七入口（收藏/历史/待看/下载/设置）+ 左上角账号头像 + 落盘改 `auth.json`/`config.json` + 待看接口（第 4.19、4.20、4.21 节） | **编译与测试已绿**（第三十五轮 60 passed、零 Rust warning，产物 `0d659a8`，md5 `b2687551…`）；待真机看：第 4.19–4.21 节 + 该轮记录里那 5 条 |
| P9 | 收藏夹下钻 + 三种搜索（第 4.22 节）：收藏列表可展开成内容页、卡片进详情页；内容页服务端关键词搜索 + 加载更多；历史本地过滤 + 游标"再加载 30 条"；待看本地过滤 | **编译与测试已绿**（第三十八轮 62 passed、零 Rust warning，产物 `22c2f87`，md5 `b7aeeaa1…`）；待真机看：第 4.22 节那三条搜索口径 + 该轮记录 |
| P10 | 触摸屏优化（单指上下滚动 = 按住拖动平移内容，11 个滚动容器全接）+ 侧栏入口点击直接换页（先清下钻栈）（第 4.23 节） | **完成**（第三十九轮 65 passed、零 Rust warning，产物 `014b71c`，md5 `4c813442…`；他真机确认拖动滚动"这个好了"，侧栏换页同轮通过） |

| P11 | 详情页与评论区文字**可选中可复制**（第 4.25 节）+ **下载**（第 4.24 节：双文件 `.m4v`+`.m4a`、每次弹框选目录、只下当前 P、进度与取消、备用域名、打开所在目录/用 mpv 打开）+ 顺手修掉 roadmap R0 的两条（清晰度按钮按封顶过滤、`src/README.md` 里那条已经不存在的 `--input-ipc-server` 说法） | **编译与测试已绿**（第四十二轮 75 passed、零 Rust warning，产物 `4338445`，md5 `dd020f45…`；第四十轮红一次、第四十一轮绿但剩两条 warning，见第 8 节）；待真机看：选字与拖动滚动的分界（风险第 24 条）、弹框能不能正常选到目录、403/时效表现（风险第 25 条） |

| P12 | 空间页第四格**合集与系列** + 点进去的**内容页**（分页）+ 详情页右栏**分集**那一节（分段 chips、跨段连续行号、`N P` 角标、"本集的分 P"）（第 4.26 节） | **编译与测试已绿**（第四十三轮一次过：82 passed、零 Rust warning、Cargo.lock 零差异，产物 `858d275`，md5 `a591abdf…`）；待真机看：他那两个合集（含 189 集那个）能不能列全、分段切换、点一集进去还能不能正常播/下 |
| P13 | 设置页**四项真能改**（MPV 位置 / 默认画质 / 默认编码 / 调试模式，第 4.27 节）+ 三件补齐（起播前探地址换 `backupUrl`、`-101` 自动开重登小窗、滚到底自动翻页，第 4.28 节）+ 落盘搬到 `%APPDATA%/ArrowMeo/rustBiliPC`（旧位置代码清零）+ CI/CD 拆成 `ci.yml`/`cd.yml`（多平台 + LTO/strip + UPX + Release，资产只有 exe） | **编译与测试已绿**（第四十四轮红 5 条、第四十五轮红 1 条，第四十六轮 `85ef8ff` 绿：**102 passed、零 Rust warning、Cargo.lock 零差异**；第四十七轮 `12ce966` 补上本轮 exe 的 artifact 并交付，产物 md5 `efb03a94…`，见第 8 节三段）；待真机看：设置页四项改完是不是真的生效（尤其编码偏好与 4K 档）、`-101` 那一路会不会自己弹出小窗、滚到底翻页顺不顺、`%APPDATA%` 里三个文件长得对不对 |
| P14 | 全面审之后的优化（第 4.29 节）：八条修正（**panic 钩子 + `crash.log` 永远写**、设置页不再每帧扫磁盘、重登不再先删凭据、探测并行 + 收预算、**下载进度不再倒退**、日志不写完整直链、自动翻页加 240 条软闸、去掉没人用的 `tokio/macros`）+ 三件要动体积与依赖的（图标只嵌 110 个 / `rustls` 换 `ring` / 下载同时限 3 条与退出收尾） | **第五十一轮已绿**（`fcf9329`，107 passed / 0 warning / Cargo.lock 零差异，产物 **36,053,504** 字节、比第四十七轮**小 927,232**，已覆盖 `publish/rustbilipc.exe`，md5 `3197f5c9…`）。三条兑现度不同：① 图标兑现（改口的实测数字见第 4.29 节）；② provider **只兑现一半**——`aws-lc-sys` 去不掉，要它的是 `quinn-proto`（随 reqwest 进来），运行时确实走 ring；③ 下载排队待他真机点五条验。**第五十二轮**改 `cd.yml`：目标表拆到 `.github/cd-matrix.json`，`targets`（默认 win-x64）与 `publish`（默认关）两个输入 —— 先量 `release-dist`+UPX 的真实体积，别先公开 |

## 6. 已知风险与待验证

| # | 风险 | 处置 |
| --- | --- | --- |
| 1 | release 需要 `fxc.exe`（gpui `build.rs` 找不到就 panic） | **已解除**：CI 探测到 Windows SDK 的 fxc 并设 `GPUI_FXC_PATH`，该步通过 |
| 2 | `rustls 0.23` 默认 provider 拉 `aws-lc-sys`，MSVC 需 CMake + NASM | **已解除**：CI 的 `choco install nasm` 成功，aws-lc 编译通过 |
| 3 | 构建内有两份 reqwest（kit 的 `gpui-pre-reqwest 0.12.15` + bpi-rs 的 `0.13.5`） | 只影响编译时长，不动 |
| 4 | gpui-pre 快照每周变动、API 破坏性 | lock 入库作种子 + `=0.3.9` 精确 pin；升级要连带检查 kit 的 pin |
| 5 | gpui Windows 侧 `WindowDecorations::Client` 是空实现、不支持原生锚定弹窗、无托盘 API | 依赖 kit 组件自绘弹层；托盘如需，自调 `Shell_NotifyIconW` |
| 6 | 无 Mica/Acrylic 等价物（Bili.Copilot 的观感依赖它） | 用实色底近似，不追求一致 |
| 7 | mpv 对 DASH/fmp4 双流 seek 无任何文档 | 这条**没解除**，只是观测方变了：seek 现在是用户在 mpv 窗口里拖的，我们看不见。真机若反馈"拖进度后黑屏/音画错位/没声音"，先怀疑这条。**"下载合并再本地播"这个备选与'在线流式、不落盘'的定稿冲突**，真要走到那一步必须先问用户 |
| 8 | 1080P 需登录、HDR/4K/8K 需大会员 | 按 `accept_quality` 降级 + `requires_vip` 提示 |
| 9 | rustls 同时启用 aws-lc-rs 与 ring → 未装默认 provider 时 TLS 握手 panic（症状：加载态永不结束） | 启动时 `app::install_tls_provider()`；所有 `JoinHandle` 错误都上界面（第 4.7 节） |
| 10 | **`cx.spawn` 的 `Task` 被丢弃 = 任务立即取消**（症状：请求发出去了、界面永远停在加载态，日志里连闭包第一行都没有） | 已修：所有 spawn 一律 `.detach()` 或存进实体字段，并用 `[lints.rust] unused_must_use = "deny"` 让这类写法编译失败，见第 4.9 节 |
| 11 | 扫码成功的响应 cookie 里没有 `buvid3`，而 `Account::validate_complete` 要它 → 登录在最后一步失败 | `poll_qr` 在 `code==0` 且缺 buvid3 时调 `misc().buvid()`（`/x/frontend/finger/spi`）补一个，见第 4.10 节 |
| 12 | 收藏/历史那几个响应结构**没有 `#[serde(default)]`**（`HistoryListData.cursor/tab/list`、`FavListDetailData.info/has_more/ttl` 等），B 站少给任何一个字段就变成 `BpiError::parse` | 症状可控：那一块显示自己的错误位而不是整页空白（第 4.11 节），日志里有原始错误文本可判断缺哪个字段 |
| 13 | 播放直链有**时效**（URL 里的 `deadline`），放久了或跨网络就 403；`backupUrl` 也只在同一次响应里有效 | P4a 每次都现取现用（不缓存 `StreamChoice` 去起播）；真机若出现"窗口开了但黑屏"，先对照 `debug.log` 里 playurl 与 launch 的时间差。P6 再加失败换备用域名 |
| 14 | 目标机 i5-6200U 是 Skylake：**没有 AV1 硬解**，HEVC 只到 8bit；`--hwdec=auto` 选不到就退软解，1080P AV1 大概率丢帧 | 选流已把 HEVC 排在 AV1 之前（第 4.2 节），`player_note` 会打出实际 `codecs`；真机先看那串是 `hev1/hvc1` 还是 `avc1/av01`，再谈要不要在封顶档内偏好某一编码。第十九轮真机报的是 `hev1.1.6.L150.90` ✅ |
| 15 | 视觉重做只经过编译与代码核对，**没人眼看过**：度量抄的是 xaml，gpui 的排版引擎是 taffy，字体度量/行高/滚动条与 WinUI 不可能一模一样 | 必须真机看过再谈"完成"；第一版允许局部丑，但色板与字号层级不许在页面里各写一套（一律走 `ui/palette.rs`） |
| 16 | gpui 没有 Acrylic/Mica、没有 ItemContainer 选中视觉、没有图标字库 | 用纯色 + 1px 描边 + 圆角 + 字号差 + 三档灰表达层级；图标暂时用文字（参考项目自己也用"弹"字），需要成套图标时再自绘 SVG 走 asset |
| 17 | **主页载荷是严格结构**：`RcmdItem` 除四个 `Option` 外没有 `#[serde(default)]`，外层 `preload_expose_pct` 等也必填，B 站少给任一字段就整批 `BpiError::parse` | 症状可控：主页那一块显示原因文本、其余页不受影响（第 4.16 节）。真机若出现"主页永远失败"，先失败在哪个字段——日志里有原始错误文本，据此在 `api/home.rs` 里改成自己解析 `Value`（bpi-rs 那条 `popular_list` 用 Value 就是同一原因的产物） |
| 18 | `fresh_idx`/`fetch_row` 的递增口径**没有官方文档**（bpi-rs 只把两者原样发出去） | 真机验证"换一批"是否真换出不同内容；不换就改成只递增 `fresh_idx`、`fetch_row` 恒为 1，再不行就换 `video_ranking().popular_list()` 那条热门接口 |
| 19 | 评论区类型代码如果写错（**11 是相簿、1 才是视频**），症状是"评论永远是空的"而不报错；另外 `Comment`/`Member` 全是必填字段，少给任一就 `BpiError::parse` | 类型代码钉成常量 + 单测 `video_comment_target_type_is_one`（值来自 bpi-rs 的 `CommentType::Video = 1` 与它的 live 测试，见 api-notes 第 11 节）；解析失败只砸在评论区那一块，原因写在提示位，详情页其余部分照常 |
| 20 | **空间页三块各自可能被严格结构砸到**：`UserUploadedVideos`/`SpaceItem`（图文）这些模型同样没有全套 `#[serde(default)]`，B 站少给任一字段就 `BpiError::parse`；`list.tlist` 更是 `serde_json::Value`，形状完全由接口决定 | 三块各有独立提示位与独立序号（第 4.18 节），坏哪块只显示哪块，`debug.log` 里有原始错误文本；chips 解析失败只少一格（[`parse_tlist`](../src/api/space.rs) 有畸形输入单测）。真机若"投稿空但日志是 parse 失败"，就照第 4.16 节主页那条同法改成自己读 `Value` |
| 21 | **待看这一路是全有或全无**：`ToViewVideoItem` 与嵌套的 `ToViewRights/Owner/Stat/Dimension` 一个 `#[serde(default)]` 都没有（`ref/bpi-rs/src/historytoview/toview.rs:16-144`），B 站少给任一字段就整次 `BpiError::Decode`；这条接口还没有分页（零参数），所以失败时不是少几条而是整块没有 | 错误只写进待看那一页的提示位，别处照常（第 4.20 节）；日志里有原始文本。真机若稳定复现，照主页那条同法改成自己读 `Value` |
| 22 | **换窗口的顺序**：登录成功后如果先关登录窗再开主窗，Windows 的默认 `QuitMode` 会在最后一个窗口关闭时直接退进程（`gpui/src/app.rs:458` + `:3180-3188`），症状是扫完码程序闪退 | 顺序钉死在 `open_main_and_close_login`（先开主窗、开成功才关登录窗），且整段跑在 `Context::spawn` 的任务里而不是事件回调里（`app.rs:1520` 会强制画一帧）；改窗口代码前先读第 4.19 节 |
| 23 | **收藏夹内容这一层是严格结构 + 页码分页**：`FavListDetailData.info/has_more/ttl` 与 `FavListMedia` 十几个字段都没有 `#[serde(default)]`（整个 fav 模块只有 `medias` 有，`ref/bpi-rs/src/fav/list.rs:89`），B 站少给任一字段就整次解析失败 | 失败只显示在收藏夹内容那一页的提示位，收藏列表与别处照常；日志有原文。真机若稳定复现，照第 4.16 节主页那条同法改成自己读 `Value` |
| 24 | **选中与拖动的分界没在真机上验过**：`TextSelection::is_selectable_at` 读的是"当前作用域"里登记的 `text_bounds`（`base/src/text_selection.rs:2484`），而 `push/pop_text_selection_scope` 只在部分回调路径里包住作用域。如果我们那个按下回调跑在没有作用域的时刻，它会**恒返回 false**，症状是"在文字上拖动时既选字又滚页" | 判据集中在 `ui/scroll.rs::on_press` 一处，真机觉得不划算就改这一处（可以换成"只在详情页让位"或"移动超过 N px 才决定归谁"），**别关掉选择功能**。已定稿的口径：文字区域优先给选字，要滚就在卡片与空白处拖 |
| 25 | **下载写盘是长事务**：一次任务里 B 站给的 `size` 可能与实际不符（进度退一步）、`rename` 可能被占用文件的程序或杀毒软件挡下、主线程退出时还在写的任务会被硬杀留下 `.part` | 每种失败都原话进那一行的 `note`（不静默）；`.part` 不是成品名，看得见就能删；重试的口径是"再点一次下载"，重取一次地址（直链本身有时效，见 `bpi-rs` 那条 `120 分钟有效` 的注释），这一版**不做续传** |
| 26 | **没有队列上限**：点五个稿件就是五个任务同时在跑（每个任务内部两条轨是顺序写的），带宽与文件句柄都归 tokio 的多线程 runtime | 真机觉得太满就加"同时在跑不超过 N"的排队；现在先把取消做确实 |
| 27 | **合集/系列那三条接口的结构是严格的**：`Archive` 一个 `#[serde(default)]` 都没有（`collection/info.rs:25-55`），`SeasonsMeta`/`SeriesMeta` 同理，B 站少给任一字段就整次解析失败（与待看第 21 条、收藏夹内容第 23 条同一类） | 错误只写进本页提示位（空间页那一格 / 内容页那一页），别处照常；日志有原文。真机若稳定复现，照第 4.16 节主页那条同法改成自己读 `Value` |
| 28 | **分集那一路是我们自己发的第二个 `view` 请求**（第 4.26 节）：它比库里那次多一倍请求量，而且 B 站若把 `ugc_season` 改名/挪走，我们只会"少一节面板"，不会报错 | 宽容解析 + 失败只写进 `notes` 那一行（"分集信息没取到：…"），主体照常；日志里 `分集：标题（N 集，本次列出 M 集）` 一行能判断是"没给"还是"没解析出来"。真要合并成一次请求，就得整块换成自己读 `view`——那是架构决定，得他点头 |
| 29 | **起播前的地址探测是我们替 mpv 做的一次判断**（第 4.28 节）：`Range: bytes=0-0` 在个别 CDN/代理上可能被拒（416/403），那样会"明明能播却写成换了备用域名"；探测也带来每次起播 2 个小请求与最长 6 秒/条的等待 | ① **200 也算通**，不把"不认 Range"当失败；② 全探不通时**仍然把主地址交给 mpv**（误判不会变成"彻底播不了"）；③ 换址与失败原因都写在提示里（只写主机名，不写带签名令牌的完整 URL），真机若发现"每次都说不通"，看日志里 `CDN 探测完成 视频=N` 那一行就能判断是探测的问题还是 CDN 的问题；④ 探测期间换 P/再点一次播放由 `probe_seq` 作废，不会串台 |
| 30 | **`-101` 自动重登会删掉 `auth.json`**（第 4.28 节）：如果哪条接口在**没登录也会成功**的场景里回了 `-101`（或库里 `requires_login` 的口径将来变宽），就是"好端端把凭据删了"；另外重登只刷新当前那一格，其余页面此刻仍是失败态 | 删之前先 `log::info!` 写清是哪一路判定的（真机出问题看 `debug.log` 第一行就有）；`Auth::clear()` 失败只 warn、不挡住开窗（拿不到新凭据时旧文件留着反而有用）；同一进程里靠"登录窗句柄还在不在"去重，不会开出一排小窗；其余格子的"重新加载"按钮都还在 |
| 31 | **`cd.yml` 的四个目标里只有 win-x64 是长期编过的**：win-arm64 要 runner 镜像里有 MSVC ARM64 工具，两个 Linux 目标要 gpui 的一串系统库（X11/wayland/fontconfig/ALSA/vulkan），`ubuntu-24.04-arm` 这个 runner 标签是否存在也没验过；UPX 压过的 Rust/GUI exe 还有被杀软误报或压坏的先例风险 | 矩阵是 `fail-fast: false`，一个目标红不影响其余出产物；UPX 那一步**失败就发未压缩那份**（不为了压尺寸把发布卡死）；Linux 依赖那一步只装公共包，没有额外源。**第一次真跑完必须把结论回填本节与 QODER.md 第 2 节**（哪个目标真绿、报什么错），别把"配上了"当"能用了"。**第五十二轮的开关**：`targets` 默认只挑 win-x64、`publish` 默认关，所以现在可以"只量体积不公开" |
| 32 | **自动连续翻页在 `render` 里判断**（第 4.28 节）：`ScrollHandle::max_offset()` 反映的是**上一帧**布局，所以刚加长列表那一帧读到的是旧行程；另外窗口很高、一屏装得下时 `is_at_bottom` 恒为真 | 四条判据里 `scrollable()` 与 `loading` 就是为这两条准备的（前者挡"零行程也触发"，后者挡"同一游标连发"）；每页的"加载更多/下一页"按钮全部保留，自动那一路没命中还能手点。若真机发现"翻两页就停住"，先看日志里那句"滚到底，自动取第 N 页"有没有出现 |
| 33 | **图标只嵌我们自己那张表之后，"用了没嵌的名字"从"不会发生"变成"会发生"**（第 4.29 节第 ① 条）：gpui 找不到资源不报错、不 panic，只画一片空白 | `assets_tests` 三条把它变成编译期之外的第一道闸（用到的名字逐个查表 + 每项能 `load` 出像 svg 的内容 + 条目数/字节数当体积闸）；加图标的三步写在 `src/ui/icons/README.md`、`ui::layout::side_item` 注释与 QODER.md 三处。**真出现某处空白时**：先 `grep` 那个 kebab-case 在不在 `src/ui/icons/`，再看 `ICONS` 表有没有那一行 |
| 34 | **退出收尾只在"我们叫的退出"路径上成立**（第 4.29 节第 ③ 条）：gpui 没有"即将退出"回调，`DownloadQueue::Drop` 兜住点 X 那一路，但置完标志到进程真退可能只剩几毫秒，任务不一定跑到下一个分块 → 磁盘上仍可能留一个 `.part`；被任务管理器杀掉更是完全不走这条路 | 现状：`.part` 一眼能看出不是成品（这是 P11 定"先写 `.part` 再改名"时就买下的保险），删它交给用户即可。要真治：给"退出收尾"加一次极短等待（1~2 秒）或做启动时清理遗留 `.part` —— **两者都会改变退出手感/会误删正在下的东西，等他点头** |

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

### 第十四轮（run 39895619637，commit `5c7c8ae`）：P2b 通过

- 全步骤 success，**31 passed / 0 failed**（新增 `mine_tests` 两条：`format_seconds` 的边界、
  `vip_label` 的口径），我们的 crate 编译 **35.13 秒**，整轮约 4 分钟；零 warning。
- 这轮的产物**还没给用户测**：分支上新增了"个人中心懒加载"（第 4.11 节）那一个提交，
  所以要再烧第十五轮，让被测二进制就是分支顶端。

### 第十五轮（run 39896064537，commit `2529841`）：懒加载 + P2b 最终产物

- 全步骤 success；产物 `publish/rustbilipc.exe` 32,409,088 字节（md5 `c0e6e1dd…`），
  比第十四轮大 0.5 MB，是新增三个 api 封装与两个页面的量。
- `Cargo.lock` 与库内完全一致：P2b 只用到已经锁好的依赖，没引入任何新包。
- 取产物时依旧走"解到临时目录 → 只覆盖 exe"，`publish/debug.log` 与 `publish/data/` 原样不动。

### 第十七轮（run 39900823747，commit `89272e2`）：P3 详情页一轮通过

- **37 passed / 0 failed**（新增 `detail_tests`：`favorite`/`fav` 合并、字幕地址补协议、
  `seconds_to_u32` 的超限截断），我们的 crate 编译 6.39 秒（缓存全命中），`Cargo.lock` 零差异。
- 交付物：`publish/rustbilipc.exe` 33,105,408 字节，md5 `1e26a5bb…`。
- 一次过没有返工，靠的是写之前把 `video()` 的每个签名与响应字段逐个查过（记在
  `docs/api-notes.md` 第 4 节），以及第 2 节那条"`on_click` 也要先 `.id()`"——
  这条是 P3 写完后补进清单的，所以卡片点击没再踩坑。

### 第十六轮（run 39898075460，commit `970c4a9`）：封面图真因修复

- **34 passed / 0 failed**（新增 `cover_tests` 三条），产物 32,436,224 字节（md5 `43253293…`），
  `Cargo.lock` 与库内零差异。
- 修复的是第十五轮真机日志里那条 `Embedded resource not found: //i2.hdslb.com/…`：
  搜索接口给的是协议相对地址，gpui 判 URL 靠 `Url::parse`，于是从没发过请求（第 4.5 节）。
  顺带按实测数字改成要缩略图：封面 541KB→14.8KB、头像 812KB→2.7KB。
- Actions 新增一条与我们有关的废弃警告：`##[warning]Input 'save-always' has been deprecated
  with message: save-always does not work as intended`。本轮缓存仍然命中（我们的 crate 三十多秒编完），
  所以先不改 workflow；**如果以后缓存命中率变差，这条是第一个嫌疑**，替代方案是拆成
  `post: true` 的独立保存步骤或用 `cache-save` action。

- **待真机验证**（P2b）：外壳两个入口、"我的"第一次点下去才发那三个请求、资料头与收藏夹/历史
  能出内容、三块各自的错误位是否显示得下 B 站少字段造成的 `BpiError::parse`。

### 第十八轮（run 39903211796，commit `c48b048`）：P4a 起播一轮通过

- **41 passed / 0 failed**（第三十七→四十一：`playback_tests` 三条 + 新增的
  `single_file_playback_adds_no_audio_argument`），**零 Rust warning**（`grep warning` 只剩
  Actions 自己的 node 弃用提示），产物 33,349,632 字节（md5 `1b50ce45…`），
  `Cargo.lock` 与库内零差异（这一轮没加依赖，`which 8.0.6` 早在上一轮的 lock 里就解析好了）。
- 整轮 2 分 45 秒。**编译这么快是可信的**：日志里 `Compiling bpi-rs v0.3.1`（约 18 秒）与
  `Compiling rustbilipc v0.1.0`（约 23 秒）都在，`Finished` 那句写的是 42.52 秒，
  而 exe 比第十七轮大 24 万字节——不是"缓存空跑出的旧产物"。
  （依赖图本身全命中缓存，所以只重编这两个 crate。）
- 写代码前用 vendored 源码把新用到的接口逐条核过，是这轮一次过的原因：
  `VideoPlayUrlParams` 的 setter 名与参数类型、`DashStream/DurlInfo/SupportFormat/
  PlayUrlResponseData` 的字段名与类型、`Bvid::new(impl Into<String>)`、
  `BpiError::unsupported_response(impl Into<String>)`、`impl IntoElement for SharedString`
  （`gpui/src/elements/text.rs:378`）。
- 顺手补上一条 mpv 的机制证据（写进了第 4.13 节）：`--http-header-fields` 是
  `OPT_STRINGLIST`（`stream/network.c:37`），列表选项按**逗号**切分
  （`options/m_option.c:1403-1440` 的 `separate_input_param` + `m_option.h:644` 的
  `OPTION_LIST_SEPARATOR ','`），而**重复给同一个列表选项是"替换"不是"追加"**
  （`parse_str_list_impl` 默认 `OP_NONE` 走 `VAL(dst) = res`）。
  所以多条头必须拼成一个逗号分隔的参数——我们现在的写法是对的；
  反过来说，cookie 值里一旦出现裸逗号就会被切成两条坏头。对着现场凭据核过：
  四个 cookie 字段都不含逗号与空格（含 222 字符的 `SESSDATA`），拼接安全。
- **待真机验证**（P4a）：详情页点"播放选中分P"能不能拉起 mpv 并出声（DASH 双流走
  `--audio-file`）；`player_note` 打出的档位/编码是 `hev1/hvc1` 还是 `avc1/av01`
  （风险表 #14：Skylake 无 AV1 硬解）；换清晰度时"旧窗口收掉、新窗口起来"是否符合预期；
  返回上一页或关窗口后 mpv 是否真的没了（风险：还有声音就是 `Drop` 没生效）。

### 第十八轮真机结果（2026-10-23）：playurl 对了，mpv 窗口没出现

- 日志证据（`publish/debug.log`，同一份现场）：
  ```text
  05:58:08.347 playurl 选好流：qn=80 codecs=hev1.1.6.L150.90 音频=true 用时=160.0093ms
  05:58:08.361 已拉起 mpv：pid=3452 管道=\.\pipe\rustbilipc-9040 音频=true 附加头=1 条
  05:58:09.710 再次起播，收掉上一个 mpv pid=3452
  05:58:17.305 详情页退出，收掉 mpv pid=4740
  ```
  取地址、选流、封顶（请求 116 实际给 80，账号能给的档）、HEVC 命中、进程拉起与回收**全都对**；
  唯独用户看不到任何窗口。
- **根因不在 mpv 的存活，而在 mpv 的命令行语法**：`build_arguments` 把 `--user-agent`、
  `--referrer`、`--http-header-fields` 的值当作**下一个独立参数**给出，而 mpv 只接受
  `--opt=value`（外加单连字符 legacy `-opt value`，`ref/mpv/DOCS/man/mpv.rst:440-444`）。
  本机拿真 mpv 复现，报错与退出原因就是它自己给的：
  ```text
  Error parsing commandline option user-agent: option requires parameter
  Make sure you're using e.g. '--user-agent=value' instead of '--user-agent value'.
  Exiting... (Fatal error)
  ```
  这是**建窗口之前**的致命错误，所以现象是"进程活着几百毫秒，窗口一次都没出现"。
- 第二条线索同时暴露：PATH 上 `PATHEXT` 把 `.COM` 排在 `.EXE` 前面，`which("mpv")` 命中的是
  `C:\Program Files\mpv-lazy\mpv.com`（3.5 KB 的控制台启动器，会再派生 75 MB 的 `mpv.exe`），
  我们 `kill` 的只是外层。按用户定稿改为**只认运行目录的 `mpv.exe`**，`which` 依赖删除。
- 第十八轮真正缺的是**可排查性**：那一轮日志里连一行 mpv 的输出都没有，所以我们只能反推。
  现在 `launch()` 把 mpv 的 stderr 逐行写进 `debug.log`（前缀 `mpv:`），下一轮再有"窗口没出现"，
  原因会直接是日志里的第一句。管道名也补上了实例序号，
  避免"刚 kill 的上一个 mpv 还在听同名管道"（mpv 用 `PIPE_UNLIMITED_INSTANCES`，同名不报错）。
- 三条改动都用本机真 mpv（v0.41.0）跑过：改成 `--opt=value` 之后，同一套参数只剩
  `Failed to open https://nonexistent.invalid/v.m4s.` 这一条网络错误，再没有 option 解析错误。
  中途试过加 `--no-ytdl`（想省掉一句没用的 `youtube-dl failed` 噪音），查源码发现它在
  `#if HAVE_LUA` 里、不带 Lua 的构建会当成未知选项，**遂放弃**：不能为了日志好看再引入一个
  "某些构建不出现窗口"的写法（第 4.3 节）。

### 第十九轮（run 39910738892，commit `87e87e9`）：起播修好——`--opt=value`、运行目录 mpv、stderr 进日志

- **43 passed / 0 failed**（上一轮 41 → 加 `options_never_take_their_value_as_a_separate_argument`
  与 `mpv_is_resolved_from_the_run_directory_not_path`），**零 Rust warning**，
  产物 33,353,216 字节（md5 `4905bb51…`）。
- 删掉 `which` 依赖后 `Cargo.lock` 确实变了（我们的包少一行 `which`，`which 8.0.6` 仍在图里，
  由 `gpui-pre-util` 带着）。**但没有冷编译**：整轮 4 分钟、`Finished … in 40.69s`，
  日志里照样只有 `Compiling bpi-rs` + `Compiling rustbilipc`。
  原因是 workflow 的 `restore-keys` 写了 `cargo-win-…-release-` **前缀**，key 未精确命中时
  Actions 会回退到最近一份同前缀缓存。也就是说：**改 Cargo.lock 的代价不是重编依赖图**，
  真正会掉缓存的是 `runner.os`/路径变化那类。这条修正了第 8 节早先"改 lock 就可能全量重编"的担心。
- 三处真机取向一起落进代码：参数用 `--opt=value`、mpv 只认 `<exe 同目录>/mpv.exe`、
  mpv 的 stderr 逐行进 `debug.log`（前缀 `mpv:`）。外加管道名带实例序号。
- **待真机验证**（P4a 复看）：窗口应当出现并出声；`debug.log` 里应当能看到
  `已拉起 mpv：pid=… exe=C:\…\publish\mpv.exe 管道=\.\pipe\rustbilipc-<pid>-1 …`
  后面跟着 mpv 自己的几行（`cplayer: Playing…`、解码器/VO 的名字）。
  如果这次还没窗口，日志里的 `mpv:` 行会直接给出原因，不用再猜。

### 第二十~二十三轮（P4b-1 遥控）：三轮都是我自己的错，第四轮绿

| 轮 | run | SHA | 结果 | 唯一失败点 |
| --- | --- | --- | --- | --- |
| 二十 | 39921240482 | `151a5cd` | 编译失败 E0308 | `properties.iter()` 的元素是 `&&str`，我用 `|&(index, name)|` 去解 `(usize, &&str)` |
| 二十一 | 39921927646 | `29475d7` | 编译失败 E0599 | `Vec::replace_range` 不存在（那是 `String` 的；`Vec` 上同名方法来自 `bstr::ByteVec`） |
| 二十二 | 39922602530 | `351a1c9` | 编译过、56 passed / 1 failed | 我的**断言**写错：`line()` 的定稿是"连上遥控就显示 `进度 · 状态`"，我却期望裸 `"00:00"` |
| 二十三 | 39923268051 | `60b8131` | **57 passed / 0 failed、零 Rust warning** | — |

- 值得留下的两点：**每一轮都只报一个错**，这本身是强信号——第二十一轮那个 E0599 说明新写的
  tokio 命名管道 API（`try_read`/`try_write`/`ready` + `select!`）、pump 的借用、`PlayState`
  与详情页的接线全都通过了类型检查；第二十二轮编译全过、57 条里只有我的期望值错。
  也就是说这三条没有一条是环境问题，全是"本地没有 Rust 工具链，却凭印象写 API/期望值"的代价，
  已写进 QODER.md 第 5 节那条新规矩。
- **给待办项留一条真实的健壮性修复**：`pump` 里"写出去多少就丢掉多少"最初写成
  `pending.drain(..written);`——`Drain` 是不消费的迭代器，会撞上本项目的
  `unused_must_use = "deny"`；换成 `pending = pending.split_off(written)`。
  同时补了 `Ok(0)` 分支：一次都没写进去必须退出那一轮去等下一次可写，否则 `pending` 非空、
  `written` 恒为 0，就是在共享的 API runtime 线程上死转。
- 产物：33,493,504 字节，md5 `894054d8…`（上一版是 33,353,216 / `4905bb51…`）。
  `Cargo.lock` 与库内**零差异**——给 tokio 开 `net` feature 不动 lock：feature 不写进 lock，
  而 `mio`/`socket2`/`windows-sys` 本来就在图里（也解释了为什么这一轮又是 4 分钟出产物）。
- ~~**待真机验证**（P4b-1）：进度行走 `property-change`、暂停按钮文案、±10 秒、断开检测、日志里那两行
  IPC 记录~~ —— **这五条随第 4.14 节的代码一起作废**（范围定稿后删净，见下面那一节）。
  进度/暂停本来确实是这样验证的，留个记录免得以后以为"没验过"。

### 第二十四轮前：范围纠正 = 删掉播放期 UI，再按参考项目重做视觉

这轮**不是一次 CI 记录，是一次方向纠正**，所以单独写。

- 触发语（用户原话）：「视频播放相关的交给 MPV 就好了，rustBiliPC 是做视频播放之外的，
  包括主页、搜索、个人、视频详情、评论、分集/分P等、清晰度等，至于播放时间什么的不需要也不要做进去」
  「我说了 GUI 样式要参考 ref/Bili.Copilot，你这个太丑了」。
- **我做错了什么**：第二十三轮那版把"遥控"当成了目标，做出了一条进度行 + 暂停/±10 秒按钮，
  而 mpv 自己的 OSC 本来就有这些，还比我做得好。我把力气花在"重造播放器的轮子"上，
  没花在客户端该做的东西上（主页、评论区、清晰度呈现）。代价是三轮编译 + 一整块要删的代码。
  这是"先确认边界再动手"的又一次教训，已写进 QODER.md 的红色线。
- **删掉什么**：`src/player/ipc.rs`、`src/player/state.rs` 整两个文件（`git rm`），
  `launch()` 的返回值从 `(Child, pipe_name)` 退回 `Child`，详情页去掉播放控制条与状态轮询，
  `Cargo.toml` 收回 tokio 的 `net` feature，测试里 `ipc_tests`/`play_state_tests` 与管道名/命令构造
  那几条一并删除（断言从 57 条降到本来的集合）。机制与坑**没有删**，全留在第 4.14 节。
- **改了什么（P5 视觉）**：新增 `src/ui/palette.rs`（颜色/尺寸唯一出处）与 `src/ui/layout.rs`
  （页头/分隔线/侧栏项/头像/统计行/提示/卡片壳），重写 `src/ui/video_card.rs`（232×238 卡、
  232×122 封面、徽章黑 60%、`flex_wrap` 网格 gap 20、hover 只换底色），
  外壳换成 240 侧栏 + 48 应用名块 + `#202020` 页面底，搜索/我的/详情统一成 56 页头 + 1px 分隔线，
  详情右栏 360、选集行最小高 40 + 每行常驻 2px 边框（未选中透明、选中主色）。
  所有数字来源见第 4.15 节。
- **这轮的验收方式**：视觉我只能保证数字抄自 XAML，**不能保证好看**——本地没工具链、也没眼睛。
  产物出来后请他看的四处：卡片网格节奏与留白、卡片文字三行的顺序（UP 主在上）、
  页头/侧栏比例、详情右栏的高度分配。

### 第二十四轮（P5 视觉 + 范围纠正）：2 个错，都是我自己漏的收尾

run 39941368216，SHA `12e63aa`，`cargo build` 在 2 分 23 秒后失败（10:45:41→10:48:04）：**E0308 ×2、零 warning**。

- **信号很强的一条**：这一轮把全新写的 `ui/palette.rs`、`ui/layout.rs`、重写后的
  `ui/video_card.rs` 与四个页面的排版**全部过了一遍类型检查**——所有样式方法名
  （`rounded`/`border_2`/`gap`/`min_h`/`h_full`/`line_clamp`/`hover`…）与 `hover(|style| style.bg(..))`
  这个形状都是对的，报错的只有两处收尾，而且都不是"API 猜错"：
  1. `pages/detail/view.rs:567` 的 `content()` 签名是 `-> AnyElement`，链子的结尾忘了
     `.into_any_element()`，于是"expected AnyElement, found Div"。
  2. `pages/mine/view.rs:214` 那个 `match` 的两个分支元组类型不一致：一边全是 `String`，
     另一边的最后一格塞了 `self.profile_note.clone()`（`SharedString`）。
     改法是让这一格也产出 `String`（`to_string()`），不是把上面那格改成 `SharedString`。
- **教训**：改签名之后要把函数体最后一步跟着改完（`AnyElement` 的返回点必须显式转换）；
  以及"`X` 与 `X.clone()` 在同类型分支里才对称"——`SharedString` 与 `String` 看着都能当文本，
  但元组/枚举的两个分支必须同型。**这类错本地能查：编译器已经把话说全了，是我没检查返回点。**
- 下一轮（第二十五轮）修这两处，其余不动。

### 第二十五轮（P5 视觉）：绿了，40 passed、零 Rust warning

| 项 | 值 |
| --- | --- |
| run | 39943283279（手动触发，SHA `89d97a7`） |
| 结果 | **success**：Build、Test、Collect、Upload 全绿 |
| 测试 | `40 passed; 0 failed`（另两条 0 passed 是 bin 与集成测试的空集合） |
| warning | **Rust 侧 0 条**；日志里那 7 条 warning 全是 runner 自己的（`save-always` 弃用、node 的 `--trace-deprecation`） |
| 耗时 | 11:02:25 → 11:06:42（UTC），**4 分 17 秒**；`Compiling rustbilipc` 出现两次（build 与 test），`Finished … in 1m 08s` + `6.77s` |
| 产物 | `rustbilipc.exe` 33,404,416 字节，md5 `a6796377…`（上一版含遥控时是 33,493,504 / `894054d8…`，**小了约 87 KB**，与"删掉整块 IPC + 收回 tokio `net` feature"的方向一致） |
| Cargo.lock | 与库内**完全一致**（`diff` 无输出）——删 feature 不动 lock，这条再次确认 |

- **为什么"快"这次可信**：不是只看时长，而是日志里确实有 `Compiling rustbilipc` 与 `Finished`，
  且产物字节数与 md5 都变了（QODER.md 第 5 节那条）。缓存命中只重编本项目，符合预期。
- **这一轮编出来的东西代表什么**：`ui/palette.rs`、`ui/layout.rs`、重写后的 `ui/video_card.rs`
  与四个页面的排版**全部通过了类型检查**，包括 `hover(|style| style.bg(..))` 这个形状
  （`StyleRefinement` 自己实现 `Styled`，`ref/gpui/gpui/src/style.rs:324`）。
  所以宏生成的那些 tailwind 式方法名（`border_2`/`rounded`/`gap`/`min_h`/`h_full`…）都是真实存在的。
- **还没证明的只有"好不好看"**：编译器不知道比例、留白、字号层级对不对。
  交给真机看的四处（第 4.15 节末尾）：卡片网格节奏与留白、卡片文字三行顺序（UP 主在上）、
  页头/侧栏比例、详情页右栏高度分配。
- 本轮起交付物是 `publish/rustbilipc.exe`（**构建自 `89d97a7`**）；紧接着这一份 §8 记录是纯文档提交，
  按 QODER.md 的例外不再烧一轮编译。

### 第二十六轮（真机反馈：搜索框白底 → 全局主题根本没切暗色）

| 项 | 值 |
| --- | --- |
| run | 40203358610（SHA `163579c`） |
| 结果 | **success**，14:40:43 → 14:43:19（**2 分 36 秒**，缓存命中） |
| 测试 | `40 passed; 0 failed` |
| warning | Rust 侧 **0 条**（日志里只剩 runner 的 punycode/url.parse 弃用提示） |
| 产物 | `rustbilipc.exe` 33,413,120 字节，md5 `fb222c5a…`（上一版 33,404,416 / `a6796377…`） |
| Cargo.lock | 与库内**零差异**（这轮没碰依赖） |

- **他报的是现象，根因在上一层**：原话是"右上角的搜索框你也做成黑底白字"。真去查 kit 的源码才发现
  `gpui_kit::init` 走的是 `Theme::change(ThemeMode::Light, …)`（gpui-component 0.8.0
  `src/theme/mod.rs:40`）——**整个应用一直是亮色主题**，只有我们自己的 div 是暗色。
  如果照字面去"给那个 Input 刷一层黑底"，下一个 kit 组件（按钮、弹层、滚动条）还会是白的。
  机制与映射写在第 4.15 节最后两条。
- **顺手抓到的一条**：`Button::primary()` 读 `tokens.button_primary`（`button/button.rs:1002`），
  不吃 `colors.accent`——所以前面十几轮我们以为"品牌粉已经生效"，其实主按钮一直是亮色默认的那支。
  这类"改了没报错也没生效"的东西只能靠读组件源码确认，编译器不会告诉你。
- **代理口换了**：7887 在第二十五轮的文档推送上重试到第 8 次才通，用户随后定口 `127.0.0.1:7890`；
  QODER.md 第 3 节已改。**仓库级 `git config` 里仍是 7887，没动它**，推送一律用
  `-c http.proxy=… -c https.proxy=…` 覆盖。

### 第二十七轮（P6a 主页）：一次过，43 passed、零 Rust warning

| 项 | 值 |
| --- | --- |
| run | 40205254346（SHA `dd2bf73`） |
| 结果 | **success**，15:05:05 → 15:08:46（**3 分 41 秒**，缓存命中） |
| 测试 | `43 passed; 0 failed`（新增 3 条 `home_tests`：`goto` 过滤、`stat` 缺失写空不写 0、批次大小在 1–30 内） |
| warning | Rust 侧 **0 条**（日志里只有 runner 的 punycode / url.parse 弃用提示） |
| 产物 | `rustbilipc.exe` 33,575,936 字节，md5 `b95cdd66…`（上一版 33,413,120 / `fb222c5a…`） |
| Cargo.lock | **零差异** |

- 这一轮没有我自己的错，是因为动手前把接口逐条读到了行号（`docs/api-notes.md` 第 10 节）：
  WBI 是库里签的、参数只有 `ps/fresh_idx/fetch_row`、`duration` 是数字秒、
  `goto` 有三种取值——**这四条里有两条如果猜错，界面上就是"点了报错"或"时长差 60 倍"**。
- **仍然只有编译器能保证的部分**：`fresh_idx`/`fetch_row` 能不能真的"换一批"（风险表第 18 条）、
  `RcmdItem` 会不会哪天少给字段（第 17 条），都要真机看。产物已换到 `publish/rustbilipc.exe`，
  登录后第一眼应该是主页的推荐卡片网格。

### 第二十八轮（P6b 评论区）：1 个错，是 derive 的连带条件

run 40206928324（SHA `63dbcac`）2 分 32 秒失败：**E0277 ×1**。

```text
error[E0277]: the trait bound `CommentSortKind: std::default::Default` is not satisfied
   --> src\api\comments.rs:107:5
    | pub sort: CommentSortKind,   // CommentPage 派生了 Default，字段就必须都能默认
```

- **教训（本地就能查的那类）**：`#[derive(Default)]` 不是免费的——它要求**每个字段类型都能默认**。
  我给 `CommentPage` 派了 `Default`，里面的 `sort` 是自己的枚举，没派 `Default` 就整体不成立。
  修法是给枚举 `#[derive(Default)]` + `#[default] Hot`（默认"最热"，与参考项目一致），
  而不是把 `CommentPage` 的 `Default` 删掉：外壳与单测都靠它做初值。
  编译器直接把 help 写出来了（`consider annotating ... with #[derive(Default)]`），
  这种错**不需要联网查证**，是我在写 `derive` 时没顺着看一眼字段。
- 顺带说明这轮的信息量：整份 `bpi-rs`（含新开的 `comment` feature）与本项目其余部分都编过了，
  评论区的接口调用、详情页那一块的排版、`layout::avatar(&str)` 这些**都没报错**；
  也证实了"开 `comment` feature 不动 `Cargo.lock`"（`comment = []` 是纯 cfg）这条判断。
- `cargo build` 失败 ⇒ 这轮**没跑到 `cargo test`**，所以 `comment_tests` 五条还没被编译过；
  第二十九轮才是一次完整的验证。

### 第二十九轮（P6b 评论区）：绿了，48 passed、零 Rust warning

| 项 | 值 |
| --- | --- |
| run | 40207402041（SHA `edd553e`） |
| 结果 | **success**，15:32:10 → 15:35:03（**2 分 53 秒**，缓存命中） |
| 测试 | `48 passed; 0 failed`（比上一轮多的 5 条就是 `comment_tests`：类型代码、`has_more` 两种口径、相对时间各档、排序标签） |
| warning | Rust 侧 **0 条** |
| 产物 | `rustbilipc.exe` 34,055,680 字节，md5 `eeb8c6ff…`（上一版 33,575,936 / `b95cdd66…`，**大了约 470 KB**，与"新增评论区 + 打开 `comment` 模块"相称） |
| Cargo.lock | **零差异** ⇒ 第二轮确认"开 feature 不动 lock、不掉缓存" |

- 交付：`publish/rustbilipc.exe` 已经是这一版；取产物照旧只搬 exe，
  `debug.log` 与 `data/` 现场没碰。
- **真机要看的三件事**（都在这一轮新增的路径上）：① 进详情页往下翻，评论区有没有内容
  （没有内容就先看 `debug.log` 里"评论区返回 N 条（总数 …）"那行，再怀疑类型代码 1）；
  ② "最热/最新"两颗按钮点下去有没有换列表、有没有回到第 1 页；
  ③ "下一页"在总数取满时应当自己停住。
- 剩下的仍是编译器管不到的：`fresh_idx` 的"换一批"（第 18 条）与楼中楼展开（没做，见第 4.17 节末尾）。






### 第三十轮（P7 用户空间）：红，三条编译错，全是我自己的写法问题

| 项 | 值 |
| --- | --- |
| run | 40209688820（SHA `0e71db4`） |
| 结果 | **failure**，`cargo build --release` 16:03:44 → 16:04:27（**约 43 秒**就挂了，编译缓存全命中） |
| 错误 | `E0282` ×2（`api/space.rs`）+ `E0308` ×1（`ui/video_card.rs`） |
| 产物 | 无（没走到打包） |

三条错没有一条是库或接口的问题，全是我新写的形状：

1. **`let videos = …collect()` / `let posts = …collect()` 推不出类型**（`space.rs:245`、`:286`）。
   `.map(→ SpaceVideo).collect()` 的收集目标是泛型 `C: FromIterator<_>`，编译器不会替我挑 `Vec`；
   上一行 `let data = …` 的类型要等我用到 `data.list.videos` 才定得下来，于是它把整条链判成"类型未知"。
   修法是**给 `let` 写显式类型**（`let videos: Vec<SpaceVideo> = …`），不是给闭包加标注。
   教训：`collect()` 只有在返回值能由函数签名或 `collect::<Vec<_>>()` 决定时才免标注。
2. **`badge()` 签名写成 `-> impl IntoElement`，外层 `badge_or_nothing` 要返回 `AnyElement`**（`video_card.rs:418`）。
   `impl IntoElement` 是不透明类型，`AnyElement` 是具体类型，两者不能直接对接——`badge()` 函数体
   本来就以 `.into_any_element()` 结尾，签名改成 `-> AnyElement` 就一致了。这是本项目第二次栽在
   "复用件到底返回哪个类型"上（第一次是详情页 `content()` 忘转 `AnyElement`），
   **规矩：会被别的函数当值返回的复用件，签名一律写 `AnyElement`。**

顺手把两处真实缺陷一起带上（不是编译错，是逻辑与卫生）：

- **页数算错**：`video_controls` 里写的是 `total / ps + 1`，总数正好是 30 的整数倍时多算一页，
  用户点"下一页"看到一片空。抽成 `api::space::last_page(total, page_size)`（向上取整、至少 1 页、
  `page_size=0` 不除崩），并补一条边界单测（0/1/30/31/60 + ps=0）。
- **四个"从来没被读"的字段**：`SpaceProfile.mid`/`followed`、`SpaceVideo.tid`、`SpacePost.opus_id`。
  我们是二进制 crate，`pub` 字段一样进 `dead_code` 分析，留着就是 warning；
  而且它们确实是"顺手抄接口"抄进来的——页面用的是自己的 `mid`，图文点不开详情所以不需要稳定 key。
  删掉，并在类型文档里写明"为什么不留"，免得下一轮又被"补回来"。

### 第三十一轮（第三十轮那三条错的修复）：绿了

| 项 | 值 |
| --- | --- |
| run | 40210553306（SHA `203723b`） |
| 结果 | **success**，16:12:17 → 16:15:52（**3 分 35 秒**），`cargo build --release` 1m 23s |
| 测试 | `54 passed; 0 failed`（48 + P7 那 6 条：`tlist` 排序、畸形输入、`tid` 回退键、封面比例、`last_page` 边界、页签/排序文案） |
| warning | Rust 侧 **0 条**（那四个没人读的字段删掉后才做到的） |
| 产物 | **没装**——这一版还带着"chips 的『全部』跟着筛选变数"那个问题，直接等第三十二轮 |

### 第三十二轮（P7 完整交付）：绿了，54 passed、零 Rust warning

| 项 | 值 |
| --- | --- |
| run | 40210920022（SHA `40f556d`） |
| 结果 | **success**，16:16:54 → 16:20:09（**3 分 15 秒**，缓存命中） |
| 测试 | `54 passed; 0 failed`（与上一轮同数，这一轮只动界面计数） |
| warning | Rust 侧 **0 条** |
| 产物 | `rustbilipc.exe` **34,460,672** 字节，md5 `86297ed0…`（上一版 34,055,680 / `eeb8c6ff…`，**大 405 KB**，与"新增空间页 + 打开 `opus` 模块"相称） |
| Cargo.lock | **零差异** |

- 交付：`publish/rustbilipc.exe` 已是这一版；取产物只搬 exe，`debug.log` 与 `data/` 现场没碰。
- **真机要看的几件事**（都是编译器管不到的）：
  ① 详情页点 UP 主那一行——头像/昵称/粉丝数有没有出来（`debug.log` 里应有"空间资料卡返回 …"与"空间投稿返回 N 条"两行）；
  ② 投稿的**分区 chips** 有没有出现、点一格下去列表有没有换、"全部"那一格的数是不是各分区之和；
  ③ 排序三档 + 上一页/下一页（页数现在是向上取整，总数正好 30 的整数倍时不该再多一页）；
  ④ **图文**页签：竖图有没有被裁、"加载更多"能不能续上（游标 `offset` 是接口回什么我们存什么）；
  ⑤ **收藏**页签：列不列得出来（TA 的全公开才给，私有收藏夹不会返回，那一格写"TA 没有公开的收藏夹"）；
  ⑥ 下钻连续性：空间页里点卡片 → 又进详情页 → 再点 UP 主 → 又进空间，"返回上一层"是逐层退而不是清空。
- 已知没做（不是遗漏，是决定）：收藏夹**只列不钻**、关注/取关是写操作不在清单里、图文点不开详情。

### 第三十三轮（P8 外壳重构）：红一条，是我多订了一次事件

| 项 | 值 |
| --- | --- |
| run | 40248735645（SHA `3bc3efc`） |
| 结果 | **failure**，`cargo build --release` 00:20:39 → 00:21:09（约 30 秒） |
| 错误 | `E0277` ×1：`shell.watch_videos(&favorites, ..)` 要求 `FavoritesPage: EventEmitter<OpenVideo>` |

这条不是手滑，是**结构没对上**：收藏页按用户定稿"只列收藏夹、不下钻"，那一行不是可交互元素，
所以它本来就不该实现 `OpenVideo`。上一版个人中心能点是因为历史卡片在那一页里；拆开之后
我照抄了"每个入口页都订一次"的写法。改法是**去掉那次订阅**（而不是给收藏页加一个没人发的事件），
并把"哪些页要订、为什么"写在调用点——这类把关交给类型系统是对的，编译错正好指出多余的一条边。

### 第三十四轮：编译全过、零 warning，红在一条我自己写错的断言

| 项 | 值 |
| --- | --- |
| run | 40249047594（SHA `75802a2`） |
| 结果 | **failure**，但 `Build (release)` **成功**：59 passed / 1 failed，Rust warning **0 条** |
| 失败 | `auth_tests::broken_auth_text_is_treated_as_not_logged_in`，`src/tests/mod.rs:188` |

我按"新增字段一律 `#[serde(default)]`"的惯例，断言"只有两条 cookie 的旧文件也能解析"。
实际上 `dede_user_id/sessdata/bili_jct/buvid3` **本来就没有 default**，而且这是故意的：
拿一份缺 buvid3 的半成品去构造 `Account`，只会在第一个接口上撞 `-101`，比"重新扫一次码"难查得多。
断言改成"缺必填键 → `None`（当成没登录）"，另加一条"四条键齐但值是空串 → 能解析、由 `is_complete` 判不合格"，
把两种失败分开钉住。

### 第三十五轮（P8 完整交付）：绿了，60 passed、零 Rust warning

| 项 | 值 |
| --- | --- |
| run | 40249382084（SHA `0d659a8`） |
| 结果 | **success**，00:26:39 → 00:29:30（**2 分 51 秒**，缓存命中） |
| 测试 | `60 passed; 0 failed`（第三十二轮 54 → 新增 `config_tests` 4 条 + `auth_tests` 1 条 + `profile_tests` 的 `progress_text` 1 条） |
| warning | Rust 侧 **0 条** |
| 产物 | `rustbilipc.exe` **35,793,920** 字节，md5 `b2687551…`（上一版 34,460,672 / `86297ed0…`，**大 1.33 MB**，与"侧栏图标改挂 `AllAssets`（1830 个 svg 全嵌）+ 五个新页面"相称） |
| Cargo.lock | **零差异**（`historytoview` feature 早就开着，这一轮没动任何依赖） |

- 交付：`publish/rustbilipc.exe` 已是这一版；只搬 exe，`debug.log`、`data/cookies.json` 现场没碰。
- **他第一次跑这一版时会发生的事**（都是有意的）：`publish/auth.json` 还不存在 → 读一次旧的
  `publish/data/cookies.json`，字段合格就迁移写出 `auth.json`（**旧文件保留不删**）；
  `config.json` 不存在就写一份默认的出去。日志开头会有"从 … 迁移凭据到 …"与"已写入默认设置"两行。
- **真机要看的几件事**：
  ① 登录窗：把 `auth.json` 临时改名再启动，应该只弹一个 320×420 的小窗（只有应用名 + 二维码 + 状态 + 重新获取）；
  扫完码应当**直接进主窗口而不是闪退**（第 4.19 节那条顺序）。
  ② 侧栏：宽 80、七项图标 + 两字都画得出来（**图标是一片空白就说明名字找不到**，见第 4.20 节）、
  左上是自己的头像且点不动、左下角没有 mpv 那行、进详情/空间后第一项是"返回"。
  ③ 五个入口页：收藏列收藏夹（点不动是对的）、历史一屏 30 条、待看有没有内容 + "看到 xx:xx"、
  下载页是那句实话、设置页九个字段有没有值（尤其"画质上限"与"播放内核"）。
  ④ 播放：起播仍应封顶 1080P60（`config.json` 手改 `max_quality` 成 64 重启，日志里 playurl 的 qn 应当跟着变）。
  ⑤ 下钻与返回仍然连续（详情 → 空间 → 点卡片 → 详情）。

### 第三十六轮（P9 收藏夹下钻 + 三种搜索）：红两条，同一个形状

| 项 | 值 |
| --- | --- |
| run | 40251667656（SHA `50c07d1`） |
| 结果 | **failure**，`cargo build --release` 00:57:13 → 00:57:41（约 28 秒） |
| 错误 | `E0277` ×2，都在新写的 `ui/layout.rs:355`/`:362`（`clickable_row` 的 `.child(label)` / `.child(meta)`） |

泛型参数 `impl Into<SharedString>` **本身不是元素**：`ParentElement::child` 要的是
`impl IntoElement`（`gpui-pre-0.3.9/src/element.rs:194`），所以必须先 `.into()` 转成
`SharedString` 再交给 `child`。旁边的 `list_row` 早就这么写了——新复用件没照自己旁边那一段抄。
（与第三十二轮那条同类，第二次踩；`docs` 里"会被当值用的泛型文本要先转成具体元素类型"这条要记住。）

### 第三十七轮：绿，但那一版还差一条行为修复

| 项 | 值 |
| --- | --- |
| run | 40252030871（SHA `ab42887`） |
| 结果 | **success**，01:00:07 → 01:04:01（3 分 54 秒），`62 passed`、Rust warning **0 条** |
| 产物 | **没装**：工作区里还有一条"翻第 N 页失败时保留已加载条目"的行为修复未提交 |

顺手记两条查证结果，免得下一轮又凭感觉：`SharedString` **有** `Display`
（`ref/gpui/gpui_shared_string/gpui_shared_string.rs:80`），所以 `format!("{}", s)` 合法，
但项目里一律写 `.as_str()`（详情页一直这么写），跟着齐；kit 的 `Button::label` 收
`impl Into<SharedString>`（`component/src/button/button.rs:373`），`format!` 的结果可以直接传。

### 第三十八轮（P9 交付）：绿了，62 passed、零 Rust warning

| 项 | 值 |
| --- | --- |
| run | 40259685138（SHA `22c2f87`） |
| 结果 | **success**，02:33:32 → 02:36:57（**3 分 25 秒**，缓存命中） |
| 测试 | `62 passed; 0 failed`（第三十五轮 60 → 新增 `favorites_tests` 两条：`pick_bvid` 的三种键位组合、`matches_keyword` 的空串放行与大小写） |
| warning | Rust 侧 **0 条** |
| 产物 | `rustbilipc.exe` **36,061,696** 字节，md5 `b7aeeaa1…`（第三十五版 35,793,920 / `b2687551…`，**大 268 KB**，与"收藏夹内容页 + 三处输入框 + 共用件"相称） |
| Cargo.lock | **零差异**（这一轮没动依赖，`fav` 模块本来就在开着） |

### 真机日志的第一份证据（第三十五轮的 exe，2026-10-26 10:32 那一次运行）

`publish/debug.log` 里这一段把 P8 的启动链子逐环证明了，其中三条正是"编译绿不等于能用"的那部分：

- `设置就绪：max_quality=116（…\publish\config.json）` —— 配置文件读到、路径显示正确；
- `启动判定：凭据文件 …auth.json（可读=true，字段完整=Some(true)）` + `本机已有凭据，直接开主窗口 mid=…`
  —— **旧 `data/cookies.json` 的迁移在这之前那次运行已经完成**，这次走的是"直接开主窗"分支；
- `提交 个人资料 … mid=…` → `space_info 返回 name=CatMeo2430 level=3 用时=435ms` →
  `页面加载：外壳头像 结果回到前台（请求 #1）` → `主页推荐返回 12 条（过滤后 12 条是稿件）`
  —— 侧栏头像那一路与主页那一路都真的回来了。

同一份日志结尾（02:32:39，也就是关掉窗口的瞬间）有四行 gpui 自己的错误：
`window not found` + `无效的窗口句柄 (0x80040102/0x80070578)` ×3，紧跟 `rustls … CloseNotify`。
发生在**进程退出时的窗口销毁**这一段，界面已经关完，所以不影响使用；记在这里是为了下次看到
"退出时红四行"不用怀疑是我们把窗口顺序写错了。（登录→主窗那条换窗口路径这一次没走到，
因为本机已经有 `auth.json`；要验那条得把 `auth.json` 临时改名再启动。）

### 第三十九轮（P10 触摸屏滚动 + 侧栏换页）：绿了，65 passed、零 Rust warning

| 项 | 值 |
| --- | --- |
| run | 40262415547（SHA `014b71c`） |
| 结果 | **success**，03:06:39 → 03:11:08（**4 分 29 秒**） |
| 测试 | `65 passed; 0 failed`（第三十八轮 62 → `touch_scroll_tests` 三条：往下拖回顶、往上拖到底、不越界攒死区） |
| warning | Rust 侧 **0 条** |
| 产物 | `rustbilipc.exe` **36,099,072** 字节，md5 `4c813442…`（上一版 36,061,696 / `b7aeeaa1…`，只大 37 KB，与"每页多两行钩子 + 一个新模块"相称） |
| Cargo.lock | **零差异**（没动依赖：这套做法只用 gpui 已有的东西） |

一次过、没有红轮。**这轮赌的机制是"Windows 把触点注入成鼠标消息"**：它在 CI 侧证明不了，只能靠手指划——
**2026-10-26 他真机确认「这个好了」**，所以注入成立，`ScrollHandle` + `MouseMoveEvent::dragging()`
就是这台机器上的可用方案。`WM_POINTER*` 那条退路（要 `unsafe` + 自己抓 Win32 消息）**不需要走，
也别当待办**；要重新考虑它，得先出现"某些控件收不到注入"这种新证据。

### P9/P10 的真机日志证据（2026-10-26 10:44 那一次运行，`publish/debug.log`）

这一份把 P8 之后新加的几路全走通了，一条错误都没有：

- `设置就绪：max_quality=116` → `凭据文件 auth.json（可读=true，字段完整=Some(true)）` → `本机已有凭据，直接开主窗口`
- `space_info 返回 name=CatMeo2430 level=3` → `页面加载：外壳头像 结果回到前台` → `主页推荐返回 12 条（过滤后 12 条是稿件）`
- **历史**：`提交 观看历史：limit=30 续翻=false` → `历史返回 30 条（cursor.max=117478344627500）`（游标带回来了，"再加载"有的用）
- **收藏下钻**：`打开收藏夹内容 id=3394223239（默认收藏夹，栈深 0）` → `收藏夹内容返回 19 条（跳过 0、总数 19、has_more=false）`
  ——`跳过 0` 说明那 19 条全有稿件号，"不猜 type、看有没有 bvid"这条准入没冤枉任何条目
- **详情**：`view 返回 pages=1` → 字幕 `1 条` → 评论 `20 条（总数 Some(129)）`
- **待看**：`待看列表返回 6 条（可用 6 条）`

上一版（第三十五轮）同一段日志结尾那四行 `无效的窗口句柄` 在这次没再出现，
也就是说它确实是退出瞬间的销毁噪音，不是功能问题。

### 第四十轮（P11 选中复制 + 下载）：红，两条错都在"我以为我知道这个 API"

| 项 | 值 |
| --- | --- |
| run | 40293890622（SHA `de58bc0`） |
| 结果 | **failure**，红在 `Build (release)` 的 lib 阶段 |

编译器原话与根因（**都不是设计问题，是把两个 API 的形状记错了**）：

1. `E0614: type StatusCode cannot be dereferenced`（`src/api/download.rs:355`）。
   我写的是 `let status = *response.status();`，理由是"http 的 `status()` 返回引用"——那是 **http 0.2 的印象**，
   `http 1.5` 里它按值返回 `StatusCode`（本身 `Copy`）。**记错版本 = 白跑一轮**，
   而这轮本来已经在注释里写了"先拿走状态码再看响应体"，说明意图对、事实错。
2. `E0599: no method named flatten found for Option<Result<Option<Vec<PathBuf>>, anyhow::Error>>`
   （`src/pages/detail/view.rs:421`）。`prompt_for_paths` 的结果是**三层壳**：
   `await` 的 `Result<_, RecvError>` → 平台的 `Result<_, anyhow::Error>` → 最里的 `Option`（用户按取消）。
   我按两层写了 `.ok().flatten()`，而 `flatten` 只对 `Option<Option<_>>` 存在，对 `Option<Result<_,_>>` 不存在。
   改成 `.ok().and_then(|inner| inner.ok()).flatten()` 再 `pop()`。
3. `warning: unused variable: cx`（`src/pages/download/view.rs:124`）——**这条不是笔误，是功能缺口**：
   「用 mpv 打开」失败时只写了日志，界面上点完什么都没发生。按第 4.7 节"失败必须看得见"补了下载页自己的提示位。
   顺手修掉一个真 bug：原来 `self.player = Some(child);` 被漏掉了（`Child` 的 drop 不杀子进程，
   漏掉它就会留下关不掉的 mpv），而且 `child.id()` 写在 move 之后会编译失败——**pid 要在 move 前取**。

这一轮还留下一条**没被编译器抓到、只能靠真机**的事：`TextSelection::is_selectable_at` 用的是"当前作用域"里
登记的 `text_bounds`（第 4.25 节），我们把它放在 div 自己的 `on_mouse_down` 里问。如果那一瞬间作用域不对，
它会恒返回 `false`，症状是"在标题上划字时页面也跟着滚"。列在风险表第 24 条，别当已验证。

### 第四十一轮：绿了，75 passed、Cargo.lock 零差异，但还剩两条 warning

| 项 | 值 |
| --- | --- |
| run | 40297103213（SHA `8ef7cfe`） |
| 结果 | **success**，09:07:07 → 09:10:35（**3 分 28 秒**） |
| 测试 | `75 passed; 0 failed`（第三十九轮 65 → `download_tests` 十条 + `quality_options` 封顶一条） |
| warning | **2 条**：`src/api/download.rs:365` `unused_mut`、`src/main.rs:27` `unused_imports: gpui_kit` |
| Cargo.lock | **零差异**——这一轮往 `Cargo.toml` 加了 `futures`（只为 `AsyncReadExt`），
  lock 里那条依赖边是我手写的，CI 不用 `--locked` 重解之后取回来 diff 是 0 行，说明手写的那条正好就是答案 |

`main.rs` 那条 glob 为什么现在才冒出来：**它属于 bin 目标**，而第四十轮在 lib 阶段就红了、根本没编到 bin。
教训是"零 warning"这个口径只有**整轮全绿**才作数——只看 lib 那几行会漏掉 bin 与 test 两个目标。

### 第四十二轮（P11 交付）：绿了，75 passed、零 Rust warning

| 项 | 值 |
| --- | --- |
| run | 40298364353（SHA `4338445`） |
| 结果 | **success**，09:17:11 → 09:20:17（**3 分 06 秒**） |
| 测试 | `75 passed; 0 failed` |
| warning | Rust 侧 **0 条**（日志里 `warning:` 一处也没有） |
| 产物 | `rustbilipc.exe` **36,417,024** 字节，md5 `dd020f45…`（上一版 36,099,072 / `4c813442…`，+317 KB，与"下载整条链路 + 选中文本元素 + 队列实体"相称）；已只覆盖 `publish/rustbilipc.exe`，`debug.log`/`auth.json`/`config.json`/`mpv.exe`/`vulkan-1.dll` 都没动 |
| Cargo.lock | 零差异（同第四十一轮） |

**待他真机验的三条**（按好不好验排序）：① 选字与拖动滚动的分界（风险第 24 条，
如果不划算，改的是 `ui/scroll.rs::on_press` 那一句判据，不是选择功能）；
② 弹框能不能正常选到目录 + 一次完整下载（看 `提交下载到 API runtime` / `下载完成` 两段日志；
`HTTP 403` 就是时效或 cookie，风险第 25 条）；③ 触摸屏上那颗「复制选中」（选中一段评论再点它，应能粘出中文）。

顺带记一条省事的 gpui 事实：**`App::spawn` 内部是 `boxed_local()`，不要求 future 是 `Send`**
（`ref/gpui/gpui/src/app.rs:2297-2306`），所以下载队列可以在 gpui 的任务里直接 await
`tokio::sync::mpsc::UnboundedReceiver`（它 `Send` 但不 `Sync`）——不用为"跨线程"再造一层通道。

### 第四十三轮（P12 合集与系列 + 分集）：一次过，82 passed、零 Rust warning

| 项 | 值 |
| --- | --- |
| run | 40308052883（SHA `858d275`） |
| 结果 | **success**，10:31:15 → 10:34:16（**3 分 01 秒**），**没有红轮** |
| 测试 | `82 passed; 0 failed`（第四十二轮 75 → `collection_tests` 两条 + `season_tests` 五条） |
| warning | Rust 侧 **0 条** |
| 产物 | `rustbilipc.exe` **36,836,864** 字节，md5 `a591abdf…`（上一版 36,417,024 / `dd020f45…`，+410 KB，与"一个新接口模块 + 一个新页面 + 右栏一节"相称）；只覆盖 `publish/rustbilipc.exe` |
| Cargo.lock | **零差异**（这一轮一个新依赖都没加：三条合集接口都在 `video` feature 里，日期换算不引 chrono） |

**为什么这轮没红**：提交前把新写的每个第三方调用都对了一遍 vendored 源码，抓到三条必红的——
两处 `loading::spawn_into` 的写回闭包漏 `move`（闭包要读外面栈上的 `page`，而它必须 `'static`，
就是 E0373；`favorites/folder.rs` 里本来就有这条的注释，新代码还是踩了），
以及内容页在 `batch.items` 被搬走之后还 `&batch` 调 `has_more`（E0382，部分移动之后再借整个结构）。
另外三处是"能编但会说假话"：`CollectionList` 上两个没人读的计数（第 4.20 节那条老规矩）、
翻页判据用"两列之和"会在"合集 6 + 系列 5"上误报还能翻、
以及 `page.total` 若是页数就会画出"共 7 集 · 已加载 30"——现在 `total_known` 拿已加载条数校一次，
不可信就当没给。**结论：核 API 的收益是整轮，不是几分钟。**

**这一轮的 P11/P12 都还没有真机证据**：`publish/debug.log` 现在这份是 09:37–09:50 那次运行
（P10 时代的构建），结尾那四行 `无效的窗口句柄` 仍是已知的退出销毁噪音（第 8 节第三十五轮那条）。
他跑新产物之后要看的三件事按难验排序：① 分集那节在他那两个合集（尤其 189 集那个）里列全没有、
分段 chips 切得对不对、`N P` 角标是否落在真的多 P 那一集上；② 空间页第四格两类都出得来吗
（`提交 合集与系列 到 API runtime：mid=… 第 1 页` → `合集与系列返回 合集=a 系列=b`）；
③ 内容页"再加载一页"到底能不能翻到底（`提交 合集内容 …` 那行的"总数"与"本页"对不对得上）。
库里那份类型没带 `ugc_season`，所以 ① 走的是我们自己发的第二个 `view`——
它失败时日志里是 `取分集信息失败（详情页照常显示）: …`，界面上是"部分信息缺失：分集信息没取到…"。

### 第四十四轮（P13 全量一次提交）：红，5 条错

| 项 | 结果 |
| --- | --- |
| 触发 | **push 自动**（新 `ci.yml` 第一次跑，SHA `3b0bcf2`，run 40334131522） |
| 结果 | **failure**，红在 `Build (release)`；`Test` 没跑到 |
| 错误 | `E0308`×2（detail 的 `audio_url` 两个分支一个给 `String` 一个给 `Option<String>`；gate 把 `AsyncApp::update` 的返回值当 `Result` 判）、`E0502`+`E0521`×2（`cx.background_executor()` 那份句柄**借的是 `cx`**，于是 `cx.notify()` 与 `cx.spawn` 一起炸） |

**这一轮值得记的是三条新事实**（都是"文档里没写、只能实测"的那类）：

1. **`AsyncApp::update` 返回闭包的 `R` 本身，不是 `Result`**（`open_window` 那条老路之所以
   `if let Err` 成立，是因为闭包自己返回 `anyhow::Result`）。上一版 gate 的 Startup 分支照抄了那个
   写法，Relogin 分支的闭包返回 `()`，于是 `if let Err(error) = outcome` 当场类型错。
2. **`Context::background_executor()` 拿到的句柄借的是 `cx`**：第四十四轮那三条错其实是同一条——
   存进局部变量就一路占着不可变借用（`cx.notify()` E0502），把它搬进 `cx.spawn` 的 `'static` future
   更是直接 E0521。**`.clone()` 一下才是 owning 的那份**，这行不是多余的 clone（注释里写了原因）。
   `pages/login/view.rs` 里早就有 `cx.background_executor().timer(..)` 的现成写法，
   但那是"当场用掉"、不存变量，所以照抄时看不出差别 —— **看到例子要先看它有没有把值留下来**。
3. **`ci.yml` 按 push 自动跑起来了**：从推送到红只用 5 分钟（缓存全命中），
   比原来"手动 dispatch + 等排队"快得多，这就是把它拆开的收益，当场拿到证据。

### 第四十五轮：红 1 条

`Config::load_or_create` 不存在（E0599）—— 上一版把它留成了模块级自由函数 `config::load_or_create()`，
而 `main.rs` 按 `Config::load_or_create()` 调。**改法不是把调用点改掉，而是把它做成关联函数**：
三个落盘点各自一个入口（`Auth::load`、`Config::load_or_create`、`logging::init`）读起来才对称。
改的过程中差点把 `load_or_create` 塞进 `LoadOutcome` 枚举里 —— 我用来定位的那个 `}`
其实是枚举的闭合括号。**教训：跨结构体/枚举边界的替换要连上面的 `impl`/`enum` 一起读一遍再动手。**

### 第四十六轮（P13 绿）：102 passed、零 Rust warning

| 项 | 结果 |
| --- | --- |
| SHA | `85ef8ff`（run 40336309511） |
| Build | success，`cargo build --release` 58.33s（缓存命中） |
| Test | **102 passed / 0 failed**（上一版 82，本轮 +20：选流偏好 5、设置 5、mpv 定位 4、探测 6 里挑纯函数部分、自动翻页 2…… 全在 `src/tests/`） |
| Rust warning | **0 条**（日志里只剩 runner 自己的 `save-always` 弃用提示与 node 的 `--trace-deprecation` 行，那不是我们的 warning） |
| Cargo.lock | 本轮**没加任何依赖**：`futures`/`tokio` 都是原有的（探测用 `futures::future::select` + gpui 的 `timer`），要拿 diff 再核一次 |

**本轮交付之后 `ci.yml` 补了一个步骤**：原来那份只上传 `Cargo.lock`，没有 exe 的 artifact ——
按"日常真机验证取 ci 产物、`cd.yml` 才是发布"的分工，这是漏的。补上
`Upload Windows x64 release artifact`（`path: target/release/rustbilipc.exe`，
**只有 exe、`if-no-files-found: error`**），产物从第四十七轮起才有。

### 第四十七轮（P13 交付）：绿，产物补上了

| 项 | 结果 |
| --- | --- |
| 触发 | push 自动（SHA `12ce966`，run 40337802314）——本轮只动文档 + 往 `ci.yml` 补一步 artifact |
| Build / Test | success，`cargo test --release` 仍 **102 passed / 0 failed**（代码与第四十六轮那份同一个 SHA 的源码，只多了文档） |
| Rust warning | **0 条**（日志里只有 runner 的 `save-always` 弃用提示） |
| Cargo.lock | 与仓库里那份 **零差异**（从 `cargo-lock-ci` artifact 取回逐字节比过） |
| 产物 | `rustbilipc.exe` **36,980,736** 字节，md5 `efb03a94…`（上一版 36,836,864 / `a591abdf…`，+144 KB：一个新的探测模块 + 重登协调器 + 设置页四块 + 自动翻页判据）；**只覆盖 `publish/rustbilipc.exe`**，`publish/` 里他的 `auth.json`/`config.json`/`debug.log`/`mpv.exe` 一个字没动 |

**给他人话，这三条是真机第一次跑这版会立刻撞上的**（都是定稿的直接后果，不是 bug）：

1. **登录态不会跟着搬过来。** 他现在这份凭据在 `publish/auth.json`，而这版只读
   `%APPDATA%\ArrowMeo\rustBiliPC\auth.json`——旧位置按他的要求**一行兼容代码都没留**，
   所以启动会直接弹扫码小窗，**扫一次**就好（旧文件我没删，那是他某次登录的现场）。
2. **默认没有 `debug.log`。** "调试模式"默认关（他的原话），关着时连文件都不创建。
   要我看日志：设置页开"调试模式" → **退出重开一次** → `%APPDATA%\ArrowMeo\rustBiliPC\debug.log`。
3. **exe 不再弹黑色控制台窗口**（链接期 GUI 子系统）。调试模式开着也不弹，这不是回归。
   命令行想看输出的话这份 exe 已经没有 stderr 了 —— 唯一通道就是上面那条 `debug.log`。

验完这一版要看的东西（按"日志里能不能看出来"排）：
① 设置页四项改完是不是真的生效（改编码为 AV1 后日志里 `playurl 选好流：codecs=av01…`；
改 4K 后 `提交 playurl … 想要 qn=120`，同时 `.fourk` 那位才开）；
② 起播那行的前缀有没有"视频轨主地址不通，已换 xxx（a：HTTP 403）"这种换址话术
（对应日志 `CDN 探测完成 视频=N 音频=Some(M)`）；
③ 把 `%APPDATA%` 里的 `auth.json` 改坏一个字节再操作一次 → 应当**自己弹出扫码小窗**，
扫完主窗口当前那一格自动重取（日志 `重登：凭据已被接口判定为无效，删除本地凭据并开扫码登录窗`）；
④ 历史/收藏内容/搜索/合集内容滚到底自动接下一页，且**不会**看到同一页连发两次
（`滚到底，自动取第 N 页` 每次 N 递增）；
⑤ `mpv.exe` 还在 `publish/` 里也能直接播（运行目录那一档留着是故意的）。

### 第四十八轮（warning 改成机器数）：绿，**0 条**

第四十六/四十七轮的"零 Rust warning"是我从日志里**肉眼**确认的（`cargo build` 命中缓存时不会
重复报上一轮修掉的 warning，所以肉眼只能看"这轮新出现的"）。第四十八轮把这件事交给 CI 自己数：
`cargo check --release --all-targets --message-format=json` 强制前端把所有目标重跑一遍，
再 `grep -c '"level":"warning"'` 写进 summary。**结果：`### Rust warning 条数：0`**，
`cargo test --release` 仍是 **102 passed / 0 failed**（SHA `e7a337e`，run 40338758130）。

以后每轮的"零 warning"以这一步的数字为准，不再用肉眼结论。
（`cd.yml` 那份**还没真跑过**：四个目标里只有 win-x64 是长期验证的，另三个连 runner 标签带系统库
是否齐全都没证据 —— 见风险第 31 条。要不要现在发一个 Release，等他点头。）

### 第四十九轮（P13 之后全面审：八条修正）：红 1 条

| 项 | 结果 |
| --- | --- |
| 触发 | push 自动（SHA `cfae5c4`，run 40349330310） |
| 结果 | **failure**，红在 `Build (release)`，只一条：`E0425` —— 新建的 `api/probe.rs` 里用了 `StreamChoice` 却没 `use super::stream::StreamChoice;`（`mod.rs` 那层的 re-export 不会自动进子模块视野） |
| 本轮内容 | 按"设计缺陷 / 依赖 / 性能 / 占用 / 体积 / 稳定"六项过了一遍 P13 的代码，改的是**机制**不是配置旋钮 |

八条各自为什么算缺陷（都已落到代码注释里，这里只记结论）：

1. **GUI 子系统 + 没有 panic 钩子 = 静默死亡**。`main.rs` 顶部 `windows_subsystem = "windows"`
   是链接定的，进程崩了连一行字都不留。新增 `logging::install_panic_hook()`，写
   `crash.log`（**与"调试模式"无关，永远写**，超过 256 KB 滚动），并且**第一件事就是装它**——
   不然装在配置读取之后，坏配置本身引发的 panic 就记不下来。
2. **设置页每帧扫 PATH**。解析 mpv 位置原来写在 `render` 里，窗口一动就 `stat` 一遍整个 PATH。
   改成构造时与设置变化时各查一次，存 `mpv_found: Option<PathBuf>`。
3. **重登先删 `auth.json` 是 irreversible 的动作**。`-101` 有风控误判这一说，删了就是让人重扫一次。
   现在只置标志 + 开小窗；登录成功本来就会覆盖那个文件（`relogin.rs` 文件头写了这条理由）。
4. **探测串行且没预算**。三条地址一个个问、且不问"值不值得问"，最坏 3×4 秒。改成
   `futures::future::join` 两轨并行 + `MAX_PROBE_CANDIDATES = 3` 收预算。
5. **下载进度会倒退**。换备用域名时任务回报的是一个**较小**的数（那次失败写的字节不算），
   界面就往下掉。`apply` 改成只做单调增，`pull_track` 把丢弃的字节累进 `done`。
6. **日志写了带签名令牌的完整直链**。CDN 直链的 query 里是临时凭证，落到 `debug.log` 等于把
   可复用地址写进用户要发给我看的文件。改成只记主机名（`probe::host_of`）。
7. **自动翻页没有条目上限**。列表无限接下去会同时拖渲染和内存，加 `AUTO_PAGE_MAX_ITEMS = 240` 软闸。
8. **`tokio` 的 `macros` 特征没人用**（全项目没有一个 `#[tokio::main]`/`#[tokio::test]`）。去掉。
   顺带修掉取消路径的真 bug：`attempt` 取消时没 `drop(writer)` 就返回，上层删 `.part` 撞上
   "Windows 上文件还开着删不掉" —— 这正是"取消了但目录里留个半成品"的原因。

### 第五十轮（补 import）：绿，104 passed

| 项 | 结果 |
| --- | --- |
| SHA | `00b1655`（run 40350305866） |
| Build / Test | success，**104 passed / 0 failed**（上一版 102，本轮 +2：探测预算与进度单调） |
| Rust warning | **0 条**（CI 自己数的） |
| Cargo.lock | 与仓库那份零差异；本轮**只减不加**（去掉 `tokio/macros` 特征没动 lock 里的包，`tokio` 本来就在） |

**教训**：新写一个 `api/*.rs` 子模块时，第一件事是把要用的兄弟模块类型 `use` 进来，
别指望 `api/mod.rs` 里的 re-export —— `mod.rs` 那一份 `pub use` 是给**外面**用的。

### 第五十一轮（P14a/b/c：体积、依赖、下载排队）：绿，107 passed，产物小了 0.88 MB

| 项 | 结果 |
| --- | --- |
| SHA | `fcf9329`（run 40425352626，push 自动） |
| Build | success，`cargo build --release` 96s（上一轮 64s；改了 rustls 特征让依赖缓存**部分失效**，多出来的就是重编那截） |
| Test | **107 passed / 0 failed**（+3：`assets_tests` —— 用到的图标逐个查表 / 表里每项真能 `load` 且像 svg / 条目数与总字节当体积闸） |
| Rust warning | **0 条** |
| Cargo.lock | **与仓库零差异**（从 `cargo-lock-ci` artifact 取回逐字节比过）—— 也就是说 `aws-lc-sys` **没有被拿掉**，见下面第 ② 条 |
| 产物 | `rustbilipc.exe` **36,053,504** 字节，md5 `3197f5c9…`（上一版 36,980,736 / `efb03a94…`，**小了 927,232 字节**）；**只覆盖 `publish/rustbilipc.exe`**，`publish/` 里他的 `auth.json`/`config.json`/`debug.log`/`mpv.exe` 一个字没动 |

三条要动体积与依赖的，兑现程度不一样，逐条按证据说（细节在第 4.29 节）：

1. **图标只嵌我们那张表（P14a）**：兑现了。`AllAssets` 那份 1830 个 svg（磁盘 732 KB，
   嵌进 `.rdata` 比原始字节更贵）换成 110 项 / 48 KB。**先别把省下的字节全记在它头上**——
   这一轮同时还加了下载排队与图标表自身的代码，所以它是净收益里的大头但不是全部。
2. **crypto provider 换 `ring`（P14b）**：**只兑现一半**。我们这一侧的 `rustls` 依赖确实
   `default-features = false` + 只开 `ring`，运行时全进程走的也是 ring；
   但依赖图里 `aws-lc-rs`/`aws-lc-sys` **还在**——点名要它的是 `quinn-proto 0.11.19`
   （HTTP/3，随 `reqwest 0.13.5` 进来，而 reqwest 是 gpui 与 bpi-rs 带的，我们改不动它的特征）。
   所以**没有减少一个 crate、没有减少编译时间**，`choco install nasm` 那一步继续留着。
   这一条的价值只剩"运行时用哪个是我们定的" + "aws-lc 那块代码躺在二进制里不被调用"。
3. **下载排队 + 退出收尾（P14c）**：代码与测试到位，**收益要他真机验**（同时点五条看是不是
   前 3 条在跑、后 2 条显示"等前面的下完就轮到"；跑着点 X 退出，再去看目标目录里 `.part` 剩几个）。
