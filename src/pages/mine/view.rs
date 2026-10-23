//! 个人中心页（阶段二 P2b）。
//!
//! 三块内容一起呈现：**头部资料**（昵称/头像/等级/签名/关注粉丝）、**收藏夹列表**、
//! **最近观看历史**。它们来自三个独立接口（`api/profile.rs`、`api/favorites.rs`、
//! `api/history.rs`），所以按三路并发拉取、各写各的槽位：任何一路失败只显示在自己那一块，
//! 不会把整页变成空白（第 4.7 节"失败必须可见"的落地形式）。
//!
//! 两条硬规矩在这里同样适用（第 4.9 节）：
//! - 每个 `cx.spawn` 末尾 `.detach()`，否则任务被 drop 即取消；
//! - 用 `seq` 序号判定"这次结果还是不是最新一次请求的"，旧任务自己让位，
//!   绝不在任务自己的 future 里去丢弃它的 `Task`（那会主线程死锁）。

use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;

use crate::api::favorites::{self, FavFolder};
use crate::api::history::{self, HistoryItem};
use crate::api::profile::{self, ProfileSummary};
use crate::ui::video_card::{self, VideoCardData};

/// 列表区显示哪一块。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MineTab {
    /// 收藏夹。
    Favorites,
    /// 观看历史。
    History,
}

impl MineTab {
    fn label(self) -> &'static str {
        match self {
            Self::Favorites => "我的收藏夹",
            Self::History => "最近观看",
        }
    }
}

/// 历史条目数量。
const HISTORY_LIMIT: u32 = 20;

/// 头像边长（逻辑像素）。
const AVATAR_PIXELS: f32 = 56.;

/// 向图床要的头像素尺寸（逻辑 56 × 2 倍缩放上限）。
const AVATAR_DEVICE_PIXELS: u32 = 112;

/// 个人中心页根视图。
pub struct MinePage {
    client: Arc<bpi_rs::BpiClient>,
    /// 凭据里的 `DedeUserID`。资料与收藏接口都要求显式传自己的 mid（bpi-rs 没有 "mine" 变体）。
    mid_raw: String,
    profile: Option<ProfileSummary>,
    folders: Vec<FavFolder>,
    history: Vec<HistoryItem>,
    tab: MineTab,
    /// 三块各自的提示：加载中 / 空 / 失败原因。空字符串表示无提示。
    profile_note: SharedString,
    folders_note: SharedString,
    history_note: SharedString,
    /// 每次 `reload` 自增；旧任务发现自己不是最新一代就丢弃结果。
    seq: u32,
    /// 有没有真的加载过。外壳第一次切到"我的"时才发请求（见 `pages/shell/view.rs`）。
    loaded: bool,
}

impl MinePage {
    /// 建页，但**不**立刻发请求：这一页随外壳一起构造，而用户可能根本不点"我的"，
    /// 一上来就打三个接口既浪费，又可能在首屏多出三条与本意无关的错误行。
    /// 真正的首次加载由外壳调 [`MinePage::ensure_loaded`] 触发。
    pub fn new(client: Arc<bpi_rs::BpiClient>, mid_raw: String) -> Self {
        Self {
            client,
            mid_raw,
            profile: None,
            folders: Vec::new(),
            history: Vec::new(),
            tab: MineTab::Favorites,
            profile_note: "".into(),
            folders_note: "".into(),
            history_note: "".into(),
            seq: 0,
            loaded: false,
        }
    }

    /// 第一次被看到时加载一次；之后保持已有数据，要更新用"重新加载"按钮。
    pub fn ensure_loaded(&mut self, cx: &mut Context<Self>) {
        if self.loaded {
            return;
        }
        self.reload(cx);
    }

    /// 三路并发重新加载。
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.loaded = true;
        self.seq += 1;
        let seq = self.seq;
        self.profile_note = "资料加载中…".into();
        self.folders_note = "收藏夹加载中…".into();
        self.history_note = "历史加载中…".into();
        cx.notify();

