use core_types::PhysicalRect;

use super::RegionHandle;

#[derive(Clone, Copy)]
pub(crate) struct RegionAdjustDraft {
    pub(crate) handle: RegionHandle,
    pub(crate) original: PhysicalRect,
    pub(crate) current: PhysicalRect,
}

#[derive(Default)]
pub(crate) struct ToolbarState {
    pub(crate) more_open: bool,
    pub(crate) effects_open: bool,
    pub(crate) hints_visible: bool,
    pub(crate) hidden: bool,
    pub(crate) custom_color_open: bool,
    pub(crate) custom_color_buffer: String,
    pub(crate) custom_color_fresh: bool,
}

impl ToolbarState {
    pub(crate) fn close_panels(&mut self) -> bool {
        let open = self.more_open || self.effects_open || self.hints_visible;
        self.more_open = false;
        self.effects_open = false;
        self.hints_visible = false;
        open
    }
}

#[derive(Default)]
pub(crate) struct RegionAdjuster {
    pub(crate) draft: Option<RegionAdjustDraft>,
}

impl RegionAdjuster {
    pub(crate) fn begin(&mut self, handle: RegionHandle, rect: PhysicalRect) {
        self.draft = Some(RegionAdjustDraft {
            handle,
            original: rect,
            current: rect,
        });
    }

    pub(crate) fn take(&mut self) -> Option<RegionAdjustDraft> {
        self.draft.take()
    }

    pub(crate) fn cancel(&mut self) -> bool {
        self.draft.take().is_some()
    }

    pub(crate) fn update(&mut self, rect: PhysicalRect) {
        if let Some(draft) = &mut self.draft {
            draft.current = rect;
        }
    }
}
