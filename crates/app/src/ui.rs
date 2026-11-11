//! Small shared UI helpers used by the auxiliary windows.

use std::time::Duration;

#[cfg(any(target_os = "windows", test))]
use core_types::PhysicalRect;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::chrome_theme::ChromeTheme;
use crate::icons;

/// Centers a window on the primary display, or falls back to a stable origin
/// when no display information is available.
pub fn centered_bounds(cx: &App, width: f32, height: f32) -> Bounds<Pixels> {
    match cx.primary_display() {
        Some(display) => {
            let center = display.bounds().center();
            Bounds {
                origin: point(center.x - px(width / 2.0), center.y - px(height / 2.0)),
                size: size(px(width), px(height)),
            }
        }
        None => Bounds {
            origin: point(px(80.0), px(80.0)),
            size: size(px(width), px(height)),
        },
    }
}

/// Converts logical window bounds into the physical rectangle used to
/// re-place a popup client area. Origins round to the nearest physical pixel;
/// sizes round up so fractional scaling never clips the content.
#[cfg(any(target_os = "windows", test))]
pub fn physical_window_rect(bounds: Bounds<Pixels>, scale: f32) -> PhysicalRect {
    let scale = scale.max(0.1);
    PhysicalRect::new(
        (f32::from(bounds.origin.x) * scale).round() as i32,
        (f32::from(bounds.origin.y) * scale).round() as i32,
        (f32::from(bounds.size.width) * scale).ceil().max(1.0) as u32,
        (f32::from(bounds.size.height) * scale).ceil().max(1.0) as u32,
    )
}

/// Re-places a popup's client area on Windows after GPUI has applied its own
/// frame offset. Other platforms place the window in display-local points and
/// do not need the correction.
pub fn align_client_to_bounds(window: &Window, bounds: Bounds<Pixels>) {
    #[cfg(target_os = "windows")]
    {
        let scale = window.scale_factor().max(0.1);
        let physical = physical_window_rect(bounds, scale);
        let dpi = core_types::DpiScale::new(scale).unwrap_or_default();
        let _ = platform_window::place_on_monitor_deferred(window, physical, dpi);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, bounds);
    }
}

/// Appends a rounded-rectangle path to `builder`, clamping the radius to half
/// the shorter side.
pub fn push_rounded_rect(builder: &mut PathBuilder, rect: Bounds<Pixels>, radius: f32) {
    let left = f32::from(rect.origin.x);
    let top = f32::from(rect.origin.y);
    let width = f32::from(rect.size.width);
    let height = f32::from(rect.size.height);
    let radius = radius.clamp(0.0, width.min(height) / 2.0);
    let right = left + width;
    let bottom = top + height;
    builder.move_to(point(px(left + radius), px(top)));
    builder.line_to(point(px(right - radius), px(top)));
    builder.curve_to(
        point(px(right), px(top + radius)),
        point(px(right), px(top)),
    );
    builder.line_to(point(px(right), px(bottom - radius)));
    builder.curve_to(
        point(px(right - radius), px(bottom)),
        point(px(right), px(bottom)),
    );
    builder.line_to(point(px(left + radius), px(bottom)));
    builder.curve_to(
        point(px(left), px(bottom - radius)),
        point(px(left), px(bottom)),
    );
    builder.line_to(point(px(left), px(top + radius)));
    builder.curve_to(point(px(left + radius), px(top)), point(px(left), px(top)));
    builder.close();
}

