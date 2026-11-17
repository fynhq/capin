use std::sync::Arc;

use core_types::{PhysicalSize, RgbaFrame};
use gpui_kit::*;

use crate::image_util;

use super::{PinKind, text_pin_size, text_signature};

#[derive(Clone)]
pub(super) enum PinContent {
    Image {
        frame: Option<Arc<RgbaFrame>>,
        image: Arc<RenderImage>,
        display: PhysicalSize,
        size: PhysicalSize,
        hash: u64,
    },
    Text(String),
}

impl PinContent {
    pub(super) fn from_frame(frame: RgbaFrame) -> Self {
        let hash = crate::capture_history::hash_frame(&frame);
        Self::from_frame_with_hash(frame, hash)
    }

    pub(super) fn from_frame_with_hash(frame: RgbaFrame, hash: u64) -> Self {
        Self::from_frame_shared(Arc::new(frame), hash)
    }

    pub(super) fn from_frame_shared(frame: Arc<RgbaFrame>, hash: u64) -> Self {
        let size = PhysicalSize::new(frame.width(), frame.height());
        let image = image_util::render_image_ref(&frame);
        Self::Image {
            frame: Some(frame),
            image,
            display: size,
            size,
            hash,
        }
    }

    pub(super) fn display_size(&self) -> Option<PhysicalSize> {
        match self {
            Self::Image { display, .. } => Some(*display),
            Self::Text(_) => None,
        }
    }

    pub(super) fn set_display_image(
        &mut self,
        image: Arc<RenderImage>,
        display: PhysicalSize,
    ) -> Option<Arc<RenderImage>> {
        match self {
            Self::Image {
                image: current,
                display: current_display,
                ..
            } => {
                *current_display = display;
                Some(std::mem::replace(current, image))
            }
            Self::Text(_) => None,
        }
    }

    pub(super) fn frame(&self) -> Option<&RgbaFrame> {
        match self {
            Self::Image { frame, .. } => frame.as_deref(),
            _ => None,
        }
    }

    pub(super) fn frame_arc(&self) -> Option<Arc<RgbaFrame>> {
        match self {
            Self::Image { frame, .. } => frame.as_ref().map(Arc::clone),
            _ => None,
        }
    }

    pub(super) fn drop_frame(&mut self) {
        if let Self::Image { frame, .. } = self {
            *frame = None;
        }
    }

    pub(super) fn hash(&self) -> Option<u64> {
        match self {
            Self::Image { hash, .. } => Some(*hash),
            Self::Text(text) => Some(text_signature(text)),
        }
    }

    pub(super) fn image_size(&self) -> Option<PhysicalSize> {
        match self {
            Self::Image { size, .. } => Some(*size),
            _ => None,
        }
    }

    pub(super) fn clipboard_text(&self) -> Option<String> {
        match self {
            Self::Image { .. } => None,
            Self::Text(text) => Some(text.clone()),
        }
    }

    pub(super) fn kind(&self) -> PinKind {
        match self {
            Self::Image { .. } => PinKind::Image,
            Self::Text(text) => PinKind::Text(text.clone()),
        }
    }

    pub(super) fn preferred_zoom(&self) -> f32 {
        match self {
            Self::Image { size, .. } => (900.0 / size.width as f32)
                .min(700.0 / size.height as f32)
                .min(1.0),
            Self::Text(_) => 1.0,
        }
    }

    pub(super) fn size(&self, zoom: f32) -> (f32, f32) {
        match self {
            Self::Image { size, .. } => (size.width as f32 * zoom, size.height as f32 * zoom),
            Self::Text(text) => text_pin_size(text),
        }
    }
}
