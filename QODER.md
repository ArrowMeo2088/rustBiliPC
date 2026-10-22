# QODER.md — Agent 协作规则

在本仓库工作的 Agent 必须遵守以下规则。

## 1. 文件结构规则

- 目录用途固定：`src/`=项目源代码，`publish/`=编译产物，`test/`=单元测试（约定说明，单元测试代码在 `src/tests/`，集成测试按 cargo 惯例在 `tests/`），`ref/`=供参考的文件和代码，`docs/`=文档，`res/`=资源文件。
- **每个项目文件夹都必须有自己的 `README.md`** 描述该目录用途与内容。
- **每次修改某个目录的内容，必须同步更新对应 README/文档，并提交 git**（一次修改一个可复原的小提交，提交信息说明改了什么）。
- `.gitignore` 按项目规则排除 `ref/`、`publish/`、`res/` 的**内容**（`/ref/*` 这种写法，各目录的 README.md 通过 `!` 例外保留入库）；不要随意把这些目录的其他内容加入暂存区（也不要滥用 `git add -f`）。

## 2. 工具链约束

- **本机没有安装 Rust 工具链，也不要安装**（不装 rustup/cargo，不跑本地 cargo 命令做验证）。
- 所有编译只通过 GitHub Actions 的 `Build` workflow 完成（手动触发，workflow_dispatch；现阶段仅 Windows x64 Debug）。
- 验证 Rust 改动是否正确：推送后查看 Actions 运行日志，以 CI 结果为准；`build.bat` 只是产物归集/本地可选脚本，不代表本机可执行。

## 3. GitHub 访问代理

- **访问 GitHub 一律走 HTTP 代理 `http://127.0.0.1:7887`**：
  - git：`git -c http.proxy=http://127.0.0.1:7887 -c https.proxy=http://127.0.0.1:7887 push ...`（或仓库级 `git config http.proxy`）。
  - gh CLI / curl：设置环境变量 `HTTPS_PROXY=http://127.0.0.1:7887 HTTP_PROXY=http://127.0.0.1:7887`。
- 直连与代理都可能偶发失败；一种方式报错时切换另一种重试，不要长时间前台轮询等待。

## 4. 其他约定

- 输出与文档中的中文必须是可读的 UTF-8，禁止 `\uXXXX` 转义或 GBK 乱码；`.bat` 文件保持 ASCII 内容（cmd 按 OEM 代码页解析）。
- 不要在前台长时间轮询 CI；用后台轮询或短探测，以日志证据为准。
