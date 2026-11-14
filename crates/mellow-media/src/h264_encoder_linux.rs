//! A shared screen's three H.264 layers encoded on the GPU through VA-API (Vesper's
//! `vesper-encode` in its real-time mode, ADR-0054): DMA-BUF pictures from the portal stay on
//! the GPU, the top layer is encoded at the quality's size and the two lower ones are scaled from
//! it on the GPU. Runs on its own thread, fed the newest picture through a slot where the newest
//! picture wins, so the capture callback never waits for it.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mellow_protocol::SimulcastLayer;
use vesper_core::VideoFrame;
use vesper_encode::Encoder;

use crate::h264::{is_h264_keyframe, nal_unit_type, nal_units};
use crate::h264_stream::{ParameterSets, is_constrained_baseline_sequence_parameter_set};
use crate::screen_capture::ScreenShareContext;
use crate::session::ScreenShareEnd;
use crate::simulcast::{
    FRAMES_PER_SECOND, HARDWARE_SCREEN_FRAMES_PER_SECOND, LAYERS, ScreenShareQuality,
    VideoSourceKind, layer_size,
};
use crate::video_frame::fit_within;
use crate::video_sender::{EncodedVideoInput, EncoderRequests};

/// Without a new picture (the portal sends one only when the screen changes), the last one is
/// encoded again this often, as the VP8 path does: late watchers get a picture and the server
/// never sees the screen as silent.
const LONGEST_INTERVAL_BETWEEN_FRAMES: Duration = Duration::from_secs(1);
/// How often the thread looks at the server's requests while no picture comes.
const REQUEST_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// The size and rate of the probe that tells whether the GPU encodes Constrained Baseline.
const PROBE_SIZE: (u32, u32) = (640, 360);
const PROBE_BITS_PER_SECOND: u64 = 500_000;
const SEQUENCE_PARAMETER_SET: u8 = 7;

/// Whether this machine's VA-API driver opens a real-time Constrained Baseline H.264 encoder.
pub(crate) fn can_encode_h264() -> bool {
    match Encoder::new_real_time_h264(
        PROBE_SIZE.0,
        PROBE_SIZE.1,
        FRAMES_PER_SECOND,
        PROBE_BITS_PER_SECOND,
    ) {
        Ok(_) => true,
        Err(error) => {
            tracing::info!(%error, "no VA-API H.264 encoder: screens are shared in VP8");
            false
        }
    }
}

/// The newest picture the capture handed over and not taken yet, shared with the encoding
/// thread: a new picture replaces a waiting one, which goes back to the compositor at once.
struct LatestPicture<P> {
    state: Mutex<LatestState<P>>,
    changed: Condvar,
}

struct LatestState<P> {
    picture: Option<P>,
    is_closed: bool,
}

/// What [`LatestPicture::take`] found.
#[derive(Debug, PartialEq, Eq)]
enum Taken<P> {
    Picture(P),
    /// Nothing came in time.
    Nothing,
    /// The capture is gone.
    Closed,
}

impl<P> LatestPicture<P> {
    fn new() -> Self {
        Self {
            state: Mutex::new(LatestState {
                picture: None,
                is_closed: false,
            }),
            changed: Condvar::new(),
        }
    }

    fn state(&self) -> MutexGuard<'_, LatestState<P>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Never blocks longer than the encoding thread holds the lock to take a picture.
    fn offer(&self, picture: P) {
        let replaced = {
            let mut state = self.state();
            if state.is_closed {
                return;
            }
            state.picture.replace(picture)
        };
        // Released outside the lock: a DMA-BUF goes back to the compositor.
        drop(replaced);
        self.changed.notify_one();
    }

    fn close(&self) {
        self.state().is_closed = true;
        self.changed.notify_one();
    }

    /// The picture waiting, or the next one within `timeout`.
    fn take(&self, timeout: Duration) -> Taken<P> {
        let state = self.state();
        let (mut state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |state| {
                state.picture.is_none() && !state.is_closed
            })
            .unwrap_or_else(PoisonError::into_inner);
        match state.picture.take() {
            Some(picture) => Taken::Picture(picture),
            None if state.is_closed => Taken::Closed,
            None => Taken::Nothing,
        }
    }
}

/// Where the capture puts its pictures: the newest one wins, a picture it replaces is released
/// at once (and goes back to the compositor).
pub(crate) struct H264PictureSlot {
    latest: Arc<LatestPicture<VideoFrame>>,
}

impl H264PictureSlot {
    /// Never waits for the encoder.
    pub(crate) fn offer(&self, frame: VideoFrame) {
        self.latest.offer(frame);
    }
}

impl Drop for H264PictureSlot {
    fn drop(&mut self) {
        self.latest.close();
    }
}

/// The encoding thread; dropping it waits for it, once its input closed.
pub(crate) struct H264ScreenEncoder {
    thread: Option<JoinHandle<()>>,
}

impl Drop for H264ScreenEncoder {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("the screen's H.264 encoding thread panicked");
        }
    }
}

