use std::path::PathBuf;
use std::time::{Duration, Instant};

use core_capture::{CaptureTarget, MonitorInfo, platform_backend};
use core_record::{FrameRate, RecordConfig};
use core_types::{PhysicalPoint, PhysicalRect, RecordFormat};
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_record::Recorder;
use platform_window::{WindowStyle, apply as apply_window_style, system_accent_hex};

use crate::i18n::{Language, t};
use crate::settings_state;

const BAR_WIDTH: f32 = 500.0;
const BAR_HEIGHT: f32 = 68.0;
const FALLBACK_ACCENT: u32 = 0x4c9aff;

pub fn start(target: CaptureTarget, cx: &mut App) {
    let settings = settings_state::get();
    let format = settings.record_format;
    let output = match crate::storage::recording_path(format) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("capin: failed to prepare the recording output: {error}");
            return;
        }
    };
    let config = RecordConfig {
        target,
        format,
        frame_rate: if format == RecordFormat::WebP {
            FrameRate::Fps30
        } else {
            settings.record_frame_rate
        },
        include_cursor: settings.record_cursor,
    };

    let bounds = bar_bounds(target);
    let display_id = target_monitor(target).map(|monitor| DisplayId::new(monitor.raw()));
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
                eprintln!("capin: failed to style recording bar: {error}");
                false
            }
        };
        // The bar must exist before the macOS content filter is built so it
        // can be excluded from the stream. Windows already applies
        // WDA_EXCLUDEFROMCAPTURE above.
        let excluded: Vec<u64> = platform_window::native_window_id(&*window)
            .into_iter()
            .collect();
        let recorder = Recorder::start_excluding(config, output.clone(), &excluded)
            .map_err(|error| error.to_string());
        cx.new(|cx| RecordBar::new(output, recorder, format, exclusion_ok, cx))
    }) {
        Ok(_) => {}
        Err(error) => eprintln!("capin: failed to open recording bar: {error}"),
    }
}

struct RecordBar {
    output: PathBuf,
    recorder: Option<Recorder>,
    format: RecordFormat,
    fps: u32,
    started: Instant,
    paused_total: Duration,
    pause_started: Option<Instant>,
    paused: bool,
    stopping: bool,
    completed: bool,
    status: Option<String>,
    language: Language,
}

impl RecordBar {
    fn new(
        output: PathBuf,
        recorder: Result<Recorder, String>,
        format: RecordFormat,
        exclusion_ok: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = settings_state::get();
        let fps = settings.record_frame_rate.as_u32();
        let language = settings.language;
        let (recorder, mut status, completed) = match recorder {
            Ok(recorder) => (Some(recorder), None, false),
            Err(error) => (
                None,
                Some(format!("{}{}", t(language, "record.failed"), error)),
                true,
            ),
        };
        if !exclusion_ok && !completed {
            status = Some(t(language, "record.exclusion_failed").to_owned());
        }

        let bar = Self {
            output,
            recorder,
            format,
            fps,
            started: Instant::now(),
            paused_total: Duration::ZERO,
            pause_started: None,
            paused: false,
            stopping: false,
            completed,
            status,
            language,
        };
        if bar.recorder.is_some() {
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
        }
        bar
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
        let mut elapsed = self.started.elapsed().saturating_sub(self.paused_total);
        if let Some(started) = self.pause_started {
            elapsed = elapsed.saturating_sub(started.elapsed());
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
            return;
        };
        self.stopping = true;
        recorder.stop();
        self.status = Some(t(self.language, "record.saving").to_owned());
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        let handles = cx.window_stack().unwrap_or_default();
        cx.defer(move |cx| {
            crate::windows::close_auxiliary(cx, handles);
        });
    }
}

impl Render for RecordBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = rgb(system_accent_hex().unwrap_or(FALLBACK_ACCENT));
        let muted = rgba(0xFFFFFF99);
        let language = self.language;
        let elapsed = self.elapsed();
        let timer = format!(
            "{:02}:{:02}",
            elapsed.as_secs() / 60,
            elapsed.as_secs() % 60
        );
        let state = if self.completed {
            t(language, "record.done")
        } else if self.stopping {
            t(language, "record.saving")
        } else if self.paused {
            t(language, "record.paused")
        } else {
            t(language, "record.recording")
        };
        let status = self.status.clone();
        let pause_label = if self.paused {
            t(language, "record.resume").to_owned()
        } else {
            t(language, "record.pause").to_owned()
        };
        let pause = (!self.stopping && !self.completed).then(|| {
            crate::ui::flat_button("record-pause", pause_label, accent, cx, Self::toggle_pause)
        });
        let stop = (!self.stopping && !self.completed).then(|| {
            crate::ui::flat_button(
                "record-stop",
                t(language, "record.stop").to_owned(),
                accent,
                cx,
                Self::stop,
            )
        });
        let close = self.completed.then(|| {
            crate::ui::flat_button(
                "record-close",
                t(language, "record.close").to_owned(),
                muted,
                cx,
                Self::close,
            )
        });

        div().size_full().flex().items_center().child(
            div()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .px(px(12.0))
                .py(px(9.0))
                .rounded(px(12.0))
                .bg(rgba(0x1E1E1EF2))
                .border_1()
                .border_color(rgba(0xFFFFFF1F))
                .text_color(rgb(0xF2F2F2))
                .child(
                    div()
                        .w(px(8.0))
                        .h(px(8.0))
                        .rounded(px(4.0))
                        .bg(if self.paused {
                            rgba(0xFFFFFF66)
                        } else {
                            rgb(0xFF453A)
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .flex_1()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(state),
                                )
                                .child(div().text_size(px(12.0)).text_color(accent).child(timer))
                                .child(div().text_size(px(10.5)).text_color(muted).child(format!(
                                    "{} · {} fps",
                                    self.format.display_name(),
                                    self.fps
                                ))),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(muted)
                                .child(status.unwrap_or_else(|| self.output.display().to_string())),
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
