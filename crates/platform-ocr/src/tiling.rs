//! Platform-neutral helpers for tiled OCR detection.

/// Long side of one detection tile. The bundled detector resizes its input to
/// a 960 px long side, so a 1280 px tile keeps small UI text close to native
/// resolution instead of shrinking a whole 4K screenshot four times.
pub(crate) const TILE: u32 = 1280;
/// Overlap between adjacent tiles. Lines crossing a seam appear whole in at
/// least one tile, and `merge_rects` joins the duplicate fragments.
pub(crate) const OVERLAP: u32 = 128;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub confidence: f32,
}

impl Rect {
    pub(crate) fn new(left: f32, top: f32, right: f32, bottom: f32, confidence: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
            confidence,
        }
    }

    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) fn width(&self) -> f32 {
        (self.right - self.left).max(0.0)
    }

    pub(crate) fn height(&self) -> f32 {
        (self.bottom - self.top).max(0.0)
    }
}

/// Splits an image into overlapping tiles. The last tile in each direction is
/// clamped to the image edge and may be narrower than [`TILE`].
pub(crate) fn tile_rects(width: u32, height: u32) -> Vec<(u32, u32, u32, u32)> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let xs = tile_starts(width);
    let ys = tile_starts(height);
    let mut tiles = Vec::with_capacity(xs.len() * ys.len());
    for y in ys {
        for &x in &xs {
            let tile_width = TILE.min(width - x);
            let tile_height = TILE.min(height - y);
            tiles.push((x, y, tile_width, tile_height));
        }
    }
    tiles
}

fn tile_starts(length: u32) -> Vec<u32> {
    let mut starts = Vec::new();
    let mut position = 0;
    loop {
        starts.push(position);
        if position + TILE >= length {
            break;
        }
        position += TILE - OVERLAP;
    }
    starts
}

/// Joins duplicate boxes produced by overlapping tiles and fragments of the
/// same line. Boxes on adjacent rows stay separate because their vertical
/// overlap is below the threshold.
pub(crate) fn merge_rects(rects: &mut Vec<Rect>) {
    loop {
        let mut merged = Vec::with_capacity(rects.len());
        let mut used = vec![false; rects.len()];
        let mut changed = false;
        for index in 0..rects.len() {
            if used[index] {
                continue;
            }
            let mut current = rects[index];
            for (other_index, other) in rects.iter().enumerate().skip(index + 1) {
                if used[other_index] || !should_merge(current, *other) {
                    continue;
                }
                current = union(current, *other);
                used[other_index] = true;
                changed = true;
            }
            merged.push(current);
        }
        *rects = merged;
        if !changed {
            break;
        }
    }
}

fn should_merge(a: Rect, b: Rect) -> bool {
    let min_height = a.height().min(b.height());
    if min_height <= 0.0 {
        return false;
    }
    let vertical_overlap = a.bottom.min(b.bottom) - a.top.max(b.top);
    if vertical_overlap < min_height * 0.55 {
        return false;
    }
    let horizontal_gap = (b.left - a.right).max(a.left - b.right);
    let average_height = (a.height() + b.height()) / 2.0;
    horizontal_gap <= 6.0_f32.max(average_height * 1.4)
}

fn union(a: Rect, b: Rect) -> Rect {
    Rect::new(
        a.left.min(b.left),
        a.top.min(b.top),
        a.right.max(b.right),
        a.bottom.max(b.bottom),
        a.confidence.max(b.confidence),
    )
}

/// Reorders boxes top-to-bottom and left-to-right, grouping boxes whose
/// vertical ranges overlap into the same row.
pub(crate) fn sort_rects(rects: &mut Vec<Rect>) {
    let mut sorted = std::mem::take(rects);
    sorted.sort_by(|a, b| {
        a.top
            .total_cmp(&b.top)
            .then_with(|| a.left.total_cmp(&b.left))
    });
    let mut rows: Vec<Vec<Rect>> = Vec::new();
    for rect in sorted {
        let mut placed = false;
        if let Some(row) = rows.last_mut() {
            let row_top = row.iter().map(|r| r.top).fold(f32::INFINITY, f32::min);
            let row_bottom = row
                .iter()
                .map(|r| r.bottom)
                .fold(f32::NEG_INFINITY, f32::max);
            let overlap = row_bottom.min(rect.bottom) - row_top.max(rect.top);
            let min_height = (row_bottom - row_top).min(rect.height());
            if overlap > min_height * 0.5 {
                row.push(rect);
                placed = true;
            }
        }
        if !placed {
            rows.push(vec![rect]);
        }
    }
    for mut row in rows {
        row.sort_by(|a, b| a.left.total_cmp(&b.left));
        rects.extend(row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_cover_large_images() {
        assert_eq!(tile_rects(800, 600), vec![(0, 0, 800, 600)]);
        assert_eq!(
            tile_rects(1920, 1080),
            vec![(0, 0, 1280, 1080), (1152, 0, 768, 1080)]
        );
        let tiles = tile_rects(3840, 2160);
        assert_eq!(tiles.len(), 8);
        assert_eq!(tiles.last().copied(), Some((3456, 1152, 384, 1008)));
    }

    #[test]
    fn tile_overlap_keeps_seams_covered() {
        let tiles = tile_rects(1920, 1080);
        let (first_x, _, first_w, _) = tiles[0];
        let (second_x, _, _, _) = tiles[1];
        assert!(first_x + first_w - second_x >= OVERLAP);
    }

    #[test]
    fn merge_joins_tile_duplicates_and_wrapped_fragments() {
        let mut rects = vec![
            Rect::new(0.0, 0.0, 150.0, 20.0, 0.9),
            Rect::new(120.0, 1.0, 260.0, 20.0, 0.8),
        ];
        merge_rects(&mut rects);
        assert_eq!(rects, vec![Rect::new(0.0, 0.0, 260.0, 20.0, 0.9)]);
    }

    #[test]
    fn merge_keeps_rows_and_columns_apart() {
        let mut rects = vec![
            Rect::new(0.0, 0.0, 100.0, 20.0, 0.9),
            Rect::new(0.0, 30.0, 100.0, 50.0, 0.9),
            Rect::new(300.0, 0.0, 400.0, 20.0, 0.9),
        ];
        merge_rects(&mut rects);
        assert_eq!(rects.len(), 3);
    }

    #[test]
    fn rows_sort_left_to_right_then_top_to_bottom() {
        let mut rects = vec![
            Rect::new(200.0, 2.0, 300.0, 22.0, 0.9),
            Rect::new(0.0, 0.0, 100.0, 20.0, 0.9),
            Rect::new(0.0, 40.0, 100.0, 60.0, 0.9),
        ];
        sort_rects(&mut rects);
        assert_eq!(rects[0].left, 0.0);
        assert_eq!(rects[0].top, 0.0);
        assert_eq!(rects[1].left, 200.0);
        assert_eq!(rects[2].top, 40.0);
    }
}
