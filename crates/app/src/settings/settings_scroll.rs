use gpui_kit::base::{Scrollbar, ScrollbarMode, ScrollbarThumbStyle, ScrollbarTrackStyle};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::Theme;

pub(super) fn scrollbar(theme: &Theme, scroll: &ScrollHandle) -> AnyElement {
    let thumb = |opacity: f32| {
        move |style: ScrollbarThumbStyle| {
            style
                .bg(Hsla::from(theme.muted).opacity(opacity))
                .width(px(6.0))
                .radius(px(3.0))
                .min_length(px(24.0))
        }
    };
    let track = |style: ScrollbarTrackStyle| style.bg(hsla(0.0, 0.0, 0.0, 0.0));
    Scrollbar::vertical(scroll)
        .id("settings-scrollbar")
        .mode(ScrollbarMode::Scrolling)
        .styles(|styles| {
            styles
                .track(track)
                .track_hover(track)
                .track_active(track)
                .thumb(thumb(0.35))
                .thumb_hover(thumb(0.55))
                .thumb_active(thumb(0.7))
        })
        .into_any_element()
}
