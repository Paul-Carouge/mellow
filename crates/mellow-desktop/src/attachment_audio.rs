//! Sounds and videos attached to messages, played in the app (ADR-0038, `web-inline-media.md`):
//! one at a time, as the web pauses every other player; nothing is fetched until someone presses
//! play, the file comes through byte ranges on the authenticated download path, and
//! `mellow-media`'s [`ClipPlayer`] decodes the sound off the interface thread. A video's pictures
//! come from a [`VideoStream`] that follows the player's position, so its sound, volume and output
//! are those of any sound. Opening another channel, signing out or the row leaving the list (it
//! scrolled far away: [`keep_player_while_shown`]) stops the sound and frees it, as the web's
//! element goes with its row. The volume is remembered for the next sound, as the web's
//! `mellow.media-volume`. Sounds play on the voice output chosen in the settings and move there
//! when it changes ([`follow_voice_output_device`]), as the web's elements follow `setSinkId`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{App, ElementId, Global, Task, Window};
use mellow_client_core::{DownloadableFile, RestClient};
use mellow_media::{
    ClipPhase, ClipPlayer, ClipStart, MAXIMUM_WAVEFORM_SOURCE_BYTES,
    PLAYABLE_ATTACHMENT_AUDIO_TYPES, SystemAudioDevices, VideoStream, VideoStreamPhase,
    VideoTiming, plays_video_type, sound_waveform,
};
use mellow_protocol::{Attachment, Snowflake};

use crate::components::child_id;
use crate::preferences::{change_preferences, preferences};
use crate::tokio_bridge::spawn_on_network_runtime;
use crate::voice::video_view::VideoPicture;
use crate::youtube_video::{
    YouTubeFailure, YouTubeTracks, YouTubeVideo, is_youtube_attachment, plays_youtube_here,
    set_youtube_failure, start_of as start_of_youtube, youtube_tracks, youtube_video_of,
};

/// Larger sounds and videos are offered as downloads: the whole file is held in memory while it
/// plays. The instance's default limit is 25 MB.
pub const MAXIMUM_PLAYED_ATTACHMENT_BYTES: u64 = 64 * 1024 * 1024;
/// How often the seek bar moves while a sound plays.
const PLAYING_REDRAW_PERIOD: Duration = Duration::from_millis(250);
/// Redraws kept after a command, while the player's thread takes it in (it answers in
/// milliseconds; a second covers a busy machine).
const SETTLING_REDRAWS: u8 = 4;
/// The file's own loudness, when nothing is remembered.
const DEFAULT_VOLUME: f32 = 1.0;
/// A long session's worth of sounds whose shape is remembered; the oldest is forgotten first.
const MAXIMUM_REMEMBERED_SOUNDS: usize = 200;

/// What a sound's player shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioPlayback {
    /// Not the loaded sound: at 0:00, nothing fetched.
    Idle,
    /// Its file is being fetched.
    Loading,
    /// Its download stopped part way: pressing play fetches it again.
    Stopped,
    Playing,
    Paused,
    Ended,
    NoOutput,
    /// Not played here: the card says so and offers the file instead.
    CannotPlay,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioView {
    pub playback: AudioPlayback,
    pub position: Duration,
    pub duration: Option<Duration>,
    /// When the file started loading, for the "Loading…" notice that waits a moment.
    pub loading_since: Option<Instant>,
    /// Where a press on the waveform asked to start before the sound was loaded, 0 to 1.
    pub pending_start: Option<f32>,
}

/// What the list knows of a sound's shape: learnt the first time it plays, kept for the session.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SoundShape {
    /// One level per bar, 0 to 1, or `None` while unknown.
    pub levels: Option<Vec<f32>>,
    pub duration: Option<Duration>,
}

/// The shapes of the sounds played this session, and those that could not be read: bounded,
/// oldest first out, in memory only, so a shared computer keeps no trace of what was heard.
#[derive(Debug, Default)]
struct SoundShapes {
    shapes: HashMap<Snowflake, SoundShape>,
    order: VecDeque<Snowflake>,
    failed: VecDeque<Snowflake>,
    being_read: HashSet<Snowflake>,
}

impl SoundShapes {
    fn get(&self, attachment_id: Snowflake) -> Option<&SoundShape> {
        self.shapes.get(&attachment_id)
    }

    fn remember(&mut self, attachment_id: Snowflake, shape: SoundShape) {
        self.order.retain(|id| *id != attachment_id);
        self.order.push_back(attachment_id);
        self.shapes.insert(attachment_id, shape);
        while self.order.len() > MAXIMUM_REMEMBERED_SOUNDS {
            if let Some(oldest) = self.order.pop_front() {
                self.shapes.remove(&oldest);
            }
        }
    }

    fn remember_duration(&mut self, attachment_id: Snowflake, duration: Duration) {
        let known = self.get(attachment_id);
        if known.and_then(|shape| shape.duration) != Some(duration) {
            let levels = known.and_then(|shape| shape.levels.clone());
            self.remember(
                attachment_id,
                SoundShape {
                    levels,
                    duration: Some(duration),
                },
            );
        }
    }

    fn remember_failure(&mut self, attachment_id: Snowflake) {
        self.failed.push_back(attachment_id);
        if self.failed.len() > MAXIMUM_REMEMBERED_SOUNDS {
            self.failed.pop_front();
        }
    }

    /// Whether this sound's shape is still worth reading: never read, not failing, not being read.
    fn should_read(&self, attachment_id: Snowflake) -> bool {
        self.get(attachment_id)
            .is_none_or(|shape| shape.levels.is_none())
            && !self.failed.contains(&attachment_id)
            && !self.being_read.contains(&attachment_id)
    }
}

/// The volume every sound plays at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttachmentVolume {
    pub volume: f32,
    pub is_muted: bool,
}

/// The loaded video's pictures.
struct PlayingVideo {
    attachment_id: Snowflake,
    stream: VideoStream,
    /// The newest picture, drawn by the video's card.
    picture: Option<VideoPicture>,
    /// Brings the stream's pictures here.
    _pump: Task<()>,
}

