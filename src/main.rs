//! rustBiliPC 启动入口。
//!
//! 启动顺序是有讲究的，别随意调换：
//! 0. **`logging::install_panic_hook()`** —— `crash.log` 与"调试模式"无关，永远写（GUI 子系统下
//!    panic 只打 stderr，不装钩子就是"静默消失"）。
//! 1. 读设置 —— `%APPDATA%\ArrowMeo\rustBiliPC\config.json`，见 [`rustbilipc::store::config`]。
//! 2. 日志 —— 按设置里的"调试模式"决定要不要写同目录的 `debug.log`（每次启动清空、级别 debug），
//!    见 [`rustbilipc::logging`]。关掉时连文件都不创建。
//! 3. tokio runtime + rustls 默认 provider —— 见 [`rustbilipc::app`] 的说明：API 调用依赖前者，
//!    没有后者任何 HTTPS 请求都会在握手处 panic（表现是界面一直停在加载态）。
//! 4. `with_assets` —— kit 的图标/字体资源，必须在 `init` 之前挂上。
//! 5. `gpui_kit::init` → 主题覆盖 → **装 HttpClient** → 读凭据 → 开窗。
//!
//! 第 5 步里的 HttpClient 不能省：gpui 默认装的是 `NullHttpClient`（`gpui` 的 `app.rs` 里
//! 直接构造），任何网络图片请求都会失败；封面图靠 `ImageAssetLoader` 走
//! `App::http_client()`，所以必须换成真的 reqwest 客户端。
//!
//! ## 开窗这一步现在有两种结局（2026-10-26 用户定稿）
//!
//! settings 目录有可用的 `auth.json` → 只开主窗口；没有 → 只开一个小的扫码登录窗，
//! 登录成功后由 [`LoginGate`](rustbilipc::pages::gate::LoginGate) 换窗口。
//! 尺寸与"先开主窗再关登录窗"的理由都写在 [`windows`](rustbilipc::pages::windows) 里。
//!
//! ## 设置为什么要在开窗之前装
//!
//! 起播那条路读的封顶档来自 `config.json`（见 [`stream::request_qn`](rustbilipc::api::stream::request_qn)）。
//! 装晚了不会崩，但"第一帧就起播"的路径上会读到默认值，界面上说的话和实际请求的参数可能对不上。
//! 日志也要读它（调试模式），所以设置必须排在最前 —— 这也是 1、2 两步顺序的原因。

// 双击启动不再弹那个黑色控制台窗口（2026-10-26 P13）。
//
// 他说"调试模式关闭则启动时不显示控制台"。控制台的存在与否是**链接期**决定的子系统，
// 不是运行时能开关的，所以这里取"永远不弹"：调试模式只决定要不要写 `debug.log`
// （关了连文件都不创建）。真想要"调试时弹一个控制台"，得另外做 `AttachConsole`，那没做。
// **诊断版（`--features console_diag`）故意不加这条**：留着控制台，双击就能看到加载器的报错
// （缺 DLL、不是有效的 Win32 应用）与 panic 文本——GUI 子系统下这些全是静默消失。
#![cfg_attr(
    all(target_os = "windows", not(feature = "console_diag")),
    windows_subsystem = "windows"
)]

use std::sync::Arc;

use rustbilipc::app;
use rustbilipc::logging;
use rustbilipc::pages::gate::GateMode;
use rustbilipc::pages::relogin;
use rustbilipc::pages::windows;
use rustbilipc::store::auth::Auth;
use rustbilipc::store::config::{Config, LoadOutcome};
use rustbilipc::theme;

/// 我们对外 HTTP 客户端自报的家门名（User-Agent 的一部分）。
const HTTP_CLIENT_NAME: &str = "rustbilipc";

