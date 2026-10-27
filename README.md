# rustBiliPC

用 Rust 编写的 Bilibili 桌面客户端（Windows x64）。**完全不使用浏览器技术**：
没有 Electron / WebView / JS / TS，界面是 gpui（Zed 的 GPU 加速 UI 框架）经
Direct3D11 + DirectComposition + DirectWrite 原生绘制；视频播放交给外置 mpv 进程。

## 解决方案简介

| 方面 | 选择 |
| --- | --- |
| 界面 | `gpui-kit 0.8.0`（umbrella，内含 gpui-component 0.8.0 组件层），实际 gpui 快照 `gpui-pre =0.3.9` |
| 接口 | `bpi-rs 0.3.1`，关闭默认 `full`，只开 login/search/video/user/fav/historytoview/misc/comment |
| 登录 | 扫码登录为主：启动时若**用户数据目录**里的 `auth.json` 没有可用凭据，**先弹一个 320×420 的小登录窗**，扫完码才开主窗；**跑起来之后凭据过期（`-101`）也会自己再开这个小窗**（P13） |口（2026-10-26 定稿）。凭据与设置都是明文 JSON 平铺在运行目录（`auth.json` / `config.json`，不再写 `data/`） |
| 播放 | 把 DASH 直链 URL 交给外部 mpv **在线流式播**（不下整片），独立窗口。**播放期的一切（进度条/时间/暂停/音量/全屏/字幕显示）都归 mpv 自带的 OSC，本项目不做**；我们只做播放之外（见下一行） |
| 范围 | rustBiliPC 负责：主页（首页推荐）、搜索、收藏/历史/待看（各一个侧栏入口，能搜）、视频详情、评论区（**文字可选中可复制**）、**用户空间（投稿/图文/收藏/合集与系列）**、**分集与分 P（合集里的分段；一集自己还有多 P 时标 `N P`，点进去照常能播/能下）**、清晰度档位（按设置封顶）、**下载（双文件，落盘位置每次弹框选）**、拉起 mpv（**起播前会先探一次直链地址**）；**长列表滚到底自动接下一页**（P13）。不负责：播放器界面本身；**字幕投递与同实例换清晰度也不做**（用户 2026-10-25 判定"两个都没必要"） |
| 画质 | **设置页可选**：1080P / 1080P 60帧 / 4K（默认 116），同档内编码偏好可选 HEVC / AVC / AV1（默认 HEVC）。B 站没有"2K"这一档，界面上不造 |
| 编译 | 本机**不装** Rust 工具链，全部由 GitHub Actions 编：`ci.yml` push 自动跑 win x64 release + 单测；`cd.yml` 手动出多平台发布包（LTO/strip + UPX，只打 exe） |
| 阶段 | P0–P13 全部编译交付（P13 见本节末尾那条）。**真机进度**：P8 与 P9 已在他机器上跑过（`debug.log` 里收藏下钻、收藏夹内容 19 条、历史 30 条、待看 6 条、详情字幕/评论全通，零错误），登录独立小窗与 P10 的拖动滚动也已真机确认。**P11**（第四十二轮 75 passed、零 Rust warning，产物 md5 `dd020f45…`）= 详情页与评论区文字**可选中 + 复制**（gpui 没有选择机制，用 kit 的 `base::SelectableText`；触摸屏给一颗「复制选中」）+ **下载**（`.m4v`+`.m4a` 双文件、画质吃封顶、只下当前 P、每次弹框选目录、进度与取消、备用域名、打开所在目录/用 mpv 打开）——**这是唯一允许写到运行目录之外的功能**（用户 2026-10-26 定稿）。另修掉 roadmap R0 两条：清晰度按钮以前能点出 4K/HDR（现在按 `config.json` 封顶过滤），`src/README.md` 里那条早已不存在的 `--input-ipc-server` 说法。第四十轮红过一次，原因写在第 8 节。见第 4.24、4.25 节。**P12**（第四十三轮一次过：82 passed、零 Rust warning，产物 md5 `a591abdf…`）= 他推翻自己"合集列表不做"之后的**合集与系列 + 分集**：空间页第四格同时列合集与系列、点进去是一页页分集（这两条接口没有 `keyword`，所以不放搜索框），详情页右栏最上一节"分集"带**分段 chips**、跨分段连续行号与 `N P` 角标——"复杂"就是他那张 189 集的截图：合集里再分段，而一集自己还是个多 P 稿件。分集信息库里那份 `VideoView` 没带（带它的类型是死代码），所以用 `BpiClient::get` 自己再发一次 `view` 读原始 JSON、宽容解析，失败只少那一节。见第 4.26 节。**P13**（第四十四轮红 5 条、四十五轮红 1 条，正在第四十六轮上）= 四类完善：
① **设置页四项真能改**（MPV 位置默认可选文件、默认画质 1080P/1080P60/4K、默认编码 HEVC/AVC/AV1、
调试模式默认关——关时不产生 `debug.log`、不输出日志，Windows 上 exe 改 GUI 子系统所以永不弹控制台）；
② 三件补齐：**起播前探地址、主地址不通就换 `backupUrl`**（mpv 内部的地址换不了，只能在交给它之前定），
**凭据 `-101` → 自动开扫码登录小窗**（重登只关小窗并刷新当前格，绝不再开一个主窗口），
**滚到底自动连续翻页**（历史、收藏夹内容、合集内容、搜索、空间的图文与合集）；
③ **落盘搬到 `%APPDATA%/ArrowMeo/rustBiliPC`，旧位置代码清零**；
④ **工作流拆成 `ci.yml`（push 自动）与 `cd.yml`（手动：一条作业 + 静态四腿矩阵同时编四平台，LTO/strip + UPX，`publish` 开关控制是否发 Release，只打 exe）**。
见第 4.27、4.28 节 |

