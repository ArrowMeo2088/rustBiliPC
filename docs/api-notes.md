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

## 4. 视频详情（P3 逐个核对过）

| 调用 | 位置 | 返回 | WBI | 需登录 |
| --- | --- | --- | --- | --- |
| `video().view(VideoViewParams::from_bvid(Bvid))` | `src/video/client.rs:65`，参数 `src/video/params.rs:28` | `VideoView { aid: Aid, bvid: Bvid, videos: u32, title: String, pic: String, owner: VideoOwner, stat: VideoStat, cid: Cid, pages: Vec<VideoPage> }`（`model.rs:6`，只有 `pages` 有 `#[serde(default)]`） | 否 | 否 |
| `video().desc(VideoDescParams::from_bvid(..))` | `client.rs:92`、`params.rs:119` | `BpiResult<String>`（**简介只能这么拿**，`VideoView` 里没有 `desc`） | 否 | 否 |
| `video().tags(VideoTagsParams::from_bvid(..))` | `client.rs:243`、`params.rs:455` | `Vec<video::tags::VideoTag>`：`{ tag_id: Option<u64>, tag_name: String, music_id: Option<String>, tag_type: String, jump_url: Option<String> }`（`tags.rs:12`） | 否 | 否 |
| `video().related_videos(VideoRelatedParams::from_bvid(..))` | `client.rs:206`、`params.rs:337` | `Vec<recommend::RelatedVideo>`：`{ aid: u64, bvid: String, title: String, pic: String, tname: String, pubdate: u64, duration: u64, owner: Owner{mid,name,face}, stat: Stat{view,danmaku,reply,favorite,coin,share,like,...}, cid: u64, rights, dimension }`（`recommend.rs:89`） | 否 | 否 |
| `video().player_info_v2(VideoPlayerInfoParams::from_bvid(Bvid, Cid))` | `client.rs:192`、`params.rs:288` | `PlayerInfoResponseData`，其中 `subtitle: Option<SubtitleInfo>`、`SubtitleInfo.subtitles: Vec<SubtitleItem>`、`SubtitleItem { lan, lan_doc, subtitle_url, ai_status: u8, id: u64, is_lock, ai_type, subtitle_type }`（`player.rs:24/93/103`）；另有 `need_login_subtitle: bool` | **是** | 字幕可能要登录 |
| `video().detail(VideoDetailParams::from_bvid(..))` | `client.rs:74` | `VideoDetail { view, tags, related, card: Option<Value>, reply: Option<Value> }`（字段是 `#[serde(rename="View")]` 等大写法） | 否 | 否 |

- **`detail()` 不要用**：它的 `tags` 是 `model::VideoTag`，`tag_id: u64` **既非 Option 也无 default**
  （`model.rs:100-108`），而 B 站的 BGM/话题标签常省 `tag_id` ⇒ 整个 `detail()` 解析失败、
  连标题一起没。`tags()` 返回的是另一个容错类型（`tags::VideoTag`），两者**同名不同型**，
  用错路径就是类型不匹配。
- `VideoStat` 的收藏数有**两个**字段：`favorite`（新）与 `fav`（别名），都是 `Option<u64>+default`
  ⇒ 我们封装成 `api::detail::favorite_count(favorite, fav)`，只读一个会显示 0。
- 单位：`VideoPage.duration`、`RelatedVideo.duration`、`pubdate/ctime` 是**秒**；
  只有 `PlayUrlResponseData.timelength` 是**毫秒**（P4 播放要用，别混）。
  `OnlineTotalResponseData.{total,count}` 是 **`String`** 不是数字。
- ID 是 newtype，在 `bpi_rs::ids::{Aid,Bvid,Cid,Mid}`（**crate 根没 re-export**）：
  `Aid::new(0)` 报错、`Bvid::new("BV…")` 要求 `BV` 前缀 + 长度 ≥12 + 全 ASCII 字母数字
  （`ids.rs:161-183`），坏的 bvid 在我们这层就变 `InvalidParameter`，不会去打网络；
  `.get() -> u64` / `.as_str() -> &str` / 都实现了 `FromStr` 与 `Display`。
