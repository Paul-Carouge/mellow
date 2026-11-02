//! Cameras through the real gateway and UDP port (ADR-0035), with headless `str0m` clients that
//! send three synthetic VP8 simulcast layers and answer keyframe requests, as in the ADR's spike.

use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mellow_protocol::{
    ClientFrame, GatewayEvent, Permissions, SimulcastLayer, Snowflake, VideoTileSize,
    VoiceVideoRefusal, WatchedCamera,
};
use str0m::change::{SdpAnswer, SdpPendingOffer};
use str0m::format::Codec;
use str0m::media::{Direction, Frequency, MediaKind, MediaTime, Mid, Rid, Simulcast};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};

use super::tests::{Fixture, HEARING_TIMEOUT, fixture, join, next_voice_state_update, pending_bot};
use crate::gateway::tests::{ClientSocket, identified, receive_dispatch, send};

const VIDEO_FRAME_INTERVAL: Duration = Duration::from_millis(33);
const VIDEO_CLOCK_TICKS_PER_FRAME: u64 = 3_000;
const VIDEO_RECEIVE_SLOTS: usize = 2;
/// The RIDs the web client sends, highest first.
const LAYER_RIDS: [&str; 3] = ["h", "l", "q"];
/// Where the synthetic frame carries its camera's marker and its layer's RID.
const MARKER_OFFSET: usize = 10;
const LAYER_OFFSET: usize = 11;
const FRAME_BYTES: usize = 16;
/// Long enough for several frames of every layer to arrive if they were still forwarded.
const SETTLE_TIME: Duration = Duration::from_millis(300);

/// A frame the VP8 depacketizer accepts: a frame tag (bit 0 clear on keyframes), for keyframes
/// the start code and a picture size, then the camera's marker and the layer.
fn synthetic_vp8_frame(marker: u8, rid: &str, is_keyframe: bool, length: usize) -> Vec<u8> {
    let mut frame = vec![0_u8; length.max(FRAME_BYTES)];
    frame[0] = if is_keyframe { 0x10 } else { 0x11 };
    if is_keyframe {
        frame[3..6].copy_from_slice(&[0x9d, 0x01, 0x2a]);
        frame[6..8].copy_from_slice(&320_u16.to_le_bytes());
        frame[8..10].copy_from_slice(&180_u16.to_le_bytes());
    }
    frame[MARKER_OFFSET] = marker;
    frame[LAYER_OFFSET] = rid.as_bytes()[0];
    frame
}

struct PendingCameraBot {
    rtc: Rtc,
    pending: SdpPendingOffer,
    camera_mid: Mid,
    socket: UdpSocket,
    offer: String,
}

/// An offer as the web client makes it with video: a voice line and one voice slot, then a
/// simulcast camera and `VIDEO_RECEIVE_SLOTS` camera slots.
fn pending_camera_bot() -> PendingCameraBot {
    str0m::crypto::from_feature_flags().install_process_default();
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut rtc = Rtc::builder()
        .clear_codecs()
        .enable_opus(true, false)
        .enable_vp8(true)
        .build(Instant::now());
    rtc.add_local_candidate(Candidate::host(socket.local_addr().unwrap(), "udp").unwrap());
    let mut change = rtc.sdp_api();
    change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
    change.add_media(MediaKind::Audio, Direction::RecvOnly, None, None, None);
    let mut simulcast = Simulcast::new();
    for rid in LAYER_RIDS {
        simulcast.add_send_layer(str0m::media::SimulcastLayer::new(rid));
    }
    let camera_mid = change.add_media(
        MediaKind::Video,
        Direction::SendOnly,
        None,
        None,
        Some(simulcast),
    );
    for _ in 0..VIDEO_RECEIVE_SLOTS {
        change.add_media(MediaKind::Video, Direction::RecvOnly, None, None, None);
    }
    let (offer, pending) = change.apply().unwrap();
    PendingCameraBot {
        rtc,
        pending,
        camera_mid,
        socket,
        offer: offer.to_sdp_string(),
    }
}

