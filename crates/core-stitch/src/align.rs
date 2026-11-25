use core_types::RgbaFrame;

use super::{
    AMBIGUITY_MARGIN, COARSE_ROWS, DUPLICATE_CHANGED_DIVISOR, DUPLICATE_MEAN, Direction,
    EXACT_MATCH_SCORE, EndpointMatch, MAX_CANDIDATES, MIN_ANCHOR_ROWS, REFINE_RADIUS, RowSet,
    Stitcher, insert_candidate, row_energies, row_total,
};

const CONTAINED_ANCHORS: usize = 8;
const CONTAINED_MIN_ANCHORS: usize = 3;
const CONTAINED_SCAN_SAMPLES: usize = 64;
const CONTAINED_CANDIDATES: usize = 4;
const BAND_COUNT: u32 = 6;
const BAND_MEAN_LIMIT: f64 = 8.0;
const BAND_EXCESS_RATIO: f64 = 0.5;
const STATIONARY_MEAN: f64 = 4.0;
const STATIONARY_CHANGED_DIVISOR: usize = 20;

pub(crate) fn bands_consistent(
    previous: &RowSet,
    new: &RowSet,
    previous_offset: u32,
    new_offset: u32,
    overlap: u32,
) -> bool {
    if overlap == 0 {
        return false;
    }
    for band in 0..BAND_COUNT {
        let start = overlap * band / BAND_COUNT;
        let end = overlap * (band + 1) / BAND_COUNT;
        let mut sum = 0u64;
        let mut excess = 0u64;
        let mut signal = 0u64;
        let mut count = 0u64;
        for row in start..end {
            let before = previous.signature(previous_offset + row);
            let after = new.signature(new_offset + row);
            for index in 0..before.len() {
                let delta = before[index].abs_diff(after[index]);
                sum += u64::from(delta);
                excess += u64::from(delta.saturating_sub(2));
                if index > 0 {
                    signal += u64::from(before[index].abs_diff(before[index - 1]))
                        .max(u64::from(after[index].abs_diff(after[index - 1])));
                }
                count += 1;
            }
        }
        if count == 0 {
            continue;
        }
        let mean = sum as f64 / count as f64;
        if mean > BAND_MEAN_LIMIT || excess as f64 > (signal as f64).max(1.0) * BAND_EXCESS_RATIO {
            return false;
        }
    }
    true
}

impl Stitcher {
    pub(crate) fn is_duplicate_frame(&self, signatures: &[u8], height: u32) -> bool {
        if self.previous_frame_rows != height {
            return false;
        }
        let samples = self.samples as usize;
        let total = height as usize * samples;
        if total == 0 || signatures.len() < total || self.previous_frame.len() < total {
            return false;
        }
        let mut sum = 0u64;
        let mut changed = 0usize;
        for (&current, &previous) in signatures[..total]
            .iter()
            .zip(&self.previous_frame[..total])
        {
            let delta = current.abs_diff(previous);
            sum += u64::from(delta);
            if delta > 3 {
                changed += 1;
            }
        }
        let mean = sum as f64 / total as f64;
        mean < DUPLICATE_MEAN && changed <= (total / DUPLICATE_CHANGED_DIVISOR).max(1)
    }

    pub(crate) fn is_stationary_frame(&self, signatures: &[u8], height: u32) -> bool {
        if self.previous_frame_rows != height {
            return false;
        }
        let samples = self.samples as usize;
        let total = height as usize * samples;
        if total == 0 || signatures.len() < total || self.previous_frame.len() < total {
            return false;
        }
        let mut sum = 0u64;
        let mut changed = 0usize;
        for (&current, &previous) in signatures[..total]
            .iter()
            .zip(&self.previous_frame[..total])
        {
            let delta = current.abs_diff(previous);
            sum += u64::from(delta);
            if delta > 3 {
                changed += 1;
            }
        }
        let mean = sum as f64 / total as f64;
        mean < STATIONARY_MEAN && changed <= (total / STATIONARY_CHANGED_DIVISOR).max(1)
    }
}

