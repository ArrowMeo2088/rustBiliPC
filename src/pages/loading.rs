//! 各入口页共用的"把一次 tokio 请求的结果送回实体"样板。
//!
//! 这条路上有三条规矩是踩过坑定下来的（`docs/architecture.md` 第 4.9 节）：
//! 1. 末尾必须 `.detach()`——`Task` 一被 drop 任务就取消，表现是"请求发出去了、界面永远停在加载态"；
//! 2. 结果回来后要先 `upgrade()`，实体已经释放（用户关了窗口）就丢弃，不许拿悬空句柄写状态；
//! 3. 要用**序号**判定"这一次结果还是不是最新那次请求的"，旧任务自己让位。
//!    序号是每页自己的（分块加载时更要分开，见第 4.18 节空间页那三个槽），
//!    所以这里收一个 `is_latest` 闭包去读页面当前的序号，而不是把序号存在样板里。
//!
//! 与"个人中心页里那三份重复的 spawn 代码"（P2b 时期）的区别：那一页三路并发、
//! 每路写各的槽位，样板化后调用点只剩"投请求 + 写哪个槽"，加新页面不用再抄 20 行。
//!
//! ## P13 起的第四条：`-101` 不用人管
//!
//! 错误是 `BpiError` 时先问一句 [`requires_login`](bpi_rs::BpiError::requires_login)，
//! 是"要登录"那一类就交给 [`relogin`](super::relogin) 处置（清凭据 + 自动开扫码小窗）。
//! 两步分开做：**判断**在 `await` 之后立刻做（那时还没有能开窗口的上下文），
//! **处置**回到 `AsyncApp::update` 那一层做 —— `App::open_window` 会强制画一帧，
//! 不能塞在某个实体的 `update` 里（第 4.11 节那条，`pages/windows.rs` 文件头写得更细）。

use gpui_kit::*;

use crate::pages::relogin;

/// 起一路加载任务。`label` 只用于日志；`write` 决定结果写进哪个槽位。
///
/// `T: 'static` 不是可选项：任务要跨 `await` 持有 `JoinHandle<BpiResult<T>>`，
/// gpui 的 spawn 要求整个 future 是 `'static`（CI 实测 E0310 就指这一行）。
pub fn spawn_into<V, T>(
    cx: &mut Context<V>,
    seq: u32,
    label: &'static str,
    join: tokio::task::JoinHandle<bpi_rs::BpiResult<T>>,
    is_latest: impl Fn(&V) -> u32 + 'static,
    write: impl FnOnce(&mut V, Result<T, String>) + 'static,
) where
    V: 'static,
    T: 'static,
{
    cx.spawn(async move |this, cx| {
        let outcome = match join.await {
            Ok(inner) => inner.map_err(|error| relogin::message(&error)),
            Err(join_error) => Err(crate::app::describe_join_error(join_error)),
        };
        log::debug!("页面加载：{label} 结果回到前台（请求 #{seq}）");

        let Some(view) = this.upgrade() else {
            log::warn!("页面加载：等待期间实体已释放，丢弃 {label} 结果");
            return;
        };
        view.update(cx, |view, cx| {
            if is_latest(view) != seq {
                log::debug!("页面加载：请求 #{seq} 已被取代，丢弃 {label} 结果");
                return;
            }
            write(view, outcome);
            cx.notify();
        });
        // 处置放在实体更新之外那一层（见文件头第四条）。
        relogin::flush(cx);
    })
    .detach();
}