#[derive(Default)]
struct AttachmentAudio {
    rest: Option<RestClient>,
    /// Started at the first play.
    player: Option<ClipPlayer>,
    /// The loaded attachment's pictures, when it is a video.
    video: Option<PlayingVideo>,
    /// The attachment whose file is on its way, the download, and when it started.
    fetching: Option<(Snowflake, Task<()>, Instant)>,
    /// Refused before any download (type or size), or by a decoder.
    unplayable: HashSet<Snowflake>,
    /// Whose download stopped part way; pressing play tries again.
    stopped: HashSet<Snowflake>,
    /// A press on a waveform before its sound was loaded: where it starts once played.
    pending_start: Option<(Snowflake, f32)>,
    /// The speeds chosen for sounds other than 1×, kept while their card is used.
    speeds: HashMap<Snowflake, f32>,
    shapes: SoundShapes,
    /// Redraws the windows while something plays or loads.
    redraw: Option<Task<()>>,
    /// Redraws left after the last command, so its answer shows even once nothing plays.
    settling_redraws: u8,
    /// Bumped by each session, so a late download of the previous one is dropped.
    session: u64,
    /// How many surfaces (a message's card, the media viewer) show each attachment's player.
    shown_surfaces: std::collections::HashMap<Snowflake, usize>,
}

impl Global for AttachmentAudio {}

impl AttachmentAudio {
    /// Stops the sound and drops the video; its last picture is handed back to be freed.
    #[must_use]
    fn stop(&mut self) -> Option<VideoPicture> {
        self.fetching = None;
        if let Some(player) = self.player.as_ref() {
            // What the player refused (or what broke part way: a streamed track can) stays
            // refused once stopped, so its card says so rather than offering play again.
            let status = player.status();
            if let (Some(clip), ClipPhase::CannotPlay(error)) = (status.clip, status.phase) {
                tracing::info!(%error, "cannot play a sound or video here");
                self.unplayable.insert(Snowflake::from_raw(clip));
            }
            player.stop();
        }
        self.video.take().and_then(|video| video.picture)
    }

    fn is_active(&self) -> bool {
        self.settling_redraws > 0
            || self.fetching.is_some()
            || self
                .player
                .as_ref()
                .is_some_and(|player| player.status().phase == ClipPhase::Playing)
    }

    /// Whether `attachment_id` is the one fetched, loaded or refused by the player last.
    fn holds(&self, attachment_id: Snowflake) -> bool {
        self.fetching
            .as_ref()
            .is_some_and(|(id, _, _)| *id == attachment_id)
            || self
                .player
                .as_ref()
                .is_some_and(|player| player.status().clip == Some(attachment_id.raw()))
            || self
                .video
                .as_ref()
                .is_some_and(|video| video.attachment_id == attachment_id)
    }
}

/// Stops whatever plays and frees the video's last picture.
fn stop_playback(cx: &mut App) {
    if let Some(picture) = cx.default_global::<AttachmentAudio>().stop() {
        picture.free(cx);
    }
}

/// Sounds of a signed-in session download with `rest`.
pub fn start_attachment_audio_session(rest: RestClient, cx: &mut App) {
    stop_playback(cx);
    let store = cx.default_global::<AttachmentAudio>();
    forget_session_sounds(store);
    store.session += 1;
    store.rest = Some(rest);
}

/// Signing out stops the sound and forgets the session's files.
pub fn end_attachment_audio_session(cx: &mut App) {
    stop_playback(cx);
    let store = cx.default_global::<AttachmentAudio>();
    forget_session_sounds(store);
    store.session += 1;
    store.rest = None;
}

/// What a session learnt of its sounds goes with it.
fn forget_session_sounds(store: &mut AttachmentAudio) {
    store.unplayable.clear();
    store.stopped.clear();
    store.pending_start = None;
    store.speeds.clear();
    store.shapes = SoundShapes::default();
}

/// The signed-in session's client, for saving an attachment.
pub(crate) fn attachment_rest_client(cx: &App) -> Option<RestClient> {
    cx.try_global::<AttachmentAudio>()?.rest.clone()
}

/// The voice output changed: a playing sound moves there.
pub fn follow_voice_output_device(output_device_id: Option<String>, cx: &mut App) {
    if let Some(player) = cx
        .try_global::<AttachmentAudio>()
        .and_then(|store| store.player.as_ref())
    {
        player.set_output_device(output_device_id);
    }
}

/// Another channel opened: the sound stops and is freed, as the web's element goes with the page.
pub fn stop_attachment_audio(cx: &mut App) {
    if let Some(store) = cx.try_global::<AttachmentAudio>()
        && (store.fetching.is_some() || store.player.is_some() || store.video.is_some())
    {
        stop_playback(cx);
        cx.refresh_windows();
    }
}

/// Keeps `attachment_id`'s player for as long as the element `id` is drawn: once no surface shows
/// it any more (its row scrolled far away or was deleted, the conversation and the media viewer
/// closed), the sound or video it plays stops, as the web's element is removed with its row.
pub(crate) fn keep_player_while_shown(
    id: &ElementId,
    attachment_id: Snowflake,
    window: &mut Window,
    cx: &mut App,
) {
    window.use_keyed_state(child_id(id, "shown"), cx, |_, cx| {
        *cx.default_global::<AttachmentAudio>()
            .shown_surfaces
            .entry(attachment_id)
            .or_default() += 1;
        cx.on_release(move |(), cx| {
            let store = cx.default_global::<AttachmentAudio>();
            let remaining = store.shown_surfaces.get_mut(&attachment_id).map(|count| {
                *count = count.saturating_sub(1);
                *count
            });
            if remaining == Some(0) {
                store.shown_surfaces.remove(&attachment_id);
                stop_attachment_playback(attachment_id, cx);
            }
        })
        .detach();
    });
}

/// Stops `attachment_id`'s sound or video, even while its file is still on its way: the surface
/// that asked for it is gone.
pub(crate) fn stop_attachment_playback(attachment_id: Snowflake, cx: &mut App) {
    if cx
        .try_global::<AttachmentAudio>()
        .is_some_and(|store| store.holds(attachment_id))
    {
        stop_playback(cx);
        cx.refresh_windows();
    }
}

