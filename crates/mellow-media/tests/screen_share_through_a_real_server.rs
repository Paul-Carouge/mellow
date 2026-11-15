//! Screen sharing through a real in-process `mellow-server` (ADR-0054). A synthetic screen with
//! its sound is shared in VP8 and another engine watches it: pictures and the screen's sound must
//! reach it. A screen encoded elsewhere in H.264 (a recorded Constrained Baseline stream) makes
//! the session offer again with H.264 alone on its camera line: the server must announce the
//! codec and forward the frames to a watcher whose receive line decodes H.264. No real screen,
//! camera, GPU or audio device is touched.

// The server refuses Windows until its master key file can be protected there (ADR-0006).
#![cfg(not(windows))]
// Test helpers outside `#[test]` functions: failing loudly is the point.
#![allow(clippy::unwrap_used, clippy::panic)]

mod support;

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mellow_media::{
    RunningScreenCapture, ScreenCaptureError, ScreenChoice, ScreenPicker, ScreenShareContext,
    ScreenShareQuality, ScreenSourceDescription, ScreenSources, ShareSound, ShareSoundState,
    SyntheticScreens, SyntheticSound, VoicePhase, VoiceSessionEvent, WatchedVideo,
};
use mellow_protocol::{
    ClientFrame, GatewayEvent, SimulcastLayer, VideoCodec, VideoTileSize, WatchedCamera,
};
use str0m::change::SdpAnswer;
use str0m::format::Codec;
use str0m::media::{Direction, MediaKind, Mid, Pt};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};
use support::{
    HEARING_TIMEOUT, LOOPBACK, Participant, RunningInstance, identified_gateway,
    shared_voice_channel,
};

const SCREEN_SIZE: (u32, u32) = (640, 360);
const PICTURES_EXPECTED: usize = 10;
/// A tenth of a second of the screen's tone heard by the watcher.
const AUDIBLE_SAMPLES_EXPECTED: u64 = 4_800;
/// libx264, Constrained Baseline, 320 × 180: small enough for every layer's size check.
const RECORDED_H264: &[u8] = include_bytes!("fixtures/h264_constrained_baseline.h264");
const H264_FRAMES_EXPECTED: usize = 10;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shared_screen_and_its_sound_reach_a_watcher() {
    let instance = RunningInstance::start().await;
    let (owner, sharer_id) = instance.registered("sharer").await;
    let (watcher_client, watcher_id) = instance.registered("watcher").await;
    let channel_id = shared_voice_channel(&owner, &watcher_client).await;
    let sharer_gateway = identified_gateway(instance.listen_address, owner.token().unwrap()).await;
    let watcher_gateway =
        identified_gateway(instance.listen_address, watcher_client.token().unwrap()).await;
    let mut sharer = Participant::join(
        sharer_gateway,
        channel_id,
        sharer_id,
        SyntheticSound::Silence,
    );
    let mut watcher = Participant::join(
        watcher_gateway,
        channel_id,
        watcher_id,
        SyntheticSound::Silence,
    );
    sharer.wait_until_connected().await;
    watcher.wait_until_connected().await;

    let screens = SyntheticScreens {
        width: SCREEN_SIZE.0,
        height: SCREEN_SIZE.1,
    };
    assert_eq!(screens.picker(), ScreenPicker::InApp);
    let source = screens.sources(false).unwrap().remove(0);
    let share = sharer
        .session
        .start_screen_share(
            &screens,
            &ScreenChoice::Source(source.id),
            ScreenShareQuality::Standard,
            &ShareSound::FromSource,
        )
        .unwrap();
    assert_eq!(share.sound(), ShareSoundState::Captured);
    assert_eq!(share.codec(), VideoCodec::Vp8);
    watcher
        .wait_for_event(|event| {
            matches!(
                event,
                GatewayEvent::VoiceStateUpdate { voice_state }
                    if voice_state.user_id == sharer_id && voice_state.self_stream
            )
        })
        .await;
    let frames = watcher.session.video_frames();
    watcher.session.watch_videos(&[WatchedVideo {
        user_id: sharer_id,
        tile_size: VideoTileSize::Large,
        display_width: SCREEN_SIZE.0,
        display_height: SCREEN_SIZE.1,
    }]);

    let deadline = Instant::now() + HEARING_TIMEOUT;
    let mut pictures = 0;
    while pictures < PICTURES_EXPECTED || watcher.probe.audible_samples() < AUDIBLE_SAMPLES_EXPECTED
    {
        assert!(
            Instant::now() < deadline,
            "pictures: {pictures}, audible samples: {}, sharer events: {:?}, watcher events: \
             {:?}",
            watcher.probe.audible_samples(),
            sharer.seen_session_events,
            watcher.seen_session_events,
        );
        sharer.pump();
        watcher.pump();
        pictures += frames.take_new_frames().len();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        sharer.probe.audible_samples(),
        0,
        "the sharer never hears their own screen"
    );

    // Dropping the share turns it off, and the server hears of it.
    drop(share);
    assert_eq!(sharer.session.video_source(), None);
    watcher
        .wait_for_event(|event| {
            matches!(
                event,
                GatewayEvent::VoiceStateUpdate { voice_state }
                    if voice_state.user_id == sharer_id && !voice_state.self_stream
            )
        })
        .await;
    sharer.session.leave();
    watcher.session.leave();
    instance.stop().await;
}

