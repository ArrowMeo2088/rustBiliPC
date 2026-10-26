//! 合集/系列内容页（P12）：从空间页那一格点进来的一列分集。
//!
//! ## 这一页与"收藏夹内容页"的三处不同
//!
//! | | 收藏夹内容（P9） | 这一页（P12） |
//! | --- | --- | --- |
//! | 分页 | 页码 + `has_more`（接口自己给布尔） | 页码 + `meta.total` / `page.total`（**接口不给布尔**，"还有下一页"是我们按已加载条数算的） |
//! | 搜索 | **有服务端 `keyword`** | **没有**（`seasons_archives_list` 与 `series_archives` 的参数表里都没有关键词，见 `ref/bpi-rs/src/video/collection/params.rs:24-76/182-249`）→ 这一页**不放搜索框**，也不做"只搜了本页"的假输入框（QODER.md"接口没给的能力不编"） |
//! | 一集的信息 | 有 `page`（分 P 数） | `Archive` **没有 `videos` 字段**，所以这里不标"这一集有几个 P"；想知道只能点进去（那一集自己的 `view` 会给），或在详情页的分集面板里看（那里的 `arc.videos` 有，见 [`crate::api::detail`]） |
//!
//! ## 序号为什么自己数
//!
//! 接口给的 `Archive` 里没有"这是合集第几集"的字段（`order`/`index` 都没有），
//! 而我们**按已加载顺序累加编号**——合集列表本身就是接口给的顺序，
//! 翻页只是往后接，所以这个编号与 B 站网页那一列对得上。
//! 真出现对不上的稿件（例如合集里有分段），编号只当"列表里的第几行"读，
//! 不当"这一集在原稿里的集号"。

use std::sync::Arc;

use gpui_kit::component::button::Button;
use gpui_kit::*;

use crate::api::collection::{self, CollectionEpisode, CollectionKind};
use crate::pages::OpenVideo;
use crate::ui::scroll::{should_auto_page, DragScroll};
use crate::ui::{layout, palette};

/// 合集/系列内容页根视图。
pub struct CollectionContentPage {
    client: Arc<bpi_rs::BpiClient>,
    /// 两条接口都要 `mid`。
    mid: u64,
    kind: CollectionKind,
    /// 合集的 `season_id` 或系列的 `series_id`。
    id: u64,
    title: SharedString,
    /// 简介。系列那一路接口不给，留空就不画。
    desc: SharedString,
    items: Vec<CollectionEpisode>,
    total: Option<u64>,
    page: u64,
    has_more: bool,
    note: SharedString,
    seq: u32,
    loaded: bool,
    /// 有没有在途请求（自动连续翻页的第二道闸，见 [`should_auto_page`]）。
    loading: bool,
    scroll: DragScroll,
}

impl CollectionContentPage {
    /// 建页。`title` 是从空间页带过来的名字（系列那一路接口不给名字，先靠它占住页头）。
    pub fn new(
        client: Arc<bpi_rs::BpiClient>,
        mid: u64,
        kind: CollectionKind,
        id: u64,
        title: String,
    ) -> Self {
        Self {
            client,
            mid,
            kind,
            id,
            title: title.into(),
            desc: "".into(),
            items: Vec::new(),
            total: None,
            page: 0,
            has_more: false,
            note: "加载中…".into(),
            seq: 0,
            loaded: false,
            loading: false,
            scroll: DragScroll::default(),
        }
    }

    /// 第一次被看到时取第 1 页（这一页是点进来的，不该先给一片空白）。
    pub fn ensure_loaded(&mut self, cx: &mut Context<Self>) {
        if self.loaded {
            return;
        }
        self.request(1, cx);
    }

    /// 取某一页。`page == 1` 是替换，大于 1 是往后**累加**。
    fn request(&mut self, page: u64, cx: &mut Context<Self>) {
        self.loaded = true;
        self.loading = true;
        self.seq += 1;
        let seq = self.seq;
        self.note = if page == 1 {
            "加载中…".into()
        } else {
            "正在加载下一页…".into()
        };
        cx.notify();

        let join = match self.kind {
            CollectionKind::Season => collection::fetch_season_archives(
                Arc::clone(&self.client),
                self.mid,
                self.id,
                page,
            ),
            CollectionKind::Series => collection::fetch_series_archives(
                Arc::clone(&self.client),
                self.mid,
                self.id,
                self.title.to_string(),
                page,
            ),
        };

        crate::pages::loading::spawn_into(
            cx,
            seq,
            self.kind.label(),
            join,
            |page_state| page_state.seq,
            // `move` 不能省：这个闭包要跨 `await` 活下来（`'static`），
            // 而它外面那个 `page` 是栈上的局部值——不搬进来就是 E0373。
            move |page_state, outcome| {
                // 到站了（成功或失败都算）：放开自动翻页那道闸。
                page_state.loading = false;
                match outcome {
                    Ok(batch) => {
                        // 先把"已加载多少"与两个判据算出来再搬 `batch` 的字段：
                        // `has_more`/`total_known` 借的是整个 `batch`，而下面要把 `batch.items` 搬走
                        // （部分移动之后再 `&batch` 就是 E0382）。
                        let loaded = if batch.page <= 1 {
                            batch.items.len() as u64
                        } else {
                            page_state.items.len() as u64 + batch.items.len() as u64
                        };
                        let more = batch.has_more(loaded);
                        // 存**校验过的**总数：比已加载还小的那个数不是条目数，界面就不该拿它说"共 N 集"。
                        let known_total = batch.total_known(loaded);
                        let title = batch.title;
                        let desc = batch.desc;
    
                        // 名字以接口为准：合集会给 `meta.name`，系列那一路是我们自己带进去的。
                        if !title.is_empty() {
                            page_state.title = title.into();
                        }
                        if !desc.is_empty() {
                            page_state.desc = desc.into();
                        }
                        page_state.total = known_total;
                        page_state.page = batch.page;
                        if batch.page <= 1 {
                            page_state.items = batch.items;
                        } else {
                            page_state.items.extend(batch.items);
                        }
                        page_state.has_more = more;
                        page_state.note = if page_state.items.is_empty() {
                            format!("这个{}里还没有能看的稿件", page_state.kind.label()).into()
                        } else {
                            "".into()
                        };
                    }
                    Err(message) => {
                        // 只有第一页失败才清空：翻下一页失败时把已加载的留着，比抹成空白有用。
                        if page == 1 {
                            page_state.items.clear();
                        }
                        page_state.note = format!("取内容失败：{message}").into();
                    }
                }
            },
        );
    }

