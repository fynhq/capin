use std::path::PathBuf;
use std::time::{Duration, Instant};

use core_capture::{CaptureTarget, MonitorInfo};
use core_record::{CursorHighlight, FRAME_RATE, RecordConfig};
use core_types::{PhysicalPoint, PhysicalRect};
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_capture::platform_backend;
use platform_record::Recorder;
use platform_window::system_accent_hex;

use crate::chrome_theme::ChromeTheme;
use crate::i18n::{Language, t};
use crate::logging::{log_error, log_info};
use crate::settings_state;

const BAR_WIDTH: f32 = 520.0;
const BAR_HEIGHT: f32 = 64.0;
const COUNTDOWN: Duration = Duration::from_secs(3);
const COMPLETE_LINGER: Duration = Duration::from_millis(3000);

thread_local! {
    static RECORD_BAR: std::cell::RefCell<Option<WeakEntity<RecordBar>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecordShortcut {
    Pause,
    Stop,
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) fn handle_shortcut(shortcut: RecordShortcut, cx: &mut App) {
    let view = RECORD_BAR.with(|slot| slot.borrow().as_ref().and_then(|weak| weak.upgrade()));
    if let Some(view) = view {
        view.update(cx, |bar, cx| match shortcut {
            RecordShortcut::Pause => bar.toggle_pause(cx),
            RecordShortcut::Stop => bar.stop(cx),
        });
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) fn shutdown(cx: &mut App, timeout: Duration) -> bool {
    let view = RECORD_BAR.with(|slot| slot.borrow().as_ref().and_then(|weak| weak.upgrade()));
    if let Some(view) = view {
        return view.update(cx, |bar, _| bar.shutdown(timeout));
    }
    platform_record::stop_active();
    platform_record::wait_active(timeout)
}

pub fn start(target: CaptureTarget, cx: &mut App) {
    log_info!("record", "recording requested for {:?}", target);
    let settings = settings_state::get();
    let (width, height) = platform_record::target_dimensions(target)
        .map(|size| (size.width, size.height))
        .unwrap_or((0, 0));
    let output = match crate::storage::recording_path(width, height) {
        Ok(path) => path,
        Err(error) => {
            log_error!("record", "failed to prepare the recording output: {error}");
            return;
        }
    };
    let cleanup_path = output.clone();
    let config = RecordConfig {
        target,
        include_cursor: true,
        cursor_highlight: settings.record_cursor_highlight.then_some(CursorHighlight {
            color: settings.record_cursor_highlight_color & 0x00FF_FFFF,
        }),
    };

    crate::region_outline::open(target, cx);

    let shortcuts_ok = crate::hotkey::set_recording_shortcuts(true);
    let monitor_id = target_monitor(target);
    let monitor = monitor_id.and_then(|id| {
        platform_backend()
            .monitors()
            .ok()?
            .into_iter()
            .find(|monitor| monitor.id == id)
    });
    let desktop_bounds = bar_bounds(target);
    let bar_overlaps = bar_overlaps_target(target, desktop_bounds);
    let bounds = monitor.as_ref().map_or(desktop_bounds, |monitor| {
        crate::display::window_bounds_on_monitor(monitor, desktop_bounds)
    });
    let display_id = monitor_id.map(|id| DisplayId::new(id.raw()));

    match crate::control_bar::open(
        "capin-record",
        bounds,
        display_id,
        cx,
        move |window, cx, exclusion_ok| {
            let excluded: Vec<u64> = platform_window::native_window_id(&*window)
                .into_iter()
                .collect();
            let window_handle = window.window_handle();
            cx.new(|cx| {
                RecordBar::new(
                    output,
                    config,
                    excluded,
                    exclusion_ok,
                    bar_overlaps,
                    shortcuts_ok,
                    window_handle,
                    cx,
                )
            })
        },
    ) {
        Ok(entity) => {
            RECORD_BAR.with(|slot| *slot.borrow_mut() = Some(entity.downgrade()));
        }
        Err(error) => {
            let _ = std::fs::remove_file(&cleanup_path);
            crate::hotkey::set_recording_shortcuts(false);
            crate::region_outline::close(cx);
            log_error!("record", "failed to open recording bar: {error}");
        }
    }
}

struct RecordBar {
    output: PathBuf,
    window_handle: AnyWindowHandle,
    recorder: Option<Recorder>,
    config: RecordConfig,
    excluded: Vec<u64>,
    countdown_started: Instant,
    stopping: bool,
    completed: bool,
    blocked: bool,
    status: Option<String>,
    language: Language,
    accent_hex: u32,
    shortcuts_ok: bool,
    auto_close_scheduled: bool,
    closing: bool,
}

impl RecordBar {
    #[allow(clippy::too_many_arguments)]
    fn new(
        output: PathBuf,
        config: RecordConfig,
        excluded: Vec<u64>,
        exclusion_ok: bool,
        bar_overlaps: bool,
        shortcuts_ok: bool,
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = settings_state::get();
        let language = settings.language;
        let accent_hex =
            system_accent_hex().unwrap_or_else(|| crate::theme::fallback_accent(false));
        let blocked = !exclusion_ok && bar_overlaps;
        let mut status = None;
        if blocked {
            status = Some(t(language, "record.exclusion_blocked").to_owned());
        } else if !exclusion_ok {
            status = Some(t(language, "record.exclusion_failed").to_owned());
        }

        let bar = Self {
            output,
            window_handle,
            recorder: None,
            config,
            excluded,
            countdown_started: Instant::now(),
            stopping: false,
            completed: false,
            blocked,
            status,
            language,
            accent_hex,
            shortcuts_ok,
            auto_close_scheduled: false,
            closing: false,
        };
        cx.on_release(|_, cx| {
            crate::hotkey::set_recording_shortcuts(false);
            RECORD_BAR.with(|slot| *slot.borrow_mut() = None);
            crate::record_spotlight::close(cx);
            crate::region_outline::close(cx);
        })
        .detach();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                let alive = this
                    .update(cx, |this, cx| {
                        if this.completed {
                            return false;
                        }
                        if this.blocked {
                            return false;
                        }
                        if this.recorder.is_none() && this.countdown_started.elapsed() < COUNTDOWN {
                            cx.notify();
                            return true;
                        }
                        if this.recorder.is_none() && this.countdown_started.elapsed() >= COUNTDOWN
                        {
                            this.start_recording(cx);
                        }
                        if this
                            .recorder
                            .as_ref()
                            .is_some_and(|recorder| recorder.is_finished())
                        {
                            this.completed = true;
                            this.stopping = false;
                            this.apply_final_status();
                            if this.output_saved() {
                                this.schedule_auto_close(cx);
                            }
                            cx.notify();
                            return false;
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        })
        .detach();
        bar
    }

    fn start_recording(&mut self, cx: &mut Context<Self>) {
        match Recorder::start_excluding(self.config.clone(), self.output.clone(), &self.excluded) {
            Ok(recorder) => {
                self.recorder = Some(recorder);
                self.status = None;
                if let Some(highlight) = self.config.cursor_highlight {
                    crate::record_spotlight::open(self.config.target, highlight.color, cx);
                }
            }
            Err(error) => {
                let _ = std::fs::remove_file(&self.output);
                self.completed = true;
                self.status = Some(format!("{}{}", t(self.language, "record.failed"), error));
            }
        }
        cx.notify();
    }

    fn countdown_remaining(&self) -> u64 {
        COUNTDOWN
            .as_secs()
            .saturating_sub(self.countdown_started.elapsed().as_secs())
            .max(1)
    }

    fn apply_final_status(&mut self) {
        let Some(recorder) = self.recorder.as_ref() else {
            return;
        };
        self.status = Some(match recorder.error() {
            Some(error) => {
                let _ = std::fs::remove_file(&self.output);
                format!("{}{}", t(self.language, "record.failed"), error)
            }
            None => {
                if let Some(warning) = recorder.warning() {
                    crate::logging::log_warn!("record", "{warning}");
                }
                format!(
                    "{} {}",
                    t(self.language, "record.saved"),
                    self.output.display()
                )
            }
        });
    }

    fn output_saved(&self) -> bool {
        self.recorder
            .as_ref()
            .is_some_and(|recorder| recorder.error().is_none())
    }

    fn schedule_auto_close(&mut self, cx: &mut Context<Self>) {
        if self.auto_close_scheduled {
            return;
        }
        self.auto_close_scheduled = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COMPLETE_LINGER).await;
            let alive = this
                .update(cx, |this, cx| {
                    if this.closing {
                        return false;
                    }
                    this.closing = true;
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
            this.update(cx, |this, cx| this.close(cx)).ok();
        })
        .detach();
    }

    fn open_folder(&mut self, cx: &mut Context<Self>) {
        if let Some(directory) = self.output.parent() {
            cx.open_with_system(directory);
        }
        self.close(cx);
    }

    fn elapsed(&self) -> Duration {
        self.recorder
            .as_ref()
            .map_or(Duration::ZERO, Recorder::elapsed_active)
    }

    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        if self.stopping || self.completed {
            return;
        }
        let Some(recorder) = self.recorder.as_ref() else {
            return;
        };
        recorder.set_paused(!recorder.is_paused());
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if self.stopping || self.completed {
            return;
        }
        let Some(recorder) = self.recorder.as_ref() else {
            let _ = std::fs::remove_file(&self.output);
            self.completed = true;
            self.status = Some(t(self.language, "record.cancelled").to_owned());
            cx.notify();
            return;
        };
        self.stopping = true;
        recorder.stop();
        crate::record_spotlight::close(cx);
        self.status = Some(t(self.language, "record.saving").to_owned());
        cx.notify();
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn shutdown(&mut self, timeout: Duration) -> bool {
        if self.completed {
            return true;
        }
        let Some(recorder) = self.recorder.as_mut() else {
            let _ = std::fs::remove_file(&self.output);
            self.completed = true;
            return true;
        };
        self.stopping = true;
        recorder.stop_and_wait(timeout)
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if !self.output_saved() {
            let _ = std::fs::remove_file(&self.output);
        }
        crate::hotkey::set_recording_shortcuts(false);
        RECORD_BAR.with(|slot| *slot.borrow_mut() = None);
        crate::record_spotlight::close(cx);
        crate::region_outline::close(cx);
        self.closing = true;
        cx.notify();
        let handle = self.window_handle;
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        });
        crate::windows::schedule_memory_trim();
    }
}

