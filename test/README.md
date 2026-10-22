# test — 单元测试说明

本项目 Rust 代码的测试布局约定：

- **单元测试**：放在 `src/tests/`（作为 crate 的模块编译，可访问私有项），入口见 `src/lib.rs` 中的 `pub mod tests;`。
- **集成测试**：按 cargo 惯例放仓库根的 `tests/` 目录（需要时再创建），只依赖公开 API。
- 运行方式：`cargo test`，或由 `build.bat` 在编译后自动执行。

本目录本身不存放 `.rs` 代码（cargo 不识别 `test/`），仅承载上述约定说明与将来的测试脚本。

修改测试布局时必须同步更新本 README 并提交 git。
