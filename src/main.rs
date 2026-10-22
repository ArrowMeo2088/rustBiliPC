//! rustBiliPC 启动入口。
//!
//! 启动顺序是有讲究的，别随意调换：
//! 1. 日志 —— gpui 与 bpi-rs 都用 `log`，没有 logger 时启动失败只会静默退出。
//! 2. tokio runtime —— 见 [`rustbilipc::app`] 的说明，API 调用依赖它。
//! 3. `with_assets` —— kit 的图标/字体资源，必须在 `init` 之前挂上。
//! 4. `gpui_kit::init` → 主题覆盖 → 开窗。

use gpui_kit::*;
use rustbilipc::app;
use rustbilipc::pages::search::SearchPage;
use rustbilipc::theme;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // 提前建好 API 用的 runtime，避免第一次网络请求时才初始化带来的不确定性。
    app::runtime();

    // Assets 是 gpui-kit 自带的 rust-embed 资源（Lucide 图标等）。
    // 将来要加载我们自己的图片/字体时，改成 `Assets::before(our_assets)`。
    let application = gpui_kit::application().with_assets(gpui_kit::assets::Assets);

    application.run(move |cx| {
        // 使用任何 kit 组件前必须调用。
        gpui_kit::init(cx);
        theme::apply(cx);

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(
                size(px(1120.), px(760.)),
                cx,
            )),
            ..Default::default()
        };

        // open_window 会把视图包进 kit 的 Root，弹层/对话框/菜单才有宿主。
        gpui_kit::open_window(window_options, cx, |window, cx| {
            cx.new(|cx| SearchPage::new(window, cx))
        })
        .expect("打开主窗口失败");
    });
}
