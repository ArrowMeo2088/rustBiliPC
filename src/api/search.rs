//! 搜索接口的封装层。
//!
//! ## 为什么要有这一层
//!
//! 分层规则（`docs/architecture.md` 第 3 节）要求 `pages/*` 不直接 `use bpi_rs`：
//! bpi-rs 的响应结构会随上游接口变动，把它挡在这一层，页面只依赖 [`SearchHit`] 这种
//! 我们自己的 DTO。转换与清洗也集中在这里，方便单测。
//!
//! ## 已核实的接口形状
//!
//! - `BpiClient::search() -> SearchClient<'_>`（`src/client.rs:568`，借用客户端）
//! - `SearchClient::video(SearchVideoParams) -> BpiResult<SearchData<Vec<Video>>>`
//!   （`src/search/client.rs:105`，走 `/x/web-interface/wbi/search/type`，WBI 自动签）
//! - `SearchVideoParams::new(kw) -> BpiResult<Self>`、`.with_order(..)`、`.with_page(..) -> BpiResult<Self>`
//!   （`src/search/search_params.rs:430/440/455`）
//!
//! 因为要借用客户端，且 future 必须是 `'static + Send` 才能投给 tokio，
//! 这里统一接收 `Arc<BpiClient>` 并在 async 块内部完成借用。

use std::sync::Arc;

use bpi_rs::{
    BpiClient, BpiResult,
    search::{SearchData, SearchOrder, SearchVideoParams, Video},
};

use super::spawn_on_api_runtime;

/// 一条搜索命中。字段都已经是可直接展示的最终形态。
#[derive(Clone, Debug, Default)]
pub struct SearchHit {
    /// 稿件号，详情页入参兼列表项稳定 key。
    pub bvid: String,
    /// 已去掉高亮标签的标题。
    pub title: String,
    /// UP 主昵称。
    pub author: String,
    /// 封面原图 URL（交给 gpui 的 URI 图片加载器）。
    pub cover_url: String,
    /// 时长，B 站直接给 `mm:ss`。
    pub duration: String,
    /// 播放量，已格式化成 `x.xx万` 这类中文读法。
    pub play_text: String,
    /// 弹幕数。
    pub danmaku: u64,
}

/// 分页信息。
#[derive(Clone, Debug, Default)]
pub struct PageInfo {
    pub page: i64,
    pub page_size: i64,
    pub num_results: i64,
    pub num_pages: i64,
}

/// 一次搜索的完整结果。
#[derive(Clone, Debug, Default)]
pub struct SearchOutcome {
    pub hits: Vec<SearchHit>,
    pub info: PageInfo,
}

/// 去掉 B 站标题里的高亮标签。
///
/// 搜索接口返回的 `title` 形如 `…<em class="keyword">关键词</em>…`，
/// 直接显示会把标签暴露在界面上。
pub fn strip_keyword_tags(raw: &str) -> String {
    raw.replace("<em class=\"keyword\">", "")
        .replace("</em>", "")
}

/// 把播放量这类大数字压成中文读法。
///
/// B 站前端就是这个口径：1 万以内原样，之后按万/亿截断到两位小数并去掉多余的 `.00`。
pub fn format_count(count: u64) -> String {
    const WAN: u64 = 10_000;
    const YI: u64 = 100_000_000;

    if count < WAN {
        return count.to_string();
    }
    if count < YI {
        return trim_tail(format!("{:.2}", count as f64 / WAN as f64)).push_suffix("万");
    }
    trim_tail(format!("{:.2}", count as f64 / YI as f64)).push_suffix("亿")
}

/// 去掉小数部分尾部的 `.` 与 `0`（`12.00` -> `12`，`12.30` -> `12.3`）。
fn trim_tail(text: String) -> String {
    let mut out = text;
    while out.ends_with('0') {
        out.pop();
    }
    if out.ends_with('.') {
        out.pop();
    }
    out
}

trait PushSuffix {
    fn push_suffix(self, suffix: &str) -> String;
}

impl PushSuffix for String {
    fn push_suffix(mut self, suffix: &str) -> String {
        self.push_str(suffix);
        self
    }
}

/// 把接口响应转成 UI 用的结果。
pub fn to_outcome(data: SearchData<Vec<Video>>) -> SearchOutcome {
    let info = PageInfo {
        page: data.page,
        page_size: data.page_size,
        num_results: data.num_results,
        num_pages: data.num_pages,
    };

    // `result` 是 Option：无结果时上游返回 null 而不是空数组。
    let hits = data
        .result
        .unwrap_or_default()
        .into_iter()
        .map(|video| SearchHit {
            bvid: video.bvid,
            title: strip_keyword_tags(&video.title),
            author: video.author,
            cover_url: video.pic,
            duration: video.duration,
            play_text: format_count(video.play),
            danmaku: video.danmaku,
        })
        .collect();

    SearchOutcome { hits, info }
}

/// 我们自己的排序口径。
///
/// 存在的理由就是分层规则：`pages/` 不许出现 bpi 的类型，这里包一层再映射。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchSort {
    /// 综合排序（默认）。
    Comprehensive,
    /// 按播放量。
    PlayCount,
    /// 按发布时间。
    PublishDate,
}

impl SearchSort {
    /// 中文化标签，供页面的排序控件直接用。
    pub fn label(self) -> &'static str {
        match self {
            Self::Comprehensive => "综合",
            Self::PlayCount => "播放量",
            Self::PublishDate => "最新",
        }
    }

    fn to_order(self) -> SearchOrder {
        match self {
            Self::Comprehensive => SearchOrder::TotalRank,
            Self::PlayCount => SearchOrder::Click,
            Self::PublishDate => SearchOrder::PubDate,
        }
    }
}

/// 发起一次视频搜索。
///
/// 返回的 `JoinHandle` 在 tokio 上跑，gpui 侧 await 它即可（见 [`crate::api`] 模块注释）。
/// `keyword` 为空会被 `SearchVideoParams::new` 判为参数错误——调用方应在本地先挡住空输入，
/// 免得为一个空串发一次网络请求。
pub fn search_videos(
    client: Arc<BpiClient>,
    keyword: String,
    page: u32,
    sort: SearchSort,
) -> tokio::task::JoinHandle<BpiResult<SearchOutcome>> {
    log::debug!("提交 video 搜索到 API runtime：page={page}");
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let params = SearchVideoParams::new(keyword)?
            .with_order(sort.to_order())
            .with_page(page)?;
        let data = client.search().video(params).await?;
        let hits = data.result.as_ref().map_or(0, Vec::len);
        log::debug!("搜索返回 {hits} 条，用时 {:?}", started.elapsed());
        Ok(to_outcome(data))
    })
}
