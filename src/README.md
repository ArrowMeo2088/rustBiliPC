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
| `theme.rs` | 全局主题 | **先 `Theme::change(Dark)` 再覆盖语义色**（kit 的 init 默认亮色，不切就出现"页面黑、输入框白"）：底色/字色/描边/输入框/按钮（含 `button_primary`，主按钮不读 `accent`）都对到 `ui::palette`，另外把 `radius` 改成 4、`font_size` 改成 14（WinUI 的 ControlCornerRadius 与 Body）。**这里不另定义一份颜色** |
| `api/` | bpi-rs 封装 + DASH 选择 | `spawn_on_api_runtime` 是唯一的 gpui↔tokio 桥，`format_seconds`/`seconds_to_u32` 放在 `mod.rs`（历史、详情与主页共用）；`login.rs`（取码/轮询 + 码值翻译 + cookie→凭据 + **缺 buvid3 时向 `misc().buvid()` 补取**）、`profile.rs`（`space_info` + `login().stat()`）、`favorites.rs`（`created_list` + `collected_list`，DTO 带 `created` 标记）、`history.rs`（`history_list` 只要 Archive）、`home.rs`（**P6 主页**：`video().homepage_recommendations()`，WBI 由库里签；**必须按 `goto=="av"` 过滤**，理由见文件头）、`comments.rs`（**P6b 评论区**：`comment().list()`，**`type=1`+`oid=aid`**、`ps` 被库卡在 1–20、相对时间只做减法不引 chrono）、`space.rs`（**P7 用户空间**：`user().card()` 不签 WBI、`uploaded_videos()` 签且带 `tid` 分区筛选、`opus().space_feed()` 是 offset 游标；`tlist` 是 `Value`，[`parse_tlist`] 认不出来就少一个 chip 而不是整块失败）、`detail.rs`（`view` 主体 + `desc`/`tags`/`related_videos`/字幕**四路各自容错**，原因见文件头）、`stream.rs`（`play_url` 取地址 → `StreamChoice`：DASH 双流或 durl 单文件、清晰度中文档、备用地址；页面不起播细节都从这里出）、`search.rs` 出 DTO（`SearchHit`/`PageInfo`/`SearchSort`），bpi 类型不外泄；`stream_select` 是纯函数 |
| `store/` | 落盘 | `credentials.rs` 明文 JSON，见文件头的风险说明；`cookie_header()` 把四条 cookie 拼成 mpv 要的那一行（1080P 直链校验登录态） |
| `player/` | 外置 mpv（**只有 `mod.rs` 一个文件**） | `mpv_path()`/`find_mpv()` 只认**运行同目录的 `mpv.exe`**（不走 PATH，原因见文件头）；`build_arguments` 纯函数（带值选项一律 `--opt=value`，音频 URL 是 `Option`，多条头逗号拼成一个 `--http-header-fields`）；`launch()` 拉起进程、stdout/stderr 都 `null()`（**不接 mpv 的文本输出**）。**播放期的控制与显示全归 mpv 自己的 OSC**：本项目不做进度条/暂停/时间显示，所以没有 IPC 客户端代码；仍然传 `--input-ipc-server`（管道名带实例序号）是给 P6 的"同实例换清晰度 / `sub-add`"预留，机制与坑已核实并记在 `docs/architecture.md` 第 4.14 节 |
| `pages/` | 按阶段划分的页面 | `mod.rs` 里有两个共用导航事件：`OpenVideo{bvid}` 与 `OpenUser{mid,name}`；`root.rs` 是窗口根视图（无凭据→登录页，有→主界面外壳，登录成功原地切换，事件带 mid）；`shell/` 240 侧栏（48 应用名块 + **主页/搜索/我的** 三入口 + 返回上一层 + 底部 mpv 目录提示），**下钻栈是枚举**（`StackPage::{Detail, Space}`，因为从 P7 起详情与空间是交错下钻的，两个栈就没法"返回上一层"），三个入口页实体常驻不重刷；`login/` 扫码 + 1.5 s 轮询 + 过期自动重载；`home/` 首页推荐流（**默认页**，构造时就取第一批；"换一批"= 批次号 +1，没有分页条，失败只写这一页的提示位）；`mine/` 资料 + 收藏夹 + 历史**三路并发**（第一次切到它才发请求）；`detail/` 主体 + 选集 + **起播**（一键交给 mpv，持 `Child` 并在 `Drop` 里 kill，见第 4.13 节）+ 字幕 + **评论区**（顶层列表 + 最热/最新 + 上一页/下一页，见第 4.17 节）+ **UP 主那一行整行可点 → 空间页** + 相关推荐（360 右栏）；`space/` 用户空间（资料卡 + 投稿/图文/收藏三块，**三块各一个序号**，投稿带分区 chips，见第 4.18 节）；`search/`：输入 → 回车/按钮/排序/分页 → tokio 请求 → `upgrade()+update()` 回填 |
| `ui/` | 跨页面复用组件 | `palette.rs`：**颜色与尺寸常量的唯一出处**（`rgba` 是 RRGGBBAA，`ref/gpui/gpui/src/color.rs:20-23`）；`layout.rs`：由这些常量组成的复用件（56 页头 + 1px 线、侧栏项、圆头像、统计行、提示、卡片壳、**`list_row` 一栏"标题+说明"的列表行**）；`video_card.rs`（竖卡 232×238、封面 232×122、文字区内边距 16,12,16,16、`flex_wrap` 网格 gap 20、hover 只换底色，横卡 104 高用于侧栏，**`post_card` 是图文条目的竖排卡（封面按接口给的比例摆、正文、点赞），因为接口不给标题和时间**）、`cover.rs`（**显示前必过**：补 `//`→`https://` + 拼图床 `@Ww_Hh.jpg` 缩略图后缀）、`qr.rs`（qrcode-generator 出 `Vec<Vec<bool>>`，一格一个 div 画出来）。页面**不许**自己写 `rgba(...)`/`px(56.)` 这类字面量，度量来源逐条记在 `docs/architecture.md` 第 4.15 节 |
| `tests/` | 单元测试 | 只测纯逻辑：码流边界、mpv 参数形状（**带值选项必须 `--opt=value`**、单文件不加 `--audio-file`、不给 Lua 门控的 `--no-ytdl`）、mpv 定位在运行目录、凭据往返与 `cookie_header()` 只列有的字段、搜索标签清洗与播放量格式化、**主页只放行 `goto=="av"`、`stat` 缺失时不写 0、批次大小在接口区间内**、**评论：目标类型码必须是 1、`has_more` 两种口径、相对时间各档且不出负数**、**空间：`tlist` 的排序与畸形输入**、登录码值翻译 / cookie 完整性 / buvid3 补取判定、二维码矩阵、秒数格式化与会员文案、封面 URL 补协议与缩放、详情的 `favorite`/`fav` 双字段与字幕地址补协议、清晰度中文档的四级兜底顺序 |

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
再次起播时也先收掉上一个实例。否则就是"关掉详情页、视频还在放、没人能关掉那个窗口"。

