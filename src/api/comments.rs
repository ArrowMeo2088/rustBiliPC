//! 评论区（P6b）的接口封装：给详情页主列下面那一块用。
//!
//! ## 用哪条接口、为什么这么调
//!
//! - `comment().list(CommentListParams)` → `CommentListData`
//!   （`ref/bpi-rs/src/comment/client.rs:43-50`，端点 `https://api.bilibili.com/x/v2/reply`，
//!   **不带 WBI 签名**：那条 `list()` 里只 `query(&params.query_pairs())`，
//!   与 `video().homepage_recommendations()` 走 `get_wbi_sign2` 不同）。
//! - **`oid` 是 aid（数字稿件号），不是 bvid**（`src/comment/list.rs:20-23` 的 `CommentTarget`），
//!   所以这一路的入口依赖主体请求先回来：`VideoDetailInfo.aid`。
//! - 评论区类型代码：**1 = 视频**（`bilibili-API-collect` 的 comment/readme 类型代码表，
//!   bpi-rs 自己也这么写：`CommentType::Video = 1`，`src/comment/action.rs:22-27`；
//!   它的 live 测试同样用 `TEST_TYPE = 1`，`src/comment/list.rs:322`）。
//!   注意 **11 是相簿**，网上很多帖子写"视频=11"是把两套编号混了；这里按 1 走，
//!   真机万一取到空列表，第一个要复查的就是这个数字（`docs/architecture.md` 风险表第 19 条）。
//! - 分页是页码 `pn`（从 1 开始）+ `ps`；`ps` 被这个库限制在 1–20（`list.rs:94-101`）。
//!   总数在 `CommentListData.page.count`（根评论条数，`types.rs:76-82`），
//!   有它才能判断"还有没有下一页"。
//! - 排序 `sort`：0 时间 / 1 点赞 / 2 回复（`list.rs:52-66`）。
//!   参考项目默认给的是"最热"，我们对齐成 `CommentSortKind::Hot`。
//!
//! ## 时间只做减法
//!
//! `ctime` 是 unix 秒。这里显示"N分钟前 / N小时前 / N天前"这类相对时间，
//! **不需要时区**（同一时刻相减与 TZ 无关），所以既不引 `chrono`，也不手搓公历换算；
//! 参考项目也是这个读法（`CommentItemControl.xaml:97-104` 绑的是 `RelativeTime`，
//! 绝对时间只放在 ToolTip 里）。
//! [`relative_time`] 收两个参数（`ctime` 与 `now`）就是为了能单测：读时钟只发生在
//! [`fetch_comments`] 那一处。
//!
//! ## 载荷是严格结构
//!
//! `Comment` 与 `Member`（`src/comment/types.rs:12-60`、`:101-120`）里
//! `member`/`content`/`up_action`/`reply_control`/`folder` 与 `Member` 内的
//! `level_info/pendant/nameplate/official_verify/vip` **都不是 `Option`、也没有
//! `#[serde(default)]`**：B 站少给任何一个就整页 `BpiError::parse`。
//! 处置与主页/收藏同一套——原因写进评论区自己的提示位，主体那一块不受影响。

use std::sync::Arc;

use bpi_rs::{
    BpiClient, BpiResult,
    comment::{CommentListParams, CommentSort, CommentTarget},
    comment::list::CommentListData,
};

use super::spawn_on_api_runtime;

/// 视频评论区的类型代码。**1 = 视频**，11 是相簿（见文件头那条）。
pub const COMMENT_TYPE_VIDEO: i32 = 1;

/// 每页条数。这个库把 `ps` 的上限卡死在 20（`src/comment/list.rs:94-101`）。
pub const COMMENT_PAGE_SIZE: u32 = 20;

/// 评论排序口径（页面不碰 bpi 的类型，分层规则）。
///
/// `Default` 是必须的：`CommentPage` 派生了 `Default`，字段就得都能默认。
/// 默认给"最热"，与参考项目的评论区一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CommentSortKind {
    /// 按点赞（参考项目的默认）。
    #[default]
    Hot,
    /// 按时间。
    Latest,
}

impl CommentSortKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Hot => "最热",
            Self::Latest => "最新",
        }
    }

    fn to_bpi(self) -> CommentSort {
        match self {
            Self::Hot => CommentSort::Like,
            Self::Latest => CommentSort::Time,
        }
    }
}

