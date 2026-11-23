use std::time::Duration;

use super::*;

#[derive(Default)]
pub(super) struct SettingsToast {
    message: Option<String>,
    serial: u64,
    leaving: bool,
}

impl SettingsToast {
    pub(super) fn from_message(message: Option<String>) -> Self {
        Self {
            message,
            serial: 0,
            leaving: false,
        }
    }

    pub(super) fn message(&self) -> Option<&String> {
        self.message.as_ref()
    }

    pub(super) fn serial(&self) -> u64 {
        self.serial
    }

    pub(super) fn leaving(&self) -> bool {
        self.leaving
    }

    pub(super) fn clear(&mut self) {
        self.message = None;
        self.leaving = false;
    }

    pub(super) fn show(&mut self, message: String, cx: &mut Context<SettingsView>) {
        self.message = Some(message);
        self.leaving = false;
        self.serial = self.serial.wrapping_add(1);
        let serial = self.serial;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(2600))
                .await;
            let alive = this
                .update(cx, |this, cx| {
                    if this.status.serial != serial {
                        return false;
                    }
                    this.status.leaving = true;
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !alive {
                return;
            }
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            this.update(cx, |this, cx| {
                if this.status.serial == serial {
                    this.status.message = None;
                    this.status.leaving = false;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}
