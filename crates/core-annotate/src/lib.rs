//! Vector annotation objects, undo history, hit-testing, and rasterization.

mod document;
mod raster;

pub use document::{Annotation, AnnotationDocument, AnnotationId, AnnotationStyle, Shape};
pub use raster::{RasterError, measure_text_height, rasterize};
