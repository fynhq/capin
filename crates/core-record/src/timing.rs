use std::fmt;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FrameRate {
    #[default]
    Fps30,
    Fps60,
}

impl FrameRate {
    pub const fn as_u32(self) -> u32 {
        match self {
            Self::Fps30 => 30,
            Self::Fps60 => 60,
        }
    }

    pub const fn as_f32(self) -> f32 {
        self.as_u32() as f32
    }

    pub const fn interval(self) -> Duration {
        Duration::from_nanos(1_000_000_000 / self.as_u32() as u64)
    }
}

impl fmt::Display for FrameRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} fps", self.as_u32())
    }
}

/// Maps capture timestamps onto a fixed output frame index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameTimeline {
    frame_rate: FrameRate,
    start: Option<Duration>,
    next_index: u64,
}

impl FrameTimeline {
    pub fn new(frame_rate: FrameRate) -> Self {
        Self {
            frame_rate,
            start: None,
            next_index: 0,
        }
    }

    pub fn frame_rate(&self) -> FrameRate {
        self.frame_rate
    }

    /// The first timestamp defines frame zero. Frames that map to an already
    /// emitted index are dropped; gaps report how many previous frames the
    /// encoder must duplicate.
    pub fn submit(&mut self, timestamp: Duration) -> FrameDecision {
        let start = *self.start.get_or_insert(timestamp);
        let elapsed = timestamp.saturating_sub(start);
        let index =
            (elapsed.as_nanos() * u128::from(self.frame_rate.as_u32()) / 1_000_000_000) as u64;

        if index < self.next_index {
            return FrameDecision::Drop;
        }

        let duplicates = index - self.next_index;
        self.next_index = index + 1;
        FrameDecision::Encode { index, duplicates }
    }

    pub fn reset(&mut self) {
        self.start = None;
        self.next_index = 0;
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
        let mut timeline = FrameTimeline::new(FrameRate::Fps30);
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
        let mut timeline = FrameTimeline::new(FrameRate::Fps30);
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
        let mut timeline = FrameTimeline::new(FrameRate::Fps30);
        timeline.submit(Duration::ZERO);
        timeline.submit(Duration::from_millis(100));
        assert_eq!(
            timeline.submit(Duration::from_millis(50)),
            FrameDecision::Drop
        );
    }

    #[test]
    fn gaps_report_duplicate_count() {
        let mut timeline = FrameTimeline::new(FrameRate::Fps30);
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
        let mut timeline = FrameTimeline::new(FrameRate::Fps30);
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

    #[test]
    fn sixty_fps_uses_sixteen_millisecond_steps() {
        let mut timeline = FrameTimeline::new(FrameRate::Fps60);
        timeline.submit(Duration::ZERO);
        assert_eq!(
            timeline.submit(Duration::from_millis(17)),
            FrameDecision::Encode {
                index: 1,
                duplicates: 0
            }
        );
    }

    #[test]
    fn intervals_match_the_target_rate() {
        assert_eq!(
            FrameRate::Fps30.interval(),
            Duration::from_nanos(33_333_333)
        );
        assert_eq!(
            FrameRate::Fps60.interval(),
            Duration::from_nanos(16_666_666)
        );
    }
}
