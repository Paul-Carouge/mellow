//! Spike measurement: scroll from the newest to the oldest message and report frame render times.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle as _, RenderingState, Timer, TimerMode};

use crate::generated_ui::AppWindow;

pub const ENABLE_SCROLL_BENCHMARK_VARIABLE: &str = "MELLOW_SPIKE_SCROLL_BENCHMARK";
/// One screen-and-a-half per step: fast enough to cross 10 000 messages, slow enough to render each step.
const SCROLL_STEP_PIXELS: f32 = 900.0;
const SCROLL_STEP_INTERVAL: Duration = Duration::from_millis(16);
const FRAME_BUDGET_AT_60_HZ: Duration = Duration::from_micros(16_667);

#[derive(Default)]
struct FrameTimings {
    render_started_at: Option<Instant>,
    render_durations: Vec<Duration>,
}

pub fn start(window: &AppWindow) -> Result<(), slint::PlatformError> {
    let timings = Rc::new(RefCell::new(FrameTimings::default()));

    let recorded_timings = timings.clone();
    window
        .window()
        .set_rendering_notifier(move |state, _graphics_api| {
            let mut timings = recorded_timings.borrow_mut();
            match state {
                RenderingState::BeforeRendering => timings.render_started_at = Some(Instant::now()),
                RenderingState::AfterRendering => {
                    if let Some(started_at) = timings.render_started_at.take() {
                        timings.render_durations.push(started_at.elapsed());
                    }
                }
                _ => {}
            }
        })
        .map_err(|error| {
            slint::PlatformError::Other(format!("rendering notifier unavailable: {error:?}"))
        })?;

    let weak_window = window.as_weak();
    let benchmark_started_at = Instant::now();
    let step_timer = Rc::new(Timer::default());
    let step_timer_handle = step_timer.clone();
    step_timer.start(TimerMode::Repeated, SCROLL_STEP_INTERVAL, move || {
        let _keep_timer_alive = &step_timer_handle;
        let Some(window) = weak_window.upgrade() else {
            return;
        };
        if window.invoke_scroll_up_by(SCROLL_STEP_PIXELS) {
            return;
        }
        report(
            &timings.borrow().render_durations,
            benchmark_started_at.elapsed(),
        );
        let _ = slint::quit_event_loop();
    });
    Ok(())
}

fn report(render_durations: &[Duration], total_duration: Duration) {
    let first_frame = render_durations.first().copied().unwrap_or_default();
    let mut sorted_durations = render_durations.get(1..).unwrap_or_default().to_vec();
    sorted_durations.sort_unstable();
    let percentile = |fraction: f64| {
        // Index derived from a fraction of the length, so it is always in bounds.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let index = ((sorted_durations.len().saturating_sub(1)) as f64 * fraction) as usize;
        sorted_durations.get(index).copied().unwrap_or_default()
    };
    let frames_over_budget = sorted_durations
        .iter()
        .filter(|duration| **duration > FRAME_BUDGET_AT_60_HZ)
        .count();
    eprintln!(
        "scroll benchmark: first frame {first_frame:?}; then {} frames in {:?}; render p50 {:?}, p95 {:?}, p99 {:?}, max {:?}; {} frames over 16.7 ms",
        sorted_durations.len(),
        total_duration,
        percentile(0.50),
        percentile(0.95),
        percentile(0.99),
        sorted_durations.last().copied().unwrap_or_default(),
        frames_over_budget,
    );
}
