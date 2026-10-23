# Bilibili 接口笔记（bpi-rs 0.3.1 + mpv）

本页记录**已核实**的接口形状与外部依赖约束，代码里的取舍都以这里的行号为证据。
来源全部是 `ref/` 下的参考仓库（`ref/` 不入库，所以结论必须抄进本文）。

## 1. bpi-rs 概况

- 包名 `bpi-rs 0.3.1`，MIT，edition 2024，`rust-version = 1.88`；约 309 个 `.rs`、7.1 万行。
- crate 名 `bpi_rs`。顶层 `pub mod`：`client`、`err`、`models`、`session`、以及各领域模块；
  `pub use session::{Account, AccountProfile}`（`src/lib.rs:106`）。
- 我们启用的 feature：`login`、`search`、`video`、`user`、`fav`、`historytoview`
  （默认 `full` 是 27 个模块，全开纯属浪费编译时间）。
- 客户端对象：`BpiClient::builder()...build()?` / `BpiClient::new()`（`src/client.rs:282-291`），
  内部持有 `reqwest::Client`、cookie jar、`Mutex<Option<Account>>`、WBI key 缓存。
  可以长期放在应用状态里复用。
- **builder 默认值**（`src/client.rs:112-126`）：`timeout = 10s`、`connect_timeout = 10s`、
  **`no_proxy = true`**（默认不吃系统/环境变量代理，所以本机 7887 代理与 B 站请求互不干扰），
  并自带 `DEFAULT_USER_AGENT / REFERER / ORIGIN`。结论：请求不会无限挂着——界面若停在加载态，
  应先怀疑 tokio 任务 panic（见 architecture.md 第 4.7 节）。
- `set_account(&self, Account) -> Result<(), BpiError>`（`src/client.rs:294`）会走
  `Account::validate_complete`：DedeUserID / SESSDATA / bili_jct / **buvid3** 四者缺一不可
  （`src/session/account.rs:99-115`），缺任何一个直接返回 `InvalidParameter{field:"account"}`，
  而且 `jar`/`account` 都不提供"读单个 cookie"的公开方法（`get_account` 只在 set 之后才有值）。

## 2. 登录（扫码）

| 调用 | 位置 | 返回 |
| --- | --- | --- |
| `login().qr_generate()` | `src/login/client.rs:193` | `GenerateQrCodeData { url, qrcode_key }` |
| `login().qr_poll(LoginQrPollParams)` | `src/login/client.rs:202` | `CheckQrCodeStatusData { url, refresh_token, timestamp, code, message, cookies: Vec<(String,String)> }` |
| `misc().buvid()` | `src/misc/client.rs:55`，`GET https://api.bilibili.com/x/frontend/finger/spi`（`client.rs:11`） | `BuvidData { buvid3, buvid4 }`（`b_3`/`b_4` 重命名而来） |
| `misc().buvid3()` | `src/misc/client.rs:48`，`GET /x/web-frontend/getbuvid`（`client.rs:10`） | `Buvid3Data { buvid }` |

- **`qr_poll` 成功时 `data.cookies` 只有响应自己的 Set-Cookie**：bpi-rs 用
  `response.cookies()` 收集、并且**只在 `code == 0` 时才填**（`src/login/client.rs:211-223`）。
  B 站给的是登录三件套，**没有 buvid3** → 必须像上面那样补取一个，否则 `set_account` 永远失败。
  选 `misc().buvid()`：它的响应模型是明确 JSON 字段 `b_3`/`b_4`，且 finger/spi 是 web 前端自己
  开屏就在用的匿名接口。`misc().buvid3()`（`/x/web-frontend/getbuvid`）的响应体形态我们没核实过，
  暂不用。为此 bpi-rs 的 features 里加了 `misc`。

- `code == 0` 才算成功；码值翻译集中在 `src/api/login.rs::QrPollState::from_code`
  （`86101` 等待扫码、`86038` 过期→自动重新取码、`86090`/含"扫描"文案 视为已扫码待确认，
  其余归 `Failed{code,message}`）。`qr_poll` 只在 `code == 0` 时抓取响应 cookie。
- **库不做轮询、不做超时、不落盘**，节奏与持久化都由我们负责（`src/login/cookie_refresh.rs` 是空文件）。
- 轮询参数：`bpi_rs::login::LoginQrPollParams::new(key) -> BpiResult<Self>`
  （`src/login/mod.rs:22` 再导出，定义在 `src/login/params.rs:72`）。
