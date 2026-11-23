use std::collections::{HashMap, HashSet};

use core_types::{PhysicalPoint, PhysicalRect, WindowId};

use super::element_scroll::ElementScroll;

#[derive(Default)]
pub(super) struct ElementProbe {
    pub(super) window: Option<WindowId>,
    pub(super) pending: Option<WindowId>,
    pub(super) pending_point: Option<PhysicalPoint>,
    pub(super) cache: HashMap<WindowId, Vec<PhysicalRect>>,
    pub(super) cache_point: Option<PhysicalPoint>,
    pub(super) retry: HashSet<WindowId>,
    pub(super) candidates: Vec<PhysicalRect>,
    pub(super) level: usize,
    pub(super) scroll: ElementScroll,
}

impl ElementProbe {
    pub(super) fn clear_hover(&mut self) {
        self.window = None;
        self.pending = None;
        self.pending_point = None;
        self.candidates.clear();
        self.level = 0;
        self.scroll.reset();
    }

    pub(super) fn reset(&mut self) {
        self.clear_hover();
        self.cache.clear();
        self.cache_point = None;
        self.retry.clear();
    }
}
