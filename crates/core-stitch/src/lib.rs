use core_types::{FrameError, RgbaFrame};
use thiserror::Error;

const BYTES_PER_PIXEL: usize = 4;
/// Rows compared per candidate during the coarse pass.
const COARSE_ROWS: usize = 16;
/// Rows compared per candidate when a sparse fallback cannot use anchors.
const FULL_ROWS: usize = 32;
/// A candidate must beat the best one by this margin to avoid ambiguity.
const REFINE_RADIUS: u32 = 4;
const AMBIGUITY_MARGIN: f64 = 0.5;
/// Scores at or below this are treated as an exact match, where periodic
/// content cannot be told apart but any offset reproduces the same pixels.
const EXACT_MATCH_SCORE: f64 = 2.0;
const MAX_CANDIDATES: usize = 5;
const MIN_ANCHOR_ROWS: u32 = 6;

#[derive(Clone, Debug)]
pub struct StitchConfig {
    pub min_overlap_rows: u32,
    /// Upper bound for overlap candidates. `0` searches the full frame height,
    /// so a gentle scroll that leaves a large overlap can still be matched.
    pub max_overlap_rows: u32,
    /// Sampled columns per row. `0` derives one sample per four pixels,
    /// clamped to `48..=384`.
    pub sample_columns: u32,
    /// Mean absolute luminance difference allowed per sampled pixel.
    pub tolerance: u32,
    /// Rows that may be ignored at the top of later frames, for fixed headers.
    pub max_top_inset_rows: u32,
    /// Rows that may be ignored at the bottom of later frames, for fixed
    /// footers such as a chat input box.
    pub max_bottom_inset_rows: u32,
    /// Minimum height of a static band before it is treated as chrome.
    pub min_static_rows: u32,
    /// Mean difference allowed between rows of a fixed header or footer.
    pub static_tolerance: u32,
    /// Mean difference above which a row counts as a visual anchor.
    pub anchor_threshold: u32,
    pub max_height: u32,
    /// Upper bound for the retained RGBA buffer. `0` disables the byte cap.
    /// This prevents a very wide monitor from turning `max_height` into an
    /// unexpectedly large allocation.
    pub max_bytes: usize,
}

impl Default for StitchConfig {
    fn default() -> Self {
        Self {
            min_overlap_rows: 24,
            max_overlap_rows: 0,
            sample_columns: 0,
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
    pub total_height: u32,
    pub frames: u32,
}

pub struct Stitcher {
    width: u32,
    config: StitchConfig,
    samples: u32,
    pixels: Vec<u8>,
    /// One luminance signature row per stitched row, kept in sync with
    /// `pixels` so matching does not rescan the whole tail every frame.
    signatures: Vec<u8>,
    height: u32,
    frames: u32,
    /// Row signatures of the previous frame's top rows, used to detect a
    /// fixed header without comparing raw pixels again.
    previous_top: Vec<u8>,
    previous_top_rows: u32,
    /// Row signatures of the previous frame's bottom rows, used to detect a
    /// fixed footer.
    previous_bottom: Vec<u8>,
    previous_bottom_rows: u32,
    /// Fixed footer rows kept out of the stitched content and appended once.
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
            previous_top: Vec::new(),
            previous_top_rows: 0,
            previous_bottom: Vec::new(),
            previous_bottom_rows: 0,
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
        );
        let energies = row_energies(&signatures, frame.height(), self.samples);
        let new = RowSet {
            signatures: &signatures,
            energies: &energies,
            samples: self.samples as usize,
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
            return Ok(self.report(0, 0, 0, self.height));
        }

        // A fixed footer is identical at the same distance from the bottom in
        // consecutive frames. Detect it before matching, keep it out of the
        // appended content, and append it once at the end.
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
        let content_height = frame
            .height()
            .saturating_sub(footer_rows)
            .saturating_sub(top_inset);
        let overlap = if content_height == 0 {
            Some(0)
        } else {
            self.find_overlap(&new, top_inset, content_height)
        };
        let Some(overlap) = overlap else {
            self.remember_bands(&new, frame.height());
            return Err(StitchError::NoOverlap);
        };

        let appended = content_height.saturating_sub(overlap);
        if appended == 0 {
            self.frames += 1;
            self.remember_bands(&new, frame.height());
            return Ok(self.report(overlap, top_inset, footer_rows, 0));
        }
        let room = self.height_limit().saturating_sub(self.height);
        if room < appended {
            self.remember_bands(&new, frame.height());
            return Err(self.limit_error());
        }
        self.reserve_rows(appended as usize)?;
        self.append_frame_rows(frame, top_inset + overlap, appended);
        let signature_start = (top_inset + overlap) as usize * self.samples as usize;
        let signature_end = signature_start + appended as usize * self.samples as usize;
        self.signatures
            .extend_from_slice(&signatures[signature_start..signature_end]);
        self.height += appended;
        self.frames += 1;
        self.remember_bands(&new, frame.height());
        Ok(self.report(overlap, top_inset, footer_rows, appended))
    }

