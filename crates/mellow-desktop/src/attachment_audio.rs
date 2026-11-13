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

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, ElementId, Global, Task, Window};
use mellow_client_core::{DownloadableFile, RestClient};
use mellow_media::{
    ClipPhase, ClipPlayer, PLAYABLE_ATTACHMENT_AUDIO_TYPES, SystemAudioDevices, VideoStream,
    VideoStreamPhase, VideoTiming, plays_video_type,
};
use mellow_protocol::{Attachment, Snowflake};

use crate::components::child_id;
use crate::preferences::{change_preferences, preferences};
use crate::tokio_bridge::spawn_on_network_runtime;
use crate::voice::video_view::VideoPicture;

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

/// What a sound's player shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioPlayback {
    /// Not the loaded sound: at 0:00, nothing fetched.
    Idle,
    /// Its file is being fetched.
    Loading,
    Playing,
    Paused,
    Ended,
    NoOutput,
    /// Not played here: the card says so and offers the file instead.
    CannotPlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioView {
    pub playback: AudioPlayback,
    pub position: Duration,
    pub duration: Option<Duration>,
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
    /// The attachment whose file is on its way, and the download.
    fetching: Option<(Snowflake, Task<()>)>,
    /// Refused before any download (type or size), or by a decoder.
    unplayable: HashSet<Snowflake>,
    /// Redraws the windows while something plays or loads.
    redraw: Option<Task<()>>,
    /// Redraws left after the last command, so its answer shows even once nothing plays.
    settling_redraws: u8,
    /// Bumped by each session, so a late download of the previous one is dropped.
    session: u64,
}

impl Global for AttachmentAudio {}

impl AttachmentAudio {
    /// Stops the sound and drops the video; its last picture is handed back to be freed.
    #[must_use]
    fn stop(&mut self) -> Option<VideoPicture> {
        self.fetching = None;
        if let Some(player) = self.player.as_ref() {
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
            .is_some_and(|(id, _)| *id == attachment_id)
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
    store.unplayable.clear();
    store.session += 1;
    store.rest = Some(rest);
}

/// Signing out stops the sound and forgets the session's files.
pub fn end_attachment_audio_session(cx: &mut App) {
    stop_playback(cx);
    let store = cx.default_global::<AttachmentAudio>();
    store.unplayable.clear();
    store.session += 1;
    store.rest = None;
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

/// Keeps `attachment_id`'s player for as long as the element `id` is drawn: once its row leaves
/// the list (scrolled far away, the conversation closed), the sound or video it plays stops, as
/// the web's element is removed with its row.
pub(crate) fn keep_player_while_shown(
    id: &ElementId,
    attachment_id: Snowflake,
    window: &mut Window,
    cx: &mut App,
) {
    window.use_keyed_state(child_id(id, "shown"), cx, |_, cx| {
        cx.on_release(move |(), cx| {
            if cx
                .try_global::<AttachmentAudio>()
                .is_some_and(|store| store.holds(attachment_id))
            {
                stop_playback(cx);
                cx.refresh_windows();
            }
        })
        .detach();
    });
}

/// Where `attachment_id`'s player stands.
#[must_use]
pub fn attachment_audio_view(attachment_id: Snowflake, cx: &App) -> AudioView {
    let idle = AudioView {
        playback: AudioPlayback::Idle,
        position: Duration::ZERO,
        duration: None,
    };
    let Some(store) = cx.try_global::<AttachmentAudio>() else {
        return idle;
    };
    if store.unplayable.contains(&attachment_id) {
        return AudioView {
            playback: AudioPlayback::CannotPlay,
            ..idle
        };
    }
    if store
        .fetching
        .as_ref()
        .is_some_and(|(id, _)| *id == attachment_id)
    {
        return AudioView {
            playback: AudioPlayback::Loading,
            ..idle
        };
    }
    let Some(status) = store
        .player
        .as_ref()
        .map(ClipPlayer::status)
        .filter(|status| status.clip == Some(attachment_id.raw()))
    else {
        return idle;
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
        duration: status.duration,
    }
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
        AudioPlayback::Idle => fetch_and_play(attachment, cx),
        AudioPlayback::Loading | AudioPlayback::CannotPlay => return,
    }
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
    // The sound the player refused stays refused: pressing play again would fetch it again only
    // to fail the same way.
    if let Some(status) = store.player.as_ref().map(ClipPlayer::status)
        && let (Some(refused), ClipPhase::CannotPlay(_)) = (status.clip, status.phase)
    {
        store.unplayable.insert(Snowflake::from_raw(refused));
    }
    stop_playback(cx);
    let store = cx.global_mut::<AttachmentAudio>();
    if store.player.is_none() {
        match ClipPlayer::new(Box::new(SystemAudioDevices)) {
            Ok(player) => {
                // Attachments play where voices do (the web's `setSinkId` on its elements).
                player.set_output_device(output_device_id);
                store.player = Some(player);
            }
            Err(error) => {
                tracing::warn!(error = %error.kind(), "cannot start the attachment player");
                return;
            }
        }
    }
    let session = store.session;
    let content_type = attachment.content_type.clone();
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
                    .is_some_and(|(id, _)| *id == attachment_id);
            if !is_still_wanted {
                return;
            }
            store.fetching = None;
            match answer {
                Ok(Ok(bytes)) => {
                    // The volume as it is now (it may have moved during the download), set
                    // before the sound starts.
                    apply_volume(attachment_volume(cx), cx);
                    if is_video {
                        start_video(attachment_id, Arc::clone(&bytes), &content_type, cx);
                    }
                    if let Some(player) = cx.global::<AttachmentAudio>().player.as_ref() {
                        player.load_and_play(attachment_id.raw(), bytes, &content_type);
                    }
                }
                // The player stays: pressing play again tries again, as on the web, rather than
                // retrying alone, since each try costs the instance one whole decryption per
                // 8 MiB range.
                Ok(Err(error)) => tracing::warn!(%error, "cannot download a sound to play"),
                Err(_) => tracing::warn!("the sound download stopped"),
            }
            keep_redrawing(cx);
            cx.refresh_windows();
        });
    });
    cx.global_mut::<AttachmentAudio>().fetching = Some((attachment_id, task));
}

/// Starts decoding a fetched video's pictures, timed by the player's position of its sound.
fn start_video(attachment_id: Snowflake, bytes: Arc<[u8]>, content_type: &str, cx: &mut App) {
    let Some(clock) = cx
        .global::<AttachmentAudio>()
        .player
        .as_ref()
        .map(|player| player.clock(attachment_id.raw()))
    else {
        return;
    };
    let stream = match VideoStream::start(bytes, content_type, VideoTiming::Follow(Box::new(clock)))
    {
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
            cx.default_global::<AttachmentAudio>().fetching = Some((playing, Task::ready(())));
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
}