impl Stitcher {
    pub(crate) fn resolve_endpoint_matches(
        &self,
        tail: Option<(u32, f64)>,
        head: Option<(u32, f64)>,
        content_height: u32,
    ) -> Option<EndpointMatch> {
        let Some((tail_overlap, tail_score)) = tail else {
            return head.map(|(overlap, _)| EndpointMatch {
                direction: Direction::Up,
                overlap,
            });
        };
        let Some((head_overlap, head_score)) = head else {
            return Some(EndpointMatch {
                direction: Direction::Down,
                overlap: tail_overlap,
            });
        };
        let available = self.height.saturating_sub(self.header_rows);
        let tail_top = i64::from(available) - i64::from(tail_overlap);
        let head_top = i64::from(head_overlap) - i64::from(content_height);
        if tail_top == head_top {
            return Some(EndpointMatch {
                direction: Direction::Down,
                overlap: tail_overlap,
            });
        }
        let prefer = |direction: Direction| EndpointMatch {
            direction,
            overlap: if direction == Direction::Down {
                tail_overlap
            } else {
                head_overlap
            },
        };
        let tail_exact = tail_score <= EXACT_MATCH_SCORE;
        let head_exact = head_score <= EXACT_MATCH_SCORE;
        if tail_exact && head_exact {
            let direction = if tail_overlap >= head_overlap {
                Direction::Down
            } else {
                Direction::Up
            };
            return Some(prefer(direction));
        }
        if tail_exact != head_exact {
            return Some(prefer(if tail_exact {
                Direction::Down
            } else {
                Direction::Up
            }));
        }
        if (tail_score - head_score).abs() <= AMBIGUITY_MARGIN {
            return self.last_direction.map(prefer);
        }
        Some(prefer(if tail_score < head_score {
            Direction::Down
        } else {
            Direction::Up
        }))
    }

    pub(crate) fn find_head_overlap(
        &self,
        new: &RowSet,
        inset: u32,
        content_height: u32,
    ) -> Option<(u32, f64)> {
        let available = self.height.saturating_sub(self.header_rows);
        if available == 0 {
            return None;
        }
        let max_overlap = if self.config.max_overlap_rows == 0 {
            available.min(content_height)
        } else {
            self.config
                .max_overlap_rows
                .min(available)
                .min(content_height)
        };
        let min_overlap = self.config.min_overlap_rows.min(max_overlap);
        if max_overlap == 0 || max_overlap < min_overlap {
            return None;
        }

        let samples = self.samples as usize;
        let head_start = self.header_rows as usize * samples;
        let head_end = head_start + max_overlap as usize * samples;
        let head_signatures = &self.signatures[head_start..head_end];
        let head_energies = row_energies(head_signatures, max_overlap, self.samples);
        let previous = RowSet {
            signatures: head_signatures,
            energies: &head_energies,
            samples,
            stable: None,
        };
        let anchors: Vec<u32> = (0..max_overlap)
            .filter(|row| u32::from(previous.energy(*row)) >= self.config.anchor_threshold)
            .collect();

        let mut candidates: Vec<(f64, u32)> = Vec::with_capacity(MAX_CANDIDATES);
        for overlap in (min_overlap..=max_overlap).rev() {
            let score = head_candidate_score(
                samples,
                &previous,
                new,
                &anchors,
                inset + content_height - overlap,
                overlap,
                COARSE_ROWS,
            );
            insert_candidate(&mut candidates, score, overlap);
        }
        if candidates.is_empty() {
            return None;
        }

        let mut scored: Vec<(f64, u32)> = Vec::with_capacity(candidates.len());
        for (_, overlap) in &candidates {
            let offset = inset + content_height - *overlap;
            let score = head_full_score(samples, &previous, new, offset, *overlap);
            scored.push((score, *overlap));
        }
        scored.sort_by(|left, right| left.0.total_cmp(&right.0));
        let (best_score, best_overlap) = scored[0];
        let runner = scored
            .iter()
            .skip(1)
            .find(|(_, overlap)| overlap.abs_diff(best_overlap) > REFINE_RADIUS)
            .map_or(f64::INFINITY, |entry| entry.0);

        let low = best_overlap.saturating_sub(REFINE_RADIUS).max(min_overlap);
        let high = (best_overlap + REFINE_RADIUS).min(max_overlap);
        let mut refined = (f64::INFINITY, best_overlap);
        for overlap in (low..=high).rev() {
            let offset = inset + content_height - overlap;
            let score = head_full_score(samples, &previous, new, offset, overlap);
            if score < refined.0 {
                refined = (score, overlap);
            }
        }
        if refined.0 > f64::from(self.config.tolerance) {
            return None;
        }
        if !bands_consistent(
            &previous,
            new,
            0,
            inset + content_height - refined.1,
            refined.1,
        ) {
            return None;
        }
        if runner.is_finite()
            && runner - best_score < AMBIGUITY_MARGIN
            && best_score > EXACT_MATCH_SCORE
        {
            return None;
        }
        Some((refined.1, refined.0))
    }

