# rustBiliPC

用 Rust 编写的 Bilibili 桌面客户端（Windows x64）。**完全不使用浏览器技术**：
没有 Electron / WebView / JS / TS，界面是 gpui（Zed 的 GPU 加速 UI 框架）经
Direct3D11 + DirectComposition + DirectWrite 原生绘制；视频播放交给外置 mpv 进程。

## 解决方案简介

| 方面 | 选择 |
| --- | --- |
| 界面 | `gpui-kit 0.8.0`（umbrella，内含 gpui-component 0.8.0 组件层），实际 gpui 快照 `gpui-pre =0.3.9` |
| 接口 | `bpi-rs 0.3.1`，关闭默认 `full`，只开 login/search/video/user/fav/historytoview/misc/comment |
| 登录 | 扫码登录为主：启动时若 `<exe 同目录>/auth.json` 没有可用凭据，**先弹一个 320×420 的小登录窗**，扫完码才开主窗口（2026-10-26 定稿）。凭据与设置都是明文 JSON 平铺在运行目录（`auth.json` / `config.json`，不再写 `data/`） |
| 播放 | 把 DASH 直链 URL 交给外部 mpv **在线流式播**（不下整片），独立窗口。**播放期的一切（进度条/时间/暂停/音量/全屏/字幕显示）都归 mpv 自带的 OSC，本项目不做**；我们只做播放之外（见下一行） |
| 范围 | rustBiliPC 负责：主页（首页推荐）、搜索、收藏/历史/待看（各一个侧栏入口，能搜）、视频详情、评论区（**文字可选中可复制**）、分集分P 的呈现、清晰度档位（按设置封顶）、**用户空间（投稿/图文/收藏）**、**下载（双文件，落盘位置每次弹框选）**、拉起 mpv。不负责：播放器界面本身；**字幕投递与同实例换清晰度也不做**（用户 2026-10-25 判定"两个都没必要"） |
| 画质 | HEVC 优先 + 默认封顶 1080P（`id <= 116`），无 1080P 时自动取最高可用档 |
| 编译 | 本机**不装** Rust 工具链，全部由 GitHub Actions 手动触发编译，仅 Windows x64 **release** |
| 阶段 | P0–P10 全部编译交付。**真机进度**：P8 与 P9 已在他机器上跑过（`debug.log` 里收藏下钻、收藏夹内容 19 条、历史 30 条、待看 6 条、详情字幕/评论全通，零错误），登录独立小窗与 P10 的拖动滚动也已真机确认。**P11**（第四十轮编译中）= 详情页与评论区文字**可选中 + 复制**（gpui 没有选择机制，用 kit 的 `base::SelectableText`；触摸屏给一颗「复制选中」）+ **下载**（`.m4v`+`.m4a` 双文件、画质吃封顶、只下当前 P、每次弹框选目录、进度与取消、备用域名、打开所在目录/用 mpv 打开）——**这是唯一允许写到运行目录之外的功能**（用户 2026-10-26 定稿）。另修掉 roadmap R0 两条：清晰度按钮以前能点出 4K/HDR（现在按 `config.json` 封顶过滤），`src/README.md` 里那条早已不存在的 `--input-ipc-server` 说法。见第 4.24、4.25 节 |

## 大概的文件结构

```
rustBiliPC/
├── .github/workflows/   手动触发的 Windows x64 release 编译（cargo/target 缓存）
├── src/                 项目源代码（单元测试在 src/tests/）
│   ├── main.rs lib.rs app.rs theme.rs
│   ├── api/             bpi-rs 封装 + DASH 码流选择 + 播控取地址 + 首页推荐
│   ├── store/           凭据落盘
│   ├── player/          外置 mpv 参数与拉起（一个文件，不做遥控）
│   ├── pages/           主页 / 搜索 / 收藏 / 历史 / 待看 / 下载(任务列表) / 设置(关于) / 登录(独立小窗) / 详情页 / 空间页
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
- 触发：Actions → Build workflow → **Run workflow**（仅手动，`workflow_dispatch`）
- 产物：Artifacts `rustbilipc-win-x64-release`（exe 直接在压缩包根目录）；本机跑 `build.bat` 同样归集到 `publish/` 根
- 缓存：`~/.cargo` 与 `target/` 按 profile 分 key；首次冷编译约 25–45 分钟属预期
- 为什么是 release 而不是 debug：gpui 的 Windows 后端在 debug 下**运行时**从编译机路径读 HLSL 源，
  CI 产出的 debug exe 在别的机器上打不开窗口。详见架构文档第 2 节。
- 现阶段不做 clippy / rustfmt 门禁，不追求零 warning，优先把功能跑出来。

## 运行前提

- Windows 10/11 x64，带 D3D11 10.1 以上显卡驱动（无 WARP 回退）。
- 需要播放时：把**官方原版 `mpv.exe` 放到程序同目录**（即 `publish/mpv.exe`）。程序只认这个位置，
  不查 PATH（PATH 上会先命中 `mpv.com` 那个"再派生一个 mpv.exe"的启动器，我们的 kill 管不到真正播放的进程）。
  缺它只是不能播，其余功能照常；搜索页底部状态行会显示它找的是哪个路径。
- 首次登录后会在 exe 同目录创建 `auth.json`（明文，含 `SESSDATA`）；`config.json` 第一次启动也会写出一份默认的（只放真的有代码在读的 `max_quality`）。旧的 `data/cookies.json` 会被读一次做迁移，**迁移后保留不删**——
  不要把带凭据的目录提交或分享出去。