    /// 页头副标题：类型 + 总数/已加载。总数缺失时只说已加载，不编一个分母。
    fn summary(&self) -> String {
        if self.items.is_empty() {
            return self.kind.label().to_string();
        }
        match self.total {
            Some(total) => format!(
                "{} · 共 {total} 集 · 已加载 {}",
                self.kind.label(),
                self.items.len()
            ),
            None => format!("{} · 已加载 {}", self.kind.label(), self.items.len()),
        }
    }

    fn load_more_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.has_more {
            return None;
        }
        let next = self.page + 1;
        Some(
            div()
                .flex()
                .w_full()
                .justify_center()
                .py(px(palette::CARD_TEXT_ROW_GAP))
                .child(
                    Button::new("collection-more-btn")
                        .label("再加载一页")
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.request(next, cx);
                        })),
                )
                .into_any_element(),
        )
    }

    /// 一列分集。行号是"已加载列表里的第几行"（见文件头那条口径说明）。
    fn rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let meta = {
                    let mut parts: Vec<String> =
                        vec![format!("{:02}", index + 1), item.duration_text.clone()];
                    if !item.play_text.is_empty() {
                        parts.push(item.play_text.clone());
                    }
                    if !item.date_text.is_empty() {
                        parts.push(item.date_text.clone());
                    }
                    parts.join(" · ")
                };
                let bvid = item.bvid.clone();
                // 参数直接给 `String`：`clickable_row` 那两个位置是 `impl Into<SharedString>`，
                // 在这里再写一次 `.into()` 会让目标类型推不出来（E0282）。
                layout::clickable_row(
                    format!("collection-{}", item.bvid),
                    item.title.clone(),
                    meta,
                    cx.listener(move |_this, _event, _window, cx| {
                        cx.emit(OpenVideo { bvid: bvid.clone() });
                    }),
                )
            })
            .collect()
    }
}

/// 点一行 = 请外壳压一层详情页（与卡片同一条路）。
impl EventEmitter<OpenVideo> for CollectionContentPage {}

impl Render for CollectionContentPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.scroll.watch(window, cx.entity_id());
        // 滚到底就自动接下一页（P13）。这一页的"还有下一页"是我们按已加载条数算的
        // （接口不给布尔），判据四条见 [`should_auto_page`]。
        if should_auto_page(&self.scroll, self.has_more, self.items.len(), self.loading) {
            let next = self.page + 1;
            log::debug!("合集内容页：滚到底，自动取第 {next} 页");
            self.request(next, cx);
        }

        let mut lines: Vec<AnyElement> = Vec::new();
        if !self.desc.is_empty() {
            lines.push(
                div()
                    .w_full()
                    .px(px(palette::CARD_TEXT_PADDING_X))
                    .py(px(palette::CARD_TEXT_ROW_GAP))
                    .text_size(px(palette::FONT_CAPTION))
                    .line_height(px(palette::LINE_HEIGHT_BODY))
                    .text_color(palette::text_secondary())
                    .child(self.desc.clone())
                    .into_any_element(),
            );
        }
        if !self.note.is_empty() {
            lines.push(
                div()
                    .w_full()
                    .px(px(palette::CARD_TEXT_PADDING_X))
                    .py(px(palette::CARD_TEXT_ROW_GAP))
                    .text_size(px(palette::FONT_BODY))
                    .text_color(if self.items.is_empty() {
                        palette::text_secondary()
                    } else {
                        palette::critical()
                    })
                    .child(self.note.clone())
                    .into_any_element(),
            );
        }
        lines.extend(self.rows(cx));
        if let Some(button) = self.load_more_button(cx) {
            lines.push(button);
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(layout::header(self.title.clone(), Some(self.summary().into()), None))
            .child(
                div()
                    .id("collection-body")
                    .flex_1()
                    .w_full()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(self.scroll.handle())
                    .on_mouse_down(MouseButton::Left, self.scroll.on_press())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w_full()
                            .gap(px(palette::LIST_GAP))
                            .px(px(palette::CARD_TEXT_PADDING_X))
                            .py(px(palette::BODY_PADDING_Y))
                            .children(lines),
                    ),
            )
    }
}
