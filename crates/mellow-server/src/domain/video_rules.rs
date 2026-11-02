//! Rules of cameras in voice channels (ADR-0035) that need no I/O: which simulcast layer a tile
//! receives, when a watcher switches layers, how often a camera is asked for a keyframe, which
//! layers a camera must keep sending, and how many cameras may be on at once. The media loop and
//! the voice service apply them; the permission itself is `domain::permissions::may_send_video`.

use std::collections::{BTreeSet, HashMap};
use std::hash::Hash;
use std::time::{Duration, Instant};

use mellow_protocol::{SimulcastLayer, Snowflake, VideoTileSize, VoiceVideoRefusal};

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

/// One camera as one watcher receives it: the layer it wants (from its tile) and the layer it is
/// forwarded. A watcher switches only at a keyframe of the wanted layer, and keeps receiving the
/// old layer until then, so its decoder never gets a frame it cannot decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LayerSwitch {
    wanted: SimulcastLayer,
    forwarded: Option<SimulcastLayer>,
}

impl LayerSwitch {
    #[must_use]
    pub(crate) const fn new(wanted: SimulcastLayer) -> Self {
        Self {
            wanted,
            forwarded: None,
        }
    }

    pub(crate) const fn want(&mut self, layer: SimulcastLayer) {
        self.wanted = layer;
    }

    #[must_use]
    pub(crate) const fn wanted(self) -> SimulcastLayer {
        self.wanted
    }

    /// The layer being forwarded, `None` before the first keyframe.
    #[must_use]
    pub(crate) const fn forwarded(self) -> Option<SimulcastLayer> {
        self.forwarded
    }

    /// A frame of `layer` arrived from the camera.
    pub(crate) fn on_frame(&mut self, layer: SimulcastLayer, is_keyframe: bool) -> FrameForwarding {
        let is_forwarded = self.forwarded == Some(layer);
        if layer != self.wanted {
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
        active.insert(switch.wanted);
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

/// The most a camera may send on one layer, in bytes per second: twice the layer's target bit
/// rate (ADR-0035), so keyframes and an encoder's overshoot pass but a client sending a large
/// picture under the name of a small layer does not reach every small tile.
#[must_use]
pub(crate) const fn maximum_bytes_per_second_of(layer: SimulcastLayer) -> u32 {
    match layer {
        // 150 kbit/s target.
        SimulcastLayer::Quarter => 37_500,
        // 500 kbit/s target.
        SimulcastLayer::Low => 125_000,
        // 1.5 Mbit/s target.
        SimulcastLayer::High => 375_000,
    }
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
        let mut switch = LayerSwitch::new(Low);
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

        switch.want(High);
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
        assert_eq!(switch.wanted(), High);
    }

    #[test]
    fn cameras_keep_their_lowest_layer_and_every_layer_wanted_or_still_forwarded() {
        assert_eq!(active_layers([]), [Quarter], "nobody watches");
        let mut large = LayerSwitch::new(High);
        let small = LayerSwitch::new(Quarter);
        assert_eq!(active_layers([&large, &small]), [Quarter, High]);
        assert_eq!(
            large.on_frame(High, true),
            FrameForwarding::Forward { is_switch: true }
        );
        large.want(Low);
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
