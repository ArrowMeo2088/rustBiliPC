//! 登录凭据落盘：**明文 JSON**，写在 `%APPDATA%/ArrowMeo/rustBiliPC/auth.json`。
//!
//! ## 位置的两次改动（都不留兼容代码）
//!
//! - 2026-10-26 上午：从 `<exe>/data/cookies.json` 搬到 `<exe>/auth.json`。
//! - 2026-10-26 下午（P13）：用户原话"config.json/auth.json/debug.json 改为存储到
//!   `%APPDATA%/ArrowMeo/rustBiliPC` 里，**不要保留任何支持旧位置的 json 文件的代码**"。
//!   所以 `legacy_path()` 与那套一次性迁移逻辑**整块删掉**了：本模块现在只认这一个路径，
//!   读不到就是"未登录"，不会去老位置找。
//!
//! 换目录顺带解决了一件交付上的麻烦：以前 `publish/` 里既有 exe 也有登录态，
//! 取产物时"绝不能清空目录"是一条要靠人记住的红线；现在用户数据在 `%APPDATA%` 下，
//! 构建产物怎么覆盖都碰不到它。
//!
//! ## 明文带来的后果（已知且接受）
//!
//! `sessdata` 等价于登录态令牌，任何能读这个目录的人/进程都能直接复用账号。
//! 定稿时选择了"最快跑通"而不是 DPAPI 加密；如果以后要加密，只需要换掉本模块的
//! `save`/`load` 实现，字段与调用方都不用动。公开分发时 README 要提示这一点。
//!
//! ## 为什么要自己存
//!
//! `bpi_rs::session::Account` 只 derive 了 `Deserialize`（没有 `Serialize`），
//! bpi-rs 也不提供任何落盘/刷新逻辑（`session/cookie_refresh.rs` 是空文件）。
//! 因此这里用一个可序列化的镜像结构保存，读取时再构造回 `Account` 交给
//! `BpiClient::set_account`。
//!
//! ## 字段为什么是这四个
//!
//! `Account::validate_complete` 要求 DedeUserID / SESSDATA / bili_jct / buvid3
//! 全部非空，缺 `buvid3` 会直接被判定为不完整——扫码成功响应里能拿到 cookie，
//! 但 `buvid3` 要靠一次导航/风控接口补齐，见 [`crate::api::login`]。

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use bpi_rs::Account;
use serde::{Deserialize, Serialize};

/// 落盘结构。字段名即 JSON 键名，改动要考虑旧文件兼容（新增字段一律 `#[serde(default)]`）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Auth {
    /// 用户 mid（`DedeUserID`）。
    pub dede_user_id: String,
    /// 登录态令牌。
    pub sessdata: String,
    /// CSRF 令牌（写操作必需）。
    pub bili_jct: String,
    /// 设备标识，风控依赖它。
    pub buvid3: String,
    /// 扫码登录返回的 refresh_token，供后续续期使用。
    #[serde(default)]
    pub refresh_token: String,
    /// 写入时间（秒级 Unix 时间戳），用于判断凭据新旧。
    #[serde(default)]
    pub saved_at_unix: i64,
}

impl Auth {
    /// 凭据文件路径：`<设置目录>/auth.json`（目录见 [`crate::app::settings_dir`]）。
    pub fn path() -> PathBuf {
        crate::app::settings_dir().join("auth.json")
    }

    /// 从 JSON 文本解析。坏文本返回 `None`，调用方按"未登录"处理。
    ///
    /// 单独做成函数是为了让这段逻辑能进单测（[`from_json`] 不碰文件系统）。
    pub fn from_json(raw: &str) -> Option<Self> {
        match serde_json::from_str::<Auth>(raw) {
            Ok(auth) => Some(auth),
            Err(error) => {
                log::warn!("凭据文件无法解析，按未登录处理: {error}");
                None
            }
        }
    }

    /// 拼成给 mpv 用的 `Cookie` 头值（不含 `Cookie: ` 前缀）。
    ///
    /// B 站的 m4s 直链会校验这个：1080P/高码率没有登录 cookie 就是 403/-101，
    /// 而 bpi-rs 不把它的 cookie jar 交出来（`jar` 是私有的），所以由我们自己拼。
    /// 顺序与 B 站网页一致，纯为可读性；字段缺任何一个都少一条（不编造空值）。
    pub fn cookie_header(&self) -> String {
        let mut parts: Vec<String> = Vec::with_capacity(4);
        if !self.dede_user_id.is_empty() {
            parts.push(format!("DedeUserID={}", self.dede_user_id));
        }
        if !self.sessdata.is_empty() {
            parts.push(format!("SESSDATA={}", self.sessdata));
        }
        if !self.bili_jct.is_empty() {
            parts.push(format!("bili_jct={}", self.bili_jct));
        }
        if !self.buvid3.is_empty() {
            parts.push(format!("buvid3={}", self.buvid3));
        }
        parts.join("; ")
    }

    /// 是否够构造一个完整的 `Account`。
    pub fn is_complete(&self) -> bool {
        !self.dede_user_id.is_empty()
            && !self.sessdata.is_empty()
            && !self.bili_jct.is_empty()
            && !self.buvid3.is_empty()
    }

    /// 构造回 bpi-rs 的账号对象（不校验，调用方自行 `is_complete` 判断）。
    pub fn to_account(&self) -> Account {
        Account {
            dede_user_id: self.dede_user_id.clone(),
            sessdata: self.sessdata.clone(),
            bili_jct: self.bili_jct.clone(),
            buvid3: self.buvid3.clone(),
        }
    }

    /// 覆盖写入 `auth.json`（目录不存在会先建——`%APPDATA%/ArrowMeo/rustBiliPC` 首次运行还没有）。
    /// 失败时把路径带进错误信息方便排查权限问题。
    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("创建凭据目录失败: {}", dir.display()))?;
        }
        let json = serde_json::to_string_pretty(self).context("序列化凭据失败")?;
        std::fs::write(&path, json).with_context(|| format!("写入凭据失败: {}", path.display()))?;
        Ok(path)
    }

    /// 读取已保存的凭据。读不到/坏了一次性按"未登录"处理——**不再去看任何旧位置**。
    pub fn load() -> Option<Self> {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|raw| Self::from_json(&raw))
    }

    /// 删除已保存的凭据（退出登录用）。文件不存在视为成功。
    pub fn clear() -> Result<()> {
        let path = Self::path();
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).with_context(|| format!("删除凭据失败: {}", path.display())),
        }
    }

    /// 当前秒级时间戳，写入 `saved_at_unix` 用。
    pub fn now_unix() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or_default()
    }
}