- **`ref/bpi-rs/src/video/info/` 整块是死代码**：`VideoData`（有 `desc/pubdate/tname/duration/
  is_chargeable_season`）、`PageItem`、`VideoDescResponse`、`VideoDetailData` 没有任何方法返回它们。
  想要的字段如果只在那里，说明**当前拿不到**，别 import。
- 空 stub 文件（只有文档注释、没有代码）：`video/appeal.rs`、`attribute_data.rs`、`pbp.rs`、
  `snapshot.rs`、`video_zone.rs`。
- `get_wbi_sign2` 是 `pub(crate)`：不能自己给别的 URL 签名，只能用库里已经签好的方法。

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
1080P 及以上还要**登录 cookie**：`SavedCredential::cookie_header()` 从我们自己的凭据现拼一条
`Cookie: DedeUserID=…; SESSDATA=…; bili_jct=…; buvid3=…` 塞进 `--http-header-fields`
（bpi-rs 的 cookie jar 是私有字段，拿不到）。

### P4a 落地时核对到的事实（代码已按这些写）

- **请求档 ≠ 得到档**：`.quality(116)` 之后 B 站按账号权限回落，实际能给的高清档写在
  `data.quality`，而每条流自己还带 `DashStream.id`。界面报的是**选中那条流的 `id`**，
  只有 `durl` 回退时才退回 `data.quality`。真机已验证这条：请求 116、回来
  `qn=80 · codecs=hev1.1.6.L150.90`（第十八轮日志），封顶与编码优先级都按预期工作。
- **`timelength` 单位是毫秒**，`DashInfo.duration` 与 `VideoPage.duration` 是秒；
  DTO 里那个字段一律叫 `timelength_ms`，避免又一次单位事故。
- **`support_formats` 才是给人看的清晰度列表**：`SupportFormat`
  （`src/video/videostream_url.rs:93`）= `{ quality, format, new_description,
  display_desc, superscript, codecs: Option<Vec<String>> }`。
  `new_description` 是"1080P 高清"这种中文，`display_desc` 是"1080P"这种短标签，
  优先前者（`api::stream::quality_label` 的顺序：`new_description` → `display_desc` →
  `accept_quality`/`accept_description` 按下标配对 → 兜底 `"{qn}P"`）。
- **`durl` 与 `dash` 互斥**：`durl: Vec<DurlInfo>` 是已经合成好的一路文件
  （`DurlInfo` = `{ order, length, size, ahead, vhead, url, backup_url }`），里面**已含音频**，
  所以 `audio_url = None`；此时 `codecs` 那格填 `data.accept_format`（没有具体串可报）。
  `backup_url` 两处都有 `deserialize_with = "deserialize_vec_or_default"`，
  B 站给 `null` 也不会炸，可以放心 `.clone()` 出来当备用地址列表。
- **响应结构全无 `#[serde(default)]`**（`videostream_url.rs:104-125`）：`from`/`result`/`message`/
  `video_codecid`/`seek_param`/`seek_type`/`last_play_time`/`last_play_cid` 少任何一个、
  或 `dash` 里少 `duration`，整次 playurl 就变成 `BpiError::parse`。
  **真机如果起播失败并打出一段解析错误原文，第一反应是"字段缺失"，不是登录或会员问题**——
  这两类症状在界面上都是"取播控地址失败：…"。

## 6. bpi-rs 需要我们补齐的空白

| 空白 | 证据 | 我们的处置 |
| --- | --- | --- |
| `Account` 无 `Serialize`，库不落盘 | `src/session/account.rs:11` | `SavedCredential` 镜像 + `<exe>/data/cookies.json` |
| 视频响应无码流选择助手 | `src/video/videostream_url.rs` 无 impl 方法 | `api/stream_select.rs` 自实现 |
| `models::stream::best_video()` 优先分辨率、会选 4K | `src/models/stream.rs:150-172` | 不使用；且加封顶 |
| `cookie_refresh.rs` 空文件 | `src/login/cookie_refresh.rs` | 自己实现失效检测与重新登录提示 |
| 无自动重试 / 风控处理 | `err/error.rs`、`err/code.rs` | 按 `is_risk_control()`(-352/-412/412)、`requires_login()`(-101)、`requires_vip()`(-106/-650) 分类处理 |
| 扫码成功不补 buvid3（`validate_complete` 却要求它） | `src/login/client.rs:211-223` 只搬响应 cookie | `poll_qr` 在 `code==0` 且缺 buvid3 时调 `misc().buvid()` 补，见第 2 节与 architecture.md 第 4.10 节 |
| 不外发请求头，也不交出 cookie jar（`jar` 是私有字段） | `src/client.rs:394-413` | `player::BILIBILI_REFERER` / `MPV_USER_AGENT` 常量 + `SavedCredential::cookie_header()` 现拼一条 `Cookie:` 给 mpv |