- cookie 白名单见 `src/client.rs:79`；我们要求 `DedeUserID/SESSDATA/bili_jct/buvid3` 四个全中，
  缺任何一个 `credential_from_cookies` 返回 `None`（否则会撞上 `Account::validate_complete`）。
- `Account` 只 derive `Deserialize`（`src/session/account.rs:11`），所以落盘走
  `SavedCredential` 镜像结构，见 `src/store/credentials.rs`。

## 3. 搜索

```rust
// src/search/client.rs:105
pub async fn video(&self, params: SearchVideoParams) -> BpiResult<SearchData<Vec<Video>>>
```

- `SearchVideoParams::new(keyword)` **返回 Result**（`src/search/search_params.rs:170`），
  链上 `.with_order(SearchOrder)`（:179）、`.with_page(..)`、`.with_duration(..)`、`.with_tid(..)`。
- `SearchData<T> { seid, page, page_size, num_results, num_pages, result }`。
  - `result: Option<T>`：无结果时上游给 `null` 而不是空数组，转换处要 `unwrap_or_default()`。
  - `pageinfo` 字段是**私有的**，crate 外无法用结构体字面量构造 `SearchData`，
    所以单元测试只测纯函数（标签清洗、数字格式化），不测 `to_outcome` 的完整入参。
  - `page/page_size/num_results/num_pages` 都是 `i64`，与我们的 `page: u32` 比较前先夹成 i64。
- `Video`（`src/search/result.rs:134-192`）可用字段：`bvid, aid, title, author, mid, typeid,
  typename, pic, play, danmaku, favorites, like, tag, review, pubdate, duration, arcurl`。
  - `title` 内含 `<em class="keyword">…</em>` 高亮标签 → 展示前要剥。
  - `duration` 是已格式化的 `mm:ss` 字符串，直接显示即可。
- 端点 `/x/web-interface/wbi/search/type`，WBI 由 `get_wbi_sign2` 自动签（`src/client.rs:137-152`）。
- **P1 落地补充**：
  - `SearchVideoParams::new(kw)`、`.with_page(page)` 都返回 `BpiResult`（会拒绝空关键词与非法页码），
    `with_order/with_duration/with_tid` 返回 `Self`。
  - 排序：`SearchOrder::{TotalRank, Click, PubDate, Dm, Stow}`；我们用
    `SearchSort::{Comprehensive, PlayCount, PublishDate}` 包一层，映射分别到前三个。
  - `pic` 是 `*.hdslb.com` 的公开 CDN，**不需要 Referer**（实测 `http`/`https` × 我们的 UA /
    浏览器 UA × 带不带 Referer 四种组合全部 200）。
  - **但它是协议相对地址 `//i2.hdslb.com/bfs/archive/xxx.jpg`**（真机日志实证，见
    architecture.md 第 4.5 节）：gpui 用 `url::Url::parse` 判 URL，这种写法会被当成内置资源，
    一次网络都不发。展示前必须补 `https:`，交给 `src/ui/cover.rs::canonical`。
  - 图床支持私有缩放后缀 `@{w}w_{h}h.jpg`（**只对 `hdslb.com` 有效**，写错域名或写法返回 400）：
    实测同一张稿件封面原图 541,549 字节 → `@320w_180h.jpg` 14,832 字节；
    头像原图 812,735 字节 → `@112w_112h.jpg` 2,740 字节。搜索一页 30 条的差别是 16MB vs 0.44MB。
  - 对照：`video().view()` / `user().space_info()` 给的 `pic`/`face` 是**绝对地址**（带 https），
    所以同一份日志里头像能正常连上 `i2.hdslb.com`。别假设"所有接口给同一种地址"。

## 4. 视频详情

- `video().view(VideoViewParams) -> VideoView`：`title / pic / owner{mid,name,face} / stat / cid / pages[]`。
- `video().detail(..) -> VideoDetail { view, tags, related, card, reply }`。
- `video().page_list(..) -> Vec<VideoPage>`（多 P，每 P 有自己的 `cid`）。
- `video().player_info_v2(..)` 里有 `subtitle: Option<SubtitleInfo>`，
  `SubtitleItem.subtitle_url` 可直接喂 mpv 的 `sub-add`。