impl Render for RecordBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ChromeTheme::for_window(window, Some(self.accent_hex));
        let language = self.language;
        let elapsed = self.elapsed();
        let paused = self.recorder.as_ref().is_some_and(Recorder::is_paused);
        let counting_down = !self.blocked && self.recorder.is_none() && !self.completed;
        let timer = if self.blocked {
            "--:--".to_owned()
        } else if counting_down {
            format!("00:{:02}", self.countdown_remaining())
        } else {
            format!(
                "{:02}:{:02}",
                elapsed.as_secs() / 60,
                elapsed.as_secs() % 60
            )
        };
        let state = if self.blocked {
            t(language, "record.exclusion_blocked_state")
        } else if self.completed {
            t(language, "record.done")
        } else if self.stopping {
            t(language, "record.saving")
        } else if paused {
            t(language, "record.paused")
        } else if counting_down {
            t(language, "record.countdown")
        } else {
            t(language, "record.recording")
        };
        let status = self
            .status
            .clone()
            .unwrap_or_else(|| self.output.display().to_string());
        let mut pause_label = if paused {
            t(language, "record.resume").to_owned()
        } else {
            t(language, "record.pause").to_owned()
        };
        if self.shortcuts_ok {
            pause_label = format!("{pause_label} · F9");
        }
        let pause = (!self.blocked && self.recorder.is_some() && !self.stopping && !self.completed)
            .then(|| {
                crate::ui::bar_button(
                    "record-pause",
                    pause_label,
                    None,
                    crate::ui::BarButtonTone::Neutral,
                    &theme,
                    cx,
                    Self::toggle_pause,
                )
            });
        let mut stop_label = t(language, "record.stop").to_owned();
        if self.shortcuts_ok {
            stop_label = format!("{stop_label} · F10");
        }
        let stop = (!self.blocked && !self.stopping && !self.completed).then(|| {
            crate::ui::bar_button(
                "record-stop",
                stop_label,
                None,
                crate::ui::BarButtonTone::Danger,
                &theme,
                cx,
                Self::stop,
            )
        });
        let open_folder = (self.completed && self.output_saved()).then(|| {
            crate::ui::bar_button(
                "record-open-folder",
                t(language, "record.open_folder").to_owned(),
                Some(crate::icons::FOLDER),
                crate::ui::BarButtonTone::Neutral,
                &theme,
                cx,
                Self::open_folder,
            )
        });
        let close = (self.completed || self.blocked).then(|| {
            crate::ui::bar_button(
                "record-close",
                t(language, "record.close").to_owned(),
                None,
                crate::ui::BarButtonTone::Neutral,
                &theme,
                cx,
                Self::close,
            )
        });
        let recording = self.recorder.is_some() && !self.stopping && !self.completed;
        let dot = if paused || self.blocked {
            theme.muted
        } else if self.completed {
            theme.accent
        } else {
            rgb(0xFF453A)
        };
        let dot_core = div()
            .w(px(if recording { 9.0 } else { 7.0 }))
            .h(px(if recording { 9.0 } else { 7.0 }))
            .rounded(px(5.0))
            .bg(dot);
        let dot = if recording {
            dot_core
                .with_animation(
                    "record-dot-pulse",
                    Animation::new(Duration::from_millis(1600))
                        .repeat_synced()
                        .with_max_fps(30.0)
                        .with_easing(|delta| 0.7 + 0.3 * (delta * std::f32::consts::TAU).cos()),
                    |dot, delta| dot.opacity(delta),
                )
                .into_any_element()
        } else {
            dot_core.into_any_element()
        };
        let timer_text = div()
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.accent)
            .child(timer);
        let timer_text = if counting_down {
            crate::ui::enter_fade(
                timer_text,
                SharedString::from(format!("record-countdown-{}", self.countdown_remaining())),
                Duration::from_millis(180),
            )
            .into_any_element()
        } else {
            timer_text.into_any_element()
        };
        let progress = counting_down.then(|| {
            div()
                .absolute()
                .left(px(12.0))
                .right(px(12.0))
                .bottom(px(2.0))
                .h(px(2.0))
                .rounded(px(1.0))
                .bg(theme.selected)
                .child(
                    div()
                        .h_full()
                        .rounded(px(1.0))
                        .bg(theme.accent)
                        .with_animation(
                            "record-countdown-progress",
                            Animation::new(COUNTDOWN).with_easing(|delta| delta),
                            |line, delta| line.w(relative(1.0 - delta)),
                        ),
                )
        });

        let title = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(7.0))
            .child(crate::control_bar::bar_title(&theme, state.to_owned()))
            .child(timer_text)
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(theme.muted)
                    .child(format!("MP4 · {} fps", FRAME_RATE)),
            );
        let badge = crate::control_bar::bar_badge(
            if recording {
                rgba(0xFF453A33)
            } else {
                theme.selected
            },
            15.0,
        )
        .child(dot);
        let buttons: Vec<AnyElement> = [pause, stop, open_folder, close]
            .into_iter()
            .flatten()
            .collect();
        let bar =
            crate::control_bar::bar_shell(&theme, badge, title, status, buttons).children(progress);

        if self.closing && !cx.reduce_motion() {
            bar.with_animation(
                "record-bar-exit",
                Animation::new(Duration::from_millis(150))
                    .with_easing(|delta| 1.0 - (1.0 - delta).powi(3)),
                |bar, delta| bar.opacity(1.0 - delta),
            )
            .into_any_element()
        } else {
            bar.into_any_element()
        }
    }
}

