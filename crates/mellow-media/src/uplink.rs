//! How many simulcast layers this client's uplink carries, from the loss the server reports on
//! it each second (transport-wide congestion feedback, `PeerStats.egress_loss_fraction`). A
//! browser's congestion control drops `h` then `l` on a slow uplink; this does the same, with the
//! thresholds the server applies to its own sends since #204 (`domain/video_rules.rs`), so a
//! large screen cannot flood a slow uplink and drown the voice sharing it.

use std::time::{Duration, Instant};

use mellow_protocol::SimulcastLayer;

/// Loss over the last second, in percent of the packets sent, from which the top layer goes.
const LOSS_PERCENT_TO_GO_DOWN: u32 = 10;
/// Loss under which the uplink counts as clear; between the two, the layers hold.
const LOSS_PERCENT_OF_A_CLEAR_LINK: u32 = 2;
/// How long the uplink stays clear before a layer comes back, at first...
const FIRST_WAIT_BEFORE_RAISING: Duration = Duration::from_secs(5);
/// ...doubled each time a raise meets loss within `TRIAL_OF_A_RAISE`, up to this, so an uplink
/// at its limit tries rarely...
const LONGEST_WAIT_BEFORE_RAISING: Duration = Duration::from_secs(60);
const TRIAL_OF_A_RAISE: Duration = Duration::from_secs(10);
/// ...and back to the first wait once a raise held this long.
const HELD_RAISE: Duration = Duration::from_secs(30);

/// A loss fraction (0 to 1) as a whole percentage.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) fn loss_percent_of(fraction: f32) -> u32 {
    // Clamped to 0..=100 first: the cast cannot truncate or lose a sign.
    (fraction.clamp(0.0, 1.0) * 100.0).round() as u32
}

/// The highest layer the uplink carries now.
#[derive(Debug, Clone)]
pub(crate) struct UplinkLayerCeiling {
    ceiling: SimulcastLayer,
    clear_since: Option<Instant>,
    raised_at: Option<Instant>,
    wait_before_raising: Duration,
}

impl Default for UplinkLayerCeiling {
    fn default() -> Self {
        Self {
            ceiling: SimulcastLayer::High,
            clear_since: None,
            raised_at: None,
            wait_before_raising: FIRST_WAIT_BEFORE_RAISING,
        }
    }
}

const fn layer_below(layer: SimulcastLayer) -> SimulcastLayer {
    match layer {
        SimulcastLayer::High => SimulcastLayer::Low,
        SimulcastLayer::Low | SimulcastLayer::Quarter => SimulcastLayer::Quarter,
    }
}

const fn layer_above(layer: SimulcastLayer) -> SimulcastLayer {
    match layer {
        SimulcastLayer::Quarter => SimulcastLayer::Low,
        SimulcastLayer::Low | SimulcastLayer::High => SimulcastLayer::High,
    }
}

impl UplinkLayerCeiling {
    pub(crate) fn ceiling(&self) -> SimulcastLayer {
        self.ceiling
    }

    /// The ceiling after a second's `loss_percent` (`None`: no feedback covered it, so nothing
    /// to judge on).
    pub(crate) fn after_report(
        &mut self,
        loss_percent: Option<u32>,
        now: Instant,
    ) -> SimulcastLayer {
        let elapsed_since = |at: Instant| now.saturating_duration_since(at);
        let Some(loss_percent) = loss_percent else {
            return self.ceiling;
        };
        if loss_percent >= LOSS_PERCENT_TO_GO_DOWN {
            self.clear_since = None;
            if self
                .raised_at
                .take()
                .is_some_and(|at| elapsed_since(at) < TRIAL_OF_A_RAISE)
            {
                self.wait_before_raising =
                    (self.wait_before_raising * 2).min(LONGEST_WAIT_BEFORE_RAISING);
            }
            self.ceiling = layer_below(self.ceiling);
            return self.ceiling;
        }
        if loss_percent >= LOSS_PERCENT_OF_A_CLEAR_LINK {
            self.clear_since = None;
            return self.ceiling;
        }
        if self
            .raised_at
            .is_some_and(|at| elapsed_since(at) >= HELD_RAISE)
        {
            self.raised_at = None;
            self.wait_before_raising = FIRST_WAIT_BEFORE_RAISING;
        }
        let since = *self.clear_since.get_or_insert(now);
        if self.ceiling != SimulcastLayer::High && elapsed_since(since) >= self.wait_before_raising
        {
            self.ceiling = layer_above(self.ceiling);
            self.clear_since = Some(now);
            self.raised_at = Some(now);
        }
        self.ceiling
    }
}

