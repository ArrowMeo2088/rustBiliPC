//! rustBiliPC 启动入口。
//!
//! 启动顺序是有讲究的，别随意调换：
//! 1. 日志 —— 写 **exe 同目录**的 `debug.log`（每次启动清空、级别 debug），见 [`rustbilipc::logging`]。
//! 2. tokio runtime + rustls 默认 provider —— 见 [`rustbilipc::app`] 的说明：API 调用依赖前者，
//!    没有后者任何 HTTPS 请求都会在握手处 panic（表现是界面一直停在加载态）。
//! 3. `with_assets` —— kit 的图标/字体资源，必须在 `init` 之前挂上。
//! 4. `gpui_kit::init` → 主题覆盖 → **装 HttpClient** → 装进程级设置 → 读凭据 → 开窗。
//!
//! 第 4 步里的 HttpClient 不能省：gpui 默认装的是 `NullHttpClient`（`gpui` 的 `app.rs` 里
//! 直接构造），任何网络图片请求都会失败；封面图靠 `ImageAssetLoader` 走
//! `App::http_client()`，所以必须换成真的 reqwest 客户端。
//!
//! ## 开窗这一步现在有两种结局（2026-10-26 用户定稿）
//!
//! 运行同目录有可用的 `auth.json` → 只开主窗口；没有 → 只开一个小的扫码登录窗，
//! 登录成功后由 [`LoginGate`](rustbilipc::pages::gate::LoginGate) 换窗口。
//! 尺寸与"先开主窗再关登录窗"的理由都写在 [`windows`](rustbilipc::pages::windows) 里。
//!
//! ## 设置为什么要在开窗之前装
//!
//! 起播那条路读的封顶档来自 `config.json`（见 [`stream::request_qn`](rustbilipc::api::stream::request_qn)）。
//! 装晚了不会崩，但"第一帧就起播"的路径上会读到默认值，界面上说的话和实际请求的参数可能对不上。

use std::sync::Arc;

use rustbilipc::app;
use rustbilipc::logging;
use rustbilipc::pages::windows;
use rustbilipc::store::auth::Auth;
use rustbilipc::store::config::Config;
use rustbilipc::theme;

/// 我们对外 HTTP 客户端自报的家门名（User-Agent 的一部分）。
const HTTP_CLIENT_NAME: &str = "rustbilipc";

fn main() {
    // 日志必须第一个装：GUI 程序没有控制台，出问题时全靠 exe 同目录的 debug.log。
    let log_path = logging::init();
    log::info!(
        "rustBiliPC v{} 启动，工作目录 {:?}",
        rustbilipc::VERSION,
        crate::app::exe_dir()
    );
    log::info!("日志文件 {:?}", log_path);

    // 提前建好 API 用的 runtime，避免第一次网络请求时才初始化带来的不确定性。
    app::runtime();
    // 必须在任何 HTTPS 请求之前装好 rustls 的默认 provider，否则握手处 panic。
    app::install_tls_provider();

    // 客户端是 Send+Sync 的（内部 reqwest::Client + Mutex<Option<Account>> + WBI 缓存），
    // 用 Arc 在 gpui 与 tokio 两侧共享同一份 cookie jar 与 WBI key 缓存。
    let client = Arc::new(
        bpi_rs::BpiClient::new().expect("初始化 BpiClient 失败，请检查网络与系统时间"),
    );
    log::info!("BpiClient 就绪");

    // Assets 用 kit 的**完整** Lucide 集（1830 个 svg 全嵌进 exe）。
    // 默认那份 `Assets` 只嵌 106 个，侧栏要的 House/Clock/Bookmark/Download 都不在里面；
    // 名字找不到不会报错，只会画出一片空白（见 `ui::layout::side_item` 的说明）。
    // 将来要加载我们自己的图片/字体时，改成组合形式而不是退回 `Assets`。
    let application = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);

    application.run(move |cx| {
        // 使用任何 kit 组件前必须调用。
        gpui_kit::init(cx);
        theme::apply(cx);

        let http_client = reqwest_client::ReqwestClient::user_agent(HTTP_CLIENT_NAME)
            .expect("初始化 HttpClient 失败");
        cx.set_http_client(Arc::new(http_client));

        // 进程级设置：读运行目录的 config.json，没有就写一份默认的出来。
        let config = Config::load_or_create();
        log::info!(
            "设置就绪：max_quality={}（{:?}）",
            config.max_quality,
            Config::path()
        );
        app::init_config(config);

        // 凭据：读 auth.json；只有它不合格时才去看一次旧的 data/cookies.json（迁移，见 store::auth）。
        let auth = Auth::load();
        log::info!(
            "启动判定：凭据文件 {:?}（可读={}，字段完整={:?}）",
            Auth::path(),
            auth.is_some(),
            auth.as_ref().map(|auth| auth.is_complete())
        );

        let opened = match auth.filter(|auth| auth.is_complete()) {
            Some(auth) => {
                log::info!("本机已有凭据，直接开主窗口 mid={}", auth.dede_user_id);
                if let Err(error) = client.set_account(auth.to_account()) {
                    // 凭据可能已失效；不致命，接口会用错误码提示，用户也能从设置页看到账户栏的空态。
                    log::warn!("复用已保存的登录态失败: {error}");
                }
                windows::open_main_window(Arc::clone(&client), auth.dede_user_id, cx)
            }
            None => {
                log::info!("本机没有可用凭据，开扫码登录小窗");
                windows::open_login_window(Arc::clone(&client), cx)
            }
        };

        // 一个窗口都没开成功就没有可看的界面，也没有任何东西能再触发退出：
        // 直接把原因写进日志并结束进程，比留一个隐形的主线程挂在循环里有用。
        if let Err(error) = opened {
            log::error!("开窗口失败，退出: {error}");
            cx.quit();
        }
    });
}
