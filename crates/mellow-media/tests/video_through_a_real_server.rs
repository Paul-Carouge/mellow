//! Two engines join a voice channel of a real in-process `mellow-server`: one turns on a
//! synthetic camera, encoded in VP8 simulcast; the other watches it and must receive decoded
//! pictures of the top layer at its tile's size. The server must ask the sender for the layers
//! watched. No real camera or audio device is touched.

// The server refuses Windows until its master key file can be protected there (ADR-0006).
#![cfg(not(windows))]
// Test helpers outside `#[test]` functions: failing loudly is the point.
#![allow(clippy::unwrap_used, clippy::panic)]

mod support;

use std::time::{Duration, Instant};

use mellow_media::{
    Cameras as _, DecodedPixelFormat, SyntheticCameras, SyntheticSound, VideoSourceKind,
    WatchedVideo,
};
use mellow_protocol::{GatewayEvent, SimulcastLayer, VideoTileSize};
use support::{
    HEARING_TIMEOUT, Participant, RunningInstance, identified_gateway, shared_voice_channel,
};

/// Small enough for unoptimised test builds to convert in real time.
const CAMERA_SIZE: (u32, u32) = (640, 360);
/// Larger than the camera: pictures arrive at the camera's size, never enlarged.
const TILE_SIZE: (u32, u32) = (1_280, 720);
/// Pictures to receive: about a second of video.
const PICTURES_EXPECTED: usize = 20;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_camera_is_encoded_forwarded_and_decoded_by_a_watcher() {
    let instance = RunningInstance::start().await;
    let (owner, camera_id) = instance.registered("camera").await;
    let (watcher_client, watcher_id) = instance.registered("watcher").await;
    let channel_id = shared_voice_channel(&owner, &watcher_client).await;
    let camera_gateway = identified_gateway(instance.listen_address, owner.token().unwrap()).await;
    let watcher_gateway =
        identified_gateway(instance.listen_address, watcher_client.token().unwrap()).await;
    let mut camera = Participant::join(
        camera_gateway,
        channel_id,
        camera_id,
        SyntheticSound::Silence,
    );
    let mut watcher = Participant::join(
        watcher_gateway,
        channel_id,
        watcher_id,
        SyntheticSound::Silence,
    );
    camera.wait_until_connected().await;
    watcher.wait_until_connected().await;

    let input = camera.session.start_video(VideoSourceKind::Camera);
    let synthetic = SyntheticCameras {
        width: CAMERA_SIZE.0,
        height: CAMERA_SIZE.1,
    };
    let running_camera = synthetic.open(None, input).unwrap();
    watcher
        .wait_for_event(|event| {
            matches!(
                event,
                GatewayEvent::VoiceStateUpdate { voice_state }
                    if voice_state.user_id == camera_id && voice_state.self_video
            )
        })
        .await;
    let frames = watcher.session.video_frames();
    watcher.session.watch_videos(&[WatchedVideo {
        user_id: camera_id,
        tile_size: VideoTileSize::Large,
        display_width: TILE_SIZE.0,
        display_height: TILE_SIZE.1,
    }]);

    let deadline = Instant::now() + HEARING_TIMEOUT;
    let mut pictures = Vec::new();
    let mut is_top_layer_asked = false;
    while pictures.len() < PICTURES_EXPECTED || !is_top_layer_asked {
        assert!(
            Instant::now() < deadline,
            "pictures: {}, top layer asked: {is_top_layer_asked}, camera events: {:?}, watcher \
             events: {:?}",
            pictures.len(),
            camera.seen_session_events,
            watcher.seen_session_events,
        );
        is_top_layer_asked |= camera.pump().iter().any(|event| {
            matches!(
                event,
                GatewayEvent::VoiceVideoLayers { active, .. } if active.contains(&SimulcastLayer::High)
            )
        });
        watcher.pump();
        pictures.extend(frames.take_new_frames());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert!(pictures.iter().all(|(user_id, _)| *user_id == camera_id));
    let (_, last) = pictures.last().unwrap();
    assert_eq!((last.width, last.height), CAMERA_SIZE, "the top layer");
    assert_eq!(last.format, DecodedPixelFormat::Bgra);
    assert_eq!(
        last.pixels.len(),
        (CAMERA_SIZE.0 * CAMERA_SIZE.1 * 4) as usize
    );
    // The test pattern's bands are grey to bright: a black or empty picture means a broken
    // decode or conversion.
    let brightest = last.pixels.iter().step_by(4).copied().max().unwrap();
    assert!(brightest > 150, "brightest blue sample: {brightest}");

    drop(running_camera);
    camera.session.stop_video();
    camera.session.leave();
    watcher.session.leave();
    instance.stop().await;
}
