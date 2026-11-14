//! `Melli.svelte` (DESIGN.md §2.10): Melli, the Mellow mascot and logo, drawn as vector paths in
//! the web's 256-unit viewBox, part for part, so it stays sharp at every size and zoom. A state only
//! shows, hides or restyles parts: loops restart from rest when the state changes, poses (eyes,
//! face, colours, marks) ease over `component.melli.state-change-duration`, and the puffs keep their
//! angle while they pause. Under reduced motion (or `still`) Melli shows the still pose of its state
//! and requests no frame. Decorative: the host names what Melli stands for.
//!
//! Cost: each Melli is a cached view, and every animated Melli shares one timer at 30 frames a
//! second, which stops when no Melli was drawn since its last tick (a hidden or minimized window
//! gets no frames). GPUI still redraws the views around a notified view, so a host that wants a
//! Melli frame to cost only the Melli puts its own heavy panes in cached views.
//!
//! The states are the ones a desktop host can show today: the Home button's rule (`idle`,
//! `connecting`, `connection-lost`, `notification`, `do-not-disturb`). The web's call, update and
//! sleeping states and the chime are not wired on either client's desktop yet (§2.10).

use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, Context, ElementId, Global, Hsla, IntoElement, PathBuilder, Pixels, Render,
    RenderOnce, Rgba, StyleRefinement, Styled as _, Task, WeakEntity, Window, canvas, point, px,
    rgba,
};
use gpui_base::motion::Easing;

use super::easing;
use crate::session::ConnectionStatus;
use crate::theme::tokens::{
    COMPONENT_MELLI_BLINK_DURATION, COMPONENT_MELLI_FLOAT_DURATION, COMPONENT_MELLI_HOP_DURATION,
    COMPONENT_MELLI_PUFF_SPIN_DURATION, COMPONENT_MELLI_RAIN_DURATION,
    COMPONENT_MELLI_SLEEP_DURATION, COMPONENT_MELLI_STATE_CHANGE_DURATION,
    COMPONENT_MELLI_STATE_CHANGE_EASING, COMPONENT_MELLI_WAIT_DURATION, Delay,
};
use crate::theme::{length, theme};

/// What Melli is showing (`melli.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MelliState {
    /// Nothing to report: the logo.
    #[default]
    Idle,
    /// Connecting: looks up at three dots that rise in turn.
    Connecting,
    /// The connection dropped and the client reconnects: overcast, heavy eyes, raining.
    ConnectionLost,
    /// Unread conversations or mentions: happy eyes, blush, two sparkles.
    Notification,
    /// Do not disturb: eyes closed under a crossed-out bell, a slow breath.
    DoNotDisturb,
}

impl MelliState {
    /// Every state, in the gallery's order.
    pub const ALL: [Self; 5] = [
        Self::Idle,
        Self::Connecting,
        Self::ConnectionLost,
        Self::Notification,
        Self::DoNotDisturb,
    ];

    /// The web's name, for the gallery and logs.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Connecting => "connecting",
            Self::ConnectionLost => "connection-lost",
            Self::Notification => "notification",
            Self::DoNotDisturb => "do-not-disturb",
        }
    }

    /// The Home button's Melli (`homeMelliState`): connection trouble first, then Do not disturb
    /// (quiet even with unread messages), then something for the reader.
    #[must_use]
    pub fn for_home(
        status: ConnectionStatus,
        is_do_not_disturb: bool,
        has_something_for_the_reader: bool,
    ) -> Self {
        match status {
            ConnectionStatus::Reconnecting => Self::ConnectionLost,
            ConnectionStatus::Connecting => Self::Connecting,
            ConnectionStatus::Connected | ConnectionStatus::SignedOut => {
                if is_do_not_disturb {
                    Self::DoNotDisturb
                } else if has_something_for_the_reader {
                    Self::Notification
                } else {
                    Self::Idle
                }
            }
        }
    }

    /// Whether the puffs turn; they pause (keeping their angle) while the connection is lost and
    /// in Do not disturb.
    fn are_puffs_turning(self) -> bool {
        !matches!(self, Self::ConnectionLost | Self::DoNotDisturb)
    }

    /// The parts a state shows or restyles; these ease between states.
    fn pose(self) -> Pose {
        let rest = Pose {
            face_offset: (0.0, 0.0),
            eyes_scale_y: 1.0,
            eyes_opacity: 1.0,
            happy_eyes_opacity: 0.0,
            closed_eyes_opacity: 0.0,
            blush_opacity: 0.0,
            overcast: 0.0,
            dots_opacity: 0.0,
            drops_opacity: 0.0,
            sparkle_opacity: 0.0,
            bell_opacity: 0.0,
        };
        match self {
            Self::Idle => rest,
            Self::Connecting => Pose {
                face_offset: (3.0, -5.0),
                dots_opacity: 1.0,
                ..rest
            },
            Self::ConnectionLost => Pose {
                eyes_scale_y: 0.45,
                overcast: 1.0,
                drops_opacity: 1.0,
                ..rest
            },
            Self::Notification => Pose {
                eyes_opacity: 0.0,
                happy_eyes_opacity: 1.0,
                blush_opacity: 1.0,
                sparkle_opacity: 1.0,
                ..rest
            },
            Self::DoNotDisturb => Pose {
                eyes_opacity: 0.0,
                closed_eyes_opacity: 1.0,
                bell_opacity: 1.0,
                ..rest
            },
        }
    }
}

/// The values that ease between states (CSS transitions in `Melli.svelte`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pose {
    face_offset: (f32, f32),
    eyes_scale_y: f32,
    eyes_opacity: f32,
    happy_eyes_opacity: f32,
    closed_eyes_opacity: f32,
    blush_opacity: f32,
    /// 0 is the cream cloud, 1 the overcast one.
    overcast: f32,
    dots_opacity: f32,
    drops_opacity: f32,
    sparkle_opacity: f32,
    bell_opacity: f32,
}

fn mix(from: f32, to: f32, progress: f32) -> f32 {
    from + (to - from) * progress
}

impl Pose {
    fn towards(self, to: Self, progress: f32) -> Self {
        let at = |from: f32, target: f32| mix(from, target, progress);
        Self {
            face_offset: (
                at(self.face_offset.0, to.face_offset.0),
                at(self.face_offset.1, to.face_offset.1),
            ),
            eyes_scale_y: at(self.eyes_scale_y, to.eyes_scale_y),
            eyes_opacity: at(self.eyes_opacity, to.eyes_opacity),
            happy_eyes_opacity: at(self.happy_eyes_opacity, to.happy_eyes_opacity),
            closed_eyes_opacity: at(self.closed_eyes_opacity, to.closed_eyes_opacity),
            blush_opacity: at(self.blush_opacity, to.blush_opacity),
            overcast: at(self.overcast, to.overcast),
            dots_opacity: at(self.dots_opacity, to.dots_opacity),
            drops_opacity: at(self.drops_opacity, to.drops_opacity),
            sparkle_opacity: at(self.sparkle_opacity, to.sparkle_opacity),
            bell_opacity: at(self.bell_opacity, to.bell_opacity),
        }
    }
}

