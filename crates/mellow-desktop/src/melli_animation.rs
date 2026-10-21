//! Melli's shared animation clock (`MelliAnimation` in `melli.slint`).

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{Platform, WindowAdapter};

    use slint::ComponentHandle as _;

    use crate::generated_ui::{AppWindow, MelliAnimation};

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
