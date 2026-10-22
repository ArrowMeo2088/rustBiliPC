//! 登录凭据落盘：**明文 JSON**，写在可执行文件同目录的 `data/cookies.json`。
//!
//! ## 这个决定带来的后果（已知且接受）
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
//! 但 `buvid3` 要靠一次导航/风控接口补齐，P2 落地时注意。

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use bpi_rs::Account;
use serde::{Deserialize, Serialize};

/// 落盘结构。字段名即 JSON 键名，改动要考虑旧文件兼容（新增字段一律 `#[serde(default)]`）。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SavedCredential {
    /// 用户 mid。
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

impl SavedCredential {
    /// 凭据文件路径：`<exe 所在目录>/data/cookies.json`。
    pub fn path() -> PathBuf {
        crate::app::data_dir().join("cookies.json")
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

    /// 覆盖写入。先建 `data/` 目录，失败时把路径带进错误信息方便排查权限问题。
    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建数据目录失败: {}", parent.display()))?;
        }

        let json = serde_json::to_string_pretty(self).context("序列化凭据失败")?;
        std::fs::write(&path, json).with_context(|| format!("写入凭据失败: {}", path.display()))?;
        Ok(path)
    }

    /// 读取已保存的凭据。
    ///
    /// 文件不存在或解析失败都返回 `None`（当作未登录），因为这两种情况下的处理
    /// 路径是一样的：走扫码登录重新拿一份。
    pub fn load() -> Option<Self> {
        let raw = std::fs::read_to_string(Self::path()).ok()?;
        match serde_json::from_str::<Self>(&raw) {
            Ok(credential) => Some(credential),
            Err(error) => {
                log::warn!("凭据文件无法解析，按未登录处理: {error}");
                None
            }
        }
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
