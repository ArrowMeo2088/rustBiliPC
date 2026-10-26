//! 滚动容器的**按住拖动滚动**（触摸屏优化，2026-10-26）。
//!
//! ## 先说清楚这不是"触摸事件"
//!
//! gpui 0.3.9 的 Windows 平台层**没有触摸管线**：`ref/gpui/gpui_windows/src` 里搜不到
//! `WM_TOUCH`、`WM_POINTERDOWN/UP/UPDATE`、`RegisterPointerInputTarget`、`EnableMouseInPointer`，
//! 只处理 `WM_MOUSEMOVE`（`events.rs:112`）、`WM_LBUTTONDOWN/UP`（`:136/:142`）、`WM_MOUSEWHEEL`（`:148`）。
//! gpui 核心的触摸类型（`TouchEvent`/`TouchPhase`/`dispatch_touch_event`，`window.rs:5810`）都在，
//! 但喂它们的是 macOS 后端（这个仓库里没 vendored），所以 Windows 上那条管线是**永久闲置**的。
//!
//! 手指还有反应吗——有，靠 Windows 把触点**注入成鼠标消息**：按住拖动是一条带 `MK_LBUTTON` 的
//! `WM_MOUSEMOVE`，落到 gpui 就是 `MouseMoveEvent { position, pressed_button: Some(Left), .. }`，
//! `event.dragging()` 为真（`gpui/src/interactive.rs:494-515`）。所以"单指上下滚"能做成，
//! 实现就是**按住把内容拖着走**；同一份代码顺带给了鼠标用户"按住左键拖列表"这个常见行为。
//! 反过来说：**触摸屏不会有 hover**，靠 hover 才显出来的东西（例如只在悬停时加深的底色）
//! 在手指上是看不见的，这条与拖动无关，但属于同一份"为触摸屏优化"的事实。
//!
//! ## 机制照 gpui 自己的滚动条抄
//!
//! - 偏移存在 `ScrollHandle`（`gpui/src/elements/div.rs:4249`）里；容器用
//!   `.id(..).track_scroll(&handle)`（`:1549`）绑上，之后 paint 偏移与 handle 是同一个
//!   `Rc<RefCell<..>>`（`:2272-2275`）。**`ScrollHandle` 没有 `scroll_by`**
//!   （那个只在 `ListState` 上，`list.rs:565`），所以走 `set_offset`；偏移是负值，
//!   手指往下拖（dy 为正）偏移跟着变大 = 往上翻，符合"拖纸"的直觉。
//!   这里自己先夹一次到 `[0 - max_offset, 0]`（`max_offset()` 在 `:4269`）：prepaint 也会夹
//!   （`:2474-2486`），但不夹的话拖出边界时我们记录的内部偏移会一路累出去，回弹方向就乱了。
//! - 改完偏移要催重绘：`cx.notify(view)`，view 由 `Context::entity_id()`（`app/context.rs:46`）
//!   在页面 render 里取好传进来（`App::notify` 在 `app.rs:3052`）——与 kit 滚动条
//!   `notify_drag(…, view_id, …)`（`base/src/scrollbar.rs:517-533`）同一做法。
//! - 移动/抬起监听用 `Window::on_mouse_event`（`window.rs:5432`），**它只登记"下一帧"的监听、
//!   且只能在 paint 阶段调**（文档原文：register … for the next frame … the listener will be cleared）。
//!   所以 [`wrap`](DragScroll::wrap) 必须在页面的 `render` 里调，不能挪到 `new()`：
//!   挪过去只有第一次那一帧收得到移动事件，之后监听被清空、内容就再也不跟着手指走了。
//! - 移动里 `cx.stop_propagation()`（`app.rs:2645`）：gpui 滚动条在同一处也停传播，理由一样——
//!   不停会顺带开始选中文本，屏幕上看到"拖出一段高亮"而不是内容在动。
//!
//! ## 按下不等于要滚
//!
//! 只有真的动了才改偏移：点在一行卡片上按下→抬起没有位移，滚动不触发、`on_click` 照常。
//! 反过来，从一张卡片起步拖到别处，抬起时那张卡片**不会**被点着——gpui 的点击要求同一个元素
//! 自己的 `pending_mouse_down`（`elements/div.rs:3609-3622`）配得上，起点终点不是同一元素就不配对，
//! 这正好是"拖完不误触"。

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::*;

/// 按下时的起点：指针 y 与当时的偏移。都用 `Pixels` 做加减与比较，不拆它的内部字段
/// （`Pixels(pub(crate) f32)`，`geometry.rs:2689`——字段是私有的，跨 crate 只能用运算符）。
#[derive(Clone, Copy)]
struct DragStart {
    pointer_y: Pixels,
    offset_y: Pixels,
}

