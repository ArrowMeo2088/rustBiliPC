//! 设置页（这一版的内容是"关于本机"）。
//!
//! ## 为什么没有开关
//!
//! 2026-10-26 用户定稿：设置这一格**先只做关于页**（他的原话选项："先只做关于页"）。
//! 所以这页只读：显示版本、当前账户、几个文件的位置与真正生效的那个设置值。
//! **不做显示了但不生效的假开关**——`config.json` 里的 `max_quality` 是真被起播那条路读走的
//! （见 [`crate::api::stream::request_qn`]），要改就手改文件，重启生效，这一栏把当前值如实写出来。
//!
//! ## 侧栏左下角那行 mpv 自检搬到了这里
//!
//! 他把侧栏那行删了（"左下角的mpv信息也删除"），但"mpv 到底找没找到"这件事在播不出来的时候
//! 还是要看——所以它出现在这里，而不是丢掉。同理，凭据与配置的路径也写出来：
//! 明文落盘是定稿的选择，出问题第一步就是去看那两个文件。
//!
//! ## 账户那一栏是唯一一路网络请求
//!
//! 用 [`profile::fetch_profile`]（与 P2b 时期同一份），失败只把原因写在页面上；
//! 其余行全是本机信息，网络断了也照样显示。

use std::sync::Arc;

use gpui_kit::component::button::Button;
use gpui_kit::*;

use crate::api::profile::{self, ProfileSummary};
use crate::pages::loading;
use crate::store::auth::Auth;
use crate::store::config;
use crate::ui::{layout, palette};

/// 设置页根视图。
pub struct AboutPage {
    client: Arc<bpi_rs::BpiClient>,
    /// 凭据里的 mid（字符串）。
    mid_raw: String,
    account: Option<ProfileSummary>,
    note: SharedString,
    seq: u32,
    loaded: bool,
}

impl AboutPage {
    /// 建页但不发请求（懒加载）。
    pub fn new(client: Arc<bpi_rs::BpiClient>, mid_raw: String) -> Self {
        Self {
            client,
            mid_raw,
            account: None,
            note: "".into(),
            seq: 0,
            loaded: false,
        }
    }

    /// 第一次被看到时取一次账户资料。
    pub fn ensure_loaded(&mut self, cx: &mut Context<Self>) {
        if self.loaded {
            return;
        }
        self.reload(cx);
    }

    /// 重新取账户资料（本机那几行不用重载）。
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.loaded = true;
        self.seq += 1;
        let seq = self.seq;
        self.note = "账户资料加载中…".into();
        cx.notify();

        let Ok(mid) = self.mid_raw.trim().parse::<u64>() else {
            log::error!("设置页：凭据里的 mid 不是数字: {:?}", self.mid_raw);
            self.note = "凭据里的用户 ID 无法解析，请重新扫码登录".into();
            return;
        };
        let join = profile::fetch_profile(Arc::clone(&self.client), mid);
        loading::spawn_into(cx, seq, "账户资料", join, |page| page.seq, |page, outcome| {
            match outcome {
                Ok(summary) => {
                    page.note = "".into();
                    page.account = Some(summary);
                }
                Err(message) => {
                    page.account = None;
                    page.note = format!("取账户资料失败：{message}").into();
                }
            }
        });
    }

    /// 本机那几行：路径与真正生效的设置值。
    fn machine_rows(&self) -> Vec<AnyElement> {
        let mpv = match crate::player::find_mpv() {
            Some(path) => format!("{} · 已找到", path.display()),
            None => format!("{} · 没找到 mpv.exe", crate::player::mpv_path().display()),
        };
        let quality = config::quality_name(crate::app::config().max_quality);
        let rows: Vec<(String, String)> = vec![
            ("版本".to_string(), crate::VERSION.to_string()),
            ("凭据文件".to_string(), Auth::path().display().to_string()),
            ("配置文件".to_string(), config::Config::path().display().to_string()),
            ("画质上限".to_string(), quality),
            ("播放内核".to_string(), mpv),
            (
                "日志文件".to_string(),
                crate::logging::log_path().display().to_string(),
            ),
        ];
        rows.into_iter()
            .map(|(label, meta)| layout::list_row(label, meta))
            .collect()
    }

    /// 账户那两行（昵称/UID 与等级/会员）。加载中或失败时用一句提示占住位置。
    fn account_rows(&self) -> Vec<AnyElement> {
        match self.account.as_ref() {
            Some(account) => vec![
                layout::list_row(
                    "当前账户",
                    format!("{} · UID {}", account.name, account.mid),
                ),
                layout::list_row(
                    "等级与会员",
                    format!(
                        "Lv{}{} · 粉丝 {} · 关注 {}",
                        account.level,
                        if account.vip_label.is_empty() {
                            String::new()
                        } else {
                            format!(" · {}", account.vip_label)
                        },
                        account.follower,
                        account.following
                    ),
                ),
            ],
            None => vec![layout::list_row(
                "当前账户",
                if self.note.is_empty() {
                    "没拿到账户资料"
                } else {
                    self.note.as_ref()
                },
            )],
        }
    }

    fn reload_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("about-reload")
            .label("重新取账户资料")
            .on_click(cx.listener(|this, _event, _window, cx| this.reload(cx)))
            .into_any_element()
    }
}

impl Render for AboutPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self
            .account_rows()
            .into_iter()
            .chain(self.machine_rows())
            .collect::<Vec<AnyElement>>();

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(layout::header("设置", None, Some(self.reload_button(cx))))
            .child(layout::divider())
            .child(
                div()
                    .id("about-body")
                    .flex_1()
                    .w_full()
                    .min_w(px(0.))
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w_full()
                            .gap(px(palette::LIST_GAP))
                            .px(px(palette::CARD_TEXT_PADDING_X))
                            .py(px(palette::BODY_PADDING_Y))
                            .children(rows),
                    ),
            )
    }
}
