//! Vector annotation objects, undo history, hit-testing, and rasterization.

mod document;
mod outline;
mod raster;

pub use document::{
    Annotation, AnnotationDocument, AnnotationId, AnnotationStyle, HIGHLIGHTER_ALPHA, Shape,
    TextAlignment, highlighter_color,
};
pub use outline::{OutlinePoint, stroke_outline};
pub use raster::{RasterError, badge_foreground, measure_text_height, rasterize};
