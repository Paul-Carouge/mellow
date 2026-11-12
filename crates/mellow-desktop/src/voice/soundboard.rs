//! The soundboard in a call (`soundboard-session.svelte.ts`, `soundboard-audio.ts`, ADR-0032):
//! playing your own sounds, hearing the sounds others play, join sounds in place of the join
//! cues, the emoji by a player's name while their sound plays, and previews for the reader only.
//!
//! Each sound is downloaded once on the network runtime and kept decoded (`SoundCache`, at most
//! 64); a sound still downloading when it is played again is downloaded again (rare: one click
//! every 2 s at most per person). Previews play through the call when there is one, where the
//! echo canceller hears them, else on the chosen output.

use std::time::Duration;

use gpui::{App, Context};
use mellow_client_core::shared_logic::soundboard::{
    JoinCue, SoundboardBlock, SoundboardFacts, join_cue_of, should_hear_sound, soundboard_block,
};
use mellow_media::{
    LocalSound, SoundboardPlayer, SoundboardSoundsToStop, VoiceCue, download_sound, soundboard_gain,
};
use mellow_protocol::Snowflake;

use super::media_settings::gain_of_percent;
use super::{CallPhase, Voice, current_settings};
use crate::session::now_unix_milliseconds;
use crate::tokio_bridge::spawn_on_network_runtime;

/// What a downloaded sound is for once it arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SoundUse {
    /// Played in the call by `player`; `fallback` plays instead when the sound cannot load (a
    /// join sound standing in for its cue).
    Heard {
        player: Snowflake,
        fallback: Option<VoiceCue>,
    },
    Preview,
}

impl Voice {
    // ---- Playing your own ----

    /// Why the soundboard cannot play in the call now, or `None`.
    #[must_use]
    pub fn soundboard_block(
        &self,
        now_unix_milliseconds: u64,
        cx: &App,
    ) -> Option<SoundboardBlock> {
        let state = self.session.read(cx).state();
        let channel_id = self
            .channel_id()
            .filter(|_| self.phase() == Some(CallPhase::Connected));
        soundboard_block(SoundboardFacts {
            is_connected: channel_id.is_some(),
            channel_permissions: channel_id
                .and_then(|channel_id| state.my_channel_permissions(channel_id)),
            is_deafened: self.toggles.is_deafened(),
            now_unix_milliseconds,
            cooldown_ends_at_unix_milliseconds: state.sound_cooldown_ends_at_unix_milliseconds(),
        })
    }

    /// Asks the instance to play `sound_id` to the call; false when it cannot now.
    pub fn play_soundboard_sound(&mut self, sound_id: Snowflake, cx: &mut Context<Self>) -> bool {
        if self.soundboard_block(now_unix_milliseconds(), cx).is_some() {
            return false;
        }
        self.session.update(cx, |session, cx| {
            session.send_soundboard_sound(sound_id, cx)
        })
    }

    // ---- Hearing them ----

    /// `VOICE_SOUND_PLAYED` in the reader's channel (the state already showed its emoji).
    pub(super) fn note_sound_played(
        &mut self,
        channel_id: Snowflake,
        player: Snowflake,
        sound_id: Snowflake,
        cx: &mut Context<Self>,
    ) {
        let is_mine = self
            .session
            .read(cx)
            .state()
            .current_user()
            .is_some_and(|user| user.id == player);
        let is_heard = should_hear_sound(
            channel_id,
            self.channel_id(),
            self.phase() == Some(CallPhase::Connected),
            self.toggles.is_deafened(),
            !is_mine && Self::person_setting(player, cx).is_locally_muted,
        );
        if is_heard {
            self.play_sound(
                sound_id,
                SoundUse::Heard {
                    player,
                    fallback: None,
                },
                cx,
            );
        }
        self.redraw_when_indicator_ends(player, cx);
    }

    /// Someone came into the call: their join sound when it would be heard, else the cue.
    pub(super) fn play_peer_join(
        &mut self,
        guild_id: Snowflake,
        user_id: Snowflake,
        cx: &mut Context<Self>,
    ) {
        let state = self.session.read(cx).state();
        let join_sound_id = state
            .voice_state_of(user_id)
            .and_then(|voice_state| voice_state.join_sound_id);
        let cue = join_cue_of(
            join_sound_id,
            state.sounds_of(guild_id),
            self.toggles.is_deafened(),
            Self::person_setting(user_id, cx).is_locally_muted,
        );
        self.play_join(guild_id, user_id, cue, VoiceCue::PeerJoin, cx);
    }

    /// The reader's own call connected: their join sound in place of the join cue.
    pub(super) fn play_own_join(&mut self, cx: &mut Context<Self>) {
        let Some((guild_id, channel_id)) = self.connected_place() else {
            return;
        };
        let state = self.session.read(cx).state();
        let own = state
            .current_user()
            .and_then(|user| state.voice_state_of(user.id))
            .filter(|own| own.channel_id == channel_id)
            .map(|own| (own.user_id, own.join_sound_id));
        let cue = own.map_or(JoinCue::Chime, |(_, join_sound_id)| {
            join_cue_of(
                join_sound_id,
                state.sounds_of(guild_id),
                self.toggles.is_deafened(),
                false,
            )
        });
        match own {
            Some((user_id, _)) => self.play_join(guild_id, user_id, cue, VoiceCue::SelfJoin, cx),
            None => self.play_call_cue(VoiceCue::SelfJoin),
        }
    }