## 大概的文件结构

```
rustBiliPC/
├── .github/workflows/   `ci.yml`（push 自动：win x64 release + 单测）+ `cd.yml`（手动：一条 build 作业 + 静态 include 四腿矩阵，release-dist + UPX，`publish` 开关控制是否发 GitHub Release）+ `../scripts/check_workflows.py`（推 workflow 改动前的本地校验）
├── src/                 项目源代码（单元测试在 src/tests/）
│   ├── main.rs lib.rs app.rs theme.rs logging.rs
│   ├── api/             bpi-rs 封装 + DASH 码流选择 + 播控取地址 + 下载 + 起播前地址探测
│   ├── store/           凭据与设置落盘（都在 `%APPDATA%/ArrowMeo/rustBiliPC`）
│   ├── player/          外置 mpv 参数与拉起（一个文件，不做遥控）
│   ├── pages/           主页 / 搜索 / 收藏 / 历史 / 待看 / 下载(任务列表) / 设置(四项能改) / 登录(独立小窗) / 详情页 / 空间页 / 合集内容页 / 重登协调页
│   └── ui/              色板与尺寸常量、复用布局件、视频卡片、封面 URL 规整、二维码
├── publish/             编译产物（不入库）
├── test/                单元测试布局说明（代码在 src/tests/）
├── ref/                 参考源码：gpui(0.3.9)/gpui-kit/Ely/bpi-rs/mpv/Bili.Copilot（不入库）
├── docs/                architecture.md、api-notes.md
├── res/                 资源文件（不入库）
├── Cargo.toml           依赖与 profile 配置
├── Cargo.lock           **入库**，与 gpui-pre 精确 pin 一起挡住快照漂移
├── build.bat            自动化编译脚本（默认 release）
├── QODER.md             Agent 协作规则
└── README.md
```

每个项目文件夹都有自己的 `README.md`；改哪个目录就更新哪个目录的文档，并提交 git。
深度内容看 [docs/architecture.md](docs/architecture.md)，接口与选项证据看
[docs/api-notes.md](docs/api-notes.md)。

## 编译与产物

