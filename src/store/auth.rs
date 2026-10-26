//! 登录凭据落盘：**明文 JSON**，写在可执行文件同目录的 `auth.json`。
//!
//! ## 为什么从 `data/cookies.json` 搬到这里（2026-10-26 用户定稿）
//!
//! 定稿原话："登录完成后凭据保存到运行同目录的 auth.json，不要再用 data/cookies.json 了。"
//! 摆放上的好处是**一眼看得见**：运行目录里就 `rustbilipc.exe`、`mpv.exe`、`debug.log`、
//! `auth.json`、`config.json` 这几样，出问题时不用钻进子目录找。
//! 旧文件**不删**（[`load`] 只把它当一次性迁移源）：那是他登录一次拿到的现场，
//! 删掉不可逆，万一迁移判断有误就没退路了。
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
    /// 凭据文件路径：`<exe 所在目录>/auth.json`（**不在 `data/` 里**，那是上一版的位置）。
    pub fn path() -> PathBuf {
        crate::app::exe_dir().join("auth.json")
    }

    /// 旧位置：`<exe 所在目录>/data/cookies.json`。只作为一次性迁移的来源读取，
    /// 迁移成功后保留原文件（见文件头那条理由）。
    pub fn legacy_path() -> PathBuf {
        crate::app::data_dir().join("cookies.json")
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

    /// 覆盖写入 `auth.json`。运行目录一定存在（exe 就在里面），所以不建目录；
    /// 失败时把路径带进错误信息方便排查权限问题。
    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::path();
        let json = serde_json::to_string_pretty(self).context("序列化凭据失败")?;
        std::fs::write(&path, json).with_context(|| format!("写入凭据失败: {}", path.display()))?;
        Ok(path)
    }

    /// 读取已保存的凭据；只有 `auth.json` 读不到时才去看旧文件，并把拿到的那份补写回去。
    ///
    /// 迁移判断只看"能不能读、字段够不够"，够格才写新文件——拿一份不合格的旧凭据
    /// 写到新位置，下次启动就会以为已登录。
    pub fn load() -> Option<Self> {
        let path = Self::path();
        if let Some(auth) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| Self::from_json(&raw))
        {
            return Some(auth);
        }

        let legacy = Self::legacy_path();
        let migrated = std::fs::read_to_string(&legacy)
            .ok()
            .and_then(|raw| Self::from_json(&raw))?;
        if !migrated.is_complete() {
            log::info!("旧凭据 {:?} 字段不全，不迁移（当成未登录）", legacy);
            return None;
        }
        log::info!("从 {:?} 迁移凭据到 {:?}（旧文件保留）", legacy, path);
        match migrated.save() {
            Ok(_) => Some(migrated),
            // 写不回去也要让用户能继续用这一份，别让一次 IO 失败把人挡在门外。
            Err(error) => {
                log::warn!("迁移后写入 auth.json 失败，本次仍用内存里的凭据: {error}");
                Some(migrated)
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
