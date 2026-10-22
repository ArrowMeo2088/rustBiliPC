//! 二维码渲染：把登录 URL 编成模块矩阵，用 div 网格画出来。
//!
//! 为什么不走图片：gpui 的 `RenderImage::new` 需要 `image::Frame`，而那个 `image` 是 Zed 的
//! 分支（`gpui-pre-0.3.9/src/svg_renderer.rs:5` 的 `use image::Frame`），自己构造要连带引入
//! 它的缓冲类型；二维码用一格格 div 铺出来更简单，且模块数是固定的（二十几个 × 二十几个）。
//!
//! 依赖侧也只开 `qrcode-generator` 的 `["std","qr"]`，把默认的图片/SVG/PIC 渲染后端关掉，
//! 免得把 pixels 版的 `image` 也拖进同一棵树。

use gpui_kit::*;
use qrcode_generator::qr::{Encoder, ErrorCorrection};

/// 生成模块矩阵：`true` 是黑块。编码失败（内容过长等）返回空矩阵，由调用方显示占位。
pub fn matrix_for(text: &str) -> Vec<Vec<bool>> {
    match Encoder::new(ErrorCorrection::Medium).encode_text(text) {
        Ok(symbol) => symbol.to_matrix(),
        Err(error) => {
            log::error!("二维码编码失败: {error}");
            Vec::new()
        }
    }
}

/// 一个模块的边长（逻辑像素）。21~25 个模块的 QR，6 px 大约 130~150 px，手机上够扫。
pub const MODULE_PIXELS: f32 = 6.;

/// 把矩阵画成二维码。外层留一圈白（B 站 App 对留白不敏感，但有比没有稳）。
pub fn render(matrix: &[Vec<bool>]) -> AnyElement {
    let step = px(MODULE_PIXELS);

    div()
        .flex()
        .flex_col()
        .bg(rgb(0xFFFFFF))
        .p_2()
        .children(matrix.iter().map(|row| {
            div().flex().children(row.iter().map(|dark| {
                div()
                    .size(step)
                    .bg(if *dark { rgb(0x000000) } else { rgb(0xFFFFFF) })
            }))
        }))
        .into_any_element()
}