## 7. mpv（`ref/mpv`，DOCS/man）

> **标注**：下面凡是"IPC / 订阅 / 命令"相关的行，当前代码**不使用**（范围定稿：播放期归 mpv 自己的
> OSC，客户端不做遥控 UI，见 architecture.md 第 4.3 节）。这些结论是第二十二~二十三轮为那版管道
> 客户端核实并踩过的，P6 的"同实例换清晰度 / `sub-add`"要用时照此实现，不必重新查证。
> 客户端不必重新查证。现在**连 `--input-ipc-server` 都不传**（参数表里没有它），
> 以免留下"是不是还有人在听这条管道"的错觉；P6 恢复时按下面两行把参数与序号规则一起加回来。

| 需求 | 选项/命令 | 证据 |
| --- | --- | --- |
| **命令行语法** | 带值的选项必须 `--opt=value`；`--opt value` 是致命错误 | `DOCS/man/mpv.rst:440-444`（只额外承认单连字符 legacy `-opt value`）；本机复现原文：`Error parsing commandline option user-agent: option requires parameter` → `Exiting... (Fatal error)`，**在建窗口之前**退出 |
| 关掉 URL 提取器（**我们不给**） | `--no-ytdl`（`ytdl` 是 bool，默认 true） | `options/options.c:572` 的 `{"ytdl", OPT_BOOL(lua_load_ytdl)}` 整块在 `#if HAVE_LUA` 里 ⇒ 不带 Lua 的构建会把它当未知选项、又是"窗口不出现"，所以参数表只保留所有构建都有的选项 |
| 单独一路音频 URL | `--audio-file=<file>`（= `--audio-files-append` 别名，可重复） | `options.rst:2362-2367`；`player/loadfile.c:962` 走 `mp_is_url` → 接受远程 URL |
| 运行中加音频 | IPC `audio-add <url> [flags [title [lang]]]` | `input.rst:767` |
| HTTP 头 | `--http-header-fields`、`--user-agent`、`--referrer`、`--cookies-file` | `options.rst:5770`；`stream/network.c:34` 全在 `mp_network_conf` 组 → **实例级全局生效**；`--http-header-fields` 是 string list，**按 `,` 切分**且**重复给同一选项是替换**（`m_option.c:1403-1440`、`m_option.h:644`），故多条头拼成一个参数一次给出 |
| Windows IPC | `--input-ipc-server=\\.\pipe\<name>`，前缀缺失会自动补 | `options.rst:4655`、`ipc.rst:64`；`input/ipc-win.c:352-359` 服务端建法是 `PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED` + `PIPE_TYPE_MESSAGE | PIPE_READMODE_BYTE` + `PIPE_UNLIMITED_INSTANCES` + `PIPE_REJECT_REMOTE_CLIENTS` ⇒ 多客户端合法、**同名不报错**，所以管道名必须每次起播都换序号 |
| 客户端要同时读写这条管道 | 必须用 overlapped I/O（或封装好的库） | `ipc.rst:69-73` 原文：要像 Linux 那样同时读写，"得写一个用 overlapped file I/O（或 .NET `NamedPipeClientStream` 那种封装）的程序"。我们选 `tokio::net::windows::named_pipe`：`try_read`/`try_write`/`ready` 都取 `&self`，同一个任务里读写能交错，overlapped 由 tokio 内部处理。**当时**给 tokio 开过 `net` feature，
随范围纠正已收回（要用时再开，`mio`/`socket2`/`windows-sys` 本来就在依赖图里，`Cargo.lock` 不动） |
| 订阅状态 | `observe_property <id> <name>` → `property-change` | `ipc.rst:269-289`：id 只要在同一条连接内唯一；**连接一断注册即失效**（文档专门警告"用两次 socat 分别收发"会看起来不工作）⇒ 订阅与命令必须走同一条一直开着的连接 |
| 属性名的出处 | `time-pos` / `duration` / `pause` / `eof-reached` / `idle-active` / `paused-for-cache` | `ref/mpv/player/command.c`：`duration` :4697、`time-pos` :4708、`eof-reached` :4727、`paused-for-cache` :4737、`idle-active` :4743。值可能是 `null`，界面要能分开"没进度"与"0 秒" |
| 跳转与换档 | `seek <秒> [relative]`、`loadfile <url> [flags]`、`audio-add <url>` | `input.rst:300-308`（默认就是 relative）、`:542`、`:767`。**坑**：`:546-548` 明说替换当前文件的 `loadfile` 会在旧文件还没停、新文件还没开始加载时就返回 ⇒ **不能拿它的回包判断流开没开成** |
| 文本消息走 **stdout** | 我们**不接**（`launch()` 一律 `null()`） | `common/msg.c:190-202` 的 `term_msg_fileno`：只有 `force_stderr` 才走 stderr；本机实测一次失败的打开 stdout 613 字节、stderr 0 字节。记这条是为了以后别又"只接 stderr" |
| 窗口行为 | `--force-window`、`--keep-open`、`--idle=once`、`--taskbar-progress` | `options.rst` 对应条目 |
| 时间轴对齐 | `--rebase-start-time=yes`（默认即 yes） | `options.rst:251` |
| 隔离用户配置 | `--no-config`、`--config-dir` | 仅命令行生效 |

