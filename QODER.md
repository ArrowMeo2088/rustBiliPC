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
- 所有编译只通过 GitHub Actions 的 `Build` workflow（手动触发 `workflow_dispatch`），
  现阶段仅 **Windows x64 release**。
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
- 画质：HEVC 优先 + **默认封顶 1080P（qn ≤ 116）**，不选 4K/HDR/8K（目标机 i5-6200U 硬解不到）。
- 播放：**外置 mpv 独立进程 + 独立窗口**，不做 `--wid` 嵌入，不自研解码渲染。
  **是"把直链 URL 交给 mpv 在线流式播"，不是下载下来再播**——B 站是 DASH 音视频两条流，
  我们只负责拼参数起进程，读字节/切片/缓存全归 mpv 自己（不许写整片到磁盘）。
- mpv 可执行文件的位置：**运行同目录的 `mpv.exe`**（`publish/mpv.exe`，2026-10-23 用户定稿），
  **不走 PATH**：PATH 先命中 `mpv.com`（PATHEXT 把 `.COM` 排在 `.EXE` 前），它会再派生一个
  mpv.exe，我们 `kill` 不到真正的播放器；用户装的那份还是 mpv-lazy 懒人包，带自己的
  `portable_config` 与单实例启动器。`which` 依赖因此已删除。
- **不接 mpv 的文本输出**（用户定稿，2026-10-23）：`launch()` 把 stdout/stderr 都设 `null()`。
  别"顺手"改成 `piped()`：接了没人抽水，缓冲区写满会把 mpv 的写阻塞住。要查 mpv 为什么不动，
  就在本机把同一套参数复跑一遍看它自己说什么。
- **播放期归 mpv，客户端只做播放之外**（用户定稿，2026-10-23；范围纠正的直接后果）：
  进度条、播放时间、暂停、音量、全屏、字幕显示都由 mpv 自带的 OSC 负责，
  **不要**在 `pages/` 里加播放控制按钮或进度行。rustBiliPC 的清单是：主页（首页推荐）、搜索、
  收藏（**P9 起可以点进去看内容，也能搜索**）、历史、待看、下载（这一版只有入口）、设置（这一版只是关于页）、视频详情、评论区、
  分集/分P 的呈现、清晰度档位、用户空间（投稿/图文/收藏）、拉起 mpv。
  **2026-10-26 用户把"个人中心"拆平了**：不要"我的"这一格，收藏/历史/待看/下载/设置直接做侧栏入口
  （原来的 `pages/mine/` 因此删除，别再"顺手"把它加回来）。
  第二十二~二十三轮我按"遥控"做过一版（`player/ipc.rs` + `player/state.rs` + 详情页控制条），
  方向错了、已整体删除，**连 `--input-ipc-server` 也不传了**；核实过的机制与坑记在
  `docs/architecture.md` 第 4.14 节**只作档案**。**别把那版代码"捡回来"**。
  **2026-10-25 又划掉两项**：我提议过"字幕投递（把 B 站 JSON 转成 SRT 给 `--sub-file`）"和
  "同实例 `loadfile` 换清晰度保住窗口与进度"，用户判定**两个都没必要做**——换档位就是重开一个实例。
  所以将来别拿第 4.14 节当"待办"：它是档案。
- **接口没给的东西不编**（P9 定稿的两条，都属于"宁可不显示也别解释错"）：
  ① `FavListMedia.type` 的取值在库里**没有**枚举也没有常量（自带测试那个 `"type": 2` 是夹子 info 的），
  所以收藏夹内容**不猜"2=视频"**，改看"拿不拿得到稿件号 `bvid`/`bv_id`"，跳过的条数在页头写成"另有 M 条打不开"；
  ② 历史与待看**没有**服务端 `keyword`（api-notes 第 14 节把 grep 结论写全了），所以那两页的搜索是
  **本地过滤已加载的条目**，界面上必须把范围说出来（"筛选已加载的历史"、"筛出 N / 已加载 M"），
  要扩范围就手动"再加载"。**不许**做一个看起来像全站搜索、其实只搜了一页的输入框。
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
- **落盘位置（2026-10-26 用户定稿，取代旧的 `data/cookies.json`）**：凭据 =
  `<exe 同目录>/auth.json`（`src/store/auth.rs`），设置 = `<exe 同目录>/config.json`
  （`src/store/config.rs`）。**别再往 `data/` 里写任何东西**；旧文件只允许在 `auth.json`
  读不到时作一次性迁移来源（迁移后保留、不删）。都是明文 JSON（用户知情选择），
  要改加密只动 `save`/`load`。
- **`config.json` 里只允许放"真的有代码在读"的项**。目前只有 `max_quality`（起播的封顶档，
  被 `api::stream::request_qn` 读走）；设置页这一版是**只读关于页**，不做显示了但不生效的假开关。
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
  `publish/auth.json`（以及历史遗留的 `publish/data/cookies.json`）是用户的现场与凭据，
  **绝不能被清理动作带走**。

## 6. 输出与文档

- 中文必须可读：UTF-8，禁止 `\uXXXX` 转义与 GBK 乱码；`.bat` 文件内容保持 ASCII（cmd 按 OEM 代码页解析）。
- 从 `ref/` 得出的结论要抄进 `docs/`，因为 `ref/` 不入库。
- **调试靠 `debug.log`，不靠猜测**：GUI 程序没有控制台，日志由 `src/logging.rs` 写到
  **exe 同目录**的 `debug.log`（与 `auth.json`、`config.json` 同一层，每次启动清空、Debug 级）。
  从 GitHub 取产物时**只搬 exe**（解到临时目录再 copy），不要清空 `publish/`——日志就在那里。
  新增任何异步链路时，必须在"提交任务 / 任务内请求前后 / 回到 gpui / 写入状态"四类位置各留日志，
  否则真机问题无法定位。排查用户问题时先要这个文件。