/// Where a loop is, from 0 to 1. The elapsed time is taken modulo the loop before dividing, so the
/// motion stays smooth however long the app runs.
fn cycle(elapsed: Duration, period: Delay) -> f32 {
    let period_ms = u128::from(period.0.max(1));
    let within = elapsed.as_millis() % period_ms;
    // Both are below the period, a few seconds: exact in an f32.
    #[allow(clippy::cast_precision_loss)]
    let progress = within as f32 / period_ms as f32;
    progress
}

/// Where a loop is when it started `offset_fraction` of its period later (positive, the web's
/// `animation-delay`) or earlier (negative). Before a delayed loop starts, the part shows its rest
/// value: `None`. Whole milliseconds until the last step, so a month-old clock is as exact as a
/// new one.
fn delayed(elapsed: Duration, period: Delay, offset_fraction: f32) -> Option<f32> {
    let period_ms = i128::from(period.0.max(1));
    // The offsets are fractions of a few-second period: well inside an f32's exact range.
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    let offset_ms = (period.0 as f32 * offset_fraction).round() as i128;
    let elapsed_ms = i128::try_from(elapsed.as_millis()).unwrap_or(i128::MAX / 2);
    let shifted_ms = elapsed_ms - offset_ms;
    if shifted_ms < 0 {
        return None;
    }
    // Both are below the period, a few seconds: exact in an f32.
    #[allow(clippy::cast_precision_loss)]
    let progress = (shifted_ms % period_ms) as f32 / period_ms as f32;
    Some(progress)
}

/// A CSS keyframe track: `(offset, value)` stops from 0 to 1, each segment eased by `easing`, as an
/// `animation-timing-function` applies per keyframe.
fn keyframes(progress: f32, stops: &[(f32, f32)], easing: &Easing) -> f32 {
    let Some(&(_, first)) = stops.first() else {
        return 0.0;
    };
    let mut previous = (0.0, first);
    for &(offset, value) in stops {
        if progress <= offset {
            let span = offset - previous.0;
            if span <= f32::EPSILON {
                return value;
            }
            return mix(
                previous.1,
                value,
                easing.sample((progress - previous.0) / span),
            );
        }
        previous = (offset, value);
    }
    previous.1
}

/// The looping values of one frame; all at rest (identity) in a still pose.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Loops {
    /// The cloud and face: translation then squash, pivoting on the cloud's base (128, 200).
    motion_offset_y: f32,
    motion_scale: (f32, f32),
    shadow_scale_x: f32,
    shadow_opacity: f32,
    blink_scale_y: f32,
    dots: [(f32, f32); 3],
    drops: [(f32, f32); 3],
    /// Each sparkle's scale and turn in degrees.
    stars: [(f32, f32); 2],
}

impl Loops {
    const REST: Self = Self {
        motion_offset_y: 0.0,
        motion_scale: (1.0, 1.0),
        shadow_scale_x: 1.0,
        shadow_opacity: 1.0,
        blink_scale_y: 1.0,
        // (vertical offset, opacity)
        dots: [(0.0, 1.0); 3],
        drops: [(0.0, 1.0); 3],
        stars: [(1.0, 0.0); 2],
    };

    /// The loops of `state`, `elapsed` after it started (the keyframes of `Melli.svelte`).
    fn at(state: MelliState, elapsed: Duration) -> Self {
        let ease_in_out = Easing::EaseInOut;
        let mut loops = Self::REST;
        let float = cycle(elapsed, COMPONENT_MELLI_FLOAT_DURATION);
        loops.blink_scale_y = keyframes(
            cycle(elapsed, COMPONENT_MELLI_BLINK_DURATION),
            &[(0.0, 1.0), (0.92, 1.0), (0.95, 0.1), (1.0, 1.0)],
            &Easing::Ease,
        );
        match state {
            MelliState::Idle | MelliState::Connecting | MelliState::Notification => {
                loops.motion_offset_y =
                    keyframes(float, &[(0.0, 0.0), (0.5, -8.0), (1.0, 0.0)], &ease_in_out);
                loops.shadow_scale_x =
                    keyframes(float, &[(0.0, 1.0), (0.5, 0.82), (1.0, 1.0)], &ease_in_out);
                loops.shadow_opacity =
                    keyframes(float, &[(0.0, 1.0), (0.5, 0.7), (1.0, 1.0)], &ease_in_out);
            }
            MelliState::ConnectionLost => {
                loops.motion_offset_y =
                    keyframes(float, &[(0.0, 0.0), (0.5, 4.0), (1.0, 0.0)], &ease_in_out);
                let squash = keyframes(float, &[(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)], &ease_in_out);
                loops.motion_scale = (mix(1.0, 1.02, squash), mix(1.0, 0.98, squash));
            }
            MelliState::DoNotDisturb => {
                let breath = keyframes(
                    cycle(elapsed, COMPONENT_MELLI_SLEEP_DURATION),
                    &[(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)],
                    &ease_in_out,
                );
                loops.motion_scale = (mix(1.0, 1.04, breath), mix(1.0, 0.96, breath));
                loops.shadow_scale_x = mix(1.0, 1.06, breath);
            }
        }
        match state {
            MelliState::Connecting => {
                for (index, dot) in loops.dots.iter_mut().enumerate() {
                    #[allow(clippy::cast_precision_loss)]
                    let delay = index as f32 / 6.0;
                    if let Some(progress) = delayed(elapsed, COMPONENT_MELLI_WAIT_DURATION, delay) {
                        let stops = [(0.0, 0.0), (0.3, 1.0), (0.6, 0.0), (1.0, 0.0)];
                        let rise = keyframes(progress, &stops, &ease_in_out);
                        *dot = (mix(0.0, -6.0, rise), mix(0.3, 1.0, rise));
                    }
                }
            }
            MelliState::ConnectionLost => {
                for (index, drop) in loops.drops.iter_mut().enumerate() {
                    #[allow(clippy::cast_precision_loss)]
                    let delay = -(index as f32) / 3.0;
                    let progress =
                        delayed(elapsed, COMPONENT_MELLI_RAIN_DURATION, delay).unwrap_or(0.0);
                    let fall = keyframes(progress, &[(0.0, -24.0), (1.0, 16.0)], &Easing::EaseIn);
                    let opacity = keyframes(
                        progress,
                        &[(0.0, 0.0), (0.25, 1.0), (0.75, 1.0), (1.0, 0.0)],
                        &Easing::EaseIn,
                    );
                    *drop = (fall, opacity);
                }
            }
            MelliState::Notification => {
                for (index, star) in loops.stars.iter_mut().enumerate() {
                    #[allow(clippy::cast_precision_loss)]
                    let delay = -(index as f32) / 2.0;
                    let progress =
                        delayed(elapsed, COMPONENT_MELLI_HOP_DURATION, delay).unwrap_or(0.0);
                    let stops = [(0.0, 0.0), (0.28, 1.0), (0.6, 0.0), (1.0, 0.0)];
                    let twinkle = keyframes(progress, &stops, &ease_in_out);
                    *star = (mix(0.8, 1.2, twinkle), mix(0.0, 20.0, twinkle));
                }
            }
            MelliState::Idle | MelliState::DoNotDisturb => {}
        }
        loops
    }
}