/// Starts encoding into `input` what arrives in the returned slot, until `input` closes or the
/// slot is dropped. A failure ends the share through `share`.
pub(crate) fn start(
    input: EncodedVideoInput,
    share: ScreenShareContext,
) -> std::io::Result<(H264ScreenEncoder, H264PictureSlot)> {
    let latest = Arc::new(LatestPicture::new());
    let pictures = Arc::clone(&latest);
    let thread = std::thread::Builder::new()
        .name("mellow-screen-h264".into())
        .spawn(move || {
            if let Err(error) = run(input, &pictures) {
                share.end(ScreenShareEnd::Failed(error));
            }
        })?;
    Ok((
        H264ScreenEncoder {
            thread: Some(thread),
        },
        H264PictureSlot { latest },
    ))
}

/// One layer's encoder and what it needs to send its frames.
struct LayerEncoder {
    layer: SimulcastLayer,
    encoder: Encoder,
    parameter_sets: ParameterSets,
    is_active: bool,
    /// Whether this layer's first SPS was checked to be Constrained Baseline.
    is_profile_checked: bool,
}

/// The source's rate is counted over this last stretch of pictures.
const SOURCE_RATE_WINDOW: Duration = Duration::from_secs(1);
/// Pictures a second from which the top layer moves to 60, and to which it comes back to 30:
/// apart, so a source near 40 never makes it switch back and forth.
const SOURCE_RATE_FOR_SIXTY: usize = 45;
const SOURCE_RATE_FOR_THIRTY: usize = 35;
/// How long the source's rate must hold before the encoder follows it: a switch reopens the
/// encoders and costs a keyframe on each layer, so at most one a second up, one in two down.
const SUSTAINED_BEFORE_SIXTY: Duration = Duration::from_secs(1);
const SUSTAINED_BEFORE_THIRTY: Duration = Duration::from_secs(2);
/// Pictures counted at most: a source faster than the portal's 60 is not counted further.
const MAXIMUM_COUNTED_PICTURES: usize = 2 * HARDWARE_SCREEN_FRAMES_PER_SECOND as usize;

/// The top layer's encoder rate, 30 or 60 pictures a second, following the source's measured
/// rate with hysteresis (ADR-0058). The GPU's constant rate gives each picture a fixed share
/// of the second's bits: at 60 a source changing 30 times a second would get half the rate,
/// at 30 a 60 one would have its pictures squeezed by the rate control.
#[derive(Debug)]
struct SourceRate {
    pictures: VecDeque<Instant>,
    encoder_rate: u32,
    /// Since when the source's rate asks for the other encoder rate.
    asking_since: Option<Instant>,
}

impl SourceRate {
    fn new() -> Self {
        Self {
            pictures: VecDeque::with_capacity(MAXIMUM_COUNTED_PICTURES),
            encoder_rate: FRAMES_PER_SECOND,
            asking_since: None,
        }
    }

    /// The encoders could not switch: stays at `frames_per_second`, and asks again only once the
    /// source's rate held for a whole window again.
    fn keep(&mut self, frames_per_second: u32) {
        self.encoder_rate = frames_per_second;
        self.asking_since = None;
    }

    /// Counts a new picture at `now` and returns the encoder rate to use for it.
    fn picture(&mut self, now: Instant) -> u32 {
        if self.pictures.len() == MAXIMUM_COUNTED_PICTURES {
            self.pictures.pop_front();
        }
        self.pictures.push_back(now);
        while self
            .pictures
            .front()
            .is_some_and(|&at| now.saturating_duration_since(at) >= SOURCE_RATE_WINDOW)
        {
            self.pictures.pop_front();
        }
        let rate = self.pictures.len();
        let (asks_other, sustained, other) = if self.encoder_rate == FRAMES_PER_SECOND {
            (
                rate >= SOURCE_RATE_FOR_SIXTY,
                SUSTAINED_BEFORE_SIXTY,
                HARDWARE_SCREEN_FRAMES_PER_SECOND,
            )
        } else {
            (
                rate <= SOURCE_RATE_FOR_THIRTY,
                SUSTAINED_BEFORE_THIRTY,
                FRAMES_PER_SECOND,
            )
        };
        if !asks_other {
            self.asking_since = None;
            return self.encoder_rate;
        }
        let since = *self.asking_since.get_or_insert(now);
        if now.saturating_duration_since(since) >= sustained {
            self.encoder_rate = other;
            self.asking_since = None;
        }
        self.encoder_rate
    }
}

/// The top layer's bit rate at `frames_per_second`: the 30 fps budget, or the 60 fps one.
fn top_bits_per_second(quality: ScreenShareQuality, frames_per_second: u32) -> u32 {
    if frames_per_second == HARDWARE_SCREEN_FRAMES_PER_SECOND {
        quality.hardware_top_bits_per_second()
    } else {
        quality.top_bits_per_second()
    }
}

