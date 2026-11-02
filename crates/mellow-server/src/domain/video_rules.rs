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

/// When each camera layer was last asked for a keyframe, so a camera gets at most one request per
/// layer every `MINIMUM_INTERVAL_BETWEEN_KEYFRAME_REQUESTS`.
#[derive(Debug)]
pub(crate) struct KeyframeThrottle<Camera> {
    last_request_at: HashMap<(Camera, SimulcastLayer), Instant>,
}

impl<Camera> Default for KeyframeThrottle<Camera> {
    fn default() -> Self {
        Self {
            last_request_at: HashMap::new(),
        }
    }
}

impl<Camera: Copy + Eq + Hash> KeyframeThrottle<Camera> {
    /// Whether to send a request now; records it when so.
    pub(crate) fn may_request(
        &mut self,
        camera: Camera,
        layer: SimulcastLayer,
        now: Instant,
    ) -> bool {
        let last = self.last_request_at.get(&(camera, layer));
        if last.is_some_and(|last| {
            now.saturating_duration_since(*last) < MINIMUM_INTERVAL_BETWEEN_KEYFRAME_REQUESTS
        }) {
            return false;
        }
        self.last_request_at.insert((camera, layer), now);
        true
    }

    /// The camera left: its entries go, so the map stays bounded by the cameras on.
    pub(crate) fn forget(&mut self, camera: Camera) {
        self.last_request_at
            .retain(|(known, _), _| *known != camera);
    }
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
    fn keyframe_requests_are_throttled_per_camera_and_layer() {
        let start = Instant::now();
        let at = |milliseconds| start + Duration::from_millis(milliseconds);
        let mut throttle = KeyframeThrottle::default();
        assert!(throttle.may_request('a', High, at(0)));
        assert!(!throttle.may_request('a', High, at(299)));
        assert!(throttle.may_request('a', Low, at(10)), "another layer");
        assert!(throttle.may_request('b', High, at(10)), "another camera");
        assert!(throttle.may_request('a', High, at(300)));
        throttle.forget('a');
        assert!(throttle.may_request('a', High, at(301)), "forgotten");
    }
}
