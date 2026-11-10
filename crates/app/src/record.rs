use std::path::PathBuf;
use std::time::{Duration, Instant};

use core_capture::{CaptureTarget, MonitorInfo, platform_backend};
use core_record::{CursorHighlight, FRAME_RATE, RecordConfig};
use core_types::{PhysicalPoint, PhysicalRect};
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_record::Recorder;
use platform_window::{WindowStyle, apply as apply_window_style, system_accent_hex};

use crate::chrome_theme::ChromeTheme;
use crate::i18n::{Language, t};
use crate::logging::{log_error, log_info};
use crate::settings_state;

const BAR_WIDTH: f32 = 520.0;
const BAR_HEIGHT: f32 = 64.0;
const COUNTDOWN: Duration = Duration::from_secs(3);

pub fn start(target: CaptureTarget, cx: &mut App) {
    log_info!("record", "recording requested for {:?}", target);
    let settings = settings_state::get();
    let output = match crate::storage::recording_path() {
        Ok(path) => path,
        Err(error) => {
            log_error!("record", "failed to prepare the recording output: {error}");
            return;
        }
    };
    let config = RecordConfig {
        target,
        // The highlight is only meaningful with the cursor in frame, so it
        // implies cursor capture even when "include cursor" is off.
        include_cursor: settings.record_cursor || settings.record_cursor_highlight,
        cursor_highlight: settings.record_cursor_highlight.then_some(CursorHighlight {
            color: settings.record_cursor_highlight_color & 0x00FF_FFFF,
        }),
    };

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
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: false,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("capin-record".to_owned()),
        ..Default::default()
    };

    match gpui_kit::open_window(options, cx, move |window, cx| {
        let exclusion_ok = match apply_window_style(&*window, WindowStyle::CAPTURE_BAR) {
            Ok(()) => true,
            Err(error) => {
                log_error!("record", "failed to style recording bar: {error}");
                false
            }
        };
        // The bar must exist before the macOS content filter is built so it
        // can be excluded from the stream. Windows already applies
        // WDA_EXCLUDEFROMCAPTURE above.
        let excluded: Vec<u64> = platform_window::native_window_id(&*window)
            .into_iter()
            .collect();
        cx.new(|cx| RecordBar::new(output, config, excluded, exclusion_ok, bar_overlaps, cx))
    }) {
        Ok(_) => {}
        Err(error) => log_error!("record", "failed to open recording bar: {error}"),
    }
}

struct RecordBar {
    output: PathBuf,
    recorder: Option<Recorder>,
    config: RecordConfig,
    excluded: Vec<u64>,
    started: Option<Instant>,
    countdown_started: Instant,
    paused_total: Duration,
    pause_started: Option<Instant>,
    paused: bool,
    stopping: bool,
    completed: bool,
    blocked: bool,
    status: Option<String>,
    language: Language,
}

impl RecordBar {
    fn new(
        output: PathBuf,
        config: RecordConfig,
        excluded: Vec<u64>,
        exclusion_ok: bool,
        bar_overlaps: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = settings_state::get();
        let language = settings.language;
        let blocked = !exclusion_ok && bar_overlaps;
        let mut status = None;
        if blocked {
            status = Some(t(language, "record.exclusion_blocked").to_owned());
        } else if !exclusion_ok {
            status = Some(t(language, "record.exclusion_failed").to_owned());
        }

        let bar = Self {
            output,
            recorder: None,
            config,
            excluded,
            started: None,
            countdown_started: Instant::now(),
            paused_total: Duration::ZERO,
            pause_started: None,
            paused: false,
            stopping: false,
            completed: false,
            blocked,
            status,
            language,
        };
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
                            cx.notify();
                            return true;
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
                self.started.get_or_insert_with(Instant::now);
                self.status = None;
            }
            Err(error) => {
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
            Some(error) => format!("{}{}", t(self.language, "record.failed"), error),
            None => format!(
                "{} {}",
                t(self.language, "record.saved"),
                self.output.display()
            ),
        });
    }

    fn elapsed(&self) -> Duration {
        let Some(started) = self.started else {
            return Duration::ZERO;
        };
        let mut elapsed = started.elapsed().saturating_sub(self.paused_total);
        if let Some(pause_started) = self.pause_started {
            elapsed = elapsed.saturating_sub(pause_started.elapsed());
        }
        elapsed
    }

    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        if self.stopping || self.completed {
            return;
        }
        let Some(recorder) = self.recorder.as_ref() else {
            return;
        };
        if self.paused {
            self.paused = false;
            recorder.set_paused(false);
            if let Some(started) = self.pause_started.take() {
                self.paused_total += started.elapsed();
            }
        } else {
            self.paused = true;
            self.pause_started = Some(Instant::now());
            recorder.set_paused(true);
        }
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if self.stopping || self.completed {
            return;
        }
        let Some(recorder) = self.recorder.as_ref() else {
            self.completed = true;
            self.status = Some(t(self.language, "record.cancelled").to_owned());
            cx.notify();
            return;
        };
        self.stopping = true;
        recorder.stop();
        self.status = Some(t(self.language, "record.saving").to_owned());
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        let handles = cx.windows();
        cx.defer(move |cx| {
            crate::windows::close_capture_auxiliary(cx, handles);
        });
    }
}

