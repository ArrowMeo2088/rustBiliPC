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
- `set_account(&self, Account) -> Result<(), BpiError>`（`src/client.rs:294`）会走
  `Account::validate_complete`：DedeUserID / SESSDATA / bili_jct / **buvid3** 四者缺一不可。

## 2. 登录（扫码）

| 调用 | 位置 | 返回 |
| --- | --- | --- |
| `login().qr_generate()` | `src/login/client.rs:193` | `GenerateQrCodeData { url, qrcode_key }` |
| `login().qr_poll(LoginQrPollParams)` | `src/login/client.rs:202` | `CheckQrCodeStatusData { url, refresh_token, timestamp, code, message, cookies: Vec<(String,String)> }` |

- `code == 0` 才算成功（`86101` = 等待扫码）；`qr_poll` 只在 `code == 0` 时抓取响应 cookie。
- **库不做轮询、不做超时、不落盘**，节奏与持久化都由我们负责（`src/login/cookie_refresh.rs` 是空文件）。
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
  - `pic` 是 `*.hdslb.com` 的公开 CDN，**不需要 Referer**，直接给 gpui 的 URI 图片加载器即可
    （但要装真 HttpClient，见 architecture.md 第 4.5 节）。

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
- 它的画质逻辑与我们的定稿一致：`p.Codecs.Contains(prefer_codec)`，默认偏好 `"avc"`（我们改成 HEVC 优先），
  降级用 `availableFormats.Max(Quality)`。