**文档里完全没有 DASH/CMAF/m4s 的章节**：双流 seek 与同步必须实测（风险表 #7）。
`--wid` 在 Windows 是把 mpv 自建窗口 reparent 成我们 HWND 的子窗口，我们**不用**它（定稿为独立窗口）。
`mpv 0.2.3`（crates.io）是 libmpv 嵌入绑定，与外置进程路线相反，不引入。

## 8. 视觉规格来源（`ref/Bili.Copilot`，C#/WinUI3）

只移植度量，不移植 Fluent 材质（gpui 无 Mica/Acrylic 等价物）。
**逐条数字与文件行号在 `docs/architecture.md` 第 4.15 节**（那里是权威清单，本处只留结论与几件额外事实）：

- 卡片网格：`VideoLayout` 232×238、列距/行距 20（`App.xaml:70-76`）。
- 卡片内部：文字三行是 `Auto`/`*`/`Auto`，顺序 **UP 主 → 标题 → 统计**；角标黑底 60% + `Margin=8 Padding=4`；
  信息区 `Padding="16,12,16,16" RowSpacing=8`；12px 三级灰（`VideoCardControl.xaml`）。
- 外壳：48px 标题栏（`RootLayout.xaml:33`）+ 240 侧栏（`LayoutPageViewModelBase.cs:41`）；
  搜索框 `MinWidth=320 MaxWidth=500`（`RootLayout.xaml:74-80`）；
  详情页以 `OverlayFrame` 覆盖在主 Frame 之上（同一窗口）。
- 选中态：`ItemContainer` 的 `PART_SelectionVisual` 是**常驻的 2px 透明边框**
  （`Overrides.xaml:190-197`），选中只换 brush（`:233-247`）——所以我们的列表行也一律 `border_2()`，
  否则点选会抖 2px。
- 骨架屏：`ShimmerLayout`（600px 封面 + 3 条微光条）；图片缓存目录 `ImageCache` + 独立 HTTP 客户端。
- **它的 `DecodeWidth=600` 就是"别下原图"的同一件事**：我们改成在 URL 上要小图
  （`@320w_180h.jpg`），比下完原图再解码更省，见第 3 节。
- 它的画质逻辑与我们的定稿一致：`p.Codecs.Contains(prefer_codec)`，默认偏好 `"avc"`（我们改成 HEVC 优先），
  降级用 `availableFormats.Max(Quality)`。
