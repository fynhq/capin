use core_types::{PhysicalPoint, PhysicalRect, PhysicalSize};

const HISTORY_LIMIT: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AnnotationId(u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlignment {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnnotationStyle {
    pub color: u32,
    pub stroke_width: f32,
    pub font_size: f32,
    pub corner_radius: f32,
    pub highlighter_radius: f32,
    pub blur_radius: u32,
    pub filled: bool,
    pub arrow_start: bool,
    pub arrow_end: bool,
    pub text_background: bool,
    pub text_outline: bool,
    pub text_alignment: TextAlignment,
    pub spotlight_intensity: u8,
    pub spotlight_radius: f32,
}

pub const HIGHLIGHTER_ALPHA: u32 = 0x66;

pub fn highlighter_color(color: u32) -> u32 {
    (color & 0xFFFF_FF00) | HIGHLIGHTER_ALPHA
}

impl Default for AnnotationStyle {
    fn default() -> Self {
        Self {
            color: 0xFF3B30FF,
            stroke_width: 3.0,
            font_size: 20.0,
            corner_radius: 0.0,
            highlighter_radius: 0.0,
            blur_radius: 16,
            filled: false,
            arrow_start: false,
            arrow_end: true,
            text_background: false,
            text_outline: false,
            text_alignment: TextAlignment::Left,
            spotlight_intensity: 60,
            spotlight_radius: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Rect(PhysicalRect),
    Ellipse(PhysicalRect),
    Arrow {
        start: PhysicalPoint,
        end: PhysicalPoint,
    },
    Polyline {
        points: Vec<PhysicalPoint>,
    },
    HighlighterBox(PhysicalRect),
    Blur(PhysicalRect),
    Text {
        bounds: PhysicalRect,
        text: String,
    },
    Number {
        bounds: PhysicalRect,
        value: u32,
    },
    Spotlight(PhysicalRect),
}

impl Shape {
    pub fn bounds(&self) -> PhysicalRect {
        match self {
            Self::Rect(rect)
            | Self::Ellipse(rect)
            | Self::HighlighterBox(rect)
            | Self::Blur(rect)
            | Self::Spotlight(rect)
            | Self::Text { bounds: rect, .. }
            | Self::Number { bounds: rect, .. } => *rect,
            Self::Arrow { start, end } => points_bounds(&[*start, *end]),
            Self::Polyline { points } => points_bounds(points),
        }
    }

    fn translate(&mut self, dx: i32, dy: i32) {
        match self {
            Self::Rect(rect)
            | Self::Ellipse(rect)
            | Self::HighlighterBox(rect)
            | Self::Blur(rect)
            | Self::Spotlight(rect) => {
                rect.origin.x += dx;
                rect.origin.y += dy;
            }
            Self::Text { bounds, .. } | Self::Number { bounds, .. } => {
                bounds.origin.x += dx;
                bounds.origin.y += dy;
            }
            Self::Arrow { start, end } => {
                start.x += dx;
                start.y += dy;
                end.x += dx;
                end.y += dy;
            }
            Self::Polyline { points } => {
                for point in points {
                    point.x += dx;
                    point.y += dy;
                }
            }
        }
    }

    pub fn translated(mut self, dx: i32, dy: i32) -> Self {
        self.translate(dx, dy);
        self
    }

    fn hit_test(&self, point: PhysicalPoint, tolerance: f32) -> bool {
        match self {
            Self::Rect(rect)
            | Self::Ellipse(rect)
            | Self::HighlighterBox(rect)
            | Self::Blur(rect)
            | Self::Spotlight(rect)
            | Self::Text { bounds: rect, .. }
            | Self::Number { bounds: rect, .. } => {
                let expanded = expand(*rect, tolerance);
                expanded.contains(point)
            }
            Self::Arrow { start, end } => distance_to_segment(point, *start, *end) <= tolerance,
            Self::Polyline { points } => points
                .windows(2)
                .any(|pair| distance_to_segment(point, pair[0], pair[1]) <= tolerance),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
    pub id: AnnotationId,
    pub shape: Shape,
    pub style: AnnotationStyle,
}

impl Annotation {
    pub fn bounds(&self) -> PhysicalRect {
        self.shape.bounds()
    }

    pub fn translated(mut self, dx: i32, dy: i32) -> Self {
        self.shape.translate(dx, dy);
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    objects: Vec<Annotation>,
    next_id: u64,
}

#[derive(Clone)]
pub struct AnnotationDocument {
    image_size: PhysicalSize,
    objects: Vec<Annotation>,
    next_id: u64,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    history_serial: u64,
    selected: Option<AnnotationId>,
}

impl AnnotationDocument {
    pub fn new(image_size: PhysicalSize) -> Self {
        Self {
            image_size,
            objects: Vec::new(),
            next_id: 1,
            undo: Vec::new(),
            redo: Vec::new(),
            history_serial: 0,
            selected: None,
        }
    }

    pub fn image_size(&self) -> PhysicalSize {
        self.image_size
    }

    pub fn objects(&self) -> &[Annotation] {
        &self.objects
    }

    pub fn crop_to(&self, origin: PhysicalPoint, size: PhysicalSize) -> Self {
        let mut cropped = Self::new(size);
        for annotation in &self.objects {
            let translated = annotation.clone().translated(-origin.x, -origin.y);
            let id = AnnotationId(cropped.next_id);
            cropped.next_id += 1;
            cropped.objects.push(Annotation {
                id,
                shape: translated.shape,
                style: translated.style,
            });
        }
        cropped
    }

    pub fn selected(&self) -> Option<AnnotationId> {
        self.selected
    }

    pub fn object(&self, id: AnnotationId) -> Option<&Annotation> {
        self.objects.iter().find(|annotation| annotation.id == id)
    }

    pub fn add(&mut self, shape: Shape, style: AnnotationStyle) -> AnnotationId {
        self.record_before_edit();
        let id = AnnotationId(self.next_id);
        self.next_id += 1;
        self.objects.push(Annotation { id, shape, style });
        self.selected = Some(id);
        id
    }

    pub fn remove(&mut self, id: AnnotationId) -> bool {
        let Some(index) = self
            .objects
            .iter()
            .position(|annotation| annotation.id == id)
        else {
            return false;
        };
        let was_number = matches!(self.objects[index].shape, Shape::Number { .. });
        self.record_before_edit();
        self.objects.remove(index);
        if was_number {
            self.renumber_numbers();
        }
        if self.selected == Some(id) {
            self.selected = None;
        }
        true
    }

    pub fn clear(&mut self) -> bool {
        if self.objects.is_empty() {
            return false;
        }
        self.record_before_edit();
        self.objects.clear();
        self.selected = None;
        true
    }

    pub fn select(&mut self, id: Option<AnnotationId>) {
        self.selected = id.filter(|id| self.object(*id).is_some());
    }

    pub fn hit_test(&self, point: PhysicalPoint) -> Option<AnnotationId> {
        self.objects
            .iter()
            .rev()
            .find(|annotation| {
                annotation
                    .shape
                    .hit_test(point, annotation.style.stroke_width.max(6.0))
            })
            .map(|annotation| annotation.id)
    }

    pub fn move_selected(&mut self, dx: i32, dy: i32) -> bool {
        let Some(id) = self.selected else {
            return false;
        };
        let Some(index) = self
            .objects
            .iter()
            .position(|annotation| annotation.id == id)
        else {
            return false;
        };
        self.record_before_edit();
        self.objects[index].shape.translate(dx, dy);
        true
    }

    pub fn update_selected_style(&mut self, style: AnnotationStyle) -> bool {
        let Some(id) = self.selected else {
            return false;
        };
        let Some(index) = self
            .objects
            .iter()
            .position(|annotation| annotation.id == id)
        else {
            return false;
        };
        self.record_before_edit();
        self.objects[index].style = style;
        true
    }

    pub fn update_shape_live(&mut self, id: AnnotationId, shape: Shape, record: bool) -> bool {
        let Some(index) = self
            .objects
            .iter()
            .position(|annotation| annotation.id == id)
        else {
            return false;
        };
        if record {
            self.record_before_edit();
        }
        self.objects[index].shape = shape;
        true
    }

    pub fn commit_shape_gesture(
        &mut self,
        id: AnnotationId,
        original: Shape,
        shape: Shape,
    ) -> bool {
        let Some(index) = self
            .objects
            .iter()
            .position(|annotation| annotation.id == id)
        else {
            return false;
        };
        if original == shape {
            self.objects[index].shape = original;
            return false;
        }
        let mut snapshot = self.snapshot();
        if let Some(object) = snapshot
            .objects
            .iter_mut()
            .find(|annotation| annotation.id == id)
        {
            object.shape = original;
        }
        self.history_serial = self.history_serial.wrapping_add(1);
        self.undo.push(snapshot);
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.objects[index].shape = shape;
        self.selected = Some(id);
        true
    }

    pub fn update_shape(&mut self, id: AnnotationId, shape: Shape, style: AnnotationStyle) -> bool {
        let Some(index) = self
            .objects
            .iter()
            .position(|annotation| annotation.id == id)
        else {
            return false;
        };
        self.record_before_edit();
        self.objects[index].shape = shape;
        self.objects[index].style = style;
        self.selected = Some(id);
        true
    }

    pub fn duplicate(&mut self, id: AnnotationId, dx: i32, dy: i32) -> Option<AnnotationId> {
        let annotation = self.object(id)?.clone();
        Some(self.paste(&annotation, dx, dy))
    }

    pub fn paste(&mut self, annotation: &Annotation, dx: i32, dy: i32) -> AnnotationId {
        self.record_before_edit();
        let id = AnnotationId(self.next_id);
        self.next_id += 1;
        let mut pasted = annotation.clone().translated(dx, dy);
        pasted.id = id;
        if let Shape::Number { value, .. } = &mut pasted.shape {
            *value = self.next_number();
        }
        self.objects.push(pasted);
        self.selected = Some(id);
        id
    }

    fn renumber_numbers(&mut self) {
        let mut numbers: Vec<(u32, usize)> = self
            .objects
            .iter()
            .enumerate()
            .filter_map(|(index, annotation)| match annotation.shape {
                Shape::Number { value, .. } => Some((value, index)),
                _ => None,
            })
            .collect();
        numbers.sort_by_key(|(value, index)| (*value, *index));
        for (position, (_, index)) in numbers.into_iter().enumerate() {
            if let Shape::Number { value, .. } = &mut self.objects[index].shape {
                *value = position as u32 + 1;
            }
        }
    }

    pub fn next_number(&self) -> u32 {
        self.objects
            .iter()
            .filter_map(|annotation| match annotation.shape {
                Shape::Number { value, .. } => Some(value),
                _ => None,
            })
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn history_serial(&self) -> u64 {
        self.history_serial
    }

    pub fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(snapshot);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.restore(snapshot);
        true
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            objects: self.objects.clone(),
            next_id: self.next_id,
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.objects = snapshot.objects;
        self.next_id = snapshot.next_id;
        if let Some(id) = self.selected
            && self.object(id).is_none()
        {
            self.selected = None;
        }
    }

    fn record_before_edit(&mut self) {
        self.history_serial = self.history_serial.wrapping_add(1);
        self.undo.push(self.snapshot());
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
}

fn points_bounds(points: &[PhysicalPoint]) -> PhysicalRect {
    let Some(first) = points.first() else {
        return PhysicalRect::default();
    };
    let mut left = first.x;
    let mut top = first.y;
    let mut right = first.x;
    let mut bottom = first.y;
    for point in points {
        left = left.min(point.x);
        top = top.min(point.y);
        right = right.max(point.x);
        bottom = bottom.max(point.y);
    }
    PhysicalRect::new(
        left,
        top,
        (right - left).max(0) as u32,
        (bottom - top).max(0) as u32,
    )
}

fn expand(rect: PhysicalRect, amount: f32) -> PhysicalRect {
    let amount = amount.max(0.0).round() as i32;
    let x = rect.origin.x - amount;
    let y = rect.origin.y - amount;
    let width = rect.size.width as i32 + amount * 2;
    let height = rect.size.height as i32 + amount * 2;
    PhysicalRect::new(x, y, width.max(0) as u32, height.max(0) as u32)
}

fn distance(a: PhysicalPoint, b: PhysicalPoint) -> f32 {
    let dx = (a.x - b.x) as f32;
    let dy = (a.y - b.y) as f32;
    (dx * dx + dy * dy).sqrt()
}

fn distance_to_segment(point: PhysicalPoint, start: PhysicalPoint, end: PhysicalPoint) -> f32 {
    let px = point.x as f32;
    let py = point.y as f32;
    let ax = start.x as f32;
    let ay = start.y as f32;
    let bx = end.x as f32;
    let by = end.y as f32;
    let dx = bx - ax;
    let dy = by - ay;
    let length_sq = dx * dx + dy * dy;
    if length_sq <= f32::EPSILON {
        return distance(point, start);
    }
    let t = (((px - ax) * dx + (py - ay) * dy) / length_sq).clamp(0.0, 1.0);
    let nearest = PhysicalPoint::new((ax + t * dx).round() as i32, (ay + t * dy).round() as i32);
    distance(point, nearest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> AnnotationDocument {
        AnnotationDocument::new(PhysicalSize::new(800, 600))
    }

    #[test]
    fn add_and_undo_redo_preserve_ids() {
        let mut doc = document();
        let id = doc.add(
            Shape::Rect(PhysicalRect::new(10, 10, 50, 50)),
            AnnotationStyle::default(),
        );
        assert_eq!(doc.objects().len(), 1);
        assert!(doc.undo());
        assert!(doc.objects().is_empty());
        assert!(doc.redo());
        assert_eq!(doc.objects()[0].id, id);
    }

    #[test]
    fn remove_and_undo_restore_object() {
        let mut doc = document();
        let id = doc.add(
            Shape::Rect(PhysicalRect::new(10, 10, 50, 50)),
            AnnotationStyle::default(),
        );
        assert!(doc.remove(id));
        assert!(doc.objects().is_empty());
        assert!(doc.undo());
        assert_eq!(doc.object(id).unwrap().id, id);
    }

    #[test]
    fn hit_test_prefers_topmost_object() {
        let mut doc = document();
        let bottom = doc.add(
            Shape::Rect(PhysicalRect::new(0, 0, 100, 100)),
            AnnotationStyle::default(),
        );
        let top = doc.add(
            Shape::Rect(PhysicalRect::new(50, 50, 100, 100)),
            AnnotationStyle::default(),
        );
        assert_eq!(doc.hit_test(PhysicalPoint::new(75, 75)), Some(top));
        assert_eq!(doc.hit_test(PhysicalPoint::new(25, 25)), Some(bottom));
        assert_eq!(doc.hit_test(PhysicalPoint::new(300, 300)), None);
    }

    #[test]
    fn highlighter_box_hit_test_and_move_follow_the_bounds() {
        let mut doc = document();
        let id = doc.add(
            Shape::HighlighterBox(PhysicalRect::new(10, 10, 40, 20)),
            AnnotationStyle::default(),
        );
        assert_eq!(doc.hit_test(PhysicalPoint::new(30, 20)), Some(id));
        assert_eq!(doc.hit_test(PhysicalPoint::new(80, 80)), None);
        doc.select(Some(id));
        assert!(doc.move_selected(5, 5));
        assert_eq!(
            doc.object(id).unwrap().bounds().origin,
            PhysicalPoint::new(15, 15)
        );
    }

    #[test]
    fn highlighter_color_keeps_rgb_and_only_lowers_alpha() {
        assert_eq!(super::highlighter_color(0xFF0000FF), 0xFF000066);
        assert_eq!(super::highlighter_color(0x000000FF), 0x00000066);
        assert_eq!(super::highlighter_color(0xFFFFFFFF), 0xFFFFFF66);
        assert_eq!(super::highlighter_color(0x0A84FFFF), 0x0A84FF66);
    }

    #[test]
    fn arrow_hit_test_uses_segment_distance() {
        let mut doc = document();
        doc.add(
            Shape::Arrow {
                start: PhysicalPoint::new(10, 10),
                end: PhysicalPoint::new(110, 10),
            },
            AnnotationStyle::default(),
        );
        assert!(doc.hit_test(PhysicalPoint::new(60, 12)).is_some());
        assert!(doc.hit_test(PhysicalPoint::new(60, 60)).is_none());
    }

    #[test]
    fn move_and_style_change_are_undoable() {
        let mut doc = document();
        let id = doc.add(
            Shape::Rect(PhysicalRect::new(10, 10, 50, 50)),
            AnnotationStyle::default(),
        );
        doc.select(Some(id));
        assert!(doc.move_selected(5, 7));
        assert_eq!(
            doc.object(id).unwrap().bounds().origin,
            PhysicalPoint::new(15, 17)
        );
        let style = AnnotationStyle {
            color: 0x00FF00FF,
            stroke_width: 5.0,
            font_size: 20.0,
            ..Default::default()
        };
        assert!(doc.update_selected_style(style));
        assert_eq!(doc.object(id).unwrap().style.color, 0x00FF00FF);
        assert!(doc.undo());
        assert_eq!(doc.object(id).unwrap().style.color, 0xFF3B30FF);
        assert!(doc.undo());
        assert_eq!(
            doc.object(id).unwrap().bounds().origin,
            PhysicalPoint::new(10, 10)
        );
    }

    #[test]
    fn live_shape_updates_commit_one_undo_step_per_gesture() {
        let mut doc = document();
        let id = doc.add(
            Shape::Rect(PhysicalRect::new(10, 10, 50, 50)),
            AnnotationStyle::default(),
        );
        let original = doc.object(id).unwrap().shape.clone();
        for step in 1..=4 {
            let shape = original.clone().translated(step, step * 2);
            assert!(doc.update_shape_live(id, shape, step == 1));
        }
        assert_eq!(
            doc.object(id).unwrap().bounds().origin,
            PhysicalPoint::new(14, 18)
        );
        assert!(doc.undo());
        assert_eq!(
            doc.object(id).unwrap().bounds().origin,
            PhysicalPoint::new(10, 10)
        );
        assert!(doc.can_undo(), "the initial add remains undoable");
    }

    #[test]
    fn committed_gesture_records_the_pre_gesture_state_once() {
        let mut doc = document();
        let id = doc.add(
            Shape::Rect(PhysicalRect::new(10, 10, 50, 50)),
            AnnotationStyle::default(),
        );
        let original = doc.object(id).unwrap().shape.clone();
        let serial = doc.history_serial();
        for step in 1..=4 {
            let shape = original.clone().translated(step, step * 2);
            assert!(doc.update_shape_live(id, shape, false));
        }
        assert_eq!(doc.history_serial(), serial, "live updates must not record");
        let final_shape = original.clone().translated(4, 8);
        assert!(doc.commit_shape_gesture(id, original, final_shape));
        assert!(doc.undo());
        assert_eq!(
            doc.object(id).unwrap().bounds().origin,
            PhysicalPoint::new(10, 10)
        );
        assert!(doc.can_undo(), "the initial add remains undoable");
    }

    #[test]
    fn net_zero_gesture_keeps_history_and_redo() {
        let mut doc = document();
        let id = doc.add(
            Shape::Rect(PhysicalRect::new(10, 10, 50, 50)),
            AnnotationStyle::default(),
        );
        let original = doc.object(id).unwrap().shape.clone();
        doc.add(
            Shape::Rect(PhysicalRect::new(80, 80, 10, 10)),
            AnnotationStyle::default(),
        );
        assert!(doc.undo());
        assert!(doc.can_redo());
        let moved = original.clone().translated(9, 9);
        assert!(doc.update_shape_live(id, moved, false));
        assert!(!doc.commit_shape_gesture(id, original.clone(), original));
        assert_eq!(
            doc.object(id).unwrap().bounds().origin,
            PhysicalPoint::new(10, 10)
        );
        assert!(doc.can_redo(), "a net-zero gesture must not clear redo");
    }

    #[test]
    fn restyling_one_object_leaves_the_others_untouched() {
        let mut doc = document();
        let rect = doc.add(
            Shape::Rect(PhysicalRect::new(10, 10, 50, 50)),
            AnnotationStyle::default(),
        );
        let text = doc.add(
            Shape::Text {
                bounds: PhysicalRect::new(10, 80, 120, 24),
                text: "note".to_owned(),
            },
            AnnotationStyle {
                color: 0x0A84FFFF,
                ..Default::default()
            },
        );
        doc.select(Some(rect));
        let mut style = doc.object(rect).unwrap().style;
        style.color = 0x34C759FF;
        assert!(doc.update_selected_style(style));

        assert_eq!(doc.object(rect).unwrap().style.color, 0x34C759FF);
        assert_eq!(doc.object(text).unwrap().style.color, 0x0A84FFFF);
    }

    #[test]
    fn clear_is_undoable() {
        let mut doc = document();
        doc.add(
            Shape::Ellipse(PhysicalRect::new(0, 0, 20, 20)),
            AnnotationStyle::default(),
        );
        assert!(doc.clear());
        assert!(doc.objects().is_empty());
        assert!(doc.undo());
        assert_eq!(doc.objects().len(), 1);
    }

    #[test]
    fn undo_clears_dangling_selection() {
        let mut doc = document();
        let id = doc.add(
            Shape::Rect(PhysicalRect::new(0, 0, 20, 20)),
            AnnotationStyle::default(),
        );
        doc.select(Some(id));
        assert!(doc.undo());
        assert_eq!(doc.selected(), None);
    }

    #[test]
    fn sequence_numbers_follow_the_existing_badges() {
        let mut doc = document();
        assert_eq!(doc.next_number(), 1);
        doc.add(
            Shape::Number {
                bounds: PhysicalRect::new(0, 0, 30, 30),
                value: 1,
            },
            AnnotationStyle::default(),
        );
        assert_eq!(doc.next_number(), 2);
        doc.add(
            Shape::Number {
                bounds: PhysicalRect::new(40, 0, 30, 30),
                value: 2,
            },
            AnnotationStyle::default(),
        );
        assert_eq!(doc.next_number(), 3);
    }

    #[test]
    fn a_new_document_restarts_sequence_numbers() {
        let mut first = document();
        first.add(
            Shape::Number {
                bounds: PhysicalRect::new(0, 0, 30, 30),
                value: 1,
            },
            AnnotationStyle::default(),
        );
        assert_eq!(first.next_number(), 2);

        let second = document();
        assert_eq!(second.next_number(), 1);
    }

    #[test]
    fn removing_a_number_compacts_the_following_numbers() {
        let mut doc = document();
        let first = doc.add(
            Shape::Number {
                bounds: PhysicalRect::new(0, 0, 30, 30),
                value: 1,
            },
            AnnotationStyle::default(),
        );
        let second = doc.add(
            Shape::Number {
                bounds: PhysicalRect::new(40, 0, 30, 30),
                value: 2,
            },
            AnnotationStyle::default(),
        );
        let third = doc.add(
            Shape::Number {
                bounds: PhysicalRect::new(80, 0, 30, 30),
                value: 3,
            },
            AnnotationStyle::default(),
        );

        assert!(doc.remove(second));
        assert_eq!(number_value(&doc, first), 1);
        assert_eq!(number_value(&doc, third), 2);
        assert!(doc.undo());
        assert_eq!(number_value(&doc, second), 2);
        assert_eq!(number_value(&doc, third), 3);
    }

    #[test]
    fn pasting_an_annotation_offsets_and_assigns_a_new_number() {
        let mut doc = document();
        let first = doc.add(
            Shape::Number {
                bounds: PhysicalRect::new(0, 0, 30, 30),
                value: 1,
            },
            AnnotationStyle::default(),
        );
        assert_eq!(doc.next_number(), 2);
        let annotation = doc.object(first).unwrap().clone();
        let pasted = doc.paste(&annotation, 20, 10);

        assert_eq!(doc.objects().len(), 2);
        assert_eq!(doc.selected(), Some(pasted));
        assert_eq!(
            doc.object(pasted).unwrap().bounds().origin,
            PhysicalPoint::new(20, 10)
        );
        assert_eq!(number_value(&doc, pasted), 2);
        assert_ne!(first, pasted);
    }

    #[test]
    fn pasted_annotation_is_undoable() {
        let mut doc = document();
        let source = doc.add(
            Shape::Rect(PhysicalRect::new(10, 10, 30, 30)),
            AnnotationStyle::default(),
        );
        let annotation = doc.object(source).unwrap().clone();
        doc.paste(&annotation, 10, 10);
        assert_eq!(doc.objects().len(), 2);
        assert!(doc.undo());
        assert_eq!(doc.objects().len(), 1);
    }

    fn number_value(doc: &AnnotationDocument, id: AnnotationId) -> u32 {
        match doc.object(id).unwrap().shape {
            Shape::Number { value, .. } => value,
            _ => panic!("expected a sequence badge"),
        }
    }

    #[test]
    fn crop_to_translates_objects_and_clears_history() {
        let mut doc = document();
        doc.add(
            Shape::Rect(PhysicalRect::new(10, 10, 20, 20)),
            AnnotationStyle::default(),
        );
        let mut cropped = doc.crop_to(PhysicalPoint::new(5, 7), PhysicalSize::new(40, 30));
        assert_eq!(cropped.image_size(), PhysicalSize::new(40, 30));
        assert_eq!(cropped.objects().len(), 1);
        assert_eq!(
            cropped.objects()[0].bounds().origin,
            PhysicalPoint::new(5, 3)
        );
        assert!(cropped.selected().is_none());
        assert!(!cropped.undo());
    }

    #[test]
    fn update_shape_preserves_the_object_identity() {
        let mut doc = document();
        let id = doc.add(
            Shape::Rect(PhysicalRect::new(10, 10, 20, 20)),
            AnnotationStyle::default(),
        );
        assert!(doc.update_shape(
            id,
            Shape::Text {
                bounds: PhysicalRect::new(10, 10, 80, 24),
                text: "edited".to_owned(),
            },
            AnnotationStyle::default(),
        ));
        assert_eq!(doc.objects().len(), 1);
        assert_eq!(doc.selected(), Some(id));
        assert!(matches!(doc.object(id).unwrap().shape, Shape::Text { .. }));
        assert!(doc.undo());
        assert!(matches!(doc.object(id).unwrap().shape, Shape::Rect(_)));
    }
}
