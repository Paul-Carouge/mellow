//! Rules of cameras in voice channels (ADR-0035) that need no I/O: which simulcast layer a tile
//! receives, when a watcher switches layers, how often a camera is asked for a keyframe, which
//! layers a camera must keep sending, which layers fit a watcher's bandwidth, and how many
//! cameras may be on at once. The media loop and
//! the voice service apply them; the permission itself is `domain::permissions::may_send_video`.

use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap};
use std::hash::Hash;
use std::time::{Duration, Instant};

use mellow_protocol::{SimulcastLayer, Snowflake, VideoCodec, VideoTileSize, VoiceVideoRefusal};

use crate::domain::h264_keyframes::constrained_baseline_picture_size;

/// Cameras on at once in one voice channel. The owner's server lost frames with 25 cameras and
/// default socket buffers (ADR-0035): raised once a run with larger buffers shows it holds more.
pub(crate) const MAXIMUM_CAMERAS_PER_VOICE_CHANNEL: usize = 12;
/// Cameras on at once on the whole instance: bounds what the server receives (ADR-0035).
pub(crate) const MAXIMUM_CAMERAS_PER_INSTANCE: usize = 25;
/// A camera is asked for a keyframe of one layer at most this often, however many watchers
/// switch to it or lose packets.
pub(crate) const MINIMUM_INTERVAL_BETWEEN_KEYFRAME_REQUESTS: Duration = Duration::from_millis(300);
/// One watcher's own keyframe requests (after a loss) reach a camera at most this often, so a
/// watcher sending them on purpose cannot fill every other watcher's stream with keyframes.
pub(crate) const MINIMUM_INTERVAL_BETWEEN_KEYFRAME_REQUESTS_OF_A_WATCHER: Duration =
    Duration::from_secs(1);
/// A camera turned on that sends no frame for this long is turned off, so a camera that only
/// holds a place counts no more toward the cameras allowed (ADR-0035). Long enough for a browser
/// to start a camera and connect.
pub(crate) const MAXIMUM_SILENCE_OF_A_CAMERA: Duration = Duration::from_secs(10);
/// A layer asked of a camera for this long that sent no frame for this long counts as gone: its
/// sender's browser dropped it for a short uplink, and its watchers fall back to the next lower
/// layer still arriving (ADR-0035).
pub(crate) const SILENCE_OF_A_LOST_LAYER: Duration = Duration::from_secs(1);

/// Whether a camera's layer is gone: asked of the camera since `requested_since` and last heard at
/// `heard_at`, both at least `SILENCE_OF_A_LOST_LAYER` ago, while its lowest layer, last heard at
/// `lowest_heard_at`, still flows. A layer not asked yet is not gone: it is about to be asked, and
/// its first frames take a moment. Every layer silent together is not a lost layer but a picture
/// that does not change (a static screen sends a frame only now and then): nothing is gone.
#[must_use]
pub(crate) fn is_layer_gone(
    requested_since: Option<Instant>,
    heard_at: Option<Instant>,
    lowest_heard_at: Option<Instant>,
    now: Instant,
) -> bool {
    let is_old = |at: Instant| now.saturating_duration_since(at) >= SILENCE_OF_A_LOST_LAYER;
    lowest_heard_at.is_some_and(|at| !is_old(at))
        && requested_since.is_some_and(is_old)
        && heard_at.is_none_or(is_old)
}

/// The layer a tile allocated `allocated` receives: the highest at most `allocated` that is not
/// gone, and the lowest when every one is (a camera silent for long is turned off anyway).
#[must_use]
pub(crate) fn arriving_layer_at_most(
    allocated: SimulcastLayer,
    is_gone: impl Fn(SimulcastLayer) -> bool,
) -> SimulcastLayer {
    let mut layer = allocated;
    while is_gone(layer)
        && let Some(lower) = layer_below(layer)
    {
        layer = lower;
    }
    layer
}

/// The codec a camera m-line sends, from those it negotiated (ADR-0052): VP8 whenever it is among
/// them, since every client decodes it, else H.264. A client that wants to send H.264 offers only
/// H.264 on its camera line.
#[must_use]
pub(crate) fn camera_codec_of(negotiated: &[VideoCodec]) -> Option<VideoCodec> {
    [VideoCodec::Vp8, VideoCodec::H264]
        .into_iter()
        .find(|codec| negotiated.contains(codec))
}

/// The codecs every one of a participant's video receive m-lines negotiated, so any free slot can
/// carry a camera in one of them (ADR-0052). Nothing without receive lines.
#[must_use]
pub(crate) fn codecs_decoded_on_every_line(lines: &[Vec<VideoCodec>]) -> Vec<VideoCodec> {
    let Some((first, others)) = lines.split_first() else {
        return Vec::new();
    };
    first
        .iter()
        .filter(|codec| others.iter().all(|line| line.contains(codec)))
        .copied()
        .collect()
}

/// The layer that fits a tile shown at `size` (ADR-0035).
#[must_use]
pub(crate) const fn layer_for_tile(size: VideoTileSize) -> SimulcastLayer {
    match size {
        VideoTileSize::Large => SimulcastLayer::High,
        VideoTileSize::Medium => SimulcastLayer::Low,
        VideoTileSize::Small => SimulcastLayer::Quarter,
    }
}

/// Why `user_id` may not turn a camera on in `channel_id`, if they may not. `cameras_on` names
/// every camera on the instance as (owner, channel); the asker's own is not counted.
pub(crate) fn missing_room_for_a_camera(
    cameras_on: impl IntoIterator<Item = (Snowflake, Snowflake)>,
    user_id: Snowflake,
    channel_id: Snowflake,
) -> Option<VoiceVideoRefusal> {
    let (mut cameras_in_channel, mut cameras_on_instance) = (0_usize, 0_usize);
    for (owner_id, camera_channel_id) in cameras_on {
        if owner_id != user_id {
            cameras_on_instance += 1;
            cameras_in_channel += usize::from(camera_channel_id == channel_id);
        }
    }
    (cameras_in_channel >= MAXIMUM_CAMERAS_PER_VOICE_CHANNEL
        || cameras_on_instance >= MAXIMUM_CAMERAS_PER_INSTANCE)
        .then_some(VoiceVideoRefusal::TooManyCameras)
}

/// What a layer costs a watcher's link at least, in bits per second: its encoder's target
/// (ADR-0035). A layer arriving faster, such as a 1440p screen's full layer, costs what it
/// measures instead (`LayerRate`).
#[must_use]
pub(crate) const fn nominal_bits_per_second_of(layer: SimulcastLayer) -> u64 {
    match layer {
        SimulcastLayer::Quarter => 150_000,
        SimulcastLayer::Low => 500_000,
        SimulcastLayer::High => 1_500_000,
    }
}

