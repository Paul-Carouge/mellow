//! Sounds attached to messages, played in the app (ADR-0038, `web-inline-media.md`): one at a
//! time, as the web pauses every other player; nothing is fetched until someone presses play, the
//! file comes through byte ranges on the authenticated download path, and `mellow-media`'s
//! [`ClipPlayer`] decodes it off the interface thread. Opening another channel or signing out
//! stops the sound and frees it. The volume is remembered for the next sound, as the web's
//! `mellow.media-volume`.
//!
//! shortcut: a sound keeps playing when its row scrolls far out of the list (the web's element is
//! removed with the row, which pauses it); stop it from the list once rows report being dropped.
//! shortcut: sounds play on the system's default output: the desktop has no voice settings yet;
//! call `ClipPlayer::set_output_device` with the chosen output when they come (M3 step 7).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Global, Task};
use mellow_client_core::{DownloadableFile, RestClient};
use mellow_media::{ClipPhase, ClipPlayer, PLAYABLE_ATTACHMENT_AUDIO_TYPES, SystemAudioDevices};
use mellow_protocol::{Attachment, Snowflake};

use crate::preferences::{change_preferences, preferences};
use crate::tokio_bridge::spawn_on_network_runtime;

/// Larger sounds are offered as downloads: the whole file is held in memory while it plays. The
/// instance's default limit is 25 MB.
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

#[derive(Default)]
struct AttachmentAudio {
    rest: Option<RestClient>,
    /// Started at the first play.
    player: Option<ClipPlayer>,
    /// The attachment whose file is on its way, and the download.
    fetching: Option<(Snowflake, Task<()>)>,
    /// Refused before any download (type or size).
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
    fn stop(&mut self) {
        self.fetching = None;
        if let Some(player) = self.player.as_ref() {
            player.stop();
        }
    }

    fn is_active(&self) -> bool {
        self.settling_redraws > 0
            || self.fetching.is_some()
            || self
                .player
                .as_ref()
                .is_some_and(|player| player.status().phase == ClipPhase::Playing)
    }
}

/// Sounds of a signed-in session download with `rest`.
pub fn start_attachment_audio_session(rest: RestClient, cx: &mut App) {
    let store = cx.default_global::<AttachmentAudio>();
    store.stop();
    store.unplayable.clear();
    store.session += 1;
    store.rest = Some(rest);
}

/// Signing out stops the sound and forgets the session's files.
pub fn end_attachment_audio_session(cx: &mut App) {
    let store = cx.default_global::<AttachmentAudio>();
    store.stop();
    store.unplayable.clear();
    store.session += 1;
    store.rest = None;
}

/// Another channel opened: the sound stops and is freed, as the web's element goes with the page.
pub fn stop_attachment_audio(cx: &mut App) {
    if let Some(store) = cx.try_global::<AttachmentAudio>()
        && (store.fetching.is_some() || store.player.is_some())
    {
        cx.global_mut::<AttachmentAudio>().stop();
        cx.refresh_windows();
    }
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

/// Whether the app plays this attachment itself rather than offering only the file.
#[must_use]
pub fn is_played_in_app(attachment: &Attachment) -> bool {
    PLAYABLE_ATTACHMENT_AUDIO_TYPES.contains(&attachment.content_type.as_str())
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
    // Checked before anything is fetched or decoded: the player checks the bytes again.
    if !is_played_in_app(attachment) || attachment.size_bytes > MAXIMUM_PLAYED_ATTACHMENT_BYTES {
        cx.default_global::<AttachmentAudio>()
            .unplayable
            .insert(attachment_id);
        return;
    }
    let volume = attachment_volume(cx);
    let store = cx.default_global::<AttachmentAudio>();
    let Some(rest) = store.rest.clone() else {
        return;
    };
    store.stop();
    if store.player.is_none() {
        match ClipPlayer::new(Box::new(SystemAudioDevices)) {
            Ok(player) => store.player = Some(player),
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
                    if let Some(player) = store.player.as_ref() {
                        player.load_and_play(attachment_id.raw(), Arc::from(bytes), &content_type);
                    }
                    apply_volume(volume, cx);
                }
                // The player stays: pressing play again tries again, as on the web, since each
                // try costs the instance a whole decryption.
                Ok(Err(error)) => tracing::warn!(%error, "cannot download a sound to play"),
                Err(_) => tracing::warn!("the sound download stopped"),
            }
            keep_redrawing(cx);
            cx.refresh_windows();
        });
    });
    cx.global_mut::<AttachmentAudio>().fetching = Some((attachment_id, task));
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
}