/// Puts `attachment_id` in the state of a file on its way, as after pressing play.
#[cfg(test)]
pub(crate) fn pretend_fetching_for_tests(attachment_id: Snowflake, cx: &mut App) {
    cx.default_global::<AttachmentAudio>().fetching =
        Some((attachment_id, Task::ready(()), Instant::now()));
}

/// Where `attachment_id`'s player stands.
#[must_use]
pub fn attachment_audio_view(attachment_id: Snowflake, cx: &App) -> AudioView {
    let Some(store) = cx.try_global::<AttachmentAudio>() else {
        return AudioView {
            playback: AudioPlayback::Idle,
            position: Duration::ZERO,
            duration: None,
            loading_since: None,
            pending_start: None,
        };
    };
    let idle = AudioView {
        playback: AudioPlayback::Idle,
        position: Duration::ZERO,
        duration: store
            .shapes
            .get(attachment_id)
            .and_then(|shape| shape.duration),
        loading_since: None,
        pending_start: store
            .pending_start
            .filter(|(id, _)| *id == attachment_id)
            .map(|(_, fraction)| fraction),
    };
    if store.unplayable.contains(&attachment_id) {
        return AudioView {
            playback: AudioPlayback::CannotPlay,
            ..idle
        };
    }
    if let Some((_, _, since)) = store
        .fetching
        .as_ref()
        .filter(|(id, _, _)| *id == attachment_id)
    {
        return AudioView {
            playback: AudioPlayback::Loading,
            loading_since: Some(*since),
            ..idle
        };
    }
    let Some(status) = store
        .player
        .as_ref()
        .map(ClipPlayer::status)
        .filter(|status| status.clip == Some(attachment_id.raw()))
    else {
        return if store.stopped.contains(&attachment_id) {
            AudioView {
                playback: AudioPlayback::Stopped,
                ..idle
            }
        } else {
            idle
        };
    };
    AudioView {
        playback: match status.phase {
            ClipPhase::Playing => AudioPlayback::Playing,
            ClipPhase::Paused => AudioPlayback::Paused,
            ClipPhase::Ended => AudioPlayback::Ended,
            ClipPhase::NoOutput => AudioPlayback::NoOutput,
            ClipPhase::CannotPlay(_) => AudioPlayback::CannotPlay,
        },
        position: status.position,
        duration: status.duration.or(idle.duration),
        loading_since: None,
        pending_start: None,
    }
}

/// The shape of `attachment_id` as far as it is known this session.
#[must_use]
pub fn attachment_sound_shape(attachment_id: Snowflake, cx: &App) -> Option<&SoundShape> {
    cx.try_global::<AttachmentAudio>()?
        .shapes
        .get(attachment_id)
}

/// The speed `attachment_id` plays at, 1 unless its card chose another.
#[must_use]
pub fn attachment_speed(attachment_id: Snowflake, cx: &App) -> f32 {
    cx.try_global::<AttachmentAudio>()
        .and_then(|store| store.speeds.get(&attachment_id).copied())
        .unwrap_or(1.0)
}

/// Plays `attachment_id` at `speed` from now on, and whenever it plays again this session.
pub fn set_attachment_speed(attachment_id: Snowflake, speed: f32, cx: &mut App) {
    let store = cx.default_global::<AttachmentAudio>();
    #[allow(clippy::float_cmp)] // 1 is set exactly, from the speed list.
    if speed == 1.0 {
        store.speeds.remove(&attachment_id);
    } else {
        store.speeds.insert(attachment_id, speed);
    }
    if let Some(player) = store.player.as_ref()
        && player.status().clip == Some(attachment_id.raw())
    {
        player.set_speed(speed);
    }
    cx.refresh_windows();
}

/// A press or a key on the waveform: the loaded sound moves to `fraction` of its length; one not
/// loaded yet keeps it and starts there when played.
pub fn seek_attachment_audio_to_fraction(attachment_id: Snowflake, fraction: f32, cx: &mut App) {
    let fraction = fraction.clamp(0.0, 1.0);
    let known_duration = attachment_audio_view(attachment_id, cx).duration;
    let store = cx.default_global::<AttachmentAudio>();
    match store.player.as_ref() {
        Some(player) if player.status().clip == Some(attachment_id.raw()) => {
            match start_of(Some(fraction), known_duration) {
                ClipStart::At(position) => player.seek(position),
                _ => player.seek_to_fraction(fraction),
            }
            keep_redrawing(cx);
        }
        _ => store.pending_start = Some((attachment_id, fraction)),
    }
    cx.refresh_windows();
}

/// The newest picture of `attachment_id`, while it is the loaded video.
#[must_use]
pub(crate) fn attachment_video_picture(attachment_id: Snowflake, cx: &App) -> Option<VideoPicture> {
    cx.try_global::<AttachmentAudio>()?
        .video
        .as_ref()
        .filter(|video| video.attachment_id == attachment_id)?
        .picture
        .clone()
}

/// Whether the app plays this attachment's sound itself rather than offering only the file.
#[must_use]
pub fn is_played_in_app(attachment: &Attachment) -> bool {
    PLAYABLE_ATTACHMENT_AUDIO_TYPES.contains(&attachment.content_type.as_str())
}

/// Whether the app plays this video itself: this system decodes its container (the codec inside
/// is found out once fetched; the card then offers the file instead).
#[must_use]
pub fn is_video_played_in_app(attachment: &Attachment) -> bool {
    plays_video_type(&attachment.content_type)
        || (is_youtube_attachment(attachment) && plays_youtube_here())
}

/// Play or pause: the first play fetches the file, stopping whichever sound played before.
pub fn toggle_attachment_audio(attachment: &Attachment, cx: &mut App) {
    match attachment_audio_view(attachment.id, cx).playback {
        AudioPlayback::Playing => {
            if let Some(player) = cx.global::<AttachmentAudio>().player.as_ref() {
                player.pause();
            }
        }
        AudioPlayback::Paused | AudioPlayback::Ended | AudioPlayback::NoOutput => {
            if let Some(player) = cx.global::<AttachmentAudio>().player.as_ref() {
                player.play();
            }
        }
        AudioPlayback::Idle | AudioPlayback::Stopped => fetch_and_play(attachment, cx),
        // Pause while it loads: the download is dropped and the card is back to play, as the
        // web's pause before the sound could start.
        AudioPlayback::Loading => {
            let store = cx.global_mut::<AttachmentAudio>();
            store.fetching = None;
            // A streamed video may already be opening in the player.
            if let Some(player) = store.player.as_ref() {
                player.stop();
            }
        }
        AudioPlayback::CannotPlay => return,
    }
    keep_redrawing(cx);
    cx.refresh_windows();
}