impl Render for RecordBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ChromeTheme::for_window(window, system_accent_hex());
        let language = self.language;
        let elapsed = self.elapsed();
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
        } else if self.paused {
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
        let pause_label = if self.paused {
            t(language, "record.resume").to_owned()
        } else {
            t(language, "record.pause").to_owned()
        };
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
        let stop = (!self.blocked && !self.stopping && !self.completed).then(|| {
            crate::ui::bar_button(
                "record-stop",
                t(language, "record.stop").to_owned(),
                None,
                crate::ui::BarButtonTone::Danger,
                &theme,
                cx,
                Self::stop,
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
        let dot = if self.paused || self.blocked {
            theme.muted
        } else if self.completed {
            theme.accent
        } else {
            rgb(0xFF453A)
        };

        div().size_full().flex().items_center().child(
            div()
                .size_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .px(px(12.0))
                .rounded(px(10.0))
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .shadow(theme.shadow())
                .text_color(theme.text)
                .child(
                    div()
                        .w(px(30.0))
                        .h(px(30.0))
                        .flex_shrink_0()
                        .rounded(px(15.0))
                        .bg(if recording {
                            rgba(0xFF453A33)
                        } else {
                            theme.selected
                        })
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .w(px(if recording { 9.0 } else { 7.0 }))
                                .h(px(if recording { 9.0 } else { 7.0 }))
                                .rounded(px(5.0))
                                .bg(dot),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(1.0))
                        .flex_1()
                        .min_w(px(0.0))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(7.0))
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(state),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(theme.accent)
                                        .child(timer),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.5))
                                        .text_color(theme.muted)
                                        .child(format!("MP4 · {} fps", FRAME_RATE)),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(theme.muted)
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(status),
                        ),
                )
                .children(pause)
                .children(stop)
                .children(close),
        )
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

/// Whether the capture bar sits inside the recorded rectangle. If capture
/// exclusion fails and this returns true, recording must not start: otherwise
/// the bar itself would be encoded into the video.
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

    let scale = monitor
        .as_ref()
        .map(|monitor| monitor.scale.get())
        .unwrap_or(1.0);
    let (monitor_left, monitor_top, monitor_right, monitor_bottom) = monitor
        .as_ref()
        .map(|monitor| {
            let scale = monitor.scale.get();
            (
                monitor.bounds.origin.x as f32 / scale,
                monitor.bounds.origin.y as f32 / scale,
                (monitor.bounds.origin.x + monitor.bounds.size.width as i32) as f32 / scale,
                (monitor.bounds.origin.y + monitor.bounds.size.height as i32) as f32 / scale,
            )
        })
        .unwrap_or((0.0, 0.0, 1920.0, 1080.0));

    let (anchor_left, anchor_top, anchor_width) = anchor
        .map(|rect| {
            (
                rect.origin.x as f32 / scale,
                rect.origin.y as f32 / scale,
                rect.size.width as f32 / scale,
            )
        })
        .unwrap_or((monitor_left, monitor_top, monitor_right - monitor_left));

    let x = (anchor_left + anchor_width / 2.0 - BAR_WIDTH / 2.0).clamp(
        monitor_left + 8.0,
        (monitor_right - BAR_WIDTH - 8.0).max(monitor_left + 8.0),
    );
    let above = anchor_top - BAR_HEIGHT - 8.0;
    let y = if above >= monitor_top + 8.0 {
        above
    } else {
        (anchor_top + 8.0).min(monitor_bottom - BAR_HEIGHT - 8.0)
    };

    Bounds {
        origin: point(px(x), px(y.max(monitor_top + 8.0))),
        size: size(px(BAR_WIDTH), px(BAR_HEIGHT)),
    }
}