/// What one frame draws: the pose, the loops and the puffs' angle.
#[derive(Debug, Clone, Copy, PartialEq)]
struct MelliFrame {
    pose: Pose,
    loops: Loops,
    puff_degrees: f32,
}

/// Between frames: when the state's loop started, the pose eased from, and the puffs' turn.
#[derive(Debug, Clone, Copy)]
struct MelliClock {
    state: MelliState,
    loop_started: Instant,
    pose_from: Pose,
    pose_changed_at: Option<Instant>,
    /// The puffs' angle when they last paused or resumed.
    puff_degrees: f32,
    puffs_resumed_at: Option<Instant>,
}

impl MelliClock {
    fn new(state: MelliState, now: Instant) -> Self {
        Self {
            state,
            loop_started: now,
            pose_from: state.pose(),
            pose_changed_at: None,
            puff_degrees: 0.0,
            puffs_resumed_at: state.are_puffs_turning().then_some(now),
        }
    }

    fn shown_pose(&self, now: Instant) -> Pose {
        let target = self.state.pose();
        let Some(changed_at) = self.pose_changed_at else {
            return target;
        };
        #[allow(clippy::cast_precision_loss)]
        let progress = now.saturating_duration_since(changed_at).as_millis() as f32
            / COMPONENT_MELLI_STATE_CHANGE_DURATION.0.max(1) as f32;
        if progress >= 1.0 {
            return target;
        }
        let curve = easing(COMPONENT_MELLI_STATE_CHANGE_EASING);
        self.pose_from.towards(target, curve.sample(progress))
    }

    fn puff_degrees_at(&self, now: Instant) -> f32 {
        let turning = self.puffs_resumed_at.map_or(Duration::ZERO, |resumed_at| {
            now.saturating_duration_since(resumed_at)
        });
        (self.puff_degrees + 360.0 * cycle(turning, COMPONENT_MELLI_PUFF_SPIN_DURATION)) % 360.0
    }

    /// Takes in the state shown now and returns the frame to draw. A still Melli changes pose at
    /// once and shows its loops at rest.
    fn frame(&mut self, state: MelliState, now: Instant, is_still: bool) -> MelliFrame {
        if state != self.state {
            self.pose_from = self.shown_pose(now);
            self.pose_changed_at = (!is_still).then_some(now);
            self.loop_started = now;
            self.puff_degrees = self.puff_degrees_at(now);
            self.puffs_resumed_at = state.are_puffs_turning().then_some(now);
            self.state = state;
        }
        if is_still {
            return MelliFrame {
                pose: state.pose(),
                loops: Loops::REST,
                puff_degrees: 0.0,
            };
        }
        MelliFrame {
            pose: self.shown_pose(now),
            loops: Loops::at(state, now.saturating_duration_since(self.loop_started)),
            puff_degrees: self.puff_degrees_at(now),
        }
    }
}

/// A 2D affine map `(x, y) -> (a x + c y + e, b x + d y + f)`, in viewBox units until the last
/// step maps them to the window.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Affine {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Affine {
    const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    fn translation(x: f32, y: f32) -> Self {
        Self {
            e: x,
            f: y,
            ..Self::IDENTITY
        }
    }

    /// Applies `self`, then `next`.
    fn then(self, next: Self) -> Self {
        Self {
            a: next.a * self.a + next.c * self.b,
            b: next.b * self.a + next.d * self.b,
            c: next.a * self.c + next.c * self.d,
            d: next.b * self.c + next.d * self.d,
            e: next.a * self.e + next.c * self.f + next.e,
            f: next.b * self.e + next.d * self.f + next.f,
        }
    }

    /// CSS `scale(x, y)` with its `transform-origin` at `(origin_x, origin_y)`.
    fn scale_about(x: f32, y: f32, origin_x: f32, origin_y: f32) -> Self {
        Self::translation(-origin_x, -origin_y)
            .then(Self {
                a: x,
                d: y,
                ..Self::IDENTITY
            })
            .then(Self::translation(origin_x, origin_y))
    }

    /// CSS `rotate(degrees)` around `(origin_x, origin_y)`; clockwise on screen, as in SVG.
    fn rotation_about(degrees: f32, origin_x: f32, origin_y: f32) -> Self {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Self::translation(-origin_x, -origin_y)
            .then(Self {
                a: cos,
                b: sin,
                c: -sin,
                d: cos,
                ..Self::IDENTITY
            })
            .then(Self::translation(origin_x, origin_y))
    }

    fn apply(self, x: f32, y: f32) -> gpui::Point<Pixels> {
        point(
            px(self.a * x + self.c * y + self.e),
            px(self.b * x + self.d * y + self.f),
        )
    }

    /// How much the map scales lengths, for stroke widths.
    fn length_scale(self) -> f32 {
        (self.a * self.d - self.b * self.c).abs().sqrt()
    }

    /// The scale of a map that keeps circles round (a uniform scale, a turn, a move), or `None`
    /// when it squashes them.
    fn circle_scale(self) -> Option<f32> {
        let is_similarity =
            (self.a - self.d).abs() < SHAPE_TOLERANCE && (self.b + self.c).abs() < SHAPE_TOLERANCE;
        is_similarity.then(|| self.a.hypot(self.b))
    }

    /// The scale of a map that keeps rectangles upright and their corners round (a uniform scale
    /// and a move), or `None`.
    fn upright_scale(self) -> Option<f32> {
        let is_upright = self.b.abs() < SHAPE_TOLERANCE && self.c.abs() < SHAPE_TOLERANCE;
        (is_upright && (self.a - self.d).abs() < SHAPE_TOLERANCE).then_some(self.a)
    }
}

/// How far from exact a map may be and still draw a circle or a pill as a rounded quad: far below
/// what a pixel shows at Melli's sizes.
const SHAPE_TOLERANCE: f32 = 1e-4;

/// The circle's Bézier constant: four cubic quarters within 0.03 % of a true circle.
const KAPPA: f32 = 0.552_284_7;
/// `stroke-width` of the eyes' arcs and the crossed-out bell.
const STROKE_WIDTH: f32 = 8.0;
const VIEW_BOX: f32 = 256.0;

