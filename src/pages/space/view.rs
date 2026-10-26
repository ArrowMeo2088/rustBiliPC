//! 用户空间页（P7）：点详情页的 UP 主进这里。
//!
//! ## 三块内容来自三条不同形状的接口
//!
//! | 页签 | 接口 | 分页口径 |
//! | --- | --- | --- |
//! | 投稿 | `user().uploaded_videos` | 页码 `pn`/`ps` + 总数，所以做"上一页/下一页" |
//! | 图文 | `opus().space_feed` | **游标**（`offset` + `has_more`），只能往后，所以做"加载更多"并累加 |
//! | 收藏 | `fav().created_list` + `collected_list`（复用 [`favorites::fetch_folders`]） | 一次给全，没有分页 |
//!
//! 细节与行号都在 [`space`](crate::api::space) 的文件头，这里只记界面上的决定。
//!
//! ## 标签（分区）筛选
//!
//! 投稿接口顺手把"这个 UP 用过的分区"回在 `list.tlist` 里，所以筛选 chips **不需要额外的接口**：
//! 第一页回来后把 chips 画出来，点一个就带 `tid` 重新取第 1 页。
//! "全部"这一档是我们自己加的（`tid = 0`），因为 `tlist` 里没有它。
//! `tlist` 在库里是 `serde_json::Value`，解析口径见 [`space::parse_tlist`]：认不出来就少一个 chip，
//! 不整页报错。
//!
//! ## 为什么每一块自己一个序号
//!
//! 三块都能各自被反复触发（切页签、点筛选、点排序、翻页）。共用一个 `seq` 的话，
//! "图文正在加载时点了投稿的下一页"会互相把对方的结果判成过期。
//! 分块序号 + 分块提示位，和第 4.11 节"三路并发各写各的槽"是同一个道理。
//!
//! ## 这一轮没做的
//!
//! 收藏夹**只列出来，点进去看内容**要再加一层页（`fav().list_detail` 我们已经在"我的"那页用过），
//! 那是下钻与栈的问题，不是接口的问题；先不顺手加，等这一版真机看过再定形状。

use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::api::collection::{self, CollectionBrief};
use crate::api::favorites::{self, FavFolder};
use crate::api::space::{
    self, SpacePost, SpaceProfile, SpaceSection, SpaceVideo, VideoFilter, VideoOrder,
    VIDEO_PAGE_SIZE,
};
use crate::pages::{OpenCollection, OpenUser, OpenVideo};
use crate::ui::video_card::{self, VideoCardData};
use crate::ui::scroll::DragScroll;
use crate::ui::{layout, palette};

/// 用户空间页根视图。
pub struct UserSpacePage {
    client: Arc<bpi_rs::BpiClient>,
    /// 单指/按住拖动的滚动状态（`ui::scroll::DragScroll`，与下面那个滚动容器一一对应）。
    scroll: DragScroll,
    /// 空间主人的 mid。三块接口都要它。
    mid: u64,
    /// 从上一页带过来的昵称：资料卡回来之前也要有名字可显示。
    name: SharedString,
    profile: Option<SpaceProfile>,
    profile_note: SharedString,
    section: SpaceSection,

    videos: Vec<SpaceVideo>,
    video_filters: Vec<VideoFilter>,
    video_page: u32,
    video_total: u64,
    /// 当前选中的分区（0 = 全部）。
    video_tid: u64,
    video_order: VideoOrder,
    video_note: SharedString,
    video_seq: u32,
    video_loaded: bool,

    posts: Vec<SpacePost>,
    /// 下一页的游标（接口回什么我们存什么，不自造）。
    post_offset: SharedString,
    post_page: u32,
    post_has_more: bool,
    post_note: SharedString,
    post_seq: u32,
    post_loaded: bool,

    folders: Vec<FavFolder>,
    folder_note: SharedString,
    folder_seq: u32,
    folder_loaded: bool,

    /// 合集与系列（P12）。接口一次同时给两类，我们合成一列，所以这里只有"第几页"一个游标。
    collections: Vec<CollectionBrief>,
    collection_page: u64,
    collection_has_more: bool,
    collection_note: SharedString,
    collection_seq: u32,
    collection_loaded: bool,
}