/// Plays a card's YouTube video again after YouTube refused it or its tracks could not be read:
/// the refusal is forgotten and the tracks are asked for anew.
pub fn retry_youtube_video(attachment: &Attachment, cx: &mut App) {
    if !is_youtube_attachment(attachment) {
        return;
    }
    fetch_and_play(attachment, cx);
    // After: stopping the refused stream marks it refused again.
    cx.default_global::<AttachmentAudio>()
        .unplayable
        .remove(&attachment.id);
    keep_redrawing(cx);
    cx.refresh_windows();
}

/// Moves the loaded sound to `position`; another attachment's seek bar does nothing.
pub fn seek_attachment_audio(attachment_id: Snowflake, position: Duration, cx: &mut App) {
    if let Some(store) = cx.try_global::<AttachmentAudio>()
        && let Some(player) = store.player.as_ref()
        && player.status().clip == Some(attachment_id.raw())
    {
        player.seek(position);
        cx.refresh_windows();
        keep_redrawing(cx);
    }
}

/// The remembered volume of every sound.
#[must_use]
pub fn attachment_volume(cx: &App) -> AttachmentVolume {
    let remembered = preferences(cx);
    AttachmentVolume {
        volume: remembered
            .and_then(|preferences| preferences.media_volume)
            .unwrap_or(DEFAULT_VOLUME),
        is_muted: remembered.is_some_and(|preferences| preferences.is_media_muted),
    }
}

/// Sets and remembers the volume; moving it unmutes, as a browser's player does.
pub fn set_attachment_volume(volume: AttachmentVolume, cx: &mut App) {
    let volume = AttachmentVolume {
        volume: volume.volume.clamp(0.0, 1.0),
        ..volume
    };
    change_preferences(cx, move |preferences| {
        preferences.media_volume = Some(volume.volume);
        preferences.is_media_muted = volume.is_muted;
    });
    apply_volume(volume, cx);
    cx.refresh_windows();
}

fn apply_volume(volume: AttachmentVolume, cx: &App) {
    if let Some(player) = cx
        .try_global::<AttachmentAudio>()
        .and_then(|store| store.player.as_ref())
    {
        player.set_volume(if volume.is_muted { 0.0 } else { volume.volume });
    }
}

fn fetch_and_play(attachment: &Attachment, cx: &mut App) {
    if is_youtube_attachment(attachment) {
        stream_youtube_video(attachment.id, cx);
        return;
    }
    let attachment_id = attachment.id;
    let is_video = is_video_played_in_app(attachment);
    // Checked before anything is fetched or decoded: the players check the bytes again.
    if !(is_played_in_app(attachment) || is_video)
        || attachment.size_bytes > MAXIMUM_PLAYED_ATTACHMENT_BYTES
    {
        cx.default_global::<AttachmentAudio>()
            .unplayable
            .insert(attachment_id);
        return;
    }
    let output_device_id = crate::preferences::preferences(cx)
        .and_then(|preferences| preferences.voice_settings.output_device_id.clone());
    let store = cx.default_global::<AttachmentAudio>();
    let Some(rest) = store.rest.clone() else {
        return;
    };
    store.stopped.remove(&attachment_id);
    // The sound the player refused stays refused: pressing play again would fetch it again only
    // to fail the same way.
    if let Some(status) = store.player.as_ref().map(ClipPlayer::status)
        && let (Some(refused), ClipPhase::CannotPlay(_)) = (status.clip, status.phase)
    {
        store.unplayable.insert(Snowflake::from_raw(refused));
    }
    stop_playback(cx);
    if !ensure_player(output_device_id, cx) {
        return;
    }
    let store = cx.global_mut::<AttachmentAudio>();
    let session = store.session;
    let fetched = Fetched {
        attachment_id,
        content_type: attachment.content_type.clone(),
        is_video,
        reads_shape: !is_video
            && attachment.size_bytes <= MAXIMUM_WAVEFORM_SOURCE_BYTES
            && store.shapes.should_read(attachment_id),
    };
    let download = spawn_on_network_runtime(cx, async move {
        rest.download_in_ranges(
            DownloadableFile::Attachment(attachment_id),
            usize::try_from(MAXIMUM_PLAYED_ATTACHMENT_BYTES).unwrap_or(usize::MAX),
        )
        .await
        // Moved into shared memory here, off the interface thread.
        .map(Arc::<[u8]>::from)
    });
    let task = cx.spawn(async move |cx| {
        let answer = download.await;
        cx.update(|cx| {
            let store = cx.global_mut::<AttachmentAudio>();
            let is_still_wanted = store.session == session
                && store
                    .fetching
                    .as_ref()
                    .is_some_and(|(id, _, _)| *id == attachment_id);
            if !is_still_wanted {
                return;
            }
            store.fetching = None;
            match answer {
                Ok(Ok(bytes)) => play_fetched(&fetched, bytes, cx),
                // The player stays: pressing play again tries again, as on the web, rather than
                // retrying alone, since each try costs the instance one whole decryption per
                // 8 MiB range.
                Ok(Err(error)) => {
                    tracing::warn!(%error, "cannot download a sound to play");
                    cx.global_mut::<AttachmentAudio>()
                        .stopped
                        .insert(attachment_id);
                }
                Err(_) => {
                    tracing::warn!("the sound download stopped");
                    cx.global_mut::<AttachmentAudio>()
                        .stopped
                        .insert(attachment_id);
                }
            }
            keep_redrawing(cx);
            cx.refresh_windows();
        });
    });
    cx.global_mut::<AttachmentAudio>().fetching = Some((attachment_id, task, Instant::now()));
}

