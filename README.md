# rustBiliPC

用 Rust 编写的 Bilibili 桌面客户端（Windows x64）。**完全不使用浏览器技术**：
没有 Electron / WebView / JS / TS，界面是 gpui（Zed 的 GPU 加速 UI 框架）经
Direct3D11 + DirectComposition + DirectWrite 原生绘制；视频播放交给外置 mpv 进程。

## 解决方案简介

| 方面 | 选择 |
| --- | --- |
| 界面 | `gpui-kit 0.8.0`（umbrella，内含 gpui-component 0.8.0 组件层），实际 gpui 快照 `gpui-pre =0.3.9` |
| 接口 | `bpi-rs 0.3.1`，关闭默认 `full`，只开 login/search/video/user/fav/historytoview |
| 登录 | 扫码登录为主，凭据明文存 `<exe 同目录>/data/cookies.json`，下次启动自动复用 |
| 播放 | 调用外部 mpv（默认在 PATH 中查找），独立窗口，通过命名管道 IPC 遥控 |
| 画质 | HEVC 优先 + 默认封顶 1080P（`id <= 116`），无 1080P 时自动取最高可用档 |
| 编译 | 本机**不装** Rust 工具链，全部由 GitHub Actions 手动触发编译，仅 Windows x64 **release** |
| 阶段 | 登录 → 搜索 → 个人 → 详情 → 播放（当前 P2a：扫码登录代码完成，待真机验证） |

## 大概的文件结构

```
rustBiliPC/
├── .github/workflows/   手动触发的 Windows x64 release 编译（cargo/target 缓存）
├── src/                 项目源代码（单元测试在 src/tests/）
│   ├── main.rs lib.rs app.rs theme.rs
│   ├── api/             bpi-rs 封装 + DASH 码流选择
│   ├── store/           凭据落盘
│   ├── player/          外置 mpv 参数与 IPC 命令
│   ├── pages/search/    阶段一页面
│   └── ui/              复用组件（视频卡片）
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
- 产物：Artifacts `rustbilipc-win-x64-release`；本机跑 `build.bat` 会归集到 `publish/release/`
- 缓存：`~/.cargo` 与 `target/` 按 profile 分 key；首次冷编译约 25–45 分钟属预期
- 为什么是 release 而不是 debug：gpui 的 Windows 后端在 debug 下**运行时**从编译机路径读 HLSL 源，
  CI 产出的 debug exe 在别的机器上打不开窗口。详见架构文档第 2 节。
- 现阶段不做 clippy / rustfmt 门禁，不追求零 warning，优先把功能跑出来。

## 运行前提

- Windows 10/11 x64，带 D3D11 10.1 以上显卡驱动（无 WARP 回退）。
- 需要播放器时：安装 mpv 并保证 `mpv` 在 PATH 中；程序底部状态条会显示是否找到。
- 首次登录后会在 exe 同目录创建 `data/cookies.json`（明文，含 `SESSDATA`）——
  不要把带凭据的目录提交或分享出去。
