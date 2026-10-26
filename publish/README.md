# publish — 编译产物

存放 GitHub Actions（或本机 `build.bat`）产出的可执行文件与依赖。

- 本目录内容**不提交 git**（`.gitignore` 排除，仅保留本 README）。
- **只有 release 产物才有意义**：gpui 的 Windows 后端在 debug 构建里于运行时从编译机路径读
  `.hlsl` 源文件，CI 产出的 debug exe 在别的机器上起不了窗口。详见 `docs/architecture.md` 第 2 节。
- Actions 侧同一份产物另存于 Artifacts：`rustbilipc-win-x64-release`，**zip 根目录直接就是 exe**，
  不再套一层 `release/`。
- `build.bat` 按同一约定归集到 `publish/rustbilipc.exe`（debug 只用于本机验证能否编译，不外发）。

## 预期内容

| 文件 | 说明 |
| --- | --- |
| `rustbilipc.exe` | 主程序 |
| `mpv.exe` | **要自己放的官方原版 mpv**（约 75 MB，不入库、不由 CI 产出）——起播只认这个位置 |
| `auth.json` | 首次扫码登录后由程序写入 **exe 同目录**（2026-10-26 起取代 `data/cookies.json`），不是编译产物；取产物时**只搬 exe**，这个文件绝不能被清理动作带走 |
| `config.json` | 第一次启动由程序写出一份默认设置（目前只有 `max_quality`），同样**不是编译产物**，不要删 |

运行时不需要额外 DLL：gpui 的 Windows 后端用系统 DLL（D3D11 / DirectComposition / DirectWrite），
mpv 是**外部独立进程**，不被链接进来。

**为什么 mpv 要放这一层而不是走 PATH**（第十八轮真机的教训）：程序与 `debug.log`、`data/`
都在这一层，把 mpv 也放这里，进程树是我们自己的一条。PATH 那条路会先命中
`mpv.com`（`PATHEXT` 把 `.COM` 排在 `.EXE` 前面），而控制台版 mpv 是个会**再派生一个
mpv.exe** 的启动器——我们 `kill` 掉的只是外层，真正的播放器留在那儿没人管。
懒人包（mpv-lazy / mpv_PlayKit）还额外带自己的 `portable_config` 与单实例启动器，更不可控。

缺 `mpv.exe` 时程序照常运行，只是详情页起播会写明"运行目录里没有 mpv.exe（应放在 …）"。

产物结构有变化时更新本 README 并提交。