/// Frames received, by camera marker and layer RID byte.
type ReceivedFrames = HashMap<(u8, u8), u32>;

/// A running camera: sends its three layers 30 times a second (a keyframe first, and again for a
/// layer the server asks one of), and counts the camera frames it receives.
struct CameraBot {
    received: Arc<Mutex<ReceivedFrames>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl CameraBot {
    fn received(&self, marker: u8, rid: &str) -> u32 {
        self.received
            .lock()
            .unwrap()
            .get(&(marker, rid.as_bytes()[0]))
            .copied()
            .unwrap_or(0)
    }

    async fn wait_until_receiving(&self, marker: u8, rid: &str) {
        let deadline = Instant::now() + HEARING_TIMEOUT;
        while self.received(marker, rid) == 0 {
            assert!(
                Instant::now() < deadline,
                "never received layer {rid} of {marker}"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }
}

impl Drop for CameraBot {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

async fn join_with_camera(
    socket: &mut ClientSocket,
    channel_id: Snowflake,
    marker: u8,
) -> CameraBot {
    join_with_camera_sending(socket, channel_id, marker, FRAME_BYTES).await
}

/// A camera whose `q` frames are `quarter_frame_bytes` long, as a client lying about its layers.
async fn join_with_camera_sending(
    socket: &mut ClientSocket,
    channel_id: Snowflake,
    marker: u8,
    quarter_frame_bytes: usize,
) -> CameraBot {
    let PendingCameraBot {
        mut rtc,
        pending,
        camera_mid,
        socket: udp,
        offer,
    } = pending_camera_bot();
    let answer = join(socket, channel_id, &offer).await;
    rtc.sdp_api()
        .accept_answer(pending, SdpAnswer::from_sdp_string(&answer).unwrap())
        .unwrap();
    let received = Arc::new(Mutex::new(HashMap::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let (received, stop) = (received.clone(), stop.clone());
        std::thread::spawn(move || {
            run_camera_bot(
                rtc,
                &udp,
                (camera_mid, marker, quarter_frame_bytes),
                &received,
                &stop,
            );
        })
    };
    CameraBot {
        received,
        stop,
        thread: Some(thread),
    }
}

fn run_camera_bot(
    mut rtc: Rtc,
    socket: &UdpSocket,
    (camera_mid, marker, quarter_frame_bytes): (Mid, u8, usize),
    received: &Mutex<ReceivedFrames>,
    stop: &AtomicBool,
) {
    let local_address = socket.local_addr().unwrap();
    let mut buffer = vec![0_u8; 2_000];
    let mut is_connected = false;
    let mut next_frame_at = Instant::now();
    let mut rtp_time: u64 = 0;
    let mut is_keyframe_due = [true; LAYER_RIDS.len()];
    while !stop.load(Ordering::Relaxed) {
        let timeout = loop {
            match rtc.poll_output().unwrap() {
                Output::Timeout(at) => break at,
                Output::Transmit(transmit) => {
                    socket
                        .send_to(&transmit.contents, transmit.destination)
                        .unwrap();
                }
                Output::Event(Event::Connected) => is_connected = true,
                Output::Event(Event::KeyframeRequest(request)) => {
                    for (is_due, rid) in is_keyframe_due.iter_mut().zip(LAYER_RIDS) {
                        if request.rid.is_none_or(|asked| &*asked == rid) {
                            *is_due = true;
                        }
                    }
                }
                Output::Event(Event::MediaData(frame)) if frame.data.len() >= FRAME_BYTES => {
                    let key = (frame.data[MARKER_OFFSET], frame.data[LAYER_OFFSET]);
                    *received.lock().unwrap().entry(key).or_default() += 1;
                }
                Output::Event(_) => {}
            }
        };
        let now = Instant::now();
        if is_connected && now >= next_frame_at {
            for (rid, is_due) in LAYER_RIDS.iter().zip(is_keyframe_due.iter_mut()) {
                let Some(writer) = rtc.writer(camera_mid) else {
                    break;
                };
                let payload_type = writer
                    .payload_params()
                    .find(|params| params.spec().codec == Codec::Vp8)
                    .map(str0m::format::PayloadParams::pt)
                    .unwrap();
                writer
                    .rid(Rid::from(*rid))
                    .write(
                        payload_type,
                        now,
                        MediaTime::new(rtp_time, Frequency::NINETY_KHZ),
                        synthetic_vp8_frame(
                            marker,
                            rid,
                            *is_due,
                            if *rid == "q" {
                                quarter_frame_bytes
                            } else {
                                FRAME_BYTES
                            },
                        ),
                    )
                    .unwrap();
                *is_due = false;
            }
            rtp_time += VIDEO_CLOCK_TICKS_PER_FRAME;
            next_frame_at += VIDEO_FRAME_INTERVAL;
            continue;
        }
        let wake_at = if is_connected {
            timeout.min(next_frame_at)
        } else {
            timeout
        };
        let wait = wake_at
            .saturating_duration_since(Instant::now())
            .clamp(Duration::from_millis(1), Duration::from_millis(50));
        socket.set_read_timeout(Some(wait)).unwrap();
        let input = match socket.recv_from(&mut buffer) {
            Ok((length, source)) => Input::Receive(
                Instant::now(),
                Receive {
                    proto: Protocol::Udp,
                    source,
                    destination: local_address,
                    contents: buffer[..length].try_into().unwrap(),
                },
            ),
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                Input::Timeout(Instant::now())
            }
            Err(error) => panic!("camera bot {marker}: {error}"),
        };
        rtc.handle_input(input).unwrap();
    }
}

fn camera_state(self_video: bool) -> ClientFrame {
    ClientFrame::VoiceState {
        self_mute: false,
        self_deaf: false,
        self_video,
    }
}

fn watch(user_id: Snowflake, size: VideoTileSize) -> ClientFrame {
    ClientFrame::VoiceVideoWatch {
        cameras: vec![WatchedCamera { user_id, size }],
    }
}

/// Reads `socket` until `user_id`'s voice state shows their camera as `self_video`.
async fn wait_for_camera(socket: &mut ClientSocket, user_id: Snowflake, self_video: bool) {
    while next_voice_state_update(socket, user_id).await.self_video != self_video {}
}

async fn next_layers(socket: &mut ClientSocket) -> Vec<SimulcastLayer> {
    match receive_dispatch(socket, |event| {
        matches!(event, GatewayEvent::VoiceVideoLayers { .. })
    })
    .await
    .1
    {
        GatewayEvent::VoiceVideoLayers { active, .. } => active,
        other => unreachable!("{other:?}"),
    }
}

async fn next_video_refusal(socket: &mut ClientSocket) -> VoiceVideoRefusal {
    match receive_dispatch(socket, |event| {
        matches!(event, GatewayEvent::VoiceVideoRefused { .. })
    })
    .await
    .1
    {
        GatewayEvent::VoiceVideoRefused { reason, .. } => reason,
        other => unreachable!("{other:?}"),
    }
}

async fn camera_slot_of(socket: &mut ClientSocket, camera_id: Snowflake) -> String {
    match receive_dispatch(socket, |event| {
        matches!(event, GatewayEvent::VoiceSlotAssigned { user_id, .. } if *user_id == camera_id)
    })
    .await
    .1
    {
        GatewayEvent::VoiceSlotAssigned { mid, .. } => mid,
        other => unreachable!("{other:?}"),
    }
}

async fn identified_pair(fixture: &Fixture) -> (ClientSocket, ClientSocket) {
    let (owner, _) = identified(fixture.address, fixture.token("owner")).await;
    let (member, _) = identified(fixture.address, fixture.token("member")).await;
    (owner, member)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_watcher_gets_the_layer_of_its_tile_and_switches_at_a_keyframe() {
    let fixture = fixture().await;
    let (mut owner, mut member) = identified_pair(&fixture).await;
    let owner_id = fixture.user_id("owner");
    let owner_bot = join_with_camera(&mut owner, fixture.lounge, 1).await;
    let member_bot = join_with_camera(&mut member, fixture.lounge, 2).await;

    send(&mut owner, &camera_state(true)).await;
    wait_for_camera(&mut member, owner_id, true).await;
    let layers_unwatched = next_layers(&mut owner).await;
    send(&mut member, &watch(owner_id, VideoTileSize::Small)).await;
    let slot = camera_slot_of(&mut member, owner_id).await;
    member_bot.wait_until_receiving(1, "q").await;
    send(&mut member, &watch(owner_id, VideoTileSize::Large)).await;
    let layers_watched_large = next_layers(&mut owner).await;
    member_bot.wait_until_receiving(1, "h").await;
    let small_frames_at_switch = member_bot.received(1, "q");
    tokio::time::sleep(SETTLE_TIME).await;
    let small_frames_after = member_bot.received(1, "q");
    let (medium_frames, own_frames) = (member_bot.received(1, "l"), owner_bot.received(1, "h"));
    drop((owner_bot, member_bot));
    fixture.close().await;

    assert_eq!(layers_unwatched, [SimulcastLayer::Quarter]);
    assert_ne!(slot, "", "the camera got a video slot");
    assert_eq!(
        layers_watched_large,
        [SimulcastLayer::Quarter, SimulcastLayer::High]
    );
    assert_eq!(
        small_frames_after, small_frames_at_switch,
        "after the switch, only the large layer flows"
    );
    assert_eq!(medium_frames, 0, "no tile asked for the medium layer");
    assert_eq!(own_frames, 0, "a camera is never sent back to its owner");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_camera_needs_stream_and_video_lines_and_stops_when_stream_is_taken_away() {
    let fixture = fixture().await;
    let (mut owner, mut member) = identified_pair(&fixture).await;
    let (mut muted, _) = identified(fixture.address, fixture.token("muted")).await;
    let member_id = fixture.user_id("member");
    let owner_bot = join_with_camera(&mut owner, fixture.lounge, 1).await;
    let member_bot = join_with_camera(&mut member, fixture.lounge, 2).await;
    // An audio-only offer, as from a client without video.
    join(&mut muted, fixture.lounge, &pending_bot().offer).await;

    send(&mut muted, &camera_state(true)).await;
    let without_video_lines = next_video_refusal(&mut muted).await;
    send(&mut member, &camera_state(true)).await;
    wait_for_camera(&mut owner, member_id, true).await;
    send(&mut owner, &watch(member_id, VideoTileSize::Medium)).await;
    owner_bot.wait_until_receiving(2, "l").await;
    let layers_while_on = next_layers(&mut member).await;
    fixture.deny_in_lounge("member", Permissions::STREAM).await;
    wait_for_camera(&mut owner, member_id, false).await;
    // The media loop turned the camera off when it reports no layers; then only frames already
    // in flight may still arrive.
    while !next_layers(&mut member).await.is_empty() {}
    tokio::time::sleep(SETTLE_TIME).await;
    let frames_when_stopped = owner_bot.received(2, "l");
    tokio::time::sleep(SETTLE_TIME).await;
    let frames_later = owner_bot.received(2, "l");
    send(&mut member, &camera_state(true)).await;
    let without_stream = next_video_refusal(&mut member).await;
    drop((owner_bot, member_bot));
    fixture.close().await;

    assert_eq!(without_video_lines, VoiceVideoRefusal::VideoUnavailable);
    assert_eq!(layers_while_on, [SimulcastLayer::Quarter]);
    assert_eq!(
        frames_later, frames_when_stopped,
        "nothing flows without STREAM"
    );
    assert_eq!(without_stream, VoiceVideoRefusal::MissingPermissions);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_camera_replacing_another_in_the_tiles_takes_its_slot_and_the_first_stops() {
    let fixture = fixture().await;
    let (mut owner, mut member) = identified_pair(&fixture).await;
    let (mut muted, _) = identified(fixture.address, fixture.token("muted")).await;
    let (owner_id, member_id) = (fixture.user_id("owner"), fixture.user_id("member"));
    let owner_bot = join_with_camera(&mut owner, fixture.lounge, 1).await;
    let member_bot = join_with_camera(&mut member, fixture.lounge, 2).await;
    let watcher_bot = join_with_camera(&mut muted, fixture.lounge, 3).await;
    send(&mut owner, &camera_state(true)).await;
    send(&mut member, &camera_state(true)).await;
    // Read together: waiting for one camera skips the other's event.
    let mut cameras_seen_on = HashSet::new();
    while cameras_seen_on.len() < 2 {
        if let GatewayEvent::VoiceStateUpdate { voice_state } =
            receive_dispatch(&mut muted, |event| {
                matches!(event, GatewayEvent::VoiceStateUpdate { voice_state } if voice_state.self_video)
            })
            .await
            .1
        {
            cameras_seen_on.insert(voice_state.user_id);
        }
    }

    send(&mut muted, &watch(owner_id, VideoTileSize::Small)).await;
    let owner_slot = camera_slot_of(&mut muted, owner_id).await;
    watcher_bot.wait_until_receiving(1, "q").await;
    send(&mut muted, &watch(member_id, VideoTileSize::Small)).await;
    let member_slot = camera_slot_of(&mut muted, member_id).await;
    watcher_bot.wait_until_receiving(2, "q").await;
    // The owner's camera is no longer watched once the member's frames arrive on its slot.
    tokio::time::sleep(SETTLE_TIME).await;
    let owner_frames_after_switch = watcher_bot.received(1, "q");
    tokio::time::sleep(SETTLE_TIME).await;
    let owner_frames_later = watcher_bot.received(1, "q");
    drop((owner_bot, member_bot, watcher_bot));
    fixture.close().await;

    assert_eq!(member_slot, owner_slot, "the freed slot is given again");
    assert_eq!(owner_frames_later, owner_frames_after_switch);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_layer_sent_far_above_its_bit_rate_reaches_watchers_only_up_to_its_budget() {
    // Five times the 37 500 bytes per second `q` may send.
    const OVERSIZED_FRAME_BYTES: u32 = 6_000;
    let fixture = fixture().await;
    let (mut owner, mut member) = identified_pair(&fixture).await;
    let owner_id = fixture.user_id("owner");
    let started = Instant::now();
    let owner_bot = join_with_camera_sending(
        &mut owner,
        fixture.lounge,
        1,
        OVERSIZED_FRAME_BYTES as usize,
    )
    .await;
    let member_bot = join_with_camera(&mut member, fixture.lounge, 2).await;
    send(&mut owner, &camera_state(true)).await;
    wait_for_camera(&mut member, owner_id, true).await;
    send(&mut member, &watch(owner_id, VideoTileSize::Small)).await;
    member_bot.wait_until_receiving(1, "q").await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let received = member_bot.received(1, "q");
    let elapsed = started.elapsed();
    drop((owner_bot, member_bot));
    fixture.close().await;

    // Never more than a second's worth to start, plus the refill since the camera joined.
    let allowed_bytes = elapsed.as_secs_f64().mul_add(37_500.0, 37_500.0);
    assert!(
        f64::from(received * OVERSIZED_FRAME_BYTES) <= allowed_bytes,
        "{received} frames in {elapsed:?}"
    );
}