    pub(crate) fn find_contained(
        &self,
        new: &RowSet,
        inset: u32,
        content_height: u32,
    ) -> Option<u32> {
        let available = self.height.saturating_sub(self.header_rows);
        if available < content_height || content_height == 0 {
            return None;
        }
        let range = available - content_height;
        let anchors = self.contained_anchors(new, inset, content_height);
        if anchors.len() < CONTAINED_MIN_ANCHORS {
            return None;
        }

        let samples = self.samples as usize;
        let stride = samples.div_ceil(CONTAINED_SCAN_SAMPLES).max(1);
        let limit = u64::from(self.config.tolerance) * 2;
        let header = self.header_rows as usize;
        let mut votes = vec![0u8; range as usize + 1];
        for &(content_row, frame_row) in &anchors {
            let anchor = new.signature(frame_row);
            for buffer_index in 0..available as usize {
                let q = buffer_index as i64 - i64::from(content_row);
                if q < 0 || q > i64::from(range) {
                    continue;
                }
                let start = (header + buffer_index) * samples;
                if sampled_row_distance(&self.signatures[start..start + samples], anchor, stride)
                    <= limit
                {
                    let slot = &mut votes[q as usize];
                    *slot = slot.saturating_add(1);
                }
            }
        }

        let threshold = (anchors.len() as u8).div_ceil(2).max(2);
        let mut candidates: Vec<(u8, u32)> = votes
            .iter()
            .enumerate()
            .filter_map(|(q, &count)| (count >= threshold).then_some((count, q as u32)))
            .collect();
        candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        candidates.truncate(CONTAINED_CANDIDATES);
        if candidates.is_empty() {
            return None;
        }

        let mut scored: Vec<(f64, u32)> = candidates
            .iter()
            .map(|&(_, q)| (self.contained_full_score(new, inset, content_height, q), q))
            .collect();
        scored.sort_by(|left, right| left.0.total_cmp(&right.0));
        let (best_score, best_q) = scored[0];
        let runner = scored
            .iter()
            .skip(1)
            .find(|(_, q)| q.abs_diff(best_q) > REFINE_RADIUS)
            .map_or(f64::INFINITY, |entry| entry.0);

        let low = best_q.saturating_sub(REFINE_RADIUS);
        let high = (best_q + REFINE_RADIUS).min(range);
        let mut refined = (f64::INFINITY, best_q);
        for q in low..=high {
            let score = self.contained_full_score(new, inset, content_height, q);
            if score < refined.0 {
                refined = (score, q);
            }
        }
        if refined.0 > f64::from(self.config.tolerance) {
            return None;
        }
        if self.contained_anchor_score(new, &anchors, refined.1) > f64::from(self.config.tolerance)
        {
            return None;
        }
        if runner.is_finite()
            && runner - best_score < AMBIGUITY_MARGIN
            && best_score > EXACT_MATCH_SCORE
        {
            return None;
        }
        Some(refined.1)
    }

    pub(crate) fn insert_frame_rows(
        &mut self,
        position: u32,
        frame: &RgbaFrame,
        first_row: u32,
        rows: u32,
    ) {
        let row_bytes = self.width as usize * 4;
        let offset = position as usize * row_bytes;
        let added = rows as usize * row_bytes;
        let old_len = self.pixels.len();
        self.pixels.resize(old_len + added, 0);
        self.pixels.copy_within(offset..old_len, offset + added);
        let stride = frame.stride();
        if stride == row_bytes {
            let start = first_row as usize * row_bytes;
            self.pixels[offset..offset + added]
                .copy_from_slice(&frame.pixels()[start..start + added]);
            return;
        }
        for row in 0..rows as usize {
            let source = (first_row as usize + row) * stride;
            let target = offset + row * row_bytes;
            self.pixels[target..target + row_bytes]
                .copy_from_slice(&frame.pixels()[source..source + row_bytes]);
        }
    }

    pub(crate) fn insert_signature_rows(&mut self, position: u32, rows: &[u8]) {
        let samples = self.samples as usize;
        let offset = position as usize * samples;
        let old_len = self.signatures.len();
        self.signatures.resize(old_len + rows.len(), 0);
        self.signatures
            .copy_within(offset..old_len, offset + rows.len());
        self.signatures[offset..offset + rows.len()].copy_from_slice(rows);
    }

