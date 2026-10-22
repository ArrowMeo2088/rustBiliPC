# rustBiliPC

基于 Rust 编写的 Bilibili 桌面（Windows x64）客户端，目前处于脚手架阶段。

## 解决方案简介

- **语言 / 生态**：Rust（edition 2021），入口为 `src/main.rs`，逻辑收敛在 lib crate `rustbilipc` 中便于测试。
- **编译方式**：本机 **不安装** Rust 工具链，全部编译通过 GitHub Actions 完成。
  - Workflow：`.github/workflows/build.yml`，仅 **手动触发**（workflow_dispatch）。
  - 现阶段只做 **Windows x64 Debug** 编译，并使用 cargo / target 缓存加速。
  - 编译产物同时上传到 Actions Artifacts，并由 `build.bat` 归集到 `publish/`。
- **文档约定**：每个项目文件夹都有自己的 `README.md` 描述用途；**任何修改必须同步更新对应文档并提交 git**。

## 大概的文件结构

```
rustBiliPC/
├── .github/workflows/   GitHub Actions（手动触发 Windows x64 Debug 编译，含缓存）
├── src/                 项目源代码（Rust crate 根，单元测试位于 src/tests/）
├── publish/             编译产物（被 git 忽略，不上传）
├── test/                单元测试说明目录（集成测试按 Rust 惯例放 tests/）
├── ref/                 供参考的文件和代码（被 git 忽略）
├── docs/                文档
├── res/                 资源文件（被 git 忽略）
├── Cargo.toml           Rust 包清单
├── build.bat            本地/CI 通用的自动化编译脚本（cargo build + 产物归集到 publish/）
├── .gitignore           排除 ref、publish、res 及 target 等
└── QODER.md             Agent 协作规则
```

## 快速上手

1. 在 GitHub 上手动触发 "Build" workflow（Run workflow）。
2. 从 Actions Artifacts 下载 `rustbilipc-win-x64-debug`。
3. 若本机装有工具链，也可直接运行 `build.bat`，产物会自动复制到 `publish/`。

## 仓库信息

- 远程仓库：`https://github.com/<你的账号>/rustBiliPC`（公开）
- 访问 GitHub 需走本机代理 `http://127.0.0.1:7887`（详见 [QODER.md](QODER.md)）。