fn main() {
    // **第一件事**是装 panic 钩子：GUI 子系统下 panic 只打 stderr，而那份 stderr 没人看得见，
    // 症状就是"窗口自己没了、什么痕迹都不留"。`crash.log` 与"调试模式"无关，永远写。
    // 装在读设置之前，是因为连"读设置"这一步崩掉也要有现场。
    logging::install_panic_hook();

    // 顺序变了（P13）：**先读设置，再装日志**。
    // 因为"要不要写 debug.log"本身就是设置里的一项，日志必须等知道答案的那一步之后才装。
    // 读设置这一步不碰网络、不碰 tokio，所以放在最前面是安全的。
    let (config, outcome) = Config::load_or_create();
    // 配置文件坏了就用默认值，而默认值是"不写日志"——那样他就既看不到错误、
    // 也没法打开调试模式（开关就写在那个坏文件里）。所以这一轮强制开日志。
    // 诊断版**强制开日志**：那份 exe 就是为了拿日志回来查的，不该还要求"先进设置页开、再重启"。
    let debug_log = config.debug_log
        || cfg!(feature = "console_diag")
        || matches!(outcome, LoadOutcome::RecoveredFromBroken);
    app::init_config(config);

    let log_path = logging::init(debug_log);
    log::info!(
        "rustBiliPC v{} 启动，工作目录 {:?}，数据目录 {:?}（平台 {}/{}，诊断版={}）",
        rustbilipc::VERSION,
        crate::app::exe_dir(),
        crate::app::settings_dir(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        cfg!(feature = "console_diag")
    );
    match &log_path {
        Some(path) => log::info!("日志文件 {:?}", path),
        None => log::info!("调试模式关闭：本轮不产生 debug.log"),
    }
    match outcome {
        LoadOutcome::Loaded => {}
        LoadOutcome::Created => log::info!("设置：本次写入了默认的 config.json"),
        LoadOutcome::RecoveredFromBroken => {
            log::error!("设置：config.json 读不出来，本次用默认值并强制开启日志")
        }
    }
    log::info!(
        "设置：max_quality={} codec={} mpv={:?} debug_log={}",
        app::config().max_quality,
        app::config().codec.label(),
        app::config().mpv_path,
        app::config().debug_log
    );

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
    // 重登协调器要拿**同一份**客户端去开扫码窗（凭据过期时就地重登，不必重启）：
    // 换一份客户端就等于换一个 cookie jar，扫完码主窗口那些页面还是旧的登录态。
    relogin::set_client(Arc::clone(&client));

    // 图标资源用**我们自己那张表**（`src/ui/assets.rs`：kit 默认那 106 个 + 侧栏多用的 4 个，
    // 48 KB），不用 kit 的 `AllAssets`（1830 个 svg、磁盘 732 KB 全塞进 exe，其中 94% 永远不会被请求；
    // 实测挂在二进制里比原始字节更贵，换掉这版 exe 小了 0.9 MB）。
    // 为什么不用 kit 默认的 `Assets`：侧栏要的 House/Clock/Bookmark/Download 不在那 106 个里，
    // 而**图标名找不到不会报错、只会画出一片空白**（见 `ui::layout::side_item` 的说明）——
    // 所以我们把默认那 106 个一并带上，让 kit 自己的组件也不缺图。
    // 加新图标：拷 svg + `ICONS` 表加一行 + `assets_tests` 那个数组加一项，三处都要做
    // （见 `src/ui/icons/README.md`）。
    let application = gpui_kit::application().with_assets(rustbilipc::ui::assets::AppAssets);

    application.run(move |cx| {
        // 使用任何 kit 组件前必须调用。
        gpui_kit::init(cx);
        theme::apply(cx);

        let http_client = reqwest_client::ReqwestClient::user_agent(HTTP_CLIENT_NAME)
            .expect("初始化 HttpClient 失败");
        cx.set_http_client(Arc::new(http_client));

        // 凭据：读 settings 目录的 auth.json（P13 起不再有旧位置的兼容代码，见 store::auth）。
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
                windows::open_login_window(Arc::clone(&client), GateMode::Startup, cx).map(|handle| {
                    // 登记给重登协调器：这个窗还开着的时候，`-101` 不该再开第二个。
                    relogin::note_login_window(handle);
                })
            }
        };

        // 一个窗口都没开成功就没有可看的界面，也没有任何东西能再触发退出：
        // 直接把原因写进日志并结束进程，比留一个隐形的主线程挂在循环里有用。
        if let Err(error) = opened {
            log::error!("开窗口失败，退出: {error}");
            // 这条路上外壳根本没建起来，也就没有 `DownloadQueue::Drop` 兜底 —— 手工叫一次收尾。
            rustbilipc::api::download::cancel_all_active();
            cx.quit();
        }
    });
}