/// Starts the player at the first play; false when it cannot start.
fn ensure_player(output_device_id: Option<String>, cx: &mut App) -> bool {
    let store = cx.global_mut::<AttachmentAudio>();
    if store.player.is_some() {
        return true;
    }
    match ClipPlayer::new(Box::new(SystemAudioDevices)) {
        Ok(player) => {
            // Attachments play where voices do (the web's `setSinkId` on its elements).
            player.set_output_device(output_device_id);
            store.player = Some(player);
            true
        }
        Err(error) => {
            tracing::warn!(error = %error.kind(), "cannot start the attachment player");
            false
        }
    }
}

/// Plays a card's YouTube video (ADR-0063): its tracks are asked of YouTube on the network
/// runtime, then streamed, the sound by the player and the pictures following it. It shows as
/// loading until the player has opened its sound.
fn stream_youtube_video(attachment_id: Snowflake, cx: &mut App) {
    let Some(video) = youtube_video_of(attachment_id, cx) else {
        cx.default_global::<AttachmentAudio>()
            .unplayable
            .insert(attachment_id);
        return;
    };
    let output_device_id = crate::preferences::preferences(cx)
        .and_then(|preferences| preferences.voice_settings.output_device_id.clone());
    stop_playback(cx);
    set_youtube_failure(attachment_id, None, cx);
    if !ensure_player(output_device_id, cx) {
        return;
    }
    let tracks = youtube_tracks(video.video_id.clone());
    ask_for_youtube_tracks(attachment_id, video, tracks, cx);
}

/// Waits for `tracks` on the network runtime, the video loading meanwhile, then plays them or
/// says why it cannot.
fn ask_for_youtube_tracks(
    attachment_id: Snowflake,
    video: YouTubeVideo,
    tracks: impl Future<Output = Result<YouTubeTracks, YouTubeFailure>> + Send + 'static,
    cx: &mut App,
) {
    let session = cx.global::<AttachmentAudio>().session;
    let asking = spawn_on_network_runtime(cx, tracks);
    let task = cx.spawn(async move |cx| {
        let answer = asking.await;
        cx.update(|cx| {
            let store = cx.global_mut::<AttachmentAudio>();
            let is_still_wanted = store.session == session
                && store
                    .fetching
                    .as_ref()
                    .is_some_and(|(id, _, _)| *id == attachment_id);
            if !is_still_wanted {
                return;
            }
            match answer {
                Ok(Ok(tracks)) => {
                    apply_volume(attachment_volume(cx), cx);
                    let start =
                        start_of_youtube(&video).map_or(ClipStart::Beginning, ClipStart::At);
                    start_video(
                        attachment_id,
                        |timing| VideoStream::start_remote(tracks.pictures, timing),
                        cx,
                    );
                    if let Some(player) = cx.global::<AttachmentAudio>().player.as_ref() {
                        player.load_remote_and_play_from(attachment_id.raw(), tracks.sound, start);
                    }
                    // Still loading: the redraw loop ends it once the player holds the sound.
                }
                Ok(Err(failure)) => {
                    set_youtube_failure(attachment_id, Some(failure), cx);
                    let store = cx.global_mut::<AttachmentAudio>();
                    store.fetching = None;
                    store.unplayable.insert(attachment_id);
                }
                // The extraction panicked (rustypipe unwraps some network errors): the card
                // says it could not play and offers to try again, rather than silently going
                // back to its play key as if the press had been lost.
                Err(_) => {
                    tracing::warn!("asking YouTube for a video's tracks stopped");
                    set_youtube_failure(attachment_id, Some(YouTubeFailure::CannotRead), cx);
                    let store = cx.global_mut::<AttachmentAudio>();
                    store.fetching = None;
                    store.unplayable.insert(attachment_id);
                }
            }
            keep_redrawing(cx);
            cx.refresh_windows();
        });
    });
    cx.global_mut::<AttachmentAudio>().fetching = Some((attachment_id, task, Instant::now()));
}

/// Where a sound starts for a press at `fraction` of it: a time when its length is known here
/// (some files do not tell the decoder theirs), else the share of the length the file says.
fn start_of(fraction: Option<f32>, known_duration: Option<Duration>) -> ClipStart {
    match (fraction.filter(|fraction| *fraction > 0.0), known_duration) {
        (None, _) => ClipStart::Beginning,
        (Some(fraction), Some(duration)) => ClipStart::At(duration.mul_f32(fraction.min(1.0))),
        (Some(fraction), None) => ClipStart::Fraction(fraction),
    }
}

/// A file being fetched to play, and how it plays once here.
struct Fetched {
    attachment_id: Snowflake,
    content_type: String,
    is_video: bool,
    /// Whether its shape is read from the same bytes.
    reads_shape: bool,
}

/// Where `attachment_id` starts and how fast, as its card says once its file is here: a press
/// on the waveform or a speed chosen while it loaded counts. Another sound's press stays.
fn take_start_and_speed(store: &mut AttachmentAudio, attachment_id: Snowflake) -> (ClipStart, f32) {
    let fraction = match store.pending_start {
        Some((id, fraction)) if id == attachment_id => {
            store.pending_start = None;
            Some(fraction)
        }
        _ => None,
    };
    let known_duration = store
        .shapes
        .get(attachment_id)
        .and_then(|shape| shape.duration);
    (
        start_of(fraction, known_duration),
        store.speeds.get(&attachment_id).copied().unwrap_or(1.0),
    )
}

/// Plays a fetched file: its pictures too for a video, its shape read for a sound.
fn play_fetched(fetched: &Fetched, bytes: Arc<[u8]>, cx: &mut App) {
    let attachment_id = fetched.attachment_id;
    // The volume as it is now (it may have moved during the download), set before it starts.
    apply_volume(attachment_volume(cx), cx);
    if fetched.is_video {
        let (bytes, content_type) = (Arc::clone(&bytes), fetched.content_type.clone());
        start_video(
            attachment_id,
            move |timing| VideoStream::start(bytes, &content_type, timing),
            cx,
        );
    }
    if fetched.reads_shape {
        read_sound_shape(attachment_id, Arc::clone(&bytes), &fetched.content_type, cx);
    }
    let store = cx.global_mut::<AttachmentAudio>();
    let (start, speed) = take_start_and_speed(store, attachment_id);
    if let Some(player) = store.player.as_ref() {
        player.load_and_play_from(attachment_id.raw(), bytes, &fetched.content_type, start);
        #[allow(clippy::float_cmp)] // A new sound starts at exactly 1.
        if speed != 1.0 {
            player.set_speed(speed);
        }
    }
}

