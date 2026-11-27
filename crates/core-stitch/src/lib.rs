use core_types::{FrameError, RgbaFrame};
use thiserror::Error;

mod align;
mod preview;

pub use preview::PreviewAnchor;

const BYTES_PER_PIXEL: usize = 4;
const COARSE_ROWS: usize = 16;
const REFINE_RADIUS: u32 = 4;
const AMBIGUITY_MARGIN: f64 = 0.5;
const EXACT_MATCH_SCORE: f64 = 2.0;
const MAX_CANDIDATES: usize = 5;
const MIN_ANCHOR_ROWS: u32 = 6;
const DUPLICATE_MEAN: f64 = 0.6;
const DUPLICATE_CHANGED_DIVISOR: usize = 10_000;
const MIN_ROW_SPREAD: u8 = 8;

#[derive(Clone, Debug)]
pub struct StitchConfig {
    pub min_overlap_rows: u32,
    pub max_overlap_rows: u32,
    pub sample_columns: u32,
    pub edge_margin: u32,
    pub tolerance: u32,
    pub max_top_inset_rows: u32,
    pub max_bottom_inset_rows: u32,
    pub min_static_rows: u32,
    pub static_tolerance: u32,
    pub anchor_threshold: u32,
    pub max_height: u32,
    pub max_bytes: usize,
}