/// 拖动后的目标偏移：**纸跟着手指走**（手指往下走，内容往下移 = 往上翻），并且夹在
/// `[0 - max, 0]` 里（gpui 的 `ScrollHandle::offset()` 是负值，`max_offset()` 是正的可用行程）。
///
/// 单独做成纯函数并进单测，是因为这里两处都能"写得出来但方向不对"：符号反了就是
/// "手指往上、内容往下"；忘了夹会在拖出边界后攒出一段"要往回拖一大截才动"的死区。
///
/// `Pixels` 的 `Add/Sub/Ord` 都在（derive 见 `geometry.rs:2671-2686`，
/// `PartialOrd`/`Ord` 是手写的 `:2891-2899`），所以能直接 `clamp`。
pub fn target_offset(start: Pixels, dy: Pixels, max: Pixels) -> Pixels {
    (start + dy).clamp(px(0.) - max, px(0.))
}

/// 一个滚动容器的拖动状态。**每个容器一个**：详情页主列与右栏是两个字段，互不牵连。
///
/// 用法（页面的 `render` 里三处，缺一个就退化成"只能滚轮"）：
///
/// ```ignore
/// let view = cx.entity_id();
/// self.scroll.watch(window, view);              // 1. 本帧登记移动/抬起监听
/// ...
/// div()
///     .id("home-body")
///     .overflow_y_scroll()
///     .track_scroll(self.scroll.handle())       // 2. 把偏移交给这个句柄
///     .on_mouse_down(MouseButton::Left, self.scroll.on_press()) // 3. 按下记起点
///     .child(body)
/// ```
#[derive(Clone, Default)]
pub struct DragScroll {
    handle: ScrollHandle,
    drag: Rc<RefCell<Option<DragStart>>>,
}

impl DragScroll {
    /// 给滚动容器用：`.track_scroll(page.scroll.handle())`。
    ///
    /// 借出去而不是克隆——`track_scroll` 收的就是 `&ScrollHandle`，
    /// 而句柄内部是 `Rc<RefCell<..>>`，克隆一份也只是同一个偏移的两个名字，没必要。
    pub fn handle(&self) -> &ScrollHandle {
        &self.handle
    }

    /// 给滚动容器用：`.on_mouse_down(MouseButton::Left, page.scroll.on_press())`。
    ///
    /// 只记起点，不 `stop_propagation`、不吞事件：按下要照常落到里面的卡片与按钮上，
    /// 真正的"滚"只在下一步移动里发生（没移动就是点击，一切照旧）。
    pub fn on_press(&self) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
        let handle = self.handle.clone();
        let drag = self.drag.clone();
        move |event: &MouseDownEvent, _window: &mut Window, _cx: &mut App| {
            *drag.borrow_mut() = Some(DragStart {
                pointer_y: event.position.y,
                offset_y: handle.offset().y,
            });
        }
    }

    /// 在**这一帧的 paint 阶段**登记移动与抬起监听（页面的 `render` 里调一次，每帧都要调）。
    ///
    /// `Window::on_mouse_event` 注册的是"下一帧"的监听、帧末清空
    /// （`gpui/src/window.rs:5427-5445` 的文档原文），所以不能挪到 `new()` 里只调一次：
    /// 那样只有头一帧收得到移动，之后手指拖得再久也没人改偏移。
    /// `view` 是页面实体的 id（`Context::entity_id()`），改完偏移要用它催重绘。
    pub fn watch(&self, window: &mut Window, view: EntityId) {
        window.on_mouse_event({
            let drag = self.drag.clone();
            move |_event: &MouseUpEvent, _phase, _window: &mut Window, _cx: &mut App| {
                *drag.borrow_mut() = None;
            }
        });

        window.on_mouse_event({
            let handle = self.handle.clone();
            let drag = self.drag.clone();
            move |event: &MouseMoveEvent, _phase, _window: &mut Window, cx: &mut App| {
                let Some(start) = *drag.borrow() else {
                    return;
                };
                // `dragging()` = 这条移动消息带着按住的键（`interactive.rs:515`）。
                if !event.dragging() {
                    return;
                }
                let y = target_offset(
                    start.offset_y,
                    event.position.y - start.pointer_y,
                    handle.max_offset().y,
                );
                let target = Point {
                    x: handle.offset().x,
                    y,
                };
                if handle.offset() == target {
                    return;
                }
                handle.set_offset(target);
                // 停在传播路径上：不停会顺带开始选中文本，看到的是"拖出一段高亮"而不是内容在动。
                cx.stop_propagation();
                cx.notify(view);
            }
        });
    }
}
