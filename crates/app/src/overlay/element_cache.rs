use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use core_types::{PhysicalRect, WindowId};

const FULL_CACHE_TTL: Duration = Duration::from_secs(10);
const FULL_CACHE_MAX_WINDOWS: usize = 8;
const LARGE_TREE_COOLDOWN: Duration = Duration::from_secs(60);
const LARGE_TREE_ELAPSED: Duration = Duration::from_millis(800);
const LARGE_TREE_ELEMENTS: usize = 4_096;

struct FullEntry {
    bounds: PhysicalRect,
    elements: Vec<PhysicalRect>,
    stored_at: Instant,
}

#[derive(Default)]
struct CacheState {
    full: HashMap<WindowId, FullEntry>,
    large: HashMap<WindowId, Instant>,
}

impl CacheState {
    fn prune(&mut self, now: Instant) {
        self.full
            .retain(|_, entry| now.duration_since(entry.stored_at) < FULL_CACHE_TTL);
        self.large
            .retain(|_, marked| now.duration_since(*marked) < LARGE_TREE_COOLDOWN);
    }

    fn lookup(
        &mut self,
        window: WindowId,
        bounds: PhysicalRect,
        now: Instant,
    ) -> Option<Vec<PhysicalRect>> {
        self.prune(now);
        let entry = self.full.get(&window)?;
        (entry.bounds == bounds).then(|| entry.elements.clone())
    }

    fn store(
        &mut self,
        window: WindowId,
        bounds: PhysicalRect,
        elements: &[PhysicalRect],
        elapsed: Duration,
        now: Instant,
    ) {
        if elements.is_empty() {
            return;
        }
        self.prune(now);
        if !self.full.contains_key(&window) && self.full.len() >= FULL_CACHE_MAX_WINDOWS {
            let oldest = self
                .full
                .iter()
                .min_by_key(|(_, entry)| entry.stored_at)
                .map(|(id, _)| *id);
            if let Some(oldest) = oldest {
                self.full.remove(&oldest);
            }
        }
        self.full.insert(
            window,
            FullEntry {
                bounds,
                elements: elements.to_vec(),
                stored_at: now,
            },
        );
        if elements.len() >= LARGE_TREE_ELEMENTS || elapsed >= LARGE_TREE_ELAPSED {
            self.large.insert(window, now);
        } else {
            self.large.remove(&window);
        }
    }

    fn is_large(&mut self, window: WindowId, now: Instant) -> bool {
        self.prune(now);
        self.large.contains_key(&window)
    }
}

static STATE: OnceLock<Mutex<CacheState>> = OnceLock::new();

fn state() -> &'static Mutex<CacheState> {
    STATE.get_or_init(|| Mutex::new(CacheState::default()))
}

pub(super) fn lookup(window: WindowId, bounds: PhysicalRect) -> Option<Vec<PhysicalRect>> {
    state().lock().ok()?.lookup(window, bounds, Instant::now())
}

pub(super) fn store(
    window: WindowId,
    bounds: PhysicalRect,
    elements: &[PhysicalRect],
    elapsed: Duration,
) {
    if let Ok(mut state) = state().lock() {
        state.store(window, bounds, elements, elapsed, Instant::now());
    }
}

pub(super) fn is_large(window: WindowId) -> bool {
    state()
        .lock()
        .is_ok_and(|mut state| state.is_large(window, Instant::now()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: u64) -> WindowId {
        WindowId::from_raw(id)
    }

    fn rect(x: i32, y: i32, width: u32, height: u32) -> PhysicalRect {
        PhysicalRect::new(x, y, width, height)
    }

    #[test]
    fn fresh_entry_requires_matching_bounds() {
        let mut state = CacheState::default();
        let now = Instant::now();
        let bounds = rect(0, 0, 800, 600);
        let elements = vec![rect(10, 10, 100, 20)];
        state.store(window(1), bounds, &elements, Duration::ZERO, now);
        assert_eq!(state.lookup(window(1), bounds, now), Some(elements));
        assert_eq!(state.lookup(window(1), rect(0, 0, 801, 600), now), None);
    }

    #[test]
    fn entries_expire_after_ttl() {
        let mut state = CacheState::default();
        let now = Instant::now();
        let bounds = rect(0, 0, 800, 600);
        state.store(
            window(1),
            bounds,
            &[rect(10, 10, 100, 20)],
            Duration::ZERO,
            now,
        );
        let later = now + FULL_CACHE_TTL + Duration::from_secs(1);
        assert_eq!(state.lookup(window(1), bounds, later), None);
        assert!(state.full.is_empty());
    }

    #[test]
    fn empty_results_are_not_cached() {
        let mut state = CacheState::default();
        state.store(
            window(1),
            rect(0, 0, 800, 600),
            &[],
            Duration::ZERO,
            Instant::now(),
        );
        assert!(state.full.is_empty());
    }

    #[test]
    fn oldest_entry_is_evicted_when_limit_is_reached() {
        let mut state = CacheState::default();
        let now = Instant::now();
        for id in 1..=FULL_CACHE_MAX_WINDOWS as u64 + 1 {
            let stored_at = now + Duration::from_secs(id);
            state.store(
                window(id),
                rect(0, 0, 800, 600),
                &[rect(0, 0, 10, 10)],
                Duration::ZERO,
                stored_at,
            );
        }
        let later = now + Duration::from_secs(FULL_CACHE_MAX_WINDOWS as u64);
        assert_eq!(state.full.len(), FULL_CACHE_MAX_WINDOWS);
        assert_eq!(state.lookup(window(1), rect(0, 0, 800, 600), later), None);
        assert!(
            state
                .lookup(window(2), rect(0, 0, 800, 600), later)
                .is_some()
        );
    }

    #[test]
    fn slow_full_queries_mark_large_windows() {
        let mut state = CacheState::default();
        let now = Instant::now();
        state.store(
            window(1),
            rect(0, 0, 800, 600),
            &[rect(0, 0, 10, 10)],
            LARGE_TREE_ELAPSED,
            now,
        );
        assert!(state.is_large(window(1), now));
        assert!(!state.is_large(
            window(1),
            now + LARGE_TREE_COOLDOWN + Duration::from_secs(1)
        ));
    }

    #[test]
    fn element_count_marks_large_windows_and_small_results_clear_it() {
        let mut state = CacheState::default();
        let now = Instant::now();
        let elements = vec![rect(0, 0, 10, 10); LARGE_TREE_ELEMENTS];
        state.store(
            window(1),
            rect(0, 0, 800, 600),
            &elements,
            Duration::ZERO,
            now,
        );
        assert!(state.is_large(window(1), now));
        state.store(
            window(1),
            rect(0, 0, 800, 600),
            &[rect(0, 0, 10, 10)],
            Duration::ZERO,
            now,
        );
        assert!(!state.is_large(window(1), now));
    }
}