/// Places a popover beside its anchor without covering the anchor or another
/// chrome bar. Preferring the space below keeps a card from clamping back over
/// the info bar when the selection sits near the bottom of the screen.
pub fn popover_top(
    anchor: Bounds<Pixels>,
    height: f32,
    window_height: f32,
    avoid: Option<Bounds<Pixels>>,
) -> f32 {
    const GAP: f32 = 6.0;
    let above = f32::from(anchor.origin.y) - height - GAP;
    let below = f32::from(anchor.origin.y) + f32::from(anchor.size.height) + GAP;
    let fits_above = above >= 8.0;
    let fits_below = below + height <= window_height - 8.0;
    let overlaps = |top: f32| {
        avoid.is_some_and(|bar| {
            top < f32::from(bar.origin.y) + f32::from(bar.size.height)
                && top + height > f32::from(bar.origin.y)
        })
    };
    if fits_below && !overlaps(below) {
        return below;
    }
    if fits_above && !overlaps(above) {
        return above;
    }
    if fits_below {
        return below;
    }
    if fits_above {
        return above;
    }
    8.0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarButtonTone {
    Neutral,
    Accent,
    Danger,
}

/// Compact action button used by the long-capture and recording bars. The
/// native flyout surface and one leading glyph keep both bars consistent with
/// the screenshot chrome instead of the old dark text-only pills.
pub fn bar_button<View: 'static>(
    id: &'static str,
    label: String,
    icon: Option<&'static [u8]>,
    tone: BarButtonTone,
    theme: &ChromeTheme,
    cx: &mut Context<View>,
    action: fn(&mut View, &mut Context<View>),
) -> AnyElement {
    let theme = *theme;
    let (background, foreground) = match tone {
        BarButtonTone::Neutral => (rgba(0x00000000), theme.text),
        BarButtonTone::Accent => (theme.accent, theme.accent_text),
        BarButtonTone::Danger => (rgb(0xFF453A), rgb(0xFFFFFF)),
    };
    let hover = match tone {
        BarButtonTone::Neutral => theme.hover,
        BarButtonTone::Accent => opacity_color(theme.accent, 0.88),
        BarButtonTone::Danger => opacity_color(rgb(0xFF453A), 0.88),
    };
    div()
        .id(id)
        .h(px(28.0))
        .px(px(10.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(5.0))
        .flex_shrink_0()
        .rounded(px(6.0))
        .bg(background)
        .text_size(px(11.5))
        .text_color(foreground)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .active(|style| style.opacity(0.85))
        .on_click(cx.listener(move |this, _, _, cx| action(this, cx)))
        .children(icon.map(|data| icons::icon(data, 14.0, foreground)))
        .child(label)
        .into_any_element()
}

fn opacity_color(color: Rgba, opacity: f32) -> Rgba {
    rgba((u32::from(color) & 0x00FF_FFFF) << 8 | ((opacity * 255.0) as u32))
}

/// Native text caret blink. `repeat_synced` keeps multiple carets in phase and
/// the low frame cap avoids waking the renderer at monitor refresh just to
/// toggle a one-pixel bar.
pub fn caret_blink() -> Animation {
    Animation::new(Duration::from_millis(530))
        .repeat_synced()
        .with_max_fps(4.0)
        .with_easing(|delta| if delta < 0.5 { 1.0 } else { 0.0 })
}

fn ease_out_cubic(delta: f32) -> f32 {
    1.0 - (1.0 - delta).powi(3)
}

/// Grab cursor for a movable object. gpui's Windows backend maps
/// `OpenHand`/`ClosedHand` to the plain arrow, so Windows uses the pointing
/// hand that the backend does implement.
pub fn open_hand_cursor() -> CursorStyle {
    #[cfg(target_os = "windows")]
    {
        CursorStyle::PointingHand
    }
    #[cfg(not(target_os = "windows"))]
    {
        CursorStyle::OpenHand
    }
}

/// Dragging variant of [`open_hand_cursor`], with the same Windows fallback.
pub fn closed_hand_cursor() -> CursorStyle {
    #[cfg(target_os = "windows")]
    {
        CursorStyle::PointingHand
    }
    #[cfg(not(target_os = "windows"))]
    {
        CursorStyle::ClosedHand
    }
}

/// Fades and slightly slides a transient surface in. Only the entrance is
/// animated; conditional panels disappear immediately so dismissing a menu
/// never feels delayed.
pub fn enter_slide_fade<E>(
    element: E,
    id: impl Into<ElementId>,
    base_top: f32,
    offset: f32,
    duration: Duration,
) -> AnimationElement<E>
where
    E: IntoElement + Styled + 'static,
{
    element.with_animation(
        id,
        Animation::new(duration).with_easing(ease_out_cubic),
        move |element, delta| {
            element
                .opacity(delta)
                .top(px(base_top + (1.0 - delta) * offset))
        },
    )
}

/// Fades a surface in without changing its layout position.
pub fn enter_fade<E>(
    element: E,
    id: impl Into<ElementId>,
    duration: Duration,
) -> AnimationElement<E>
where
    E: IntoElement + Styled + 'static,
{
    element.with_animation(
        id,
        Animation::new(duration).with_easing(ease_out_cubic),
        |element, delta| element.opacity(delta),
    )
}

/// Converts a hue in degrees to a saturated RGB swatch used by the custom
/// color panels.
pub fn hue_rgb(hue: f32) -> u32 {
    let hue = hue.rem_euclid(360.0) / 60.0;
    let saturation = 0.82;
    let value = 0.95;
    let chroma = value * saturation;
    let secondary = chroma * (1.0 - (hue % 2.0 - 1.0).abs());
    let (red, green, blue) = match hue as u32 {
        0 => (chroma, secondary, 0.0),
        1 => (secondary, chroma, 0.0),
        2 => (0.0, chroma, secondary),
        3 => (0.0, secondary, chroma),
        4 => (secondary, 0.0, chroma),
        _ => (chroma, 0.0, secondary),
    };
    let offset = value - chroma;
    let channel = |value: f32| ((value + offset) * 255.0).round().clamp(0.0, 255.0) as u32;
    (channel(red) << 16) | (channel(green) << 8) | channel(blue)
}

#[cfg(test)]
mod tests {
    use super::physical_window_rect;
    use core_types::PhysicalRect;
    use gpui_kit::{Bounds, point, px, size};

    #[test]
    fn physical_window_rect_rounds_origins_and_ceils_sizes() {
        let bounds = Bounds {
            origin: point(px(10.0), px(20.6)),
            size: size(px(100.2), px(50.1)),
        };
        assert_eq!(
            physical_window_rect(bounds, 1.25),
            PhysicalRect::new(13, 26, 126, 63)
        );
    }

    #[test]
    fn physical_window_rect_keeps_a_minimum_pixel() {
        let bounds = Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(0.0), px(0.0)),
        };
        assert_eq!(
            physical_window_rect(bounds, 1.0),
            PhysicalRect::new(0, 0, 1, 1)
        );
    }
}
