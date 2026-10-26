//! 持久化层：登录凭据（[`auth`]）与设置（[`config`]）。
//!
//! 两者的摆放位置是 2026-10-26 定稿的：**都明文平铺在可执行文件同目录**
//! （`auth.json` / `config.json`），不再放进 `data/`。理由是运行目录一眼看得全
//! ——`rustbilipc.exe`、`mpv.exe`、`debug.log`、`auth.json`、`config.json`，
//! 出问题时不用钻子目录。旧的 `data/cookies.json` 只作一次性迁移的来源，迁移后保留不删
//! （见 [`auth`] 文件头）。

pub mod auth;
pub mod config;
