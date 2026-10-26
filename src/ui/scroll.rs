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
//!
//! ## P11：按在可选中文字上的那一下让给文本选择
//!
//! 详情页与评论区的正文换成了 `gpui_kit::base::SelectableText`（gpui 自己没有选择机制，
//! 这一层在 kit 的 `base` 里，由 `base::Root` 挂的 `TextSelectionLayer` 接管指针）。
//! 于是同一个"按住左键拖"出现了两种想要的结果：拖列表 vs 划一段文字。
//! 分界按**按下时落点**定，用 `TextSelection::is_selectable_at(position, window, cx)` 问一句
//! （它读的是上一帧 paint 登记的 `text_bounds`，所以必须在按下时问、不能事后问）：
//! - 落在可选中的文字上 → 不记拖动起点（并把已有起点清空），这一路拖动就只是选字；
//! - 落在卡片、封面、空白处 → 照旧拖着走。
//!
//! 为什么不做"手指=滚、鼠标=选"这种区分：gpui 的 Windows 后端**没有触摸管线**（文件头那一节），
//! 手指与鼠标到这一层是同一条 `WM_MOUSEMOVE`，没有可辨的来源。
//! 也正因为这样，`cx.stop_propagation()` 那句仍然留着——它现在只会在"落点不是文字"的拖动里发出，
//! 不会再打断选择。副作用要知道：**长段落区域只能选字，要滚就得在卡片之间或空白处拖**。
use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::base::TextSelection;
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

/// 自动连续翻页的提前量（px）。
///
/// 为什么不是"真的滚到底才取下一页"：那样用户每次都要先停住、再等一批发回来、
/// 再看见新条目，手感是"卡一下"。留 240px（大约两行卡片）就够把等待藏在滚动里。
pub const AUTO_PAGE_MARGIN: f32 = 240.;

/// 是不是"已经到底部附近"（自动翻页的判据，纯函数，进单测）。
///
/// 单位是 `f32` 而不是 `Pixels`：`ScrollHandle::offset()` 给的是**负的**偏移（内容往上移了多少），
/// `max_offset()` 给的是正的可用行程，两个符号不一样，先取 `as_f32()` 再比较最不容易写错。
///
/// 三条边界都要照顾到：
/// - `max == 0`（内容比视口短，压根没有行程）：按定义就是"在底部" → 会触发翻页。
///   所以调用方必须自己再问一句"有没有真的可滚/有没有在途请求"，见各页的 `auto_load_more`。
/// - 还没 paint 过时 `offset == 0` 且 `max == 0`：同上，靠调用方的"列表非空 + 未在途"挡住。
/// - 拖过头（`offset.abs() > max`）：差值更大，仍然算在底部，不会翻不回来。
pub fn is_at_bottom(offset: f32, max: f32, margin: f32) -> bool {
    max - offset.abs() <= margin
}

/// 自动连续翻页的判据（各列表页在自己的 `render` 里问一句，见 `should_auto_page` 的调用点）。
///
/// 四条都得过，缺一条就会出现"看起来在滚但一直在发请求"：
/// 1. `has_more`：接口说还有下一页（历史那种"没拿到新游标就停"也归这条）；
/// 2. `items > 0`：空列表时的"到底"是**还没加载**，不是滚到底了，翻页只会原地打转；
/// 3. `!loading`：**没有在途请求**。少了这条，一帧渲染就能连发五个下一页（请求回来又触发下一帧），
///    而游标类分页（历史、图文）在途期间游标还没更新，发的其实是同一个请求；
/// 4. `scroll.scrollable()`：内容比视口短时 [`is_at_bottom`] 按定义给 `true`（零行程），
///    可那种列表本来就没必要翻页 —— 这一条也顺手挡住了"打开页面就自动捞三页"。
///
/// `render` 里读到的偏移来自**上一帧**的布局，所以刚加长列表那一帧可能还没反映新行程；
/// 效果只是"晚一帧才触发"，不会连发，可以接受。
pub fn should_auto_page(
    scroll: &DragScroll,
    has_more: bool,
    items: usize,
    loading: bool,
) -> bool {
    has_more && items > 0 && !loading && scroll.scrollable() && scroll.at_bottom(AUTO_PAGE_MARGIN)
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

    /// 现在离底部还有没有 `margin` 以内（自动连续翻页在 render 里问这一句）。
    ///
    /// 读的是**上一帧**的布局：`max_offset()` 由 paint 算出来，所以刚把列表加长的那一帧
    /// 还可能读到旧的行程。这就是各页的 `auto_load_more` 除了问这一句、还要自己看
    /// "有没有在途请求"和"列表非空"的原因（否则一帧能连发五个请求）。
    pub fn at_bottom(&self, margin: f32) -> bool {
        is_at_bottom(
            self.handle.offset().y.as_f32(),
            self.handle.max_offset().y.as_f32(),
            margin,
        )
    }

    /// 这一帧有没有真的可滚的行程（内容比视口短时是 `false`）。
    ///
    /// 自动翻页要它当第二个闸：一屏就装得下的列表**不需要**翻下一页，
    /// 而 [`is_at_bottom`] 在这种"零行程"情况下按定义是给 `true` 的。
    pub fn scrollable(&self) -> bool {
        self.handle.max_offset().y.as_f32() > 1.
    }

    /// 给滚动容器用：`.on_mouse_down(MouseButton::Left, page.scroll.on_press())`。
    ///
    /// 只记起点，不 `stop_propagation`、不吞事件：按下要照常落到里面的卡片与按钮上，
    /// 真正的"滚"只在下一步移动里发生（没移动就是点击，一切照旧）。
    /// 例外是 P11 加的那条：**按在可选中的文字上就不记起点**（见文件头），
    /// 顺带把已有的起点清掉，免得上一段拖动的起点在这一次移动里被复用。
    pub fn on_press(&self) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
        let handle = self.handle.clone();
        let drag = self.drag.clone();
        move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
            if TextSelection::is_selectable_at(event.position, window, cx) {
                *drag.borrow_mut() = None;
                return;
            }
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