/// The layers to encode: those the server asks for, up to the uplink's `ceiling`. When the
/// ceiling is below all of them, the lowest asked for still goes: the server always asks for the
/// lowest layer while a source is on, so this is only a guard.
pub(crate) fn layers_within(
    asked: &[SimulcastLayer],
    ceiling: SimulcastLayer,
) -> Vec<SimulcastLayer> {
    let within: Vec<SimulcastLayer> = asked
        .iter()
        .copied()
        .filter(|layer| *layer <= ceiling)
        .collect();
    if within.is_empty() {
        asked.iter().copied().min().into_iter().collect()
    } else {
        within
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seconds(start: Instant, count: u64) -> Instant {
        start + Duration::from_secs(count)
    }

    #[test]
    fn loss_drops_the_top_layer_then_the_middle_one_never_the_lowest() {
        let start = Instant::now();
        let mut uplink = UplinkLayerCeiling::default();
        assert_eq!(uplink.after_report(Some(0), start), SimulcastLayer::High);
        assert_eq!(
            uplink.after_report(Some(5), seconds(start, 1)),
            SimulcastLayer::High,
            "holds"
        );
        assert_eq!(
            uplink.after_report(Some(12), seconds(start, 2)),
            SimulcastLayer::Low
        );
        assert_eq!(
            uplink.after_report(Some(30), seconds(start, 3)),
            SimulcastLayer::Quarter
        );
        assert_eq!(
            uplink.after_report(Some(30), seconds(start, 4)),
            SimulcastLayer::Quarter
        );
        assert_eq!(
            uplink.after_report(None, seconds(start, 5)),
            SimulcastLayer::Quarter
        );
    }

    #[test]
    fn a_clear_uplink_gets_a_layer_back_and_a_failed_raise_waits_longer() {
        let start = Instant::now();
        let mut uplink = UplinkLayerCeiling::default();
        uplink.after_report(Some(20), start);
        // Clear from second 1: the layer comes back once it stayed clear for 5 s.
        for second in 1..6 {
            assert_eq!(
                uplink.after_report(Some(0), seconds(start, second)),
                SimulcastLayer::Low
            );
        }
        assert_eq!(
            uplink.after_report(Some(1), seconds(start, 6)),
            SimulcastLayer::High
        );
        // Loss right after the raise: down again, and the next raise waits 10 s.
        assert_eq!(
            uplink.after_report(Some(15), seconds(start, 7)),
            SimulcastLayer::Low
        );
        for second in 8..18 {
            assert_eq!(
                uplink.after_report(Some(0), seconds(start, second)),
                SimulcastLayer::Low
            );
        }
        assert_eq!(
            uplink.after_report(Some(0), seconds(start, 18)),
            SimulcastLayer::High
        );
    }

    #[test]
    fn the_ceiling_trims_the_layers_the_server_asks_for() {
        let all = [
            SimulcastLayer::Quarter,
            SimulcastLayer::Low,
            SimulcastLayer::High,
        ];
        assert_eq!(layers_within(&all, SimulcastLayer::High), all);
        assert_eq!(
            layers_within(&all, SimulcastLayer::Low),
            [SimulcastLayer::Quarter, SimulcastLayer::Low]
        );
        assert_eq!(
            layers_within(
                &[SimulcastLayer::Low, SimulcastLayer::High],
                SimulcastLayer::Quarter
            ),
            [SimulcastLayer::Low]
        );
    }

    #[test]
    fn loss_fractions_become_whole_percentages() {
        assert_eq!(loss_percent_of(0.104), 10);
        assert_eq!(loss_percent_of(1.5), 100);
        assert_eq!(loss_percent_of(f32::NAN), 0);
    }
}