        // mid 来自凭据；不是数字说明凭据本身坏了，就地说明，一路请求都不发。
        let Ok(mid) = self.mid_raw.trim().parse::<u64>() else {
            log::error!("个人中心：凭据里的 mid 不是数字: {:?}", self.mid_raw);
            let note: SharedString = "凭据里的用户 ID 无法解析，请重新扫码登录".into();
            self.profile_note = note.clone();
            self.folders_note = note.clone();
            self.history_note = note;
            return;
        };

        // 先取一份 client 克隆，避免三次调用互相抢 self 的借用。
        let client = Arc::clone(&self.client);
        self.spawn_load(
            seq,
            profile::fetch_profile(Arc::clone(&client), mid),
            "个人资料",
            cx,
            |page, outcome| match outcome {
                Ok(summary) => {
                    page.profile_note = "".into();
                    page.profile = Some(summary);
                }
                Err(message) => page.profile_note = message.into(),
            },
        );
        self.spawn_load(
            seq,
            favorites::fetch_folders(Arc::clone(&client), mid),
            "收藏夹",
            cx,
            |page, outcome| match outcome {
                Ok(list) => {
                    page.folders_note = if list.is_empty() { "还没有收藏夹".into() } else { "".into() };
                    page.folders = list;
                }
                Err(message) => page.folders_note = message.into(),
            },
        );
        self.spawn_load(
            seq,
            history::fetch_history(Arc::clone(&client), HISTORY_LIMIT),
            "观看历史",
            cx,
            |page, outcome| match outcome {
                Ok(list) => {
                    page.history_note = if list.is_empty() { "还没有观看记录".into() } else { "".into() };
                    page.history = list;
                }
                Err(message) => page.history_note = message.into(),
            },
        );
    }

    /// 起一路加载任务。`label` 只用于日志，`write` 决定结果写进哪个槽位。
    ///
    /// `T: 'static` 不是可选项：任务要跨 `await` 持有 `JoinHandle<BpiResult<T>>`，
    /// gpui 的 spawn 要求整个 future 是 `'static`（CI 实测 E0310 就指这一行）。
    fn spawn_load<T: 'static>(
        &mut self,
        seq: u32,
        join: tokio::task::JoinHandle<bpi_rs::BpiResult<T>>,
        label: &'static str,
        cx: &mut Context<Self>,
        write: impl FnOnce(&mut Self, Result<T, String>) + 'static,
    ) {
        cx.spawn(async move |this, cx| {
            let outcome = match join.await {
                Ok(inner) => inner.map_err(|error| error.to_string()),
                Err(join_error) => Err(crate::app::describe_join_error(join_error)),
            };
            log::debug!("个人中心：{label} 结果已回到前台（请求 #{seq}）");

            let Some(view) = this.upgrade() else {
                log::warn!("个人中心：等待期间实体已释放，丢弃 {label} 结果");
                return;
            };
            view.update(cx, |view, cx| {
                if view.seq != seq {
                    log::debug!("个人中心：请求 #{seq} 已被取代，丢弃 {label} 结果");
                    return;
                }
                write(view, outcome);
                cx.notify();
            });
        })
        .detach();
    }

    /// 头部：头像 + 昵称 + 等级/会员 + 签名 + 关注粉丝。
    fn header(&self) -> impl IntoElement {
        let avatar_box = div()
            .size(px(AVATAR_PIXELS))
            .flex_shrink_0()
            .overflow_hidden()
            .bg(rgb(0x2A2A2A));
        // 和封面同一套处理：补协议 + 按这个 56px 的框要 2× 的图（头像原图实测 812KB）。
        let avatar = match self.profile.as_ref() {
            Some(profile) if !profile.face.is_empty() => avatar_box.child(
                img(SharedString::from(crate::ui::cover::for_display(
                    &profile.face,
                    AVATAR_DEVICE_PIXELS,
                    AVATAR_DEVICE_PIXELS,
                )))
                .size_full()
                .object_fit(ObjectFit::Cover)
                .with_fallback(|| div().size_full().bg(rgb(0x1A1A1A)).into_any_element()),
            )
            .into_any_element(),
            _ => avatar_box.into_any_element(),
        };

        let mut column = div().flex().flex_col().flex_1().gap_1();
        match self.profile.as_ref() {
            Some(profile) => {
                column = column
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .items_center()
                            .child(div().text_base().child(profile.name.clone()))
                            .child(div().text_sm().child(format!("Lv{}", profile.level)))
                            .child(div().text_sm().child(profile.vip_label.clone())),
                    )
                    .child(div().text_sm().child(format!(
                        "关注 {} · 粉丝 {}",
                        profile.following, profile.follower
                    )))
                    .child(div().text_sm().child(profile.sign.clone()));
            }
            None => {
                column = column
                    .child(div().text_base().child("未取到账号资料"))
                    .child(div().text_sm().child(self.profile_note.clone()));
            }
        }
        // 资料刷新失败时，即使旧数据还在也要把原因显示出来（错误不许只进日志）。
        if self.profile.is_some() && !self.profile_note.is_empty() {
            column = column.child(div().text_sm().child(self.profile_note.clone()));
        }

        div()
            .flex()
            .gap_3()
            .p_4()
            .items_center()
            // avatar 自己带尺寸与裁剪，这里不再套一层同尺寸的框。
            .child(avatar)
            .child(column)
    }

    /// 列表区切换按钮。
    fn tab_button(&self, tab: MineTab, cx: &mut Context<Self>) -> impl IntoElement {
        let button = Button::new(format!("mine-tab-{}", tab.label()))
            .label(tab.label())
            .on_click(cx.listener(move |this, _, _window, cx| {
                if this.tab != tab {
                    this.tab = tab;
                    cx.notify();
                }
            }));
        if self.tab == tab { button.primary() } else { button }
    }

    /// 收藏夹列表。
    fn folders_body(&self) -> AnyElement {
        if !self.folders_note.is_empty() {
            return div().p_4().text_sm().child(self.folders_note.clone()).into_any_element();
        }

        div()
            .flex()
            .flex_col()
            .w_full()
            .children(self.folders.iter().map(|folder| {
                let kind = if folder.created { "我创建" } else { "我收藏" };
                let upper = if folder.upper.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", folder.upper)
                };
                div()
                    .flex()
                    .w_full()
                    .gap_2()
                    .px_4()
                    .p_2()
                    .items_center()
                    .child(div().flex_1().child(folder.title.clone()))
                    .child(div().text_sm().child(format!(
                        "{kind} · {} 个内容{upper}",
                        folder.media_count
                    )))
            }))
            .into_any_element()
    }

    /// 历史列表：复用搜索页那套卡片（字段语义一致，省一遍排版）。
    fn history_body(&self) -> AnyElement {
        if !self.history_note.is_empty() {
            return div().p_4().text_sm().child(self.history_note.clone()).into_any_element();
        }

        let cards = self
            .history
            .iter()
            .map(|item| VideoCardData {
                bvid: item.bvid.clone().into(),
                title: item.title.clone().into(),
                author: if item.tag.is_empty() {
                    item.author.clone().into()
                } else {
                    format!("{} · {}", item.author, item.tag).into()
                },
                duration: item.duration_text.clone().into(),
                // 卡片右下角那一栏在历史里放"看到 xx:xx"，比播放量更贴切。
                play_text: item.progress_text.clone().into(),
                cover_url: item.cover.clone().into(),
            })
            .collect::<Vec<_>>();

        div()
            .flex()
            .flex_col()
            .w_full()
            .children(cards.into_iter().map(video_card::render))
            .into_any_element()
    }
}

impl Render for MinePage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.tab {
            MineTab::Favorites => self.folders_body(),
            MineTab::History => self.history_body(),
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.header())
            .child(
                div()
                    .flex()
                    .gap_1()
                    .p_4()
                    .items_center()
                    .child(self.tab_button(MineTab::Favorites, cx))
                    .child(self.tab_button(MineTab::History, cx))
                    .child(div().flex_1())
                    .child(
                        Button::new("mine-reload")
                            .label("重新加载")
                            .on_click(cx.listener(|this, _, _window, cx| this.reload(cx))),
                    ),
            )
            .child(
                div()
                    .id("mine-body")
                    .flex_1()
                    .w_full()
                    .overflow_y_scroll()
                    .child(body),
            )
    }
}
