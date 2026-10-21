//! Spike measurement: what an idle window costs while Melli animates, visible and then minimized.
//! Nobody touches the window; the busiest demo channel is open. Read the CPU from the OS meanwhile.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle as _, RenderingState, Timer};

use crate::generated_ui::{AppWindow, MelliAnimation};

pub const ENABLE_IDLE_BENCHMARK_VARIABLE: &str = "MELLOW_SPIKE_IDLE_BENCHMARK";
/// Lets the first layout and font loading settle before counting.
const WARM_UP: Duration = Duration::from_secs(2);
const PHASE_LENGTH: Duration = Duration::from_secs(10);

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
    Timer::single_shot(WARM_UP, move || {
        timings.borrow_mut().render_durations.clear();
        eprintln!("idle benchmark: visible phase started");
        Timer::single_shot(PHASE_LENGTH, move || {
            report("visible", &timings.borrow().render_durations);
            timings.borrow_mut().render_durations.clear();
            if let Some(window) = weak_window.upgrade() {
                window.window().set_minimized(true);
            }
            eprintln!("idle benchmark: minimized phase started");
            Timer::single_shot(PHASE_LENGTH, move || {
                report("minimized", &timings.borrow().render_durations);
                if let Some(window) = weak_window.upgrade() {
                    eprintln!(
                        "idle benchmark: Melli running while minimized: {}",
                        window.global::<MelliAnimation>().get_is_running()
                    );
                }
                let _ = slint::quit_event_loop();
            });
        });
    });
    Ok(())
}

fn report(phase: &str, render_durations: &[Duration]) {
    let mut sorted_durations = render_durations.to_vec();
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
    // Frame counts over a few seconds fit an f64 exactly.
    #[allow(clippy::cast_precision_loss)]
    let frames_per_second = sorted_durations.len() as f64 / PHASE_LENGTH.as_secs_f64();
    eprintln!(
        "idle benchmark ({phase}): {} frames in {PHASE_LENGTH:?} ({frames_per_second:.1} fps); render p50 {:?}, p95 {:?}, max {:?}",
        sorted_durations.len(),
        percentile(0.50),
        percentile(0.95),
        sorted_durations.last().copied().unwrap_or_default(),
    );
}
