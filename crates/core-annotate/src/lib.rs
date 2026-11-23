mod document;
mod outline;
mod raster;

pub use document::{
    Annotation, AnnotationDocument, AnnotationId, AnnotationStyle, HIGHLIGHTER_ALPHA, Shape,
    TextAlignment, highlighter_color,
};
pub use outline::{OutlinePoint, stroke_outline};
pub use raster::{
    BlurPreview, RasterError, badge_foreground, blur_frame_region, blur_previews,
    measure_text_height, rasterize, warmup_text_engine,
};
