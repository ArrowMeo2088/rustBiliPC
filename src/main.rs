//! rustBiliPC 启动入口。
//!
//! 启动顺序是有讲究的，别随意调换：
//! 1. 日志 —— gpui 与 bpi-rs 都用 `log`，没有 logger 时启动失败只会静默退出。
//! 2. tokio runtime + rustls 默认 provider —— 见 [`rustbilipc::app`] 的说明：API 调用依赖前者，
//!    没有后者任何 HTTPS 请求都会在握手处 panic（表现是界面一直停在加载态）。
//! 3. `with_assets` —— kit 的图标/字体资源，必须在 `init` 之前挂上。
//! 4. `gpui_kit::init` → 主题覆盖 → **装 HttpClient** → 开窗。
//!
//! 第 4 步里的 HttpClient 不能省：gpui 默认装的是 `NullHttpClient`（`gpui` 的 `app.rs` 里
//! 直接构造），任何网络图片请求都会失败；封面图靠 `ImageAssetLoader` 走
//! `App::http_client()`，所以必须换成真的 reqwest 客户端。

use std::sync::Arc;

use gpui_kit::*;
use rustbilipc::app;
use rustbilipc::pages::RootView;
use rustbilipc::theme;

/// 我们对外 HTTP 客户端自报的家门名（User-Agent 的一部分）。
const HTTP_CLIENT_NAME: &str = "rustbilipc";

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // 提前建好 API 用的 runtime，避免第一次网络请求时才初始化带来的不确定性。
    app::runtime();
    // 必须在任何 HTTPS 请求之前装好 rustls 的默认 provider，否则握手处 panic。
    app::install_tls_provider();

    // 客户端是 Send+Sync 的（内部 reqwest::Client + Mutex<Option<Account>> + WBI 缓存），
    // 用 Arc 在 gpui 与 tokio 两侧共享同一份 cookie jar 与 WBI key 缓存。
    let client = Arc::new(
        bpi_rs::BpiClient::new().expect("初始化 BpiClient 失败，请检查网络与系统时间"),
    );

    // Assets 是 gpui-kit 自带的 rust-embed 资源（Lucide 图标等）。
    // 将来要加载我们自己的图片/字体时，改成 `Assets::before(Assets)` 的组合形式。
    let application = gpui_kit::application().with_assets(gpui_kit::assets::Assets);

    application.run(move |cx| {
        // 使用任何 kit 组件前必须调用。
        gpui_kit::init(cx);
        theme::apply(cx);

        let http_client = reqwest_client::ReqwestClient::user_agent(HTTP_CLIENT_NAME)
            .expect("初始化 HttpClient 失败");
        cx.set_http_client(Arc::new(http_client));

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1120.), px(760.)), cx)),
            ..Default::default()
        };

        // open_window 会把视图包进 kit 的 Root，弹层/对话框/菜单才有宿主。
        // 客户端在闭包外 clone 一次：open_window 的构造闭包是 FnMut，会被多次调用。
        // 根视图内部再决定进登录页还是主界面（见 docs/architecture.md 第 4.6 节）。
        let client_for_window = Arc::clone(&client);
        gpui_kit::open_window(window_options, cx, move |window, cx| {
            let client = Arc::clone(&client_for_window);
            cx.new(|cx| RootView::new(window, cx, client))
        })
        .expect("打开主窗口失败");
    });
}
