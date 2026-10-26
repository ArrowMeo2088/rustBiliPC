# QODER.md — Agent 协作规则

在本仓库工作的 Agent 必须遵守以下规则。

## 1. 文件结构规则

- 目录用途固定：`src/`=项目源代码，`publish/`=编译产物，`test/`=单元测试布局说明（代码在 `src/tests/`），
  `ref/`=供参考的文件和代码，`docs/`=文档，`res/`=资源文件。
- **每个项目文件夹都必须有自己的 `README.md`** 描述用途与内容。
- **每次修改某个目录的内容，必须同步更新该目录 README 与 `docs/` 对应小节，并提交 git**
  （小步可复原的提交，提交信息说清改了什么）。
- 代码**必须写注释**：模块头注释写职责与定稿理由，非显然处写"为什么"（机制、坑、被否决的方案），
  不写"是什么"。公开项写文档注释（`//!` 模块 / `///` 项）。
- `.gitignore` 按规则排除 `ref/`、`publish/`、`res/` 的**内容**（`/ref/*` 写法 + `!dir/README.md` 例外）；
  不要滥用 `git add -f` 把这些目录的内容塞进暂存区。运行期的 `data/` 与 `Cargo.lock` 规则见第 5 节。

## 2. 工具链与编译约束

- **本机没有安装 Rust 工具链，也不要安装**（不装 rustup/cargo，不在本地跑 cargo 命令做验证）。
- 编译只通过 GitHub Actions，2026-10-26（P13）按用途拆成两条：
  - **`ci.yml`**：**push / PR 自动跑**，只做 **Windows x64 release** 编译 + `cargo test`。
    这条是"改动还能不能编过"的证据通道，所以刻意不做多平台、不压 UPX（矩阵一次 20 分钟起，
    放在 push 上会让人不敢推）。
  - **`cd.yml`**：**手动触发**（`workflow_dispatch`），win/linux × x64/arm64 四个目标，
    用 `[profile.release-dist]`（LTO + `codegen-units=1` + `strip`）编，UPX 压，
    发 GitHub Release。**资产只有 exe 本身**（用户定稿："注意只需要打包 exe，不要打包其他东西"）：
    不塞 dll、不塞 mpv、不塞 README。
  - 旧的单文件 `build.yml` 已删除；不要再往"手动一条路什么都干"的方向合回去。
- **不要改成 debug**：gpui 的 Windows 后端（`gpui-pre-windows 0.3.9`）在 `cfg(debug_assertions)` 下
  于运行时用 `D3DCompileFromFile(env!("CARGO_MANIFEST_DIR")/src/shaders.hlsl)` 编译着色器，
  路径被烧进二进制，CI 产出的 debug exe 在别的机器上无法开窗。`runtime_shaders` 特性只管 macOS。
- 验证改动 = 推送后看 Actions 日志。一次冷编译 25–45 分钟，所以**动手前先把 API 签名核对准**：
  以 `ref/gpui-kit/examples/*/src/main.rs` 与 `ref/gpui-kit/crates/*/src` 里真实出现过的写法为准，
  不要用没在参考仓库里见过的调用。
- **在 `ref/gpui` 里 grep 不到某个样式方法，不等于它不存在**：`rounded`/`border_2`/`gap`/`min_h`/
  `h_full`/`p_4` 这类 tailwind 式方法是 `gpui-pre-macros` 的 `styles.rs` 按"前缀表 × 后缀表"生成的
  （`styled.rs:27-34` 只是调用宏）。要确认就下 `gpui-pre-macros-<版本>.crate` 读那几张表，
  或者在 `ref/gpui-kit/crates/*/src` 里找现成调用——**别凭"源码里搜不到"就改写法或放弃度量**。

## 3. GitHub 访问代理

