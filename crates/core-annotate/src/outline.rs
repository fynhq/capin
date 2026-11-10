//! Filled polygon outlines for line annotations.
//!
//! Both the GPU preview and the CPU exporter fill these polygons instead of
//! stroking the centre line. Stroke tessellation gave the preview butt caps
//! and miter spikes, while the exporter used round caps, so the two surfaces
//! could never match. The outline is cheap to build and gives both paths the
//! same vector geometry.

use core_types::PhysicalPoint;

use crate::{AnnotationStyle, Shape};

const HEAD_LENGTH_FACTOR: f32 = 3.5;
const MIN_HEAD_LENGTH: f32 = 10.0;
const HEAD_BACK: f32 = 0.85;
const HEAD_HALF_WIDTH: f32 = 0.5;
const HEAD_NOTCH: f32 = 0.55;
const CAP_SEGMENTS: usize = 24;

/// A vector vertex with sub-pixel precision. Keeping the outline in floats
/// avoids faceted round caps on thin strokes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutlinePoint {
    pub x: f32,
    pub y: f32,
}

impl OutlinePoint {
    fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Returns the closed polygons that make up a line annotation, or `None` for
/// shapes that are not arrows/polylines.
pub fn stroke_outline(shape: &Shape, style: &AnnotationStyle) -> Option<Vec<Vec<OutlinePoint>>> {
    let points = match shape {
        Shape::Arrow { start, end } => vec![*start, *end],
        Shape::Polyline { points } => points.clone(),
        _ => return None,
    };
    if points.len() < 2 {
        return None;
    }
    Some(arrow_outline(
        &points,
        style,
        style.arrow_start,
        style.arrow_end,
    ))
}

fn arrow_outline(
    points: &[PhysicalPoint],
    style: &AnnotationStyle,
    arrow_start: bool,
    arrow_end: bool,
) -> Vec<Vec<OutlinePoint>> {
    let radius = (style.stroke_width.max(1.0) / 2.0).max(0.5);
    let head_length = (style.stroke_width * HEAD_LENGTH_FACTOR).max(MIN_HEAD_LENGTH);
    let mut line: Vec<OutlinePoint> = points
        .iter()
        .map(|point| OutlinePoint::new(point.x as f32, point.y as f32))
        .collect();
    let mut polygons = Vec::new();

    if arrow_end {
        let tip = line[line.len() - 1];
        let previous = line[line.len() - 2];
        if let Some(head) = arrow_head(tip, previous, head_length) {
            polygons.push(head);
            let direction = normalize(tip, previous);
            let last = line.len() - 1;
            line[last] = shift(tip, direction, head_length * HEAD_NOTCH);
        }
    }
    if arrow_start {
        let tip = line[0];
        let next = line[1];
        if let Some(head) = arrow_head(tip, next, head_length) {
            polygons.push(head);
            let direction = normalize(tip, next);
            line[0] = shift(tip, direction, head_length * HEAD_NOTCH);
        }
    }

    for pair in line.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if distance(start, end) < 0.01 {
            push_circle(&mut polygons, start, radius);
        } else {
            push_segment(&mut polygons, start, end, radius);
        }
    }
    polygons
}

/// Builds a solid dart pointing at `tip`; `previous` is the neighbouring
/// point on the shaft and therefore gives the inward direction.
fn arrow_head(
    tip: OutlinePoint,
    previous: OutlinePoint,
    head_length: f32,
) -> Option<Vec<OutlinePoint>> {
    let inward = normalize(tip, previous);
    if inward == (0.0, 0.0) {
        return None;
    }
    let normal = (-inward.1, inward.0);
    let back = shift(tip, inward, head_length * HEAD_BACK);
    let notch = shift(tip, inward, head_length * HEAD_NOTCH);
    let half = head_length * HEAD_HALF_WIDTH;
    Some(vec![
        tip,
        OutlinePoint::new(back.x + normal.0 * half, back.y + normal.1 * half),
        notch,
        OutlinePoint::new(back.x - normal.0 * half, back.y - normal.1 * half),
    ])
}

fn push_segment(
    polygons: &mut Vec<Vec<OutlinePoint>>,
    start: OutlinePoint,
    end: OutlinePoint,
    radius: f32,
) {
    let direction = normalize(start, end);
    let normal = (-direction.1, direction.0);
    polygons.push(vec![
        OutlinePoint::new(start.x + normal.0 * radius, start.y + normal.1 * radius),
        OutlinePoint::new(end.x + normal.0 * radius, end.y + normal.1 * radius),
        OutlinePoint::new(end.x - normal.0 * radius, end.y - normal.1 * radius),
        OutlinePoint::new(start.x - normal.0 * radius, start.y - normal.1 * radius),
    ]);
    push_circle(polygons, start, radius);
    push_circle(polygons, end, radius);
}

fn push_circle(polygons: &mut Vec<Vec<OutlinePoint>>, center: OutlinePoint, radius: f32) {
    let mut polygon = Vec::with_capacity(CAP_SEGMENTS);
    for step in 0..CAP_SEGMENTS {
        let angle = std::f32::consts::TAU * step as f32 / CAP_SEGMENTS as f32;
        polygon.push(OutlinePoint::new(
            center.x + angle.cos() * radius,
            center.y + angle.sin() * radius,
        ));
    }
    polygons.push(polygon);
}

fn normalize(from: OutlinePoint, to: OutlinePoint) -> (f32, f32) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let length = (dx * dx + dy * dy).sqrt();
    if length <= f32::EPSILON {
        (0.0, 0.0)
    } else {
        (dx / length, dy / length)
    }
}

fn shift(point: OutlinePoint, direction: (f32, f32), amount: f32) -> OutlinePoint {
    OutlinePoint::new(
        point.x + direction.0 * amount,
        point.y + direction.1 * amount,
    )
}

fn distance(a: OutlinePoint, b: OutlinePoint) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> AnnotationStyle {
        AnnotationStyle {
            stroke_width: 4.0,
            ..Default::default()
        }
    }

    #[test]
    fn straight_arrow_reaches_the_tip() {
        let shape = Shape::Arrow {
            start: PhysicalPoint::new(10, 50),
            end: PhysicalPoint::new(190, 50),
        };
        let polygons = stroke_outline(&shape, &style()).unwrap();
        let tip = polygons
            .iter()
            .flatten()
            .map(|point| point.x)
            .fold(f32::MIN, f32::max);
        assert!(tip >= 190.0, "head should reach the arrow tip");
    }

    #[test]
    fn line_without_heads_still_has_round_caps() {
        let shape = Shape::Arrow {
            start: PhysicalPoint::new(20, 20),
            end: PhysicalPoint::new(120, 20),
        };
        let mut style = style();
        style.arrow_end = false;
        let polygons = stroke_outline(&shape, &style).unwrap();
        let min_x = polygons
            .iter()
            .flatten()
            .map(|point| point.x)
            .fold(f32::MAX, f32::min);
        assert!(min_x <= 18.1, "round cap should extend past the endpoint");
    }

    #[test]
    fn polyline_produces_one_outline_per_segment() {
        let shape = Shape::Polyline {
            points: vec![
                PhysicalPoint::new(10, 80),
                PhysicalPoint::new(40, 30),
                PhysicalPoint::new(70, 70),
            ],
        };
        let polygons = stroke_outline(&shape, &style()).unwrap();
        assert!(polygons.len() >= 4);
    }

    #[test]
    fn other_shapes_have_no_outline() {
        let shape = Shape::Rect(core_types::PhysicalRect::new(0, 0, 10, 10));
        assert!(stroke_outline(&shape, &style()).is_none());
    }
}