impl Default for StitchConfig {
    fn default() -> Self {
        Self {
            min_overlap_rows: 24,
            max_overlap_rows: 0,
            sample_columns: 0,
            edge_margin: 16,
            tolerance: 20,
            max_top_inset_rows: 256,
            max_bottom_inset_rows: 256,
            min_static_rows: 8,
            static_tolerance: 4,
            anchor_threshold: 3,
            max_height: 16_000,
            max_bytes: 256 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Error)]
pub enum StitchError {
    #[error("frame width {frame} does not match stitcher width {expected}")]
    WidthMismatch { expected: u32, frame: u32 },
    #[error("cannot stitch an empty frame")]
    EmptyFrame,
    #[error("stitched image reached the maximum height of {0} rows")]
    HeightLimit(u32),
    #[error("stitched image would exceed the {0}-byte memory budget")]
    MemoryLimit(usize),
    #[error("could not allocate {0} bytes for the stitched image")]
    AllocationFailed(usize),
    #[error("stitched image dimensions overflow address space")]
    SizeOverflow,
    #[error("no vertical overlap was found between frames")]
    NoOverlap,
    #[error(transparent)]
    Frame(#[from] FrameError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppendReport {
    pub overlap_rows: u32,
    pub trimmed_top_rows: u32,
    pub trimmed_bottom_rows: u32,
    pub appended_rows: u32,
    pub prepended_rows: u32,
    pub total_height: u32,
    pub frames: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Down,
    Up,
}

#[derive(Clone, Copy, Debug)]
struct EndpointMatch {
    direction: Direction,
    overlap: u32,
}

pub struct Stitcher {
    width: u32,
    config: StitchConfig,
    samples: u32,
    pixels: Vec<u8>,
    signatures: Vec<u8>,
    height: u32,
    frames: u32,
    header_rows: u32,
    last_direction: Option<Direction>,
    last_placement: Option<u32>,
    previous_top: Vec<u8>,
    previous_top_rows: u32,
    previous_bottom: Vec<u8>,
    previous_bottom_rows: u32,
    previous_frame: Vec<u8>,
    previous_frame_rows: u32,
    footer_rows: u32,
    footer_pixels: Vec<u8>,
}

impl Stitcher {
    pub fn new(width: u32, config: StitchConfig) -> Result<Self, StitchError> {
        if width == 0 {
            return Err(StitchError::EmptyFrame);
        }
        let samples = if config.sample_columns == 0 {
            (width / 4).clamp(48, 384).min(width)
        } else {
            config.sample_columns.min(width).max(1)
        };
        Ok(Self {
            width,
            config,
            samples,
            pixels: Vec::new(),
            signatures: Vec::new(),
            height: 0,
            frames: 0,
            header_rows: 0,
            last_direction: None,
            last_placement: None,
            previous_top: Vec::new(),
            previous_top_rows: 0,
            previous_bottom: Vec::new(),
            previous_bottom_rows: 0,
            previous_frame: Vec::new(),
            previous_frame_rows: 0,
            footer_rows: 0,
            footer_pixels: Vec::new(),
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn frames(&self) -> u32 {
        self.frames
    }

    pub fn thumbnail(&self, max_width: u32, max_height: u32) -> Option<RgbaFrame> {
        if self.height == 0 || self.width == 0 || max_width == 0 || max_height == 0 {
            return None;
        }
        let scale = (f64::from(max_width) / f64::from(self.width))
            .min(f64::from(max_height) / f64::from(self.height))
            .min(1.0);
        let width = (f64::from(self.width) * scale).round().max(1.0) as u32;
        let height = (f64::from(self.height) * scale).round().max(1.0) as u32;
        let mut pixels = vec![0u8; width as usize * height as usize * BYTES_PER_PIXEL];
        let src_row = self.width as usize * BYTES_PER_PIXEL;
        for y in 0..height {
            let source_y = (u64::from(y) * u64::from(self.height) / u64::from(height)) as usize;
            for x in 0..width {
                let source_x = (u64::from(x) * u64::from(self.width) / u64::from(width)) as usize;
                let source = source_y * src_row + source_x * BYTES_PER_PIXEL;
                let target = (y as usize * width as usize + x as usize) * BYTES_PER_PIXEL;
                pixels[target..target + BYTES_PER_PIXEL]
                    .copy_from_slice(&self.pixels[source..source + BYTES_PER_PIXEL]);
            }
        }
        RgbaFrame::from_pixels(width, height, pixels).ok()
    }

    pub fn push(&mut self, frame: &RgbaFrame) -> Result<AppendReport, StitchError> {
        if frame.width() != self.width {
            return Err(StitchError::WidthMismatch {
                expected: self.width,
                frame: frame.width(),
            });
        }
        if frame.height() == 0 {
            return Err(StitchError::EmptyFrame);
        }

        let signatures = row_signatures(
            frame.pixels(),
            frame.stride(),
            frame.width(),
            frame.height(),
            self.samples,
            self.edge_margin(),
        );
        let stable = self.stable_rows(&signatures, frame.height());
        let energies = row_energies(&signatures, frame.height(), self.samples);
        let spreads = row_spreads(&signatures, frame.height(), self.samples);
        let new = RowSet {
            signatures: &signatures,
            energies: &energies,
            spreads: Some(&spreads),
            samples: self.samples as usize,
            stable: Some(&stable),
        };

        if self.frames == 0 {
            if frame.height() > self.height_limit() {
                return Err(self.limit_error());
            }
            self.reserve_rows(frame.height() as usize)?;
            self.append_frame_rows(frame, 0, frame.height());
            self.height = frame.height();
            self.frames = 1;
            self.remember_bands(&new, frame.height());
            self.signatures = signatures;
            self.last_placement = Some(0);
            return Ok(self.report(0, 0, 0, self.height, 0));
        }

        if self.is_duplicate_frame(&signatures, frame.height()) {
            self.frames += 1;
            self.remember_bands(&new, frame.height());
            return Ok(self.report(frame.height(), 0, 0, 0, 0));
        }

        let footer_rows = self.detect_static_bottom(&new, frame.height());
        if footer_rows > 0 {
            if self.footer_rows == 0 && self.buffer_tail_matches(&new, frame.height(), footer_rows)
            {
                let row_bytes = self.width as usize * BYTES_PER_PIXEL;
                let remove = footer_rows as usize * row_bytes;
                if remove <= self.pixels.len() {
                    self.pixels.truncate(self.pixels.len() - remove);
                    self.height = self.height.saturating_sub(footer_rows);
                    let remove_signatures = footer_rows as usize * self.samples as usize;
                    if remove_signatures <= self.signatures.len() {
                        self.signatures
                            .truncate(self.signatures.len() - remove_signatures);
                    }
                }
            }
            self.store_footer(frame, frame.height(), footer_rows);
        } else if self.footer_rows > 0 {
            self.footer_rows = 0;
            self.footer_pixels.clear();
        }

        let top_inset = self.detect_static_top(&new, frame.height());
        if top_inset > 0 && self.header_rows == 0 {
            self.header_rows = top_inset;
            self.last_placement = None;
        }
        let content_height = frame
            .height()
            .saturating_sub(footer_rows)
            .saturating_sub(top_inset);
        if content_height == 0 {
            self.frames += 1;
            self.remember_bands(&new, frame.height());
            return Ok(self.report(0, top_inset, footer_rows, 0, 0));
        }
        let available = self.height.saturating_sub(self.header_rows);
        let tail = self.find_tail_overlap(&new, top_inset, content_height);
        let head = self.find_head_overlap(&new, top_inset, content_height);
        let Some(endpoint) = self.resolve_endpoint_matches(tail, head, content_height) else {
            let still_placed = self.last_placement.is_some_and(|q| {
                q.saturating_add(content_height) <= available
                    && self.contained_full_score(&new, top_inset, content_height, q)
                        <= f64::from(self.config.tolerance)
            });
            if still_placed {
                self.frames += 1;
                self.remember_bands(&new, frame.height());
                return Ok(self.report(content_height, top_inset, footer_rows, 0, 0));
            }
            if let Some(q) = self.find_contained(&new, top_inset, content_height) {
                self.last_placement = Some(q);
                self.frames += 1;
                self.remember_bands(&new, frame.height());
                return Ok(self.report(content_height, top_inset, footer_rows, 0, 0));
            }
            if self.is_stationary_frame(&signatures, frame.height()) {
                self.frames += 1;
                self.remember_bands(&new, frame.height());
                return Ok(self.report(content_height, top_inset, footer_rows, 0, 0));
            }
            self.last_placement = None;
            self.remember_bands(&new, frame.height());
            return Err(StitchError::NoOverlap);
        };
        self.last_direction = Some(endpoint.direction);
        let overlap = endpoint.overlap;
        let placement = match endpoint.direction {
            Direction::Down => available.saturating_sub(overlap),
            Direction::Up => 0,
        };
        let added = content_height.saturating_sub(overlap);
        if added == 0 {
            self.last_placement = Some(placement);
            self.frames += 1;
            self.remember_bands(&new, frame.height());
            return Ok(self.report(overlap, top_inset, footer_rows, 0, 0));
        }
        let room = self.height_limit().saturating_sub(self.height);
        if room < added {
            self.last_placement = None;
            self.remember_bands(&new, frame.height());
            return Err(self.limit_error());
        }
        self.reserve_rows(added as usize)?;
        let (appended, prepended) = match endpoint.direction {
            Direction::Down => {
                self.append_frame_rows(frame, top_inset + overlap, added);
                let signature_start = (top_inset + overlap) as usize * self.samples as usize;
                let signature_end = signature_start + added as usize * self.samples as usize;
                self.signatures
                    .extend_from_slice(&signatures[signature_start..signature_end]);
                (added, 0)
            }
            Direction::Up => {
                let signature_start = top_inset as usize * self.samples as usize;
                let signature_end = signature_start + added as usize * self.samples as usize;
                self.insert_frame_rows(self.header_rows, frame, top_inset, added);
                self.insert_signature_rows(
                    self.header_rows,
                    &signatures[signature_start..signature_end],
                );
                (0, added)
            }
        };
        self.height += added;
        self.frames += 1;
        self.last_placement = Some(placement);
        self.remember_bands(&new, frame.height());
        Ok(self.report(overlap, top_inset, footer_rows, appended, prepended))
    }

    pub fn push_owned(&mut self, frame: RgbaFrame) -> Result<AppendReport, StitchError> {
        if self.frames != 0 {
            return self.push(&frame);
        }
        if frame.width() != self.width {
            return Err(StitchError::WidthMismatch {
                expected: self.width,
                frame: frame.width(),
            });
        }
        if frame.height() == 0 {
            return Err(StitchError::EmptyFrame);
        }
        if frame.height() > self.height_limit() {
            return Err(self.limit_error());
        }
        let height = frame.height();
        let signatures = row_signatures(
            frame.pixels(),
            frame.stride(),
            frame.width(),
            height,
            self.samples,
            self.edge_margin(),
        );
        let stable = self.stable_rows(&signatures, height);
        let energies = row_energies(&signatures, height, self.samples);
        let spreads = row_spreads(&signatures, height, self.samples);
        let rows = RowSet {
            signatures: &signatures,
            energies: &energies,
            spreads: Some(&spreads),
            samples: self.samples as usize,
            stable: Some(&stable),
        };
        self.height = height;
        self.frames = 1;
        self.last_placement = Some(0);
        self.remember_bands(&rows, height);
        self.pixels = frame.into_packed_pixels();
        self.signatures = signatures;
        Ok(self.report(0, 0, 0, height, 0))
    }

    pub fn replace_with_frame(&mut self, frame: &RgbaFrame) -> Result<AppendReport, StitchError> {
        if frame.width() != self.width {
            return Err(StitchError::WidthMismatch {
                expected: self.width,
                frame: frame.width(),
            });
        }
        if frame.height() == 0 {
            return Err(StitchError::EmptyFrame);
        }
        self.pixels.clear();
        self.signatures.clear();
        self.height = 0;
        self.frames = 0;
        self.header_rows = 0;
        self.last_direction = None;
        self.last_placement = None;
        self.previous_top.clear();
        self.previous_top_rows = 0;
        self.previous_bottom.clear();
        self.previous_bottom_rows = 0;
        self.previous_frame.clear();
        self.previous_frame_rows = 0;
        self.footer_rows = 0;
        self.footer_pixels.clear();
        self.push(frame)
    }

    fn reserve_rows(&mut self, rows: usize) -> Result<(), StitchError> {
        let row_bytes = (self.width as usize).saturating_mul(4).max(1);
        let needed = self
            .pixels
            .len()
            .checked_add(rows.saturating_mul(row_bytes))
            .ok_or(StitchError::SizeOverflow)?;
        if needed <= self.pixels.capacity() {
            return Ok(());
        }
        let limit = self.height_limit() as usize * row_bytes;
        let grown = self.pixels.len().saturating_mul(2).max(needed);
        let target = grown.min(limit).max(needed);
        self.pixels
            .try_reserve_exact(target - self.pixels.len())
            .map_err(|_| StitchError::AllocationFailed(target))
    }

    pub fn finish(&mut self) -> Result<RgbaFrame, StitchError> {
        if self.frames == 0 || self.height == 0 {
            return Err(StitchError::EmptyFrame);
        }
        if self.footer_rows > 0 {
            let room = self.height_limit().saturating_sub(self.height);
            if room >= self.footer_rows {
                self.reserve_rows(self.footer_rows as usize)?;
                self.pixels.extend_from_slice(&self.footer_pixels);
                self.height += self.footer_rows;
            }
        }
        self.footer_rows = 0;
        self.footer_pixels = Vec::new();
        self.previous_top = Vec::new();
        self.previous_top_rows = 0;
        self.previous_bottom = Vec::new();
        self.previous_bottom_rows = 0;
        self.previous_frame = Vec::new();
        self.previous_frame_rows = 0;
        self.header_rows = 0;
        self.last_direction = None;
        self.last_placement = None;
        let width = self.width;
        let height = self.height;
        let pixels = std::mem::take(&mut self.pixels);
        self.signatures = Vec::new();
        self.height = 0;
        self.frames = 0;
        Ok(RgbaFrame::from_pixels(width, height, pixels)?)
    }

    fn height_limit(&self) -> u32 {
        if self.config.max_bytes == 0 {
            return self.config.max_height;
        }
        let row_bytes = (self.width as usize).saturating_mul(4).max(1);
        let samples = self.samples as usize;
        let overhead = 512 * samples;
        let footer_reserve = 256 * row_bytes;
        let rows_by_bytes = self
            .config
            .max_bytes
            .saturating_sub(overhead)
            .saturating_sub(footer_reserve)
            / (row_bytes + samples).max(1);
        self.config
            .max_height
            .min(rows_by_bytes.max(1).min(u32::MAX as usize) as u32)
    }

    fn limit_error(&self) -> StitchError {
        if self.height_limit() < self.config.max_height {
            StitchError::MemoryLimit(self.config.max_bytes)
        } else {
            StitchError::HeightLimit(self.config.max_height)
        }
    }

    fn report(
        &self,
        overlap_rows: u32,
        trimmed_top_rows: u32,
        trimmed_bottom_rows: u32,
        appended_rows: u32,
        prepended_rows: u32,
    ) -> AppendReport {
        AppendReport {
            overlap_rows,
            trimmed_top_rows,
            trimmed_bottom_rows,
            appended_rows,
            prepended_rows,
            total_height: self.height,
            frames: self.frames,
        }
    }

    fn append_frame_rows(&mut self, frame: &RgbaFrame, first_row: u32, rows: u32) {
        let row_bytes = self.width as usize * BYTES_PER_PIXEL;
        let stride = frame.stride();
        if stride == row_bytes {
            let start = first_row as usize * row_bytes;
            let length = rows as usize * row_bytes;
            self.pixels
                .extend_from_slice(&frame.pixels()[start..start + length]);
            return;
        }
        for row in first_row..first_row + rows {
            let start = row as usize * stride;
            self.pixels
                .extend_from_slice(&frame.pixels()[start..start + row_bytes]);
        }
    }

    fn edge_margin(&self) -> u32 {
        self.config
            .edge_margin
            .min(self.width / 8)
            .min(self.width.saturating_sub(2) / 2)
    }

    fn stable_rows(&self, signatures: &[u8], height: u32) -> Vec<bool> {
        let mut stable = vec![false; height as usize];
        if self.previous_frame_rows == 0 {
            return stable;
        }
        let samples = self.samples as usize;
        let rows = height.min(self.previous_frame_rows);
        for (row, slot) in stable.iter_mut().enumerate().take(rows as usize) {
            let start = row * samples;
            let new_row = &signatures[start..start + samples];
            let old_row = &self.previous_frame[start..start + samples];
            if signature_mean(new_row, old_row) <= f64::from(self.config.static_tolerance) {
                *slot = true;
            }
        }
        stable
    }

    fn remember_bands(&mut self, rows: &RowSet, height: u32) {
        self.previous_frame.clear();
        self.previous_frame.extend_from_slice(rows.signatures);
        self.previous_frame_rows = height;
        let samples = self.samples as usize;
        let top_rows = self.config.max_top_inset_rows.min(height);
        self.previous_top.clear();
        self.previous_top
            .extend_from_slice(&rows.signatures[..top_rows as usize * samples]);
        self.previous_top_rows = top_rows;

        let bottom_rows = self.config.max_bottom_inset_rows.min(height);
        let start = (height - bottom_rows) as usize * samples;
        self.previous_bottom.clear();
        self.previous_bottom
            .extend_from_slice(&rows.signatures[start..start + bottom_rows as usize * samples]);
        self.previous_bottom_rows = bottom_rows;
    }

    fn detect_static_top(&self, rows: &RowSet, height: u32) -> u32 {
        if self.frames == 0 || self.previous_top_rows == 0 {
            return 0;
        }
        let max_rows = self
            .config
            .max_top_inset_rows
            .min(self.previous_top_rows)
            .min(height.saturating_sub(self.config.min_overlap_rows));
        if max_rows < self.config.min_static_rows {
            return 0;
        }
        let samples = self.samples as usize;
        let mut matched = 0u32;
        while matched < max_rows {
            let start = matched as usize * samples;
            let new_row = &rows.signatures[start..start + samples];
            let old_row = &self.previous_top[start..start + samples];
            if signature_mean(new_row, old_row) > f64::from(self.config.static_tolerance) {
                break;
            }
            matched += 1;
        }
        if matched == max_rows
            || matched < self.config.min_static_rows
            || !self.band_is_chrome(rows, 0, matched, matched, height)
        {
            0
        } else {
            matched
        }
    }

    fn band_is_chrome(
        &self,
        rows: &RowSet,
        start: u32,
        end: u32,
        content_start: u32,
        content_end: u32,
    ) -> bool {
        if (start..end).any(|row| rows.has_content(row)) {
            return true;
        }
        let middle = start + (end - start) / 2;
        let band_tone = signature_median(rows.signature(middle));
        let background = rows_background(rows, content_start, content_end);
        band_tone.abs_diff(background) > 8
    }

    fn detect_static_bottom(&self, rows: &RowSet, height: u32) -> u32 {
        if self.frames == 0 || self.previous_bottom_rows == 0 {
            return 0;
        }
        let max_rows = self
            .config
            .max_bottom_inset_rows
            .min(self.previous_bottom_rows)
            .min(height.saturating_sub(self.config.min_overlap_rows));
        if max_rows < self.config.min_static_rows {
            return 0;
        }
        let samples = self.samples as usize;
        let mut matched = 0u32;
        while matched < max_rows {
            let distance = matched + 1;
            let new_start = (height - distance) as usize * samples;
            let old_start = (self.previous_bottom_rows - distance) as usize * samples;
            if signature_mean(
                &rows.signatures[new_start..new_start + samples],
                &self.previous_bottom[old_start..old_start + samples],
            ) > f64::from(self.config.static_tolerance)
            {
                break;
            }
            matched = distance;
        }
        if matched == max_rows
            || matched < self.config.min_static_rows
            || !self.band_is_chrome(rows, height - matched, height, 0, height - matched)
        {
            0
        } else {
            matched
        }
    }

    fn buffer_tail_matches(&self, rows: &RowSet, height: u32, footer_rows: u32) -> bool {
        if self.height < footer_rows {
            return false;
        }
        let samples = self.samples as usize;
        let stitched_start = (self.height - footer_rows) as usize * samples;
        let stitched_end = stitched_start + footer_rows as usize * samples;
        let Some(tail) = self.signatures.get(stitched_start..stitched_end) else {
            return false;
        };
        for row in 0..footer_rows as usize {
            let new_start = (height - footer_rows) as usize * samples + row * samples;
            let old_start = row * samples;
            if signature_mean(
                &rows.signatures[new_start..new_start + samples],
                &tail[old_start..old_start + samples],
            ) > f64::from(self.config.static_tolerance)
            {
                return false;
            }
        }
        true
    }

    fn store_footer(&mut self, frame: &RgbaFrame, height: u32, footer_rows: u32) {
        let row_bytes = self.width as usize * BYTES_PER_PIXEL;
        self.footer_pixels.clear();
        self.footer_pixels.reserve(footer_rows as usize * row_bytes);
        let first = height - footer_rows;
        for row in first..height {
            let start = row as usize * frame.stride();
            self.footer_pixels
                .extend_from_slice(&frame.pixels()[start..start + row_bytes]);
        }
        self.footer_rows = footer_rows;
    }

    fn find_tail_overlap(
        &self,
        new: &RowSet,
        inset: u32,
        content_height: u32,
    ) -> Option<(u32, f64)> {
        let max_overlap = if self.config.max_overlap_rows == 0 {
            self.height.min(content_height)
        } else {
            self.config
                .max_overlap_rows
                .min(self.height)
                .min(content_height)
        };
        let min_overlap = self.config.min_overlap_rows.min(max_overlap);
        if max_overlap == 0 || max_overlap < min_overlap {
            return None;
        }

        let sample_count = max_overlap as usize * self.samples as usize;
        let tail_start = (self.height - max_overlap) as usize * self.samples as usize;
        let previous_signatures = &self.signatures[tail_start..tail_start + sample_count];
        let previous_energies = row_energies(previous_signatures, max_overlap, self.samples);
        let previous = RowSet {
            signatures: previous_signatures,
            energies: &previous_energies,
            spreads: None,
            samples: self.samples as usize,
            stable: None,
        };

        let anchors: Vec<u32> = (0..max_overlap)
            .filter(|row| u32::from(previous.energy(*row)) >= self.config.anchor_threshold)
            .collect();
        let samples = self.samples as usize;

        let mut candidates: Vec<(f64, u64, u32)> = Vec::with_capacity(MAX_CANDIDATES);
        for overlap in (min_overlap..=max_overlap).rev() {
            let window = OverlapWindow {
                max_overlap,
                inset,
                overlap,
            };
            let (score, compared) =
                candidate_score(samples, &previous, new, &anchors, window, COARSE_ROWS);
            insert_candidate(&mut candidates, score, compared, overlap);
        }
        if candidates.is_empty() {
            return None;
        }

        let mut scored: Vec<(f64, u32)> = Vec::with_capacity(candidates.len());
        for (_, _, overlap) in &candidates {
            let window = OverlapWindow {
                max_overlap,
                inset,
                overlap: *overlap,
            };
            let score = full_row_score(samples, &previous, new, window);
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
            let window = OverlapWindow {
                max_overlap,
                inset,
                overlap,
            };
            let score = full_row_score(samples, &previous, new, window);
            if score < refined.0 {
                refined = (score, overlap);
            }
        }
        if refined.0 > f64::from(self.config.tolerance) {
            return None;
        }
        if !align::bands_consistent(&previous, new, max_overlap - refined.1, inset, refined.1) {
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
}

#[derive(Clone, Copy)]
struct OverlapWindow {
    max_overlap: u32,
    inset: u32,
    overlap: u32,
}

struct RowSet<'a> {
    signatures: &'a [u8],
    energies: &'a [u16],
    spreads: Option<&'a [u8]>,
    samples: usize,
    stable: Option<&'a [bool]>,
}

impl RowSet<'_> {
    fn signature(&self, row: u32) -> &[u8] {
        let start = row as usize * self.samples;
        &self.signatures[start..start + self.samples]
    }

    fn energy(&self, row: u32) -> u16 {
        self.energies[row as usize]
    }

    fn spread(&self, row: u32) -> u8 {
        self.spreads
            .and_then(|spreads| spreads.get(row as usize))
            .copied()
            .unwrap_or(u8::MAX)
    }

    fn has_content(&self, row: u32) -> bool {
        self.spread(row) >= MIN_ROW_SPREAD
    }

    fn is_stable(&self, row: u32) -> bool {
        self.stable
            .and_then(|stable| stable.get(row as usize))
            .copied()
            .unwrap_or(false)
    }
}

fn candidate_score(
    samples: usize,
    previous: &RowSet,
    new: &RowSet,
    anchors: &[u32],
    window: OverlapWindow,
    max_rows: usize,
) -> (f64, u64) {
    let prev_offset = window.max_overlap - window.overlap;
    let start = anchors.partition_point(|row| *row < prev_offset);
    let anchor_rows = anchors.len() - start;
    let mut total = 0u64;
    let mut compared = 0u64;
    if anchor_rows as u32 >= MIN_ANCHOR_ROWS {
        let stride = anchor_rows.div_ceil(max_rows.max(1)).max(1);
        for index in (start..anchors.len()).step_by(stride) {
            let row = anchors[index];
            let new_row = window.inset + row - prev_offset;
            if new.is_stable(new_row) {
                continue;
            }
            total += row_total(previous.signature(row), new.signature(new_row));
            compared += 1;
        }
    } else {
        let stride = (window.overlap as usize).div_ceil(max_rows.max(1)).max(1);
        for offset in (0..window.overlap as usize).step_by(stride) {
            let new_row = window.inset + offset as u32;
            if new.is_stable(new_row) {
                continue;
            }
            total += row_total(
                previous.signature(prev_offset + offset as u32),
                new.signature(new_row),
            );
            compared += 1;
        }
    }
    if compared == 0 {
        return (f64::INFINITY, 0);
    }
    (total as f64 / (compared as f64 * samples as f64), compared)
}

fn full_row_score(samples: usize, previous: &RowSet, new: &RowSet, window: OverlapWindow) -> f64 {
    let prev_offset = window.max_overlap - window.overlap;
    let mut total = 0u64;
    let mut compared = 0u64;
    for offset in 0..window.overlap {
        let new_row = window.inset + offset;
        if new.is_stable(new_row) {
            continue;
        }
        total += row_total(
            previous.signature(prev_offset + offset),
            new.signature(new_row),
        );
        compared += 1;
    }
    if compared == 0 {
        return f64::INFINITY;
    }
    total as f64 / (compared as f64 * samples as f64)
}

fn insert_candidate(
    candidates: &mut Vec<(f64, u64, u32)>,
    score: f64,
    compared: u64,
    overlap: u32,
) {
    if !score.is_finite() {
        return;
    }
    if candidates.len() >= MAX_CANDIDATES
        && candidates
            .last()
            .is_some_and(|entry| entry.0 < score || (entry.0 == score && entry.1 >= compared))
    {
        return;
    }
    let position = candidates
        .partition_point(|entry| entry.0 < score || (entry.0 == score && entry.1 >= compared));
    candidates.insert(position, (score, compared, overlap));
    candidates.truncate(MAX_CANDIDATES);
}

fn row_signatures(
    pixels: &[u8],
    stride: usize,
    width: u32,
    rows: u32,
    samples: u32,
    margin: u32,
) -> Vec<u8> {
    let samples = samples as usize;
    let margin = margin.min(width / 2);
    let left = margin;
    let right = width.saturating_sub(margin).max(left + 1);
    let span = right - left;
    let mut signatures = vec![0u8; rows as usize * samples];
    for row in 0..rows {
        let row_start = row as usize * stride;
        let output = &mut signatures[row as usize * samples..][..samples];
        for (sample, slot) in output.iter_mut().enumerate() {
            let x = if samples <= 1 || span <= 1 {
                left + span / 2
            } else {
                (left + sample as u32 * (span - 1) / (samples as u32 - 1)).min(width - 1)
            };
            let index = row_start + x as usize * BYTES_PER_PIXEL;
            *slot = core_types::luma8(pixels[index], pixels[index + 1], pixels[index + 2]);
        }
    }
    signatures
}

fn row_energies(signatures: &[u8], rows: u32, samples: u32) -> Vec<u16> {
    let samples = samples as usize;
    let mut energies = vec![0u16; rows as usize];
    for row in 1..rows as usize {
        let previous = &signatures[(row - 1) * samples..][..samples];
        let current = &signatures[row * samples..][..samples];
        let total: u32 = previous
            .iter()
            .zip(current)
            .map(|(left, right)| u32::from(left.abs_diff(*right)))
            .sum();
        energies[row] = (total / samples as u32) as u16;
    }
    energies
}

fn row_spreads(signatures: &[u8], rows: u32, samples: u32) -> Vec<u8> {
    let samples = samples as usize;
    (0..rows as usize)
        .map(|row| signature_spread(&signatures[row * samples..][..samples]))
        .collect()
}

fn signature_spread(signature: &[u8]) -> u8 {
    let max = signature.iter().copied().max().unwrap_or(0);
    let min = signature.iter().copied().min().unwrap_or(0);
    max - min
}

fn signature_median(signature: &[u8]) -> u8 {
    let mut histogram = [0u32; 256];
    for &value in signature {
        histogram[value as usize] += 1;
    }
    median_from_histogram(&histogram, signature.len() as u32)
}

fn rows_background(rows: &RowSet, start: u32, end: u32) -> u8 {
    let mut histogram = [0u32; 256];
    let mut total = 0u32;
    for row in start..end {
        for &value in rows.signature(row) {
            histogram[value as usize] += 1;
            total += 1;
        }
    }
    if total == 0 {
        return 0;
    }
    let (mode, count) = histogram
        .iter()
        .enumerate()
        .max_by_key(|(value, count)| (**count, std::cmp::Reverse(*value)))
        .map(|(value, count)| (value as u8, *count))
        .unwrap_or((0, 0));
    if count.saturating_mul(50) >= total {
        mode
    } else {
        median_from_histogram(&histogram, total)
    }
}

fn median_from_histogram(histogram: &[u32; 256], total: u32) -> u8 {
    if total == 0 {
        return 0;
    }
    let half = total / 2;
    let mut seen = 0u32;
    for (value, count) in histogram.iter().enumerate() {
        seen += count;
        if seen > half {
            return value as u8;
        }
    }
    0
}

fn row_total(left: &[u8], right: &[u8]) -> u64 {
    left.iter()
        .zip(right)
        .map(|(left, right)| u64::from(left.abs_diff(*right)))
        .sum()
}

fn signature_mean(left: &[u8], right: &[u8]) -> f64 {
    row_total(left, right) as f64 / left.len().max(1) as f64
}

#[cfg(test)]
mod extra_tests;
#[cfg(test)]
mod tests;
