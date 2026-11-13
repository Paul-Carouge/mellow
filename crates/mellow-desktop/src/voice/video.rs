//! What the call shows of others' cameras and screens (`voice-session.svelte.ts`'s watching,
//! ADR-0035, ADR-0037): whether the call view is open, the screen watched large, the cameras the
//! call view shows and how large, the pictures the instance paused, and each sharer's screen
//! sound volume. The watch list goes to the engine as one list, the watched screen first.

use std::collections::{HashMap, HashSet};

use gpui::{App, Context, Entity, Task};
use mellow_client_core::shared_logic::call_view::{WatchedPicture, watch_list_of};
use mellow_client_core::shared_logic::voice_settings::{
    MAXIMUM_VOLUME_PERCENT, PersonVoiceSetting,
};
use mellow_media::{
    InMemoryPersonVoiceSettings, PersonSoundSource, PersonVoiceSettingsStore as _, VoiceSession,
    WatchedVideo,
};
use mellow_protocol::{Snowflake, VideoCodec, VideoTileSize};

use super::Voice;
use super::media_settings::person_setting_of;
use super::video_view::{VideoPictures, new_video_pictures, pump_video_pictures};
use crate::preferences::{change_preferences, preferences};

/// A picture the interface shows, at its drawn size in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShownPicture {
    pub user_id: Snowflake,
    pub tile_size: VideoTileSize,
    pub display_width: u32,
    pub display_height: u32,
}

/// The call's video, kept by [`Voice`].
pub(super) struct CallVideo {
    is_call_view_open: bool,
    watched_stream: Option<ShownPicture>,
    cameras: Vec<ShownPicture>,
    paused_user_ids: HashSet<Snowflake>,
    pictures: Entity<VideoPictures>,
    /// Brings the engine's pictures; replaced with each engine.
    pump: Option<Task<()>>,
}

impl CallVideo {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            is_call_view_open: false,
            watched_stream: None,
            cameras: Vec::new(),
            paused_user_ids: HashSet::new(),
            pictures: new_video_pictures(cx),
            pump: None,
        }
    }

    /// The call's engine started: its pictures come here, and it watches what is shown.
    pub(super) fn attach(&mut self, engine: &mut VoiceSession, cx: &mut App) {
        self.pump = Some(pump_video_pictures(
            engine.video_frames(),
            &self.pictures,
            cx,
        ));
        self.apply(Some(engine), cx);
    }

    /// The call ended: nothing is shown or watched any more, and its last pictures leave memory
    /// and the atlas (a next call would otherwise show them frozen until its engine starts).
    pub(super) fn end_call(&mut self, cx: &mut App) {
        self.is_call_view_open = false;
        self.watched_stream = None;
        self.cameras.clear();
        self.paused_user_ids.clear();
        self.pump = None;
        self.pictures
            .update(cx, |pictures, cx| pictures.keep_only(|_| false, cx));
    }

    pub(super) fn note_paused(&mut self, user_id: Snowflake, is_paused: bool) {
        if is_paused {
            self.paused_user_ids.insert(user_id);
        } else {
            self.paused_user_ids.remove(&user_id);
        }
    }

    /// Sends the watch list (the engine sends only a change) and forgets the pictures of people
    /// no longer watched.
    fn apply(&self, engine: Option<&mut VoiceSession>, cx: &mut App) {
        let sizes: HashMap<Snowflake, ShownPicture> = self
            .cameras
            .iter()
            .chain(self.watched_stream.as_ref())
            .map(|shown| (shown.user_id, *shown))
            .collect();
        let cameras: Vec<WatchedPicture> = self
            .cameras
            .iter()
            .map(|camera| WatchedPicture {
                user_id: camera.user_id,
                size: camera.tile_size,
            })
            .collect();
        let watched: Vec<WatchedVideo> =
            watch_list_of(self.watched_stream.map(|stream| stream.user_id), &cameras)
                .into_iter()
                .filter_map(|picture| sizes.get(&picture.user_id))
                .map(|shown| WatchedVideo {
                    user_id: shown.user_id,
                    tile_size: shown.tile_size,
                    display_width: shown.display_width,
                    display_height: shown.display_height,
                })
                .collect();
        if let Some(engine) = engine {
            engine.watch_videos(&watched);
        }
        let watched_ids: HashSet<Snowflake> = watched.iter().map(|video| video.user_id).collect();
        self.pictures.update(cx, |pictures, cx| {
            pictures.keep_only(|user_id| watched_ids.contains(&user_id), cx);
        });
    }
}

