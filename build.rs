//! 构建脚本：只做一件事——把 `res/icon.ico` 链进 Windows 可执行文件。
//!
//! 为什么用 `link.exe` 的 `/ICON:` 而不是 `winresource`/`embed-resource` 那类构建依赖：
//! 后者要 `rc.exe`（Windows SDK）或额外 crate，而 `/ICON:` 是链接器自带的选项，
//! **一个依赖都不加、产物体积只多那 144 KB**。
//!
//! 这颗图标会被 Explorer 与快捷方式直接用上；窗口与任务栏那颗走的是 gpui 的
//! `load_icon()`（`gpui_windows/src/platform.rs:1633`），它加载的是**资源 ID 1** 的
//! `IMAGE_ICON`——`/ICON:` 编出来的组图标就是那一个，所以两处一起生效。
//! 真机上如果发现窗口那颗还是默认图标，说明链接器给的编号不是 1，
//! 那时才改成写 `.rc` 显式命名 `1 ICON "res/icon.ico"`。
//!
//! 路径必须给绝对路径：链接器的工作目录按 cargo 调用的目录算，跨 target 目录时相对路径会找不到。

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=res/icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let icon = Path::new(&manifest_dir).join("res").join("icon.ico");
    if !icon.is_file() {
        // 不硬失败：少了图标只是难看，不该挡住一次编译。
        println!("cargo:warning=res/icon.ico 不在，产物将不带自定义图标");
        return;
    }
    println!("cargo:rustc-link-arg-bins=/ICON:{}", icon.display());
}