/// A screen whose layers are already encoded in H.264: recorded access units for the `q`, `l`
/// and `h` layers, looped at 30 frames per second, as a GPU encoder would send them.
struct RecordedH264Screen {
    layers: Arc<[Vec<Vec<u8>>; 3]>,
}

impl RecordedH264Screen {
    /// The recorded libx264 stream on every layer.
    fn same_on_every_layer() -> Self {
        let units = access_units_of(RECORDED_H264);
        Self {
            layers: Arc::new([units.clone(), units.clone(), units]),
        }
    }
}

struct RecordedH264Capture {
    should_stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl RunningScreenCapture for RecordedH264Capture {
    fn sound(&self) -> ShareSoundState {
        ShareSoundState::Off
    }
}

impl Drop for RecordedH264Capture {
    fn drop(&mut self) {
        self.should_stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

/// A recorded stream cut into access units at each slice (one slice per picture), the
/// parameter sets and SEI kept with the slice after them.
fn access_units_of(stream: &[u8]) -> Vec<Vec<u8>> {
    // NAL units after three- or four-byte start codes; zero bytes before a start code belong to
    // it, never to the unit before.
    let starts: Vec<usize> = stream
        .windows(3)
        .enumerate()
        .filter(|(_, window)| *window == [0, 0, 1])
        .map(|(index, _)| index + 3)
        .collect();
    let mut units: Vec<Vec<u8>> = vec![Vec::new()];
    for (index, &start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).map_or(stream.len(), |next| next - 3);
        let unit = &stream[start..end];
        let unit = &unit[..unit
            .iter()
            .rposition(|byte| *byte != 0)
            .map_or(0, |last| last + 1)];
        if unit.is_empty() {
            continue;
        }
        units.last_mut().unwrap().extend_from_slice(&[0, 0, 0, 1]);
        units.last_mut().unwrap().extend_from_slice(unit);
        // A slice (1) or an IDR slice (5) ends the access unit.
        if matches!(unit[0] & 0x1f, 1 | 5) {
            units.push(Vec::new());
        }
    }
    units.retain(|unit| !unit.is_empty());
    units
}

impl ScreenSources for RecordedH264Screen {
    fn picker(&self) -> ScreenPicker {
        ScreenPicker::System
    }

    fn sources(&self, _: bool) -> Result<Vec<ScreenSourceDescription>, ScreenCaptureError> {
        Ok(Vec::new())
    }