impl Voice {
    fn engine_mut(&mut self) -> Option<&mut VoiceSession> {
        self.call.as_mut().and_then(|call| call.engine.as_mut())
    }

    fn apply_watching(&mut self, cx: &mut Context<Self>) {
        let engine = self.call.as_mut().and_then(|call| call.engine.as_mut());
        self.video.apply(engine, cx);
    }

    /// The call view takes the conversation sheet's place (ADR-0035).
    #[must_use]
    pub fn is_call_view_open(&self) -> bool {
        self.video.is_call_view_open
    }

    /// Opens the call view; only during a call, as there is nothing to show otherwise.
    pub fn open_call_view(&mut self, cx: &mut Context<Self>) {
        let is_open = self.call.is_some();
        if self.video.is_call_view_open != is_open {
            self.video.is_call_view_open = is_open;
            cx.notify();
        }
    }

    /// Back to the chat; the call goes on, its cameras are no longer asked for.
    pub fn close_call_view(&mut self, cx: &mut Context<Self>) {
        if self.video.is_call_view_open {
            self.video.is_call_view_open = false;
            self.video.cameras.clear();
            self.apply_watching(cx);
            cx.notify();
        }
    }

    /// The person whose shared screen is watched large.
    #[must_use]
    pub fn watched_stream_user_id(&self) -> Option<Snowflake> {
        self.video.watched_stream.map(|stream| stream.user_id)
    }

    /// Watches `user_id`'s shared screen large, or nothing with `None`. Its first size is a
    /// guess the stream view corrects once drawn.
    pub fn watch_stream(&mut self, user_id: Option<Snowflake>, cx: &mut Context<Self>) {
        if self.watched_stream_user_id() == user_id || (user_id.is_some() && self.call.is_none()) {
            return;
        }
        self.video.watched_stream = user_id.map(|user_id| ShownPicture {
            user_id,
            tile_size: VideoTileSize::Large,
            display_width: STREAM_FIRST_WIDTH,
            display_height: STREAM_FIRST_HEIGHT,
        });
        self.apply_watching(cx);
        cx.notify();
    }

    /// The size the watched screen is drawn at, in physical pixels.
    pub fn set_stream_display_size(&mut self, width: u32, height: u32, cx: &mut Context<Self>) {
        let Some(stream) = self.video.watched_stream.as_mut() else {
            return;
        };
        if (stream.display_width, stream.display_height) != (width, height) {
            stream.display_width = width;
            stream.display_height = height;
            self.apply_watching(cx);
        }
    }

    /// The cameras and screens the call view shows, and how large; the same list again sends
    /// nothing.
    pub fn watch_cameras(&mut self, cameras: Vec<ShownPicture>, cx: &mut Context<Self>) {
        if self.video.cameras != cameras {
            self.video.cameras = cameras;
            self.apply_watching(cx);
        }
    }

    /// The decoded pictures of the people watched; only the call view observes it.
    #[must_use]
    pub fn video_pictures(&self) -> Entity<VideoPictures> {
        self.video.pictures.clone()
    }

    /// The instance paused `user_id`'s picture for this connection (too slow for it).
    #[must_use]
    pub fn is_video_paused(&self, user_id: Snowflake) -> bool {
        self.video.paused_user_ids.contains(&user_id)
    }

    /// Whether this app can show a picture sent in `codec` (ADR-0052).
    #[must_use]
    pub fn can_show_video_in(codec: Option<VideoCodec>) -> bool {
        mellow_media::decodes_video_codec(codec)
    }

    /// The volume and local mute of `user_id`'s screen sound, apart from their voice.
    #[must_use]
    pub fn stream_sound_setting(user_id: Snowflake, cx: &App) -> PersonVoiceSetting {
        preferences(cx).map_or_else(PersonVoiceSetting::default, |preferences| {
            preferences.stream_person(user_id)
        })
    }

