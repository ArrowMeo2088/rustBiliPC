# src — 项目源代码

Rust crate 根目录（crate 名 `rustbilipc`）。职责划分与维护契约见
[docs/architecture.md](../docs/architecture.md)。

## 当前结构

| 路径 | 职责 | 备注 |
| --- | --- | --- |
| `main.rs` | 启动序列 | logger → tokio runtime → `with_assets` → `gpui_kit::init` → 主题 → 开窗，顺序有意义 |
| `lib.rs` | 模块树 + `VERSION` | 逻辑都收敛在 lib crate 里便于 `cargo test` |
| `app.rs` | 进程级基础设施 | 全局 tokio runtime（`OnceLock`）、`exe_dir()`/`data_dir()` |
| `theme.rs` | 全局主题覆盖 | accent `#FB7299`，强调色前景改深色（对比度不足 4.5:1） |
| `api/` | bpi-rs 封装 + DASH 选择 | `spawn_on_api_runtime` 是唯一的 gpui↔tokio 桥；`stream_select` 是纯函数 |
| `store/` | 落盘 | `credentials.rs` 明文 JSON，见文件头的风险说明 |
| `player/` | 外置 mpv | `build_arguments` 纯函数 + Referer/UA 常量；`ipc.rs` 只做命令构造，管道 I/O 在 P4 |
| `pages/` | 按阶段划分的页面 | 目前只有 `search/`（骨架：输入框 + 假数据列表 + 环境自检） |
| `ui/` | 跨页面复用组件 | `video_card.rs`（含骨架期假数据） |
| `tests/` | 单元测试 | 只测纯逻辑：码流边界、mpv 参数形状、凭据往返 |

## 分层规则

- `pages/*` 不直接 `use bpi_rs`，网络调用一律经 `api/`。
- `api/`、`store/`、`player/` 不得依赖 gpui 类型，必须能在无窗口环境下测试。
- gpui 类型统一从 `gpui_kit::*` 取，不单独依赖 gpui（kit 的 umbrella 已把它放在根上）。

## 约定

修改本目录任何代码时，同步更新本 README、`docs/architecture.md` 相应小节，并提交 git。
注释写"为什么"（机制、坑、定稿理由），不写"是什么"。
