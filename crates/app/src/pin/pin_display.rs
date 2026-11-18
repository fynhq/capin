use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::image_util;
use crate::logging::log_warn;

use super::{MIN_PIN_ZOOM, PinContent, PinView, fit_zoom_limit, pin_display_bounds};

impl PinView {
    pub(super) fn fitted_zoom_limit(&self, window: &Window, cx: &App) -> f32 {
        let Some(size) = self.content.image_size() else {
            return 4.0;
        };
        let Some(screen) = pin_display_bounds(window, cx) else {
            return 4.0;
        };
        fit_zoom_limit(
            size.width,
            size.height,
            f32::from(screen.size.width),
            f32::from(screen.size.height),
        )
    }

    pub(super) fn fit_display_to_size(
        &mut self,
        scale: f32,
        width: f32,
        height: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(original) = self.content.image_size() else {
            return;
        };
        let Some(frame) = self.content.frame_arc() else {
            return;
        };
        let Some(current) = self.content.display_size() else {
            return;
        };
        let scale = scale.max(0.1);
        let target_width = (width * scale).ceil().max(1.0) as u32;
        let target_height = (height * scale).ceil().max(1.0) as u32;
        if target_width >= original.width && target_height >= original.height {
            return;
        }
        if target_width >= current.width && target_height >= current.height {
            return;
        }
        let Some((image, display)) =
            image_util::downscaled_render_image(&frame, target_width, target_height)
        else {
            return;
        };
        if let Some(previous) = self.content.set_display_image(image, display) {
            cx.drop_image(previous, None);
        }
    }

    pub(super) fn ensure_display_resolution(&mut self, window: &mut Window) {
        let Some(original) = self.content.image_size() else {
            return;
        };
        let Some(current) = self.content.display_size() else {
            return;
        };
        if current.width >= original.width && current.height >= original.height {
            return;
        }
        let scale = window.scale_factor().max(0.1);
        let (width, height) = self
            .content
            .size(self.values.borrow().zoom.max(MIN_PIN_ZOOM));
        let need_width = (width * scale).ceil().max(1.0) as u32;
        let need_height = (height * scale).ceil().max(1.0) as u32;
        if need_width as f32 <= current.width as f32 * 1.05
            && need_height as f32 <= current.height as f32 * 1.05
        {
            return;
        }
        let target_width = need_width
            .saturating_mul(5)
            .div_ceil(4)
            .min(original.width)
            .max(1);
        let target_height = need_height
            .saturating_mul(5)
            .div_ceil(4)
            .min(original.height)
            .max(1);
        let Some(frame) = self.frame_for_action() else {
            return;
        };
        let Some((image, display)) =
            image_util::downscaled_render_image(&frame, target_width, target_height)
        else {
            return;
        };
        if let Some(previous) = self.content.set_display_image(image, display)
            && let Err(error) = window.drop_image(previous)
        {
            log_warn!("pin", "failed to release previous display image: {error}");
        }
    }

    pub(super) fn release_image(&mut self, cx: &mut Context<Self>) {
        if let PinContent::Image { image, .. } = &self.content {
            cx.drop_image(image.clone(), None);
        }
    }
}