/// Loss over a watcher's last second, in percent of the packets sent to it, from which its link
/// counts as congested: its tiles go down to what gets through (ADR-0035, as built 2026-11-03).
pub(crate) const LOSS_PERCENT_TO_GO_DOWN: u32 = 10;
/// Loss under which a watcher's link counts as clear: its tiles may go back up. Between the two,
/// they hold.
pub(crate) const LOSS_PERCENT_OF_A_CLEAR_LINK: u32 = 2;
/// How long a link stays clear before its tiles try one step up, at first...
pub(crate) const FIRST_WAIT_BEFORE_RAISING: Duration = Duration::from_secs(5);
/// ...doubled each time a step up meets loss within `TRIAL_OF_A_RAISE`, up to this, so a link at
/// its limit tries rarely...
pub(crate) const LONGEST_WAIT_BEFORE_RAISING: Duration = Duration::from_secs(60);
pub(crate) const TRIAL_OF_A_RAISE: Duration = Duration::from_secs(10);
/// ...and back to the first wait once a step up held this long.
pub(crate) const HELD_RAISE: Duration = Duration::from_secs(30);
/// The share of what gets through a lossy link its tiles are fitted into.
const PERCENT_OF_WHAT_GETS_THROUGH: u64 = 90;

/// A loss fraction (0 to 1) as a whole percentage.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) fn loss_percent_of(fraction: f32) -> u32 {
    // Clamped to 0..=100 first: the cast cannot truncate or lose a sign.
    (fraction.clamp(0.0, 1.0) * 100.0).round() as u32
}

/// A watcher's last second: the share of packets sent to it that were lost, from its congestion
/// feedback (`None` when no feedback covers the second: none negotiated, nothing sent, or
/// feedback late behind a deep queue), and what its tiles received.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LinkReport {
    pub(crate) loss_percent: Option<u32>,
    pub(crate) delivered_bits_per_second: u64,
}

/// A watcher's share of the instance's upload budget, in bits per second, among `watchers`
/// (those watching a camera of their own channel).
#[must_use]
pub(crate) fn fair_share_of_upload(video_upload_bits_per_second: u64, watchers: usize) -> u64 {
    let watchers = u64::try_from(watchers.max(1)).unwrap_or(u64::MAX);
    video_upload_bits_per_second / watchers
}

/// What a layer costs a watcher, from what it was `measured` to send: at least its target, and at
/// most its byte budget (`maximum_bytes_per_second_of`), so a camera cannot make its layers look
/// costlier than they may be and push other tiles down.
#[must_use]
pub(crate) fn layer_cost(layer: SimulcastLayer, measured_bits_per_second: u64) -> u64 {
    let ceiling = u64::from(maximum_bytes_per_second_of(layer)).saturating_mul(8);
    measured_bits_per_second
        .min(ceiling)
        .max(nominal_bits_per_second_of(layer))
}

/// What each layer of a camera costs a watcher, in bits per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LayerCosts {
    pub(crate) quarter: u64,
    pub(crate) low: u64,
    pub(crate) high: u64,
}

impl LayerCosts {
    #[must_use]
    pub(crate) const fn of(self, layer: SimulcastLayer) -> u64 {
        match layer {
            SimulcastLayer::Quarter => self.quarter,
            SimulcastLayer::Low => self.low,
            SimulcastLayer::High => self.high,
        }
    }
}

/// One tile of a watcher, in the order the watcher lists them: the layer its size asks for and
/// what its camera's layers cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TileDemand {
    pub(crate) asked: SimulcastLayer,
    pub(crate) costs: LayerCosts,
}

/// The layer under `layer`; under the lowest, the tile is paused (`None`).
const fn layer_below(layer: SimulcastLayer) -> Option<SimulcastLayer> {
    match layer {
        SimulcastLayer::High => Some(SimulcastLayer::Low),
        SimulcastLayer::Low => Some(SimulcastLayer::Quarter),
        SimulcastLayer::Quarter => None,
    }
}

fn cost_of(tiles: &[TileDemand], layers: &[Option<SimulcastLayer>]) -> u64 {
    tiles
        .iter()
        .zip(layers)
        .filter_map(|(tile, layer)| layer.map(|layer| tile.costs.of(layer)))
        .sum()
}

/// The step after `layers` on the tiles' chain: the smallest tile goes down one layer before the
/// larger ones, the last listed first among equals; once every tile is at the lowest layer, one is
/// paused in the same order. `None` once every tile is paused.
fn lowered_once(
    tiles: &[TileDemand],
    layers: &[Option<SimulcastLayer>],
) -> Option<Vec<Option<SimulcastLayer>>> {
    let index = layers
        .iter()
        .zip(tiles)
        .enumerate()
        .filter_map(|(index, (layer, tile))| {
            layer.map(|layer| (layer == SimulcastLayer::Quarter, tile.asked, Reverse(index)))
        })
        .min()
        .map(|(_, _, Reverse(index))| index)?;
    let mut lowered = layers.to_vec();
    let layer = lowered.get_mut(index)?;
    *layer = layer.and_then(layer_below);
    Some(lowered)
}

/// The layer each tile gets within `budget` bits per second, `None` for a paused tile
/// (ADR-0035): the first step of the tiles' chain (`lowered_once`), from the layers asked, that
/// fits. The steps do not depend on the budget, so a larger budget never gives any tile a lower
/// layer.
#[must_use]
pub(crate) fn fit_tiles(tiles: &[TileDemand], budget: u64) -> Vec<Option<SimulcastLayer>> {
    let mut layers: Vec<Option<SimulcastLayer>> =
        tiles.iter().map(|tile| Some(tile.asked)).collect();
    while cost_of(tiles, &layers) > budget {
        let Some(lowered) = lowered_once(tiles, &layers) else {
            break;
        };
        layers = lowered;
    }
    layers
}

/// The step before `current` on the tiles' chain, `None` at its top or off it.
fn step_above(
    tiles: &[TileDemand],
    current: &[Option<SimulcastLayer>],
) -> Option<Vec<Option<SimulcastLayer>>> {
    let mut above = None;
    let mut layers: Vec<Option<SimulcastLayer>> =
        tiles.iter().map(|tile| Some(tile.asked)).collect();
    while layers != current {
        let lowered = lowered_once(tiles, &layers)?;
        above = Some(std::mem::replace(&mut layers, lowered));
    }
    above
}

/// Each tile's lower layer of the two.
fn lower_of_each(
    first: &[Option<SimulcastLayer>],
    second: &[Option<SimulcastLayer>],
) -> Vec<Option<SimulcastLayer>> {
    first
        .iter()
        .zip(second)
        .map(|(a, b)| (*a).min(*b))
        .collect()
}