    fn start(
        &self,
        _choice: &ScreenChoice,
        _sound: &ShareSound,
        share: ScreenShareContext,
    ) -> Result<Box<dyn RunningScreenCapture>, ScreenCaptureError> {
        let mut input = share.start_encoded(VideoCodec::H264);
        let layers = Arc::clone(&self.layers);
        assert!(
            layers.iter().all(|units| units.len() >= 2),
            "too few access units"
        );
        let should_stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let should_stop = Arc::clone(&should_stop);
            std::thread::spawn(move || {
                let started = Instant::now();
                let mut index = 0;
                while !should_stop.load(Ordering::Relaxed) && input.is_open() {
                    // Keyframe requests restart the recording at its first IDR.
                    if !input.take_requests().keyframes.is_empty() {
                        index = 0;
                    }
                    let rtp_timestamp = input.rtp_time_of(Instant::now());
                    for (layer, units) in [
                        SimulcastLayer::Quarter,
                        SimulcastLayer::Low,
                        SimulcastLayer::High,
                    ]
                    .into_iter()
                    .zip(layers.iter())
                    {
                        input.push(layer, rtp_timestamp, units[index % units.len()].clone());
                    }
                    index += 1;
                    let next = started + Duration::from_millis(33) * u32::try_from(index).unwrap();
                    std::thread::sleep(next.saturating_duration_since(Instant::now()));
                }
            })
        };
        Ok(Box::new(RecordedH264Capture {
            should_stop,
            thread: Some(thread),
        }))
    }
}

/// A watcher without the engine: a `str0m` peer whose only video receive line decodes H.264
/// (the desktop's own lines decode VP8 only), joined through the gateway with the server's
/// m-line order.
struct H264Watcher {
    rtc: Rtc,
    socket: UdpSocket,
    local: SocketAddr,
    video_receive: Mid,
}

impl H264Watcher {
    fn offer() -> (Self, String, str0m::change::SdpPendingOffer) {
        let socket = UdpSocket::bind((LOOPBACK, 0)).unwrap();
        let local = socket.local_addr().unwrap();
        let mut builder = Rtc::builder().clear_codecs().enable_opus(true, false);
        builder.codec_config().add_h264(
            Pt::new_with_value(108),
            Some(Pt::new_with_value(109)),
            true,
            0x42_e0_1f,
        );
        let mut rtc = builder.build(Instant::now());
        rtc.add_local_candidate(Candidate::host(local, "udp").unwrap());
        let mut change = rtc.sdp_api();
        change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
        change.add_media(MediaKind::Audio, Direction::RecvOnly, None, None, None);
        change.add_media(MediaKind::Video, Direction::SendOnly, None, None, None);
        let video_receive =
            change.add_media(MediaKind::Video, Direction::RecvOnly, None, None, None);
        let (offer, pending) = change.apply().unwrap();
        (
            Self {
                rtc,
                socket,
                local,
                video_receive,
            },
            offer.to_sdp_string(),
            pending,
        )
    }