/// Reads the shape of a sound just fetched to play, off the interface thread: the same bytes,
/// decoded by `mellow-media` (DESIGN.md 5.4, "Sound player"). A sound it cannot read keeps the
/// flat line and is not read again this session.
fn read_sound_shape(attachment_id: Snowflake, bytes: Arc<[u8]>, content_type: &str, cx: &mut App) {
    // A token number, a few dozen at most.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let bar_count = crate::theme::tokens::COMPONENT_AUDIO_PLAYER_WAVE_BAR_COUNT as usize;
    let content_type = content_type.to_owned();
    let session = cx.global::<AttachmentAudio>().session;
    cx.global_mut::<AttachmentAudio>()
        .shapes
        .being_read
        .insert(attachment_id);
    let reading = cx
        .background_executor()
        .spawn(async move { sound_waveform(bytes, &content_type, bar_count) });
    cx.spawn(async move |cx| {
        let waveform = reading.await;
        cx.update(|cx| {
            let store = cx.global_mut::<AttachmentAudio>();
            if store.session != session {
                return;
            }
            let shapes = &mut store.shapes;
            shapes.being_read.remove(&attachment_id);
            match waveform {
                Ok(waveform) => shapes.remember(
                    attachment_id,
                    SoundShape {
                        levels: Some(waveform.levels),
                        duration: Some(waveform.duration),
                    },
                ),
                Err(error) => {
                    // Without the file's name (logs never hold attachment names).
                    tracing::debug!(%error, "cannot read a sound's waveform");
                    shapes.remember_failure(attachment_id);
                }
            }
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Starts decoding a video's pictures with `start`, timed by the player's position of its sound.
fn start_video(
    attachment_id: Snowflake,
    start: impl FnOnce(VideoTiming) -> std::io::Result<VideoStream>,
    cx: &mut App,
) {
    let Some(clock) = cx
        .global::<AttachmentAudio>()
        .player
        .as_ref()
        .map(|player| player.clock(attachment_id.raw()))
    else {
        return;
    };
    let stream = match start(VideoTiming::Follow(Box::new(clock))) {
        Ok(stream) => stream,
        Err(error) => {
            tracing::warn!(error = %error.kind(), "cannot start a video's decoder");
            return;
        }
    };
    let wake = Arc::new(tokio::sync::Notify::new());
    let waker = Arc::clone(&wake);
    stream.set_waker(move || waker.notify_one());
    let pump = cx.spawn(async move |cx| {
        loop {
            wake.notified().await;
            if !cx.update(|cx| show_new_video_picture(attachment_id, cx)) {
                return;
            }
        }
    });
    cx.global_mut::<AttachmentAudio>().video = Some(PlayingVideo {
        attachment_id,
        stream,
        picture: None,
        _pump: pump,
    });
}

/// Takes the loaded video's newest picture; false once that video is gone. A video the system
/// cannot decode stops everything and its card offers the file.
fn show_new_video_picture(attachment_id: Snowflake, cx: &mut App) -> bool {
    let Some(video) = cx
        .try_global::<AttachmentAudio>()
        .and_then(|store| store.video.as_ref())
        .filter(|video| video.attachment_id == attachment_id)
    else {
        return false;
    };
    if let VideoStreamPhase::Failed(error) = video.stream.status().phase {
        tracing::warn!(%error, "cannot play a video here");
        stop_playback(cx);
        cx.global_mut::<AttachmentAudio>()
            .unplayable
            .insert(attachment_id);
        cx.refresh_windows();
        return false;
    }
    let Some(picture) = video
        .stream
        .take_new_picture()
        .and_then(VideoPicture::from_decoded)
    else {
        return true;
    };
    let previous = cx
        .global_mut::<AttachmentAudio>()
        .video
        .as_mut()
        .and_then(|video| video.picture.replace(picture));
    if let Some(previous) = previous {
        previous.free(cx);
    }
    cx.refresh_windows();
    true
}

/// Redraws while something plays or loads, so the seek bar and time move; stops by itself.
fn keep_redrawing(cx: &mut App) {
    let store = cx.default_global::<AttachmentAudio>();
    store.settling_redraws = SETTLING_REDRAWS;
    if store.redraw.is_some() {
        return;
    }
    let task = cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(PLAYING_REDRAW_PERIOD).await;
            let is_active = cx.update(|cx| {
                cx.refresh_windows();
                let store = cx.global_mut::<AttachmentAudio>();
                // A streamed video loads until the player has opened its sound.
                if let Some(status) = store.player.as_ref().map(ClipPlayer::status)
                    && let Some((id, _, _)) = store.fetching.as_ref()
                    && status.clip == Some(id.raw())
                {
                    store.fetching = None;
                }
                // The length stays known for the card once another sound plays.
                if let Some(status) = store.player.as_ref().map(ClipPlayer::status)
                    && let (Some(clip), Some(duration)) = (status.clip, status.duration)
                {
                    store
                        .shapes
                        .remember_duration(Snowflake::from_raw(clip), duration);
                }
                let is_active = store.is_active();
                store.settling_redraws = store.settling_redraws.saturating_sub(1);
                if !is_active {
                    store.redraw = None;
                }
                is_active
            });
            if !is_active {
                return;
            }
        }
    });
    cx.global_mut::<AttachmentAudio>().redraw = Some(task);
}

#[cfg(test)]
mod tests {
    use gpui::{
        Context, IntoElement, ParentElement as _, Render, Styled as _, TestAppContext, div,
    };

    use super::*;

    /// Asks for a video's tracks with `tracks` in place of YouTube, and waits for the answer.
    async fn answer_youtube_tracks(
        tracks: impl Future<Output = Result<YouTubeTracks, YouTubeFailure>> + Send + 'static,
        cx: &mut TestAppContext,
    ) -> (AudioPlayback, Option<YouTubeFailure>) {
        /// Each turn lets the network runtime's thread run; the answer comes in a few.
        const MAXIMUM_TURNS: usize = 500;
        let video_id = Snowflake::from_raw(5);
        let runtime = crate::tokio_bridge::build_network_runtime().unwrap();
        cx.update(|cx| {
            crate::tokio_bridge::install_network_runtime(runtime.handle().clone(), cx);
            let video = YouTubeVideo {
                link: "https://www.youtube.com/watch?v=aqz-KE-bpKQ".to_owned(),
                video_id: "aqz-KE-bpKQ".to_owned(),
                start_seconds: None,
            };
            // The player's store, as the press that asks for the tracks leaves it.
            cx.default_global::<AttachmentAudio>();
            ask_for_youtube_tracks(video_id, video, tracks, cx);
        });
        cx.executor().allow_parking();
        let mut answer = None;
        for _ in 0..MAXIMUM_TURNS {
            cx.run_until_parked();
            let playback = cx.update(|cx| attachment_audio_view(video_id, cx).playback);
            if playback != AudioPlayback::Loading {
                answer = Some(playback);
                break;
            }
            cx.background_executor.timer(Duration::from_millis(5)).await;
        }
        runtime.shutdown_background();
        let failure = cx.update(|cx| crate::youtube_video::youtube_failure_of(video_id, cx));
        (answer.expect("YouTube's answer arrives"), failure)
    }

    #[gpui::test]
    async fn a_youtube_refusal_is_said_on_the_card(cx: &mut TestAppContext) {
        let answer = answer_youtube_tracks(async { Err(YouTubeFailure::Private) }, cx).await;
        assert_eq!(
            answer,
            (AudioPlayback::CannotPlay, Some(YouTubeFailure::Private))
        );
    }

    #[gpui::test]
    async fn an_extraction_that_panics_says_it_cannot_read_rather_than_resetting(
        cx: &mut TestAppContext,
    ) {
        // As rustypipe does on some network errors (an `unwrap` in its visitor data).
        let answer = answer_youtube_tracks(
            async { panic!("error sending request for url (https://music.youtube.com/)") },
            cx,
        )
        .await;
        assert_eq!(
            answer,
            (AudioPlayback::CannotPlay, Some(YouTubeFailure::CannotRead))
        );
    }

    fn attachment(content_type: &str, size_bytes: u64) -> Attachment {
        Attachment {
            id: Snowflake::from_raw(9),
            file_name: "mémo.opus".to_owned(),
            content_type: content_type.to_owned(),
            size_bytes,
            width: None,
            height: None,
        }
    }

    #[gpui::test]
    fn a_file_too_large_or_of_another_type_is_offered_without_being_fetched(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let too_large = attachment("audio/ogg", MAXIMUM_PLAYED_ATTACHMENT_BYTES + 1);
            assert!(is_played_in_app(&too_large));
            toggle_attachment_audio(&too_large, cx);
            assert_eq!(
                attachment_audio_view(too_large.id, cx).playback,
                AudioPlayback::CannotPlay
            );
            assert!(!is_played_in_app(&attachment("audio/x-matroska", 10)));
            // Signing out forgets it, so another session asks again.
            end_attachment_audio_session(cx);
            assert_eq!(
                attachment_audio_view(too_large.id, cx).playback,
                AudioPlayback::Idle
            );
        });
    }

    #[test]
    fn shapes_are_bounded_oldest_first_and_failures_are_not_read_again() {
        let mut shapes = SoundShapes::default();
        let shape = |level: f32| SoundShape {
            levels: Some(vec![level]),
            duration: Some(Duration::from_secs(1)),
        };
        for raw in 0..=200 {
            shapes.remember(Snowflake::from_raw(raw), shape(0.5));
        }
        assert!(
            shapes.get(Snowflake::from_raw(0)).is_none(),
            "the oldest went"
        );
        assert!(shapes.get(Snowflake::from_raw(200)).is_some());
        assert_eq!(shapes.shapes.len(), MAXIMUM_REMEMBERED_SOUNDS);
        assert!(!shapes.should_read(Snowflake::from_raw(200)));

        // A length learnt from the player keeps the shape already read.
        shapes.remember_duration(Snowflake::from_raw(200), Duration::from_secs(9));
        assert_eq!(
            shapes.get(Snowflake::from_raw(200)),
            Some(&SoundShape {
                levels: Some(vec![0.5]),
                duration: Some(Duration::from_secs(9)),
            })
        );
        // A length alone still asks for the shape.
        shapes.remember_duration(Snowflake::from_raw(500), Duration::from_secs(3));
        assert!(shapes.should_read(Snowflake::from_raw(500)));

        let unreadable = Snowflake::from_raw(900);
        shapes.remember_failure(unreadable);
        assert!(!shapes.should_read(unreadable));
        for raw in 1_000..1_200 {
            shapes.remember_failure(Snowflake::from_raw(raw));
        }
        assert!(
            shapes.should_read(unreadable),
            "forgotten once 200 others failed"
        );
        assert_eq!(shapes.failed.len(), MAXIMUM_REMEMBERED_SOUNDS);
    }

    #[gpui::test]
    fn a_press_before_the_sound_is_loaded_is_kept_for_its_first_play(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let sound = attachment("audio/ogg", 10);
            seek_attachment_audio_to_fraction(sound.id, 0.4, cx);
            let view = attachment_audio_view(sound.id, cx);
            assert_eq!(view.playback, AudioPlayback::Idle);
            assert_eq!(view.pending_start, Some(0.4));
            assert_eq!(
                attachment_audio_view(Snowflake::from_raw(10), cx).pending_start,
                None,
                "another sound's card"
            );
            set_attachment_speed(sound.id, 1.5, cx);
            assert!((attachment_speed(sound.id, cx) - 1.5).abs() < f32::EPSILON);
            end_attachment_audio_session(cx);
            assert_eq!(attachment_audio_view(sound.id, cx).pending_start, None);
            assert!((attachment_speed(sound.id, cx) - 1.0).abs() < f32::EPSILON);
        });
    }

    #[gpui::test]
    fn a_speed_and_a_press_while_it_loads_are_kept_and_pause_cancels_the_load(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let (sound, other) = (Snowflake::from_raw(21), Snowflake::from_raw(22));
            let loading = |cx: &mut App| {
                cx.default_global::<AttachmentAudio>().fetching =
                    Some((sound, Task::ready(()), Instant::now()));
            };
            seek_attachment_audio_to_fraction(other, 0.3, cx);
            loading(cx);
            assert_eq!(
                attachment_audio_view(sound, cx).playback,
                AudioPlayback::Loading
            );
            set_attachment_speed(sound, 2.0, cx);
            seek_attachment_audio_to_fraction(sound, 0.25, cx);
            let store = cx.global_mut::<AttachmentAudio>();
            assert_eq!(
                take_start_and_speed(store, sound),
                (ClipStart::Fraction(0.25), 2.0)
            );
            seek_attachment_audio_to_fraction(other, 0.3, cx);
            let store = cx.global_mut::<AttachmentAudio>();
            assert_eq!(
                take_start_and_speed(store, sound),
                (ClipStart::Beginning, 2.0)
            );
            assert_eq!(
                store.pending_start,
                Some((other, 0.3)),
                "another sound's press stays"
            );

            // Pause while loading drops the download.
            loading(cx);
            let sound_attachment = Attachment {
                id: sound,
                ..attachment("audio/ogg", 10)
            };
            toggle_attachment_audio(&sound_attachment, cx);
            assert_eq!(
                attachment_audio_view(sound, cx).playback,
                AudioPlayback::Idle
            );
        });
    }

    #[test]
    fn a_press_starts_at_a_time_when_the_length_is_known_here() {
        assert_eq!(start_of(None, None), ClipStart::Beginning);
        assert_eq!(
            start_of(Some(0.0), Some(Duration::from_secs(10))),
            ClipStart::Beginning
        );
        assert_eq!(
            start_of(Some(0.5), Some(Duration::from_secs(10))),
            ClipStart::At(Duration::from_secs(5))
        );
        assert_eq!(start_of(Some(0.5), None), ClipStart::Fraction(0.5));
    }

    #[test]
    fn videos_are_played_where_the_system_decodes_their_container() {
        let mp4 = attachment("video/mp4", 10);
        assert_eq!(
            is_video_played_in_app(&mp4),
            cfg!(any(target_os = "linux", target_os = "macos"))
        );
        assert!(!is_played_in_app(&mp4), "a video is not a sound card");
        assert_eq!(
            is_video_played_in_app(&attachment("video/webm", 10)),
            cfg!(target_os = "linux")
        );
        assert!(!is_video_played_in_app(&attachment("video/x-msvideo", 10)));
    }

    /// List rows holding players, each drawn until it scrolls away.
    struct Rows {
        shown: Vec<Snowflake>,
    }

    impl Render for Rows {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            for attachment_id in &self.shown {
                let id = ElementId::Name(format!("row-{attachment_id}").into());
                keep_player_while_shown(&id, *attachment_id, window, cx);
            }
            div().size_full().child("rows")
        }
    }

    #[gpui::test]
    fn a_player_whose_row_leaves_the_list_stops(cx: &mut TestAppContext) {
        let (playing, other) = (Snowflake::from_raw(1), Snowflake::from_raw(2));
        let (rows, cx) = cx.add_window_view(|_, _| Rows {
            shown: vec![playing, other],
        });
        cx.run_until_parked();
        // The first row's attachment is being fetched, as after pressing play.
        cx.update(|_, cx| {
            cx.default_global::<AttachmentAudio>().fetching =
                Some((playing, Task::ready(()), Instant::now()));
        });
        let playback = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, cx| attachment_audio_view(playing, cx).playback)
        };
        assert_eq!(playback(cx), AudioPlayback::Loading);

        let scroll_to = |shown: Vec<Snowflake>, cx: &mut gpui::VisualTestContext| {
            rows.update(cx, |rows, cx| {
                rows.shown = shown;
                cx.notify();
            });
            cx.run_until_parked();
            // GPUI keeps the previous frame's element states one frame more.
            rows.update(cx, |_, cx| cx.notify());
            cx.run_until_parked();
        };
        scroll_to(vec![playing], cx);
        assert_eq!(playback(cx), AudioPlayback::Loading, "another row left");
        scroll_to(Vec::new(), cx);
        assert_eq!(playback(cx), AudioPlayback::Idle, "stopped with its row");
    }

    /// Surfaces showing players under their own ids: a card, the media viewer's screen.
    struct Surfaces {
        shown: Vec<(&'static str, Snowflake)>,
    }

    impl Render for Surfaces {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            for (name, attachment_id) in &self.shown {
                keep_player_while_shown(
                    &ElementId::Name((*name).into()),
                    *attachment_id,
                    window,
                    cx,
                );
            }
            div().size_full().child("surfaces")
        }
    }

    #[gpui::test]
    fn a_player_shown_in_the_viewer_too_stops_only_once_neither_shows_it(cx: &mut TestAppContext) {
        let video = Snowflake::from_raw(3);
        let (surfaces, cx) = cx.add_window_view(|_, _| Surfaces {
            shown: vec![("card", video), ("viewer", video)],
        });
        cx.run_until_parked();
        cx.update(|_, cx| pretend_fetching_for_tests(video, cx));
        let show = |shown: Vec<(&'static str, Snowflake)>, cx: &mut gpui::VisualTestContext| {
            surfaces.update(cx, |surfaces, cx| {
                surfaces.shown = shown;
                cx.notify();
            });
            cx.run_until_parked();
            surfaces.update(cx, |_, cx| cx.notify());
            cx.run_until_parked();
        };
        let playback = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, cx| attachment_audio_view(video, cx).playback)
        };
        // The row left the list (new messages, a deletion): the viewer still plays it.
        show(vec![("viewer", video)], cx);
        assert_eq!(playback(cx), AudioPlayback::Loading);
        show(Vec::new(), cx);
        assert_eq!(playback(cx), AudioPlayback::Idle);
    }
}
