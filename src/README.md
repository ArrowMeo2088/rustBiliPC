# src — 项目源代码

Rust crate 根目录（crate 名 `rustbilipc`）。职责划分与维护契约见
[docs/architecture.md](../docs/architecture.md)。

## 当前结构

| 路径 | 职责 | 备注 |
| --- | --- | --- |
| `main.rs` | 启动序列 | **先 `Config::load_or_create()` → 按设置里的"调试模式"决定 `logging::init(enabled)`**（P13 换的顺序：要不要写日志本身就是设置的一项）→ tokio runtime → TLS provider → `BpiClient`（顺手 `relogin::set_client`）→ `with_assets(gpui_kit::assets::**AllAssets**)` → `gpui_kit::init` → 主题 → `set_http_client` → **读 auth** → 开窗，顺序有意义。顶部还有 `#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]`：**控制台从此永远不弹**（子系统是链接期定的，与调试模式无关）。开窗有两个分支：有可用凭据就**只开主窗口**，没有就**只开 320×420 的扫码登录小窗**（`pages/windows.rs`，并把那个窗口句柄登记给重登协调器）；`with_assets` 必须是 `AllAssets`：默认那份 `Assets` 只嵌 106 个图标，侧栏要的 House/Clock/Bookmark/Download 都不在，而找不到图标**只画空白不报错** |
| `lib.rs` | 模块树 + `VERSION` | 逻辑都收敛在 lib crate 里便于 `cargo test` |
| `logging.rs` | 文件日志 | **设置目录**（`%APPDATA%/ArrowMeo/rustBiliPC`）的 `debug.log`（与 `auth.json`、`config.json` 同一层），每次启动清空、级别 Debug、每行 flush，同时复制到 stderr；自实现 `log::Log`，零依赖。**P13 起整个可关**：`init(enabled)` 关着时**不建目录、不开文件、不装 logger、`set_max_level(Off)`**（全项目那些 `log!` 都是空转），`log_path()` 返回 `Option<PathBuf>` 且只在真开出文件时记路径——界面那行不能再说"日志在这个文件里"而其实在别处 |
| `app.rs` | 进程级基础设施 | 全局 tokio runtime（`OnceLock`）、`install_tls_provider()`（rustls 双 provider 会 panic）、`describe_join_error()`（把任务 panic 变成可读文案）、`exe_dir()` 与 **`settings_dir()`/`ensure_settings_dir()`**（用户数据目录，P13 起是三个落盘点的唯一根；`data_dir()` 已删）、**进程级设置快照 `config()` / `init_config()` / `update_config(闭包)`**：`RwLock<Config>`（不是 `OnceLock`——设置页现在真的能改它），改一项**先改内存再写盘**，写盘失败把原因交给界面；起播的封顶档与编码偏好都从这里取，纯函数仍然收显式参数，单测不碰全局状态 |
| `theme.rs` | 全局主题 | **先 `Theme::change(Dark)` 再覆盖语义色**（kit 的 init 默认亮色，不切就出现"页面黑、输入框白"）：底色/字色/描边/输入框/按钮（含 `button_primary`，主按钮不读 `accent`）都对到 `ui::palette`，另外把 `radius` 改成 4、`font_size` 改成 14（WinUI 的 ControlCornerRadius 与 Body）。**这里不另定义一份颜色** |
| `api/` | bpi-rs 封装 + DASH 选择 | `spawn_on_api_runtime` 是唯一的 gpui↔tokio 桥，`format_seconds`/`seconds_to_u32`/**`progress_text`** 放在 `mod.rs`（历史、待看、详情与主页共用；`progress_text` 把 `0` 与 `-1` 一律收成空串，不许显示成「看到 00:00」那种误导）；`login.rs`（取码/轮询 + 码值翻译 + cookie→凭据 + **缺 buvid3 时向 `misc().buvid()` 补取**）、`profile.rs`（`space_info` + `login().stat()`）、`favorites.rs`（`created_list` + `collected_list`，DTO 带 `created` 标记；**P9 起还有 `fetch_items`**：收藏夹内容按页码 `pn/ps` + `has_more`，带服务端 `keyword`，`pick_bvid` 从 `bvid`/`bv_id` 两个键里挑一个，挑不出来的行不进列表）、`history.rs`（`history_list` 只要 Archive；**P9 起接游标**：`HistoryCursor{max,view_at,business}` 三个字段一起回传，取完的判据用"这次没拿到条目"而不是信接口的 cursor）、`home.rs`（**P6 主页**：`video().homepage_recommendations()`，WBI 由库里签；**必须按 `goto=="av"` 过滤**，理由见文件头）、`comments.rs`（**P6b 评论区**：`comment().list()`，**`type=1`+`oid=aid`**、`ps` 被库卡在 1–20、相对时间只做减法不引 chrono）、`toview.rs`（**P8 待看**：`historytoview().toview_list()`，**零参数、没有分页、要登录、不签 WBI**；`ToViewVideoItem` 那一串嵌套结构全无 `serde(default)`，所以坏一次就是整块坏，只砸在本页提示位）、`space.rs`（**P7 用户空间**：`user().card()` 不签 WBI、`uploaded_videos()` 签且带 `tid` 分区筛选、`opus().space_feed()` 是 offset 游标；`tlist` 是 `Value`，[`parse_tlist`] 认不出来就少一个 chip 而不是整块失败）、`detail.rs`（`view` 主体 + `desc`/`tags`/`related_videos`/字幕**四路各自容错**，原因见文件头；**P12 起还有第五路**：库里 `VideoView` 没带 `ugc_season`、带它的那份类型是死代码，所以用 `BpiClient::get`（公开的）自己再发一次 `view` 读原始 `Value`，宽容解析成 `SeasonBrief`（分段、跨段连续行号、`arc.videos` 认"一集自己还有几个 P"），失败只少右栏那一节 + 一行说明）、`collection.rs`（**P12 合集与系列**：`seasons_series_list` 一次同时给合集与系列、内容页两条路（`seasons_archives_list` 有权威 `meta.total`；`series_archives` 不签 WBI 也不给名字），`date_text` 是不引 chrono 的北京时间日期换算，`page.total` 语义没文档所以不显示）、`stream.rs`（`play_url` 取地址 → `StreamChoice`：DASH 双流或 durl 单文件、清晰度中文档、**两轨各自的大小与备用地址**、落盘后缀；**P11 起 `quality_options` 按封顶过滤**、**P13 起封顶值与 `.fourk` 那位都跟着设置页走**（选 4K 才开 4K 位），编码偏好由 `preferred_codec()` 交给 `stream_select`；页面不起播细节都从这里出）、`download.rs`（**P11 下载**：把一条 `StreamChoice` 拼成 `DownloadRequest` → 在 API runtime 上用 gpui 的 `HttpClient` **一块一块读、一块一块写**；`.part` 临时名 + 完成才改名、主地址失败按顺序换 `backupUrl`、取消靠 `AtomicBool` 标志而不是 `abort()`（**理由全在文件头**）、durl 分段直链**明确拒绝**；文件名清洗/重名编号/字节读数/百分比都是纯函数）、`probe.rs`（**P13 起播前的地址探测**：`Range: bytes=0-0` 问一句这个直链通不通，主地址不通就按顺序换 `backupUrl`——**因为 mpv 内部用的地址我们替换不了，只能在交给它之前定**；超时用 `BackgroundExecutor::timer()` + `futures::future::select` 夹，跑在 gpui 前台任务里、用同一个 `HttpClient`，不引第二套 HTTP 栈；`ordered_urls`（主地址在前、去空去重）与 `cdn_headers`（CDN 要的那条 Cookie）**与下载共用**，`download.rs` 里那两份私有副本已删；`choose`/`describe`/`host_of`/`is_playable_status` 都是纯函数，界面只报主机名不报带签名令牌的完整 URL）、`search.rs` 出 DTO（`SearchHit`/`PageInfo`/`SearchSort`），bpi 类型不外泄；`stream_select` 是纯函数 |
| `store/` | 落盘 | **两个文件都在设置目录**（P13 改口：Windows `%APPDATA%\ArrowMeo\rustBiliPC`，Linux `~/.config/ArrowMeo/rustBiliPC`；目录不存在由各自 `save()` 先建）：`auth.rs` = `auth.json`（**旧位置的读取与迁移代码已整块删除**——`legacy_path()`、`app::data_dir()` 都没了，本模块只认这一个路径）、`config.rs` = `config.json`（**四项**：`max_quality` 16…127、默认 116，越界由 `clamp_quality` 收回并留日志；`codec` = `hevc`/`avc`/`av1`；`mpv_path` 空串=自动找；`debug_log` 默认 false。缺字段一律 `#[serde(default)]` 还能读，`load_or_create()` 返回 `(Config, LoadOutcome)`，`RecoveredFromBroken` 那一档让 `main.rs` **强制开日志**——调试模式就写在坏文件里）。`Auth::cookie_header()` 把四条 cookie 拼成 mpv 要的那一行（1080P 直链校验登录态）；明文与风险的说明在 `auth.rs` 文件头 |
| `player/` | 外置 mpv（**只有 `mod.rs` 一个文件**） | **P13 改口：查找顺序 = 设置里指定的文件 → PATH → 运行目录**（`resolve_mpv` 是纯函数、`exists` 判定注入，三条都有单测；`find_mpv()` 读当前设置）。当年那条坑用两条规则绕开：**只拼 `目录/mpv.exe` 再 `is_file()`，绝不经过 `PATHEXT`/`where` 语义**（那会先命中会再派生 mpv.exe 的 `mpv.com`，我们 kill 不到真正在放的进程），且设置里那条必须真是可读文件否则跳过它继续找；`build_arguments` 纯函数（带值选项一律 `--opt=value`，音频 URL 是 `Option`，多条头逗号拼成一个 `--http-header-fields`）；`launch()` 拉起进程、stdout/stderr 都 `null()`（**不接 mpv 的文本输出**）。**播放期的控制与显示全归 mpv 自己的 OSC**：本项目不做进度条/暂停/时间显示，**也没有任何 IPC 客户端代码**（第二十三轮那版 `ipc.rs`+`state.rs` 已全删，`--input-ipc-server` 也不再传——参数表里从来没有它，机制与坑只留档在 `docs/architecture.md` 第 4.14 节）。P11 之后它可以收到**本地成品路径**（下载页「用 mpv 打开」），参数表不变 |
| `pages/` | 按功能划分的页面 + 窗口这一层 | `mod.rs` 里有两个共用导航事件：`OpenVideo{bvid}` 与 `OpenUser{mid,name}`；**`root.rs` 已删除**，改成 `windows.rs`（主窗 1120×760 / 登录窗 320×420 不可缩放；**先开主窗再关登录窗**，因为 Windows 默认 QuitMode 会在最后一个窗口关闭时退进程）+ `gate.rs`（登录窗根视图：摆 `LoginPage`、收到 `LoginSucceeded` 后在**任务里**换窗口）；`loading.rs` 是各入口页共用的「投请求 → 序号判定 → 写回实体」样板，**P13 起还多一手**：错误是 `BpiError` 时先问库里的 `requires_login()`，是"要登录"那一类就记下标志，并在**实体更新之外**那一层调 `relogin::flush`（`App::open_window` 会强制画一帧，不能塞在别人的 `update` 里）；`relogin.rs`（**P13 凭据过期的统一处置**：进程级协调器存"客户端 + 外壳弱句柄 + 登录窗句柄"，处置 = 删 `auth.json` + 开扫码小窗；**去重看窗口句柄还在不在**而不是记布尔——他直接关掉小窗后下一次 `-101` 还能再开，也就不用 `on_window_closed` 观察者了）；`gate.rs` 从此有两种收尾（`GateMode::{Startup, Relogin}`：**重登只关自己再叫外壳刷新，绝不再开第二个主窗口**）；`shell/` **80 宽窄侧栏**（点侧栏入口**先清下钻栈再换页**，不再要求先点"返回"；左上 48 那一格是**登录账号头像、只做显示不可点**；导航项是 Lucide 图标 + 两字竖排；**主页/搜索/收藏/历史/待看/下载/设置** 七入口 + 栈非空时的「返回」；底部 mpv 自检行已删，信息挪进设置页），**下钻栈是枚举**（`StackPage::{Detail, Space, Folder, Collection}`，因为详情/空间/收藏夹内容/合集内容是交错下钻的，分栈就没法"返回上一层"），**外壳还创建下载队列实体**（`DownloadQueue`，注入详情页与下载页，页面之间因此互不认识），三个入口页实体常驻不重刷；`login/` 扫码 + 1.5 s 轮询 + 过期自动重载；`home/` 首页推荐流（**默认页**，构造时就取第一批；"换一批"= 批次号 +1，没有分页条，失败只写这一页的提示位）；`detail/` 主体 + 选集 + **起播**（先 `api::probe` 探一次直链、不通就换 `backupUrl`，再交给 mpv；持 `Child` 并在 `Drop` 里 kill，见第 4.13、4.28 节）+ 字幕 + **评论区**（顶层列表 + 最热/最新 + 上一页/下一页，见第 4.17 节）+ **UP 主那一行整行可点 → 空间页** + 相关推荐（360 右栏）+ **P11 的「下载这一 P」**（同一条取址路，`PlanPurpose` 记住这次是播还是下载 → 弹系统对话框选目录 → 交外壳那份队列）+ **P11 的可选中文本**（标题/简介/评论昵称与正文用 `gpui_kit::base::SelectableText`，页头一颗「复制选中」给触摸屏用，Ctrl+C 由 kit 的 Root 自带）+ **P12 的分集那一节**（右栏最上面：分段 chips、跨分段连续行号、`N P` 角标标出"这一集自己还是多 P"，点一集 = 再压一层详情页；有分集时下面那节标题改成"本集的分 P"）；`download/`（**P11**，两个文件：`queue.rs` = 外壳持有的队列实体（任务表、重名编号、进度事件回传、按标志取消、只清已结束的记录），`view.rs` = 任务列表（进度条/取消/打开所在目录/用 mpv 打开（也持 `Child` + `Drop` kill）/移除记录）；**队列不持久化，也不猜目录——落盘位置只来自弹框**）；`space/` 用户空间（资料卡 + 投稿/图文/收藏三块，**三块各一个序号**，投稿带分区 chips，见第 4.18 节；**P12 加第四格"合集"**：`seasons_series_list` 一次给两类、列成一格，点一行发 `OpenCollection`，同样"第一次切过去才取"）；`collection/`（**P12** 合集/系列内容页：一列分集、跨页累加 + "再加载一页"，接口没有 `keyword` 所以**不放搜索框**，`Archive` 没有 `videos` 所以不标"这一集几个 P"）；`favorites/` 两层（列表 `view.rs`：本地筛选 + 行可点发 `OpenFolder`；内容 `folder.rs`：卡片网格 + 服务端关键词**只认回车** + 加载更多，见第 4.22 节）；`about/`（**P13 起是真设置页**：原来那几行只读信息留在下面，上面加**四个能改的块**——默认播放画质 3 个按钮（B 站没有"2K"档，这句话写在下面）、默认播放编码 3 个按钮（当前档的代价写在下面）、MPV 播放器位置（"选择…"弹 `prompt_for_paths{files:true}`，"恢复自动"只在指定过才出现，下面如实显示本次解析到的路径）、调试模式 开/关（写明**下次启动生效**与控制台从此不弹）；改一项就 `app::update_config` 落盘一次，**写盘失败把原因显示在页面上**，不静默）；`history/` 历史（本地筛选 + 再加载 30 条，**P13：滚到底自动接下一页**）；`toview/` 待看（本地筛选，接口一次给全所以没有翻页）；`search/`：输入 → 回车/按钮/排序/分页 → tokio 请求 → `upgrade()+update()` 回填，**P13：滚到底把下一页"接到"列表尾巴（`append=true`），手动上/下页仍然整屏换；追加失败不清已看到的页**；`space/` 的**图文与合集**两格、`favorites/folder.rs`、`collection/` 同样接了自动翻页（投稿那一格是整屏换页 + 筛选，评论区一页 20 条属随机访问，两处**故意没做**，理由在各自注释与第 4.28 节） |
| `ui/` | 跨页面复用组件 | `palette.rs`：**颜色与尺寸常量的唯一出处**（`rgba` 是 RRGGBBAA；**侧栏宽 80**：`SIDE_COLUMN_WIDTH` 由 240 改成 80 是用户要求的 1/3，反推过程写在那条常量的注释里，`ref/gpui/gpui/src/color.rs:20-23`）；`layout.rs`：由这些常量组成的复用件（56 页头 + 1px 线、圆头像、统计行、提示、卡片壳、`list_row` 一栏「标题+说明」的列表行、**`side_item`：80 宽栏里的「图标在上、两字在下」竖排项，高 48**、**`search_box`：一行输入框（`InputState` 由页面自己造，因为"什么时候算一次搜索"是页面的事）**、**`clickable_row`：可点的一行「标题+说明」（与 `list_row` 同一组度量，多 `id`/`on_click`/hover）**）；`scroll.rs`：**每个滚动容器一个 `DragScroll`**——页面 `render` 里 `scroll.watch(window, cx.entity_id())` 一次，容器链上补 `.track_scroll(scroll.handle())` 与 `.on_mouse_down(Left, scroll.on_press())`。**`Window::on_mouse_event` 的监听只活一帧**，所以绝不能在 `new()` 里登记完事；**P11 加了一条让位规则**：按下时先问 `TextSelection::is_selectable_at(落点)`，落在可选中文字上就不记拖动起点（那一下归文本选择），所以"划文字"与"拖内容"不会同时发生——副作用是**长段落区域只能选字，要滚就得在卡片之间或空白处拖**（Windows 后端没有触摸管线，手指与鼠标是同一条消息，分辨不了来源）；**P13 在这一份里加了自动翻页的判据**：纯函数 `is_at_bottom(offset, max, margin)`（偏移是负的、行程是正的，所以拿 `max - |offset|` 与提前量比）+ `DragScroll::scrollable()`（零行程时 `is_at_bottom` 按定义给真，必须另问一句）+ `should_auto_page(scroll, has_more, items, loading)`（四条判据的分工与"少了哪条会怎样"都写在函数注释里）；`video_card.rs`（竖卡 232×238、封面 232×122、文字区内边距 16,12,16,16、`flex_wrap` 网格 gap 20、hover 只换底色，横卡 104 高用于侧栏，**`post_card` 是图文条目的竖排卡（封面按接口给的比例摆、正文、点赞），因为接口不给标题和时间**）、`cover.rs`（**显示前必过**：补 `//`→`https://` + 拼图床 `@Ww_Hh.jpg` 缩略图后缀）、`qr.rs`（qrcode-generator 出 `Vec<Vec<bool>>`，一格一个 div 画出来）。页面**不许**自己写 `rgba(...)`/`px(56.)` 这类字面量，度量来源逐条记在 `docs/architecture.md` 第 4.15 节 |
| `tests/` | 单元测试 | 只测纯逻辑：码流边界（封顶值现在由调用方给，测试同时验「参数真的生效」与「常量只当默认值」）、**拖动偏移的方向与夹取（`target_offset`：往下拖回顶、往上拖到底、不越界攒死区）**、mpv 参数形状（**带值选项必须 `--opt=value`**、单文件不加 `--audio-file`、不给 Lua 门控的 `--no-ytdl`）、mpv 定位在运行目录、凭据往返与 `cookie_header()` 只列有的字段、搜索标签清洗与播放量格式化、**主页只放行 `goto=="av"`、`stat` 缺失时不写 0、批次大小在接口区间内**、**评论：目标类型码必须是 1、`has_more` 两种口径、相对时间各档且不出负数**、**落盘：`Auth::from_json` 坏文本按未登录处理、`clamp_quality` 收回越界档位、`config.json` 缺字段用默认上限、`quality_name` 认不出就不编名字**、**历史与待看共用 `progress_text`（`0` 与 `-1` 都收成空串）**、**收藏：`pick_bvid` 优先新键再退旧键（空串也算没有）、`matches_keyword` 命中标题或 UP 主且空关键词一律放行**、**空间：`tlist` 的排序与畸形输入、`tid` 缺失时回退到键、图文封面比例（缺数据退回 16:9 不许塌成 0 高）、`last_page` 向上取整（总数正好是整倍数时**不许**多算一页、每页 0 时不除崩）**、登录码值翻译 / cookie 完整性 / buvid3 补取判定、二维码矩阵、秒数格式化与会员文案、封面 URL 补协议与缩放、详情的 `favorite`/`fav` 双字段与字幕地址补协议、清晰度中文档的四级兜底顺序、**清晰度按钮列表按封顶过滤（超过 `cap` 的档不给，全是超限档时退回原样而不是给一排空列表）**、**下载：文件名清洗（非法字符换空格、空白折叠、去掉首尾空格与点、按字符而不是字节截断）、多 P 命名带页码、清洗成空时退回稿件号、重名编号 `(2)`、字节读数、百分比不许冒充 100%、只有全轨大小已知才给分母、durl 分段直链必须被明确拒绝、`with_stem` 同时改两个目标名**、**P12 合集：`date_text` 四个边界（0 给空串、+8h 让"凌晨发布"不归到前一天、闰日、跨年）与 `has_more` 两种口径（有 `total` 比总数、没 `total` 看本页给没给满、空页不翻）**、**P13 设置与选流：`clamp_quality` 的新边界（120/125 现在**允许**、9999 才收到 127、0/6 收到 16）与"默认仍是 116"分开验、`codec` 落盘是小写字符串且**写错值要读失败**（不许静默变默认）、四项往返、`QUALITY_CHOICES` 里**不许出现"2K"**、编码偏好命中时压过默认顺序而没命中时退回（两条各一断言）、画质档永远压过编码偏好（不为 HEVC 放弃更高档）、`resolve_mpv` 三条顺序 + "显式路径读不到就跳下去" + "只问 `mpv.exe` 不问 `mpv.com`" + 三处都没有给 `None`、`path_directories` 滤掉空段（空段在 shell 语义里等于当前目录）**、**P13 探测：`ordered_urls` 主地址在前且去空去重、200/206/3xx 算通而 403/416/5xx 不算、`choose` 探到就停（`attempted` 是"用掉第几个"）、全不通时 `url=None` 好让上层退回主地址、`host_of` 只取主机名（完整直链里是签名令牌）、没探过时界面一句话不给噪音**、**P13 自动翻页：`is_at_bottom` 的负偏移/零行程/拖过头三个边界** |

## 分层规则

- `pages/*` 不直接调接口，网络调用一律经 `api/`。唯一允许的例外是持有
  `Arc<bpi_rs::BpiClient>`（在 `main.rs` 构造后传入，只为 clone 给 `api/`），
  这样页面里不会出现任何 bpi 的请求/响应类型。
- `api/`、`store/`、`player/` 不得依赖 gpui 类型，必须能在无窗口环境下测试。
  **一条已定稿的例外**：`api/download.rs` 收 `Arc<dyn gpui::http_client::HttpClient>`。
  它是"传输抽象"而不是界面类型（`App::http_client()` 返回的那个 reqwest 客户端，与封面图共用一份），
  而 bpi-rs 每个接口都是 `response.bytes().await` 整个进内存——一部片子不能这么读，
  只能用它自己把响应体当 `AsyncRead` 分块搬到磁盘。代价记下来：这一层的这一段
  **没法在 `src/tests/` 里跑**，所以那里只测命名、大小与状态这些纯逻辑。
- gpui 类型统一从 `gpui_kit::*` 取，不单独依赖 gpui（kit 的 umbrella 已把它放在根上）。

## 约定

修改本目录任何代码时，同步更新本 README、`docs/architecture.md` 相应小节，并提交 git。
注释写"为什么"（机制、坑、定稿理由），不写"是什么"。

**异步链路的每一跳都要写日志**（`debug.log`，见 `logging.rs`；**P13 起默认不写**——"调试模式"关着时
`logging::init(false)` 什么都不装，要排查问题先在设置页打开它并重启一次）：api 层在提交前与 tokio 任务内
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

**mpv 的位置按"设置指定 → PATH → 运行目录"找**（`player::resolve_mpv`，P13 改口，可以设置页里选）。
**但"只拼 `目录/mpv.exe` 再判存在"这条不能换成 shell 的 `where`/`which` 语义**：
`PATHEXT` 先命中 `mpv.com`，那是会再派生 `mpv.exe` 的控制台启动器，我们的 `kill` 管不到真正播放的那一个。
设置里写死的那条路径也必须是真实文件，否则跳过它继续往下找（并在设置页说明），不退化成"随便跑个什么"。

**写 gpui/kit 调用前先确认该写法在已发布版本里存在**：`ref/gpui-kit/examples/` 用的是 kit 本地
main，比 crates.io 的 0.8.0 新。已踩过两条：没有 `v_flex()`/`h_flex()` 自由函数（用
`div().flex()` + `.flex_col()`）；`overflow_y_scroll()` 属于 `Stateful<Div>`，必须先 `.id("...")`。
详见 `docs/architecture.md` 第 2 节与第 8 节。