- 个人中心可用：`login().nav()/account_info()/vip_info()`、`user().card()/space_info()/followings()`、
  `fav().folder_info()/list_detail()/created_list()/collected_list()`、
  `historytoview().history_list()/toview_list()`。

## 5. playurl 与 DASH（关键）

```rust
// src/video/client.rs:101
pub async fn play_url(&self, params: VideoPlayUrlParams) -> BpiResult<PlayUrlResponseData>
// 端点 https://api.bilibili.com/x/player/wbi/playurl，WBI 自动签
```

- `VideoPlayUrlParams::from_bvid(bvid, cid)` / `from_aid(..)`；
  构造器 `.quality(qn)`、`.format_flags(fnval)`、`.format_version(fnver)`、`.fourk(bool)`、
  `.high_quality(bool)`、`.platform(..)`（默认 `"pc"`）。**没有** `high_version` 参数。
- `PlayUrlResponseData`（`src/video/videostream_url.rs:104`）关键字段：
  `quality, format, timelength, accept_quality: Vec<u64>, accept_description, video_codecid,
  durl: Option<Vec<DurlInfo>>, dash: Option<DashInfo>, support_formats, last_play_time, last_play_cid`。
- `DashInfo`（:12）= `{ video: Vec<DashStream>, audio: Vec<DashStream>, dolby, flac, duration }`。
- `DashStream`（:11）= `{ id, base_url, backup_url: Vec<String>, bandwidth, mime_type, codecs,
  width: Option<u32>, height: Option<u32>, frame_rate, sar, md5, size, start_with_sap, ... }`。
  `id` 就是画质档（下表），`codecs` 是 RFC 6381 字符串（`hev1.1.6.L150.90` / `avc1.64.00` / `av01...`）。

### 画质档（`src/models/stream.rs:8-38`）

| qn | 名称 | 备注 |
| --- | --- | --- |
| 16 | 360P | |
| 32 | 480P | |
| 64 | 720P | |
| 80 | 1080P | 需登录 |
| 112 | 1080P+ 高码率 | 需登录 |
| 116 | 1080P60 | 需登录 |
| 120 | 4K | 需大会员 |
| 125 | HDR | 需大会员 |
| 126 | Dolby Vision | 需大会员 |
| 127 | 8K | 需大会员 |

我们的默认封顶是 **116**（`src/api/stream_select.rs::MAX_QUALITY_ID`），120/125/126/127 排除：
目标机 i5-6200U（Gen9）硬解不到 4K 与 10bit HEVC。

### fnval（`src/models/stream.rs:47-62`）

`FLV=0`、`MP4=1`、`DASH=16`（**H.264 与 H.265 都靠这一位**）、`HDR=64`、`FOURK=128`、
`DOLBY_AUDIO=256`、`DOLBY_VISION=512`、`EIGHTK=1024`、`AV1=2048`、`AI_FIX=12240`（会顶掉其它值）。
没有独立的 HEVC 位：请求 `DASH` 后 HEVC 与 AVC 同出现在 `dash.video[]` 里，靠 `codecs` 区分。
`VideoCodec`：`Avc=7 / Hevc=12 / Av1=13`。

### 请求头（mpv 必须自带）

bpi-rs 自己请求时会加 `User-Agent` + `Referer: https://www.bilibili.com/` + `Origin`
（`src/client.rs:68-72,394-413`），但**没有对外导出的 header 构造函数**，
所以给 mpv 的头要在 `src/player/mod.rs` 里单独维护，改动时要两边对齐。

## 6. bpi-rs 需要我们补齐的空白

| 空白 | 证据 | 我们的处置 |
| --- | --- | --- |
| `Account` 无 `Serialize`，库不落盘 | `src/session/account.rs:11` | `SavedCredential` 镜像 + `<exe>/data/cookies.json` |
| 视频响应无码流选择助手 | `src/video/videostream_url.rs` 无 impl 方法 | `api/stream_select.rs` 自实现 |
| `models::stream::best_video()` 优先分辨率、会选 4K | `src/models/stream.rs:150-172` | 不使用；且加封顶 |
| `cookie_refresh.rs` 空文件 | `src/login/cookie_refresh.rs` | 自己实现失效检测与重新登录提示 |
| 无自动重试 / 风控处理 | `err/error.rs`、`err/code.rs` | 按 `is_risk_control()`(-352/-412/412)、`requires_login()`(-101)、`requires_vip()`(-106/-650) 分类处理 |
| 扫码成功不补 buvid3（`validate_complete` 却要求它） | `src/login/client.rs:211-223` 只搬响应 cookie | `poll_qr` 在 `code==0` 且缺 buvid3 时调 `misc().buvid()` 补，见第 2 节与 architecture.md 第 4.10 节 |
| 不外发请求头 | `src/client.rs:394-413` | `player::BILIBILI_REFERER` / `MPV_USER_AGENT` 常量 |