/// 一条顶层评论。
#[derive(Clone, Debug, Default)]
pub struct CommentItem {
    /// 评论 id（做列表项的稳定 key；将来展开子回复也要它）。
    pub rpid: i64,
    /// 昵称。
    pub author: String,
    /// 头像（过 `ui::cover` 再显示）。
    pub face: String,
    /// 正文。表情在接口里是 `[笑]` 这种转义文本，我们**按原样显示**：
    /// 参考项目有表情图床映射，我们没有，硬把 `[笑]` 换成图片只能靠猜。
    pub message: String,
    /// 点赞数。
    pub like: i64,
    /// 子回复条数（`count`）。
    pub reply_count: i64,
    /// "3天前"这类相对时间。
    pub time_text: String,
}

/// 一页评论。
#[derive(Clone, Debug, Default)]
pub struct CommentPage {
    pub items: Vec<CommentItem>,
    /// 根评论总条数（`page.count`）。没给就是 `None`，界面退而求其次：满一页就还能翻。
    pub total: Option<u64>,
    pub page: u32,
    pub sort: CommentSortKind,
}

impl CommentPage {
    /// 还能不能再翻一页。
    ///
    /// 有总数就用总数（`已翻过的条数 < 总数`）；没有总数就假设"这一页取满了 `ps`"就还有。
    pub fn has_more(&self) -> bool {
        let got = self.items.len() as u64;
        match self.total {
            Some(total) => {
                // `saturating_sub`：页号理论上从 1 起，但这里不该因为一个 0 把主线程 panic 出来。
                let before =
                    u64::from(self.page.saturating_sub(1)) * u64::from(COMMENT_PAGE_SIZE);
                before + got < total
            }
            None => got >= u64::from(COMMENT_PAGE_SIZE),
        }
    }
}

/// 相对时间：只做减法，不读时区。
pub fn relative_time(ctime: i64, now: i64) -> String {
    // 时钟偏差（评论时间戳在未来）不该显示出负数。
    let delta = now - ctime;
    if delta < 60 {
        return "刚刚".into();
    }
    if delta < 3_600 {
        return format!("{}分钟前", delta / 60);
    }
    if delta < 86_400 {
        return format!("{}小时前", delta / 3_600);
    }
    if delta < 30 * 86_400 {
        return format!("{}天前", delta / 86_400);
    }
    if delta < 365 * 86_400 {
        return format!("{}个月前", delta / (30 * 86_400));
    }
    format!("{}年前", delta / (365 * 86_400))
}

/// 响应 → 我们的一页评论。`now` 由调用方给（单测要点）。
pub fn to_page(
    data: CommentListData,
    page: u32,
    sort: CommentSortKind,
    now: i64,
) -> CommentPage {
    let total = data.page.as_ref().map(|info| info.count);
    let items = data
        .replies
        .unwrap_or_default()
        .into_iter()
        .map(|item| CommentItem {
            rpid: item.rpid,
            author: item.member.uname,
            face: item.member.avatar,
            message: item.content.message,
            like: item.like,
            reply_count: item.count,
            time_text: relative_time(item.ctime, now),
        })
        .collect();
    CommentPage {
        items,
        total,
        page,
        sort,
    }
}

/// 取一页评论。`aid` 是数字稿件号（不是 bvid）。
pub fn fetch_comments(
    client: Arc<BpiClient>,
    aid: u64,
    page: u32,
    sort: CommentSortKind,
) -> tokio::task::JoinHandle<BpiResult<CommentPage>> {
    log::debug!(
        "提交 评论区 到 API runtime：aid={aid} 第 {page} 页 · {}",
        sort.label()
    );
    spawn_on_api_runtime(async move {
        let started = std::time::Instant::now();
        let target = CommentTarget::new(COMMENT_TYPE_VIDEO, aid as i64)?;
        let params = CommentListParams::new(target)
            .with_page(page)?
            .with_page_size(COMMENT_PAGE_SIZE)?
            .with_sort(sort.to_bpi());
        let data = client.comment().list(params).await?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_secs() as i64)
            .unwrap_or_default();
        let parsed = to_page(data, page, sort, now);
        log::debug!(
            "评论区返回 {} 条（总数 {:?}），用时 {:?}",
            parsed.items.len(),
            parsed.total,
            started.elapsed()
        );
        Ok(parsed)
    })
}
