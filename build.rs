//! 构建脚本：只做一件事——把 `res/icon.ico` 作为 Windows 资源链进可执行文件。
//!
//! ## 为什么必须先过 `rc.exe`
//!
//! **MSVC 的 `link.exe` 没有 `/ICON:` 这个选项**。第五十五轮先试过直接
//! `cargo:rustc-link-arg-bins=/ICON:…/res/icon.ico`，CI 里的现场是
//! `LINK : warning LNK4044: unrecognized option '/ICON:…'; ignored`——
//! 选项被丢掉、产物里**连 `.rsrc` 段都没有**（用 `.github/scripts/check_icon.py` 扫出来的），
//! 而编译照样绿。图标是资源，只能先由 SDK 的 `rc.exe` 把 `.rc` 编成 `.res`，再让链接器吃进去。
//!
//! ## 为什么自己调 `rc.exe` 而不加 `winresource`/`embed-resource`
//!
//! `rc.exe` 与 `fxc.exe` 在同一个 SDK 目录树里，而 **gpui 的 `build.rs` 本来就要求 SDK**
//! （release 下要编 HLSL），所以这条路**一个新依赖都不加**。那两个 crate 也是去找同一个
//! `rc.exe`/`windres`，多一个依赖只是把这段搜索代码搬到别人的 crate 里。
//!
//! ## 找不到 `rc.exe` 怎么办
//!
//! 只发 `cargo:warning` 然后什么都不做——少个图标不该挡住一次编译。
//! 为了让这事不静默，`ci.yml` 里有一步 `.github/scripts/check_icon.py` 会**明确红**，
//! `cd.yml` 那份只在 summary 里写一行（不为图标卡住发布）。
//!
//! ## 窗口与任务栏那颗
//!
//! gpui 在 `gpui_windows/src/platform.rs:1633` 的 `load_icon()` 里
//! `LoadImageW(hModule, PCWSTR(1), IMAGE_ICON, …)`——**资源 ID 1**。
//! 所以下面 `.rc` 里那句就写死成 `1 ICON …`，让窗口/任务栏那颗与 exe 图标是同一份。

use std::path::{Path, PathBuf};
use std::process::Command;

fn find_rc_exe() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("WindowsSdkDir") {
        let root = Path::new(&dir).join("bin");
        if let Some(p) = pick_under(&root) {
            return Some(p);
        }
    }
    let program_files = std::env::var("ProgramFiles(x86)")
        .or_else(|_| std::env::var("ProgramFiles"))
        .unwrap_or_else(|_| r"C:\Program Files (x86)".to_string());
    Some(pick_under(&Path::new(&program_files).join(r"Windows Kits\10\bin"))?)
}

/// 在 `Windows Kits\10\bin\<版本>\x64\rc.exe` 这一堆里挑一个（版本号降序，取最新）。
fn pick_under(root: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    let versions = std::fs::read_dir(root).ok()?;
    for version in versions.flatten() {
        for arch in ["x64", ""] {
            let candidate = version.path().join(arch).join("rc.exe");
            if candidate.is_file() {
                found.push(candidate);
            }
        }
    }
    found.sort();
    found.reverse();
    found.into_iter().next()
}

fn main() {
    println!("cargo:rerun-if-changed=res/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let icon = manifest_dir.join("res").join("icon.ico");
    if !icon.is_file() {
        println!("cargo:warning=res/icon.ico 不在，产物将不带自定义图标");
        return;
    }
    let Some(rc) = find_rc_exe() else {
        println!("cargo:warning=没找到 Windows SDK 的 rc.exe，产物不带自定义图标（gpui 会用默认图标）");
        return;
    };

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());
    let rc_file = out_dir.join("rustbilipc.rc");
    let res_file = out_dir.join("rustbilipc.res");
    // `ICON` 那行的资源名写 1：gpui 的 load_icon() 找的就是 ID 1（见文件头）。
    let text = format!("1 ICON \"{}\"\n", icon.display().to_string().replace('\', "\\\\"));
    if std::fs::write(&rc_file, &text).is_err() {
        println!("cargo:warning=写 .rc 失败，产物不带自定义图标");
        return;
    }
    let status = Command::new(&rc)
        .args(["/nologo", "/fo"])
        .arg(&res_file)
        .arg(&rc_file)
        .status();
    match status {
        Ok(status) if status.success() => {
            // 位置参数交给 link.exe：它就是按输入文件处理 `.res` 的。
            println!("cargo:rustc-link-arg={}", res_file.display());
        }
        Ok(_) => println!("cargo:warning=rc.exe 编译 .rc 失败，产物不带自定义图标"),
        Err(e) => println!("cargo:warning=调用 rc.exe 失败（{e}），产物不带自定义图标"),
    }
}