- **kit 组件走全局主题，`gpui_kit::init` 默认亮色**（这是"搜索框白底黑字"的根因）：
  gpui-component 0.8.0 `src/theme/mod.rs:40` 是 `Theme::change(ThemeMode::Light, None, cx)`。
  核过的取值链路：`Input` 的底 = `Theme::input_background()`（`theme/mod.rs:485`，暗色下是
  `colors.input` 与透明色混 30%），字色 = `colors.foreground`，描边 = `colors.input`；
  `Button::primary()` 的底色 = `tokens.button_primary`（`button/button.rs:1002`），
  **不是 `colors.accent`**；`Theme::change(mode, …)` 会整体换上该模式的调色板（`theme/mod.rs:386-391`），
  所以自定义色必须在它之后写。`Theme::radius`（`:185`）与 `Theme::font_size`（`:168`）也是公开字段，
  可以直接改成 WinUI 的 4px 圆角与 14px Body。
- **gpui 侧的一条查证方法（本轮新增）**：`rounded` / `border_2` / `gap` / `min_h` / `h_full` / `p_4`
  在 `ref/gpui/gpui/src/styled.rs` 里 **grep 不到**，它们是 `gpui-pre-macros` 的 `styles.rs` 用
  "前缀表 × 后缀表"生成的（`styled.rs:27-34` 那一串 `gpui_macros::*_style_methods!()`）。
  要确认某个 tailwind 式方法存在，下 `gpui-pre-macros-<版本>.crate` 读
  `box_prefixes()/border_prefixes()/corner_prefixes()/padding_box_style_prefixes()`；
  别因为 `styled.rs` 没有就判定 API 不存在。同一批核过的还有：
  `hover` 在 `InteractiveElement`（`elements/div.rs:842`，`Div` 与 `Stateful<Div>` 都实现：`:1877`、`:4087`）、
  `ElementId: From<String>`（`window.rs:7676`）、`cx.listener` 的返回形状（`app/context.rs:246-250`）、
  `From<Rgba> for Hsla`（`color.rs:677`）与 `for Background`（`:960`）。

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

## 10. 主页（P6 第一步，已核对到行号）

| 项 | 结论 | 证据（`ref/bpi-rs`） |
| --- | --- | --- |
| 调用 | `client.video().homepage_recommendations(VideoHomepageRecommendationsParams)` → `RcmdFeedResponseData` | `src/video/client.rs:215-225` |
| 端点 | `https://api.bilibili.com/x/web-interface/wbi/index/top/feed/rcmd` | `src/video/recommend.rs:6-7` |
| WBI | **库里自动签**：`get_wbi_sign2(params.query_pairs())` 在发请求前调用 | `src/video/client.rs:218` |
| 参数 | `ps`（1–30，默认 12）、`fresh_idx`、`fetch_row`；查询串还会固定带上 `fresh_type=4` 并把 `brush` 写成 `fresh_idx` | `src/video/params.rs:350-397` |
| 校验 | `page_size` 越界 → `BpiError::invalid_parameter`（**本地就失败，不会发请求**）；`fresh_idx`/`fetch_row` 必须非 0 | `params.rs:366-371/377-385` |
| 条目字段 | `bvid/cid/duration/goto/id/is_followed/is_stock/owner/pic/pos/pubdate/rcmd_reason/show_info/stat/title/track_id/uri`；`av_feature`/`business_info`/`room_info` 是 `Option<Value>`，`stat` 是 `Option<HomeRmdStat>` | `recommend.rs:125-163` |
| 时长单位 | **数字秒**（`duration: u64`），与搜索接口的 `"mm:ss"` 字符串不同 ⇒ 必须 `format_seconds` | `recommend.rs:131` |
| `goto` | `"av"`=稿件、`"live"`=直播、`"ogv"`=剧集；后两者的 `bvid` 不是稿件号 | `recommend.rs:135-136` |
| 统计数据 | `HomeRmdStat { view, danmaku, like }`（**只有这三个**，没有收藏/硬币） | `recommend.rs:52-60` |
| 推荐理由 | `RcmdReason { reason_type: u8, content: Option<String> }`（0 无 / 1 已关注 / 3 高点赞量） | `recommend.rs:114-122` |
| 响应外层 | `RcmdFeedResponseData { item, mid, preload_expose_pct, preload_floor_expose_pct }`，**除 `item` 外都是必填数字**，B 站少给任一都会解析失败 | `recommend.rs:165-172` |

