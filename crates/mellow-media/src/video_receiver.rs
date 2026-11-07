//! Receiving cameras and screens: one decoding thread per video receive slot in use, fed by the
//! network thread through a short queue, each handing the newest picture to the interface at
//! the size its tile shows, converted for drawing. Nothing here runs on the interface's thread.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

use mellow_protocol::Snowflake;
use str0m::media::Mid;

use crate::video_frame::{DecodedPixelFormat, DecodedVideoFrame, I420View, fit_within};
use crate::vp8::Vp8Decoder;

/// Frames waiting for one decoder: a few frames of slack; beyond, the network thread drops
/// frames and asks for a keyframe rather than queueing seconds of stale video.
const DECODE_QUEUE_FRAMES: usize = 8;
/// libvpx splits a large picture's decoding over this many threads.
const DECODER_THREADS: u32 = 2;
/// A VP8 keyframe's start code, after its 3-byte frame tag (RFC 6386 §9.1).
const VP8_START_CODE: [u8; 3] = [0x9d, 0x01, 0x2a];

/// Whether `frame` is a VP8 keyframe a decoder can start from.
pub(crate) fn is_vp8_keyframe(frame: &[u8]) -> bool {
    frame.first().is_some_and(|tag| tag & 1 == 0) && frame.get(3..6) == Some(&VP8_START_CODE[..])
}

type Waker = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct SharedVideoState {
    /// Whose video each receive slot carries (`VOICE_SLOT_ASSIGNED`).
    slot_owners: HashMap<Mid, Snowflake>,
    /// The size each watched person's tile is drawn at, in physical pixels.
    display_sizes: HashMap<Snowflake, (u32, u32)>,
    /// The newest picture of each person, not yet taken by the interface.
    new_frames: HashMap<Snowflake, DecodedVideoFrame>,
    format: DecodedPixelFormat,
    waker: Option<Waker>,
}

/// The decoded pictures of the cameras and screens this session watches. Clone it to the
/// interface: it takes the newest picture of each person when it draws.
#[derive(Clone)]
pub struct VideoFrames {
    shared: Arc<Mutex<SharedVideoState>>,
}

impl std::fmt::Debug for VideoFrames {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VideoFrames")
            .finish_non_exhaustive()
    }
}

impl VideoFrames {
    pub(crate) fn new(format: DecodedPixelFormat) -> Self {
        Self {
            shared: Arc::new(Mutex::new(SharedVideoState {
                format,
                ..SharedVideoState::default()
            })),
        }
    }

    fn state(&self) -> MutexGuard<'_, SharedVideoState> {
        // Plain maps stay usable after a panic elsewhere.
        self.shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The pictures decoded since the last call, one per person (older ones were dropped).
    #[must_use]
    pub fn take_new_frames(&self) -> Vec<(Snowflake, DecodedVideoFrame)> {
        self.state().new_frames.drain().collect()
    }

    /// Called from a decoding thread after each new picture: ask the interface to draw.
    pub fn set_waker(&self, waker: impl Fn() + Send + Sync + 'static) {
        self.state().waker = Some(Arc::new(waker));
    }

    fn assign_slot(&self, mid: Mid, user_id: Snowflake) {
        self.state().slot_owners.insert(mid, user_id);
    }

    /// The people watched and their tiles' sizes; the others' pending pictures are dropped.
    pub(crate) fn set_display_sizes(&self, display_sizes: HashMap<Snowflake, (u32, u32)>) {
        let mut state = self.state();
        state
            .new_frames
            .retain(|user_id, _| display_sizes.contains_key(user_id));
        state.display_sizes = display_sizes;
    }

    /// Converts and publishes the picture of the person slot `mid` carries.
    fn publish(&self, mid: Mid, picture: I420View<'_>) {
        let (user_id, display_size, format) = {
            let state = self.state();
            let Some(&user_id) = state.slot_owners.get(&mid) else {
                return;
            };
            let Some(&display_size) = state.display_sizes.get(&user_id) else {
                return;
            };
            (user_id, display_size, state.format)
        };
        let (width, height) = fit_within((picture.width(), picture.height()), display_size);
        let scaled;
        let picture = if (width, height) == (picture.width(), picture.height()) {
            picture
        } else {
            scaled = picture.scaled(width, height);
            scaled.view()
        };
        let pixels = match format {
            DecodedPixelFormat::Bgra => picture.to_bgra(),
            DecodedPixelFormat::Nv12 => Some(picture.to_nv12()),
        };
        let Some(pixels) = pixels else {
            return;
        };
        let frame = DecodedVideoFrame {
            width,
            height,
            format,
            pixels: pixels.into(),
        };
        let waker = {
            let mut state = self.state();
            // Stopped watching while this picture was being converted.
            if !state.display_sizes.contains_key(&user_id) {
                return;
            }
            state.new_frames.insert(user_id, frame);
            state.waker.clone()
        };
        if let Some(waker) = waker {
            waker();
        }
    }
}

