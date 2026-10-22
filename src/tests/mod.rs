//! 单元测试模块（项目约定：单元测试统一放在 `src/tests/`，目录说明见仓库根 `test/README.md`）。

#[cfg(test)]
mod scaffold_tests {
    use crate::VERSION;

    #[test]
    fn version_matches_manifest() {
        assert!(!VERSION.is_empty());
        assert!(VERSION.starts_with(|c: char| c.is_ascii_digit()));
    }

    #[test]
    #[ignore = "Bilibili API 尚未实现，接入后启用"]
    fn placeholder_api_check() {
        unimplemented!();
    }
}