**外部程序的错我们不抄日志，但一定要在本机复跑那条命令行**：GUI 进程没有控制台，mpv 又"参数写错就静默退出"。
第十八轮"mpv 窗口没出现"排查了一整轮，根因是 `--opt value` 这种写法（mpv 只认 `--opt=value`，
`--opt value` 会在**建窗口之前**报致命错误退出）。定稿是**不接 mpv 的文本输出**（`launch()` 里
stdout/stderr 一律 `null()`；`piped()` 之后没人抽水会把 mpv 的写阻塞住）。
要查为什么不动，就把同一套参数在本机跑一遍，让它自己把话说出来。
顺带记一条：mpv 的消息默认写 **stdout**（`ref/mpv/common/msg.c:190-202`），所以"只接 stderr"等于什么都没接。

**播放期不做什么**（范围定稿，2026-10-23）：进度条、时间、暂停、音量、全屏、字幕显示都由 mpv 的
OSC 负责，本项目**只做播放之外**的事（主页、搜索、个人、详情、评论、分集/分P、清晰度选择）。
别再往 `pages/` 里加"遥控 mpv"的按钮或进度行——第二十三轮做过一版，全删了，原因与教训见
`docs/architecture.md` 第 4.3 / 4.14 / 8 节。

**视觉常量只有一个出处**：颜色、圆角、字号、卡片的 232×238 这类尺寸一律从 `ui::palette` 取，
复用件从 `ui::layout` 取；页面里出现 `rgba(0x…)` 或 `px(56.)` 这种字面量就是要重构的信号。
参考项目的度量是抄来的不是猜来的，逐条出处见 `docs/architecture.md` 第 4.15 节。

**mpv 只认运行同目录的 `mpv.exe`**（`player::mpv_path()`），不要改回去查 PATH：
`PATHEXT` 先命中 `mpv.com`，那是会再派生 `mpv.exe` 的控制台启动器，我们的 `kill` 管不到真正播放的那一个。

**写 gpui/kit 调用前先确认该写法在已发布版本里存在**：`ref/gpui-kit/examples/` 用的是 kit 本地
main，比 crates.io 的 0.8.0 新。已踩过两条：没有 `v_flex()`/`h_flex()` 自由函数（用
`div().flex()` + `.flex_col()`）；`overflow_y_scroll()` 属于 `Stateful<Div>`，必须先 `.id("...")`。
详见 `docs/architecture.md` 第 2 节与第 8 节。
