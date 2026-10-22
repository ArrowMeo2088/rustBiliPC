# src — 项目源代码

Rust crate 根目录。

## 当前结构

- `main.rs`：程序入口，只负责启动，后续在此挂载 UI 与 API 层。
- `lib.rs`：库 crate `rustbilipc` 根，可测试逻辑收敛在这里。
- `tests/`：单元测试模块（项目约定单元测试放 `src/tests/`，见根目录 `test/README.md`）。

## 后续规划（占位，落地时更新本文件）

- `api/`：Bilibili Web/TV 接口封装与签名（wbi 等）。
- `models/`：请求/响应数据结构。
- `ui/`：桌面界面（技术选型待定，确定后更新根 README）。

## 约定

修改本目录任何代码时，必须同步更新本 README 与相关文档，并提交 git。
