use core_types::{FrameError, RgbaFrame};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct StitchConfig {
    pub min_overlap_rows: u32,
    pub max_overlap_rows: u32,
    pub sample_columns: u32,
    pub row_step: u32,
    /// Mean absolute luminance difference allowed per sampled pixel.
    pub tolerance: u32,
    /// Rows that may be ignored at the top of later frames, for fixed headers.
    pub max_top_inset_rows: u32,
    pub max_height: u32,
}

impl Default for StitchConfig {
    fn default() -> Self {
        Self {
            min_overlap_rows: 24,
            max_overlap_rows: 720,
            sample_columns: 48,
            row_step: 2,
            tolerance: 14,
            max_top_inset_rows: 160,
            max_height: 16_000,
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
    #[error("no vertical overlap was found between frames")]
    NoOverlap,
    #[error(transparent)]
    Frame(#[from] FrameError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppendReport {
    pub overlap_rows: u32,
    pub trimmed_top_rows: u32,
    pub appended_rows: u32,
    pub total_height: u32,
    pub frames: u32,
}

pub struct Stitcher {
    width: u32,
    config: StitchConfig,
    pixels: Vec<u8>,
    height: u32,
    frames: u32,
}

impl Stitcher {
    pub fn new(width: u32, config: StitchConfig) -> Result<Self, StitchError> {
        if width == 0 {
            return Err(StitchError::EmptyFrame);
        }
        Ok(Self {
            width,
            config,
            pixels: Vec::new(),
            height: 0,
            frames: 0,
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

        if self.frames == 0 {
            if frame.height() > self.config.max_height {
                return Err(StitchError::HeightLimit(self.config.max_height));
            }
            self.pixels.extend_from_slice(frame.pixels());
            self.height = frame.height();
            self.frames = 1;
            return Ok(AppendReport {
                overlap_rows: 0,
                trimmed_top_rows: 0,
                appended_rows: self.height,
                total_height: self.height,
                frames: self.frames,
            });
        }

        let (overlap, inset) = self.find_overlap(frame).ok_or(StitchError::NoOverlap)?;
        let appended = frame.height() - inset - overlap;
        if appended == 0 {
            self.frames += 1;
            return Ok(AppendReport {
                overlap_rows: overlap,
                trimmed_top_rows: inset,
                appended_rows: 0,
                total_height: self.height,
                frames: self.frames,
            });
        }
        let room = self.config.max_height.saturating_sub(self.height);
        if room < appended {
            return Err(StitchError::HeightLimit(self.config.max_height));
        }
        let start = (inset + overlap) as usize * self.width as usize * 4;
        let end = start + appended as usize * self.width as usize * 4;
        self.pixels.extend_from_slice(&frame.pixels()[start..end]);
        self.height += appended;
        self.frames += 1;
        Ok(AppendReport {
            overlap_rows: overlap,
            trimmed_top_rows: inset,
            appended_rows: appended,
            total_height: self.height,
            frames: self.frames,
        })
    }

    /// Consumes the accumulated pixels without cloning the full stitched image.
    pub fn finish(&mut self) -> Result<RgbaFrame, StitchError> {
        if self.frames == 0 || self.height == 0 {
            return Err(StitchError::EmptyFrame);
        }
        let width = self.width;
        let height = self.height;
        let pixels = std::mem::take(&mut self.pixels);
        self.height = 0;
        self.frames = 0;
        Ok(RgbaFrame::from_pixels(width, height, pixels)?)
    }

    fn find_overlap(&self, frame: &RgbaFrame) -> Option<(u32, u32)> {
        let samples = self.config.sample_columns.max(1).min(self.width);
        let max_overlap = self
            .config
            .max_overlap_rows
            .min(self.height)
            .min(frame.height());
        let min_overlap = self.config.min_overlap_rows.min(max_overlap);
        if max_overlap < min_overlap || max_overlap == 0 {
            return None;
        }

        let mut next_signatures = vec![0u8; frame.height() as usize * samples as usize];
        for row in 0..frame.height() {
            signature_row(
                frame.pixels(),
                frame.width(),
                row,
                samples,
                &mut next_signatures[row as usize * samples as usize..][..samples as usize],
            );
        }

        let mut previous_signatures = vec![0u8; max_overlap as usize * samples as usize];
        let first_row = self.height - max_overlap;
        for row in 0..max_overlap {
            signature_row(
                &self.pixels,
                self.width,
                first_row + row,
                samples,
                &mut previous_signatures[row as usize * samples as usize..][..samples as usize],
            );
        }

        let base_step = self.config.row_step.max(1);
        let tolerance = f64::from(self.config.tolerance);
        let sample_count = f64::from(samples);

        // Mean absolute difference for one candidate alignment. Sampling the
        // full overlap and verifying the last row avoids the old "first
        // passing overlap wins" behavior, which could pick a large offset on
        // periodic or mostly blank content.
        let mean_difference = |inset: u32, overlap: u32, row_step: u32| -> Option<f64> {
            let previous_offset = (max_overlap - overlap) as usize * samples as usize;
            let mut rows: Vec<u32> = (0..overlap).step_by(row_step.max(1) as usize).collect();
            if rows.last().copied() != Some(overlap - 1) {
                rows.push(overlap - 1);
            }
            let mut difference = 0u64;
            for row in rows {
                let previous = &previous_signatures
                    [previous_offset + row as usize * samples as usize..][..samples as usize];
                let next_index = (inset + row) as usize * samples as usize;
                let next = &next_signatures[next_index..][..samples as usize];
                for (left, right) in previous.iter().zip(next) {
                    difference += u64::from(left.abs_diff(*right));
                }
            }
            let compared = (overlap.div_ceil(row_step.max(1)) as f64) * sample_count;
            (compared > 0.0).then_some(difference as f64 / compared)
        };

        // Returns the best passing alignment plus the runner-up mean. A best
        // candidate that is not clearly better than the runner-up is treated
        // as ambiguous instead of silently stitching the wrong offset.
        enum SearchResult {
            Found(u32),
            Ambiguous,
            None,
        }
        let search = |inset: u32, row_step: u32| -> SearchResult {
            let available = frame.height().saturating_sub(inset);
            let max = max_overlap.min(available);
            let min = min_overlap.min(max);
            if max < min || max == 0 {
                return SearchResult::None;
            }
            let first_next_index = inset as usize * samples as usize;
            let first_next = &next_signatures[first_next_index..][..samples as usize];
            let mut best: Option<(u32, f64)> = None;
            let mut second = f64::INFINITY;
            for overlap in (min..=max).rev() {
                let previous_offset = (max_overlap - overlap) as usize * samples as usize;
                let first_previous = &previous_signatures[previous_offset..][..samples as usize];
                let first_difference: u64 = first_previous
                    .iter()
                    .zip(first_next)
                    .map(|(left, right)| u64::from(left.abs_diff(*right)))
                    .sum();
                if first_difference > u64::from(self.config.tolerance) * u64::from(samples) * 2 {
                    continue;
                }
                let Some(mean) = mean_difference(inset, overlap, row_step) else {
                    continue;
                };
                if mean > tolerance {
                    continue;
                }
                match best {
                    Some((_, best_mean)) if mean >= best_mean - 0.01 => {
                        second = second.min(mean);
                    }
                    Some((_, best_mean)) => {
                        second = second.min(best_mean);
                        best = Some((overlap, mean));
                    }
                    None => best = Some((overlap, mean)),
                }
            }
            let Some((overlap, mean)) = best else {
                return SearchResult::None;
            };
            let Some(verified) = mean_difference(inset, overlap, 1) else {
                return SearchResult::None;
            };
            if verified > tolerance {
                return SearchResult::None;
            }
            if second.is_finite() && second - mean < 0.75 {
                return SearchResult::Ambiguous;
            }
            SearchResult::Found(overlap)
        };

        match search(0, base_step) {
            SearchResult::Found(overlap) => return Some((overlap, 0)),
            // Uniform content makes every offset equally plausible. Trying
            // all insets would be expensive and would not make the match more
            // reliable, so stop and let the caller fall back to manual scroll.
            SearchResult::Ambiguous => return None,
            SearchResult::None => {}
        }
        let max_inset = self
            .config
            .max_top_inset_rows
            .min(frame.height().saturating_sub(self.config.min_overlap_rows));
        for inset in 1..=max_inset {
            match search(inset, base_step.saturating_mul(2)) {
                SearchResult::Found(overlap) => return Some((overlap, inset)),
                SearchResult::Ambiguous => return None,
                SearchResult::None => {}
            }
        }
        None
    }
}

fn signature_row(pixels: &[u8], width: u32, row: u32, samples: u32, output: &mut [u8]) {
    let row_start = row as usize * width as usize * 4;
    for (sample, slot) in output.iter_mut().enumerate() {
        let x = if samples <= 1 {
            width / 2
        } else {
            sample as u32 * (width - 1) / (samples - 1)
        };
        let index = row_start + x as usize * 4;
        let luminance = (u32::from(pixels[index]) * 299
            + u32::from(pixels[index + 1]) * 587
            + u32::from(pixels[index + 2]) * 114)
            / 1000;
        *slot = luminance as u8;
    }
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
}
