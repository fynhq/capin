use std::time::Duration;

use crate::FRAME_RATE;

/// Maps capture timestamps onto a fixed 30 fps output frame index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameTimeline {
    start: Option<Duration>,
    next_index: u64,
}

impl FrameTimeline {
    pub fn new() -> Self {
        Self {
            start: None,
            next_index: 0,
        }
    }

    /// The first timestamp defines frame zero. Frames that map to an already
    /// emitted index are dropped; gaps report how many previous frames the
    /// encoder must duplicate.
    pub fn submit(&mut self, timestamp: Duration) -> FrameDecision {
        let start = *self.start.get_or_insert(timestamp);
        let elapsed = timestamp.saturating_sub(start);
        let index = (elapsed.as_nanos() * u128::from(FRAME_RATE) / 1_000_000_000) as u64;

        if index < self.next_index {
            return FrameDecision::Drop;
        }

        let duplicates = index - self.next_index;
        self.next_index = index + 1;
        FrameDecision::Encode { index, duplicates }
    }

    /// Index that the next encoded frame will receive.
    pub const fn next_index(&self) -> u64 {
        self.next_index
    }

    pub fn reset(&mut self) {
        self.start = None;
        self.next_index = 0;
    }
}

impl Default for FrameTimeline {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameDecision {
    /// Encode this frame as `index`; duplicate the previous frame
    /// `duplicates` times first to fill a gap.
    Encode {
        index: u64,
        duplicates: u64,
    },
    Drop,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_frame_defines_index_zero() {
        let mut timeline = FrameTimeline::new();
        assert_eq!(
            timeline.submit(Duration::from_secs(5)),
            FrameDecision::Encode {
                index: 0,
                duplicates: 0
            }
        );
    }

    #[test]
    fn consecutive_frames_map_to_increasing_indices() {
        let mut timeline = FrameTimeline::new();
        assert_eq!(
            timeline.submit(Duration::ZERO),
            FrameDecision::Encode {
                index: 0,
                duplicates: 0
            }
        );
        assert_eq!(
            timeline.submit(Duration::from_millis(34)),
            FrameDecision::Encode {
                index: 1,
                duplicates: 0
            }
        );
        assert_eq!(
            timeline.submit(Duration::from_millis(67)),
            FrameDecision::Encode {
                index: 2,
                duplicates: 0
            }
        );
    }

    #[test]
    fn late_frames_are_dropped() {
        let mut timeline = FrameTimeline::new();
        timeline.submit(Duration::ZERO);
        timeline.submit(Duration::from_millis(100));
        assert_eq!(
            timeline.submit(Duration::from_millis(50)),
            FrameDecision::Drop
        );
    }

    #[test]
    fn gaps_report_duplicate_count() {
        let mut timeline = FrameTimeline::new();
        timeline.submit(Duration::ZERO);
        assert_eq!(
            timeline.submit(Duration::from_millis(100)),
            FrameDecision::Encode {
                index: 3,
                duplicates: 2
            }
        );
    }

    #[test]
    fn reset_starts_a_new_timeline() {
        let mut timeline = FrameTimeline::new();
        timeline.submit(Duration::ZERO);
        timeline.submit(Duration::from_millis(500));
        timeline.reset();
        assert_eq!(
            timeline.submit(Duration::from_secs(10)),
            FrameDecision::Encode {
                index: 0,
                duplicates: 0
            }
        );
    }
}
