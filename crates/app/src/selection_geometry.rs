//! Shared geometry for the capture selection frame and the inline editor's
//! export-frame handles. Keeping it outside `overlay.rs` also breaks the
//! editor <-> overlay module cycle.

use core_types::{PhysicalPoint, PhysicalRect, PhysicalSize};
use gpui_kit::*;

pub(crate) const MIN_REGION_SIZE: i32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResizeHandle {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

pub(crate) fn region_handle_cursor(handle: ResizeHandle) -> CursorStyle {
    match handle {
        ResizeHandle::North | ResizeHandle::South => CursorStyle::ResizeUpDown,
        ResizeHandle::East | ResizeHandle::West => CursorStyle::ResizeLeftRight,
        ResizeHandle::NorthEast | ResizeHandle::SouthWest => CursorStyle::ResizeUpRightDownLeft,
        ResizeHandle::NorthWest | ResizeHandle::SouthEast => CursorStyle::ResizeUpLeftDownRight,
    }
}

fn handle_has_west(handle: ResizeHandle) -> bool {
    matches!(
        handle,
        ResizeHandle::West | ResizeHandle::NorthWest | ResizeHandle::SouthWest
    )
}

fn handle_has_east(handle: ResizeHandle) -> bool {
    matches!(
        handle,
        ResizeHandle::East | ResizeHandle::NorthEast | ResizeHandle::SouthEast
    )
}

fn handle_has_north(handle: ResizeHandle) -> bool {
    matches!(
        handle,
        ResizeHandle::North | ResizeHandle::NorthEast | ResizeHandle::NorthWest
    )
}

fn handle_has_south(handle: ResizeHandle) -> bool {
    matches!(
        handle,
        ResizeHandle::South | ResizeHandle::SouthEast | ResizeHandle::SouthWest
    )
}

/// Maps a press near a selection edge or corner to the matching resize handle.
/// The painted dots stay small, while the whole border becomes a grab target
/// so the selection can be adjusted again after the initial drag.
pub(crate) fn edge_resize_handle(
    rect: PhysicalRect,
    point: PhysicalPoint,
    tolerance: f32,
) -> Option<ResizeHandle> {
    let left = rect.origin.x as f32;
    let top = rect.origin.y as f32;
    let right = left + rect.size.width as f32;
    let bottom = top + rect.size.height as f32;
    let x = point.x as f32;
    let y = point.y as f32;
    if x < left - tolerance
        || x > right + tolerance
        || y < top - tolerance
        || y > bottom + tolerance
    {
        return None;
    }
    let west = (x - left).abs() <= tolerance;
    let east = (x - right).abs() <= tolerance;
    let north = (y - top).abs() <= tolerance;
    let south = (y - bottom).abs() <= tolerance;
    match (west, east, north, south) {
        (true, _, true, _) => Some(ResizeHandle::NorthWest),
        (_, true, true, _) => Some(ResizeHandle::NorthEast),
        (true, _, _, true) => Some(ResizeHandle::SouthWest),
        (_, true, _, true) => Some(ResizeHandle::SouthEast),
        (true, _, _, _) => Some(ResizeHandle::West),
        (_, true, _, _) => Some(ResizeHandle::East),
        (_, _, true, _) => Some(ResizeHandle::North),
        (_, _, _, true) => Some(ResizeHandle::South),
        _ => None,
    }
}

pub(crate) fn resize_rect(
    original: PhysicalRect,
    handle: ResizeHandle,
    cursor: PhysicalPoint,
    ratio: Option<f32>,
    shift: bool,
    alt: bool,
    size: PhysicalSize,
) -> PhysicalRect {
    let original_left = i64::from(original.origin.x);
    let original_top = i64::from(original.origin.y);
    let original_right = original_left + i64::from(original.size.width);
    let original_bottom = original_top + i64::from(original.size.height);
    let center_x = (original_left + original_right) / 2;
    let center_y = (original_top + original_bottom) / 2;
    let cursor_x = i64::from(cursor.x).clamp(0, i64::from(size.width));
    let cursor_y = i64::from(cursor.y).clamp(0, i64::from(size.height));

    let mut left = original_left;
    let mut top = original_top;
    let mut right = original_right;
    let mut bottom = original_bottom;
    if handle_has_west(handle) {
        left = cursor_x;
    }
    if handle_has_east(handle) {
        right = cursor_x;
    }
    if handle_has_north(handle) {
        top = cursor_y;
    }
    if handle_has_south(handle) {
        bottom = cursor_y;
    }

    if alt {
        if handle_has_west(handle) {
            left = center_x * 2 - right;
        } else if handle_has_east(handle) {
            right = center_x * 2 - left;
        }
        if handle_has_north(handle) {
            top = center_y * 2 - bottom;
        } else if handle_has_south(handle) {
            bottom = center_y * 2 - top;
        }
    }

    if right < left + i64::from(MIN_REGION_SIZE) {
        if handle_has_west(handle) {
            left = right - i64::from(MIN_REGION_SIZE);
        } else {
            right = left + i64::from(MIN_REGION_SIZE);
        }
    }
    if bottom < top + i64::from(MIN_REGION_SIZE) {
        if handle_has_north(handle) {
            top = bottom - i64::from(MIN_REGION_SIZE);
        } else {
            bottom = top + i64::from(MIN_REGION_SIZE);
        }
    }

    let ratio = ratio.or_else(|| shift.then_some(1.0));
    if let Some(ratio) = ratio {
        let anchor_right = handle_has_west(handle);
        let anchor_bottom = handle_has_north(handle);
        let horizontal = handle_has_west(handle) || handle_has_east(handle);
        let mut width = (right - left).max(i64::from(MIN_REGION_SIZE));
        let mut height = (bottom - top).max(i64::from(MIN_REGION_SIZE));
        if horizontal {
            height = (width as f32 / ratio).round().max(1.0) as i64;
        } else {
            width = (height as f32 * ratio).round().max(1.0) as i64;
        }
        let max_width = if anchor_right {
            right
        } else {
            i64::from(size.width) - left
        };
        let max_height = if anchor_bottom {
            bottom
        } else {
            i64::from(size.height) - top
        };
        if width > max_width.max(1) || height > max_height.max(1) {
            let scale = (max_width as f32 / width as f32)
                .min(max_height as f32 / height as f32)
                .max(0.0);
            width = (width as f32 * scale).max(1.0) as i64;
            height = (height as f32 * scale).max(1.0) as i64;
        }
        if anchor_right {
            left = right - width;
        } else {
            right = left + width;
        }
        if anchor_bottom {
            top = bottom - height;
        } else {
            bottom = top + height;
        }
    }

    left = left.clamp(0, i64::from(size.width).saturating_sub(1));
    top = top.clamp(0, i64::from(size.height).saturating_sub(1));
    right = right.clamp(left + 1, i64::from(size.width));
    bottom = bottom.clamp(top + 1, i64::from(size.height));
    PhysicalRect::new(
        left as i32,
        top as i32,
        (right - left) as u32,
        (bottom - top) as u32,
    )
}

pub(crate) fn handle_positions(rect: Bounds<Pixels>) -> Vec<(ResizeHandle, Point<Pixels>)> {
    let left = f32::from(rect.origin.x);
    let top = f32::from(rect.origin.y);
    let right = left + f32::from(rect.size.width);
    let bottom = top + f32::from(rect.size.height);
    let mid_x = (left + right) / 2.0;
    let mid_y = (top + bottom) / 2.0;
    vec![
        (ResizeHandle::North, point(px(mid_x), px(top))),
        (ResizeHandle::South, point(px(mid_x), px(bottom))),
        (ResizeHandle::West, point(px(left), px(mid_y))),
        (ResizeHandle::East, point(px(right), px(mid_y))),
        (ResizeHandle::NorthWest, point(px(left), px(top))),
        (ResizeHandle::NorthEast, point(px(right), px(top))),
        (ResizeHandle::SouthWest, point(px(left), px(bottom))),
        (ResizeHandle::SouthEast, point(px(right), px(bottom))),
    ]
}

/// Corner handles normally sit on the square corner. On a rounded selection
/// that point falls outside the visible shape and the white handle floats in
/// the dimmed corner, so pull it onto the 45-degree point of the arc.
pub(crate) fn corner_handle_offset(handle: ResizeHandle, radius: f32) -> (f32, f32) {
    let inset = radius.max(0.0) * (1.0 - std::f32::consts::FRAC_1_SQRT_2);
    match handle {
        ResizeHandle::NorthWest => (inset, inset),
        ResizeHandle::NorthEast => (-inset, inset),
        ResizeHandle::SouthWest => (inset, -inset),
        ResizeHandle::SouthEast => (-inset, -inset),
        _ => (0.0, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::{ResizeHandle, edge_resize_handle};
    use core_types::{PhysicalPoint, PhysicalRect};

    #[test]
    fn selection_borders_map_to_resize_handles() {
        let rect = PhysicalRect::new(10, 20, 100, 80);
        let handle = |x, y| edge_resize_handle(rect, PhysicalPoint::new(x, y), 6.0);
        assert_eq!(handle(110, 60), Some(ResizeHandle::East));
        assert_eq!(handle(10, 100), Some(ResizeHandle::SouthWest));
        assert_eq!(handle(60, 20), Some(ResizeHandle::North));
        assert_eq!(handle(110, 100), Some(ResizeHandle::SouthEast));
        assert_eq!(handle(60, 60), None);
        assert_eq!(handle(140, 60), None);
    }
}
