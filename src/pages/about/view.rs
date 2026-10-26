//! 设置页。P13（2026-10-26）起它是**真设置页**：四项能改、改完立刻落盘。
//!
//! ## 四项与它们的读取方
//!
//! 他要的原话："设置内增加配置 MPV播放器位置（默认在 PATH，支持打开资源管理器选择 mpv.exe）、
//! 默认播放画质（1080P、1080P 60帧、2K、4K）、默认播放编码（HEVC、AVC、AV1）、
//! 调试模式（默认关闭，关闭则启动时不显示控制台，也不产生 debug.log，不进行日志输出）"。
//!
//! 每一项都必须有真的读取方，否则就是假开关（QODER.md 红线）：
//! - 画质 → [`app::config`](crate::app::config)`.max_quality` → 请求 `qn` 与选流封顶
//!   （[`stream::request_qn`](crate::api::stream::request_qn)）；
//! - 编码 → `codec` → 同档内的选流优先级（`api::stream_select::codec_score`）；
//! - mpv 位置 → `mpv_path` → [`player::resolve_mpv`](crate::player::resolve_mpv)；
//! - 调试模式 → `debug_log` → [`logging::init`](crate::logging::init)（只在启动时读一次，见下）。
//!
//! ## "2K" 这一档为什么不给按钮
//!
//! B 站的 `qn` 是固定档位代码，**没有 2K 这一档**（1440p 的内容是在 120 里给的，
//! `ref/bpi-rs/src/models/stream.rs:8-38`）。摆一个"2K"其实等于摆"4K"，那是假开关；
//! 所以这里给 1080P / 1080P 60帧 / 4K 三个真档，并把这句话写在画质那一排下面。
//!
//! ## 调试模式为什么"下次启动才生效"
//!
//! 两件事在启动时就定了，运行时改不了：
//! 1. 控制台有没有 —— 那是**链接期**的子系统（`main.rs` 的 `#![windows_subsystem = "windows"]`）；
//! 2. `debug.log` 开没开 —— 日志器装上之后没有"再装一次"的口子（`log::set_boxed_logger` 只能一次），
//!    关掉也不能把已经打开的文件句柄收回去。
//! 所以这一项只决定**下一次启动**的行为，界面上把这句写明白，不留"我点了怎么没反应"的疑惑。
//!
//! ## 落盘位置
//!
//! 三个文件都在 [`app::settings_dir`](crate::app::settings_dir)（`%APPDATA%/ArrowMeo/rustBiliPC`）。
//! 旧位置（exe 同目录、更早的 `data/cookies.json`）的兼容代码已在 P13 全部删除，本页也不显示旧路径。

use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;

use crate::api::profile::{self, ProfileSummary};
use crate::pages::loading;
use crate::store::auth::Auth;
use crate::store::config::{self, CodecPreference};
use crate::ui::scroll::DragScroll;
use crate::ui::{layout, palette};

/// 设置页根视图。
pub struct AboutPage {
    client: Arc<bpi_rs::BpiClient>,
    /// 单指/按住拖动的滚动状态（`ui::scroll::DragScroll`，与下面那个滚动容器一一对应）。
    scroll: DragScroll,
    /// 凭据里的 mid（字符串）。
    mid_raw: String,
    account: Option<ProfileSummary>,
    note: SharedString,
    /// 最近一次**写盘**的结果。改设置是同步落盘的，失败必须让他看见，否则就是"改了却没生效"。
    save_note: SharedString,
    /// 选 mpv 那条路上"对话框已经弹出去、还没回来"的提示。
    pick_state: SharedString,
    /// **缓存下来的 mpv 解析结果**（`Some` = 找到的文件，`None` = 没找到），
    /// 由 [`refresh_mpv_status`](Self::refresh_mpv_status) 填。
    ///
    /// 为什么不直接在 `render` 里调 `player::find_mpv()`：那个函数要读 `PATH` 再对每个目录
    /// `is_file()` 一次，而 `render` 是**每帧**都跑的（按住拖动滚动时每帧一次）。
    /// 把一轮磁盘扫描挂在重绘上，症状是"设置页拖起来发涩"，而且这台机的 PATH 里有网络盘时更明显。
    /// 所以只在"进页 / 改过 mpv 相关设置"这两种真需要重算的时候刷新。
    mpv_found: Option<std::path::PathBuf>,
    seq: u32,
    loaded: bool,
}