/// How a watcher's tiles follow its link (ADR-0035, as built 2026-11-03), from the loss its
/// congestion feedback reports each second rather than a bandwidth estimate: an estimate follows
/// what flows, so a still screen dragged it low and kept a sharp 1440p screen blurred on a link
/// that carried it easily. Down at once to what gets through when 10 % or more is lost; one step up
/// after the link stayed clear (under 2 %) for a wait that doubles each time a step up meets loss.
#[derive(Debug)]
pub(crate) struct LayerAllocation {
    clear_since: Option<Instant>,
    raised_at: Option<Instant>,
    wait_before_raising: Duration,
}

impl Default for LayerAllocation {
    fn default() -> Self {
        Self {
            clear_since: None,
            raised_at: None,
            wait_before_raising: FIRST_WAIT_BEFORE_RAISING,
        }
    }
}

impl LayerAllocation {
    /// The layers the tiles get after a `report`, given the `current` ones (as many as `tiles`),
    /// never beyond the watcher's `fair_share` of the instance's upload.
    pub(crate) fn next_layers(
        &mut self,
        tiles: &[TileDemand],
        current: &[Option<SimulcastLayer>],
        report: LinkReport,
        fair_share: u64,
        now: Instant,
    ) -> Vec<Option<SimulcastLayer>> {
        let elapsed_since = |at: Instant| now.saturating_duration_since(at);
        // More watchers leave each one a smaller share.
        let current = lower_of_each(current, &fit_tiles(tiles, fair_share));
        let loss_percent = match report.loss_percent {
            Some(loss_percent) => loss_percent,
            // Video flowed but no feedback covers it: late behind a deep queue, a sign of
            // congestion rather than a clear link. Nothing to raise on.
            None if report.delivered_bits_per_second > 0 => {
                self.clear_since = None;
                return current;
            }
            // Nothing flowed (tiles paused, no feedback negotiated): no sign against going up.
            None => 0,
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
            // What flowed says what the link carries only when the tiles sent about what they
            // cost: a still screen sends little, and a tile just switched has sent nothing yet.
            // Then one step down at a time.
            if report.delivered_bits_per_second < cost_of(tiles, &current) / 2 {
                return lowered_once(tiles, &current).unwrap_or(current);
            }
            let kept_percent = u64::from(100 - loss_percent.min(100));
            let gets_through = report
                .delivered_bits_per_second
                .saturating_mul(kept_percent)
                / 100
                * PERCENT_OF_WHAT_GETS_THROUGH
                / 100;
            return lower_of_each(&current, &fit_tiles(tiles, gets_through));
        }
        if loss_percent >= LOSS_PERCENT_OF_A_CLEAR_LINK {
            self.clear_since = None;
            return current;
        }
        if self
            .raised_at
            .is_some_and(|at| elapsed_since(at) >= HELD_RAISE)
        {
            self.raised_at = None;
            self.wait_before_raising = FIRST_WAIT_BEFORE_RAISING;
        }
        let since = *self.clear_since.get_or_insert(now);
        if elapsed_since(since) < self.wait_before_raising {
            return current;
        }
        let Some(above) =
            step_above(tiles, &current).filter(|above| cost_of(tiles, above) <= fair_share)
        else {
            return current;
        };
        self.clear_since = Some(now);
        self.raised_at = Some(now);
        above
    }
}

/// What a watcher received of one tile over the last second, with no memory: 0 once nothing has
/// flowed for a second, unlike `LayerRate`, so a congestion check sees what flows now.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RecentRate {
    window_started_at: Instant,
    bytes_in_window: u64,
    bytes_in_previous_window: u64,
}

impl RecentRate {
    const WINDOW: Duration = Duration::from_secs(1);

    #[must_use]
    pub(crate) const fn new(now: Instant) -> Self {
        Self {
            window_started_at: now,
            bytes_in_window: 0,
            bytes_in_previous_window: 0,
        }
    }

    pub(crate) fn record(&mut self, bytes: usize, now: Instant) {
        let elapsed = now.saturating_duration_since(self.window_started_at);
        if elapsed >= 2 * Self::WINDOW {
            self.bytes_in_previous_window = 0;
            self.bytes_in_window = 0;
            self.window_started_at = now;
        } else if elapsed >= Self::WINDOW {
            self.bytes_in_previous_window = self.bytes_in_window;
            self.bytes_in_window = 0;
            self.window_started_at += Self::WINDOW;
        }
        self.bytes_in_window = self
            .bytes_in_window
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
    }

    /// The bits per second of the last whole second; 0 once nothing flowed for a second.
    #[must_use]
    pub(crate) fn bits_per_second(&self, now: Instant) -> u64 {
        let elapsed = now.saturating_duration_since(self.window_started_at);
        let bytes = if elapsed < Self::WINDOW {
            self.bytes_in_previous_window
        } else if elapsed < 2 * Self::WINDOW {
            self.bytes_in_window
        } else {
            0
        };
        bytes.saturating_mul(8)
    }
}

/// The bytes a camera layer sent over its last whole second in which it sent anything, so a layer
/// is costed at what it really sends (a 1440p screen's full layer is several times a camera's).
/// A layer paused because nobody receives it keeps its last measure: costed at its target instead,
/// a 1440p screen would look cheap again, be asked for, and go down again, forever.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LayerRate {
    window_started_at: Instant,
    bytes_in_window: u64,
    bytes_in_last_window: u64,
}

impl LayerRate {
    const WINDOW: Duration = Duration::from_secs(1);

    #[must_use]
    pub(crate) const fn new(now: Instant) -> Self {
        Self {
            window_started_at: now,
            bytes_in_window: 0,
            bytes_in_last_window: 0,
        }
    }

    /// A frame of `bytes` arrived from the camera.
    pub(crate) fn record(&mut self, bytes: usize, now: Instant) {
        let elapsed = now.saturating_duration_since(self.window_started_at);
        if elapsed >= Self::WINDOW {
            // Only a window the layer kept sending after is whole: the one in which it stopped
            // holds a fraction of a second, which would make the layer look cheap.
            if elapsed < 2 * Self::WINDOW && self.bytes_in_window > 0 {
                self.bytes_in_last_window = self.bytes_in_window;
            }
            self.bytes_in_window = 0;
            self.window_started_at = if elapsed >= 2 * Self::WINDOW {
                now
            } else {
                self.window_started_at + Self::WINDOW
            };
        }
        self.bytes_in_window = self
            .bytes_in_window
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
    }

    /// The bits per second of the layer's last whole second of sending, or of the second just
    /// ended when more: a layer is costed high rather than low until a whole second says otherwise.
    /// 0 before its first second.
    #[must_use]
    pub(crate) fn bits_per_second(&self, now: Instant) -> u64 {
        let elapsed = now.saturating_duration_since(self.window_started_at);
        let just_ended = if elapsed >= Self::WINDOW && elapsed < 2 * Self::WINDOW {
            self.bytes_in_window
        } else {
            0
        };
        self.bytes_in_last_window.max(just_ended).saturating_mul(8)
    }
}