- 与"热门/排行榜"的区别：`video_ranking().popular_list()` / `ranking_list()` 在**另一个 feature**
  （`video_ranking`）里，而且载荷直接是 `Vec<serde_json::Value>`
  （`src/video_ranking/popular.rs:9-15`、`ranking.rs:8-14` 的注释写"字段未提供，用 Value 代替"），
  要我们自己从 JSON 里挖字段。主页先走已启用的 `video` feature 这条路，不新增 feature、不自己挖 Value。

## 11. 评论区（P6b，已核对到行号）

| 项 | 结论 | 证据（`ref/bpi-rs`） |
| --- | --- | --- |
| 调用 | `client.comment().list(CommentListParams)` → `CommentListData` | `src/client.rs:484`、`src/comment/client.rs:43-50` |
| 端点 | `https://api.bilibili.com/x/v2/reply`，**普通 GET，没有 WBI 签名**（对比 `homepage_recommendations` 会先 `get_wbi_sign2`） | `src/comment/client.rs:7`、`:44-50` |
| 目标 | `CommentTarget::new(type: i32, oid: i64)`，两者都必须 >0；**视频是 `type=1`，`oid` 是 aid（数字稿件号，不是 bvid）** | `src/comment/list.rs:20-40` |
| 类型代码 | `1 视频 / 2 话题 / 8 直播 / 11 相簿 / 12 专栏 / 14 音频 / 17 动态 / 18 播单 …`。**11 是相簿**——网上很多帖子把"视频"写成 11，是混了另一套编号 | bilibili-API-collect 的 comment 类型代码表；bpi-rs 的 `CommentType::Video = 1`（`src/comment/action.rs:22-27`）与它 live 测试里的 `TEST_TYPE: i32 = 1`（`src/comment/list.rs:322`）互相印证 |
| 分页 | 页码制：`pn`（≥1）+ `ps`；**本库把 `ps` 卡在 1–20**（接口本身允许到 49），超了直接 `InvalidParameter` | `src/comment/list.rs:89-101` |
| 排序 | `sort`：0 时间 / 1 按点赞 / 2 按回复（`CommentSort::{Time, Like, Replies}`） | `src/comment/list.rs:52-66`、`:74` |
| 总数 | `CommentListData.page` 是 `Option<PageInfo>`，`PageInfo.count` = 根评论条数、`acount: Option<u64>` 是含楼中楼的总数 | `src/comment/list.rs:247-249`、`src/comment/types.rs:53-61` |
| 列表 | `CommentListData.replies: Option<Vec<Comment>>`（禁用评论区时是 null）；另有 `upper`（置顶）、`top`、`cursor` | `src/comment/list.rs:247-259` |
| 单条 | `Comment { rpid, oid, count(子评论数), rcount, ctime(秒), like, member: Member, content: Content, ... }`；`Member.uname / Member.avatar`、`Content.message` | `src/comment/types.rs:11-52`、`:62-95`、`:158-170` |
| 严格性 | `member`、`content`、`up_action`、`reply_control`、`folder` 以及 `Member` 里的 `level_info/pendant/nameplate/official_verify/vip` **都不是 Option、也没有 `#[serde(default)]`** ⇒ B 站少给任一就 `BpiError::parse` | `src/comment/types.rs` 通篇 |
| feature | `comment = []` 是**纯 cfg 开关**，不引入新 crate，`Cargo.lock` 应当一字不变 | `ref/bpi-rs/Cargo.toml:54` |

- 回复（楼中楼）另有一条 `comment().replies(CommentRepliesParams::new(target, root_rpid)?...)`
  （`src/comment/client.rs:53-58`、`list.rs:132-142`），返回同一个 `CommentListData`。
  P6b 只做顶层 + 翻页，**没接子回复展开**：要做的话得先定"展开后插在哪"这个结构问题。