## 7. mpv（`ref/mpv`，DOCS/man）

| 需求 | 选项/命令 | 证据 |
| --- | --- | --- |
| 单独一路音频 URL | `--audio-file=<file>`（= `--audio-files-append` 别名，可重复） | `options.rst:2362-2367`；`player/loadfile.c:962` 走 `mp_is_url` → 接受远程 URL |
| 运行中加音频 | IPC `audio-add <url> [flags [title [lang]]]` | `input.rst:767` |
| HTTP 头 | `--http-header-fields`、`--user-agent`、`--referrer`、`--cookies-file` | `options.rst:5770`；`stream/network.c:34` 全在 `mp_network_conf` 组 → **实例级全局生效** |
| Windows IPC | `--input-ipc-server=\\.\pipe\<name>`，前缀缺失会自动补 | `options.rst:4655`、`ipc.rst:64`；`input/ipc-win.c:386` 用 `PIPE_UNLIMITED_INSTANCES` |
| 订阅状态 | `observe_property <id> <name>` → `property-change` | `ipc.rst:279`（连接断开即失效） |
| 窗口行为 | `--force-window`、`--keep-open`、`--idle=once`、`--taskbar-progress` | `options.rst` 对应条目 |
| 时间轴对齐 | `--rebase-start-time=yes`（默认即 yes） | `options.rst:251` |
| 隔离用户配置 | `--no-config`、`--config-dir` | 仅命令行生效 |

**文档里完全没有 DASH/CMAF/m4s 的章节**：双流 seek 与同步必须实测（风险表 #7）。
`--wid` 在 Windows 是把 mpv 自建窗口 reparent 成我们 HWND 的子窗口，我们**不用**它（定稿为独立窗口）。
`mpv 0.2.3`（crates.io）是 libmpv 嵌入绑定，与外置进程路线相反，不引入。

## 8. 视觉规格来源（`ref/Bili.Copilot`，C#/WinUI3）

只移植度量，不移植 Fluent 材质（gpui 无 Mica/Acrylic 等价物）：

- 卡片网格：`VideoLayout` 232×238、间距 20（`App.xaml:69-115`）。
- 卡片内部：封面 `CornerRadius=4`、`DecodeWidth=600`；时长角标黑底 60% + `Margin=8 Padding=4`；
  信息区 `Padding="16,12,16,16" RowSpacing=8`；UP 主名 12px 三级色（`VideoCardControl.xaml`）。
- 外壳：48px 标题栏 + NavigationView 侧栏；搜索框 `MinWidth=320 MaxWidth=500`；
  详情页以 `OverlayFrame` 覆盖在主 Frame 之上（同一窗口）。
- 骨架屏：`ShimmerLayout`（600px 封面 + 3 条微光条）；图片缓存目录 `ImageCache` + 独立 HTTP 客户端。
- **它的 `DecodeWidth=600` 就是"别下原图"的同一件事**：我们改成在 URL 上要小图
  （`@320w_180h.jpg`），比下完原图再解码更省，见第 3 节。
- 它的画质逻辑与我们的定稿一致：`p.Codecs.Contains(prefer_codec)`，默认偏好 `"avc"`（我们改成 HEVC 优先），
  降级用 `availableFormats.Max(Quality)`。

## 9. 个人中心（P2b 已核实）

### 9.1 账号资料

| 调用 | 位置 | 返回 |
| --- | --- | --- |
| `login().nav()` | `src/login/client.rs:99` | `LoginNav { is_login: bool, mid: Option<Mid>, uname: Option<String>, face: Option<String>, wbi_img }`（`src/login/model.rs:8`） |
| `login().stat()` | `src/login/client.rs:107` | `LoginStats { following: u64, follower: u64, dynamic_count: u64 }` |
| `user().space_info(UserSpaceParams::new(Mid))` | `src/user/client.rs:251`、参数 `src/user/params.rs:121` | `UserSpaceProfile { mid: Mid, name: String, sex: Option<String>, face: String, sign: String, level: u8, silence: u8, coins: f64, fans_badge: bool, is_followed: bool, top_photo: Option<String>, official: Option<UserOfficialSummary>, vip: Option<UserVipSummary>, live_room: Option<UserSpaceLiveRoom> }`（`src/user/model.rs:133`） |