/// What to do with a frame of a camera's layer for one watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameForwarding {
    /// Write it. `is_switch`: it is the keyframe the watcher waited for, so the layers the camera
    /// must send may have changed.
    Forward {
        is_switch: bool,
    },
    Drop,
    /// Drop it, and ask the camera for a keyframe of this layer: the watcher wants it and has
    /// not had one yet.
    DropAndRequestKeyframe,
}

/// One camera as one watcher receives it: the layer it wants (from its tile and bandwidth, `None`
/// while paused) and the layer it is forwarded. A watcher switches only at a keyframe of the
/// wanted layer, and keeps receiving the old layer until then, so its decoder never gets a frame
/// it cannot decode; a paused tile stops at once and starts again at a keyframe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LayerSwitch {
    wanted: Option<SimulcastLayer>,
    forwarded: Option<SimulcastLayer>,
}

impl LayerSwitch {
    #[must_use]
    pub(crate) const fn new(wanted: Option<SimulcastLayer>) -> Self {
        Self {
            wanted,
            forwarded: None,
        }
    }

    pub(crate) const fn want(&mut self, layer: Option<SimulcastLayer>) {
        self.wanted = layer;
        if layer.is_none() {
            self.forwarded = None;
        }
    }

    #[must_use]
    pub(crate) const fn wanted(self) -> Option<SimulcastLayer> {
        self.wanted
    }

    /// The layer being forwarded, `None` before the first keyframe and while paused.
    #[must_use]
    pub(crate) const fn forwarded(self) -> Option<SimulcastLayer> {
        self.forwarded
    }

    /// A frame of `layer` arrived from the camera.
    pub(crate) fn on_frame(&mut self, layer: SimulcastLayer, is_keyframe: bool) -> FrameForwarding {
        let is_forwarded = self.forwarded == Some(layer);
        if Some(layer) != self.wanted {
            return if is_forwarded {
                FrameForwarding::Forward { is_switch: false }
            } else {
                FrameForwarding::Drop
            };
        }
        if is_forwarded {
            FrameForwarding::Forward { is_switch: false }
        } else if is_keyframe {
            self.forwarded = Some(layer);
            FrameForwarding::Forward { is_switch: true }
        } else {
            FrameForwarding::DropAndRequestKeyframe
        }
    }
}

/// The layers a camera must keep sending, given how each watcher receives it: the lowest always
/// (the server's fallback, and cheap), plus every layer wanted or still forwarded during a switch.
/// The client pauses the others (`VOICE_VIDEO_LAYERS`).
pub(crate) fn active_layers<'a>(
    switches: impl IntoIterator<Item = &'a LayerSwitch>,
) -> Vec<SimulcastLayer> {
    let mut active = BTreeSet::from([SimulcastLayer::Quarter]);
    for switch in switches {
        active.extend(switch.wanted);
        active.extend(switch.forwarded);
    }
    active.into_iter().collect()
}

/// When each key was last let through, so a key passes at most once per `interval`: a camera
/// layer gets at most one keyframe request every `MINIMUM_INTERVAL_BETWEEN_KEYFRAME_REQUESTS`,
/// and a watcher passes on its own requests for one camera at most every
/// `MINIMUM_INTERVAL_BETWEEN_KEYFRAME_REQUESTS_OF_A_WATCHER`.
#[derive(Debug)]
pub(crate) struct Throttle<Key> {
    interval: Duration,
    last_passed_at: HashMap<Key, Instant>,
}

impl<Key: Copy + Eq + Hash> Throttle<Key> {
    pub(crate) fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_passed_at: HashMap::new(),
        }
    }

    /// Whether `key` may pass now; records it when so.
    pub(crate) fn may_pass(&mut self, key: Key, now: Instant) -> bool {
        if self
            .last_passed_at
            .get(&key)
            .is_some_and(|last| now.saturating_duration_since(*last) < self.interval)
        {
            return false;
        }
        self.last_passed_at.insert(key, now);
        true
    }

    /// Forgets the keys `is_gone` names (a participant left), so the map stays bounded by those
    /// connected.
    pub(crate) fn forget(&mut self, is_gone: impl Fn(&Key) -> bool) {
        self.last_passed_at.retain(|key, _| !is_gone(key));
    }
}

/// The most a camera or screen may send on one layer, in bytes per second: twice the target of
/// the small layers (ADR-0035), and for the full layer 10 Mbit/s, a 1440p screen at 30 frames per
/// second with its keyframes (ADR-0037). Keyframes and an encoder's overshoot pass, but a client
/// sending a large picture under the name of a small layer does not reach every small tile.
#[must_use]
pub(crate) const fn maximum_bytes_per_second_of(layer: SimulcastLayer) -> u32 {
    match layer {
        // 150 kbit/s target.
        SimulcastLayer::Quarter => 37_500,
        // 500 kbit/s target.
        SimulcastLayer::Low => 125_000,
        // 8 Mbit/s for a 1440p screen, 1.5 Mbit/s for a camera.
        SimulcastLayer::High => 1_250_000,
    }
}

/// Whether a keyframe's picture fits its layer: at most twice the camera layer's size each way for
/// `q` (640 × 360) and `l` (1280 × 720), and a 1440p screen for `h` (2560 × 1440, ADR-0037). A
/// camera could otherwise send a tiny keyframe declaring a huge picture, which every watcher of
/// that layer would have to allocate and decode. A VP8 keyframe without its start code does not
/// fit, nor an H.264 one without a Constrained Baseline sequence parameter set (ADR-0052).
#[must_use]
pub(crate) fn is_keyframe_within_layer_size(
    codec: VideoCodec,
    layer: SimulcastLayer,
    keyframe: &[u8],
) -> bool {
    let picture_size = match codec {
        VideoCodec::Vp8 => vp8_keyframe_picture_size(keyframe),
        VideoCodec::H264 => constrained_baseline_picture_size(keyframe),
        VideoCodec::Unknown => None,
    };
    let (maximum_width, maximum_height) = match layer {
        SimulcastLayer::Quarter => (640, 360),
        SimulcastLayer::Low => (1_280, 720),
        SimulcastLayer::High => (2_560, 1_440),
    };
    picture_size.is_some_and(|(width, height)| width <= maximum_width && height <= maximum_height)
}

/// The picture size a VP8 keyframe declares after its start code (RFC 6386, 9.1).
fn vp8_keyframe_picture_size(keyframe: &[u8]) -> Option<(u64, u64)> {
    const VP8_START_CODE: [u8; 3] = [0x9d, 0x01, 0x2a];
    /// The two top bits of each dimension are its upscaling hint, not its size.
    const DIMENSION_MASK: u16 = 0x3fff;
    let (Some(start_code), Some(&[width_low, width_high, height_low, height_high])) =
        (keyframe.get(3..6), keyframe.get(6..10))
    else {
        return None;
    };
    let width = u16::from_le_bytes([width_low, width_high]) & DIMENSION_MASK;
    let height = u16::from_le_bytes([height_low, height_high]) & DIMENSION_MASK;
    (start_code == VP8_START_CODE).then_some((u64::from(width), u64::from(height)))
}