impl UserSpacePage {
    /// 建页。资料卡与投稿首页由 [`ensure_loaded`](Self::ensure_loaded) 触发，
    /// 图文与收藏在第一次切到那个页签时才取。
    pub fn new(client: Arc<bpi_rs::BpiClient>, mid: u64, name: String) -> Self {
        Self {
            scroll: DragScroll::default(),
            client,
            mid,
            name: name.into(),
            profile: None,
            profile_note: "资料加载中…".into(),
            section: SpaceSection::Videos,
            videos: Vec::new(),
            video_filters: Vec::new(),
            video_page: 1,
            video_total: 0,
            video_tid: 0,
            video_order: VideoOrder::Latest,
            video_note: "投稿加载中…".into(),
            video_seq: 0,
            video_loaded: false,
            posts: Vec::new(),
            post_offset: "".into(),
            post_page: 0,
            post_has_more: false,
            post_note: "".into(),
            post_seq: 0,
            post_loaded: false,
            folders: Vec::new(),
            folder_note: "".into(),
            folder_seq: 0,
            folder_loaded: false,
            collections: Vec::new(),
            collection_page: 0,
            collection_has_more: false,
            collection_note: "".into(),
            collection_seq: 0,
            collection_loaded: false,
        }
    }

    /// 第一次被看到时取资料卡 + 投稿首页（这一页是从详情页点进来的，
    /// 不该先给一片空白）。
    pub fn ensure_loaded(&mut self, cx: &mut Context<Self>) {
        if self.video_loaded {
            return;
        }
        self.load_profile(cx);
        self.load_videos(1, cx);
    }

    /// 切页签：没取过的块在这里补取（图文取第一页，收藏一次取完）。
    fn select_section(&mut self, section: SpaceSection, cx: &mut Context<Self>) {
        self.section = section;
        match section {
            SpaceSection::Videos => {
                if !self.video_loaded {
                    self.load_videos(1, cx);
                }
            }
            SpaceSection::Posts => {
                if !self.post_loaded {
                    self.load_posts(cx);
                }
            }
            SpaceSection::Folders => {
                if !self.folder_loaded {
                    self.load_folders(cx);
                }
            }
            SpaceSection::Collections => {
                if !self.collection_loaded {
                    self.load_collections(1, cx);
                }
            }
        }
        cx.notify();
    }