/// The cloud: a core and six puffs; the outline is the same circles grown by `OUTLINE_WIDTH`.
const PUFF_CIRCLES: [(f32, f32, f32); 7] = [
    (128.0, 124.0, 62.0),
    (128.0, 78.0, 30.0),
    (167.8, 101.0, 30.0),
    (167.8, 147.0, 30.0),
    (128.0, 170.0, 30.0),
    (88.2, 147.0, 30.0),
    (88.2, 101.0, 30.0),
];
const OUTLINE_WIDTH: f32 = 6.0;
const DROP_CENTRES: [f32; 3] = [104.0, 128.0, 152.0];
const DOT_CENTRES: [f32; 3] = [156.0, 180.0, 204.0];

/// Paths in viewBox units, mapped through `transform` as they are built, so they are tessellated
/// at the size they are drawn.
struct Painter<'a> {
    window: &'a mut Window,
}

impl Painter<'_> {
    fn fill(&mut self, builder: PathBuilder, colour: Hsla) {
        if colour.a <= 0.0 {
            return;
        }
        // These shapes are constants; lyon refusing one would only skip the part.
        if let Ok(path) = builder.build() {
            self.window.paint_path(path, colour);
        }
    }

    /// A rounded quad, which GPUI draws with an exact anti-aliased edge and no tessellation.
    fn rounded_quad(
        &mut self,
        origin: gpui::Point<Pixels>,
        size: (f32, f32),
        radius: f32,
        colour: Hsla,
    ) {
        if colour.a <= 0.0 {
            return;
        }
        let bounds = Bounds::new(origin, gpui::size(px(size.0), px(size.1)));
        self.window
            .paint_quad(gpui::fill(bounds, colour).corner_radii(px(radius)));
    }

    fn ellipse(&mut self, transform: Affine, centre: (f32, f32), radii: (f32, f32), colour: Hsla) {
        let (x, y) = centre;
        let (rx, ry) = radii;
        // Most of Melli is circles under a move, a turn or a uniform scale: no path needed.
        if (rx - ry).abs() < SHAPE_TOLERANCE
            && let Some(scale) = transform.circle_scale()
        {
            let radius = rx * scale;
            let middle = transform.apply(x, y);
            self.rounded_quad(
                middle - point(px(radius), px(radius)),
                (2.0 * radius, 2.0 * radius),
                radius,
                colour,
            );
            return;
        }
        let (kx, ky) = (KAPPA * rx, KAPPA * ry);
        let at = |px: f32, py: f32| transform.apply(px, py);
        let mut builder = PathBuilder::fill();
        builder.move_to(at(x + rx, y));
        builder.cubic_bezier_to(at(x, y + ry), at(x + rx, y + ky), at(x + kx, y + ry));
        builder.cubic_bezier_to(at(x - rx, y), at(x - kx, y + ry), at(x - rx, y + ky));
        builder.cubic_bezier_to(at(x, y - ry), at(x - rx, y - ky), at(x - kx, y - ry));
        builder.cubic_bezier_to(at(x + rx, y), at(x + kx, y - ry), at(x + rx, y - ky));
        builder.close();
        self.fill(builder, colour);
    }

    fn circle(&mut self, transform: Affine, centre: (f32, f32), radius: f32, colour: Hsla) {
        self.ellipse(transform, centre, (radius, radius), colour);
    }

    /// A vertical pill (`rect` with `rx` half its width): the eyes.
    fn pill(&mut self, transform: Affine, origin: (f32, f32), size: (f32, f32), colour: Hsla) {
        let (x, y) = origin;
        let (width, height) = size;
        let radius = width / 2.0;
        // Upright and unsquashed (no blink, no heavy eyes): a rounded quad.
        if let Some(scale) = transform.upright_scale() {
            self.rounded_quad(
                transform.apply(x, y),
                (width * scale, height * scale),
                radius * scale,
                colour,
            );
            return;
        }
        let k = KAPPA * radius;
        let at = |px: f32, py: f32| transform.apply(px, py);
        let mut builder = PathBuilder::fill();
        builder.move_to(at(x, y + radius));
        builder.cubic_bezier_to(
            at(x + radius, y),
            at(x, y + radius - k),
            at(x + radius - k, y),
        );
        builder.cubic_bezier_to(
            at(x + width, y + radius),
            at(x + radius + k, y),
            at(x + width, y + radius - k),
        );
        builder.line_to(at(x + width, y + height - radius));
        builder.cubic_bezier_to(
            at(x + radius, y + height),
            at(x + width, y + height - radius + k),
            at(x + radius + k, y + height),
        );
        builder.cubic_bezier_to(
            at(x, y + height - radius),
            at(x + radius - k, y + height),
            at(x, y + height - radius + k),
        );
        builder.close();
        self.fill(builder, colour);
    }

    /// A stroked quadratic with round caps (`stroke-linecap: round`): lyon's stroke has butt caps,
    /// so each end gets a disc of the stroke's width. The discs overlap the stroke, so `colour`
    /// must be opaque: a fading eye passes its ink already blended over the cloud
    /// (`blended_over`), never a translucent one whose overlaps would show darker ends.
    fn opaque_quadratic_stroke(
        &mut self,
        transform: Affine,
        from: (f32, f32),
        control: (f32, f32),
        to: (f32, f32),
        colour: Hsla,
    ) {
        let mut builder = PathBuilder::stroke(px(STROKE_WIDTH * transform.length_scale()));
        builder.move_to(transform.apply(from.0, from.1));
        builder.curve_to(
            transform.apply(to.0, to.1),
            transform.apply(control.0, control.1),
        );
        self.fill(builder, colour);
        self.circle(transform, from, STROKE_WIDTH / 2.0, colour);
        self.circle(transform, to, STROKE_WIDTH / 2.0, colour);
    }

    /// A straight stroke with round caps, as one filled outline (a stadium), so it fades as a whole
    /// over whatever lies under it: the crossed-out bell's line.
    fn line_stroke(&mut self, transform: Affine, from: (f32, f32), to: (f32, f32), colour: Hsla) {
        let length = (to.0 - from.0).hypot(to.1 - from.1);
        if length <= f32::EPSILON {
            return;
        }
        let radius = STROKE_WIDTH / 2.0;
        // Along the line, and across it, both `radius` long.
        let along = (
            (to.0 - from.0) / length * radius,
            (to.1 - from.1) / length * radius,
        );
        let across = (-along.1, along.0);
        let k = KAPPA;
        let at = |(x, y): (f32, f32)| transform.apply(x, y);
        let offset =
            |(x, y): (f32, f32), (dx, dy): (f32, f32), scale: f32| (x + dx * scale, y + dy * scale);
        let mut builder = PathBuilder::fill();
        builder.move_to(at(offset(from, across, 1.0)));
        builder.line_to(at(offset(to, across, 1.0)));
        // The cap around `to`: from one side, round its tip, to the other side.
        let to_tip = offset(to, along, 1.0);
        builder.cubic_bezier_to(
            at(to_tip),
            at(offset(offset(to, across, 1.0), along, k)),
            at(offset(to_tip, across, k)),
        );
        builder.cubic_bezier_to(
            at(offset(to, across, -1.0)),
            at(offset(to_tip, across, -k)),
            at(offset(offset(to, across, -1.0), along, k)),
        );
        builder.line_to(at(offset(from, across, -1.0)));
        let from_tip = offset(from, along, -1.0);
        builder.cubic_bezier_to(
            at(from_tip),
            at(offset(offset(from, across, -1.0), along, -k)),
            at(offset(from_tip, across, -k)),
        );
        builder.cubic_bezier_to(
            at(offset(from, across, 1.0)),
            at(offset(from_tip, across, k)),
            at(offset(offset(from, across, 1.0), along, -k)),
        );
        builder.close();
        self.fill(builder, colour);
    }

    /// A drop: `M x 194 C x+4 200 x+7 204 x+7 207 A 7 7 0 0 1 x-7 207 C x-7 204 x-4 200 x 194 Z`.
    fn drop(&mut self, transform: Affine, x: f32, colour: Hsla) {
        let at = |px: f32, py: f32| transform.apply(px, py);
        let k = KAPPA * 7.0;
        let mut builder = PathBuilder::fill();
        builder.move_to(at(x, 194.0));
        builder.cubic_bezier_to(at(x + 7.0, 207.0), at(x + 4.0, 200.0), at(x + 7.0, 204.0));
        builder.cubic_bezier_to(at(x, 214.0), at(x + 7.0, 207.0 + k), at(x + k, 214.0));
        builder.cubic_bezier_to(at(x - 7.0, 207.0), at(x - k, 214.0), at(x - 7.0, 207.0 + k));
        builder.cubic_bezier_to(at(x, 194.0), at(x - 7.0, 204.0), at(x - 4.0, 200.0));
        builder.close();
        self.fill(builder, colour);
    }

    /// A four-pointed sparkle of quadratic sides: tips `reach` from `centre`, each side's control
    /// point `bend` from both axes.
    fn star(&mut self, transform: Affine, centre: (f32, f32), shape: (f32, f32), colour: Hsla) {
        let (x, y) = centre;
        let (reach, bend) = shape;
        let at = |px: f32, py: f32| transform.apply(px, py);
        let mut builder = PathBuilder::fill();
        builder.move_to(at(x, y - reach));
        builder.curve_to(at(x + reach, y), at(x + bend, y - bend));
        builder.curve_to(at(x, y + reach), at(x + bend, y + bend));
        builder.curve_to(at(x - reach, y), at(x - bend, y + bend));
        builder.curve_to(at(x, y - reach), at(x - bend, y - bend));
        builder.close();
        self.fill(builder, colour);
    }

    /// The small bell: `M176 76V62A16 16 0 0 1 208 62V76L214 82H170Z` and its clapper.
    fn bell(&mut self, transform: Affine, colour: Hsla) {
        let at = |px: f32, py: f32| transform.apply(px, py);
        let k = KAPPA * 16.0;
        let mut builder = PathBuilder::fill();
        builder.move_to(at(176.0, 76.0));
        builder.line_to(at(176.0, 62.0));
        builder.cubic_bezier_to(at(192.0, 46.0), at(176.0, 62.0 - k), at(192.0 - k, 46.0));
        builder.cubic_bezier_to(at(208.0, 62.0), at(192.0 + k, 46.0), at(208.0, 62.0 - k));
        builder.line_to(at(208.0, 76.0));
        builder.line_to(at(214.0, 82.0));
        builder.line_to(at(170.0, 82.0));
        builder.close();
        self.fill(builder, colour);
        self.circle(transform, (192.0, 88.0), 6.0, colour);
    }
}