/// A token bucket of bytes refilled at `maximum_bytes_per_second_of` its layer and holding one
/// second of them: a frame passes only if the bucket holds its size, and only frames actually
/// forwarded are taken from it, so frames dropped for watchers waiting for a keyframe never use
/// up the room that keyframe needs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LayerByteBudget {
    bytes_per_second: u32,
    available_bytes: f64,
    refilled_at: Instant,
}

impl LayerByteBudget {
    #[must_use]
    pub(crate) fn new(layer: SimulcastLayer, now: Instant) -> Self {
        let bytes_per_second = maximum_bytes_per_second_of(layer);
        Self {
            bytes_per_second,
            available_bytes: f64::from(bytes_per_second),
            refilled_at: now,
        }
    }

    /// Whether a frame of `frame_bytes` fits now.
    pub(crate) fn has_room_for(&mut self, frame_bytes: usize, now: Instant) -> bool {
        let rate = f64::from(self.bytes_per_second);
        let elapsed = now
            .saturating_duration_since(self.refilled_at)
            .as_secs_f64();
        self.available_bytes = elapsed.mul_add(rate, self.available_bytes).min(rate);
        self.refilled_at = now;
        bytes_as_f64(frame_bytes) <= self.available_bytes
    }

    /// A frame of `frame_bytes` was forwarded: takes its bytes.
    pub(crate) fn spend(&mut self, frame_bytes: usize) {
        self.available_bytes = (self.available_bytes - bytes_as_f64(frame_bytes)).max(0.0);
    }
}

