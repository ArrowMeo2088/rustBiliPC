//! 我们自己那一套 Lucide 图标（只放**程序里真的用到**的 + kit 组件自带的那一批）。

- `assets.rs` 把这里的每个 `.svg` 用 `include_bytes!` 编进 exe，`main.rs` 用 `with_assets(AppAssets)` 挂上。
- 为什么不用 kit 的 `AllAssets`：那是 1830 个文件、2.8 MB 全塞进二进制，而我们用到的连 20 个都不到
  （省下 ~2.6 MB；图标名找不到**不会报错也不会 panic**，只会画出一片空白，所以才把"默认那 106 个"一并带上）。
- 加图标的两步：① 从 `ref/gpui-kit/crates/assets/assets/icons/` 拷对应 kebab-case 文件进来；
  ② 在 `assets.rs` 的 `ICONS` 表里加一行。只做第 ① 步等于没加（表是唯一的清单）。