fn with_opacity(colour: Hsla, opacity: f32) -> Hsla {
    Hsla {
        a: colour.a * opacity.clamp(0.0, 1.0),
        ..colour
    }
}

/// `top` at `opacity` over the opaque `base`, as one opaque colour.
fn blended_over(base: Hsla, top: Hsla, opacity: f32) -> Hsla {
    let (base, top) = (base.to_rgb(), top.to_rgb());
    let opacity = (opacity * top.a).clamp(0.0, 1.0);
    Rgba {
        r: mix(base.r, top.r, opacity),
        g: mix(base.g, top.g, opacity),
        b: mix(base.b, top.b, opacity),
        a: base.a,
    }
    .into()
}

fn mix_colours(from: u32, to: u32, progress: f32) -> Hsla {
    let (from, to): (Rgba, Rgba) = (rgba(from), rgba(to));
    Rgba {
        r: mix(from.r, to.r, progress),
        g: mix(from.g, to.g, progress),
        b: mix(from.b, to.b, progress),
        a: mix(from.a, to.a, progress),
    }
    .into()
}

/// Melli's colours, read from the theme when it renders.
#[derive(Debug, Clone, Copy)]
struct MelliColours {
    shadow: Hsla,
    body: u32,
    body_overcast: u32,
    ink: Hsla,
    outline: Hsla,
    mark: Hsla,
    blush: Hsla,
}