/// Tokens a bucket holds at most: a picture taken a little late keeps what it earned meanwhile
/// (with one token at most, every late wake-up would lower the rate), and a pause never banks a
/// burst.
const MAXIMUM_TOKENS: f64 = 1.5;
/// Forced encodes (keyframes asked for, a still screen's repeats) borrow at most this much.
const MINIMUM_TOKENS: f64 = -1.0;
/// A token earned up to floating-point rounding: the thread wakes when it is due.
const TOKEN_ROUNDING: f64 = 1e-9;

/// A token bucket refilled at `rate` a second and holding one and a half at most: what takes from it
/// averages `rate` a second at most whatever rhythm it asks at. A shortest interval would bound
/// the gap only: 30 ms apart is 33 a second, 11 % over a rate budgeted per picture.
#[derive(Debug, Clone, Copy)]
struct TokenBucket {
    rate: f64,
    tokens: f64,
    refilled_at: Option<Instant>,
}

impl TokenBucket {
    fn full(rate: u32) -> Self {
        Self {
            rate: f64::from(rate.max(1)),
            tokens: 1.0,
            refilled_at: None,
        }
    }

    fn tokens_at(&self, now: Instant) -> f64 {
        let earned = self.refilled_at.map_or(0.0, |at| {
            now.saturating_duration_since(at).as_secs_f64() * self.rate
        });
        (self.tokens + earned).min(MAXIMUM_TOKENS)
    }

    fn has_token(&self, now: Instant) -> bool {
        self.tokens_at(now) >= 1.0 - TOKEN_ROUNDING
    }

    /// Takes a token at `now`, borrowing one when there is none (a forced encode).
    fn take(&mut self, now: Instant) {
        self.tokens = (self.tokens_at(now) - 1.0).max(MINIMUM_TOKENS);
        self.refilled_at = Some(now);
    }

    /// How long until a token is there.
    fn until_token(&self, now: Instant) -> Duration {
        let missing = 1.0 - self.tokens_at(now);
        if missing <= TOKEN_ROUNDING {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(missing / self.rate)
        }
    }
}

/// What one pass of the encoding thread encodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EncodePlan {
    /// The top layer: a new picture, or the last one again.
    top: bool,
    /// Whether the top layer takes the picture waiting (else it repeats its last one).
    takes_picture: bool,
    /// The lower layers, from the top layer's last picture.
    lower: bool,
}

/// When the layers encode: the top layer takes the waiting picture at most at the encoder's
/// rate on average (so its constant rate, budgeted per picture, holds), and repeats its last
/// picture once a second when the screen is still; the lower layers follow it at 30 a second
/// at most on average, whatever rhythm the pictures come at.
#[derive(Debug)]
struct Pacing {
    top: TokenBucket,
    lower: TokenBucket,
    last_top: Option<Instant>,
    /// A top picture the lower layers have not taken yet.
    is_lower_pending: bool,
}

impl Pacing {
    fn new(frames_per_second: u32) -> Self {
        Self {
            top: TokenBucket::full(frames_per_second),
            lower: TokenBucket::full(FRAMES_PER_SECOND),
            last_top: None,
            is_lower_pending: false,
        }
    }

    /// New encoders at `frames_per_second`: their first picture goes at once.
    fn encoders_opened(&mut self, frames_per_second: u32) {
        self.top = TokenBucket::full(frames_per_second);
        self.last_top = None;
    }

    /// What to encode at `now`, given a picture waiting or a keyframe asked for.
    fn plan(
        &mut self,
        now: Instant,
        has_waiting_picture: bool,
        is_keyframe_asked: bool,
    ) -> EncodePlan {
        let is_repeat_due = self
            .last_top
            .is_some_and(|at| now.saturating_duration_since(at) >= LONGEST_INTERVAL_BETWEEN_FRAMES);
        let takes_picture = has_waiting_picture && self.top.has_token(now);
        let top = takes_picture || is_keyframe_asked || is_repeat_due;
        if top {
            self.top.take(now);
            self.last_top = Some(now);
            self.is_lower_pending = true;
        }
        let lower = self.is_lower_pending && self.lower.has_token(now);
        if lower {
            self.lower.take(now);
            self.is_lower_pending = false;
        }
        EncodePlan {
            top,
            takes_picture,
            lower,
        }
    }

    /// How long the thread may wait for a picture before something is due.
    fn wait(&self, now: Instant, has_waiting_picture: bool) -> Duration {
        let mut wait = REQUEST_POLL_INTERVAL;
        if has_waiting_picture {
            wait = wait.min(self.top.until_token(now));
        }
        if let Some(at) = self.last_top {
            wait = wait.min((at + LONGEST_INTERVAL_BETWEEN_FRAMES).saturating_duration_since(now));
        }
        if self.is_lower_pending {
            wait = wait.min(self.lower.until_token(now));
        }
        wait
    }
}

/// The three layers of one source size and quality.
struct Layers {
    kind: VideoSourceKind,
    source_size: (u32, u32),
    /// The top layer's encoder rate.
    frames_per_second: u32,
    /// Top first: the lower ones scale its last picture.
    encoders: Vec<LayerEncoder>,
}