- **访问 GitHub 一律走 HTTP 代理 `http://127.0.0.1:7890`**（2026-10-25 用户改的口，原先是 7887；
  仓库级 `git config` 里还写着 7887，**不要改 config**，用命令行覆盖）：
  - git：`git -c http.proxy=http://127.0.0.1:7890 -c https.proxy=http://127.0.0.1:7890 push origin main`
  - gh CLI / curl：`HTTPS_PROXY=http://127.0.0.1:7890 HTTP_PROXY=http://127.0.0.1:7890`
- 直连与代理都可能偶发失败；一种报错就换另一种重试，不要长时间前台轮询等待
  （第二十五轮的文档提交就是在 7887 上重试到第 8 次才推上去的）。

## 4. 选型红线（已由用户定稿，不要"顺手优化"掉）

- **零浏览器技术**：不引入 Electron / WebView / wry / JS / TS。注意 `ref/gpui-component`（0.2.2 那个
  老组件库）会经 `lb-wry` 拖进 WebView2，且它的类型与 `gpui-pre 0.3.9` 不兼容 —— 已弃用，别引。
- gpui 版本：只用 `gpui-kit` umbrella，应用侧不单独声明 gpui；pin 由 kit 的 `=0.3.9` + 我们的
  `Cargo.lock` 共同保证。升级 gpui-pre 快照要连带检查 kit/Ely 的 pin 是否同步。
- gpui 源码参照放在 `ref/gpui/`，是**我们依赖的 0.3.9 发布版**（含 `gpui_windows` 与三个 `.hlsl`）；
  `ref/gpui-kit/examples/` 是 kit 本地 main，比已发布版本新，照抄前先在 `ref/gpui` 或
  crates.io 发布包里确认该 API 存在。
- 画质：**默认**封顶 1080P60（qn=116）+ 同档内 HEVC 优先。2026-10-26（P13）他要求设置页能选到 4K，
  所以 116 从**硬上限**降为**默认值**，允许的范围是 16…127（接口里最高的那档）。
  **B 站的 `qn` 没有"2K"这一档**（1440p 的内容在 120=4K 里给，`ref/bpi-rs/src/models/stream.rs:8-38`），
  所以设置页只给 1080P / 1080P 60帧 / 4K 三个真档，**不许摆一个其实等于 4K 的"2K"按钮**。
  目标机（i5-6200U，Skylake：无 AV1 硬解、HEVC 只到 8bit）解不动 4K/10bit 的代价写在设置页那行下面。
  编码偏好同理是**能改的默认值**：偏好那一档在同画质里拿最高优先级，那一档没有才退回
  HEVC > AV1 > AVC（`api/stream_select::codec_score`）。
- 播放：**外置 mpv 独立进程 + 独立窗口**，不做 `--wid` 嵌入，不自研解码渲染。
  **起播这一路是"把直链 URL 交给 mpv 在线流式播"**——B 站是 DASH 音视频两条流，
  我们只负责拼参数起进程，读字节/切片/缓存全归 mpv 自己，**起播绝不把整片写到磁盘**。
  **2026-10-26 用户开了唯一的一个口子**："下载功能要做一下，**仅这个能下载到外部文件里**"。
  所以下载是**另一条独立的路**（`src/api/download.rs` 自己读字节写文件），不是让 mpv 代下，
  也不是把播放改成"先下再播"；除它以外仍然不许往运行目录之外写任何文件。
- mpv 可执行文件的位置（**2026-10-26 P13 改口**，原话："设置内增加配置『MPV播放器位置』
  （默认在 PATH，支持打开资源管理器选择 mpv.exe）"）：查找顺序是
  **设置里指定的文件 → PATH 里的 `mpv.exe` → 运行目录的 `mpv.exe`**（纯函数
  `src/player/mod.rs::resolve_mpv`，三条都有单测）。
  当年那条坑**没消失，是用两条规则绕开的**：PATH 里 `PATHEXT` 把 `.COM` 排在 `.EXE` 前，
  按 shell 语义（`which`/`where`）解析会先命中 `mpv.com`——它会再派生一个真正的 mpv.exe，
  我们 `kill` 不到。所以这里自己拼 `目录/mpv.exe` 再 `is_file()`，**永远不问 `.com`**；
  设置里写死的那条路径必须"真是个读得到的文件"才算数，否则继续往下找并在设置页说明原因。
  （`which` 依赖仍然不要加回来。）