/// Draws one frame inside `bounds`, in the order of the web's SVG.
fn paint_melli(
    frame: &MelliFrame,
    colours: &MelliColours,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    let scale = f32::from(bounds.size.width) / VIEW_BOX;
    let view = Affine {
        a: scale,
        d: scale,
        ..Affine::IDENTITY
    }
    .then(Affine::translation(
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
    ));
    let (pose, loops) = (frame.pose, frame.loops);
    let mut painter = Painter { window };

    painter.ellipse(
        Affine::scale_about(loops.shadow_scale_x, 1.0, 128.0, 222.0).then(view),
        (128.0, 222.0),
        (54.0, 7.0),
        with_opacity(colours.shadow, loops.shadow_opacity),
    );
    // Behind the cloud, so each drop falls out of it.
    for (x, (fall, opacity)) in DROP_CENTRES.into_iter().zip(loops.drops) {
        painter.drop(
            Affine::translation(0.0, fall).then(view),
            x,
            with_opacity(colours.mark, pose.drops_opacity * opacity),
        );
    }

    let motion = Affine::scale_about(loops.motion_scale.0, loops.motion_scale.1, 128.0, 200.0)
        .then(Affine::translation(0.0, loops.motion_offset_y))
        .then(view);
    let puff = Affine::rotation_about(frame.puff_degrees, 128.0, 124.0).then(motion);
    for (x, y, radius) in PUFF_CIRCLES {
        painter.circle(puff, (x, y), radius + OUTLINE_WIDTH, colours.outline);
    }
    let body = mix_colours(colours.body, colours.body_overcast, pose.overcast);
    for (x, y, radius) in PUFF_CIRCLES {
        painter.circle(puff, (x, y), radius, body);
    }

    let face = Affine::translation(pose.face_offset.0, pose.face_offset.1).then(motion);
    let blush = with_opacity(colours.blush, pose.blush_opacity);
    painter.ellipse(face, (100.0, 144.0), (10.0, 6.0), blush);
    painter.ellipse(face, (156.0, 144.0), (10.0, 6.0), blush);
    // The pose squashes the eyes from their base; the blink from their middle.
    let eyes = Affine::scale_about(1.0, loops.blink_scale_y, 128.0, 124.0)
        .then(Affine::scale_about(1.0, pose.eyes_scale_y, 128.0, 136.0))
        .then(face);
    let eye_ink = with_opacity(colours.ink, pose.eyes_opacity);
    painter.pill(eyes, (110.0, 112.0), (12.0, 24.0), eye_ink);
    painter.pill(eyes, (134.0, 112.0), (12.0, 24.0), eye_ink);
    // The arc eyes lie wholly on the cloud: while they fade, their ink is blended over it.
    if pose.happy_eyes_opacity > 0.0 {
        let happy = blended_over(body, colours.ink, pose.happy_eyes_opacity);
        painter.opaque_quadratic_stroke(
            face,
            (109.0, 129.0),
            (116.0, 114.0),
            (123.0, 129.0),
            happy,
        );
        painter.opaque_quadratic_stroke(
            face,
            (133.0, 129.0),
            (140.0, 114.0),
            (147.0, 129.0),
            happy,
        );
    }
    if pose.closed_eyes_opacity > 0.0 {
        let closed = blended_over(body, colours.ink, pose.closed_eyes_opacity);
        painter.opaque_quadratic_stroke(
            face,
            (109.0, 124.0),
            (116.0, 132.0),
            (123.0, 124.0),
            closed,
        );
        painter.opaque_quadratic_stroke(
            face,
            (133.0, 124.0),
            (140.0, 132.0),
            (147.0, 124.0),
            closed,
        );
    }

    paint_marks(&mut painter, view, frame, colours);
}

/// The marks drawn above the cloud, on the host's surface: dots, sparkles, the crossed-out bell.
fn paint_marks(
    painter: &mut Painter<'_>,
    view: Affine,
    frame: &MelliFrame,
    colours: &MelliColours,
) {
    let (pose, loops) = (frame.pose, frame.loops);
    for (x, (rise, opacity)) in DOT_CENTRES.into_iter().zip(loops.dots) {
        painter.circle(
            Affine::translation(0.0, rise).then(view),
            (x, 44.0),
            8.0,
            with_opacity(colours.mark, pose.dots_opacity * opacity),
        );
    }
    // `M194 36Q197 55 216 58…` and `M58 34Q59.5 42.5 68 44…`.
    for ((centre, shape), (star_scale, turn)) in
        [((194.0, 58.0), (22.0, 3.0)), ((58.0, 44.0), (10.0, 1.5))]
            .into_iter()
            .zip(loops.stars)
    {
        let transform = Affine::rotation_about(turn, centre.0, centre.1)
            .then(Affine::scale_about(
                star_scale, star_scale, centre.0, centre.1,
            ))
            .then(view);
        painter.star(
            transform,
            centre,
            shape,
            with_opacity(colours.mark, pose.sparkle_opacity),
        );
    }
    let bell = with_opacity(colours.mark, pose.bell_opacity);
    painter.bell(view, bell);
    painter.line_stroke(view, (168.0, 44.0), (216.0, 92.0), bell);
}

/// Melli at `size` (a length token: `component.auth-card.melli-size`, the rail item…).
///
/// ```ignore
/// Melli::new("home-melli", MelliState::for_home(..), COMPONENT_RAIL_ITEM_SIZE).marks_on_accent(is_lit)
/// ```
#[derive(Debug, IntoElement)]
pub struct Melli {
    id: ElementId,
    state: MelliState,
    size: f32,
    is_still: bool,
    are_marks_on_accent: bool,
}

impl Melli {
    /// `id` keeps its clock between frames: give each Melli on screen its own.
    #[must_use]
    pub fn new(id: impl Into<ElementId>, state: MelliState, size: f32) -> Self {
        Self {
            id: id.into(),
            state,
            size,
            is_still: false,
            are_marks_on_accent: false,
        }
    }

    /// The still pose of the state even when motion is allowed (the gallery); reduced motion
    /// always gets it.
    #[must_use]
    pub fn still(mut self, is_still: bool) -> Self {
        self.is_still = is_still;
        self
    }

    /// Marks in ink, for a host that puts Melli on apricot (Home hovered, focused or selected).
    #[must_use]
    pub fn marks_on_accent(mut self, are_on_accent: bool) -> Self {
        self.are_marks_on_accent = are_on_accent;
        self
    }
}

/// Melli's frame rates. Its motion is slow: the fastest steady part, the puffs' turn, moves their
/// edge about 46 units in 14 s. At the rail's 48 px that is under 4 px a second, so 15 frames
/// keep each step under 0.3 px; larger Melli (sign-in, empty states) get 30. A frame of the
/// window costs the same whatever it draws, so the rate is what Melli costs.
const FRAMES_PER_SECOND: u32 = 30;
const SMALL_FRAMES_PER_SECOND: u32 = 15;
const LARGEST_SMALL_SIZE: f32 = crate::theme::tokens::COMPONENT_RAIL_ITEM_SIZE;

fn frame_interval(size: f32) -> Duration {
    let frames_per_second = if size <= LARGEST_SMALL_SIZE {
        SMALL_FRAMES_PER_SECOND
    } else {
        FRAMES_PER_SECOND
    };
    Duration::from_secs(1) / frames_per_second
}

/// What a host asks of its Melli; a change redraws it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct MelliLook {
    state: MelliState,
    size: f32,
    is_still: bool,
    are_marks_on_accent: bool,
}

/// One Melli as its own cached view, so its frames redraw it alone, not its host's other views.
struct MelliView {
    look: MelliLook,
    clock: MelliClock,
    /// When the frame after this one is due, on a steady cadence.
    next_frame_due: Option<Instant>,
}