impl AboutPage {
    /// 建页但不发请求（懒加载）。
    pub fn new(client: Arc<bpi_rs::BpiClient>, mid_raw: String) -> Self {
        Self {
            scroll: DragScroll::default(),
            client,
            mid_raw,
            account: None,
            note: "".into(),
            save_note: "".into(),
            pick_state: "".into(),
            mpv_found: None,
            seq: 0,
            loaded: false,
        }
    }

    /// 换成本次要显示的账号 mid（重登之后可能是另一个账号）。
    pub fn set_mid(&mut self, mid_raw: String) {
        self.mid_raw = mid_raw;
    }

    /// 重算一次 mpv 的位置并缓存。见 `mpv_found` 那条注释：**这是全页唯一做磁盘扫描的地方**。
    fn refresh_mpv_status(&mut self) {
        self.mpv_found = crate::player::find_mpv();
    }

    /// 第一次被看到时取一次账户资料。
    pub fn ensure_loaded(&mut self, cx: &mut Context<Self>) {
        if self.loaded {
            return;
        }
        self.refresh_mpv_status();
        self.reload(cx);
    }

    /// 重新取账户资料（设置那几块不依赖网络，本机信息任何时候都能改）。
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

    /// 改一项设置并写盘。失败把原因写进 `save_note`（不静默）。
    fn set(&mut self, description: String, change: impl FnOnce(&mut config::Config), cx: &mut Context<Self>) {
        // 四项里只有 mpv 那项会改变解析结果，但重算一次只是一轮 `PATH` 扫描、
        // 而且是**点一下按钮**才做一次（不是每帧），所以这里一律重算，省得区分哪项影响哪项。
        self.refresh_mpv_status();
        match crate::app::update_config(change) {
            Ok(path) => {
                self.save_note =
                    SharedString::from(format!("已保存：{description}（{}）", path.display()));
            }
            Err(error) => {
                log::error!("设置页写盘失败: {error}");
                self.save_note = SharedString::from(format!(
                    "保存失败：{error}（{description} 只在内存里生效，重启会退回文件里的值）"
                ));
            }
        }
        cx.notify();
    }