impl Layers {
    fn open(
        kind: VideoSourceKind,
        source_size: (u32, u32),
        frames_per_second: u32,
    ) -> Result<Self, String> {
        let VideoSourceKind::Screen(quality) = kind else {
            return Err("only screens are encoded in H.264".into());
        };
        // The top layer at the source's rate, 30 or 60, the lower ones at 30 with their usual
        // budgets (ADR-0058).
        let top = fit_within(source_size, quality.top_size());
        let open = |layer: SimulcastLayer, encoder: Encoder| LayerEncoder {
            layer,
            encoder,
            parameter_sets: ParameterSets::default(),
            is_active: true,
            is_profile_checked: false,
        };
        let top_encoder = Encoder::new_real_time_h264(
            top.0,
            top.1,
            frames_per_second,
            u64::from(top_bits_per_second(quality, frames_per_second)),
        )
        .map_err(|error| error.to_string())?;
        let mut encoders = Vec::with_capacity(LAYERS.len());
        for layer in [SimulcastLayer::Low, SimulcastLayer::Quarter] {
            let size = layer_size(top, layer);
            let encoder = top_encoder
                .new_scaled_real_time_h264(
                    size.0,
                    size.1,
                    FRAMES_PER_SECOND,
                    u64::from(kind.bits_per_second(layer)),
                )
                .map_err(|error| error.to_string())?;
            encoders.push(open(layer, encoder));
        }
        encoders.insert(0, open(SimulcastLayer::High, top_encoder));
        Ok(Self {
            kind,
            source_size,
            frames_per_second,
            encoders,
        })
    }

    /// Sends only the `active` layers (given every time: encoders reopened for a new size or
    /// quality must not send a layer the server or the uplink paused) and forces the keyframes
    /// asked for.
    fn apply(&mut self, active: &[SimulcastLayer], keyframes: &[Option<SimulcastLayer>]) {
        for layer in &mut self.encoders {
            let is_active = active.contains(&layer.layer);
            // A resumed layer's watchers wait for a keyframe: send it at once.
            if is_active && !layer.is_active {
                layer.encoder.force_keyframe();
            }
            layer.is_active = is_active;
            if keyframes
                .iter()
                .any(|asked| asked.is_none_or(|asked| asked == layer.layer))
            {
                layer.encoder.force_keyframe();
            }
        }
    }

    /// Encodes what `plan` says: `frame` (or the last picture again) on the top layer, its last
    /// picture scaled on the active lower layers; and sends the frames.
    fn encode(
        &mut self,
        frame: Option<VideoFrame>,
        plan: EncodePlan,
        input: &EncodedVideoInput,
    ) -> Result<(), String> {
        let rtp_timestamp = input.rtp_time_of(Instant::now());
        // The portal's pictures are stamped on `CLOCK_MONOTONIC`: repeats follow them.
        let pts = vesper_audio::monotonic_now();
        let Some((top, lower)) = self.encoders.split_first_mut() else {
            return Ok(());
        };
        if plan.top {
            let packets = match frame {
                Some(frame) => top.encoder.encode(frame),
                None => top.encoder.repeat(pts),
            }
            .map_err(|error| error.to_string())?;
            send_packets(top, packets, rtp_timestamp, input)?;
        }
        if !plan.lower {
            return Ok(());
        }
        for layer in lower.iter_mut().filter(|layer| layer.is_active) {
            let packets = layer
                .encoder
                .encode_scaled(&top.encoder, pts)
                .map_err(|error| error.to_string())?;
            send_packets(layer, packets, rtp_timestamp, input)?;
        }
        Ok(())
    }
}

/// Sends a layer's packets as access units, with the parameter sets before every IDR; refuses a
/// stream whose profile is not Constrained Baseline (the server would drop all of it).
fn send_packets(
    layer: &mut LayerEncoder,
    packets: Vec<vesper_core::EncodedPacket>,
    rtp_timestamp: u64,
    input: &EncodedVideoInput,
) -> Result<(), String> {
    for packet in packets {
        if !layer.is_profile_checked
            && let Some(sequence_set) = nal_units(&packet.data)
                .find(|unit| nal_unit_type(unit) == Some(SEQUENCE_PARAMETER_SET))
        {
            if !is_constrained_baseline_sequence_parameter_set(sequence_set) {
                return Err("the GPU encoder does not write Constrained Baseline".into());
            }
            layer.is_profile_checked = true;
        }
        let access_unit = layer.parameter_sets.complete(&packet.data);
        if !layer.is_active {
            continue;
        }
        if is_h264_keyframe(&access_unit) && !layer.is_profile_checked {
            // An IDR the receiver cannot place; the next keyframe carries its SPS.
            layer.encoder.force_keyframe();
            continue;
        }
        input.push(layer.layer, rtp_timestamp, access_unit.into_owned());
    }
    Ok(())
}

/// A picture the encoders take, as the loop sees it.
trait SizedPicture {
    fn size(&self) -> (u32, u32);
}

