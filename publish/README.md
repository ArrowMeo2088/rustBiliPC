# publish — 编译产物

存放 GitHub Actions（或本机 `build.bat`）产出的可执行文件与依赖。

- 本目录内容**不提交 git**（`.gitignore` 排除，仅保留本 README）。
- **只有 release 产物才有意义**：gpui 的 Windows 后端在 debug 构建里于运行时从编译机路径读
  `.hlsl` 源文件，CI 产出的 debug exe 在别的机器上起不了窗口。详见 `docs/architecture.md` 第 2 节。
- Actions 侧同一份产物另存于 Artifacts：`rustbilipc-win-x64-release`。
- 子目录约定：`release/`（`build.bat` 归集目标）、`debug/`（仅本机临时调试用，不外发）。

## 预期内容

| 文件 | 说明 |
| --- | --- |
| `rustbilipc.exe` | 主程序 |
| `data/cookies.json` | 首次扫码登录后由程序写入（运行同目录），不是编译产物 |

运行时不需要额外 DLL：gpui 的 Windows 后端用系统 DLL（D3D11 / DirectComposition / DirectWrite），
mpv 是**外部独立进程**，不被链接进来，需要用户自行安装并保证在 PATH 中（程序启动自检会显示查找结果）。

产物结构有变化时更新本 README 并提交。