- **不接 mpv 的文本输出**（用户定稿，2026-10-23）：`launch()` 把 stdout/stderr 都设 `null()`。
  别"顺手"改成 `piped()`：接了没人抽水，缓冲区写满会把 mpv 的写阻塞住。要查 mpv 为什么不动，
  就在本机把同一套参数复跑一遍看它自己说什么。
- **播放期归 mpv，客户端只做播放之外**（用户定稿，2026-10-23；范围纠正的直接后果）：
  进度条、播放时间、暂停、音量、全屏、字幕显示都由 mpv 自带的 OSC 负责，
  **不要**在 `pages/` 里加播放控制按钮或进度行。rustBiliPC 的清单是：主页（首页推荐）、搜索、
  收藏（**P9 起可以点进去看内容，也能搜索**）、历史、待看、下载（**P11 起是真功能**：详情页发起、
  每次弹框选目录、双文件 `.m4v`+`.m4a`、只下当前那一 P、有进度可取消）、设置（**P13 起是真设置页**：
  MPV 位置 / 默认画质 / 默认编码 / 调试模式四项都能改并立刻落盘）、视频详情、评论区、
  分集/分P 的呈现（**P12：合集与系列 + 分集面板 + 分段 + 一集自己的多 P**）、清晰度档位（**列表按 `config.json` 的封顶过滤**，P11 修的缺陷）、用户空间（投稿/图文/收藏/**合集**）、拉起 mpv、
  **详情页与评论区的文字可选中复制**（P11）。
  **2026-10-26 用户把"个人中心"拆平了**：不要"我的"这一格，收藏/历史/待看/下载/设置直接做侧栏入口
  （原来的 `pages/mine/` 因此删除，别再"顺手"把它加回来）。
  第二十二~二十三轮我按"遥控"做过一版（`player/ipc.rs` + `player/state.rs` + 详情页控制条），
  方向错了、已整体删除，**连 `--input-ipc-server` 也不传了**；核实过的机制与坑记在
  `docs/architecture.md` 第 4.14 节**只作档案**。**别把那版代码"捡回来"**。
  **2026-10-25 又划掉两项**：我提议过"字幕投递（把 B 站 JSON 转成 SRT 给 `--sub-file`）"和
  "同实例 `loadfile` 换清晰度保住窗口与进度"，用户判定**两个都没必要做**——换档位就是重开一个实例。
  所以将来别拿第 4.14 节当"待办"：它是档案。
  **2026-10-26（P12）他推翻了自己早先"合集列表不做"那条**：原话"接下来继续完成UP主的主页内容…
  我需要其『合集与系列』内容，并且确保支持好了『分P』/『分集』，尤其是分集…情况比较复杂也要能支持"。
  于是空间页多了第四格"合集"（`seasons_series_list` 一次给合集+系列），点进去是一页页分集
  （`seasons_archives_list` / `series_archives`），详情页右栏最上面是**分集**那一节。
  "复杂"指的是他截图那种：**一个合集里再分段**（电动/猞狸、量力/度规…共 189 集），
  而且**分集里的一集自己还可能有多 P**——所以那一行必须标 `N P`，点进去"选集"改叫"本集的分 P"。
- **接口没给的东西不编**（P9 定稿的两条，都属于"宁可不显示也别解释错"）：
  ① `FavListMedia.type` 的取值在库里**没有**枚举也没有常量（自带测试那个 `"type": 2` 是夹子 info 的），
  所以收藏夹内容**不猜"2=视频"**，改看"拿不拿得到稿件号 `bvid`/`bv_id`"，跳过的条数在页头写成"另有 M 条打不开"；
  ② 历史与待看**没有**服务端 `keyword`（api-notes 第 14 节把 grep 结论写全了），所以那两页的搜索是
  **本地过滤已加载的条目**，界面上必须把范围说出来（"筛选已加载的历史"、"筛出 N / 已加载 M"），
  要扩范围就手动"再加载"。**不许**做一个看起来像全站搜索、其实只搜了一页的输入框。
- **库里没给 typing 的字段，允许"自己再发一次同一个端点读原始 JSON"，但只有这一条路**（2026-10-26 P12 定）：
  `video().view()` 的 `VideoView` 没有 `ugc_season`（带它的 `VideoData` 在库里是没人返回的死代码，
  不许 import），而分集信息只在那一块里。所以 `api/detail.rs` 用 `BpiClient::get`（公开的，
  自带库那套 Referer/UA/cookie）**再请求一次 `x/web-interface/view`**，只挑 `data.ugc_season`，
  用 `serde_json::Value` **宽容**解析（每个字段都可能没有，缺就少一格信息）。三条边界：
  ① 只读这一个键，别顺手把 `pubdate`/`tname` 也捞出来（那是他划掉的项，要加得他先点头）；
  ② 这一路失败**只少分集那一节** + 一行说明，绝不连累主体；
  ③ 不自建第二套 HTTP 栈（不引 reqwest 依赖，走 `BpiClient::get`）。
- **触摸屏 = 按住拖动，别去找触摸 API**（2026-10-26 核实；细节在 `docs/architecture.md` 第 4.23 节与
  `src/ui/scroll.rs` 文件头）：gpui 0.3.9 的 Windows 平台层不接 `WM_TOUCH`/`WM_POINTER*`，
  核心里那套触摸管线只有 macOS 后端在喂，Windows 上永久闲置。手指能动是 Windows 把触点注入成鼠标消息，
  所以"单指上下滚"的实现就是 `ScrollHandle::set_offset` + `MouseMoveEvent::dragging()`。三条硬规矩：
  ① `Window::on_mouse_event` 的监听**只活一帧**，必须在 `render` 里每帧登记；
  ② 改完偏移要 `cx.notify(entity_id)` 才有人重绘；③ 移动处理里 `stop_propagation()`，
  否则拖出一段文本选中高亮而不是内容在动。**不要**为了"支持触摸"去引新库、写 `unsafe` 或自己抓 Win32 消息。
  另一件别当 bug 修：手指**没有 hover**，只靠 hover 表达的层级在触摸屏上看不见（选中态另有底色+边框，不影响可用）。

- **视觉必须照 `ref/Bili.Copilot`，且度量要抄数字不要凭感觉**（用户原话："GUI 样式要参考
  ref/Bili.Copilot，你这个太丑了"）：颜色与尺寸常量只从 `src/ui/palette.rs` 取，
  复用件只从 `src/ui/layout.rs` 取，页面里不许出现 `rgba(0x…)` / `px(56.)` 这类字面量；
  每个数字的 XAML 出处逐条记在 `docs/architecture.md` 第 4.15 节。
  Acrylic/Mica、`ItemsView`、icon font 这些 gpui 没有等价物的东西**不要硬模仿**。
- **主界面外壳与窗口的形状是用户定的，不要自己改回去**（2026-10-26）：
  侧栏宽 **80**（`palette::SIDE_COLUMN_WIDTH`，用户要求 240 的 1/3）、导航项是
  "Lucide 图标 + 两字"竖排、左上角是**登录账号头像且只做显示不可点**（原来那格是
  "rustBiliPC" 文字）、**左下角的 mpv 自检行已删**（信息挪进设置页）、返回那一格文案是
  "返回"。**登录是独立小窗口**（320×420、不可缩放），不再是主窗口里换视图——
  换窗口有两条硬约束（先开主窗再关登录窗；不能在别的窗口的 update 里开窗），
  写在 `docs/architecture.md` 第 4.19 节与 `src/pages/windows.rs` 文件头，改之前先读那两处。
  图标名写错**不会报错也不会 panic**，只会画出一片空白，所以加图标前先确认
  `ref/gpui-kit/crates/assets/assets/icons/<名字>.svg` 存在。
- **落盘位置（2026-10-26 P13 用户改口，取代"exe 同目录"那一版）**：`auth.json`、`config.json`、
  `debug.log` 三样都在**用户数据目录**
  （Windows `%APPDATA%\ArrowMeo\rustBiliPC`，Linux `$XDG_CONFIG_HOME`/`~/.config` 下的同名子目录；
  拿不到环境变量才退回 exe 目录下的 `ArrowMeo/rustBiliPC`），入口是 `src/app.rs::settings_dir()`。
  他的原话："config.json/auth.json/debug.json改为存储到 %APPDATA%/ArrowMeo/rustBiliPC 里，
  **不要保留任何支持旧位置的json文件的代码**"——所以
  **`data/cookies.json` 与 `<exe 同目录>/*.json` 的读取、迁移、兼容分支全部删干净了**，
  以后也别"顺手加个旧位置回退"：那等于把这条定稿擦掉。
  都是明文 JSON（用户知情选择），要改加密只动 `save`/`load`。
  这条换目录顺带消掉了一条要靠人记的红线：**取 CI 产物不再可能碰到用户数据**
  （以前 `debug.log`/`auth.json` 就躺在 `publish/` 里，清空目录等于删掉他的登录态）。
- **唯一能写到运行目录之外的是下载**（2026-10-26 用户定稿），三条细则一起定了：
  ① **每次下载都弹系统对话框选目录**（`App::prompt_for_paths` + `directories: true`，
  Windows 后端真有实现：`FOS_PICKFOLDERS`，`ref/gpui/gpui_windows/src/platform.rs:1531`），
  **不存默认目录、不在运行目录开 downloads/、不把位置写进 `config.json`**；
  ② 形态是**双文件 + 高画质**（`.m4v` + `.m4a`，画质吃 `config.json` 的封顶，不做混流合成单文件，
  那要引 ffmpeg）；③ **只下当前选中的那一 P**。
  还在跑的任务记录只活在内存里（关程序就没了，磁盘上的成品不动）；要持久化得他点头。
- **文本可选中与复制走 kit 的 `base`，不要自己写命中测试**（2026-10-26 核实）：
  gpui 0.3.9 **没有**文本选择机制（`ref/gpui/gpui/src` 里搜不到 `Selectable`/`text_selection`），
  机制在 `gpui_kit::base::{SelectableText, TextSelection}`：`SelectableText::new(id, text)` 当叶子元素用，
  样式仍留在父 div 上；`gpui_kit::open_window` 把内容包进 `base::Root`，Root 已挂 `TextSelectionLayer`
  并装了 `Ctrl+C`/`cmd+C` → 复制，**所以 `gpui_kit::init(cx)` + `open_window` 就够了，不用自己接线**。
  复制剪贴板只有 `App::write_to_clipboard(ClipboardItem::new_string(String))` 一条路（没有 arboard）。
  **和 P10 拖动滚动的冲突要认**：手指/鼠标都是同一条 `WM_MOUSEMOVE`，分不开来源，
  所以按下时用 `TextSelection::is_selectable_at(落点)` 让位——**按在可选中文字上的那一下只选字，不滚页**。
- **`config.json` 里只允许放"真的有代码在读"的项**。P13 起一共四项，每一项都有读取方：
  `max_quality`（`api::stream::request_qn` + 选流封顶）、`codec`（`api::stream_select::codec_score`）、
  `mpv_path`（`player::resolve_mpv`）、`debug_log`（`logging::init`，只在启动时读一次）。
  设置页**能改而且立刻写盘**（`app::update_config`：先改内存快照再写文件，写失败要把原因显示出来），
  所以不存在"显示了但不生效"的假开关；要加新项必须先有读取方。
  唯一一处"改了不是马上生效"是调试模式（日志器装上没有二次安装/撤销的口子），界面上明说"下次启动生效"。
- **每个 `cx.spawn` 返回的 `gpui::Task` 必须持有或 `.detach()`**：Task 被 drop 即取消任务
  （`gpui-pre-scheduler/src/executor.rs:396-402`），裸写 `cx.spawn(...);` 会让"等结果并写回界面"
  那一段永远不执行，症状是请求在 tokio 侧正常返回、界面却卡在加载态、日志里连闭包第一行都没有。
  要停旧循环用序号（`generation`/`request_seq`）判断"我已被取代"后 `return`，**不要**从任务自己的
  future 里丢弃它的 Task（`Task::cancel` 等自己 = 主线程死锁）。已在 `Cargo.toml` 用
  `unused_must_use = "deny"` 兜底。见 `docs/architecture.md` 第 4.9 节。
- **CI 日志里的 warning 不能一律跳过**：快速开发阶段可以不管排版类 warning，但
  `unused_must_use` 这类"语义会坏"的必须当场读并处理——这条就是第七轮到第十轮一直躺在那里的信号。
- 快速开发阶段：能调库就不造轮子，不管 warning，不设 clippy/rustfmt/test 门禁；但编译必须无 error。

## 5. CI 相关的既成事实

- release 编译期需要 `fxc.exe`（gpui `build.rs` 找不到就 panic）：先探测 Windows SDK 路径，
  必要时显式设 `GPUI_FXC_PATH`。
- `rustls 0.23` 默认 provider 会拉 `aws-lc-sys`，MSVC 上要 CMake + NASM：CI 里装 nasm，
  不去砍 http3/ring provider。
- 构建里同时存在两份 reqwest（kit 的 `gpui-pre-reqwest 0.12.15` 与 bpi-rs 的 `0.13.5`），属已知浪费，
  不为它做清理；**不要**再引入第三个 HTTP 客户端。
- `Cargo.lock` **已入库**，但 CI **不加 `--locked`**：本机没有 cargo，新增依赖时无法预先刷新 lock，
  加了就会直接卡死。lock 的作用是"版本种子"（cargo 沿用它记录的版本，只为新依赖解析）。
  每次改完依赖都要把 CI 的 `cargo-lock` artifact 取回并连同改动一起提交。
- CI 缓存步骤必须 `save-always: true` 且 key 只按 `Cargo.lock` 哈希：否则一次失败构建既不写缓存
  又让下一次全量重编（已踩，见 docs/architecture.md 第 8 节）。
  **但改 `Cargo.lock` 并不会掉缓存**：`restore-keys` 写的是 `cargo-win-…-release-` **前缀**，
  key 不精确命中时 Actions 会回退到最近一份同前缀缓存（第十九轮实测：删了 `which`、lock 变了，
  仍然只重编 bpi-rs + 本项目，`Finished … in 40.69s`，整轮约 4 分钟）。
  所以**别把"改 lock 要等十几分钟"当成判断依据**。
  **注意**：第十六轮起 Actions 报 `Input 'save-always' has been deprecated … does not work as intended`；
  目前缓存仍然命中，所以先不动 workflow。**一旦编译真的变回十几分钟，这条是第一嫌疑**，
  届时改成 `post: true` 的独立保存步骤，而不是回头去猜。
- 不要在前台长时间轮询 CI：用后台轮询或短探测，以日志证据为准。
  **判"跑完"要比对字段值，不要用子串匹配整段 JSON**：`gh run view --json status,conclusion`
  的输出里随时可能出现 `completed`（步骤级 `status`），第十八轮第一版轮询就把还在编译的 run
  当成了结束。取 `--json status --jq .status` 再 `[ "$st" = "completed" ]`。
- **快得反常要通过日志证明，而不是直接采信**：第十八轮整轮 2 分 45 秒（缓存命中），
  判断依据是日志里有 `Compiling rustbilipc`、`Finished … in 42.52s`，且 exe 字节数与 md5
  都变了。只有"产物变了 + 确实编过"两条都在，才可以说这轮算数。
- **一轮只烧一次**：触发后要等这个 SHA 的 run 结束再报结果、再改代码；没看完上一轮日志就推新提交
  等于白烧一轮编译（2026-10-22 犯过一次）。本地能核对的 API 一律先核对（见第 2 节的核对顺序）。
- **std 的 API 也不许凭印象写**（2026-10-23 连烧两轮）：参考仓库之外自己脑子里的签名同样会错——
  `Vec` **没有** `replace_range`（那是 `String` 的；Vec 上同名方法来自 `bstr::ByteVec`，E0599），
  `properties.iter()` 的元素是 `&&str`，用 `|&(index, name)|` 解 `(usize, &&str)` 是 E0308。
  写不确定的那一行时，选**一定存在**的形式（`split_off` 赋值、`.copied().enumerate()`），
  或者先去 `doc.rust-lang.org` / 下 `.crate` 看清签名。
- **触发前先确认分支上没有未编译的代码改动**（`git status` 干净、`origin/main` 已更新），并记下这个
  run 对应的 SHA：**交到用户手上的产物必须是分支顶端那个 SHA**（**纯文档/纯注释提交例外**：
  只改 markdown 或 `//` `///``//!` 注释、不动任何表达式与签名，报告里写明产物来自哪个 SHA 即可，
  不必为几行说明再烧一轮）。第十四轮编的是 `5c7c8ae`，我随后才提交
  "懒加载"，那轮的 exe 就不能当作最终交付物，得再编一轮（2026-10-23）。
- 取产物一律解到临时目录，再**只**覆盖 `publish/rustbilipc.exe`：`gh run download -D publish`
  遇到同名文件会整体失败（第十轮就是这么静默没换掉的），而 `publish/debug.log`、
  `publish/auth.json`（以及历史遗留的 `publish/data/cookies.json`）是**上一版留在现场的**用户凭据与日志，
  **绝不能被清理动作带走**。（P13 之后新数据写在 `%APPDATA%`，但 `publish/` 里那份旧文件仍然不要删——
  那是他某一次登录的现场，删除不可逆。）

## 6. 输出与文档

- 中文必须可读：UTF-8，禁止 `\uXXXX` 转义与 GBK 乱码；`.bat` 文件内容保持 ASCII（cmd 按 OEM 代码页解析）。
- 从 `ref/` 得出的结论要抄进 `docs/`，因为 `ref/` 不入库。
- **调试靠 `debug.log`，不靠猜测**：GUI 程序没有控制台，日志由 `src/logging.rs` 写到
  **用户数据目录**的 `debug.log`（与 `auth.json`、`config.json` 同一层，每次启动清空、Debug 级）。
  **P13 起它默认不产生**：设置页"调试模式"关着时 `logging::init(false)` 不建目录、不开文件、
  不装 logger、级别 `Off`（`main.rs` 因此必须先读设置再装日志，顺序在 `main.rs` 文件头）。
  两个例外要知道：① exe 现在是 `#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]`，
  **是链接期决定的**，所以两种模式下都不弹控制台——"调试模式开着也不弹窗口"不是 bug，
  要真控制台得另做 `AttachConsole`（没做）；② `config.json` 坏了的那一轮**强制开日志**
  （调试模式就写在那个坏文件里，不然他既看不到错误也没法打开它）。
  从 GitHub 取产物时**只搬 exe**（解到临时目录再 copy），不要清空 `publish/`——
  现在用户数据已经不在那里了，但 `publish/` 里可能还有他自己放的 `mpv.exe` 与旧日志。
  新增任何异步链路时，必须在"提交任务 / 任务内请求前后 / 回到 gpui / 写入状态"四类位置各留日志，
  否则真机问题无法定位。排查用户问题时先要这个文件（设置页那一格显示它的路径与"有没有在写"）。
