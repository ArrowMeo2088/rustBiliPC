//! 收藏页：我创建的 + 我收藏的 收藏夹列表，**点一行进去看内容**（P9）。
//!
//! ## 形状变了两次
//!
//! - P2b：收藏与历史挤在同一页的两个 tab。
//! - P8（2026-10-26 上午）：拆成两个平级入口，当时用户定稿"只列收藏夹，不下钻"。
//! - P9（同一天下午）：同一句需求里补了"收藏夹要能展开、搜索、点击进入视频"，
//!   并把"展开"定成**下钻一层页**（他给的选项原话："下钻一层页（推荐）"，理由是与现有
//!   下钻栈与"返回"一致）。所以下钻是这一页现在的职责，`FavFolder.id` 就是为它留的。
//!
//! ## 这一页的搜索是**本地过滤**
//!
//! 匹配夹子标题与创建者昵称（[`matches_keyword`](crate::pages::matches_keyword)）。
//! 这里不做服务端搜索：收藏夹列表那条接口（`created_list`/`collected_list`）没有 keyword 参数，
//! 而"内容里搜视频"是**下一层页**的事——那一条接口有 `keyword`，见 `folder.rs`。
//!
//! ## 行是交互元素，所以用 `clickable_row`
//!
//! 上一版用 `layout::list_row`（不可点）。两个复用件共用同一组度量常量，
//! 差别只在要不要 `id`/`on_click`/hover（gpui 的 hover 只存在于 `Stateful<Div>`，必须先 `.id()`）。

use std::sync::Arc;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use crate::api::favorites::{self, FavFolder};
use crate::pages::{OpenFolder, loading, matches_keyword};
use crate::ui::{layout, palette};

/// 收藏页根视图。
pub struct FavoritesPage {
    client: Arc<bpi_rs::BpiClient>,
    /// 凭据里的 mid（字符串形式）。
    mid_raw: String,
    folders: Vec<FavFolder>,
    /// 筛选输入框的状态（本地过滤，`InputEvent::Change` 实时更新 [`filter`](Self::filter)）。
    input: Entity<InputState>,
    /// 已经落到字段里的关键词。为什么不每次渲染时现读 `InputState`：
    /// 渲染里读状态会让"输入"这件事把整页标记为脏，本地过滤还是显式存一份更清楚。
    filter: SharedString,
    note: SharedString,
    seq: u32,
    loaded: bool,
    _subscriptions: Vec<Subscription>,
}