    /// Starts a session from an owned first frame without copying it.
    ///
    /// Later frames still go through [`Self::push`]; only the first frame can
    /// donate its buffer to the stitched image.
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
        );
        let energies = row_energies(&signatures, height, self.samples);
        let rows = RowSet {
            signatures: &signatures,
            energies: &energies,
            samples: self.samples as usize,
        };
        self.height = height;
        self.frames = 1;
        self.remember_bands(&rows, height);
        self.pixels = frame.into_packed_pixels();
        self.signatures = signatures;
        Ok(self.report(0, 0, 0, height))
    }

    /// Grows the stitched buffer geometrically but never past the configured
    /// byte budget. `extend_from_slice` alone may double the capacity, which
    /// turned a 256 MiB cap into a 512 MiB allocation on wide captures.
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
        let grown = needed.saturating_add(needed / 4);
        let target = grown.min(limit).max(needed);
        self.pixels
            .try_reserve_exact(target - self.pixels.len())
            .map_err(|_| StitchError::AllocationFailed(target))
    }

    /// Consumes the accumulated pixels without cloning the full stitched image.
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
        self.footer_pixels.clear();
        self.previous_top.clear();
        self.previous_top_rows = 0;
        self.previous_bottom.clear();
        self.previous_bottom_rows = 0;
        let width = self.width;
        let height = self.height;
        let pixels = std::mem::take(&mut self.pixels);
        self.signatures.clear();
        self.height = 0;
        self.frames = 0;
        Ok(RgbaFrame::from_pixels(width, height, pixels)?)
    }

    fn height_limit(&self) -> u32 {
        if self.config.max_bytes == 0 {
            return self.config.max_height;
        }
        let row_bytes = (self.width as usize).saturating_mul(4).max(1);
        let rows_by_bytes = (self.config.max_bytes / row_bytes).max(1);
        self.config
            .max_height
            .min(rows_by_bytes.min(u32::MAX as usize) as u32)
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
    ) -> AppendReport {
        AppendReport {
            overlap_rows,
            trimmed_top_rows,
            trimmed_bottom_rows,
            appended_rows,
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

    fn remember_bands(&mut self, rows: &RowSet, height: u32) {
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

    /// Height of the static band at the top of the frame, or `0` when the top
    /// matches the previous frame all the way to the cap (no scrolled frame can
    /// be told apart from a duplicate) or is shorter than a real header.
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
        if matched == max_rows || matched < self.config.min_static_rows {
            0
        } else {
            matched
        }
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
        if matched == max_rows || matched < self.config.min_static_rows {
            0
        } else {
            matched
        }
    }

    /// Whether the stitched tail still ends with the footer that is about to
    /// be kept out of the content. Only then can it be trimmed safely.
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

    fn find_overlap(&self, new: &RowSet, inset: u32, content_height: u32) -> Option<u32> {
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
            samples: self.samples as usize,
        };

        let anchors: Vec<u32> = (0..max_overlap)
            .filter(|row| u32::from(previous.energy(*row)) >= self.config.anchor_threshold)
            .collect();
        let samples = self.samples as usize;

        // Coarse pass over every candidate, keeping the best few. The last
        // row is always part of the refinement window, so an off-by-one
        // alignment cannot win by matching only the middle of the overlap.
        let mut candidates: Vec<(f64, u32)> = Vec::with_capacity(MAX_CANDIDATES);
        for overlap in (min_overlap..=max_overlap).rev() {
            let window = OverlapWindow {
                max_overlap,
                inset,
                overlap,
            };
            let score = candidate_score(samples, &previous, new, &anchors, window, COARSE_ROWS);
            insert_candidate(&mut candidates, score, overlap);
        }
        if candidates.is_empty() {
            return None;
        }

        let mut scored: Vec<(f64, u32)> = Vec::with_capacity(candidates.len());
        for (_, overlap) in &candidates {
            let window = OverlapWindow {
                max_overlap,
                inset,
                overlap: *overlap,
            };
            let score = candidate_score(samples, &previous, new, &anchors, window, FULL_ROWS);
            scored.push((score, *overlap));
        }
        scored.sort_by(|left, right| left.0.total_cmp(&right.0));
        let (best_score, best_overlap) = scored[0];
        let runner = scored
            .iter()
            .skip(1)
            .find(|(_, overlap)| overlap.abs_diff(best_overlap) > REFINE_RADIUS)
            .map_or(f64::INFINITY, |entry| entry.0);

        // Refine every row around the best candidate; sampled rows alone can
        // let a one-row-shifted alignment through, which duplicates or drops a
        // line at the seam.
        let low = best_overlap.saturating_sub(REFINE_RADIUS).max(min_overlap);
        let high = (best_overlap + REFINE_RADIUS).min(max_overlap);
        // Walk from the largest overlap down: on uniform content every offset
        // scores zero and appending nothing is the safe choice.
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
        if runner.is_finite()
            && runner - best_score < AMBIGUITY_MARGIN
            && best_score > EXACT_MATCH_SCORE
        {
            return None;
        }
        Some(refined.1)
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
    samples: usize,
}

impl RowSet<'_> {
    fn signature(&self, row: u32) -> &[u8] {
        let start = row as usize * self.samples;
        &self.signatures[start..start + self.samples]
    }

    fn energy(&self, row: u32) -> u16 {
        self.energies[row as usize]
    }
}

