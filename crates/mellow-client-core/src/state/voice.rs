//! Who is in which voice channel (ADR-0030), who speaks, and the soundboard's indicators and
//! cooldown (ADR-0032). The voice connection itself is the voice session's, not this state's.

use mellow_protocol::{MINIMUM_MILLISECONDS_BETWEEN_SOUNDS, Snowflake, VoiceState};

use super::{ClientState, StateChange};

/// A sound's emoji shows by its player's name at least this long, even for a shorter sound.
pub const MINIMUM_SOUND_INDICATOR_MILLISECONDS: u64 = 1_500;

/// The sound a person played last, while its emoji shows by their name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundIndicator {
    pub sound_id: Snowflake,
    pub guild_id: Snowflake,
    pub shown_until_unix_milliseconds: u64,
}

impl ClientState {
    /// Who is connected to a voice channel, in the order they came.
    pub fn voice_participants(&self, channel_id: Snowflake) -> impl Iterator<Item = &VoiceState> {
        self.voice_states
            .iter()
            .filter(move |state| state.channel_id == channel_id)
    }

    #[must_use]
    pub fn voice_state_of(&self, user_id: Snowflake) -> Option<&VoiceState> {
        self.voice_states
            .iter()
            .find(|state| state.user_id == user_id)
    }

    #[must_use]
    pub fn is_speaking(&self, user_id: Snowflake) -> bool {
        self.speaking_user_ids.contains(&user_id)
    }

    /// `VOICE_STATE_UPDATE`: a join, a move, or a change of mute, deafen, camera or screen.
    pub(super) fn apply_voice_state(&mut self, state: VoiceState) -> StateChange {
        let previous_channel_id = self
            .voice_state_of(state.user_id)
            .map(|previous| previous.channel_id);
        if !self.is_known_channel(state.channel_id) {
            // Gone where this reader cannot see: left, as far as they know.
            return match previous_channel_id {
                Some(previous_channel_id) => {
                    self.drop_voice_state(state.user_id, previous_channel_id)
                }
                None => StateChange::Nothing,
            };
        }
        let change = StateChange::VoiceParticipant {
            guild_id: state.guild_id,
            user_id: state.user_id,
            previous_channel_id,
            channel_id: Some(state.channel_id),
        };
        if previous_channel_id == Some(state.channel_id) {
            if let Some(existing) = self
                .voice_states
                .iter_mut()
                .find(|existing| existing.user_id == state.user_id)
            {
                *existing = state;
            }
            return change;
        }
        // A move puts them last in their new channel, in join order.
        self.voice_states
            .retain(|existing| existing.user_id != state.user_id);
        self.speaking_user_ids.remove(&state.user_id);
        self.voice_states.push(state);
        change
    }

    /// `VOICE_STATE_DELETE`. A late delete for a channel they already moved out of changes
    /// nothing.
    pub(super) fn drop_voice_state(
        &mut self,
        user_id: Snowflake,
        channel_id: Snowflake,
    ) -> StateChange {
        let Some(index) = self
            .voice_states
            .iter()
            .position(|state| state.user_id == user_id && state.channel_id == channel_id)
        else {
            return StateChange::Nothing;
        };
        let previous = self.voice_states.remove(index);
        self.speaking_user_ids.remove(&user_id);
        StateChange::VoiceParticipant {
            guild_id: previous.guild_id,
            user_id,
            previous_channel_id: Some(channel_id),
            channel_id: None,
        }
    }

    /// `SPEAKING`: only for someone connected to that channel.
    pub(super) fn note_speaking(
        &mut self,
        channel_id: Snowflake,
        user_id: Snowflake,
        is_speaking: bool,
    ) -> StateChange {
        let is_changed = if !is_speaking {
            self.speaking_user_ids.remove(&user_id)
        } else if self
            .voice_state_of(user_id)
            .is_some_and(|state| state.channel_id == channel_id)
        {
            self.speaking_user_ids.insert(user_id)
        } else {
            false
        };
        if is_changed {
            StateChange::Speaking {
                channel_id,
                user_id,
            }
        } else {
            StateChange::Nothing
        }
    }

    /// The sound `user_id` played last, while its emoji shows.
    #[must_use]
    pub fn sound_indicator(
        &self,
        user_id: Snowflake,
        now_unix_milliseconds: u64,
    ) -> Option<&SoundIndicator> {
        self.sound_indicator_by_user_id
            .get(&user_id)
            .filter(|indicator| indicator.shown_until_unix_milliseconds > now_unix_milliseconds)
    }

    /// The user's soundboard waits until then between two sounds.
    #[must_use]
    pub fn sound_cooldown_ends_at_unix_milliseconds(&self) -> u64 {
        self.sound_cooldown_ends_at_unix_milliseconds
    }

