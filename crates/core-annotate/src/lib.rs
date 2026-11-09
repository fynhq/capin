//! Vector annotation objects, undo history, hit-testing, and rasterization.

mod document;
mod raster;

pub use document::{
    Annotation, AnnotationDocument, AnnotationId, AnnotationStyle, NumberStyle, Shape,
    TextAlignment,
};
pub use raster::{RasterError, badge_foreground, measure_text_height, rasterize};
