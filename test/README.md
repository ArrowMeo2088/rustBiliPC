# test — 单元测试说明

本目录不存放 `.rs` 代码（cargo 不识别 `test/`），只承载测试布局约定与将来的测试脚本。

## 布局约定

| 类型 | 位置 | 说明 |
| --- | --- | --- |
| 单元测试 | `src/tests/` | 作为 crate 的模块编译，入口 `src/lib.rs` 的 `pub mod tests;` |
| 集成测试 | 仓库根 `tests/`（需要时再建） | 按 cargo 惯例，只依赖公开 API |

## 现阶段覆盖范围（快速开发期）

只测**纯逻辑**，因为 CI 上没有 GUI 会话也没有 B 站账号：

- `stream_select_tests`：画质档封顶边界（16/32/64/80/112/116 允许，120/125/126/127 排除）。
- `player_tests`：mpv 参数形状——**带值的选项必须写成 `--opt=value`**（`--opt value` 会让 mpv
  在建窗口前 `Exiting... (Fatal error)`，第十八轮真机上"窗口没出现"的根因）、视频 URL 必须是
  最后一个位置参数、`--audio-file=` 单独一路、**durl 单文件时不能出现 `--audio-file`**
  （空值会让 mpv 直接报错退出）、`Referer` 在拼起来的 `--http-header-fields=` 里、附加头能追加、
  **参数表里不许出现 Lua 门控的 `--no-ytdl`**（不带 Lua 的构建把它当未知选项，又是不出现窗口），
  mpv 定位是 `<exe 同目录>/mpv.exe` 而不是 PATH。
- `credentials_tests`：JSON 往返、缺省字段容错（老文件不能读不回）、完整性判定、
  与 `bpi_rs::Account` 的字段映射。
- `search_format_tests`：标题高亮标签清洗、播放量的中文读法（万/亿与尾零截断、边界值）、
  排序项的中文标签。
- `login_tests`：B 站登录码值到状态的翻译（0/86101/86038/未知码带文案）、cookie→凭据的
  完整性要求（少 `buvid3` 必须判 `None`）、**只有"成功且缺 buvid3"才需要补取**
  （`needs_buvid3_backfill`，等待/过期时不该多发请求）、凭据 JSON 往返相等。
- `qr_tests`：矩阵存在性、正方形与最小边长（>=21 且为奇数）、空文本也能出符号。
- `mine_tests`：个人中心那两块纯函数的边界——秒数格式化（`0` / `45` / `599` / 正好一小时 /
  `1:02:05` / `23:59:59`）、会员文案（`status` 或 `kind` 任一为 0 都不显示会员）。
- `cover_tests`：**协议相对地址 `//host/path` 必须补成 `https://`**（第十五轮真机封面全空的根因，
  gpui 用 `Url::parse` 判 URL，补不上就变成"内置资源找不到"）、只有 `hdslb.com` 才升 https 与拼
  `@Ww_Hh.jpg` 缩略图后缀、已带 `@` 与 GIF 不动、空地址仍是空（空串不能变成 `https://`）。
- `detail_tests`：详情 DTO 的三处边界——`favorite`/`fav` 两个字段名取新舍旧、字幕地址补协议
  （B 站给 `//host` 时不补就是死链）、`seconds_to_u32` 在超过 `u32` 时**截断而不回绕**
  （回绕会把长视频变成几秒，比报错更难发现）。
- `playback_tests`：**清晰度中文档的兜底顺序**（`new_description` → `display_desc` →
  `accept_quality`/`accept_description` 下标配对 → `"{qn}P"`）、`quality_options` 在没有
  `support_formats` 时能退回 `accept_*` 列表、`cookie_header()` 只列出**实际有值**的 cookie
  （缺一个字段就少一条，不拼 `SESSDATA=` 这种空项——空值与"没带"在排查时容易被当成同一回事）。
- ~~`ipc_tests` / `play_state_tests`~~：第二十二~二十三轮为 mpv 遥控写的两组测试（协议一行一条、
  `data: null` 保留、`request_id` 缺省 0、订阅 id 不重复；状态口径、缓冲优先、`--:--` 不编 `00:00`）
  **已随范围纠正一并删除**——播放期归 mpv，客户端不做遥控 UI。测试代码没了但结论都在
  `docs/architecture.md` 第 4.14 节，P6 真要连管道时照着写回来。
- `scaffold_tests`：版本号非空且以数字开头。

## 不测什么

- UI 渲染与交互：验证方式是 CI 产物在真机上跑起来看（架构文档的机制决定 release 才能运行）。
  `ui/palette.rs` 与 `ui/layout.rs` 也属这一类——前者是常量，后者是 gpui 元素构造，
  没有不需要窗口就能断言的逻辑；它们的正确性靠"数字抄自 `ref/Bili.Copilot` 的 XAML"保证，
  出处逐条列在 `docs/architecture.md` 第 4.15 节。
- 真实网络请求：需要登录态，人工验证。
- **mpv 管道的读写**：当前**没有这条链路**（播放期归 mpv，客户端不遥控）。以前写过的那版测的是
  它两边的形状——消息必须是一行一条 minified JSON、事件要能解析、状态要按口径显示；
  连接与读写本身从来没有本机验证过（没有工具链、也没有真 mpv 进程可测）。
  结论保留在 `docs/architecture.md` 第 4.14 节，P6 恢复时照抄。
- **视图里的异步状态机**（例如"登录成功之后有没有停轮询、有没有写凭据"）：要用 gpui 的无窗口
  测试执行器（`TestAppContext`），它只在 `test-support` feature 下导出，而我们的入口是
  `gpui-kit` —— 得先加 `[dev-dependencies] gpui-kit = { version = "0.8.0", features =
  ["test-support"] }` 再验一遍能不能编。这一类回归现在由 `Cargo.toml` 的
  `unused_must_use = "deny"` 兜底（"忘了持有/`.detach()` Task" 直接编译失败），
  补测试排在登录真机确认之后。

## 命令

```bash
cargo test            # 本机无工具链时由 CI 执行
cargo test --release  # 与发布配置一致（现阶段 CI 只跑 --release）
```

改动测试布局或覆盖范围时，同步更新本 README 并提交 git。