    fn load_profile(&mut self, cx: &mut Context<Self>) {
        self.profile_note = "资料加载中…".into();
        let join = space::fetch_profile(Arc::clone(&self.client), self.mid);
        cx.spawn(async move |this, cx| {
            let outcome = match join.await {
                Ok(inner) => inner.map_err(|error| error.to_string()),
                Err(join_error) => Err(crate::app::describe_join_error(join_error)),
            };
            let Some(view) = this.upgrade() else { return };
            view.update(cx, |view, cx| {
                match outcome {
                    Ok(profile) => {
                        // 昵称以接口为准（上一页带过来的可能是旧缓存里的名字）。
                        view.name = profile.name.clone().into();
                        view.profile_note = "".into();
                        view.profile = Some(profile);
                    }
                    Err(message) => view.profile_note = message.into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 取投稿的一页（页码由调用方给，分区与排序读当前状态）。
    fn load_videos(&mut self, page: u32, cx: &mut Context<Self>) {
        self.video_loaded = true;
        self.video_seq += 1;
        let seq = self.video_seq;
        self.video_page = page;
        self.video_note = format!("投稿加载中…（第 {page} 页）").into();
        cx.notify();

        let join = space::fetch_videos(
            Arc::clone(&self.client),
            self.mid,
            page,
            self.video_tid,
            self.video_order,
        );
        cx.spawn(async move |this, cx| {
            let outcome = match join.await {
                Ok(inner) => inner.map_err(|error| error.to_string()),
                Err(join_error) => Err(crate::app::describe_join_error(join_error)),
            };
            let Some(view) = this.upgrade() else { return };
            view.update(cx, |view, cx| {
                // 每块自己一套序号：点筛选、排序、翻页都会让上一次在途结果作废。
                if view.video_seq != seq {
                    return;
                }
                match outcome {
                    Ok((videos, filters, total)) => {
                        view.video_note = if videos.is_empty() {
                            "这个筛选下没有投稿".into()
                        } else {
                            "".into()
                        };
                        view.video_filters = filters;
                        view.video_total = total;
                        view.videos = videos;
                    }
                    Err(message) => {
                        view.videos.clear();
                        view.video_note = format!("取投稿失败：{message}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 图文：**追加**下一页（游标只能往前走，没有"上一页"）。
    fn load_posts(&mut self, cx: &mut Context<Self>) {
        self.post_loaded = true;
        self.post_seq += 1;
        let seq = self.post_seq;
        let next_page = self.post_page + 1;
        let offset = Some(self.post_offset.to_string());
        self.post_note = "加载中…".into();
        cx.notify();

        let join = space::fetch_posts(Arc::clone(&self.client), self.mid, next_page, offset);
        cx.spawn(async move |this, cx| {
            let outcome = match join.await {
                Ok(inner) => inner.map_err(|error| error.to_string()),
                Err(join_error) => Err(crate::app::describe_join_error(join_error)),
            };
            let Some(view) = this.upgrade() else { return };
            view.update(cx, |view, cx| {
                if view.post_seq != seq {
                    return;
                }
                match outcome {
                    Ok((items, has_more, offset)) => {
                        view.post_page = next_page;
                        view.post_offset = offset.into();
                        view.post_has_more = has_more;
                        view.posts.extend(items);
                        view.post_note = if view.posts.is_empty() {
                            "TA 没有发过图文".into()
                        } else {
                            "".into()
                        };
                    }
                    Err(message) => view.post_note = format!("取图文失败：{message}").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_folders(&mut self, cx: &mut Context<Self>) {
        self.folder_loaded = true;
        self.folder_seq += 1;
        let seq = self.folder_seq;
        self.folder_note = "收藏夹加载中…".into();
        cx.notify();

        let join = favorites::fetch_folders(Arc::clone(&self.client), self.mid);
        cx.spawn(async move |this, cx| {
            let outcome = match join.await {
                Ok(inner) => inner.map_err(|error| error.to_string()),
                Err(join_error) => Err(crate::app::describe_join_error(join_error)),
            };
            let Some(view) = this.upgrade() else { return };
            view.update(cx, |view, cx| {
                if view.folder_seq != seq {
                    return;
                }
                match outcome {
                    Ok(folders) => {
                        view.folder_note = if folders.is_empty() {
                            "TA 没有公开的收藏夹".into()
                        } else {
                            "".into()
                        };
                        view.folders = folders;
                    }
                    Err(message) => {
                        view.folder_note = format!("取收藏夹失败：{message}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 取"合集与系列"的某一页。`page == 1` 是替换，大于 1 是往后**累加**。
    ///
    /// 这一格与投稿/图文不同：**一次请求同时给两类**（`seasons_list` 与 `series_list`），
    /// 我们合成一列显示，所以只有"第几页"这一个游标。翻页判据见 `api/collection.rs` 文件头第 2 条
    /// （接口那个 `page.total` 的语义没文档，我们不显示它、也不拿它当"翻完了"的依据）。
    fn load_collections(&mut self, page: u64, cx: &mut Context<Self>) {
        self.collection_loaded = true;
        self.collection_seq += 1;
        let seq = self.collection_seq;
        self.collection_note = if page == 1 {
            "合集加载中…".into()
        } else {
            "正在加载下一页…".into()
        };
        cx.notify();

        let join = collection::fetch_collections(Arc::clone(&self.client), self.mid, page);
        crate::pages::loading::spawn_into(
            cx,
            seq,
            "合集与系列",
            join,
            |view| view.collection_seq,
            // `move` 不能省：闭包要跨 `await` 活下来（`'static`），而它要读外面那个栈上的 `page`。
            move |view, outcome| match outcome {
                Ok(list) => {
                    view.collection_page = list.page;
                    view.collection_has_more = list.has_more;
                    if list.page <= 1 {
                        view.collections = list.items;
                    } else {
                        view.collections.extend(list.items);
                    }
                    view.collection_note = if view.collections.is_empty() {
                        "TA 没有公开的合集或系列".into()
                    } else {
                        "".into()
                    };
                }
                Err(message) => {
                    // 只有第一页失败才清空：翻下一页失败时把已加载的留着（同收藏夹内容那一页的口径）。
                    if page == 1 {
                        view.collections.clear();
                    }
                    view.collection_note = format!("取合集与系列失败：{message}").into();
                }
            },
        );
    }

    /// 投稿的三档排序（接口一次只能给一档，所以点了就重取第 1 页）。
    fn select_order(&mut self, order: VideoOrder, cx: &mut Context<Self>) {
        if self.video_order == order {
            return;
        }
        self.video_order = order;
        self.load_videos(1, cx);
    }

    /// 点分区 chip：0 是"全部"（那一档是我们自己加的，`tlist` 里没有）。
    fn select_filter(&mut self, tid: u64, cx: &mut Context<Self>) {
        if self.video_tid == tid {
            return;
        }
        self.video_tid = tid;
        self.load_videos(1, cx);
    }

    /// 资料卡：48 的圆头像 + 昵称/UID + 四档数字 + 签名。
    ///
    /// 度量与"我的"页那张贴片同一套（`AccountCard.xaml:13-51`：`Padding 12`、列距 12、
    /// 头像 48 圆角 24、`DecodeWidth=100`；数字行是 BodyStrong + Tertiary 标签）。
    fn profile_card(&self) -> AnyElement {
        let face = self
            .profile
            .as_ref()
            .map(|profile| profile.face.clone())
            .unwrap_or_default();
        let (fans, follow, archive, article, sign) = match self.profile.as_ref() {
            Some(profile) => (
                profile.fans_text.clone(),
                profile.follow_text.clone(),
                profile.archive_count.to_string(),
                profile.article_count.to_string(),
                profile.sign.clone(),
            ),
            None => (
                "—".into(),
                "—".into(),
                "—".into(),
                "—".into(),
                self.profile_note.to_string(),
            ),
        };

        div()
            .flex()
            .w_full()
            .gap(px(palette::STAT_ITEM_GAP))
            .items_start()
            .px(px(palette::CARD_TEXT_PADDING_X))
            .pt(px(palette::CARD_TEXT_PADDING_TOP))
            .child(layout::avatar(&face, palette::AVATAR_ACCOUNT))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(palette::CARD_LINE_GAP))
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .w_full()
                            .items_center()
                            .gap(px(palette::CARD_TEXT_ROW_GAP))
                            .child(
                                div()
                                    .text_size(px(palette::FONT_BODY))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(palette::text_primary())
                                    .child(self.name.clone()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(palette::FONT_CAPTION))
                                    .text_color(palette::text_tertiary())
                                    .child(format!("UID {}", self.mid)),
                            ),
                    )
                    .child(layout::stat_line(vec![
                        format!("粉丝 {fans}").into(),
                        format!("关注 {follow}").into(),
                        format!("投稿 {archive}").into(),
                        format!("图文 {article}").into(),
                    ]))
                    .when(!sign.is_empty(), |this| {
                        this.child(
                            div()
                                .w_full()
                                .text_size(px(palette::FONT_CAPTION))
                                .text_color(palette::text_secondary())
                                .child(sign),
                        )
                    }),
            )
            .into_any_element()
    }

    /// 页签行：投稿 / 图文 / 收藏。
    fn section_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut bar = div()
            .flex()
            .w_full()
            .items_center()
            .gap(px(palette::CARD_TEXT_ROW_GAP))
            .px(px(palette::CARD_TEXT_PADDING_X));
        for section in [
            SpaceSection::Videos,
            SpaceSection::Posts,
            SpaceSection::Folders,
            SpaceSection::Collections,
        ] {
            let button = Button::new(format!("space-{}", section.label()))
                .label(section.label())
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    this.select_section(section, cx);
                }));
            bar = bar.child(if self.section == section {
                button.primary()
            } else {
                button
            });
        }
        bar.child(div().flex_1()).child(
            div()
                .flex_shrink_0()
                .text_size(px(palette::FONT_CAPTION))
                .text_color(palette::text_tertiary())
                .child(match self.section {
                    // 这一行的数字跟着当前筛选走（它就是本页的总数，翻页也用它），
                    // 所以文字要说清是"这个分区的"，别和 chips 上"全部"那个总数打架。
                    SpaceSection::Videos => {
                        if self.video_tid == 0 {
                            format!("共 {} 个投稿", self.video_total)
                        } else {
                            format!("当前分区 {} 个投稿", self.video_total)
                        }
                    }
                    SpaceSection::Posts => format!("已加载 {} 条", self.posts.len()),
                    SpaceSection::Folders => format!("{} 个收藏夹", self.folders.len()),
                    // 这一格的数字是**我们自己数出来的**：接口那个 `page.total` 的语义没文档，
                    // 不显示它（理由见 `api/collection.rs` 文件头第 2 条）。
                    SpaceSection::Collections => {
                        let seasons = self
                            .collections
                            .iter()
                            .filter(|item| item.kind == collection::CollectionKind::Season)
                            .count();
                        let series = self.collections.len() - seasons;
                        format!("已列出 {} 个（合集 {seasons} · 系列 {series}）", self.collections.len())
                    }
                }),
        )
        .into_any_element()
    }

    /// 分区 chips：第一格固定是"全部"，其余来自接口回来的 `tlist`。
    ///
    /// 形状说明：参考项目在同类位置用的是**一行卡片 + 一个 ComboBox**
    /// （`IndexFilterItemControl.xaml:12-19`：`Padding=16,12`、列距 8、`ControlCornerRadius`）。
    /// gpui 侧没有 ComboBox 的等价物可用（kit 有，但引它等于为这一排筛选再拖一套弹层状态机），
    /// 所以按 B 站网页端空间页的读法做成一排 chip；**内边距不新造数字**，
    /// 取列表行的小一档（横向 8 = `LIST_ROW_PADDING_Y`、纵向 4 = `CARD_LINE_GAP`）。
    fn filter_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.video_filters.is_empty() {
            // 接口没给分区（新号、或这一格换了形状）：不画空的一排。
            return div().into_any_element();
        }
        // "全部"那一格的计数**不能**用 `video_total`：那是当前筛选（`tid`）的总数，
        // 点进某个分区后它会等于那个分区的数，于是"全部 12 / 游戏 12 / 单机 3"这种自相矛盾的
        // 一排就出现了。`tlist` 的计数与筛选无关（每个分区的投稿数），加起来才是诚实的总数。
        let mut chips = vec![(
            0u64,
            "全部".to_string(),
            self.video_filters.iter().map(|filter| filter.count).sum::<u64>(),
        )];
        chips.extend(
            self.video_filters
                .iter()
                .map(|filter| (filter.tid, filter.name.clone(), filter.count)),
        );

        let mut row = div()
            .flex()
            .flex_wrap()
            .w_full()
            .gap(px(palette::CARD_TEXT_ROW_GAP))
            .px(px(palette::CARD_TEXT_PADDING_X))
            .pt(px(palette::CARD_TEXT_ROW_GAP));
        for (tid, name, count) in chips {
            let selected = self.video_tid == tid;
            row = row.child(
                div()
                    .id(format!("space-tid-{tid}"))
                    .flex()
                    .items_center()
                    .gap(px(palette::CARD_TEXT_ROW_GAP))
                    .px(px(palette::LIST_ROW_PADDING_Y))
                    .py(px(palette::CARD_LINE_GAP))
                    .rounded(px(palette::RADIUS))
                    .cursor_pointer()
                    .text_size(px(palette::FONT_CAPTION))
                    .text_color(if selected {
                        palette::text_primary()
                    } else {
                        palette::text_secondary()
                    })
                    .bg(if selected {
                        palette::card_background_elevated()
                    } else {
                        palette::card_background()
                    })
                    .border_1()
                    .border_color(if selected {
                        palette::accent()
                    } else {
                        palette::divider()
                    })
                    .child(name)
                    .child(
                        div()
                            .text_color(palette::text_tertiary())
                            .child(count.to_string()),
                    )
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.select_filter(tid, cx);
                    }))
                    .hover(|style| style.bg(palette::card_background_hover())),
            );
        }
        row.into_any_element()
    }

    /// 排序档 + 翻页。
    fn video_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let page = self.video_page;
        let last_page = space::last_page(self.video_total, VIDEO_PAGE_SIZE);
        let mut bar = div()
            .flex()
            .w_full()
            .items_center()
            .gap(px(palette::CARD_TEXT_ROW_GAP))
            .px(px(palette::CARD_TEXT_PADDING_X))
            .pt(px(palette::CARD_TEXT_ROW_GAP));
        for order in [
            VideoOrder::Latest,
            VideoOrder::MostPlay,
            VideoOrder::MostFavorite,
        ] {
            let button = Button::new(format!("space-order-{}", order.label()))
                .label(order.label())
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    this.select_order(order, cx);
                }));
            bar = bar.child(if self.video_order == order {
                button.primary()
            } else {
                button
            });
        }
        bar.child(div().flex_1())
            .child(
                Button::new("space-video-prev")
                    .label("上一页")
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        if this.video_page > 1 {
                            let page = this.video_page - 1;
                            this.load_videos(page, cx);
                        }
                    })),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(palette::FONT_CAPTION))
                    .text_color(palette::text_tertiary())
                    .child(format!("{page} / {last_page}")),
            )
            .child(
                Button::new("space-video-next")
                    .label("下一页")
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        if page < last_page {
                            let next = this.video_page + 1;
                            this.load_videos(next, cx);
                        }
                    })),
            )
            .into_any_element()
    }

    /// 当前页签的内容体。
    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.section {
            SpaceSection::Videos => {
                let cards = self
                    .videos
                    .iter()
                    .cloned()
                    .map(|video| {
                        let card = VideoCardData {
                            bvid: video.bvid.into(),
                            title: video.title.into(),
                            // 空间页里 UP 主就是这个人，昵称填这一格比留空好看（卡片三行的形状不变）。
                            author: self.name.clone(),
                            duration: video.duration.into(),
                            play_text: video.play_text.into(),
                            cover_url: video.cover_url.into(),
                            extra: video.danmaku_text.into(),
                        };
                        let bvid = card.bvid.as_str().to_string();
                        video_card::clickable(
                            card,
                            cx.listener(move |_this, _event, _window, cx| {
                                cx.emit(OpenVideo { bvid: bvid.clone() });
                            }),
                        )
                        .into_any_element()
                    })
                    .collect();
                self.with_note(video_card::grid(cards), self.video_note.clone())
            }
            SpaceSection::Posts => {
                let rows: Vec<AnyElement> = self
                    .posts
                    .iter()
                    .map(|post| {
                        video_card::post_card(
                            post.text.clone().into(),
                            post.like_text.clone().into(),
                            post.cover_url.clone().into(),
                            post.cover_ratio,
                        )
                    })
                    .collect();
                let more = if self.post_has_more {
                    vec![
                        div()
                            .id("space-post-more")
                            .flex()
                            .w_full()
                            .justify_center()
                            .py(px(palette::CARD_TEXT_ROW_GAP))
                            .child(
                                Button::new("space-post-more-btn")
                                    .label("加载更多图文")
                                    .on_click(cx.listener(|this, _event, _window, cx| {
                                        this.load_posts(cx);
                                    })),
                            )
                            .into_any_element(),
                    ]
                } else {
                    Vec::new()
                };
                let list = div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .gap(px(palette::COMMENT_ROW_GAP))
                    .px(px(palette::CARD_TEXT_PADDING_X))
                    .py(px(palette::BODY_PADDING_Y))
                    .children(rows)
                    .children(more);
                self.with_note(list, self.post_note.clone())
            }
            SpaceSection::Folders => {
                let rows: Vec<AnyElement> = self
                    .folders
                    .iter()
                    .map(|folder| {
                        let kind = if folder.created { "TA 创建" } else { "TA 收藏" };
                        layout::list_row(
                            folder.title.clone(),
                            format!("{kind} · {} 个内容", folder.media_count),
                        )
                    })
                    .collect();
                let list = div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .gap(px(palette::LIST_GAP))
                    .px(px(palette::CARD_TEXT_PADDING_X))
                    .py(px(palette::BODY_PADDING_Y))
                    .children(rows);
                self.with_note(list, self.folder_note.clone())
            }
            SpaceSection::Collections => {
                let rows: Vec<AnyElement> = self
                    .collections
                    .iter()
                    .map(|item| {
                        // 最近一条稿件的标题当"这一格里大概是什么内容"的线索：
                        // 接口顺手给了（`SeasonsItem.archives`），不用多发一次请求。
                        let mut meta =
                            format!("{} · 共 {} 条", item.kind.label(), item.total);
                        if let Some(first) = item.samples.first() {
                            meta.push_str(&format!(" · 最近：{first}"));
                        }
                        let event = OpenCollection {
                            kind: item.kind,
                            id: item.id,
                            mid: self.mid,
                            title: item.title.clone(),
                        };
                        layout::clickable_row(
                            format!("collection-{}-{}", item.kind.label(), item.id),
                            item.title.clone(),
                            meta,
                            cx.listener(move |_this, _event, _window, cx| {
                                cx.emit(event.clone());
                            }),
                        )
                    })
                    .collect();
                let more: Vec<AnyElement> = if self.collection_has_more {
                    let next = self.collection_page + 1;
                    vec![div()
                        .flex()
                        .w_full()
                        .justify_center()
                        .py(px(palette::CARD_TEXT_ROW_GAP))
                        .child(
                            Button::new("space-collection-more")
                                .label("再加载一页")
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    this.load_collections(next, cx);
                                })),
                        )
                        .into_any_element()]
                } else {
                    Vec::new()
                };
                let list = div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .gap(px(palette::LIST_GAP))
                    .px(px(palette::CARD_TEXT_PADDING_X))
                    .py(px(palette::BODY_PADDING_Y))
                    .children(rows)
                    .children(more);
                self.with_note(list, self.collection_note.clone())
            }
        }
    }

    /// 有提示就先把提示摆上（错误必须看得见，第 4.7 节），再接着摆内容。
    fn with_note(&self, content: impl IntoElement, note: SharedString) -> AnyElement {
        if note.is_empty() {
            return content.into_any_element();
        }
        div()
            .flex()
            .flex_col()
            .w_full()
            .child(
                div()
                    .w_full()
                    .px(px(palette::CARD_TEXT_PADDING_X))
                    .pt(px(palette::BODY_PADDING_Y))
                    .text_size(px(palette::FONT_BODY))
                    .text_color(palette::text_secondary())
                    .child(note),
            )
            .child(content)
            .into_any_element()
    }
}

/// 空间页里的卡片也要能进详情页，发的是同一个事件。
impl EventEmitter<OpenVideo> for UserSpacePage {}
/// 从空间页再点进另一个人的空间（相关推荐这类位置以后会有），也走同一个信号。
impl EventEmitter<OpenUser> for UserSpacePage {}
/// 点"合集与系列"那一格 = 请外壳压一层内容页（P12）。
impl EventEmitter<OpenCollection> for UserSpacePage {}

impl Render for UserSpacePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 每帧登记一次移动/抬起监听（`Window::on_mouse_event` 只覆盖下一帧，见 `ui/scroll.rs`）。
        self.scroll.watch(window, cx.entity_id());
        let controls = match self.section {
            SpaceSection::Videos => div()
                .flex()
                .flex_col()
                .w_full()
                .child(self.filter_bar(cx))
                .child(self.video_controls(cx))
                .into_any_element(),
            // 图文/收藏/合集这三格没有排序与筛选可给：接口不提供这些参数，
            // 摆一颗点了没反应的按钮比不摆更糟（QODER.md"不做显示了但不生效的假开关"）。
            SpaceSection::Posts | SpaceSection::Folders | SpaceSection::Collections => {
                div().into_any_element()
            }
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(layout::header("用户空间", Some(self.name.clone()), None))
            .child(self.profile_card())
            .child(self.section_bar(cx))
            .child(controls)
            .child(layout::divider())
            .child(
                div()
                    .id("space-body")
                    .flex_1()
                    .w_full()
                    .min_w(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(self.scroll.handle())
                    .on_mouse_down(MouseButton::Left, self.scroll.on_press())
                    .child(self.body(cx)),
            )
    }
}