    /// The user sent `voice_play_sound`: their soundboard waits
    /// `MINIMUM_MILLISECONDS_BETWEEN_SOUNDS` from now.
    pub fn note_own_sound_sent(&mut self, now_unix_milliseconds: u64) {
        self.extend_sound_cooldown(now_unix_milliseconds);
    }

    fn extend_sound_cooldown(&mut self, played_at_unix_milliseconds: u64) {
        self.sound_cooldown_ends_at_unix_milliseconds = self
            .sound_cooldown_ends_at_unix_milliseconds
            .max(played_at_unix_milliseconds.saturating_add(MINIMUM_MILLISECONDS_BETWEEN_SOUNDS));
    }

    /// Shows a sound's emoji by `user_id`'s name for the sound's length (at least
    /// `MINIMUM_SOUND_INDICATOR_MILLISECONDS`): a played sound, or a join sound the app plays.
    pub fn show_sound_indicator(
        &mut self,
        user_id: Snowflake,
        guild_id: Snowflake,
        sound_id: Snowflake,
        now_unix_milliseconds: u64,
    ) -> StateChange {
        let duration = self
            .sounds_of(guild_id)
            .iter()
            .find(|sound| sound.id == sound_id)
            .map_or(0, |sound| u64::from(sound.duration_milliseconds));
        // Expired ones go here, so the map never outgrows who played lately.
        self.sound_indicator_by_user_id
            .retain(|_, indicator| indicator.shown_until_unix_milliseconds > now_unix_milliseconds);
        self.sound_indicator_by_user_id.insert(
            user_id,
            SoundIndicator {
                sound_id,
                guild_id,
                shown_until_unix_milliseconds: now_unix_milliseconds
                    .saturating_add(duration.max(MINIMUM_SOUND_INDICATOR_MILLISECONDS)),
            },
        );
        StateChange::SoundIndicator { user_id }
    }