fn target_monitor(target: CaptureTarget) -> Option<core_types::MonitorId> {
    let monitors = platform_backend().monitors().ok()?;
    match target {
        CaptureTarget::Region(region) => Some(region.monitor),
        CaptureTarget::Monitor(id) => Some(id),
        CaptureTarget::Window(id) => {
            let windows = platform_backend().windows().ok()?;
            let window = windows.into_iter().find(|window| window.id == id)?;
            window
                .monitor
                .or_else(|| monitor_for_rect(&monitors, window.bounds))
        }
    }
}

fn bar_overlaps_target(target: CaptureTarget, bar: Bounds<Pixels>) -> bool {
    let Ok(monitors) = platform_backend().monitors() else {
        return true;
    };
    let Some(monitor_id) = target_monitor(target) else {
        return true;
    };
    let Some(monitor) = monitors.iter().find(|monitor| monitor.id == monitor_id) else {
        return true;
    };
    let target_rect = match target {
        CaptureTarget::Monitor(_) => monitor.bounds,
        CaptureTarget::Region(region) => PhysicalRect::new(
            monitor.bounds.origin.x + region.rect.origin.x,
            monitor.bounds.origin.y + region.rect.origin.y,
            region.rect.size.width,
            region.rect.size.height,
        ),
        CaptureTarget::Window(id) => {
            let Ok(windows) = platform_backend().windows() else {
                return true;
            };
            let Some(window) = windows.into_iter().find(|window| window.id == id) else {
                return true;
            };
            window.bounds
        }
    };
    let scale = monitor.scale.get().max(0.1);
    let bar_left = f32::from(bar.origin.x) * scale;
    let bar_top = f32::from(bar.origin.y) * scale;
    let bar_right = bar_left + f32::from(bar.size.width) * scale;
    let bar_bottom = bar_top + f32::from(bar.size.height) * scale;
    let target_left = target_rect.origin.x as f32;
    let target_top = target_rect.origin.y as f32;
    let target_right = target_left + target_rect.size.width as f32;
    let target_bottom = target_top + target_rect.size.height as f32;
    bar_right > target_left
        && bar_left < target_right
        && bar_bottom > target_top
        && bar_top < target_bottom
}