    pub fn set_stream_volume(
        &mut self,
        user_id: Snowflake,
        volume_percent: u32,
        cx: &mut Context<Self>,
    ) {
        let setting = PersonVoiceSetting {
            volume_percent: volume_percent.min(MAXIMUM_VOLUME_PERCENT),
            ..Self::stream_sound_setting(user_id, cx)
        };
        self.set_stream_sound_setting(user_id, setting, cx);
    }

    pub fn toggle_stream_muted(&mut self, user_id: Snowflake, cx: &mut Context<Self>) {
        let current = Self::stream_sound_setting(user_id, cx);
        let setting = PersonVoiceSetting {
            is_locally_muted: !current.is_locally_muted,
            ..current
        };
        self.set_stream_sound_setting(user_id, setting, cx);
    }

    fn set_stream_sound_setting(
        &mut self,
        user_id: Snowflake,
        setting: PersonVoiceSetting,
        cx: &mut Context<Self>,
    ) {
        if let Some(engine) = self.engine_mut() {
            engine.set_person_setting(
                PersonSoundSource::ScreenSound,
                user_id,
                person_setting_of(setting),
            );
        } else if let Some(call) = self.call.as_ref() {
            // The engine reads its store when it starts.
            call.person_settings.write_person_setting(
                PersonSoundSource::ScreenSound,
                user_id,
                person_setting_of(setting).clamped(),
            );
        }
        change_preferences(cx, move |preferences| {
            preferences.set_stream_person(user_id, setting);
        });
        cx.notify();
    }
}

/// The screen sound settings a new engine starts with.
pub(super) fn seed_stream_sound_settings(store: &InMemoryPersonVoiceSettings, cx: &App) {
    if let Some(preferences) = preferences(cx) {
        for (user_id, setting) in &preferences.stream_people {
            store.write_person_setting(
                PersonSoundSource::ScreenSound,
                *user_id,
                person_setting_of(*setting),
            );
        }
    }
}

/// The watched screen's size before the stream view is drawn: a 1080p tile.
const STREAM_FIRST_WIDTH: u32 = 1_920;
const STREAM_FIRST_HEIGHT: u32 = 1_080;

#[cfg(test)]
impl Voice {
    /// A call still starting (no engine), so views of a call can be drawn without a server.
    pub(crate) fn pretend_call_for_tests(&mut self, guild_id: Snowflake, channel_id: Snowflake) {
        self.call = Some(super::Call {
            channel_id,
            guild_id,
            phase: super::CallPhase::Connected,
            round_trip: None,
            is_self_speaking: false,
            devices: super::DeviceNotices::default(),
            start_attempt: self.attempt,
            start_settings:
                mellow_client_core::shared_logic::voice_settings::VoiceSettings::DEFAULT,
            person_settings: std::sync::Arc::new(InMemoryPersonVoiceSettings::default()),
            engine: None,
            event_pump: None,
        });
    }

    /// The cameras the call view asks for, with their sizes.
    pub(crate) fn watched_cameras_for_tests(&self) -> Vec<ShownPicture> {
        self.video.cameras.clone()
    }

    /// As the engine's `VideoPaused` event.
    pub(crate) fn note_video_paused_for_tests(
        &mut self,
        user_id: Snowflake,
        is_paused: bool,
        cx: &mut Context<Self>,
    ) {
        self.video.note_paused(user_id, is_paused);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::PreferencesStore;

    #[gpui::test]
    fn a_new_engine_starts_with_the_saved_screen_sound_volumes(cx: &mut gpui::TestAppContext) {
        let ana = Snowflake::from_raw(1);
        let quieter = PersonVoiceSetting {
            volume_percent: 40,
            is_locally_muted: true,
        };
        cx.update(|cx| {
            cx.set_global(PreferencesStore::default());
            change_preferences(cx, move |preferences| {
                preferences.set_stream_person(ana, quieter);
            });
        });
        let store = InMemoryPersonVoiceSettings::default();
        cx.update(|cx| seed_stream_sound_settings(&store, cx));
        assert_eq!(
            store.person_setting(PersonSoundSource::ScreenSound, ana),
            person_setting_of(quieter)
        );
        assert_eq!(
            store.person_setting(PersonSoundSource::Voice, ana),
            mellow_media::PersonVoiceSetting::default(),
            "the voice's own setting is apart"
        );
    }
}
