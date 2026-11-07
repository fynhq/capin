use crate::MonitorId;

/// A point in physical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PhysicalPoint {
    pub x: i32,
    pub y: i32,
}

impl PhysicalPoint {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// A width and height in physical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PhysicalSize {
    pub width: u32,
    pub height: u32,
}

impl PhysicalSize {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// An axis-aligned rectangle in physical pixels.
///
/// Monitor and window bounds use the virtual desktop coordinate space, whose
/// origin can be negative when a monitor sits left of or above the primary one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PhysicalRect {
    pub origin: PhysicalPoint,
    pub size: PhysicalSize,
}

impl PhysicalRect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            origin: PhysicalPoint::new(x, y),
            size: PhysicalSize::new(width, height),
        }
    }

    pub const fn from_parts(origin: PhysicalPoint, size: PhysicalSize) -> Self {
        Self { origin, size }
    }

    pub const fn is_empty(self) -> bool {
        self.size.is_empty()
    }

    /// Exclusive right edge. Widened to `i64` so extreme origins cannot overflow.
    pub const fn right(self) -> i64 {
        self.origin.x as i64 + self.size.width as i64
    }

    /// Exclusive bottom edge. Widened to `i64` so extreme origins cannot overflow.
    pub const fn bottom(self) -> i64 {
        self.origin.y as i64 + self.size.height as i64
    }

    pub fn contains(self, point: PhysicalPoint) -> bool {
        let x = point.x as i64;
        let y = point.y as i64;
        x >= self.origin.x as i64
            && x < self.right()
            && y >= self.origin.y as i64
            && y < self.bottom()
    }

    pub fn intersects(self, other: Self) -> bool {
        (self.origin.x as i64) < other.right()
            && (other.origin.x as i64) < self.right()
            && (self.origin.y as i64) < other.bottom()
            && (other.origin.y as i64) < self.bottom()
    }

    /// Returns the overlapping area, or `None` when the rectangles only touch
    /// or do not overlap.
    pub fn intersection(self, other: Self) -> Option<Self> {
        if self.is_empty() || other.is_empty() {
            return None;
        }

        let left = self.origin.x.max(other.origin.x);
        let top = self.origin.y.max(other.origin.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());

        if right <= left as i64 || bottom <= top as i64 {
            return None;
        }

        Some(Self::new(
            left,
            top,
            (right - left as i64) as u32,
            (bottom - top as i64) as u32,
        ))
    }
}

/// A region selected for capture, relative to its monitor's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CaptureRegion {
    pub monitor: MonitorId,
    pub rect: PhysicalRect,
}

impl CaptureRegion {
    pub const fn new(monitor: MonitorId, rect: PhysicalRect) -> Self {
        Self { monitor, rect }
    }

    pub const fn is_empty(self) -> bool {
        self.rect.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_and_bottom_do_not_overflow() {
        let rect = PhysicalRect::new(i32::MAX, i32::MAX, 10, 10);
        assert_eq!(rect.right(), i32::MAX as i64 + 10);
        assert_eq!(rect.bottom(), i32::MAX as i64 + 10);
    }

    #[test]
    fn contains_uses_exclusive_edges() {
        let rect = PhysicalRect::new(-10, -10, 20, 20);
        assert!(rect.contains(PhysicalPoint::new(-10, -10)));
        assert!(rect.contains(PhysicalPoint::new(9, 9)));
        assert!(!rect.contains(PhysicalPoint::new(10, 0)));
        assert!(!rect.contains(PhysicalPoint::new(0, 10)));
    }

    #[test]
    fn intersection_handles_negative_origins() {
        let a = PhysicalRect::new(-1920, 0, 1920, 1080);
        let b = PhysicalRect::new(-960, 100, 1920, 1080);
        let overlap = a.intersection(b).unwrap();
        assert_eq!(overlap, PhysicalRect::new(-960, 100, 960, 980));
    }

    #[test]
    fn touching_rectangles_do_not_intersect() {
        let a = PhysicalRect::new(0, 0, 10, 10);
        let b = PhysicalRect::new(10, 0, 10, 10);
        assert!(!a.intersects(b));
        assert_eq!(a.intersection(b), None);
    }

    #[test]
    fn empty_rectangles_never_intersect() {
        let empty = PhysicalRect::new(0, 0, 0, 10);
        assert!(empty.is_empty());
        assert!(!empty.intersects(PhysicalRect::new(0, 0, 10, 10)));
    }

    #[test]
    fn capture_region_rejects_empty_rect() {
        let region = CaptureRegion::new(MonitorId::from_raw(1), PhysicalRect::new(0, 0, 0, 0));
        assert!(region.is_empty());
    }
}
