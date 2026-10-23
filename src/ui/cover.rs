//! 封面/头像 URL 的处理：先补协议，再向 B 站图床要**按显示尺寸缩放**的那一份。
//!
//! ## 为什么要补协议（第十五轮真机日志里的第一案发现场）
//!
//! B 站搜索接口返回的 `pic` 是**省略协议的** `//i2.hdslb.com/bfs/archive/xxx.jpg`。
//! gpui 判断"这是 URL 还是内置资源路径"的依据是 `url::Url::parse` 是否成功
//! （`ref/gpui/gpui/src/elements/img.rs:53` 的 `is_uri`），而 `//host/path` 没有 base 必然解析失败
//! ⇒ 被当成 `Resource::Embedded` 去 asset 里找，**根本没走网络**。当时的 `debug.log`
//! 里那二十多条同一句就是它：
//!
//! ```text
//! ERROR gpui::asset_cache: Failed to load asset: Asset("Embedded resource not found: //i2.hdslb.com/…jpg")
//! ```
//!
//! 对照组：个人中心头像来自 `space_info` 的 `face`，本来就是 `https://` 绝对地址，
//! 同一份日志里它正常发起了连接。所以不是 CDN 拦我们，也不是 `set_http_client` 没生效
//! （那条路已经在第 4.5 节验过一次）。
//!
//! 好消息是**不用自己加日志**：asset 管线失败时会 `log::error!("Failed to load asset: {e:?}")`
//! （`ref/gpui/gpui/src/asset_cache.rs:134`），我们的文件日志收得到。
//!
//! ## 为什么要缩放（同一张图实测）
//!
//! - 稿件封面原图 = **541,549 字节**；头像原图 = **812,735 字节**
//! - `….jpg@320w_180h.jpg` = **14,832 字节**；`….jpg@112w_112h.jpg` = **2,740 字节**
//!
//! 搜索一页 30 条就是 16MB 与 0.44MB 的差别。gpui 的图片加载没有并发上限，
//! 一台 i5-6200U 的笔记本同时拉 30 张原图并解码，表现就是"封面迟迟不出来"。
//! B 站参考项目 Bili.Copilot 是同一思路（`DecodeWidth=600`，见 `docs/api-notes.md` 第 8 节）。
//!
//! ## 为什么不是"所有 URL 一律加后缀"
//!
//! `@…` 是 B 站图床（`*.hdslb.com`）的私有语法，不是标准 HTTP 特性：给别的域名再拼一次
//! 只会拿到 400/404（实测非法写法返回 400）。所以只在
//! "hdslb 域名 + 常见栅格后缀 + 尚未带 `@`" 时才改写，其余原样返回；GIF 不动，
//! 免得把动图变成静帧。

/// 补全协议，让 gpui 把它认成 URL 而不是内置资源路径。
///
/// - `//host/path` → `https://host/path`（B 站搜索封面的实际形态）
/// - `http://…hdslb.com…` → 升 `https://`（图床两种都收，实测都是 200；统一走 https）
/// - 其余（空串、绝对地址、非图床域名）原样返回
pub fn canonical(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Some(rest) = trimmed.strip_prefix("//") {
        return format!("https://{rest}");
    }
    if let Some(rest) = trimmed.strip_prefix("http://") {
        let host = rest.split('/').next().unwrap_or(rest);
        if host.ends_with("hdslb.com") {
            return format!("https://{rest}");
        }
    }
    trimmed.to_string()
}

/// 按显示尺寸向图床要缩略图；不符合条件的 URL 原样返回。
///
/// `width`/`height` 用**设备像素**（逻辑像素 × 缩放）：卡片封面框是 160×90，
/// 按 2× 缩放取 320×180 就够，取原图纯属浪费。
pub fn sized(url: &str, width: u32, height: u32) -> String {
    let Some(rest) = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")) else {
        return url.to_string();
    };

    let Some(slash) = rest.find('/') else { return url.to_string() };
    let host = &rest[..slash];
    if !host.ends_with("hdslb.com") {
        return url.to_string();
    }

    let path = &rest[slash..];
    if path.contains('@') {
        return url.to_string();
    }
    let lower = path.to_ascii_lowercase();
    if !lower.ends_with(".jpg") && !lower.ends_with(".jpeg") && !lower.ends_with(".png") {
        return url.to_string();
    }

    format!("{url}@{width}w_{height}h.jpg")
}

/// 界面真正该用的地址：先补协议，再按尺寸缩放。
///
/// DTO 里存的仍然是接口原样给的地址——数据层不替显示做决定；但**显示前必须过这一道**，
/// 否则 `//host/path` 会被 gpui 当成 asset 路径，图永远是空的。
pub fn for_display(url: &str, width: u32, height: u32) -> String {
    sized(&canonical(url), width, height)
}