/// A received VP8 frame for a decoding thread.
#[derive(Debug)]
struct IncomingVideoFrame {
    bytes: Arc<[u8]>,
    /// False after a gap: the decoder must wait for a keyframe.
    is_contiguous: bool,
}

#[derive(Debug)]
struct DecodingThread {
    frames: SyncSender<IncomingVideoFrame>,
    has_dropped_a_frame: bool,
    thread: JoinHandle<()>,
}

/// The decoding threads, owned by the network thread.
#[derive(Debug)]
pub(crate) struct VideoReceivers {
    frames: VideoFrames,
    threads: HashMap<Mid, DecodingThread>,
    /// Decoding threads name the slot that needs a keyframe.
    keyframe_requests: Sender<Mid>,
}

impl VideoReceivers {
    pub(crate) fn new(frames: VideoFrames, keyframe_requests: Sender<Mid>) -> Self {
        Self {
            frames,
            threads: HashMap::new(),
            keyframe_requests,
        }
    }

    /// Hands a received frame of slot `mid` to its decoding thread, started on first use.
    pub(crate) fn deliver(&mut self, mid: Mid, bytes: Arc<[u8]>, is_contiguous: bool) {
        if !self.threads.contains_key(&mid) {
            let (sender, receiver) = std::sync::mpsc::sync_channel(DECODE_QUEUE_FRAMES);
            let frames = self.frames.clone();
            let keyframe_requests = self.keyframe_requests.clone();
            let spawned = std::thread::Builder::new()
                .name("mellow-video-decode".into())
                .spawn(move || decode_until_closed(mid, &receiver, &frames, &keyframe_requests));
            match spawned {
                Ok(thread) => {
                    self.threads.insert(
                        mid,
                        DecodingThread {
                            frames: sender,
                            has_dropped_a_frame: false,
                            thread,
                        },
                    );
                }
                Err(error) => {
                    tracing::warn!(%error, "a video decoding thread could not start");
                    return;
                }
            }
        }
        let Some(decoding) = self.threads.get_mut(&mid) else {
            return;
        };
        let frame = IncomingVideoFrame {
            bytes,
            is_contiguous: is_contiguous && !decoding.has_dropped_a_frame,
        };
        match decoding.frames.try_send(frame) {
            Ok(()) => decoding.has_dropped_a_frame = false,
            // The decoder is behind: the frames after this gap need a keyframe first.
            Err(TrySendError::Full(_)) => decoding.has_dropped_a_frame = true,
            Err(TrySendError::Disconnected(_)) => {
                self.threads.remove(&mid);
            }
        }
    }

    /// Slot `mid` now carries `user_id`'s camera or screen.
    pub(crate) fn assign_slot(&self, mid: Mid, user_id: Snowflake) {
        self.frames.assign_slot(mid, user_id);
    }

    /// A new peer: its slots are new lines, assigned again by the server.
    pub(crate) fn new_media_session(&mut self) {
        self.shut_down();
        self.frames.state().slot_owners.clear();
    }

    /// Stops and joins every decoding thread.
    pub(crate) fn shut_down(&mut self) {
        for (_, decoding) in self.threads.drain() {
            drop(decoding.frames);
            if decoding.thread.join().is_err() {
                tracing::error!("a video decoding thread panicked");
            }
        }
    }
}

impl Drop for VideoReceivers {
    fn drop(&mut self) {
        self.shut_down();
    }
}

