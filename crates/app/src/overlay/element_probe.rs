use std::collections::{HashMap, HashSet};

use core_types::{PhysicalPoint, PhysicalRect, WindowId};

use super::element_scroll::ElementScroll;

pub(super) struct ElementChain {
    pub(super) window: WindowId,
    pub(super) point: PhysicalPoint,
    pub(super) elements: Vec<PhysicalRect>,
}

#[derive(Default)]
pub(super) struct ElementProbe {
    pub(super) window: Option<WindowId>,
    pub(super) pending_chain: Option<(WindowId, PhysicalPoint)>,
    pub(super) pending_full: Option<WindowId>,
    pub(super) cache: HashMap<WindowId, Vec<PhysicalRect>>,
    pub(super) chain: Option<ElementChain>,
    pub(super) retry: HashSet<WindowId>,
    pub(super) candidates: Vec<PhysicalRect>,
    pub(super) level: usize,
    pub(super) scroll: ElementScroll,
}

impl ElementProbe {
    pub(super) fn clear_hover(&mut self) {
        self.window = None;
        self.chain = None;
        self.candidates.clear();
        self.level = 0;
        self.scroll.reset();
    }

    pub(super) fn reset(&mut self) {
        self.clear_hover();
        self.cache.clear();
        self.retry.clear();
        self.pending_chain = None;
        self.pending_full = None;
    }

    pub(super) fn switch_window(&mut self, window: WindowId) {
        self.window = Some(window);
        self.chain = None;
        self.candidates.clear();
        self.level = 0;
        self.scroll.reset();
    }

    pub(super) fn retain_cache_for(&mut self, window: WindowId) {
        self.cache.retain(|id, _| *id == window);
        self.retry.retain(|id| *id == window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: u64) -> WindowId {
        WindowId::from_raw(id)
    }

    #[test]
    fn switching_windows_keeps_only_the_new_cache_and_clears_the_chain() {
        let mut probe = ElementProbe::default();
        let first = window(1);
        let second = window(2);
        probe.cache.insert(first, Vec::new());
        probe.cache.insert(second, Vec::new());
        probe.retry.insert(first);
        probe.retry.insert(second);
        probe.chain = Some(ElementChain {
            window: first,
            point: PhysicalPoint::new(10, 20),
            elements: Vec::new(),
        });
        probe.retain_cache_for(second);
        probe.switch_window(second);
        assert!(probe.cache.contains_key(&second));
        assert!(!probe.cache.contains_key(&first));
        assert!(probe.retry.contains(&second));
        assert!(!probe.retry.contains(&first));
        assert!(probe.chain.is_none());
        assert_eq!(probe.window, Some(second));
    }

    #[test]
    fn reset_drops_cache_chain_and_pending_queries() {
        let mut probe = ElementProbe::default();
        probe.cache.insert(window(1), Vec::new());
        probe.chain = Some(ElementChain {
            window: window(1),
            point: PhysicalPoint::new(0, 0),
            elements: Vec::new(),
        });
        probe.pending_chain = Some((window(1), PhysicalPoint::new(0, 0)));
        probe.pending_full = Some(window(1));
        probe.reset();
        assert!(probe.cache.is_empty());
        assert!(probe.retry.is_empty());
        assert!(probe.chain.is_none());
        assert!(probe.pending_chain.is_none());
        assert!(probe.pending_full.is_none());
        assert!(probe.window.is_none());
    }
}