    /// Runs the peer for a moment; returns the H.264 frames received on the video line.
    fn step(&mut self) -> usize {
        let mut frames = 0;
        let mut buffer = [0_u8; 2_000];
        loop {
            match self.rtc.poll_output().unwrap() {
                Output::Timeout(_) => break,
                Output::Transmit(transmit) => {
                    self.socket
                        .send_to(&transmit.contents, transmit.destination)
                        .unwrap();
                }
                Output::Event(Event::MediaData(data)) if data.mid == self.video_receive => {
                    let codec = self
                        .rtc
                        .codec_config()
                        .find(|params| params.pt() == data.pt)
                        .map(|params| params.spec().codec);
                    assert_eq!(codec, Some(Codec::H264));
                    frames += 1;
                }
                Output::Event(_) => {}
            }
        }
        self.socket
            .set_read_timeout(Some(Duration::from_millis(5)))
            .unwrap();
        let input = match self.socket.recv_from(&mut buffer) {
            Ok((length, source)) => Input::Receive(
                Instant::now(),
                Receive {
                    proto: Protocol::Udp,
                    source,
                    destination: self.local,
                    contents: buffer[..length].try_into().unwrap(),
                },
            ),
            Err(_) => Input::Timeout(Instant::now()),
        };
        self.rtc.handle_input(input).unwrap();
        frames
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_h264_share_offers_again_and_its_frames_reach_an_h264_watcher() {
    let instance = RunningInstance::start().await;
    let (owner, sharer_id) = instance.registered("sharer").await;
    let (watcher_client, _watcher_id) = instance.registered("watcher").await;
    let channel_id = shared_voice_channel(&owner, &watcher_client).await;
    let sharer_gateway = identified_gateway(instance.listen_address, owner.token().unwrap()).await;
    let mut watcher_gateway =
        identified_gateway(instance.listen_address, watcher_client.token().unwrap()).await;
    let mut sharer = Participant::join(
        sharer_gateway,
        channel_id,
        sharer_id,
        SyntheticSound::Silence,
    );
    sharer.wait_until_connected().await;

    let (mut watcher, sdp_offer, pending) = H264Watcher::offer();
    watcher_gateway
        .outgoing
        .send(ClientFrame::VoiceJoin {
            channel_id,
            offer_id: 1,
            sdp_offer,
            self_mute: true,
            self_deaf: false,
        })
        .unwrap();
    let mut pending = Some(pending);
    let mut sharer_announced_h264 = false;
    let mut is_watching = false;
    let mut h264_frames = 0;
    let mut share = None;
    let deadline = Instant::now() + HEARING_TIMEOUT;
    while h264_frames < H264_FRAMES_EXPECTED {
        assert!(
            Instant::now() < deadline,
            "announced: {sharer_announced_h264}, watching: {is_watching}, frames: \
             {h264_frames}, sharer events: {:?}",
            sharer.seen_session_events
        );
        sharer.pump();
        while let Ok(event) = watcher_gateway.events.try_recv() {
            match event {
                GatewayEvent::VoiceSessionDescription { sdp_answer, .. } => {
                    let answer = SdpAnswer::from_sdp_string(&sdp_answer).unwrap();
                    watcher
                        .rtc
                        .sdp_api()
                        .accept_answer(pending.take().unwrap(), answer)
                        .unwrap();
                    share = Some(
                        sharer
                            .session
                            .start_screen_share(
                                &RecordedH264Screen::same_on_every_layer(),
                                &ScreenChoice::SystemPicker,
                                ScreenShareQuality::High,
                                &ShareSound::Off,
                            )
                            .unwrap(),
                    );
                }
                GatewayEvent::VoiceStateUpdate { voice_state }
                    if voice_state.user_id == sharer_id
                        && voice_state.self_stream
                        && voice_state.video_codec == Some(VideoCodec::H264) =>
                {
                    sharer_announced_h264 = true;
                    if !is_watching {
                        is_watching = true;
                        watcher_gateway
                            .outgoing
                            .send(ClientFrame::VoiceVideoWatch {
                                cameras: vec![WatchedCamera {
                                    user_id: sharer_id,
                                    size: VideoTileSize::Large,
                                }],
                            })
                            .unwrap();
                    }
                }
                _ => {}
            }
        }
        h264_frames += watcher.step();
        tokio::task::yield_now().await;
    }
    let share = share.unwrap();
    assert_eq!(share.codec(), VideoCodec::H264);
    let reconnecting = VoiceSessionEvent::PhaseChanged(VoicePhase::Reconnecting);
    assert!(
        !sharer.seen_session_events.contains(&reconnecting),
        "the renegotiation is no reconnection"
    );

    drop(share);
    sharer.session.leave();
    instance.stop().await;
}

/// The Linux sender's real-time H.264 layers (`vesper-encode` on VA-API, four coloured quadrants
/// turned one place each picture), as `q`, `l` and `h`.
fn linux_share_layers() -> RecordedH264Screen {
    let layer = |stream: &[u8]| access_units_of(stream);
    RecordedH264Screen {
        layers: Arc::new([
            layer(include_bytes!(
                "fixtures/vaapi-real-time-quadrants-640x360.h264"
            )),
            layer(include_bytes!(
                "fixtures/vaapi-real-time-quadrants-1280x720.h264"
            )),
            layer(include_bytes!(
                "fixtures/vaapi-real-time-quadrants-2560x1440.h264"
            )),
        ]),
    }
}

/// What a desktop watcher of a Linux share saw: the first picture, then nothing. A desktop
/// engine watching another's H.264 share through the server must keep getting new pictures, each
/// a later one of the stream (its quadrants turned), not the first again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_desktop_watcher_keeps_receiving_new_pictures_of_an_h264_share() {
    // Where the top left quadrant of a 640 × 360 picture lies, in BGRA.
    const PROBE_OFFSET: usize = (90 * 640 + 160) * 4;
    let instance = RunningInstance::start().await;
    let (owner, sharer_id) = instance.registered("sharer").await;
    let (watcher_client, watcher_id) = instance.registered("watcher").await;
    let channel_id = shared_voice_channel(&owner, &watcher_client).await;
    let sharer_gateway = identified_gateway(instance.listen_address, owner.token().unwrap()).await;
    let watcher_gateway =
        identified_gateway(instance.listen_address, watcher_client.token().unwrap()).await;
    let mut sharer = Participant::join(
        sharer_gateway,
        channel_id,
        sharer_id,
        SyntheticSound::Silence,
    );
    let mut watcher = Participant::join(
        watcher_gateway,
        channel_id,
        watcher_id,
        SyntheticSound::Silence,
    );
    sharer.wait_until_connected().await;
    watcher.wait_until_connected().await;

    let share = sharer
        .session
        .start_screen_share(
            &linux_share_layers(),
            &ScreenChoice::SystemPicker,
            ScreenShareQuality::High,
            &ShareSound::Off,
        )
        .unwrap();
    assert_eq!(share.codec(), VideoCodec::H264);
    watcher
        .wait_for_event(|event| {
            matches!(
                event,
                GatewayEvent::VoiceStateUpdate { voice_state }
                    if voice_state.user_id == sharer_id
                        && voice_state.self_stream
                        && voice_state.video_codec == Some(VideoCodec::H264)
            )
        })
        .await;
    let deadline = Instant::now() + HEARING_TIMEOUT;
    let frames = watcher.session.video_frames();
    // A small tile: the server sends the 640 × 360 layer, which reaches it unscaled.
    watcher.session.watch_videos(&[WatchedVideo {
        user_id: sharer_id,
        tile_size: VideoTileSize::Small,
        display_width: 640,
        display_height: 360,
    }]);

    let mut colours = std::collections::HashSet::new();
    let mut pictures = 0;
    while pictures < PICTURES_EXPECTED || colours.len() < 4 {
        assert!(
            Instant::now() < deadline,
            "pictures: {pictures}, top left colours seen: {colours:?}, sharer events: {:?}, \
             watcher events: {:?}",
            sharer.seen_session_events,
            watcher.seen_session_events,
        );
        sharer.pump();
        watcher.pump();
        for (_, picture) in frames.take_new_frames() {
            assert_eq!((picture.width, picture.height), (640, 360));
            let pixel = &picture.pixels[PROBE_OFFSET..PROBE_OFFSET + 3];
            // Each channel is near 0 or 255 in the four colours: rounded to which.
            colours.insert(pixel.iter().map(|value| *value > 128).collect::<Vec<_>>());
            pictures += 1;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    drop(share);
    sharer.session.leave();
    watcher.session.leave();
    instance.stop().await;
}
