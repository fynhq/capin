use std::sync::Arc;

use core_types::{PhysicalSize, RgbaFrame};

#[derive(Clone)]
pub(super) enum PinBase {
    Frame(Arc<RgbaFrame>),
    Encoded {
        bytes: Arc<[u8]>,
        size: PhysicalSize,
    },
}

impl PinBase {
    pub(super) fn frame(&self) -> Option<Arc<RgbaFrame>> {
        match self {
            Self::Frame(frame) => Some(frame.clone()),
            Self::Encoded { bytes, size } => {
                let image = image::load_from_memory(bytes).ok()?.into_rgba8();
                if image.dimensions() != (size.width, size.height) {
                    return None;
                }
                let frame =
                    RgbaFrame::from_pixels(size.width, size.height, image.into_raw()).ok()?;
                Some(Arc::new(frame))
            }
        }
    }
}
