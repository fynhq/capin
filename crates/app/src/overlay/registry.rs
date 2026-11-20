use std::cell::{Cell, RefCell};

use gpui_kit::WeakEntity;

use super::OverlayView;

thread_local! {
    static VIEWS: RefCell<Vec<WeakEntity<OverlayView>>> = const { RefCell::new(Vec::new()) };
    static FOCUSED: Cell<Option<u64>> = const { Cell::new(None) };
}

pub(super) fn live() -> Vec<WeakEntity<OverlayView>> {
    VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        views.retain(|view| view.upgrade().is_some());
        let focused = FOCUSED.with(Cell::get);
        if focused.is_some_and(|focused| {
            !views
                .iter()
                .any(|view| view.entity_id().as_u64() == focused)
        }) {
            FOCUSED.with(|slot| slot.set(None));
        }
        views.clone()
    })
}

#[cfg_attr(not(any(target_os = "windows", target_os = "macos")), allow(dead_code))]
pub(super) fn focused() -> Vec<WeakEntity<OverlayView>> {
    let mut views = live();
    if let Some(focused) = FOCUSED.with(Cell::get) {
        views.sort_by_key(|view| view.entity_id().as_u64() != focused);
    }
    views
}

pub(super) fn has_live() -> bool {
    !live().is_empty()
}

pub(super) fn register(view: WeakEntity<OverlayView>) {
    VIEWS.with(|views| views.borrow_mut().push(view));
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(super) fn clear() {
    FOCUSED.with(|slot| slot.set(None));
    VIEWS.with(|views| views.borrow_mut().clear());
}

pub(super) fn set_focus(id: u64) {
    FOCUSED.with(|slot| slot.set(Some(id)));
}

pub(super) fn clear_focus_if(id: u64) {
    if FOCUSED.with(Cell::get) == Some(id) {
        FOCUSED.with(|slot| slot.set(None));
    }
}