- 相对时间在接口里**没有**，只有 `ctime`（unix 秒）。参考项目也是自己算的
  （`CommentItemControl.xaml:97-104` 绑的是 `RelativeTime`，绝对时间只进 ToolTip）。

## 12. 用户空间（P7，已核对到行号）

| 项 | 结论 | 证据（`ref/bpi-rs`） |
| --- | --- | --- |
| 资料卡 | `user().card(UserCardParams::new(mid))` → `UserCardProfile` | `src/user/client.rs:124-130` |
| 资料卡载荷 | `card: UserCardSummary{mid,name,sex,face,sign,fans,attention}` + 顶层 `following, archive_count, article_count, follower, like_num` | `src/user/model.rs:7-52` |
| WBI | `card()` **不签**；`uploaded_videos()` **签**（`sign_wbi_params`） | `client.rs:125-130` 与 `:271-278` |
| 投稿端点 | `https://api.bilibili.com/x/space/wbi/arc/search` | `tests/contracts/user/public-read/uploaded-videos/contract.json` 的 `request.url` |
| 投稿参数 | `new(mid)` / `with_order(UserUploadedVideoOrder)` / `with_tid(u64)`（**0=全部**）/ `with_keyword` / `with_page(pn)?` / `with_page_size(ps)?` | `src/user/params.rs:532-586` |
| 排序枚举 | `Pubdate / Click / Stow` → 查询值 `pubdate/click/stow` | `params.rs:495-512` |
| 投稿载荷 | `UserUploadedVideos{ list: UserUploadedVideoList{ tlist: Value, videos: Vec<UserUploadedVideo> }, page: {count,pn,ps}, episodic_button: Option, is_risk }` | `model.rs:796-818` |
| 单条投稿 | `UserUploadedVideo{aid,bvid,mid,title,author,pic,length,description,created,play,comment,typeid,video_review,hide_click}`，**除四个标识外都带 `#[serde(default)]`** | `model.rs:821-861` |
| 时长口径 | `length` 已经是 `"mm:ss"` 字符串（搜索接口一致，历史接口才是数字秒） | `model.rs:833-834` |
| **分区筛选来源** | `list.tlist` —— 同一个响应里回"这个 UP 用过的分区摘要"，键是 tid，值里有 `name`/`count`；**库里给的是 `serde_json::Value`，没有类型可跟** | `model.rs:812-814` |
| 图文 | `opus().space_feed(OpusSpaceFeedParams::new(mid).with_page(page).with_offset(str)?.with_kind(kind))` → `SpaceData{has_more, items, offset, update_num}` | `src/opus/client.rs:24-31`、`src/opus/params.rs:24-77`、`src/opus/space.rs:44-53` |
| 图文端点 | `https://api.bilibili.com/x/polymer/web-dynamic/v1/opus/feed/space`，分页是 **offset 游标**（下一页要回传上一次给的 `offset`） | `src/opus/client.rs:4-5`、`params.rs:64-77` |
| 图文条目 | `SpaceItem{content, cover: Option<SpaceCover{height,url,width}>, jump_url, opus_id, stat: SpaceStat{like: String, view: Option}}` —— **没有标题、没有发布时间，点赞已经是字符串** | `src/opus/space.rs:10-40` |
| 收藏 | 复用 `fav().created_list(FavCreatedListParams::new(up_mid))`：`up_mid` 就是入参，**取别人的收藏夹和取自己的是同一条代码** | `src/fav/params.rs:22-37`、我们的 `src/api/favorites.rs::fetch_folders` |
| feature | `opus = []`、`user = []`、`fav = []` 都是纯 cfg 开关，不引入新 crate | `ref/bpi-rs/Cargo.toml:59,67,69` |

- 空间页三块的**分页口径互不相同**：投稿是页码（`pn/ps` + 总数），图文是游标（只能往后、界面做"加载更多"并累加），收藏一次给全。
  所以三块各用一个序号（`video_seq/post_seq/folder_seq`），不能共用：共用会出现"图文在途时点了投稿的下一页，两边互相把对方判成过期"。
- `tlist` 的解析**不许让整页失败**：`api::space::parse_tlist` 只认"`{tid: {name,count}}`"，认不出来就少一个 chip，
  这条有单测（含畸形输入与"键当 tid"的回退）。
