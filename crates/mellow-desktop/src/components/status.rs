//! `StatusIndicator.svelte` (DESIGN.md §2.3, §6.9): a shape per status (dot, crescent, barred
//! disc, ring), so colour never carries it alone. The inner shapes are true holes (even-odd fill),
//! so the surface behind shows through. Decorative: the host names the status in words.

use gpui::{
    App, Bounds, IntoElement, PathBuilder, Pixels, Point, RenderOnce, Styled as _, Window, canvas,
    point, px,
};

use crate::theme::tokens::ThemeTokens;
use crate::theme::{color, length, theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShownStatus {
    Online,
    Idle,
    DoNotDisturb,
    Offline,
    /// Looks offline, as others see it.
    Invisible,
}

impl ShownStatus {
    pub const ALL: [Self; 5] = [
        Self::Online,
        Self::Idle,
        Self::DoNotDisturb,
        Self::Offline,
        Self::Invisible,
    ];

    fn colour_token(self, tokens: &ThemeTokens) -> u32 {
        match self {
            Self::Online => tokens.component_status_online,
            Self::Idle => tokens.component_status_idle,
            Self::DoNotDisturb => tokens.component_status_dnd,
            Self::Offline | Self::Invisible => tokens.component_status_offline,
        }
    }
}

/// The web draws on a 20-unit grid, as Discord does.
const GRID_UNITS: f32 = 20.0;

/// One drawing step of the web's SVG paths, in grid units.
#[derive(Debug, Clone, Copy)]
enum PathStep {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    /// Radius, large-arc and sweep flags, end point.
    ArcTo(f32, bool, bool, f32, f32),
    Close,
}

use PathStep::{ArcTo, Close, LineTo, MoveTo};

const DISC: [PathStep; 4] = [
    MoveTo(10.0, 0.0),
    ArcTo(10.0, true, true, 10.0, 20.0),
    ArcTo(10.0, true, true, 10.0, 0.0),
    Close,
];

/// The steps of `PATH_BY_STATUS` in `StatusIndicator.svelte`.
fn path_steps(status: ShownStatus) -> Vec<PathStep> {
    match status {
        ShownStatus::Online => DISC.to_vec(),
        // The disc minus a 7.5-radius circle centred at (5, 5): the arcs meet where both cross.
        ShownStatus::Idle => vec![
            MoveTo(10.607, 0.018),
            ArcTo(10.0, true, true, 0.018, 10.607),
            ArcTo(7.5, false, false, 10.607, 0.018),
            Close,
        ],
        ShownStatus::DoNotDisturb => {
            let mut steps = DISC.to_vec();
            steps.extend([
                MoveTo(5.0, 7.5),
                LineTo(15.0, 7.5),
                ArcTo(2.5, false, true, 15.0, 12.5),
                LineTo(5.0, 12.5),
                ArcTo(2.5, false, true, 5.0, 7.5),
                Close,
            ]);
            steps
        }
        ShownStatus::Offline | ShownStatus::Invisible => {
            let mut steps = DISC.to_vec();
            steps.extend([
                MoveTo(10.0, 5.0),
                ArcTo(5.0, true, false, 10.0, 15.0),
                ArcTo(5.0, true, false, 10.0, 5.0),
                Close,
            ]);
            steps
        }
    }
}

fn status_path(status: ShownStatus, bounds: Bounds<Pixels>) -> PathBuilder {
    let scale = f32::from(bounds.size.width) / GRID_UNITS;
    let at =
        |x: f32, y: f32| -> Point<Pixels> { bounds.origin + point(px(x * scale), px(y * scale)) };
    let mut builder = PathBuilder::fill();
    for step in path_steps(status) {
        match step {
            MoveTo(x, y) => builder.move_to(at(x, y)),
            LineTo(x, y) => builder.line_to(at(x, y)),
            ArcTo(radius, is_large, is_sweep, x, y) => {
                let radius = px(radius * scale);
                builder.arc_to(point(radius, radius), px(0.0), is_large, is_sweep, at(x, y));
            }
            Close => builder.close(),
        }
    }
    builder
}

/// `size` is one of the `component.status.size-*` tokens.
#[derive(Debug, IntoElement)]
pub struct StatusIndicator {
    status: ShownStatus,
    size: f32,
}

impl StatusIndicator {
    #[must_use]
    pub fn new(status: ShownStatus, size: f32) -> Self {
        Self { status, size }
    }
}

impl RenderOnce for StatusIndicator {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let fill = color(self.status.colour_token(theme(cx)));
        let status = self.status;
        canvas(
            |_, _, _| {},
            move |bounds, (), window, _| {
                // A malformed path cannot come from these constants; drawing nothing is the
                // harmless outcome if lyon ever refused one.
                if let Ok(path) = status_path(status, bounds).build() {
                    window.paint_path(path, fill);
                }
            },
        )
        .flex_none()
        .size(length(self.size))
    }
}

#[cfg(test)]
mod tests {
    use gpui::size;

    use super::*;

    #[test]
    fn every_status_tessellates_into_a_shape() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(10.0), px(10.0)));
        for status in ShownStatus::ALL {
            assert!(status_path(status, bounds).build().is_ok(), "{status:?}");
        }
    }

    #[test]
    fn invisible_looks_offline() {
        let tokens = &crate::theme::tokens::DARK_THEME;
        assert_eq!(
            ShownStatus::Invisible.colour_token(tokens),
            ShownStatus::Offline.colour_token(tokens)
        );
    }
}