    fn contained_anchors(&self, new: &RowSet, inset: u32, content_height: u32) -> Vec<(u32, u32)> {
        let band_height = content_height.div_ceil(CONTAINED_ANCHORS as u32).max(1);
        let mut anchors = Vec::with_capacity(CONTAINED_ANCHORS);
        for band in 0..CONTAINED_ANCHORS as u32 {
            let start = band * band_height;
            if start >= content_height {
                break;
            }
            let end = (start + band_height).min(content_height);
            let mut best: Option<(u16, u32)> = None;
            for row in start..end {
                let frame_row = inset + row;
                if new.is_stable(frame_row) {
                    continue;
                }
                let energy = new.energy(frame_row);
                if u32::from(energy) >= self.config.anchor_threshold
                    && best.is_none_or(|(best_energy, _)| energy > best_energy)
                {
                    best = Some((energy, row));
                }
            }
            if let Some((_, row)) = best {
                anchors.push((row, inset + row));
            }
        }
        anchors
    }

    pub(crate) fn contained_full_score(
        &self,
        new: &RowSet,
        inset: u32,
        content_height: u32,
        q: u32,
    ) -> f64 {
        let samples = self.samples as usize;
        let header = self.header_rows as usize;
        let mut total = 0u64;
        let mut compared = 0u64;
        for row in 0..content_height {
            let frame_row = inset + row;
            if new.is_stable(frame_row) {
                continue;
            }
            let start = (header + q as usize + row as usize) * samples;
            total += row_total(
                &self.signatures[start..start + samples],
                new.signature(frame_row),
            );
            compared += 1;
        }
        total as f64 / (compared.max(1) as f64 * samples as f64)
    }

    fn contained_anchor_score(&self, new: &RowSet, anchors: &[(u32, u32)], q: u32) -> f64 {
        let samples = self.samples as usize;
        let header = self.header_rows as usize;
        let mut total = 0u64;
        let mut compared = 0u64;
        for &(content_row, frame_row) in anchors {
            if new.is_stable(frame_row) {
                continue;
            }
            let start = (header + q as usize + content_row as usize) * samples;
            total += row_total(
                &self.signatures[start..start + samples],
                new.signature(frame_row),
            );
            compared += 1;
        }
        total as f64 / (compared.max(1) as f64 * samples as f64)
    }
}

fn head_candidate_score(
    samples: usize,
    previous: &RowSet,
    new: &RowSet,
    anchors: &[u32],
    new_offset: u32,
    overlap: u32,
    max_rows: usize,
) -> f64 {
    let usable = anchors.partition_point(|row| *row < overlap);
    let mut total = 0u64;
    let mut compared = 0u64;
    if usable as u32 >= MIN_ANCHOR_ROWS {
        let stride = usable.div_ceil(max_rows.max(1)).max(1);
        for index in (0..usable).step_by(stride) {
            let row = anchors[index];
            let new_row = new_offset + row;
            if new.is_stable(new_row) {
                continue;
            }
            total += row_total(previous.signature(row), new.signature(new_row));
            compared += 1;
        }
    } else {
        let stride = overlap.div_ceil(max_rows as u32).max(1);
        for row in (0..overlap).step_by(stride as usize) {
            let new_row = new_offset + row;
            if new.is_stable(new_row) {
                continue;
            }
            total += row_total(previous.signature(row), new.signature(new_row));
            compared += 1;
        }
    }
    total as f64 / (compared.max(1) as f64 * samples as f64)
}

fn head_full_score(
    samples: usize,
    previous: &RowSet,
    new: &RowSet,
    new_offset: u32,
    overlap: u32,
) -> f64 {
    let mut total = 0u64;
    let mut compared = 0u64;
    for row in 0..overlap {
        let new_row = new_offset + row;
        if new.is_stable(new_row) {
            continue;
        }
        total += row_total(previous.signature(row), new.signature(new_row));
        compared += 1;
    }
    total as f64 / (compared.max(1) as f64 * samples as f64)
}

fn sampled_row_distance(left: &[u8], right: &[u8], stride: usize) -> u64 {
    let mut total = 0u64;
    let mut count = 0u64;
    let mut index = 0;
    while index < left.len() {
        total += u64::from(left[index].abs_diff(right[index]));
        count += 1;
        index += stride;
    }
    total / count.max(1)
}
