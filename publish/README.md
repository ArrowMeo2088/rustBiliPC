# publish — 编译产物

存放 GitHub Actions（或本机 `build.bat`）产出的可执行文件与依赖。

- 本目录内容**不提交 git**（`.gitignore` 排除，仅保留本 README）。
- **只有 release 产物才有意义**：gpui 的 Windows 后端在 debug 构建里于运行时从编译机路径读
  `.hlsl` 源文件，CI 产出的 debug exe 在别的机器上起不了窗口。详见 `docs/architecture.md` 第 2 节。
- Actions 侧的产物：`ci.yml` 每轮自动上传 `rustbilipc-win-x64-release`（**zip 根目录直接就是 exe**，
  不套 `release/`、不带 dll）；`cd.yml` 手动发的是 GitHub Release 资产（多平台、`release-dist` + UPX，
  **同样只有可执行文件**）。
- `build.bat` 按同一约定归集到 `publish/rustbilipc.exe`（debug 只用于本机验证能否编译，不外发）。

## 预期内容

| 文件 | 说明 |
| --- | --- |
| `rustbilipc.exe` | 主程序，取产物时**只搬这一个文件** |
| `mpv.exe` | 要自己放的官方原版 mpv（约 75 MB，不入库、不由 CI 产出）。**P13 起它在这一层只是兜底**：查找顺序是"设置里指定的文件 → PATH → 本目录"，所以放 PATH 里也行、设置页里指给程序也行 |
| `debug.log`、`auth.json`、`config.json` | **P13 起这三个都不再写在这里**，改到 `%APPDATA%\ArrowMeo\rustBiliPC`。本目录里那些是**上一版留下的现场**：程序不会再读也不会再写，我没删（那是你某一次登录的现场，删除不可逆）。要清就你自己决定 |

运行时不需要额外 DLL：gpui 的 Windows 后端用系统 DLL（D3D11 / DirectComposition / DirectWrite），
mpv 是**外部独立进程**，不被链接进来。

**为什么仍然强调"只找 `mpv.exe`、绝不接受 `mpv.com`"**（第十八轮真机的教训，P13 查 PATH 之后这条照旧）：
PATH 那条路会先命中 `mpv.com`（`PATHEXT` 把 `.COM` 排在 `.EXE` 前面），而控制台版 mpv 是个会
**再派生一个 mpv.exe** 的启动器——我们 `kill` 掉的只是外层，真正的播放器留在那儿没人管。
所以程序自己拼 `目录\mpv.exe` 再判存在，不经过 shell 的 `where` 语义；懒人包
（mpv-lazy / mpv_PlayKit）还带自己的 `portable_config` 与单实例启动器，更不可控。

找不到任何 mpv 时程序照常运行，只是起播会写一句"找不到 mpv：设置里没指定、PATH 里没有、
运行目录也没有（可在『设置 → MPV 播放器位置』里选，或放在本目录）"。

**跑这版要注意的三件事**（都是 2026-10-26 P13 定稿的直接后果，不是回归）：
1. 登录态不会跟着搬过来 —— 旧的 `publish\auth.json` 不再被读，**第一次启动会弹扫码小窗，扫一次**即可。
2. 默认**没有** `debug.log`（"调试模式"默认关）。要日志：设置页打开它 → **退出重开一次**。
3. exe 是 GUI 子系统，**双击不再弹黑色控制台窗口**（调试模式开着也不弹）。

产物结构或落盘位置有变化时更新本 README 并提交。