/// Mean absolute difference for one candidate alignment.
///
/// Rows with visual edges are compared first: blank rows score equally well
/// for every offset and would otherwise dominate the mean. When the overlap
/// has too few anchors, evenly spread rows are compared instead so uniform
/// content still produces a usable score.
fn candidate_score(
    samples: usize,
    previous: &RowSet,
    new: &RowSet,
    anchors: &[u32],
    window: OverlapWindow,
    max_rows: usize,
) -> f64 {
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
            total += row_total(previous.signature(row), new.signature(new_row));
            compared += 1;
        }
    } else {
        let stride = (window.overlap as usize).div_ceil(max_rows.max(1)).max(1);
        for offset in (0..window.overlap as usize).step_by(stride) {
            total += row_total(
                previous.signature(prev_offset + offset as u32),
                new.signature(window.inset + offset as u32),
            );
            compared += 1;
        }
    }
    total as f64 / (compared.max(1) as f64 * samples as f64)
}

/// Mean absolute difference over every row of the candidate alignment. The
/// seam is decided with this score so a single unmatched edge row cannot hide
/// behind blank rows or anchor sampling.
fn full_row_score(samples: usize, previous: &RowSet, new: &RowSet, window: OverlapWindow) -> f64 {
    let prev_offset = window.max_overlap - window.overlap;
    let mut total = 0u64;
    for offset in 0..window.overlap {
        total += row_total(
            previous.signature(prev_offset + offset),
            new.signature(window.inset + offset),
        );
    }
    total as f64 / (window.overlap.max(1) as f64 * samples as f64)
}

fn insert_candidate(candidates: &mut Vec<(f64, u32)>, score: f64, overlap: u32) {
    if candidates.len() >= MAX_CANDIDATES
        && score >= candidates.last().map_or(f64::INFINITY, |entry| entry.0)
    {
        return;
    }
    let position = candidates.partition_point(|entry| entry.0 <= score);
    candidates.insert(position, (score, overlap));
    candidates.truncate(MAX_CANDIDATES);
}