    /// `VOICE_SOUND_PLAYED`: counted only in the channel the user is connected to.
    pub(super) fn note_sound_played(
        &mut self,
        channel_id: Snowflake,
        user_id: Snowflake,
        sound_id: Snowflake,
        now_unix_milliseconds: u64,
    ) -> StateChange {
        let Some(me) = self.current_user_id() else {
            return StateChange::Nothing;
        };
        let Some(guild_id) = self
            .voice_state_of(me)
            .filter(|own| own.channel_id == channel_id)
            .map(|own| own.guild_id)
        else {
            return StateChange::Nothing;
        };
        if user_id == me {
            self.extend_sound_cooldown(now_unix_milliseconds);
        }
        self.show_sound_indicator(user_id, guild_id, sound_id, now_unix_milliseconds);
        StateChange::SoundPlayed {
            channel_id,
            user_id,
            sound_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{ChannelKind, GatewayEvent, SoundboardSound};

    use super::super::tests::{CATEGORY, GUILD, ME, NOW, OTHER, channel, ready_state};
    use super::*;

    const LOUNGE: Snowflake = Snowflake::from_raw(104);
    const STAGE: Snowflake = Snowflake::from_raw(105);
    const HIDDEN: Snowflake = Snowflake::from_raw(106);
    const THIRD: Snowflake = Snowflake::from_raw(3);
    const HORN: Snowflake = Snowflake::from_raw(900);

    fn voice_state(user_id: Snowflake, channel_id: Snowflake) -> VoiceState {
        VoiceState {
            user_id,
            guild_id: GUILD,
            channel_id,
            self_mute: false,
            self_deaf: false,
            suppressed: false,
            join_sound_id: None,
            self_video: false,
            self_stream: false,
        }
    }

    fn update(state: VoiceState) -> GatewayEvent {
        GatewayEvent::VoiceStateUpdate { voice_state: state }
    }

    fn voice_ready_state() -> ClientState {
        let mut state = ready_state();
        for id in [LOUNGE, STAGE] {
            state.apply_event(
                GatewayEvent::ChannelCreate(channel(id, ChannelKind::Voice, Some(CATEGORY), 3)),
                NOW,
            );
        }
        state.add_created_sound(
            GUILD,
            SoundboardSound {
                id: HORN,
                name: "horn".into(),
                emoji: None,
                duration_milliseconds: 4_000,
            },
        );
        state
    }

    fn participants(state: &ClientState, channel_id: Snowflake) -> Vec<Snowflake> {
        state
            .voice_participants(channel_id)
            .map(|participant| participant.user_id)
            .collect()
    }

    #[test]
    fn participants_keep_join_order_and_a_move_puts_them_last() {
        let mut state = voice_ready_state();
        for user_id in [OTHER, THIRD, ME] {
            state.apply_event(update(voice_state(user_id, LOUNGE)), NOW);
        }
        assert_eq!(participants(&state, LOUNGE), [OTHER, THIRD, ME]);
        let mut muted = voice_state(OTHER, LOUNGE);
        muted.self_mute = true;
        state.apply_event(update(muted), NOW);
        assert_eq!(participants(&state, LOUNGE), [OTHER, THIRD, ME], "in place");
        assert!(state.voice_state_of(OTHER).unwrap().self_mute);

        state.apply_event(
            GatewayEvent::Speaking {
                guild_id: GUILD,
                channel_id: LOUNGE,
                user_id: OTHER,
                speaking: true,
            },
            NOW,
        );
        assert!(state.is_speaking(OTHER));
        let moved = state.apply_event(update(voice_state(OTHER, STAGE)), NOW);
        assert_eq!(
            moved,
            StateChange::VoiceParticipant {
                guild_id: GUILD,
                user_id: OTHER,
                previous_channel_id: Some(LOUNGE),
                channel_id: Some(STAGE),
            }
        );
        assert!(!state.is_speaking(OTHER), "a move ends their speaking ring");
        state.apply_event(update(voice_state(OTHER, LOUNGE)), NOW);
        assert_eq!(participants(&state, LOUNGE), [THIRD, ME, OTHER]);
    }

    #[test]
    fn a_late_delete_or_a_move_out_of_sight_is_handled_as_the_web_does() {
        let mut state = voice_ready_state();
        state.apply_event(update(voice_state(OTHER, LOUNGE)), NOW);
        state.apply_event(update(voice_state(OTHER, STAGE)), NOW);
        let late = GatewayEvent::VoiceStateDelete {
            guild_id: GUILD,
            channel_id: LOUNGE,
            user_id: OTHER,
        };
        assert_eq!(state.apply_event(late, NOW), StateChange::Nothing);
        assert_eq!(participants(&state, STAGE), [OTHER]);

        state.apply_event(update(voice_state(OTHER, HIDDEN)), NOW);
        assert!(
            state.voice_state_of(OTHER).is_none(),
            "gone where this reader cannot see"
        );
        state.apply_event(
            GatewayEvent::Speaking {
                guild_id: GUILD,
                channel_id: LOUNGE,
                user_id: OTHER,
                speaking: true,
            },
            NOW,
        );
        assert!(!state.is_speaking(OTHER), "not connected there");
    }

    #[test]
    fn a_deleted_channel_takes_its_participants_along() {
        let mut state = voice_ready_state();
        state.apply_event(update(voice_state(OTHER, LOUNGE)), NOW);
        state.apply_event(
            GatewayEvent::ChannelDelete {
                guild_id: GUILD,
                id: LOUNGE,
            },
            NOW,
        );
        assert!(state.voice_state_of(OTHER).is_none());
    }

    #[test]
    fn played_sounds_show_by_their_player_and_my_own_start_my_cooldown() {
        let mut state = voice_ready_state();
        let played = |user_id| GatewayEvent::VoiceSoundPlayed {
            channel_id: LOUNGE,
            user_id,
            sound_id: HORN,
        };
        assert_eq!(
            state.apply_event(played(OTHER), NOW),
            StateChange::Nothing,
            "heard only where I am connected"
        );
        state.apply_event(update(voice_state(ME, LOUNGE)), NOW);
        assert_eq!(
            state.apply_event(played(OTHER), NOW),
            StateChange::SoundPlayed {
                channel_id: LOUNGE,
                user_id: OTHER,
                sound_id: HORN,
            }
        );
        let indicator = *state.sound_indicator(OTHER, NOW).unwrap();
        assert_eq!(indicator.shown_until_unix_milliseconds, NOW + 4_000);
        assert!(state.sound_indicator(OTHER, NOW + 4_000).is_none());
        assert_eq!(state.sound_cooldown_ends_at_unix_milliseconds(), 0);

        state.apply_event(played(ME), NOW);
        assert_eq!(
            state.sound_cooldown_ends_at_unix_milliseconds(),
            NOW + MINIMUM_MILLISECONDS_BETWEEN_SOUNDS
        );
        state.note_own_sound_sent(NOW - 1_000);
        assert_eq!(
            state.sound_cooldown_ends_at_unix_milliseconds(),
            NOW + MINIMUM_MILLISECONDS_BETWEEN_SOUNDS,
            "an older play never shortens it"
        );
    }

    #[test]
    fn a_short_or_unknown_sound_shows_for_the_minimum() {
        let mut state = voice_ready_state();
        state.show_sound_indicator(OTHER, GUILD, Snowflake::from_raw(1), NOW);
        assert_eq!(
            state
                .sound_indicator(OTHER, NOW)
                .unwrap()
                .shown_until_unix_milliseconds,
            NOW + MINIMUM_SOUND_INDICATOR_MILLISECONDS
        );
    }
}