    fn play_join(
        &mut self,
        guild_id: Snowflake,
        user_id: Snowflake,
        cue: JoinCue,
        join_cue: VoiceCue,
        cx: &mut Context<Self>,
    ) {
        match cue {
            JoinCue::Sound(sound_id) => {
                self.session.update(cx, |session, cx| {
                    session.show_sound_indicator(user_id, guild_id, sound_id, cx);
                });
                self.redraw_when_indicator_ends(user_id, cx);
                self.play_sound(
                    sound_id,
                    SoundUse::Heard {
                        player: user_id,
                        fallback: Some(join_cue),
                    },
                    cx,
                );
            }
            JoinCue::Chime => self.play_call_cue(join_cue),
        }
    }

    pub(super) fn play_call_cue(&self, cue: VoiceCue) {
        if let Some(engine) = self.engine() {
            engine.play_cue(cue);
        }
    }

    /// The emoji by a player's name goes away on its own: the session redraws when it ends.
    fn redraw_when_indicator_ends(&self, user_id: Snowflake, cx: &mut Context<Self>) {
        let now = now_unix_milliseconds();
        let Some(ends_at) = self
            .session
            .read(cx)
            .state()
            .sound_indicator(user_id, now)
            .map(|indicator| indicator.shown_until_unix_milliseconds)
        else {
            return;
        };
        let session = self.session.downgrade();
        let wait = Duration::from_millis(ends_at.saturating_sub(now));
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(wait).await;
            let _ = session.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    // ---- Previews ----

    /// A server sound for the reader only (the popover's and the settings' preview).
    pub fn preview_soundboard_sound(&mut self, sound_id: Snowflake, cx: &mut Context<Self>) {
        self.play_sound(sound_id, SoundUse::Preview, cx);
    }

    /// Samples not on the instance yet ("Add a sound"'s preview), mono at 48 kHz.
    pub fn preview_soundboard_samples(&mut self, samples: LocalSound, cx: &mut Context<Self>) {
        self.play_preview(samples, cx);
    }

    pub fn stop_soundboard_previews(&mut self) {
        if let Some(engine) = self.engine() {
            engine.stop_soundboard_sounds(SoundboardSoundsToStop::Previews);
        }
        if let Some(player) = self.sound_player.as_ref() {
            player.stop_soundboard_previews();
        }
    }

    fn play_preview(&mut self, samples: LocalSound, cx: &mut Context<Self>) {
        if let Some(engine) = self.engine() {
            engine.play_soundboard_sound(samples, SoundboardPlayer::Preview);
            return;
        }
        let settings = current_settings(cx);
        let gain = soundboard_gain(
            gain_of_percent(settings.output_volume_percent),
            gain_of_percent(settings.soundboard_volume_percent),
        );
        if let Some(player) = self.sound_player(cx) {
            player.play_soundboard_preview(samples, gain);
        }
    }

    // ---- Stopping ----

    /// Deafening stops every sound playing; muting someone for the reader stops theirs.
    pub(super) fn stop_soundboard_sounds(&self, which: SoundboardSoundsToStop) {
        if let Some(engine) = self.engine() {
            engine.stop_soundboard_sounds(which);
        }
    }

    // ---- Loading ----

    fn play_sound(&mut self, sound_id: Snowflake, use_: SoundUse, cx: &mut Context<Self>) {
        if let Some(sound) = self.sound_cache.get(sound_id) {
            self.play_loaded(sound, use_, cx);
            return;
        }
        let Some(rest) = self.session.read(cx).rest_client().cloned() else {
            return;
        };
        let download =
            spawn_on_network_runtime(cx, async move { download_sound(&rest, sound_id).await });
        cx.spawn(async move |this, cx| {
            let downloaded = download.await;
            let _ = this.update(cx, |voice, cx| match downloaded {
                Ok(Ok(sound)) => {
                    voice.sound_cache.insert(sound_id, sound.clone());
                    voice.play_loaded(sound, use_, cx);
                }
                Ok(Err(error)) => {
                    tracing::warn!(%error, "couldn't play a soundboard sound");
                    voice.play_fallback(use_);
                }
                Err(error) => {
                    tracing::warn!(%error, "a soundboard sound's download was cut short");
                    voice.play_fallback(use_);
                }
            });
        })
        .detach();
    }

    /// Plays a sound that arrived, if it is still wanted: the reader may have left, deafened
    /// or muted its player meanwhile.
    fn play_loaded(&mut self, sound: LocalSound, use_: SoundUse, cx: &mut Context<Self>) {
        match use_ {
            SoundUse::Preview => self.play_preview(sound, cx),
            SoundUse::Heard { player, .. } => {
                let is_mine = self
                    .session
                    .read(cx)
                    .state()
                    .current_user()
                    .is_some_and(|user| user.id == player);
                let is_wanted = !self.toggles.is_deafened()
                    && (is_mine || !Self::person_setting(player, cx).is_locally_muted);
                if let Some(engine) = self.engine().filter(|_| is_wanted) {
                    engine.play_soundboard_sound(sound, SoundboardPlayer::Person(player));
                }
            }
        }
    }

    fn play_fallback(&self, use_: SoundUse) {
        if let SoundUse::Heard {
            fallback: Some(cue),
            ..
        } = use_
        {
            self.play_call_cue(cue);
        }
    }
}