#[cfg(test)]
thread_local! {
    /// Melli frames drawn on this test's thread.
    static DRAWN_FRAMES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// The clock every animated Melli shares: one timer, waking for the next Melli due. A Melli asks
/// again each time it is drawn; one that is no longer drawn (its window hidden or minimized,
/// which gets no frames, or the Melli gone) does not, and the timer stops by itself.
#[derive(Default)]
struct MelliTicker {
    /// Each Melli waiting for a frame, and when it is due.
    waiting: Vec<(WeakEntity<MelliView>, Instant)>,
    timer: Option<Task<()>>,
}

impl Global for MelliTicker {}

impl MelliTicker {
    /// Asks for a frame of `view` at `due`.
    fn request_frame(view: WeakEntity<MelliView>, due: Instant, cx: &mut App) {
        let ticker = cx.default_global::<Self>();
        ticker
            .waiting
            .retain(|(waiting, _)| waiting.entity_id() != view.entity_id());
        ticker.waiting.push((view, due));
        if ticker.timer.is_some() {
            return;
        }
        let timer = cx.spawn(async move |cx| {
            loop {
                let next_due = cx.update(|cx| {
                    let ticker = cx.global_mut::<Self>();
                    let next_due = ticker.waiting.iter().map(|(_, due)| *due).min();
                    if next_due.is_none() {
                        ticker.timer = None;
                    }
                    next_due
                });
                let Some(next_due) = next_due else {
                    return;
                };
                let executor = cx.background_executor().clone();
                executor
                    .timer(next_due.saturating_duration_since(executor.now()))
                    .await;
                cx.update(|cx| {
                    let now = cx.background_executor().now();
                    let ticker = cx.global_mut::<Self>();
                    let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut ticker.waiting)
                        .into_iter()
                        .partition(|(_, due)| *due <= now);
                    ticker.waiting = later;
                    for (view, _) in due {
                        // A Melli dropped since it asked has nothing to draw.
                        let _ = view.update(cx, |_, cx| cx.notify());
                    }
                });
            }
        });
        cx.global_mut::<Self>().timer = Some(timer);
    }
}

impl Render for MelliView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        // Melli is animated while it can be seen, focused or not (the owner's choice, §2.10),
        // under reduced motion only with "Always animate" (User settings, Accessibility).
        let is_still =
            self.look.is_still || !crate::pictures::is_picture_animation_wanted(true, cx);
        // The executor's clock, which tests can drive.
        let now = cx.background_executor().now();
        let frame = self.clock.frame(self.look.state, now, is_still);
        if is_still {
            self.next_frame_due = None;
        } else {
            let interval = frame_interval(self.look.size);
            // A steady cadence from the last due time, so late frames do not slow it down;
            // restarted after a pause rather than catching up.
            let due = self
                .next_frame_due
                .map(|due| due + interval)
                .filter(|due| *due > now)
                .unwrap_or(now + interval);
            self.next_frame_due = Some(due);
            MelliTicker::request_frame(cx.entity().downgrade(), due, cx);
        }
        #[cfg(test)]
        DRAWN_FRAMES.with(|frames| frames.set(frames.get() + 1));
        let colours = MelliColours {
            shadow: crate::theme::color(tokens.color_mascot_shadow),
            body: tokens.color_mascot_body,
            body_overcast: tokens.color_mascot_body_overcast,
            ink: crate::theme::color(tokens.color_mascot_ink),
            outline: crate::theme::color(tokens.color_mascot_outline),
            mark: crate::theme::color(if self.look.are_marks_on_accent {
                tokens.color_mascot_ink
            } else {
                tokens.color_mascot_mark
            }),
            blush: crate::theme::color(tokens.color_mascot_blush),
        };
        canvas(
            |_, _, _| {},
            move |bounds, (), window, _| paint_melli(&frame, &colours, bounds, window),
        )
        .size_full()
    }
}

impl RenderOnce for Melli {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let look = MelliLook {
            state: self.state,
            size: self.size,
            is_still: self.is_still,
            are_marks_on_accent: self.are_marks_on_accent,
        };
        let now = cx.background_executor().now();
        let view = window.use_keyed_state(self.id, cx, move |_, _| MelliView {
            look,
            clock: MelliClock::new(look.state, now),
            next_frame_due: None,
        });
        view.update(cx, |view, cx| {
            if view.look != look {
                view.look = look;
                cx.notify();
            }
        });
        // Theme changes refresh the whole window, which redraws cached views too.
        view.cached(
            StyleRefinement::default()
                .flex_none()
                .size(length(self.size)),
        )
    }
}

#[cfg(test)]
mod tests {
    use gpui::ParentElement as _;

    use super::*;

    fn close(left: f32, right: f32) -> bool {
        (left - right).abs() < 1e-3
    }

    #[test]
    fn every_loop_starts_and_ends_at_rest() {
        for state in MelliState::ALL {
            let start = Loops::at(state, Duration::ZERO);
            assert!(close(start.motion_offset_y, 0.0), "{}", state.name());
            assert!(close(start.motion_scale.0, 1.0) && close(start.motion_scale.1, 1.0));
            assert!(close(start.blink_scale_y, 1.0));
        }
    }

    #[test]
    fn idle_floats_eight_units_at_half_its_loop() {
        let half = Duration::from_millis(COMPONENT_MELLI_FLOAT_DURATION.0 / 2);
        let loops = Loops::at(MelliState::Idle, half);
        assert!(close(loops.motion_offset_y, -8.0));
        assert!(close(loops.shadow_scale_x, 0.82));
    }

    #[test]
    fn every_loop_is_the_same_after_a_month_of_periods() {
        // A common multiple of every loop's duration (float, blink, wait, rain, hop, sleep).
        let common_period_ms: u64 = 2_576_000;
        for duration in [
            COMPONENT_MELLI_FLOAT_DURATION,
            COMPONENT_MELLI_BLINK_DURATION,
            COMPONENT_MELLI_WAIT_DURATION,
            COMPONENT_MELLI_RAIN_DURATION,
            COMPONENT_MELLI_HOP_DURATION,
            COMPONENT_MELLI_SLEEP_DURATION,
        ] {
            assert_eq!(common_period_ms % duration.0, 0);
        }
        for state in MelliState::ALL {
            for early_ms in [700, 1_234, 2_999] {
                let early = Loops::at(state, Duration::from_millis(early_ms));
                let month_later = Loops::at(
                    state,
                    Duration::from_millis(early_ms + common_period_ms * 1_000),
                );
                assert_eq!(early, month_later, "{} drifts", state.name());
            }
        }
    }

    #[test]
    fn connecting_dots_rise_in_turn() {
        let wait = COMPONENT_MELLI_WAIT_DURATION.0;
        // The first dot peaks at 30 % of its loop, while the third has not started yet.
        let loops = Loops::at(MelliState::Connecting, Duration::from_millis(wait * 3 / 10));
        assert!(close(loops.dots[0].0, -6.0));
        assert!(close(loops.dots[0].1, 1.0));
        assert_eq!(loops.dots[2], (0.0, 1.0), "a delayed dot shows its rest");
    }

