# ref — 参考文件和代码

存放供参考的第三方代码与资料。**本目录不提交 git**（`.gitignore` 排除内容，仅保留本 README），
所以任何从这些仓库读出的结论都必须抄进 [docs/](../docs/README.md)，否则换机器即失效。

## 当前参考仓库

| 目录 | 是什么 | 怎么用 | 关键结论落在 |
| --- | --- | --- | --- |
| `gpui/` | **我们真正依赖的 gpui 0.3.9 源码**（`gpui` / `gpui_platform` / `gpui_windows` / `gpui_util` / `gpui_shared_string` / `gpui_tokio`） | 读渲染、IME、文本、图片加载的实现；核对某 API 是否存在于 0.3.9。来源与取舍见 [gpui/README.md](gpui/README.md) | architecture.md 第 2/4 节 |
| `gpui-kit/` | gpui-kit 0.8.0 全 workspace（**我们在用的框架**，依赖 `gpui-pre =0.3.9`） | 组件与主题 API 参照；**注意它的 `examples/` 是本地 main，比已发布版本新，照抄会踩不存在的 API** | architecture.md 第 2/3/4 节 |
| `Ely-GPUI-Components/` | Ely 组件库（同为 `gpui-pre =0.3.9`，`publish=false` 只能 git 依赖） | 只读参考：它的 `AGENTS.md` 有大量 gpui 实战坑（hover/focus/动画/Taffy 折叠），排查布局问题时优先查这里 | 不引入依赖 |
| `bpi-rs/` | Bilibili API 封装 0.3.1（**我们在用的接口层**） | 已核实签名与字段 | api-notes.md 第 1-6 节 |
| `mpv/` | mpv 源码 + `DOCS/man` | 只查选项与 IPC 文档 | api-notes.md 第 7 节 |
| `Bili.Copilot/` | C#/WinUI3 的 B 站客户端（**视觉与交互参考**） | 只取度量与页面结构，不取材质（gpui 无 Mica/Acrylic） | api-notes.md 第 8 节 |

## 已核实的版本事实（避免重新踩）

- crates.io：`gpui` 最新稳定 0.2.2（停更）、`gpui-pre` 0.3.9、`gpui-pre-platform` 0.3.9、
  `gpui-pre-windows` 0.3.9、`gpui-kit` 0.8.0、`bpi-rs` 0.3.1、`which` 8.0.6、`env_logger` 0.11.11、
  `named_pipe` 0.4.1、`qrcode` 0.14.1、`image` 0.25.10。
- `gpui_windows` / `gpui_platform` / `gpui_tokio` / `sum_tree` / `collections` **不在 crates.io 上**
  （gpui-pre 的重发布分别叫 `gpui-pre-windows` / `gpui-pre-platform` / …，`gpui-pre-tokio` 不存在，
  所以 tokio 桥要自己写）。`ref/gpui/gpui_tokio` 是从 Zed 仓库 `93d7456` 单文件取来的，
  不是发布版内容。
- `gpui-pre-0.3.9.crate` 包内**不含**平台实现（只有跨平台部分 + `examples/`），Windows 后端在
  `gpui-pre-windows-0.3.9` 里，两者要一起下载才看得全。

## 使用规则

- 这里的任何内容都不参与编译，仅作人工参考；不要 `path = "ref/..."` 引依赖（`ref/` 不入库，CI 上不存在）。
- 参考结论固化进实现后，写进 `docs/`，不要依赖本目录长期存在。
- 新增参考仓库时在上表登记一行。
