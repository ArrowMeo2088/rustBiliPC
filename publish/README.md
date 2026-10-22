# publish — 编译产物

存放 GitHub Actions（或本机 `build.bat`）产出的可执行文件与依赖 DLL。

- 目录内容 **不提交 git**（`.gitignore` 排除，仅保留本 README）。
- CI 产物另有一份在 Actions Artifacts（`rustbilipc-win-x64-debug`）中可直接下载。
- 约定子目录：`debug/`、`release/`，与 `build.bat` 的归集逻辑一致。

每次重新编译后如产物有结构性变化（新增 DLL、改名等），更新本 README 并提交。