- 仓库：https://github.com/ArrowMeo2088/rustBiliPC （公开）
- **`ci.yml`**：push / PR **自动跑**，只编 Windows x64 release + `cargo test`。
  这是"改动还能不能编过"的证据通道（本机没有 Rust 工具链，每一轮都要靠它）。
  两道闸（第五十二轮加的）：`paths-ignore` 让**全是文档/只动 `cd.yml` 的提交不触发编译**
  （`ci.yml` 自己除外），`concurrency` + `cancel-in-progress` 让同分支同事件只留最新一次。
- **`cd.yml`**：Actions → CD → **Run workflow**（手动）。输入：`publish`（**默认关：只编译取产物、
  不建 Release**）、`tag`、`prerelease`、`replace_tag`、`upx`。四个目标 win-x64 / win-arm64 /
  linux-x64 / linux-arm64 是**一条 `build` 作业 + 静态 `include` 四腿矩阵**，四条**同时开跑**
  （`fail-fast: false`，一个红不带走其余三个），用 `release-dist`（LTO + `codegen-units=1` + strip）编，
  再 UPX 压（版本钉死；压不动的目标出未压缩那份）。Linux 钉 `ubuntu-22.04` 卡 glibc 下限。
  要发 Release 就把 `publish` 打开并给 tag（发之前会先查同名标签指向哪次提交）。
  **资产只有可执行文件本身**（不带 dll、不带 mpv、不带文档）。
- **改 workflow 之前先跑** `python .github/scripts/check_workflows.py`：`yaml.safe_load` 查不出
  重复键、`needs` 悬空、未声明的 `inputs.*`，尤其查不出 **`${{ steps.<id>.* }}` 指到一个没写 `id:`
  的步骤**——那条求值为空串且不报错，能让一个作业在作业列表里**凭空消失**（第五十二轮连红三次的真因，
  全过程在架构文档第 8 节）。
- 日常产物：Artifacts `rustbilipc-win-x64-release` / 本机 `build.bat` 归集到 `publish/` 根。
- 缓存：`~/.cargo` 与 `target/` 按 profile 分 key；首次冷编译约 25–45 分钟属预期。
- 为什么是 release 而不是 debug：gpui 的 Windows 后端在 debug 下**运行时**从编译机路径读 HLSL 源，
  CI 产出的 debug exe 在别的机器上打不开窗口。详见架构文档第 2 节。
- 每轮交付口径：**N passed / 零 Rust warning / Cargo.lock 零差异**；UI 对不对只由他跑产物判定。

## 运行前提

- Windows 10/11 x64，带 D3D11 10.1 以上显卡驱动（无 WARP 回退）。
- 需要播放时：**`mpv.exe` 放在 PATH 里，或在设置页"MPV 播放器位置"里选给它**；
  两处都没有时才会退回"程序同目录的 `mpv.exe`"（老用法仍然有效）。
  程序**只找 `mpv.exe`、绝不接受 `mpv.com`**——后者是个会再派生一个 mpv.exe 的控制台启动器，
  我们 kill 不到真正在放的那个进程。缺 mpv 只是不能播，其余功能照常，设置页那一行会写明找到的路径。
- 用户数据都在**用户数据目录**：Windows `%APPDATA%\ArrowMeo\rustBiliPC`
  （Linux `~/.config/ArrowMeo/rustBiliPC`），里面三个文件：
  `auth.json`（明文，含 `SESSDATA`）、`config.json`（设置四项）、`debug.log`（只在"调试模式"开着时才有）。
  **旧位置（exe 同目录、`data/cookies.json`）的读取与迁移代码已经全部删除**，程序不会再去看它们。
  `config.json` 第一次启动会写出一份默认的，四项都是**真的有代码在读**的值
  （画质封顶、编码偏好、mpv 位置、调试模式），设置页改一项就落盘一项，没有"显示了但不生效"的开关。
  凭据是明文（用户知情选择），不要把带凭据的文件提交或分享出去。
- "调试模式"默认关：关着时不创建 `debug.log`、不做任何日志输出；双击启动也**不会**弹控制台窗口
  （exe 是 GUI 子系统）。要排查问题先在设置页把它打开，**然后重启一次程序**。
