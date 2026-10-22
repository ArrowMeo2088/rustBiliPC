# docs — 文档

项目正式文档目录。中文、UTF-8。

## 现有文档

- [architecture.md](architecture.md)：硬约束、选型与版本锁、模块布局与分层规则、四个关键机制
  （异步桥 / 码流选择 / 外置 mpv / 凭据）、阶段进度表、风险清单、维护规则。
- [api-notes.md](api-notes.md)：bpi-rs 已核实的接口形状（登录/搜索/详情/playurl/fnval/画质档）、
  需要我们补齐的空白、mpv 选项证据、Bili.Copilot 的视觉度量。

## 约定

- `ref/` 不入库，所以任何从参考仓库读出的结论**必须抄进 docs**，否则换机器就丢失了。
- 改代码同步改文档：涉及机制的改动更新 architecture.md，涉及接口/选项的改动更新 api-notes.md，
  并更新所在目录的 README。
- 阶段状态只在 architecture.md 的阶段表里维护，不散落到别处。