impl FavoritesPage {
    /// 建页但不发请求：随外壳一起构造，第一次被看到才取（懒加载）。
    /// 要 `window` 是因为 `InputState::new` 与 `subscribe_in` 都要它。
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        client: Arc<bpi_rs::BpiClient>,
        mid_raw: String,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("按夹子名或创建者筛选")
        });
        let subscriptions = vec![cx.subscribe_in(
            &input,
            window,
            |this, state, event: &InputEvent, _window, cx| {
                // 本地过滤走 Change：这条不花网络，敲一个字少几条是用户预期的反馈速度。
                // （服务端搜索那一层在收藏夹**内容**页，那里是回车才请求。）
                if matches!(event, InputEvent::Change) {
                    this.filter = state.read(cx).value().trim().to_string().into();
                    cx.notify();
                }
            },
        )];

        Self {
            client,
            mid_raw,
            folders: Vec::new(),
            input,
            filter: "".into(),
            note: "".into(),
            seq: 0,
            loaded: false,
            _subscriptions: subscriptions,
        }
    }

    /// 第一次被看到时取一次。
    pub fn ensure_loaded(&mut self, cx: &mut Context<Self>) {
        if self.loaded {
            return;
        }
        self.reload(cx);
    }

    /// 重新拉收藏夹列表。
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.loaded = true;
        self.seq += 1;
        let seq = self.seq;
        self.note = "收藏夹加载中…".into();
        cx.notify();

        let Ok(mid) = self.mid_raw.trim().parse::<u64>() else {
            log::error!("收藏页：凭据里的 mid 不是数字: {:?}", self.mid_raw);
            self.note = "凭据里的用户 ID 无法解析，请重新扫码登录".into();
            return;
        };
        let join = favorites::fetch_folders(Arc::clone(&self.client), mid);
        loading::spawn_into(cx, seq, "收藏夹", join, |page| page.seq, |page, outcome| {
            match outcome {
                Ok(list) => {
                    page.note = if list.is_empty() {
                        "还没有公开的收藏夹".into()
                    } else {
                        "".into()
                    };
                    page.folders = list;
                }
                Err(message) => {
                    page.folders.clear();
                    page.note = format!("取收藏夹失败：{message}").into();
                }
            }
        });
    }

    /// 页头副标题：筛选时说明筛掉了多少，不让人以为夹子变少了。
    fn summary(&self) -> String {
        if self.folders.is_empty() {
            return String::new();
        }
        let shown = self.shown().len();
        if self.filter.is_empty() {
            format!("{} 个收藏夹", self.folders.len())
        } else {
            format!("筛出 {shown} / {} 个收藏夹", self.folders.len())
        }
    }

    /// 当前要列出来的那些夹子（本地过滤后的结果）。
    fn shown(&self) -> Vec<&FavFolder> {
        self.folders
            .iter()
            .filter(|folder| matches_keyword(&folder.title, &folder.upper, self.filter.as_str()))
            .collect()
    }

    fn reload_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("favorites-reload")
            .label("重新加载")
            .on_click(cx.listener(|this, _event, _window, cx| this.reload(cx)))
            .into_any_element()
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(palette::LIST_GAP))
            .px(px(palette::CARD_TEXT_PADDING_X))
            .py(px(palette::BODY_PADDING_Y));
        if !self.note.is_empty() {
            list = list.child(
                div()
                    .w_full()
                    .text_size(px(palette::FONT_BODY))
                    .text_color(palette::text_secondary())
                    .child(self.note.clone()),
            );
        }
        if self.filter_enabled_but_empty() {
            list = list.child(
                div()
                    .w_full()
                    .text_size(px(palette::FONT_CAPTION))
                    .text_color(palette::text_tertiary())
                    .child("这些关键词没命中任何收藏夹。"),
            );
        }
        list.children(self.shown().into_iter().map(|folder| {
            // "我创建/我收藏"：这一页是看自己的夹子，与空间页那行"TA 创建/TA 收藏"分开。
            let kind = if folder.created { "我创建" } else { "我收藏" };
            let upper = if folder.upper.is_empty() {
                String::new()
            } else {
                format!(" · {}", folder.upper)
            };
            let id = folder.id;
            let title = folder.title.clone();
            layout::clickable_row(
                format!("fav-folder-{id}"),
                folder.title.clone(),
                format!("{kind} · {} 个内容{upper}", folder.media_count),
                cx.listener(move |_this, _event, _window, cx| {
                    cx.emit(OpenFolder {
                        id,
                        title: title.clone(),
                    });
                }),
            )
        }))
        .into_any_element()
    }

    /// 筛得一条不剩时给一句话（不算错误，但空列表不解释会被当成"夹子没了"）。
    fn filter_enabled_but_empty(&self) -> bool {
        !self.filter.is_empty() && !self.folders.is_empty() && self.shown().is_empty()
    }
}

/// 点一行 = 请外壳压一层这个收藏夹的内容页（P9）。
impl EventEmitter<OpenFolder> for FavoritesPage {}

impl Render for FavoritesPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let summary = self.summary();
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(layout::header(
                "收藏",
                (!summary.is_empty()).then(|| summary.into()),
                Some(self.reload_button(cx)),
            ))
            .child(layout::search_box(&self.input))
            .child(layout::divider())
            .child(
                div()
                    .id("favorites-body")
                    .flex_1()
                    .w_full()
                    .min_w(px(0.))
                    .overflow_y_scroll()
                    .child(self.body(cx)),
            )
    }
}