fn monitor_for_rect(
    monitors: &[MonitorInfo],
    bounds: PhysicalRect,
) -> Option<core_types::MonitorId> {
    let center = PhysicalPoint::new(
        bounds.origin.x + bounds.size.width as i32 / 2,
        bounds.origin.y + bounds.size.height as i32 / 2,
    );
    monitors
        .iter()
        .find(|monitor| monitor.bounds.contains(center))
        .or_else(|| monitors.first())
        .map(|monitor| monitor.id)
}

fn bar_bounds(target: CaptureTarget) -> Bounds<Pixels> {
    let monitors = platform_backend().monitors().unwrap_or_default();
    let monitor = target_monitor(target)
        .and_then(|id| monitors.iter().find(|monitor| monitor.id == id))
        .cloned()
        .or_else(|| monitors.first().cloned());

    let anchor = match target {
        CaptureTarget::Region(region) => {
            let monitor_origin = monitors
                .iter()
                .find(|monitor| monitor.id == region.monitor)
                .map(|monitor| monitor.bounds.origin)
                .unwrap_or_default();
            Some(PhysicalRect::new(
                monitor_origin.x + region.rect.origin.x,
                monitor_origin.y + region.rect.origin.y,
                region.rect.size.width,
                region.rect.size.height,
            ))
        }
        CaptureTarget::Monitor(_) => monitor.as_ref().map(|monitor| monitor.bounds),
        CaptureTarget::Window(id) => platform_backend()
            .windows()
            .ok()
            .and_then(|windows| windows.into_iter().find(|window| window.id == id))
            .map(|window| window.bounds),
    };

    crate::control_bar::bounds_for_anchor(anchor, monitor.as_ref(), BAR_WIDTH, BAR_HEIGHT)
}
