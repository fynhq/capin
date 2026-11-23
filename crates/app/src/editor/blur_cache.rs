use std::sync::Arc;

use core_annotate::AnnotationId;
use core_types::{PhysicalPoint, PhysicalRect};

use super::*;

#[derive(Default)]
pub(super) struct BlurCache {
    previews: Vec<(AnnotationId, PhysicalRect, Arc<RenderImage>)>,
    key: Option<(u64, usize, Option<PhysicalRect>)>,
    draft: Option<(PhysicalRect, u32, Arc<RenderImage>)>,
}

impl BlurCache {
    pub(super) fn previews(&self) -> &[(AnnotationId, PhysicalRect, Arc<RenderImage>)] {
        &self.previews
    }

    pub(super) fn ensure(
        &mut self,
        document: &AnnotationDocument,
        frame: &Arc<RgbaFrame>,
        selection: Option<PhysicalRect>,
        cx: &mut Context<EditorView>,
    ) {
        let key = (
            document.history_serial(),
            Arc::as_ptr(frame) as usize,
            selection,
        );
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        let target = if let Some(selection) = selection {
            cropped_document(document, selection)
        } else {
            document.clone()
        };
        let offset = selection
            .map(|selection| selection.origin)
            .unwrap_or(PhysicalPoint::new(0, 0));
        let old_previews = std::mem::take(&mut self.previews);
        match core_annotate::blur_previews(&target, frame) {
            Ok(previews) => {
                self.previews = previews
                    .into_iter()
                    .map(|preview| {
                        let rect = PhysicalRect::new(
                            preview.rect.origin.x + offset.x,
                            preview.rect.origin.y + offset.y,
                            preview.rect.size.width,
                            preview.rect.size.height,
                        );
                        (preview.id, rect, image_util::render_image(preview.frame))
                    })
                    .collect();
            }
            Err(error) => {
                crate::logging::log_warn!("editor", "failed to rasterize blur previews: {error}");
            }
        }
        cx.defer(move |cx| {
            for (_, _, image) in old_previews {
                cx.drop_image(image, None);
            }
        });
    }

    pub(super) fn draft_image(
        &mut self,
        frame: &Arc<RgbaFrame>,
        selection: Option<PhysicalRect>,
        rect: PhysicalRect,
        radius: u32,
        cx: &mut Context<EditorView>,
    ) -> Option<(PhysicalRect, Arc<RenderImage>)> {
        if let Some((cached_rect, cached_radius, image)) = &self.draft
            && *cached_rect == rect
            && *cached_radius == radius
        {
            return Some((*cached_rect, image.clone()));
        }
        let offset = selection
            .map(|selection| selection.origin)
            .unwrap_or(PhysicalPoint::new(0, 0));
        let local = PhysicalRect::new(
            rect.origin.x - offset.x,
            rect.origin.y - offset.y,
            rect.size.width,
            rect.size.height,
        );
        let (clamped, blurred) = core_annotate::blur_frame_region(frame, local, radius)?;
        let placed = PhysicalRect::new(
            clamped.origin.x + offset.x,
            clamped.origin.y + offset.y,
            clamped.size.width,
            clamped.size.height,
        );
        let image = image_util::render_image(blurred);
        let old = self.draft.replace((placed, radius, image.clone()));
        if let Some((_, _, old_image)) = old {
            cx.defer(move |cx| {
                cx.drop_image(old_image, None);
            });
        }
        Some((placed, image))
    }

    pub(super) fn drop_images(&mut self, cx: &mut App) {
        let old_previews = std::mem::take(&mut self.previews);
        let old_draft = self.draft.take();
        self.key = None;
        cx.defer(move |cx| {
            for (_, _, image) in old_previews {
                cx.drop_image(image, None);
            }
            if let Some((_, _, image)) = old_draft {
                cx.drop_image(image, None);
            }
        });
    }

    pub(super) fn drop_images_now(&self, cx: &mut App) {
        for (_, _, image) in &self.previews {
            cx.drop_image(image.clone(), None);
        }
        if let Some((_, _, image)) = &self.draft {
            cx.drop_image(image.clone(), None);
        }
    }
}
