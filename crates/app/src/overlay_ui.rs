//! Pure overlay geometry and chrome helpers.
//!
//! The interaction state and window plumbing stay in `overlay`; these helpers
//! only map physical rectangles to the elements drawn above the frozen frame.

use core_types::PhysicalRect;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::icons;
use crate::selection_geometry::MIN_REGION_SIZE;

pub(crate) fn clamp_to_monitor(rect: PhysicalRect, size: core_types::PhysicalSize) -> PhysicalRect {
    let width = rect.size.width.clamp(1, size.width);
    let height = rect.size.height.clamp(1, size.height);
    let max_x = size.width.saturating_sub(width) as i32;
    let max_y = size.height.saturating_sub(height) as i32;
    PhysicalRect::new(
        rect.origin.x.clamp(0, max_x.max(0)),
        rect.origin.y.clamp(0, max_y.max(0)),
        width,
        height,
    )
}

pub(crate) fn fit_ratio_rect(
    rect: PhysicalRect,
    ratio: f32,
    size: core_types::PhysicalSize,
) -> PhysicalRect {
    let max_width = (size.width as f32).min(size.height as f32 * ratio).max(1.0);
    let width = (rect.size.width as f32).min(max_width).max(1.0);
    let height = (width / ratio).max(1.0);
    let center_x = rect.origin.x as f32 + rect.size.width as f32 / 2.0;
    let center_y = rect.origin.y as f32 + rect.size.height as f32 / 2.0;
    let clamped = PhysicalRect::new(
        (center_x - width / 2.0).round() as i32,
        (center_y - height / 2.0).round() as i32,
        width.round() as u32,
        height.round() as u32,
    );
    clamp_to_monitor(clamped, size)
}

/// Keeps a ratio-locked selection inside the monitor while preserving the
/// requested edge. Used by the info-bar width/height editor, where clamping
/// only one dimension used to leave a 16:9 box at an arbitrary ratio.
pub(crate) fn fit_ratio_to_bounds(
    desired_width: i32,
    ratio: f32,
    max_width: i32,
    max_height: i32,
) -> (i32, i32) {
    let ratio = ratio.max(0.01);
    let max_width = max_width.max(MIN_REGION_SIZE);
    let max_height = max_height.max(MIN_REGION_SIZE);
    let mut width = desired_width.max(MIN_REGION_SIZE) as f32;
    let mut height = (width / ratio).max(MIN_REGION_SIZE as f32);
    if width > max_width as f32 {
        width = max_width as f32;
        height = (width / ratio).max(MIN_REGION_SIZE as f32);
    }
    if height > max_height as f32 {
        height = max_height as f32;
        width = (height * ratio).max(MIN_REGION_SIZE as f32);
    }
    if width > max_width as f32 {
        width = max_width as f32;
        height = (width / ratio).max(MIN_REGION_SIZE as f32);
    }
    (
        (width.round() as i32).clamp(MIN_REGION_SIZE, max_width),
        (height.round() as i32).clamp(MIN_REGION_SIZE, max_height),
    )
}

pub(crate) fn border(rect: Bounds<Pixels>, color: Rgba, radius: f32) -> Div {
    div()
        .absolute()
        .left(rect.origin.x)
        .top(rect.origin.y)
        .w(rect.size.width)
        .h(rect.size.height)
        .rounded(px(radius.max(0.0)))
        .border_3()
        .border_color(color)
}

pub(crate) fn element_border(rect: Bounds<Pixels>, color: Rgba) -> Div {
    div()
        .absolute()
        .left(rect.origin.x)
        .top(rect.origin.y)
        .w(rect.size.width)
        .h(rect.size.height)
        .border_2()
        .border_dashed()
        .border_color(color)
}

pub(crate) fn screen_border(rect: Bounds<Pixels>, color: Rgba) -> Div {
    div()
        .absolute()
        .left(rect.origin.x + px(1.0))
        .top(rect.origin.y + px(1.0))
        .w(px((f32::from(rect.size.width) - 2.0).max(0.0)))
        .h(px((f32::from(rect.size.height) - 2.0).max(0.0)))
        .border_2()
        .border_dashed()
        .border_color(rgba((u32::from(color) & 0x00FF_FFFF) << 8 | 0xCC))
}

pub(crate) fn camera_cursor(x: f32, y: f32, dark: bool) -> Div {
    let (background, border, icon, shadow) = if dark {
        (rgba(0x000000C0), rgba(0xFFFFFF80), rgb(0xFFFFFF), 0.35)
    } else {
        (rgba(0xFFFFFFE8), rgba(0x00000033), rgb(0x1B1B1B), 0.18)
    };
    div()
        .absolute()
        .left(px(x + 2.0))
        .top(px(y + 2.0))
        .w(px(28.0))
        .h(px(28.0))
        .rounded(px(7.0))
        .bg(background)
        .border_1()
        .border_color(border)
        .shadow(vec![
            BoxShadow::new(px(0.0), px(2.0), hsla(0.0, 0.0, 0.0, shadow)).blur_radius(px(6.0)),
        ])
        .flex()
        .items_center()
        .justify_center()
        .child(icons::icon(icons::CAMERA, 17.0, icon))
}