impl SizedPicture for VideoFrame {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// The encoders of one source size, quality and rate, as the loop drives them.
trait LayerSet {
    type Picture: SizedPicture;
    /// Where the encoded frames go.
    type Sink;
    /// Whether these encoders are for this source (its kind and size), whatever their rate.
    fn is_for_source(&self, kind: VideoSourceKind, size: (u32, u32)) -> bool;
    fn frames_per_second(&self) -> u32;
    fn apply(&mut self, active: &[SimulcastLayer], keyframes: &[Option<SimulcastLayer>]);
    fn encode(
        &mut self,
        picture: Option<Self::Picture>,
        plan: EncodePlan,
        sink: &Self::Sink,
    ) -> Result<(), String>;
}

impl LayerSet for Layers {
    type Picture = VideoFrame;
    type Sink = EncodedVideoInput;

    fn is_for_source(&self, kind: VideoSourceKind, size: (u32, u32)) -> bool {
        self.kind == kind && self.source_size == size
    }

    fn frames_per_second(&self) -> u32 {
        self.frames_per_second
    }

    fn apply(&mut self, active: &[SimulcastLayer], keyframes: &[Option<SimulcastLayer>]) {
        Self::apply(self, active, keyframes);
    }

    fn encode(
        &mut self,
        picture: Option<VideoFrame>,
        plan: EncodePlan,
        sink: &EncodedVideoInput,
    ) -> Result<(), String> {
        Self::encode(self, picture, plan, sink)
    }
}

/// The encoding thread's state between passes: the encoders, what the server asked, the pacing
/// and the source's measured rate, and the picture waiting for its turn.
struct EncodingLoop<L: LayerSet> {
    layers: Option<L>,
    /// The server only says when its layers change: kept for encoders opened later.
    active_layers: Vec<SimulcastLayer>,
    pacing: Pacing,
    source_rate: SourceRate,
    /// The newest picture not encoded yet: a source faster than the encoder's rate waits here,
    /// and a newer picture replaces it (the older one goes back to the compositor).
    waiting: Option<L::Picture>,
    /// Passes whose keyframe requests are dropped: asked while the encoders reopened, their
    /// first pictures (keyframes on every layer) already answer them.
    passes_ignoring_keyframes: u8,
}

impl<L: LayerSet> EncodingLoop<L> {
    fn new() -> Self {
        Self {
            layers: None,
            active_layers: LAYERS.to_vec(),
            pacing: Pacing::new(FRAMES_PER_SECOND),
            source_rate: SourceRate::new(),
            waiting: None,
            passes_ignoring_keyframes: 0,
        }
    }

    /// How long the thread may wait for a picture before something is due.
    fn wait(&self, now: Instant) -> Duration {
        self.pacing.wait(now, self.waiting.is_some())
    }

    /// One pass at `now`: takes in `picture` (if one came) and the server's `requests`, reopens
    /// the encoders when the source's quality, size or rate changed, and encodes what is due.
    ///
    /// # Errors
    /// Encoders for a new source cannot open, or encoding failed: the share ends.
    fn step(
        &mut self,
        now: Instant,
        picture: Option<L::Picture>,
        kind: VideoSourceKind,
        requests: EncoderRequests,
        open: &mut impl FnMut(VideoSourceKind, (u32, u32), u32) -> Result<L, String>,
        sink: &L::Sink,
    ) -> Result<(), String> {
        if let Some(active) = requests.active_layers {
            self.active_layers = active;
        }
        let mut keyframes = requests.keyframes;
        if self.passes_ignoring_keyframes > 0 {
            self.passes_ignoring_keyframes -= 1;
            keyframes.clear();
        }
        if let Some(picture) = picture {
            let frames_per_second = self.source_rate.picture(now);
            if self.reopen_if_needed(kind, picture.size(), frames_per_second, open)? {
                keyframes.clear();
            }
            self.waiting = Some(picture);
        }
        let Some(layers) = self.layers.as_mut() else {
            return Ok(());
        };
        layers.apply(&self.active_layers, &keyframes);
        let plan = self
            .pacing
            .plan(now, self.waiting.is_some(), !keyframes.is_empty());
        if plan.top || plan.lower {
            let picture = if plan.takes_picture {
                self.waiting.take()
            } else {
                None
            };
            layers.encode(picture, plan, sink)?;
        }
        Ok(())
    }

