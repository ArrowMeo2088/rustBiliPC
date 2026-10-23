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

## 3. GitHub 访问代理

- **访问 GitHub 一律走 HTTP 代理 `http://127.0.0.1:7887`**：
  - git：`git -c http.proxy=http://127.0.0.1:7887 -c https.proxy=http://127.0.0.1:7887 push ...`
    （本仓库已设仓库级代理配置）。
  - gh CLI / curl：`HTTPS_PROXY=http://127.0.0.1:7887 HTTP_PROXY=http://127.0.0.1:7887`。
- 直连与代理都可能偶发失败；一种报错就换另一种重试，不要长时间前台轮询等待。

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
- 凭据：明文 JSON 存 `<exe 同目录>/data/cookies.json`（用户知情选择）。要改加密只动
  `src/store/credentials.rs` 的 `save`/`load`。
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
  **注意**：第十六轮起 Actions 报 `Input 'save-always' has been deprecated … does not work as intended`；
  目前缓存仍然命中（我们的 crate 三十多秒编完），所以先不动 workflow。**一旦命中率变差，这条是
  第一嫌疑**，届时改成 `post: true` 的独立保存步骤，而不是回头去猜。
- 不要在前台长时间轮询 CI：用后台轮询或短探测，以日志证据为准。
  **判"跑完"要比对字段值，不要用子串匹配整段 JSON**：`gh run view --json status,conclusion`
  的输出里随时可能出现 `completed`（步骤级 `status`），第十八轮第一版轮询就把还在编译的 run
  当成了结束。取 `--json status --jq .status` 再 `[ "$st" = "completed" ]`。
- **快得反常要通过日志证明，而不是直接采信**：第十八轮整轮 2 分 45 秒（缓存命中），
  判断依据是日志里有 `Compiling rustbilipc`、`Finished … in 42.52s`，且 exe 字节数与 md5
  都变了。只有"产物变了 + 确实编过"两条都在，才可以说这轮算数。
- **一轮只烧一次**：触发后要等这个 SHA 的 run 结束再报结果、再改代码；没看完上一轮日志就推新提交
  等于白烧一轮编译（2026-10-22 犯过一次）。本地能核对的 API 一律先核对（见第 2 节的核对顺序）。
- **触发前先确认分支上没有未编译的代码改动**（`git status` 干净、`origin/main` 已更新），并记下这个
  run 对应的 SHA：**交到用户手上的产物必须是分支顶端那个 SHA**（**纯文档/纯注释提交例外**：
  只改 markdown 或 `//` `///``//!` 注释、不动任何表达式与签名，报告里写明产物来自哪个 SHA 即可，
  不必为几行说明再烧一轮）。第十四轮编的是 `5c7c8ae`，我随后才提交
  "懒加载"，那轮的 exe 就不能当作最终交付物，得再编一轮（2026-10-23）。
- 取产物一律解到临时目录，再**只**覆盖 `publish/rustbilipc.exe`：`gh run download -D publish`
  遇到同名文件会整体失败（第十轮就是这么静默没换掉的），而 `publish/debug.log` 与
  `publish/data/cookies.json` 是用户的现场与凭据，绝不能被清理动作带走。

## 6. 输出与文档

- 中文必须可读：UTF-8，禁止 `\uXXXX` 转义与 GBK 乱码；`.bat` 文件内容保持 ASCII（cmd 按 OEM 代码页解析）。
- 从 `ref/` 得出的结论要抄进 `docs/`，因为 `ref/` 不入库。
- **调试靠 `debug.log`，不靠猜测**：GUI 程序没有控制台，日志由 `src/logging.rs` 写到
  **exe 同目录**的 `debug.log`（与 `data/cookies.json` 同一层，每次启动清空、Debug 级）。
  从 GitHub 取产物时**只搬 exe**（解到临时目录再 copy），不要清空 `publish/`——日志就在那里。
  新增任何异步链路时，必须在"提交任务 / 任务内请求前后 / 回到 gpui / 写入状态"四类位置各留日志，
  否则真机问题无法定位。排查用户问题时先要这个文件。