- **`space_info` 是我们唯一需要等级与签名的入口**，而且它内部自动做 WBI 签名
  （`sign_wbi_params`，`src/user/client.rs:252`）；`nav()` 只给 `uname/face`。
- **`misc::NavData` 是个坑**：它是 `{ img, sub }` 两个 WBI key（`src/misc/sign/bili_ticket.rs:31`），
  **不是**用户资料。`login::login_info::nav::NavData` 字段丰富但**没有任何方法返回它**（死代码），
  别照抄。
- `UserVipSummary { kind: i32 (serde "type"), status: i32 }`（`src/user/model.rs:188`）：
  `vip_label(status, kind)` 只按"两个都非 0 才算会员"处理，档位文案不猜。

### 9.2 收藏夹

| 调用 | 位置 | 返回 |
| --- | --- | --- |
| `fav().created_list(FavCreatedListParams::new(Mid))` | `src/fav/client.rs:69`、`params.rs:30` | `CreatedFolderListData { count: u32, list: Vec<CreatedFolderItem> }`；item（`info.rs:52`）`{ id, fid, mid: u64, attr: u32, title: String, fav_state: u8, media_count: u32 }` |
| `fav().collected_list(FavCollectedListParams::new(Mid))` | `src/fav/client.rs:81`、`params.rs:79` | `CollectedFolderListData { count, list: Vec<CollectedFolderItem> }`；item（`info.rs:81`）多给 `cover: String, upper: CollectedFolderUpper { mid, name, face }, intro, ctime, mtime` |
| `fav().list_detail(FavListDetailParams::new(MediaId))` | `src/fav/client.rs:105`、`list.rs:121` | `FavListDetailData { info, medias: Vec<FavListMedia>, has_more, ttl }`（P3 再用到） |

- **没有"取自己"的变体**：两类列表都要显式传 `up_mid`，mid 来自凭据的 `DedeUserID`
  （`Mid` 有 `FromStr` 与 `new(u64)`，`src/ids.rs:19/38`，`new(0)` 直接 `Err`）。
- `FavCreatedListParams::new` / `FavCollectedListParams::new` **不失败**；但
  `FavCollectedListParams::with_page/with_page_size/with_platform` 返回 `BpiResult<Self>`。
- 两个 item 结构都**没有 `#[serde(default)]`**：B 站少给任何字段都会变成 `BpiError::parse`，
  所以错误必须上界面（第 4.7 节）。

### 9.3 观看历史

| 调用 | 位置 | 返回 |
| --- | --- | --- |
| `historytoview().history_list(HistoryListParams)` | `src/historytoview/client.rs:36` | `HistoryListData { cursor: HistoryCursor, tab: Vec<HistoryTab>, list: Vec<HistoryListItem> }`（`history.rs:113`） |
| `historytoview().toview_list()` | `src/historytoview/client.rs:54` | `ToViewListData { count, list }`（稍后再看，P2b 未用） |
| `historytoview().delete_history(HistoryDeleteParams::new(kid)?)` | `history.rs:130` | 内部自动取 csrf，未 `set_account` 会 `auth_required` |

- `HistoryListParams::new()` → `.with_type(HistoryListType::Archive)`（`params.rs:83`，返回 `Self`）、
  `.with_page_size(u32) -> BpiResult<Self>`；**分页是游标**：下一页要把 `cursor.max / view_at / business`
  回传给 `with_max / with_view_at / with_business`（`params.rs:96-118`），不是页码。
- `HistoryListData.cursor/tab/list` 无 `#[serde(default)]`，同样解析失败即报错上界面。
- **`bvid` 在 `item.history.bvid: Option<String>`**（`history.rs:52`），顶层是 `kid: u64`；
  专栏/直播那里是 `None`，所以只取 `Archive` 类型才能保证能进详情页。
- 时长与进度是**数字秒**：`duration: Option<u32>`、`progress: i32`（`history.rs:97/101`，
  "没进度"时可能是 `-1`）；跟搜索接口的 `"mm:ss"` 字符串**不是一回事**，格式化在
  `src/api/history.rs::format_seconds`。