    /// 弹系统文件对话框选 mpv。
    ///
    /// 与下载选目录是同一个 API（`prompt_for_paths`），差别是 `files: true`：
    /// `PathPromptOptions` **没有扩展名过滤这一项**，能给的只有 `prompt` 那句话。
    /// 拿到路径后要验一次"真是个文件"——设置里写一个读不到的路径，
    /// [`resolve_mpv`](crate::player::resolve_mpv) 会跳过它继续往下找，界面上就变成"我明明选了却没生效"。
    fn pick_mpv(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择 mpv.exe（它已经在 PATH 里就不用选）".into()),
        });
        self.pick_state = "等待选择 mpv…".into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            // 三层壳：`await` 给 `Result<平台结果, RecvError>`，平台结果是
            // `Result<Option<Vec<PathBuf>>, Error>`，最里面的 `None` 才是"按了取消"。
            let picked = receiver
                .await
                .ok()
                .and_then(|inner| inner.ok())
                .flatten()
                .and_then(|mut paths| paths.pop());
            let Some(view) = this.upgrade() else { return };
            let Some(path) = picked else {
                view.update(cx, |view, cx| {
                    view.pick_state = SharedString::default();
                    cx.notify();
                });
                return;
            };
            let text = path.to_string_lossy().to_string();
            let is_file = path.is_file();
            view.update(cx, |view, cx| {
                view.pick_state = SharedString::default();
                if !is_file {
                    view.save_note =
                        SharedString::from(format!("选中的东西读不到，设置没改：{text}"));
                    cx.notify();
                    return;
                }
                view.set(
                    format!("MPV 播放器位置 = {text}"),
                    move |config| config.mpv_path = text,
                    cx,
                );
            });
        })
        .detach();
    }

    /// 标题行：左边设置名，右边当前值。
    fn option_head(title: &'static str, current: impl Into<SharedString>) -> AnyElement {
        div()
            .flex()
            .w_full()
            .gap(px(palette::LIST_ROW_PADDING_Y))
            .items_center()
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(palette::FONT_BODY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette::text_primary())
                    .child(title),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .justify_end()
                    .child(
                        div()
                            .text_size(px(palette::FONT_CAPTION))
                            .text_color(palette::text_tertiary())
                            .child(current.into()),
                    ),
            )
            .into_any_element()
    }

    /// 一行按钮（换行排，窄窗口里不会撑破）。
    fn button_row(buttons: Vec<AnyElement>) -> AnyElement {
        div()
            .flex()
            .flex_wrap()
            .w_full()
            .gap(px(palette::CARD_TEXT_ROW_GAP))
            .children(buttons)
            .into_any_element()
    }

    /// 一排里的一个按钮：当前项用主色按钮，其余普通。
    ///
    /// "当前值不在这排选项里"时（`config.json` 手改成了 112 这种没给按钮的档）
    /// 一个都不高亮，只在标题行如实写出 `quality_name`——不硬选第一个。
    fn choice_button(
        id: String,
        label: &'static str,
        selected: bool,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        let button = Button::new(id).label(label);
        if selected {
            button.primary().on_click(on_click)
        } else {
            button.on_click(on_click)
        }
        .into_any_element()
    }

    /// 一句话说明（12px 辅助灰，跟在按钮下面）。
    fn option_hint(text: String) -> AnyElement {
        div()
            .w_full()
            .text_size(px(palette::FONT_CAPTION))
            .text_color(palette::text_tertiary())
            .child(text)
            .into_any_element()
    }

    /// 画质 + 编码 + mpv + 调试模式四块。
    fn setting_blocks(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let config = crate::app::config();
        let mut blocks: Vec<AnyElement> = Vec::new();

        // —— 默认播放画质 ——
        let quality_buttons: Vec<AnyElement> = config::QUALITY_CHOICES
            .iter()
            .map(|(qn, label)| {
                let qn = *qn;
                let label: &'static str = *label;
                Self::choice_button(
                    format!("about-quality-{qn}"),
                    label,
                    qn == config.max_quality,
                    cx.listener(move |this, _, _window, cx| {
                        this.set(
                            format!("默认播放画质 = {}", config::quality_name(qn)),
                            move |config| config.max_quality = qn,
                            cx,
                        );
                    }),
                )
            })
            .collect();
        blocks.push(layout::card(vec![
            Self::option_head("默认播放画质", config::quality_name(config.max_quality)),
            Self::button_row(quality_buttons),
            Self::option_hint(
                "B 站没有\"2K\"这一档（1440p 的内容是在 4K 档里给的），所以只给这三个真档。4K 需要大会员；目标机（i5-6200U）选 4K 大概率软解丢帧。改完对下一次起播/下载生效。"
                    .to_string(),
            ),
        ]));

        // —— 默认播放编码 ——
        let codec_buttons: Vec<AnyElement> = [CodecPreference::Hevc, CodecPreference::Avc, CodecPreference::Av1]
            .into_iter()
            .map(|codec| {
                Self::choice_button(
                    format!("about-codec-{}", codec.label()),
                    codec.label(),
                    codec == config.codec,
                    cx.listener(move |this, _, _window, cx| {
                        this.set(
                            format!("默认播放编码 = {}", codec.label()),
                            move |config| config.codec = codec,
                            cx,
                        );
                    }),
                )
            })
            .collect();
        blocks.push(layout::card(vec![
            Self::option_head("默认播放编码", config.codec.label()),
            Self::button_row(codec_buttons),
            Self::option_hint(format!(
                "同一画质档里优先给这一路编码；B 站没给这一路时自动退回其它编码。{} 改完对下一次起播/下载生效。",
                config.codec.caveat()
            )),
        ]));

        // —— MPV 播放器位置 ——
        let specified = !config.mpv_path.trim().is_empty();
        let current = if specified {
            config.mpv_path.clone()
        } else {
            "自动：PATH → 运行目录".to_string()
        };
        let resolved_text = match self.mpv_found.as_ref() {
            Some(path) => format!("当前实际使用：{}", path.display()),
            None => format!(
                "没找到 {}：把它放进 PATH，或用下面的「选择…」指给程序",
                crate::player::exe_name()
            ),
        };
        let mut mpv_buttons: Vec<AnyElement> = vec![Self::choice_button(
            "about-mpv-pick".to_string(),
            "选择…",
            false,
            cx.listener(|this, _, _window, cx| this.pick_mpv(cx)),
        )];
        if specified {
            mpv_buttons.push(Self::choice_button(
                "about-mpv-auto".to_string(),
                "恢复自动",
                false,
                cx.listener(|this, _, _window, cx| {
                    this.set(
                        "MPV 播放器位置 = 自动（PATH → 运行目录）".to_string(),
                        |config| config.mpv_path.clear(),
                        cx,
                    );
                }),
            ));
        }
        let mut mpv_rows = vec![
            Self::option_head("MPV 播放器位置", current),
            Self::button_row(mpv_buttons),
            Self::option_hint(resolved_text),
        ];
        if !self.pick_state.is_empty() {
            mpv_rows.push(Self::option_hint(self.pick_state.to_string()));
        }
        mpv_rows.push(Self::option_hint(
            "查找顺序：这里指定的文件 → PATH 里的 mpv.exe → 程序目录里的 mpv.exe。只认 mpv.exe，绝不认 mpv.com（它是个会再派生一个 mpv.exe 的控制台启动器，杀掉它管不到真正在放的那个进程）。".to_string(),
        ));
        blocks.push(layout::card(mpv_rows));

        // —— 调试模式 ——
        let debug_buttons: Vec<AnyElement> = vec![
            Self::choice_button(
                "about-debug-on".to_string(),
                "开",
                config.debug_log,
                cx.listener(|this, _, _window, cx| {
                    this.set("调试模式 = 开".to_string(), |config| config.debug_log = true, cx);
                }),
            ),
            Self::choice_button(
                "about-debug-off".to_string(),
                "关",
                !config.debug_log,
                cx.listener(|this, _, _window, cx| {
                    this.set("调试模式 = 关".to_string(), |config| config.debug_log = false, cx);
                }),
            ),
        ];
        blocks.push(layout::card(vec![
            Self::option_head(
                "调试模式",
                if config.debug_log { "开：写 debug.log" } else { "关：不写日志" },
            ),
            Self::button_row(debug_buttons),
            Self::option_hint(
                "下一次启动才生效：要不要创建日志文件、要不要输出日志都在启动时决定，运行中不能改。关闭时不创建 debug.log，也不做任何日志输出；程序在 Windows 上是 GUI 子系统，两种模式下双击都不会弹控制台窗口。崩溃记录（crash.log）不受这一项影响，永远会写。"
                    .to_string(),
            ),
        ]));

        blocks
    }

    /// 本机那几行：版本与几个文件的位置。全是只读信息，网络断了也照样显示。
    fn machine_rows(&self) -> Vec<AnyElement> {
        let config = crate::app::config();
        let log_line = match crate::logging::log_path() {
            Some(path) => format!("{}（本次启动已在写）", path.display()),
            None => "没有（调试模式关闭，或日志文件没能创建）".to_string(),
        };
        let rows: Vec<(String, String)> = vec![
            ("版本".to_string(), crate::VERSION.to_string()),
            (
                "设置目录".to_string(),
                crate::app::settings_dir().display().to_string(),
            ),
            ("凭据文件".to_string(), Auth::path().display().to_string()),
            ("配置文件".to_string(), config::Config::path().display().to_string()),
            ("日志文件".to_string(), log_line),
            (
                "崩溃记录".to_string(),
                crate::logging::crash_log_path().display().to_string(),
            ),
            (
                "程序目录".to_string(),
                crate::app::exe_dir().display().to_string(),
            ),
            // 把"真正生效"的值再写一遍：万一 config.json 是手改的、或者上一次写盘失败，
            // 这一行会跟上面按钮的高亮不一致，那就是要查的线索。
            (
                "当前生效的档位".to_string(),
                format!(
                    "{} · 优先 {}",
                    config::quality_name(config.max_quality),
                    config.codec.label()
                ),
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 每帧登记一次移动/抬起监听（`Window::on_mouse_event` 只覆盖下一帧，见 `ui/scroll.rs`）。
        self.scroll.watch(window, cx.entity_id());
        let mut rows: Vec<AnyElement> = self.account_rows();
        if !self.save_note.is_empty() {
            rows.push(layout::note(self.save_note.clone()));
        }
        rows.push(layout::section_title("播放"));
        rows.extend(self.setting_blocks(cx));
        rows.push(layout::section_title("本机"));
        rows.extend(self.machine_rows());

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(layout::header("设置", None, Some(self.reload_button(cx))))
            .child(
                div()
                    .id("about-body")
                    .flex_1()
                    .w_full()
                    .min_w(px(0.))
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
                            .children(rows),
                    ),
            )
    }
}
