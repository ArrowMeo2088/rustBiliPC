# rustBiliPC

用 Rust 编写的 Bilibili 桌面客户端（Windows x64）。**完全不使用浏览器技术**：
没有 Electron / WebView / JS / TS，界面是 gpui（Zed 的 GPU 加速 UI 框架）经
Direct3D11 + DirectComposition + DirectWrite 原生绘制；视频播放交给外置 mpv 进程。

## 解决方案简介

| 方面 | 选择 |
| --- | --- |
| 界面 | `gpui-kit 0.8.0`（umbrella，内含 gpui-component 0.8.0 组件层），实际 gpui 快照 `gpui-pre =0.3.9` |
| 接口 | `bpi-rs 0.3.1`，关闭默认 `full`，只开 login/search/video/user/fav/historytoview/misc/comment |
| 登录 | 扫码登录为主，凭据明文存 `<exe 同目录>/data/cookies.json`，下次启动自动复用 |
| 播放 | 把 DASH 直链 URL 交给外部 mpv **在线流式播**（不下整片），独立窗口。**播放期的一切（进度条/时间/暂停/音量/全屏/字幕显示）都归 mpv 自带的 OSC，本项目不做**；我们只做播放之外（见下一行） |
| 范围 | rustBiliPC 负责：主页/热门、搜索、个人中心、视频详情、评论区、分集分P 的呈现、清晰度选择、字幕列表与投递、拉起并观测 mpv。不负责：播放器界面本身 |
| 画质 | HEVC 优先 + 默认封顶 1080P（`id <= 116`），无 1080P 时自动取最高可用档 |
| 编译 | 本机**不装** Rust 工具链，全部由 GitHub Actions 手动触发编译，仅 Windows x64 **release** |
| 阶段 | 登录 → 搜索 → 个人 → 详情 → 起播 均已真机通过；P5 视觉照 `ref/Bili.Copilot` 重做并已把全局主题切成暗色（kit 默认亮色是"搜索框白底黑字"的根因）。当前 **P6a 主页**：首页推荐流作为默认页 + "换一批"；接下来 P6b 评论区、P6c 字幕投递与同实例换清晰度 |

## 大概的文件结构

```
rustBiliPC/
├── .github/workflows/   手动触发的 Windows x64 release 编译（cargo/target 缓存）
├── src/                 项目源代码（单元测试在 src/tests/）
│   ├── main.rs lib.rs app.rs theme.rs
│   ├── api/             bpi-rs 封装 + DASH 码流选择 + 播控取地址 + 首页推荐
│   ├── store/           凭据落盘
│   ├── player/          外置 mpv 参数与拉起（一个文件，不做遥控）
│   ├── pages/           主页 / 登录 / 搜索 / 个人中心 / 详情页（外壳持有详情页栈）
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
- 首次登录后会在 exe 同目录创建 `data/cookies.json`（明文，含 `SESSDATA`）——
  不要把带凭据的目录提交或分享出去。