    #[test]
    fn a_state_change_eases_the_pose_and_a_still_melli_jumps() {
        let start = Instant::now();
        let mut clock = MelliClock::new(MelliState::Idle, start);
        let changed = start + Duration::from_secs(5);
        let first = clock.frame(MelliState::ConnectionLost, changed, false);
        assert!(
            close(first.pose.overcast, 0.0),
            "the change starts from the shown pose"
        );
        let halfway = clock.frame(
            MelliState::ConnectionLost,
            changed + Duration::from_millis(COMPONENT_MELLI_STATE_CHANGE_DURATION.0 / 2),
            false,
        );
        assert!(halfway.pose.overcast > 0.0 && halfway.pose.overcast < 1.0);
        let done = clock.frame(
            MelliState::ConnectionLost,
            changed + Duration::from_secs(1),
            false,
        );
        assert_eq!(done.pose, MelliState::ConnectionLost.pose());

        let mut still = MelliClock::new(MelliState::Idle, start);
        let frame = still.frame(MelliState::Connecting, changed, true);
        assert_eq!(frame.pose, MelliState::Connecting.pose());
        assert_eq!(frame.loops, Loops::REST);
    }

    #[test]
    fn the_puffs_keep_their_angle_while_the_connection_is_lost() {
        let start = Instant::now();
        let mut clock = MelliClock::new(MelliState::Idle, start);
        let quarter = Duration::from_millis(COMPONENT_MELLI_PUFF_SPIN_DURATION.0 / 4);
        let lost = clock.frame(MelliState::ConnectionLost, start + quarter, false);
        assert!(close(lost.puff_degrees, 90.0));
        let later = clock.frame(MelliState::ConnectionLost, start + quarter * 3, false);
        assert!(close(later.puff_degrees, 90.0), "paused");
        let back = clock.frame(MelliState::Idle, start + quarter * 3, false);
        assert!(
            close(back.puff_degrees, 90.0),
            "resumes from where it stopped"
        );
        let turned = clock.frame(MelliState::Idle, start + quarter * 4, false);
        assert!(close(turned.puff_degrees, 180.0));
    }

    #[test]
    fn the_home_button_shows_connection_trouble_first() {
        use ConnectionStatus::{Connected, Connecting, Reconnecting};
        assert_eq!(
            MelliState::for_home(Reconnecting, true, true),
            MelliState::ConnectionLost
        );
        assert_eq!(
            MelliState::for_home(Connecting, false, true),
            MelliState::Connecting
        );
        assert_eq!(
            MelliState::for_home(Connected, true, true),
            MelliState::DoNotDisturb,
            "quiet even with unread messages"
        );
        assert_eq!(
            MelliState::for_home(Connected, false, true),
            MelliState::Notification
        );
        assert_eq!(
            MelliState::for_home(Connected, false, false),
            MelliState::Idle
        );
    }

    struct MelliHost;

    impl Render for MelliHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div()
                .size_full()
                .child(Melli::new(
                    "rail-melli",
                    MelliState::Idle,
                    crate::theme::tokens::COMPONENT_RAIL_ITEM_SIZE,
                ))
                .child(Melli::new(
                    "sign-in-melli",
                    MelliState::Connecting,
                    crate::theme::tokens::COMPONENT_AUTH_CARD_MELLI_SIZE,
                ))
        }
    }

    fn drawn_frames() -> u32 {
        DRAWN_FRAMES.with(std::cell::Cell::get)
    }

    /// Runs the fake clock for exactly one second, in steps finer than any frame.
    fn run_one_second(cx: &mut gpui::VisualTestContext) {
        const STEP: Duration = Duration::from_millis(5);
        for _ in 0..(Duration::from_secs(1).as_millis() / STEP.as_millis()) {
            cx.executor().advance_clock(STEP);
            cx.run_until_parked();
        }
    }

    #[gpui::test]
    fn one_timer_draws_each_melli_at_its_rate_and_stops_when_still(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::theme::ActiveTheme::for_appearance(
                gpui::WindowAppearance::Dark,
            ));
        });
        let (host, cx) = cx.add_window_view(|_, _| MelliHost);
        cx.run_until_parked();
        let before = drawn_frames();
        run_one_second(cx);
        assert_eq!(
            drawn_frames() - before,
            SMALL_FRAMES_PER_SECOND + FRAMES_PER_SECOND,
            "the rail's Melli at its small rate, the sign-in one at the full rate"
        );

        cx.update(|_, cx| cx.set_reduce_motion(true));
        host.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        let still = drawn_frames();
        run_one_second(cx);
        assert!(
            drawn_frames() - still <= 2,
            "a still Melli asks for no frame: at most the ones already due"
        );
        assert!(
            cx.update(|_, cx| cx.global::<MelliTicker>().timer.is_none()),
            "the timer stops by itself"
        );

        // "Always animate" wins over the system's reduced motion.
        cx.update(|_, cx| {
            crate::preferences::change_preferences(cx, |preferences| {
                preferences.picture_animation = crate::preferences::PictureAnimationChoice::Always;
            });
            // As `start` does when the choice changes.
            cx.refresh_windows();
        });
        cx.run_until_parked();
        let always = drawn_frames();
        run_one_second(cx);
        assert_eq!(
            drawn_frames() - always,
            SMALL_FRAMES_PER_SECOND + FRAMES_PER_SECOND,
            "Melli plays again under reduced motion"
        );
    }

    #[test]
    fn transforms_compose_in_css_order() {
        let pivot = Affine::scale_about(2.0, 2.0, 10.0, 10.0);
        let mapped = pivot.apply(10.0, 10.0);
        assert!(close(f32::from(mapped.x), 10.0), "the origin stays put");
        let turned = Affine::rotation_about(90.0, 0.0, 0.0).apply(1.0, 0.0);
        assert!(close(f32::from(turned.x), 0.0) && close(f32::from(turned.y), 1.0));
        let moved = Affine::translation(5.0, 0.0)
            .then(Affine::scale_about(2.0, 2.0, 0.0, 0.0))
            .apply(1.0, 0.0);
        assert!(close(f32::from(moved.x), 12.0));
    }

    #[test]
    fn circles_become_quads_only_while_they_stay_round() {
        let turned = Affine::rotation_about(30.0, 128.0, 124.0)
            .then(Affine::scale_about(0.5, 0.5, 0.0, 0.0));
        assert!(turned.circle_scale().is_some_and(|scale| close(scale, 0.5)));
        assert_eq!(turned.upright_scale(), None, "a turned pill is a path");
        let squashed = Affine::scale_about(1.02, 0.98, 128.0, 200.0);
        assert_eq!(squashed.circle_scale(), None);
        assert_eq!(squashed.upright_scale(), None);
        assert_eq!(
            Affine::translation(3.0, -5.0).upright_scale(),
            Some(1.0),
            "a moved pill is a quad"
        );
    }
}
