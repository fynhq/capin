/// Ratio between physical pixels and logical pixels for one monitor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DpiScale(f32);

impl DpiScale {
    pub const ONE: Self = Self(1.0);

    /// Returns `None` for zero, negative, NaN, or infinite scales.
    pub fn new(scale: f32) -> Option<Self> {
        (scale.is_finite() && scale > 0.0).then_some(Self(scale))
    }

    pub const fn get(self) -> f32 {
        self.0
    }

    pub fn physical_from_logical(self, logical: f32) -> f32 {
        logical * self.0
    }

    pub fn logical_from_physical(self, physical: f32) -> f32 {
        physical / self.0
    }
}

impl Default for DpiScale {
    fn default() -> Self {
        Self::ONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_between_logical_and_physical_pixels() {
        let scale = DpiScale::new(1.25).unwrap();
        assert_eq!(scale.physical_from_logical(100.0), 125.0);
        assert_eq!(scale.logical_from_physical(125.0), 100.0);
    }

    #[test]
    fn rejects_invalid_scales() {
        assert_eq!(DpiScale::new(0.0), None);
        assert_eq!(DpiScale::new(-1.0), None);
        assert_eq!(DpiScale::new(f32::NAN), None);
        assert_eq!(DpiScale::new(f32::INFINITY), None);
    }

    #[test]
    fn default_is_one() {
        assert_eq!(DpiScale::default(), DpiScale::ONE);
    }
}
