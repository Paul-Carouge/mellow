//! Melli is always animated while the window can be seen (the owner's choice, DESIGN.md §2.10).
//! Its loops read `animation-tick()`, which makes Slint redraw every frame; this stops them while
//! the window is occluded or minimized, so a hidden window costs nothing.

use slint::ComponentHandle as _;
use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::{EventResult, WinitWindowAccessor as _};

use crate::generated_ui::{AppWindow, MelliAnimation};

pub fn pause_while_window_is_hidden(window: &AppWindow) {
    let weak_window = window.as_weak();
    let mut is_occluded = false;
    // Windows sends no `Occluded` when minimizing, only a zero size.
    let mut is_zero_sized = false;
    window
        .window()
        .on_winit_window_event(move |_window, event| {
            match event {
                WindowEvent::Occluded(occluded) => is_occluded = *occluded,
                WindowEvent::Resized(size) => is_zero_sized = size.width == 0 || size.height == 0,
                _ => return EventResult::Propagate,
            }
            let should_run = !is_occluded && !is_zero_sized;
            if let Some(window) = weak_window.upgrade() {
                let animation = window.global::<MelliAnimation>();
                if animation.get_is_running() != should_run {
                    animation.set_is_running(should_run);
                    tracing::debug!(is_running = should_run, "Melli animation");
                }
            }
            EventResult::Propagate
        });
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{Platform, WindowAdapter};

    use super::*;

    struct HeadlessPlatform {
        window: Rc<MinimalSoftwareWindow>,
    }

    impl Platform for HeadlessPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            Ok(self.window.clone())
        }
    }

    /// The bug this guards against: an `f32` clock in milliseconds moves in 256 ms steps after a
    /// month, so Melli would stutter in an app left open.
    #[test]
    fn melli_loops_keep_frame_precision_after_a_month_of_uptime() {
        const MONTH_IN_MILLISECONDS: i64 = 30 * 24 * 60 * 60 * 1_000;
        const FLOAT_PERIOD_IN_MILLISECONDS: i64 = 3_200;
        const FRAME_IN_MILLISECONDS: i64 = 16;
        let platform = HeadlessPlatform {
            window: MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer),
        };
        slint::platform::set_platform(Box::new(platform)).unwrap();
        let window = AppWindow::new().unwrap();
        let animation = window.global::<MelliAnimation>();
        let cycle = |elapsed| animation.invoke_cycle(elapsed, FLOAT_PERIOD_IN_MILLISECONDS, 0.0);

        // A month is a whole number of 3.2 s loops.
        assert!(cycle(MONTH_IN_MILLISECONDS).abs() < 1e-6);
        let one_frame_later = cycle(MONTH_IN_MILLISECONDS + FRAME_IN_MILLISECONDS);
        assert!((one_frame_later - 0.005).abs() < 1e-6, "{one_frame_later}");
        let offset_by_half = animation.invoke_cycle(
            MONTH_IN_MILLISECONDS + FRAME_IN_MILLISECONDS,
            FLOAT_PERIOD_IN_MILLISECONDS,
            0.5,
        );
        assert!((offset_by_half - 0.505).abs() < 1e-6, "{offset_by_half}");
    }
}