    /// Opens encoders for this source and rate unless the current ones are; returns whether it
    /// did. Only a rate switch may fail without ending the share: the current encoders go on at
    /// their rate, and the next try waits for the source's rate to hold again.
    fn reopen_if_needed(
        &mut self,
        kind: VideoSourceKind,
        size: (u32, u32),
        frames_per_second: u32,
        open: &mut impl FnMut(VideoSourceKind, (u32, u32), u32) -> Result<L, String>,
    ) -> Result<bool, String> {
        let current_rate = self
            .layers
            .as_ref()
            .filter(|layers| layers.is_for_source(kind, size))
            .map(LayerSet::frames_per_second);
        if current_rate == Some(frames_per_second) {
            return Ok(false);
        }
        match open(kind, size, frames_per_second) {
            Ok(layers) => {
                self.layers = Some(layers);
                self.pacing.encoders_opened(frames_per_second);
                // Requests that come in while they opened: the next pass.
                self.passes_ignoring_keyframes = 1;
                Ok(true)
            }
            Err(error) => match current_rate {
                Some(current_rate) => {
                    tracing::warn!(%error, "the screen's encoders could not switch rate");
                    self.source_rate.keep(current_rate);
                    Ok(false)
                }
                None => Err(error),
            },
        }
    }
}

fn run(mut input: EncodedVideoInput, pictures: &LatestPicture<VideoFrame>) -> Result<(), String> {
    let mut encoding = EncodingLoop::<Layers>::new();
    let mut open = Layers::open;
    while input.is_open() {
        let picture = match pictures.take(encoding.wait(Instant::now())) {
            Taken::Picture(picture) => Some(picture),
            Taken::Nothing => None,
            Taken::Closed => return Ok(()),
        };
        let Some(kind) = input.kind() else {
            return Ok(());
        };
        let requests = input.take_requests();
        encoding.step(Instant::now(), picture, kind, requests, &mut open, &input)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;

    /// Plans a pass every millisecond for `milliseconds` with the encoder at `frames_per_second`,
    /// new pictures arriving at `pictures` (milliseconds; a newer one replaces one waiting), and
    /// returns when the top layer took which picture, and when the lower layers encoded.
    fn run_pacing(
        milliseconds: u64,
        pictures: &[u64],
        frames_per_second: u32,
    ) -> (Vec<(u64, u64)>, Vec<u64>) {
        let start = Instant::now();
        let mut pacing = Pacing::new(frames_per_second);
        let (mut top, mut lower) = (Vec::new(), Vec::new());
        let mut waiting = None;
        for at in 0..milliseconds {
            if pictures.contains(&at) {
                waiting = Some(at);
            }
            let plan = pacing.plan(start + Duration::from_millis(at), waiting.is_some(), false);
            if plan.takes_picture
                && let Some(picture) = waiting.take()
            {
                top.push((at, picture));
            }
            if plan.lower {
                lower.push(at);
            }
        }
        (top, lower)
    }

    fn every(interval_micros: u64, until_millis: u64) -> Vec<u64> {
        (0..=until_millis * 1_000 / interval_micros)
            .map(|index| index * interval_micros / 1_000)
            .filter(|&at| at < until_millis)
            .collect()
    }

    #[test]
    fn a_faster_source_averages_the_encoder_s_rate_at_most() {
        // 60 pictures a second for 10 s while the encoder is at 30: 30.0 a second, not the 33
        // a 30 ms shortest interval gave.
        let (top, lower) = run_pacing(10_000, &every(16_667, 10_000), FRAMES_PER_SECOND);
        assert!(top.len() <= 300, "{} pictures in 10 s", top.len());
        assert!(top.len() >= 295, "{} pictures in 10 s", top.len());
        assert!(lower.len() <= 300, "{} lower pictures in 10 s", lower.len());
        // At 60 the lower layers still average 30 at most.
        let (top, lower) = run_pacing(
            10_000,
            &every(16_667, 10_000),
            HARDWARE_SCREEN_FRAMES_PER_SECOND,
        );
        assert!((595..=600).contains(&top.len()), "{}", top.len());
        assert!((295..=300).contains(&lower.len()), "{}", lower.len());
    }

    #[test]
    fn a_waiting_picture_is_replaced_by_the_newest_one() {
        // Pictures at 0, 10 and 20 ms, the encoder at 30: the first goes at once, the one at
        // 20 ms (not 10) at the next token, 33 ms after the first.
        let (top, _) = run_pacing(100, &[0, 10, 20], FRAMES_PER_SECOND);
        assert_eq!(top, [(0, 0), (34, 20)]);
    }

    #[test]
    fn a_30_fps_source_is_taken_as_it_comes_and_reaches_the_lower_layers_within_34_ms() {
        let pictures = every(33_334, 1_000);
        let (top, lower) = run_pacing(1_000, &pictures, FRAMES_PER_SECOND);
        let taken: Vec<u64> = top.iter().map(|&(_, picture)| picture).collect();
        assert_eq!(taken, pictures, "none dropped");
        assert!(
            top.iter().all(|&(at, picture)| at - picture <= 1),
            "{top:?}"
        );
        for picture in pictures {
            let reached = lower.iter().find(|&&at| at >= picture).copied();
            assert!(
                reached.is_some_and(|at| at - picture <= 34),
                "{picture}: {lower:?}"
            );
        }
    }

    #[test]
    fn irregular_pictures_still_reach_the_lower_layers_within_34_ms() {
        let pictures = [0, 20, 40, 60, 500, 510, 900];
        let (_, lower) = run_pacing(1_000, &pictures, HARDWARE_SCREEN_FRAMES_PER_SECOND);
        for picture in pictures {
            let reached = lower.iter().find(|&&at| at >= picture).copied();
            assert!(
                reached.is_some_and(|at| at - picture <= 34),
                "{picture}: {lower:?}"
            );
        }
    }

    #[test]
    fn a_still_screen_is_repeated_once_a_second() {
        let start = Instant::now();
        let mut pacing = Pacing::new(FRAMES_PER_SECOND);
        let repeats: Vec<u64> = (0..3_000)
            .filter(|&at| {
                let plan = pacing.plan(start + Duration::from_millis(at), at == 0, false);
                plan.top && !plan.takes_picture
            })
            .collect();
        assert_eq!(repeats, [1_000, 2_000]);
    }

    /// The encoder rate after each picture of a source whose pictures come at `rates` (pictures
    /// a second, one entry per second), from an injected clock.
    fn encoder_rates(rates: &[u64]) -> Vec<(u64, u32)> {
        let start = Instant::now();
        let mut source = SourceRate::new();
        let mut switches = Vec::new();
        let mut current = FRAMES_PER_SECOND;
        for (second, &rate) in (0_u64..).zip(rates) {
            for index in 0..rate {
                let at = second * 1_000_000 + index * 1_000_000 / rate;
                let chosen = source.picture(start + Duration::from_micros(at));
                if chosen != current {
                    switches.push((at / 1_000, chosen));
                    current = chosen;
                }
            }
        }
        switches
    }

    #[test]
    fn the_encoder_follows_a_sustained_source_rate_only() {
        // 60 a second from the start: 60 once that held a second (the window reached 45
        // after 0.75 s).
        let switches = encoder_rates(&[60, 60, 60]);
        assert_eq!(switches.len(), 1, "{switches:?}");
        assert!((1_700..=1_800).contains(&switches[0].0), "{switches:?}");
        // Back to 30 two seconds after the source fell to 30.
        let switches = encoder_rates(&[60, 60, 60, 30, 30, 30, 30, 30]);
        assert_eq!(switches.len(), 2, "{switches:?}");
        assert_eq!(switches[1].1, FRAMES_PER_SECOND);
        assert!((5_000..=6_000).contains(&switches[1].0), "{switches:?}");
        // A source at 30 never switches; one at 40, between the thresholds, never either.
        assert_eq!(encoder_rates(&[30; 10]), []);
        assert_eq!(encoder_rates(&[40; 10]), []);
    }

    #[test]
    fn a_source_wavering_around_the_thresholds_does_not_make_the_encoder_oscillate() {
        let wavering: Vec<u64> = (0..20)
            .map(|second| if second % 2 == 0 { 50 } else { 30 })
            .collect();
        assert!(encoder_rates(&wavering).len() <= 1);
        let short_bursts: Vec<u64> = (0..20)
            .map(|second| if second % 3 == 0 { 60 } else { 30 })
            .collect();
        let switches = encoder_rates(&short_bursts);
        let gaps: Vec<u64> = switches
            .windows(2)
            .map(|pair| pair[1].0 - pair[0].0)
            .collect();
        assert!(gaps.iter().all(|&gap| gap >= 1_000), "{switches:?}");
    }

    #[test]
    fn the_capture_s_newest_picture_wins_and_a_closed_slot_ends_the_thread() {
        let latest = LatestPicture::new();
        latest.offer(1);
        latest.offer(2);
        assert_eq!(latest.take(Duration::ZERO), Taken::Picture(2));
        assert_eq!(latest.take(Duration::ZERO), Taken::Nothing);
        latest.close();
        latest.offer(3);
        assert_eq!(latest.take(Duration::from_secs(10)), Taken::Closed);
    }

    /// A picture the fake encoders take: its arrival time and its size.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct FakePicture {
        id: u64,
        size: (u32, u32),
    }

    impl SizedPicture for FakePicture {
        fn size(&self) -> (u32, u32) {
            self.size
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Event {
        /// Encoders opened at this rate, on the pass at this millisecond.
        Opened {
            frames_per_second: u32,
            at: u64,
        },
        KeyframesAsked(usize),
        Encoded {
            frames_per_second: u32,
            picture: Option<u64>,
        },
    }

    struct FakeLayers {
        kind: VideoSourceKind,
        size: (u32, u32),
        frames_per_second: u32,
        events: Rc<RefCell<Vec<Event>>>,
    }

    impl LayerSet for FakeLayers {
        type Picture = FakePicture;
        type Sink = ();

        fn is_for_source(&self, kind: VideoSourceKind, size: (u32, u32)) -> bool {
            self.kind == kind && self.size == size
        }

        fn frames_per_second(&self) -> u32 {
            self.frames_per_second
        }

        fn apply(&mut self, _: &[SimulcastLayer], keyframes: &[Option<SimulcastLayer>]) {
            if !keyframes.is_empty() {
                self.events
                    .borrow_mut()
                    .push(Event::KeyframesAsked(keyframes.len()));
            }
        }

        fn encode(
            &mut self,
            picture: Option<FakePicture>,
            plan: EncodePlan,
            (): &(),
        ) -> Result<(), String> {
            if plan.top {
                self.events.borrow_mut().push(Event::Encoded {
                    frames_per_second: self.frames_per_second,
                    picture: picture.map(|picture| picture.id),
                });
            }
            Ok(())
        }
    }

    const SCREEN: VideoSourceKind = VideoSourceKind::Screen(ScreenShareQuality::Highest);
    const SIZE: (u32, u32) = (2_560, 1_440);

    /// Runs the loop every millisecond for `milliseconds`, with pictures at `pictures`
    /// (milliseconds), keyframes asked at `keyframes`, and encoders opening only at the rates
    /// `opens_at` allows; returns the events.
    fn run_loop(
        milliseconds: u64,
        pictures: &[u64],
        keyframes: &[u64],
        opens_at: &[u32],
    ) -> (Vec<Event>, Result<(), String>) {
        let start = Instant::now();
        let events = Rc::new(RefCell::new(Vec::new()));
        let pass = Rc::new(Cell::new(0));
        let mut open = |kind, size, frames_per_second| {
            if !opens_at.contains(&frames_per_second) {
                return Err(format!("no encoder at {frames_per_second}"));
            }
            events.borrow_mut().push(Event::Opened {
                frames_per_second,
                at: pass.get(),
            });
            Ok(FakeLayers {
                kind,
                size,
                frames_per_second,
                events: Rc::clone(&events),
            })
        };
        let mut encoding = EncodingLoop::<FakeLayers>::new();
        let mut result = Ok(());
        for at in 0..milliseconds {
            pass.set(at);
            let picture = pictures
                .contains(&at)
                .then_some(FakePicture { id: at, size: SIZE });
            let requests = EncoderRequests {
                active_layers: None,
                keyframes: if keyframes.contains(&at) {
                    vec![None]
                } else {
                    Vec::new()
                },
            };
            result = encoding.step(
                start + Duration::from_millis(at),
                picture,
                SCREEN,
                requests,
                &mut open,
                &(),
            );
            if result.is_err() {
                break;
            }
        }
        let events = events.borrow().clone();
        (events, result)
    }

    fn opens(events: &[Event]) -> Vec<u32> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Opened {
                    frames_per_second, ..
                } => Some(*frames_per_second),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_sustained_60_fps_source_reopens_the_encoders_at_60_once() {
        let (events, result) = run_loop(4_000, &every(16_667, 4_000), &[], &[30, 60]);
        assert_eq!(result, Ok(()));
        assert_eq!(opens(&events), [30, 60]);
        let at_sixty = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    Event::Encoded {
                        frames_per_second: 60,
                        ..
                    }
                )
            })
            .count();
        // From about 1.75 s on, every picture: some 135 of the last 2.25 s.
        assert!((120..=140).contains(&at_sixty), "{at_sixty}");
        // The first picture after the switch is the one that asked for it.
        let after = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    Event::Opened {
                        frames_per_second: 60,
                        ..
                    }
                )
            })
            .unwrap();
        assert!(matches!(
            events[after + 1],
            Event::Encoded {
                frames_per_second: 60,
                picture: Some(_)
            }
        ));
    }

    #[test]
    fn a_rate_switch_that_cannot_open_keeps_the_share_at_its_rate_and_waits_to_retry() {
        let (events, result) = run_loop(4_000, &every(16_667, 4_000), &[], &[30]);
        assert_eq!(result, Ok(()), "the share goes on");
        assert_eq!(opens(&events), [30]);
        let encoded = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    Event::Encoded {
                        frames_per_second: 30,
                        ..
                    }
                )
            })
            .count();
        assert!(
            (115..=121).contains(&encoded),
            "{encoded} pictures at 30 in 4 s"
        );
    }

    #[test]
    fn encoders_that_cannot_open_for_a_new_source_end_the_share() {
        let (_, result) = run_loop(100, &[0], &[], &[]);
        assert_eq!(result, Err("no encoder at 30".into()));
    }

    #[test]
    fn keyframes_asked_while_the_encoders_reopen_are_answered_by_their_first_pictures() {
        let pictures = every(16_667, 3_000);
        let (events, _) = run_loop(3_000, &pictures, &[], &[30, 60]);
        let switch = events
            .iter()
            .find_map(|event| match event {
                Event::Opened {
                    frames_per_second: 60,
                    at,
                } => Some(*at),
                _ => None,
            })
            .unwrap();
        // Asked on the reopening pass and on the next one (while the encoders opened): dropped.
        // Asked on the pass after: it reaches the encoders.
        let (events, _) = run_loop(
            3_000,
            &pictures,
            &[switch, switch + 1, switch + 2],
            &[30, 60],
        );
        let asked: Vec<&Event> = events
            .iter()
            .filter(|event| matches!(event, Event::KeyframesAsked(_)))
            .collect();
        assert_eq!(asked, [&Event::KeyframesAsked(1)]);
        let reached_at = events
            .iter()
            .position(|event| matches!(event, Event::KeyframesAsked(_)))
            .unwrap();
        let opened_at = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    Event::Opened {
                        frames_per_second: 60,
                        ..
                    }
                )
            })
            .unwrap();
        assert!(reached_at > opened_at);
    }
}