fn row_signatures(pixels: &[u8], stride: usize, width: u32, rows: u32, samples: u32) -> Vec<u8> {
    let samples = samples as usize;
    let mut signatures = vec![0u8; rows as usize * samples];
    for row in 0..rows {
        let row_start = row as usize * stride;
        let output = &mut signatures[row as usize * samples..][..samples];
        for (sample, slot) in output.iter_mut().enumerate() {
            let x = if samples <= 1 {
                width / 2
            } else {
                (sample as u32 * (width - 1) / (samples as u32 - 1)).min(width - 1)
            };
            let index = row_start + x as usize * BYTES_PER_PIXEL;
            let luminance = (u32::from(pixels[index]) * 299
                + u32::from(pixels[index + 1]) * 587
                + u32::from(pixels[index + 2]) * 114)
                / 1000;
            *slot = luminance as u8;
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
mod tests {
    use super::*;

    fn patterned_frame(width: u32, height: u32, seed: u32) -> RgbaFrame {
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        let mut state = seed | 1;
        for pixel in pixels.as_chunks_mut::<4>().0 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            pixel[0] = (state >> 8) as u8;
            pixel[1] = (state >> 16) as u8;
            pixel[2] = (state >> 24) as u8;
            pixel[3] = 255;
        }
        RgbaFrame::from_pixels(width, height, pixels).unwrap()
    }

    fn gradient_frame(width: u32, height: u32) -> RgbaFrame {
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        for y in 0..height {
            for x in 0..width {
                let index = (y as usize * width as usize + x as usize) * 4;
                let value = (y * 200 / height.max(1)) as u8;
                let value = if y % 37 == 0 {
                    (x * 255 / width.max(1)) as u8
                } else {
                    value
                };
                pixels[index] = value;
                pixels[index + 1] = value;
                pixels[index + 2] = value;
                pixels[index + 3] = 255;
            }
        }
        RgbaFrame::from_pixels(width, height, pixels).unwrap()
    }

    fn slice_rows(frame: &RgbaFrame, start: u32, end: u32) -> RgbaFrame {
        let start_index = start as usize * frame.width() as usize * 4;
        let end_index = end as usize * frame.width() as usize * 4;
        RgbaFrame::from_pixels(
            frame.width(),
            end - start,
            frame.pixels()[start_index..end_index].to_vec(),
        )
        .unwrap()
    }

    fn stitcher(width: u32) -> Stitcher {
        Stitcher::new(width, StitchConfig::default()).unwrap()
    }

    fn with_header(content: &RgbaFrame, header: u32) -> RgbaFrame {
        let width = content.width();
        let mut pixels = vec![0u8; width as usize * header as usize * 4];
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&[24, 24, 24, 255]);
        }
        pixels.extend_from_slice(content.pixels());
        RgbaFrame::from_pixels(width, header + content.height(), pixels).unwrap()
    }

    fn with_footer(content: &RgbaFrame, footer: u32) -> RgbaFrame {
        let width = content.width();
        let mut pixels = content.pixels().to_vec();
        for _ in 0..footer {
            for _ in 0..width {
                pixels.extend_from_slice(&[200, 80, 40, 255]);
            }
        }
        RgbaFrame::from_pixels(width, content.height() + footer, pixels).unwrap()
    }

    #[test]
    fn first_frame_is_kept_whole() {
        let frame = patterned_frame(40, 100, 7);
        let mut stitches = stitcher(40);
        let report = stitches.push(&frame).unwrap();
        assert_eq!(report.appended_rows, 100);
        assert_eq!(stitches.height(), 100);
        assert_eq!(stitches.finish().unwrap().pixels(), frame.pixels());
    }

    #[test]
    fn overlapping_frames_are_stitched() {
        let original = patterned_frame(48, 300, 42);
        let mut stitches = stitcher(48);
        stitches.push(&slice_rows(&original, 0, 140)).unwrap();
        stitches.push(&slice_rows(&original, 100, 240)).unwrap();
        stitches.push(&slice_rows(&original, 200, 300)).unwrap();
        let stitched = stitches.finish().unwrap();
        assert_eq!(stitched.width(), 48);
        assert_eq!(stitched.height(), 300);
        assert_eq!(stitched.pixels(), original.pixels());
    }

    #[test]
    fn gentle_scroll_with_large_overlap_is_stitched() {
        let original = patterned_frame(48, 1500, 21);
        let mut stitches = stitcher(48);
        stitches.push(&slice_rows(&original, 0, 1000)).unwrap();
        let report = stitches.push(&slice_rows(&original, 120, 1120)).unwrap();
        assert_eq!(report.overlap_rows, 880);
        assert_eq!(report.appended_rows, 120);
        let stitched = stitches.finish().unwrap();
        assert_eq!(stitched.height(), 1120);
        assert_eq!(stitched.pixels(), &original.pixels()[..1120 * 48 * 4]);
    }

    #[test]
    fn refinement_picks_the_exact_alignment() {
        let original = gradient_frame(64, 600);
        let mut stitches = stitcher(64);
        stitches.push(&slice_rows(&original, 0, 300)).unwrap();
        stitches.push(&slice_rows(&original, 60, 360)).unwrap();
        let stitched = stitches.finish().unwrap();
        assert_eq!(stitched.height(), 360);
        assert_eq!(stitched.pixels(), &original.pixels()[..360 * 64 * 4]);
    }

    #[test]
    fn duplicate_frame_appends_nothing() {
        let frame = patterned_frame(32, 120, 9);
        let mut stitches = stitcher(32);
        stitches.push(&frame).unwrap();
        let report = stitches.push(&frame).unwrap();
        assert_eq!(report.overlap_rows, 120);
        assert_eq!(report.appended_rows, 0);
        assert_eq!(stitches.height(), 120);
    }

    #[test]
    fn fixed_header_is_trimmed_from_later_frames() {
        let original = patterned_frame(48, 300, 17);
        let header = 20;
        let mut stitches = stitcher(48);
        stitches
            .push(&with_header(&slice_rows(&original, 0, 140), header))
            .unwrap();
        let report = stitches
            .push(&with_header(&slice_rows(&original, 100, 240), header))
            .unwrap();
        assert_eq!(report.trimmed_top_rows, header);
        stitches
            .push(&with_header(&slice_rows(&original, 200, 300), header))
            .unwrap();

        let stitched = stitches.finish().unwrap();
        assert_eq!(stitched.height(), header + 300);
        let header_bytes = header as usize * 48 * 4;
        assert_eq!(
            &stitched.pixels()[header_bytes..],
            original.pixels(),
            "content below the header should match the original image"
        );
    }

    #[test]
    fn tall_fixed_header_is_trimmed() {
        let original = patterned_frame(48, 400, 23);
        let header = 200;
        let mut stitches = stitcher(48);
        stitches
            .push(&with_header(&slice_rows(&original, 0, 160), header))
            .unwrap();
        let report = stitches
            .push(&with_header(&slice_rows(&original, 120, 280), header))
            .unwrap();
        assert_eq!(report.trimmed_top_rows, header);
        stitches
            .push(&with_header(&slice_rows(&original, 240, 400), header))
            .unwrap();
        let stitched = stitches.finish().unwrap();
        assert_eq!(stitched.height(), header + 400);
        let header_bytes = header as usize * 48 * 4;
        assert_eq!(&stitched.pixels()[header_bytes..], original.pixels());
    }

    #[test]
    fn fixed_footer_is_appended_once() {
        let original = patterned_frame(48, 400, 51);
        let footer = 32;
        let mut stitches = stitcher(48);
        stitches
            .push(&with_footer(&slice_rows(&original, 0, 160), footer))
            .unwrap();
        stitches
            .push(&with_footer(&slice_rows(&original, 100, 260), footer))
            .unwrap();
        stitches
            .push(&with_footer(&slice_rows(&original, 200, 360), footer))
            .unwrap();
        let stitched = stitches.finish().unwrap();
        assert_eq!(stitched.height(), 360 + footer);
        let content_bytes = 360 * 48 * 4;
        assert_eq!(
            &stitched.pixels()[..content_bytes],
            &original.pixels()[..content_bytes]
        );
        let footer_pixels = &stitched.pixels()[content_bytes..];
        for pixel in footer_pixels.as_chunks::<4>().0 {
            assert_eq!(*pixel, [200, 80, 40, 255]);
        }
    }

    #[test]
    fn fixed_header_and_footer_are_trimmed_together() {
        let original = patterned_frame(48, 300, 61);
        let header = 16;
        let footer = 24;
        let mut stitches = stitcher(48);
        for (start, end) in [(0, 140), (100, 240), (200, 300)] {
            let content = with_footer(
                &with_header(&slice_rows(&original, start, end), header),
                footer,
            );
            stitches.push(&content).unwrap();
        }

        let stitched = stitches.finish().unwrap();
        assert_eq!(stitched.height(), header + 300 + footer);
        let header_bytes = header as usize * 48 * 4;
        let content_bytes = 300 * 48 * 4;
        assert_eq!(
            &stitched.pixels()[header_bytes..header_bytes + content_bytes],
            original.pixels(),
            "content between the fixed header and footer should match the original"
        );
        for pixel in stitched.pixels()[header_bytes + content_bytes..]
            .as_chunks::<4>()
            .0
        {
            assert_eq!(*pixel, [200, 80, 40, 255]);
        }
    }

    #[test]
    fn blank_frames_append_nothing() {
        let width = 32;
        let pixels = vec![255u8; width as usize * 120 * 4];
        let frame = RgbaFrame::from_pixels(width, 120, pixels).unwrap();
        let mut stitches = stitcher(width);
        stitches.push(&frame).unwrap();
        let report = stitches.push(&frame).unwrap();
        assert_eq!(report.appended_rows, 0);
        assert_eq!(stitches.height(), 120);
    }

    #[test]
    fn width_mismatch_is_rejected() {
        let frame = patterned_frame(30, 120, 3);
        let mut stitches = stitcher(32);
        assert!(matches!(
            stitches.push(&frame),
            Err(StitchError::WidthMismatch { .. })
        ));
    }

    #[test]
    fn disjoint_frames_report_no_overlap() {
        let mut stitches = stitcher(40);
        stitches.push(&patterned_frame(40, 120, 11)).unwrap();
        let result = stitches.push(&patterned_frame(40, 120, 29));
        assert!(matches!(result, Err(StitchError::NoOverlap)));
    }

    #[test]
    fn height_limit_stops_growth() {
        let mut stitches = Stitcher::new(
            16,
            StitchConfig {
                max_height: 180,
                ..Default::default()
            },
        )
        .unwrap();
        let original = patterned_frame(16, 300, 5);
        stitches.push(&slice_rows(&original, 0, 180)).unwrap();
        let result = stitches.push(&slice_rows(&original, 140, 240));
        assert!(matches!(result, Err(StitchError::HeightLimit(180))));
    }

    #[test]
    fn memory_budget_stops_wide_frames() {
        let mut stitches = Stitcher::new(
            64,
            StitchConfig {
                max_height: 10_000,
                max_bytes: 64 * 4 * 60,
                ..Default::default()
            },
        )
        .unwrap();
        let original = patterned_frame(64, 120, 31);
        stitches.push(&slice_rows(&original, 0, 60)).unwrap();
        let result = stitches.push(&slice_rows(&original, 30, 90));
        assert!(matches!(result, Err(StitchError::MemoryLimit(_))));
    }

    #[test]
    fn owned_first_frame_donates_its_buffer() {
        let frame = patterned_frame(40, 100, 13);
        let pointer = frame.pixels().as_ptr();
        let mut stitches = stitcher(40);
        let report = stitches.push_owned(frame).unwrap();
        assert_eq!(report.appended_rows, 100);
        assert_eq!(stitches.height(), 100);
        assert_eq!(stitches.pixels.as_ptr(), pointer);
    }

    #[test]
    fn capacity_never_exceeds_the_byte_budget() {
        let width = 32u32;
        let row_bytes = width as usize * 4;
        let max_bytes = row_bytes * 500;
        let mut stitches = Stitcher::new(
            width,
            StitchConfig {
                max_height: 10_000,
                max_bytes,
                ..Default::default()
            },
        )
        .unwrap();
        let original = patterned_frame(width, 400, 41);
        for (start, end) in [(0, 120), (90, 210), (180, 300), (270, 390)] {
            stitches.push(&slice_rows(&original, start, end)).unwrap();
            assert!(
                stitches.pixels.capacity() <= max_bytes,
                "capacity {} exceeds budget {max_bytes}",
                stitches.pixels.capacity()
            );
        }
        assert!(stitches.finish().is_ok());
    }
}
