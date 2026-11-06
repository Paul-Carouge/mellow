//! Port of the soundboard rules of `web/src/lib/soundboard.ts` (ADR-0032): when the soundboard
//! may play, who hears a played sound and what plays when someone joins. The cooldown clock is
//! in the voice state; playing sounds is the media engine's.

use mellow_protocol::{Permissions, Snowflake, SoundboardSound};

/// Why the soundboard cannot play now; the instance would silently ignore the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundboardBlock {
    NotConnected,
    NoPermission,
    CannotSpeak,
    Deafened,
    CoolingDown,
}

/// What decides whether the soundboard can play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundboardFacts {
    /// Voice is connected (not joining or reconnecting).
    pub is_connected: bool,
    /// Your permissions in the connected channel; `None` while unknown (the instance decides).
    pub channel_permissions: Option<Permissions>,
    pub is_deafened: bool,
    pub now_unix_milliseconds: u64,
    pub cooldown_ends_at_unix_milliseconds: u64,
}

/// The first reason in the order a person can act on it; self-mute does not block (ADR-0032).
#[must_use]
pub fn soundboard_block(facts: SoundboardFacts) -> Option<SoundboardBlock> {
    if !facts.is_connected {
        return Some(SoundboardBlock::NotConnected);
    }
    if let Some(permissions) = facts.channel_permissions {
        if !permissions.contains(Permissions::USE_SOUNDBOARD) {
            return Some(SoundboardBlock::NoPermission);
        }
        if !permissions.contains(Permissions::SPEAK) {
            return Some(SoundboardBlock::CannotSpeak);
        }
    }
    if facts.is_deafened {
        return Some(SoundboardBlock::Deafened);
    }
    (facts.now_unix_milliseconds < facts.cooldown_ends_at_unix_milliseconds)
        .then_some(SoundboardBlock::CoolingDown)
}

/// A played sound is heard in your channel only, never deafened, never from someone muted for
/// you.
#[must_use]
pub fn should_hear_sound(
    sound_channel_id: Snowflake,
    my_channel_id: Option<Snowflake>,
    is_connected: bool,
    is_deafened: bool,
    is_player_muted_for_me: bool,
) -> bool {
    is_connected
        && my_channel_id == Some(sound_channel_id)
        && !is_deafened
        && !is_player_muted_for_me
}

/// What plays when someone comes into your channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinCue {
    /// Their join sound, still on the instance.
    Sound(Snowflake),
    /// The usual peer-join cue.
    Chime,
}

/// Their join sound when the instance still has it and you would hear it, else the cue
/// (deafened, muted for you, or no sound chosen).
#[must_use]
pub fn join_cue_of(
    join_sound_id: Option<Snowflake>,
    sounds: &[SoundboardSound],
    is_deafened: bool,
    is_muted_for_me: bool,
) -> JoinCue {
    match join_sound_id {
        Some(sound_id)
            if !is_deafened
                && !is_muted_for_me
                && sounds.iter().any(|sound| sound.id == sound_id) =>
        {
            JoinCue::Sound(sound_id)
        }
        _ => JoinCue::Chime,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> SoundboardFacts {
        SoundboardFacts {
            is_connected: true,
            channel_permissions: Some(Permissions::USE_SOUNDBOARD | Permissions::SPEAK),
            is_deafened: false,
            now_unix_milliseconds: 10_000,
            cooldown_ends_at_unix_milliseconds: 0,
        }
    }

    #[test]
    fn the_soundboard_says_the_first_reason_it_cannot_play() {
        assert_eq!(soundboard_block(facts()), None);
        assert_eq!(
            soundboard_block(SoundboardFacts {
                channel_permissions: None,
                ..facts()
            }),
            None,
            "unknown permissions: the instance decides"
        );
        let cases = [
            (
                SoundboardFacts {
                    is_connected: false,
                    is_deafened: true,
                    ..facts()
                },
                SoundboardBlock::NotConnected,
            ),
            (
                SoundboardFacts {
                    channel_permissions: Some(Permissions::SPEAK),
                    ..facts()
                },
                SoundboardBlock::NoPermission,
            ),
            (
                SoundboardFacts {
                    channel_permissions: Some(Permissions::USE_SOUNDBOARD),
                    ..facts()
                },
                SoundboardBlock::CannotSpeak,
            ),
            (
                SoundboardFacts {
                    is_deafened: true,
                    ..facts()
                },
                SoundboardBlock::Deafened,
            ),
            (
                SoundboardFacts {
                    cooldown_ends_at_unix_milliseconds: 10_001,
                    ..facts()
                },
                SoundboardBlock::CoolingDown,
            ),
        ];
        for (facts, block) in cases {
            assert_eq!(soundboard_block(facts), Some(block));
        }
    }

    #[test]
    fn sounds_are_heard_in_your_channel_only_and_never_deafened_or_from_the_muted() {
        let here = Snowflake::from_raw(1);
        assert!(should_hear_sound(here, Some(here), true, false, false));
        assert!(!should_hear_sound(
            here,
            Some(Snowflake::from_raw(2)),
            true,
            false,
            false
        ));
        assert!(!should_hear_sound(here, Some(here), false, false, false));
        assert!(!should_hear_sound(here, Some(here), true, true, false));
        assert!(!should_hear_sound(here, Some(here), true, false, true));
    }

    #[test]
    fn a_join_sound_replaces_the_cue_only_when_it_exists_and_would_be_heard() {
        let sound = SoundboardSound {
            id: Snowflake::from_raw(7),
            name: "horn".into(),
            emoji: None,
            duration_milliseconds: 1_000,
        };
        let sounds = [sound];
        let id = Some(Snowflake::from_raw(7));
        assert_eq!(
            join_cue_of(id, &sounds, false, false),
            JoinCue::Sound(Snowflake::from_raw(7))
        );
        assert_eq!(join_cue_of(id, &[], false, false), JoinCue::Chime);
        assert_eq!(join_cue_of(id, &sounds, true, false), JoinCue::Chime);
        assert_eq!(join_cue_of(id, &sounds, false, true), JoinCue::Chime);
        assert_eq!(join_cue_of(None, &sounds, false, false), JoinCue::Chime);
    }
}