fn decode_until_closed(
    mid: Mid,
    incoming: &Receiver<IncomingVideoFrame>,
    frames: &VideoFrames,
    keyframe_requests: &Sender<Mid>,
) {
    let mut decoder = match Vp8Decoder::new(DECODER_THREADS) {
        Ok(decoder) => decoder,
        Err(error) => {
            tracing::warn!(%error, "a VP8 decoder could not start");
            return;
        }
    };
    let mut is_waiting_for_keyframe = true;
    let request_keyframe = || {
        // The network thread gone means the session is ending.
        let _ = keyframe_requests.send(mid);
    };
    while let Ok(first) = incoming.recv() {
        // Every queued frame is decoded (each depends on the previous one), but only the newest
        // picture is converted and shown.
        let mut batch: Vec<IncomingVideoFrame> = vec![first];
        batch.extend(incoming.try_iter());
        let last_index = batch.len() - 1;
        for (index, frame) in batch.into_iter().enumerate() {
            if !frame.is_contiguous {
                is_waiting_for_keyframe = true;
                request_keyframe();
            }
            if is_waiting_for_keyframe {
                if !is_vp8_keyframe(&frame.bytes) {
                    // Asked again while waiting (the network thread allows one a second): an
                    // earlier request may have been lost or made before the connection.
                    request_keyframe();
                    continue;
                }
                is_waiting_for_keyframe = false;
            }
            match decoder.decode(&frame.bytes) {
                Ok(Some(picture)) if index == last_index => frames.publish(mid, picture),
                Ok(_) => {}
                Err(_) => {
                    is_waiting_for_keyframe = true;
                    request_keyframe();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use mellow_protocol::SimulcastLayer;

    use super::*;
    use crate::simulcast::{SimulcastEncoder, VideoSourceKind};
    use crate::video_frame::I420Picture;

    fn camera_frames(count: u32) -> Vec<Vec<u8>> {
        let mut encoder = SimulcastEncoder::new(VideoSourceKind::Camera);
        encoder.set_active_layers(&[SimulcastLayer::High]);
        (0..count)
            .map(|index| {
                let mut picture = I420Picture::black(320, 180);
                picture
                    .planes_mut()
                    .0
                    .fill(u8::try_from(40 + index).unwrap());
                let frames = encoder
                    .encode(picture.view(), u64::from(index) * 3_000)
                    .unwrap();
                frames.into_iter().next().unwrap().bytes
            })
            .collect()
    }

    #[test]
    fn keyframes_are_told_from_other_frames() {
        let frames = camera_frames(2);
        assert!(is_vp8_keyframe(&frames[0]));
        assert!(!is_vp8_keyframe(&frames[1]));
        assert!(!is_vp8_keyframe(&[]));
    }

    #[test]
    fn a_watched_slot_s_pictures_arrive_at_the_tile_s_size() {
        let frames = VideoFrames::new(DecodedPixelFormat::Bgra);
        let user_id = Snowflake::from_raw(7);
        let mid = Mid::from("20");
        frames.assign_slot(mid, user_id);
        frames.set_display_sizes(HashMap::from([(user_id, (160, 160))]));
        let (woken, was_woken) = mpsc::channel();
        frames.set_waker(move || {
            let _ = woken.send(());
        });
        let (keyframe_requests, _requested) = mpsc::channel();
        let mut receivers = VideoReceivers::new(frames.clone(), keyframe_requests);
        for frame in camera_frames(3) {
            receivers.deliver(mid, frame.into(), true);
        }
        was_woken.recv_timeout(Duration::from_secs(10)).unwrap();
        receivers.shut_down();
        let taken = frames.take_new_frames();
        let (owner, picture) = &taken[0];
        assert_eq!(*owner, user_id);
        assert_eq!(
            (picture.width, picture.height),
            (160, 90),
            "fitted, same shape"
        );
        assert_eq!(picture.pixels.len(), 160 * 90 * 4);
        assert_eq!(frames.take_new_frames(), [], "taken once");
    }

    #[test]
    fn a_gap_waits_for_a_keyframe_and_asks_for_one() {
        let frames = VideoFrames::new(DecodedPixelFormat::Nv12);
        let mid = Mid::from("21");
        let (keyframe_requests, requested) = mpsc::channel();
        let mut receivers = VideoReceivers::new(frames, keyframe_requests);
        let encoded = camera_frames(3);
        receivers.deliver(mid, encoded[0].clone().into(), true);
        receivers.deliver(mid, encoded[2].clone().into(), false);
        assert_eq!(
            requested.recv_timeout(Duration::from_secs(10)).unwrap(),
            mid
        );
        receivers.shut_down();
    }

    #[test]
    fn pictures_of_people_nobody_watches_are_not_kept() {
        let frames = VideoFrames::new(DecodedPixelFormat::Bgra);
        let user_id = Snowflake::from_raw(7);
        let mid = Mid::from("20");
        frames.assign_slot(mid, user_id);
        let picture = I420Picture::black(32, 18);
        frames.publish(mid, picture.view());
        assert_eq!(frames.take_new_frames(), []);
        frames.set_display_sizes(HashMap::from([(user_id, (64, 64))]));
        frames.publish(mid, picture.view());
        frames.set_display_sizes(HashMap::new());
        assert_eq!(frames.take_new_frames(), []);
    }
}
