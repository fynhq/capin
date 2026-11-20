use super::*;

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

pub(super) fn resolve_display_id(
    saved: Option<u64>,
    available: &[DisplayId],
    primary: Option<DisplayId>,
) -> Option<DisplayId> {
    saved
        .map(DisplayId::new)
        .filter(|id| available.contains(id))
        .or(primary)
}

pub(super) fn saved_display_id(cx: &App, saved: Option<u64>) -> Option<DisplayId> {
    let available: Vec<DisplayId> = cx.displays().iter().map(|display| display.id()).collect();
    resolve_display_id(
        saved,
        &available,
        cx.primary_display().map(|display| display.id()),
    )
}

pub(super) fn clamp_to_display(
    cx: &App,
    bounds: Bounds<Pixels>,
    display_id: Option<DisplayId>,
) -> Bounds<Pixels> {
    clamp_bounds_to_display(bounds, pin_open_display_bounds(cx, bounds, display_id))
}

pub(super) fn clamp_bounds_to_display(
    bounds: Bounds<Pixels>,
    display: Option<Bounds<Pixels>>,
) -> Bounds<Pixels> {
    let Some(display) = display else {
        return bounds;
    };
    let left = f32::from(display.origin.x);
    let top = f32::from(display.origin.y);
    let display_width = f32::from(display.size.width);
    let display_height = f32::from(display.size.height);
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let origin_x = f32::from(bounds.origin.x);
    let origin_y = f32::from(bounds.origin.y);
    Bounds {
        origin: point(
            px(if width <= display_width {
                origin_x.clamp(left, left + display_width - width)
            } else {
                left + (display_width - width) / 2.0
            }),
            px(if height <= display_height {
                origin_y.clamp(top, top + display_height - height)
            } else {
                top + (display_height - height) / 2.0
            }),
        ),
        size: bounds.size,
    }
}

pub(super) fn pin_open_display_bounds(
    cx: &App,
    bounds: Bounds<Pixels>,
    display_id: Option<DisplayId>,
) -> Option<Bounds<Pixels>> {
    let displays = cx.displays();
    if let Some(id) = display_id
        && let Some(display) = displays.iter().find(|display| display.id() == id)
    {
        return Some(display.bounds());
    }
    let center_x = bounds.origin.x + bounds.size.width / 2.0;
    let center_y = bounds.origin.y + bounds.size.height / 2.0;
    displays
        .iter()
        .find(|display| {
            let bounds = display.bounds();
            center_x >= bounds.origin.x
                && center_x < bounds.origin.x + bounds.size.width
                && center_y >= bounds.origin.y
                && center_y < bounds.origin.y + bounds.size.height
        })
        .map(|display| display.bounds())
        .or_else(|| cx.primary_display().map(|display| display.bounds()))
}

pub(super) fn fit_pin_to_display(
    content: &PinContent,
    bounds: Bounds<Pixels>,
    zoom: f32,
    display: Option<Bounds<Pixels>>,
) -> (Bounds<Pixels>, f32) {
    let mut zoom = zoom.clamp(MIN_PIN_ZOOM, MAX_PIN_ZOOM);
    if let (Some(size), Some(display)) = (content.image_size(), display) {
        let fit = fit_zoom_limit(
            size.width,
            size.height,
            f32::from(display.size.width),
            f32::from(display.size.height),
        );
        zoom = zoom.min(fit);
    }
    let (width, height) = content.size(zoom);
    let center_x = f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.0;
    let center_y = f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0;
    let mut origin_x = center_x - width / 2.0;
    let mut origin_y = center_y - height / 2.0;
    if let Some(display) = display {
        let left = f32::from(display.origin.x);
        let top = f32::from(display.origin.y);
        let screen_width = f32::from(display.size.width);
        let screen_height = f32::from(display.size.height);
        origin_x = if width <= screen_width {
            origin_x.clamp(left, left + screen_width - width)
        } else {
            left + (screen_width - width) / 2.0
        };
        origin_y = if height <= screen_height {
            origin_y.clamp(top, top + screen_height - height)
        } else {
            top + (screen_height - height) / 2.0
        };
    }
    (
        Bounds {
            origin: point(px(origin_x), px(origin_y)),
            size: size(px(width), px(height)),
        },
        zoom,
    )
}

pub(super) fn fit_zoom_limit(
    image_width: u32,
    image_height: u32,
    screen_width: f32,
    screen_height: f32,
) -> f32 {
    if image_width == 0 || image_height == 0 || screen_width <= 0.0 || screen_height <= 0.0 {
        return MAX_PIN_ZOOM;
    }
    (screen_width / image_width as f32)
        .min(screen_height / image_height as f32)
        .clamp(MIN_PIN_ZOOM, MAX_PIN_ZOOM)
}

pub(super) fn pin_display_bounds(window: &Window, cx: &App) -> Option<Bounds<Pixels>> {
    window
        .display(cx)
        .map(|display| display.bounds())
        .or_else(|| cx.primary_display().map(|display| display.bounds()))
}