/// A frame is at most a few megabytes: exact in an f64.
#[allow(clippy::cast_precision_loss)]
const fn bytes_as_f64(bytes: usize) -> f64 {
    bytes as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use SimulcastLayer::{High, Low, Quarter};

    const NOMINAL: LayerCosts = LayerCosts {
        quarter: 150_000,
        low: 500_000,
        high: 1_500_000,
    };

    fn tile(asked: SimulcastLayer) -> TileDemand {
        TileDemand {
            asked,
            costs: NOMINAL,
        }
    }

    #[test]
    fn a_layer_costs_its_measure_between_its_target_and_its_byte_budget() {
        assert_eq!(layer_cost(Quarter, 0), 150_000);
        assert_eq!(layer_cost(Quarter, 200_000), 200_000);
        assert_eq!(
            layer_cost(Quarter, 12_000_000),
            300_000,
            "a camera inflating a layer counts its budget at most"
        );
        assert_eq!(layer_cost(High, 8_000_000), 8_000_000, "a 1440p screen");
    }

    #[test]
    fn a_share_of_the_upload_for_each_watcher() {
        assert_eq!(fair_share_of_upload(100_000_000, 4), 25_000_000);
        assert_eq!(
            fair_share_of_upload(100_000_000, 0),
            100_000_000,
            "a first watcher counts as one"
        );
    }

    fn report(loss_percent: u32, delivered_bits_per_second: u64) -> LinkReport {
        LinkReport {
            loss_percent: Some(loss_percent),
            delivered_bits_per_second,
        }
    }

    #[test]
    fn a_clear_link_keeps_a_sharp_screen_however_light_it_runs() {
        let start = Instant::now();
        let screen = TileDemand {
            asked: High,
            costs: LayerCosts {
                high: 8_000_000,
                ..NOMINAL
            },
        };
        let mut allocation = LayerAllocation::default();
        for second in 0..30 {
            assert_eq!(
                allocation.next_layers(
                    &[screen],
                    &[Some(High)],
                    report(0, 300_000),
                    100_000_000,
                    start + Duration::from_secs(second)
                ),
                [Some(High)]
            );
        }
    }

    #[test]
    fn heavy_loss_lowers_to_what_gets_through_and_light_loss_holds() {
        let now = Instant::now();
        let tiles = [tile(High)];
        let mut allocation = LayerAllocation::default();
        assert_eq!(
            allocation.next_layers(
                &tiles,
                &[Some(High)],
                report(5, 1_500_000),
                100_000_000,
                now
            ),
            [Some(High)],
            "5 % lost: hold"
        );
        // 1.5 Mbit/s sent, 40 % lost: 900 kbit/s get through, 810 kbit/s for tiles.
        assert_eq!(
            allocation.next_layers(
                &tiles,
                &[Some(High)],
                report(40, 1_500_000),
                100_000_000,
                now
            ),
            [Some(Low)]
        );
        assert_eq!(
            allocation.next_layers(&tiles, &[Some(Low)], report(90, 500_000), 100_000_000, now),
            [None],
            "almost nothing gets through: paused"
        );
    }

    #[test]
    fn a_clear_link_goes_up_one_step_at_a_time_and_waits_longer_after_a_failed_step() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let tiles = [tile(High)];
        let share = 100_000_000;
        let mut allocation = LayerAllocation::default();
        assert_eq!(
            allocation.next_layers(&tiles, &[None], report(0, 0), share, at(0)),
            [None]
        );
        assert_eq!(
            allocation.next_layers(&tiles, &[None], report(0, 0), share, at(5)),
            [Some(Quarter)],
            "5 s clear: one step"
        );
        assert_eq!(
            allocation.next_layers(&tiles, &[Some(Quarter)], report(1, 150_000), share, at(9)),
            [Some(Quarter)]
        );
        assert_eq!(
            allocation.next_layers(&tiles, &[Some(Quarter)], report(1, 150_000), share, at(10)),
            [Some(Low)]
        );
        // The step to Low meets loss within its trial: back down, and the next wait is 10 s.
        assert_eq!(
            allocation.next_layers(&tiles, &[Some(Low)], report(30, 500_000), share, at(12)),
            [Some(Quarter)]
        );
        assert_eq!(
            allocation.next_layers(&tiles, &[Some(Quarter)], report(0, 150_000), share, at(13)),
            [Some(Quarter)]
        );
        assert_eq!(
            allocation.next_layers(&tiles, &[Some(Quarter)], report(0, 150_000), share, at(22)),
            [Some(Quarter)],
            "9 s only"
        );
        assert_eq!(
            allocation.next_layers(&tiles, &[Some(Quarter)], report(0, 150_000), share, at(23)),
            [Some(Low)]
        );
    }

    #[test]
    fn no_feedback_while_video_flows_holds_and_none_while_nothing_flows_may_go_up() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let tiles = [tile(High)];
        let late = |delivered| LinkReport {
            loss_percent: None,
            delivered_bits_per_second: delivered,
        };
        let mut allocation = LayerAllocation::default();
        for second in 0..20 {
            assert_eq!(
                allocation.next_layers(
                    &tiles,
                    &[Some(Low)],
                    late(500_000),
                    100_000_000,
                    at(second)
                ),
                [Some(Low)],
                "feedback late behind a queue: no step up"
            );
        }
        let mut paused = LayerAllocation::default();
        assert_eq!(
            paused.next_layers(&tiles, &[None], late(0), 100_000_000, at(0)),
            [None]
        );
        assert_eq!(
            paused.next_layers(&tiles, &[None], late(0), 100_000_000, at(5)),
            [Some(Quarter)]
        );
    }

    #[test]
    fn loss_on_a_light_or_new_stream_lowers_one_step_only() {
        let now = Instant::now();
        let screen = TileDemand {
            asked: High,
            costs: LayerCosts {
                high: 8_000_000,
                ..NOMINAL
            },
        };
        let mut allocation = LayerAllocation::default();
        assert_eq!(
            allocation.next_layers(
                &[screen],
                &[Some(High)],
                report(10, 300_000),
                100_000_000,
                now
            ),
            [Some(Low)],
            "a still screen sends 300 kbit/s: one step, not down to what 300 kbit/s fits"
        );
        assert_eq!(
            allocation.next_layers(&[screen], &[Some(High)], report(30, 0), 100_000_000, now),
            [Some(Low)],
            "a tile just switched has sent nothing yet"
        );
    }

    #[test]
    fn the_wait_doubles_up_to_a_minute_and_resets_after_a_step_held_half_a_minute() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let tiles = [tile(High), tile(High), tile(High), tile(High), tile(High)];
        let share = 100_000_000;
        let mut allocation = LayerAllocation::default();
        let current = vec![None; 5];
        let mut now = 0;
        // Each step up meets loss 1 s later: the waits are 5, 10, 20, 40, 60, 60 s.
        for wait in [5, 10, 20, 40, 60, 60] {
            assert_eq!(
                allocation.next_layers(&tiles, &current, report(0, 0), share, at(now)),
                current
            );
            let raised =
                allocation.next_layers(&tiles, &current, report(0, 0), share, at(now + wait));
            assert_ne!(raised, current, "raised after {wait} s");
            let lowered =
                allocation.next_layers(&tiles, &raised, report(50, 0), share, at(now + wait + 1));
            assert_eq!(lowered, current);
            now += wait + 1;
        }
        // A step up that holds 30 s brings the wait back to 5 s: the link stayed clear all along,
        // so the next step comes at once rather than after 60 s.
        assert_eq!(
            allocation.next_layers(&tiles, &current, report(0, 0), share, at(now)),
            current
        );
        let raised = allocation.next_layers(&tiles, &current, report(0, 0), share, at(now + 60));
        assert_ne!(raised, current);
        assert_eq!(
            allocation.next_layers(&tiles, &raised, report(0, 0), share, at(now + 89)),
            raised,
            "not held 30 s yet: the wait is still 60 s"
        );
        assert_ne!(
            allocation.next_layers(&tiles, &raised, report(0, 0), share, at(now + 90)),
            raised,
            "held 30 s: the wait is 5 s again, already passed"
        );
    }

    #[test]
    fn a_step_up_never_passes_the_fair_share_and_a_smaller_share_lowers_at_once() {
        let start = Instant::now();
        let tiles = [tile(High)];
        let mut allocation = LayerAllocation::default();
        assert_eq!(
            allocation.next_layers(&tiles, &[Some(Low)], report(0, 500_000), 1_000_000, start),
            [Some(Low)]
        );
        assert_eq!(
            allocation.next_layers(
                &tiles,
                &[Some(Low)],
                report(0, 500_000),
                1_000_000,
                start + Duration::from_secs(60)
            ),
            [Some(Low)],
            "High would pass the share"
        );
        assert_eq!(
            allocation.next_layers(&tiles, &[Some(High)], report(0, 1_500_000), 400_000, start),
            [Some(Quarter)]
        );
    }

    #[test]
    fn loss_fractions_become_whole_percentages() {
        assert_eq!(loss_percent_of(0.0), 0);
        assert_eq!(loss_percent_of(0.104), 10);
        assert_eq!(loss_percent_of(1.5), 100);
        assert_eq!(loss_percent_of(-0.2), 0);
        assert_eq!(loss_percent_of(f32::NAN), 0);
    }

    #[test]
    fn small_tiles_go_down_first_then_tiles_pause_smallest_first() {
        let tiles = [tile(High), tile(Quarter), tile(Low), tile(Quarter)];
        // Asked: 1.5 M + 150 k + 500 k + 150 k = 2.3 M.
        assert_eq!(
            fit_tiles(&tiles, 2_300_000),
            [Some(High), Some(Quarter), Some(Low), Some(Quarter)]
        );
        assert_eq!(
            fit_tiles(&tiles, 2_000_000),
            [Some(High), Some(Quarter), Some(Quarter), Some(Quarter)],
            "the medium tile goes down before the large one"
        );
        assert_eq!(
            fit_tiles(&tiles, 1_000_000),
            [Some(Low), Some(Quarter), Some(Quarter), Some(Quarter)],
            "then the large one"
        );
        assert_eq!(
            fit_tiles(&tiles, 700_000),
            [Some(Quarter), Some(Quarter), Some(Quarter), Some(Quarter)]
        );
        assert_eq!(
            fit_tiles(&tiles, 599_999),
            [Some(Quarter), Some(Quarter), Some(Quarter), None],
            "the last small tile pauses first"
        );
        assert_eq!(
            fit_tiles(&tiles, 300_000),
            [Some(Quarter), None, Some(Quarter), None]
        );
        assert_eq!(
            fit_tiles(&tiles, 150_000),
            [Some(Quarter), None, None, None],
            "the large tile pauses last"
        );
        assert_eq!(fit_tiles(&tiles, 0), [None, None, None, None]);
        assert_eq!(fit_tiles(&[], 0), []);
    }

    #[test]
    fn a_layer_is_costed_at_what_it_measures_when_above_its_target() {
        let screen = TileDemand {
            asked: High,
            costs: LayerCosts {
                high: 8_000_000,
                ..NOMINAL
            },
        };
        assert_eq!(fit_tiles(&[screen], 9_000_000), [Some(High)]);
        assert_eq!(fit_tiles(&[screen], 5_000_000), [Some(Low)]);
    }

    #[test]
    fn a_recent_rate_forgets_a_stream_that_stopped() {
        let start = Instant::now();
        let at = |milliseconds| start + Duration::from_millis(milliseconds);
        let mut rate = RecentRate::new(start);
        rate.record(1_000, at(0));
        assert_eq!(rate.bits_per_second(at(500)), 0);
        assert_eq!(rate.bits_per_second(at(1_100)), 8_000);
        rate.record(500, at(1_200));
        assert_eq!(rate.bits_per_second(at(1_500)), 8_000);
        assert_eq!(rate.bits_per_second(at(2_500)), 4_000);
        assert_eq!(rate.bits_per_second(at(3_100)), 0, "nothing for a second");
        rate.record(100, at(9_000));
        assert_eq!(
            rate.bits_per_second(at(9_500)),
            0,
            "the old seconds are gone"
        );
    }

    #[test]
    fn a_layer_rate_is_its_last_whole_second() {
        let start = Instant::now();
        let at = |milliseconds| start + Duration::from_millis(milliseconds);
        let mut rate = LayerRate::new(start);
        rate.record(1_000, at(0));
        rate.record(1_000, at(900));
        assert_eq!(rate.bits_per_second(at(950)), 0, "no whole second yet");
        assert_eq!(rate.bits_per_second(at(1_100)), 16_000);
        rate.record(500, at(1_100));
        assert_eq!(rate.bits_per_second(at(1_200)), 16_000);
        assert_eq!(
            rate.bits_per_second(at(2_100)),
            16_000,
            "a lighter second just ended does not lower the cost yet"
        );
        assert_eq!(
            rate.bits_per_second(at(9_000)),
            16_000,
            "a paused layer keeps its last whole second"
        );
        rate.record(100, at(9_000));
        assert_eq!(
            rate.bits_per_second(at(9_500)),
            16_000,
            "the second it stopped in was partial: forgotten"
        );
        rate.record(100, at(10_100));
        assert_eq!(
            rate.bits_per_second(at(10_200)),
            800,
            "a whole second again"
        );
    }

    #[test]
    fn tiles_get_the_layer_of_their_size() {
        assert_eq!(layer_for_tile(VideoTileSize::Large), High);
        assert_eq!(layer_for_tile(VideoTileSize::Medium), Low);
        assert_eq!(layer_for_tile(VideoTileSize::Small), Quarter);
    }

    #[test]
    fn twelve_cameras_per_channel_and_twenty_five_per_instance_the_asker_not_counted() {
        let (asker, lounge, booth) = (
            Snowflake::from_raw(1),
            Snowflake::from_raw(50),
            Snowflake::from_raw(51),
        );
        let cameras = |in_lounge: u64, in_booth: u64| {
            (100..100 + in_lounge)
                .map(move |id| (Snowflake::from_raw(id), lounge))
                .chain((200..200 + in_booth).map(move |id| (Snowflake::from_raw(id), booth)))
        };
        assert_eq!(
            missing_room_for_a_camera(cameras(11, 13), asker, lounge),
            None
        );
        assert_eq!(
            missing_room_for_a_camera(cameras(12, 0), asker, lounge),
            Some(VoiceVideoRefusal::TooManyCameras)
        );
        assert_eq!(
            missing_room_for_a_camera(cameras(12, 0), asker, booth),
            None,
            "the channel cap counts that channel only"
        );
        assert_eq!(
            missing_room_for_a_camera(cameras(0, 25), asker, lounge),
            Some(VoiceVideoRefusal::TooManyCameras),
            "the instance cap counts every channel"
        );
        let with_own = cameras(11, 0).chain([(asker, lounge)]);
        assert_eq!(
            missing_room_for_a_camera(with_own, asker, lounge),
            None,
            "the asker's own camera is not counted"
        );
    }

    #[test]
    fn a_watcher_starts_and_switches_only_at_a_keyframe_of_the_wanted_layer() {
        let mut switch = LayerSwitch::new(Some(Low));
        assert_eq!(switch.on_frame(High, true), FrameForwarding::Drop);
        assert_eq!(
            switch.on_frame(Low, false),
            FrameForwarding::DropAndRequestKeyframe,
            "nothing decodable yet"
        );
        assert_eq!(
            switch.on_frame(Low, true),
            FrameForwarding::Forward { is_switch: true }
        );
        assert_eq!(
            switch.on_frame(Low, false),
            FrameForwarding::Forward { is_switch: false }
        );

        switch.want(Some(High));
        assert_eq!(
            switch.on_frame(Low, false),
            FrameForwarding::Forward { is_switch: false },
            "the old layer flows until the new one's keyframe"
        );
        assert_eq!(
            switch.on_frame(High, false),
            FrameForwarding::DropAndRequestKeyframe
        );
        assert_eq!(
            switch.on_frame(High, true),
            FrameForwarding::Forward { is_switch: true }
        );
        assert_eq!(switch.on_frame(Low, false), FrameForwarding::Drop);
        assert_eq!(switch.forwarded(), Some(High));
        assert_eq!(switch.wanted(), Some(High));
    }

    #[test]
    fn a_layer_asked_for_a_second_without_a_frame_is_gone() {
        let start = Instant::now();
        let at = |milliseconds| start + Duration::from_millis(milliseconds);
        let lowest = Some(at(4_900));
        assert!(
            !is_layer_gone(None, None, lowest, at(5_000)),
            "not asked yet"
        );
        assert!(
            !is_layer_gone(Some(at(4_100)), None, lowest, at(5_000)),
            "just asked"
        );
        assert!(is_layer_gone(Some(at(0)), None, lowest, at(5_000)));
        assert!(!is_layer_gone(
            Some(at(0)),
            Some(at(4_500)),
            lowest,
            at(5_000)
        ));
        assert!(is_layer_gone(
            Some(at(0)),
            Some(at(3_900)),
            lowest,
            at(5_000)
        ));
        assert!(
            !is_layer_gone(Some(at(0)), Some(at(3_900)), Some(at(3_900)), at(5_000)),
            "every layer silent together: a still picture, nothing gone"
        );
        assert!(!is_layer_gone(Some(at(0)), None, None, at(5_000)));
    }

    #[test]
    fn a_tile_falls_back_to_the_highest_layer_still_arriving() {
        assert_eq!(arriving_layer_at_most(High, |_| false), High);
        assert_eq!(arriving_layer_at_most(High, |layer| layer == High), Low);
        assert_eq!(
            arriving_layer_at_most(High, |layer| layer != Quarter),
            Quarter
        );
        assert_eq!(
            arriving_layer_at_most(Low, |layer| layer == High),
            Low,
            "a higher layer gone changes nothing"
        );
        assert_eq!(arriving_layer_at_most(High, |_| true), Quarter);
    }

    #[test]
    fn a_paused_tile_stops_at_once_and_starts_again_at_a_keyframe() {
        let mut switch = LayerSwitch::new(Some(Low));
        assert_eq!(
            switch.on_frame(Low, true),
            FrameForwarding::Forward { is_switch: true }
        );
        switch.want(None);
        assert_eq!(switch.on_frame(Low, false), FrameForwarding::Drop);
        assert_eq!(switch.forwarded(), None);
        assert_eq!(active_layers([&switch]), [Quarter], "nothing kept for it");
        switch.want(Some(Low));
        assert_eq!(
            switch.on_frame(Low, false),
            FrameForwarding::DropAndRequestKeyframe
        );
        assert_eq!(
            switch.on_frame(Low, true),
            FrameForwarding::Forward { is_switch: true }
        );
    }

    #[test]
    fn cameras_keep_their_lowest_layer_and_every_layer_wanted_or_still_forwarded() {
        assert_eq!(active_layers([]), [Quarter], "nobody watches");
        let mut large = LayerSwitch::new(Some(High));
        let small = LayerSwitch::new(Some(Quarter));
        assert_eq!(active_layers([&large, &small]), [Quarter, High]);
        assert_eq!(
            large.on_frame(High, true),
            FrameForwarding::Forward { is_switch: true }
        );
        large.want(Some(Low));
        assert_eq!(
            active_layers([&large]),
            [Quarter, Low, High],
            "High stays until Low's keyframe"
        );
        assert_eq!(
            large.on_frame(Low, true),
            FrameForwarding::Forward { is_switch: true }
        );
        assert_eq!(active_layers([&large]), [Quarter, Low]);
    }

    #[test]
    fn a_throttle_lets_each_key_through_once_per_interval() {
        let start = Instant::now();
        let at = |milliseconds| start + Duration::from_millis(milliseconds);
        let mut throttle = Throttle::new(MINIMUM_INTERVAL_BETWEEN_KEYFRAME_REQUESTS);
        assert!(throttle.may_pass(('a', High), at(0)));
        assert!(!throttle.may_pass(('a', High), at(299)));
        assert!(throttle.may_pass(('a', Low), at(10)), "another layer");
        assert!(throttle.may_pass(('b', High), at(10)), "another camera");
        assert!(throttle.may_pass(('a', High), at(300)));
        throttle.forget(|(camera, _)| *camera == 'a');
        assert!(throttle.may_pass(('a', High), at(301)), "forgotten");
    }

    #[test]
    fn a_camera_sends_vp8_whenever_it_can_and_watchers_decode_what_all_their_lines_carry() {
        use VideoCodec::{H264, Vp8};
        assert_eq!(camera_codec_of(&[H264, Vp8]), Some(Vp8));
        assert_eq!(camera_codec_of(&[H264]), Some(H264));
        assert_eq!(camera_codec_of(&[]), None);
        assert_eq!(
            codecs_decoded_on_every_line(&[vec![H264, Vp8], vec![Vp8, H264]]),
            vec![H264, Vp8]
        );
        assert_eq!(
            codecs_decoded_on_every_line(&[vec![Vp8, H264], vec![Vp8]]),
            vec![Vp8],
            "a line without H.264 could be the free one"
        );
        assert_eq!(codecs_decoded_on_every_line(&[]), Vec::<VideoCodec>::new());
    }

    #[test]
    fn keyframes_declare_at_most_twice_their_layer_size() {
        let keyframe = |width: u16, height: u16| {
            let mut frame = vec![0x10, 0, 0, 0x9d, 0x01, 0x2a];
            frame.extend(width.to_le_bytes());
            frame.extend(height.to_le_bytes());
            frame
        };
        assert!(is_keyframe_within_layer_size(
            VideoCodec::Vp8,
            Quarter,
            &keyframe(320, 180)
        ));
        assert!(is_keyframe_within_layer_size(
            VideoCodec::Vp8,
            Quarter,
            &keyframe(640, 360)
        ));
        assert!(!is_keyframe_within_layer_size(
            VideoCodec::Vp8,
            Quarter,
            &keyframe(641, 360)
        ));
        assert!(!is_keyframe_within_layer_size(
            VideoCodec::Vp8,
            Quarter,
            &keyframe(16_383, 16_383)
        ));
        assert!(is_keyframe_within_layer_size(
            VideoCodec::Vp8,
            High,
            &keyframe(2_560, 1_440)
        ));
        assert!(!is_keyframe_within_layer_size(
            VideoCodec::Vp8,
            High,
            &keyframe(2_561, 1_440)
        ));
        assert!(
            is_keyframe_within_layer_size(
                VideoCodec::Vp8,
                Quarter,
                &keyframe(0x0140 | 0xc000, 180)
            ),
            "the scaling bits are not part of the size"
        );
        let mut without_start_code = keyframe(320, 180);
        without_start_code[3] = 0;
        assert!(!is_keyframe_within_layer_size(
            VideoCodec::Vp8,
            Quarter,
            &without_start_code
        ));
        assert!(!is_keyframe_within_layer_size(
            VideoCodec::Vp8,
            Quarter,
            &[0x10, 0, 0]
        ));
    }

    #[test]
    fn h264_keyframes_declare_at_most_twice_their_layer_size_too() {
        use crate::domain::h264_keyframes::test_streams::keyframe;
        let fits = |layer, width, height| {
            is_keyframe_within_layer_size(VideoCodec::H264, layer, &keyframe(width, height, &[1]))
        };
        assert!(fits(Quarter, 320, 180));
        assert!(fits(Quarter, 640, 360));
        assert!(!fits(Quarter, 656, 360));
        assert!(fits(Low, 1_280, 720));
        assert!(!fits(Low, 1_920, 1_080));
        assert!(fits(High, 2_560, 1_440));
        assert!(!fits(High, 3_840, 2_160));
        let vp8_keyframe = [0x10, 0, 0, 0x9d, 0x01, 0x2a, 0x40, 0x01, 0xb4, 0x00];
        assert!(
            !is_keyframe_within_layer_size(VideoCodec::H264, Quarter, &vp8_keyframe),
            "a frame of another codec never fits"
        );
    }

    #[test]
    fn a_layer_passes_twice_its_bit_rate_with_a_second_of_burst() {
        let start = Instant::now();
        let at = |milliseconds| start + Duration::from_millis(milliseconds);
        let mut budget = LayerByteBudget::new(Quarter, start);
        let mut forward = |bytes, now| {
            let has_room = budget.has_room_for(bytes, now);
            if has_room {
                budget.spend(bytes);
            }
            has_room
        };
        assert!(forward(30_000, at(0)), "a keyframe burst");
        assert!(forward(7_500, at(0)));
        assert!(!forward(1, at(0)), "a second's worth is spent");
        assert!(!forward(4_000, at(100)), "3 750 bytes came back");
        assert!(forward(3_750, at(100)));
        assert!(
            forward(37_500, at(10_000)),
            "the bucket holds one second, however long it rested"
        );
        assert!(!forward(1, at(10_000)));
        let mut unspent = LayerByteBudget::new(Quarter, start);
        assert!(unspent.has_room_for(37_500, at(0)));
        assert!(
            unspent.has_room_for(37_500, at(0)),
            "a frame nobody received takes nothing"
        );
        let high = maximum_bytes_per_second_of(High);
        assert!(high > maximum_bytes_per_second_of(Low));
        assert!(maximum_bytes_per_second_of(Low) > maximum_bytes_per_second_of(Quarter));
    }
}
