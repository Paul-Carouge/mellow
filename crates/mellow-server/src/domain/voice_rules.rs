//! Rules of voice channels (ADR-0030) that need no I/O: the shape of an SDP offer, room limits,
//! which receive slot carries which voice, and when someone counts as speaking. The media loop
//! and the voice service apply them; permissions themselves are decided in `domain::permissions`.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

use mellow_protocol::{
    MAXIMUM_SCREEN_SOUND_RECEIVE_SLOTS, MAXIMUM_VIDEO_RECEIVE_SLOTS,
    MAXIMUM_VOICE_CHANNEL_PARTICIPANTS, MAXIMUM_VOICE_OFFER_BYTES, MAXIMUM_VOICE_RECEIVE_SLOTS,
    MINIMUM_MILLISECONDS_BETWEEN_SOUNDS, Snowflake,
};

/// A receive slot goes to another speaker only once its current one has been silent this long.
pub(crate) const SILENCE_BEFORE_SLOT_REUSE: Duration = Duration::from_millis(300);
/// RFC 6464 audio level, in dBov (0 is the loudest, -127 silence): louder than this is a voice.
/// Browsers measure about -30 dBov for normal speech and below -60 for a quiet room.
pub(crate) const SPEAKING_AUDIO_LEVEL_THRESHOLD_DBOV: i8 = -50;
/// Someone stops speaking after this long without a voiced packet, so short pauses between
/// words do not flicker the indicator.
pub(crate) const SPEAKING_HANGOVER: Duration = Duration::from_millis(300);

/// Whether a member who last played a sound at `last_played_at` may play another one now
/// (ADR-0032).
pub(crate) fn may_play_another_sound(last_played_at: Option<Instant>, now: Instant) -> bool {
    last_played_at.is_none_or(|last| {
        now.saturating_duration_since(last)
            >= Duration::from_millis(MINIMUM_MILLISECONDS_BETWEEN_SOUNDS)
    })
}

/// Why an SDP offer is refused (`invalid_offer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum InvalidVoiceOffer {
    #[error("the offer is larger than {MAXIMUM_VOICE_OFFER_BYTES} bytes")]
    TooLarge,
    #[error("only audio and video m-lines are accepted")]
    UnexpectedMediaKind,
    #[error(
        "the offer needs one sendonly or sendrecv audio m-line, then 1 to {MAXIMUM_VOICE_RECEIVE_SLOTS} recvonly ones, then optionally one sendonly or sendrecv video m-line and 1 to {MAXIMUM_VIDEO_RECEIVE_SLOTS} recvonly ones, then optionally one sendonly or sendrecv audio m-line and 1 to {MAXIMUM_SCREEN_SOUND_RECEIVE_SLOTS} recvonly ones, each with its mid"
    )]
    UnexpectedMediaLines,
}

/// The m-lines of a valid offer: how many voices and cameras the participant can receive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VoiceOfferShape {
    pub(crate) audio_receive_slots: usize,
    /// 0 when the offer has no video m-lines: this participant can neither send nor watch
    /// cameras.
    pub(crate) video_receive_slots: usize,
    /// The audio m-lines after the video ones, for the sound of shared screens (ADR-0037).
    pub(crate) screen_sound: Option<ScreenSoundLines>,
}

/// The mids of the screen sound m-lines: they are audio like the voice ones, so only their place
/// in the offer tells them apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScreenSoundLines {
    /// The line the client sends its shared screen's sound on.
    pub(crate) send_mid: String,
    /// The lines it hears the screens it watches on.
    pub(crate) receive_mids: Vec<String>,
}

/// One media section of an offer: its kind, direction and mid.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Section {
    kind: SectionKind,
    direction: MediaDirection,
    mid: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediaDirection {
    SendReceive,
    SendOnly,
    ReceiveOnly,
    Inactive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionKind {
    Audio,
    Video,
}

fn media_direction(attribute_line: &str) -> Option<MediaDirection> {
    match attribute_line.trim_end() {
        "a=sendrecv" => Some(MediaDirection::SendReceive),
        "a=sendonly" => Some(MediaDirection::SendOnly),
        "a=recvonly" => Some(MediaDirection::ReceiveOnly),
        "a=inactive" => Some(MediaDirection::Inactive),
        _ => None,
    }
}

/// How many receive m-lines follow one send m-line of `kind` at the start of `sections`, if
/// they are within `1..=maximum`; `None` otherwise.
fn count_send_then_receive_lines(
    sections: &[Section],
    kind: SectionKind,
    maximum: usize,
) -> Option<usize> {
    let (first, rest) = sections.split_first()?;
    let is_sent = matches!(
        first.direction,
        MediaDirection::SendOnly | MediaDirection::SendReceive
    );
    let receive_lines = rest
        .iter()
        .take_while(|section| section.kind == kind)
        .collect::<Vec<_>>();
    let are_received_only = receive_lines
        .iter()
        .all(|section| section.direction == MediaDirection::ReceiveOnly);
    (first.kind == kind
        && is_sent
        && are_received_only
        && (1..=maximum).contains(&receive_lines.len()))
    .then_some(receive_lines.len())
}

/// Checks the offer's media sections before the WebRTC library parses it: one audio m-line the
/// client sends its voice on, then 1 to `MAXIMUM_VOICE_RECEIVE_SLOTS` audio m-lines it receives
/// others on (ADR-0030); then, from clients able to show video, one video m-line for its camera
/// and 1 to `MAXIMUM_VIDEO_RECEIVE_SLOTS` video m-lines it receives cameras on (ADR-0035); then,
/// optionally, one audio m-line for its shared screen's sound and 1 to
/// `MAXIMUM_SCREEN_SOUND_RECEIVE_SLOTS` it hears watched screens on (ADR-0037); and nothing else
/// (no data channel).
///
/// # Errors
/// `InvalidVoiceOffer` names what is wrong.
pub(crate) fn validate_voice_offer(sdp_offer: &str) -> Result<VoiceOfferShape, InvalidVoiceOffer> {
    if sdp_offer.len() > MAXIMUM_VOICE_OFFER_BYTES {
        return Err(InvalidVoiceOffer::TooLarge);
    }
    // The session-level direction, if any, is the default of every media section (RFC 4566).
    let mut session_direction = MediaDirection::SendReceive;
    let mut sections: Vec<Section> = Vec::new();
    for line in sdp_offer.lines() {
        if let Some(media_line) = line.strip_prefix("m=") {
            let kind = if media_line.starts_with("audio ") {
                SectionKind::Audio
            } else if media_line.starts_with("video ") {
                SectionKind::Video
            } else {
                return Err(InvalidVoiceOffer::UnexpectedMediaKind);
            };
            sections.push(Section {
                kind,
                direction: session_direction,
                mid: None,
            });
        } else if let Some(direction) = media_direction(line) {
            match sections.last_mut() {
                Some(section) => section.direction = direction,
                None => session_direction = direction,
            }
        } else if let Some(mid) = line.strip_prefix("a=mid:")
            && let Some(section) = sections.last_mut()
        {
            section.mid = Some(mid.trim_end().to_owned());
        }
    }
    let audio_receive_slots =
        count_send_then_receive_lines(&sections, SectionKind::Audio, MAXIMUM_VOICE_RECEIVE_SLOTS)
            .ok_or(InvalidVoiceOffer::UnexpectedMediaLines)?;
    let video_sections = sections.get(audio_receive_slots + 1..).unwrap_or_default();
    if video_sections.is_empty() {
        return Ok(VoiceOfferShape {
            audio_receive_slots,
            video_receive_slots: 0,
            screen_sound: None,
        });
    }
    let video_receive_slots = count_send_then_receive_lines(
        video_sections,
        SectionKind::Video,
        MAXIMUM_VIDEO_RECEIVE_SLOTS,
    )
    .ok_or(InvalidVoiceOffer::UnexpectedMediaLines)?;
    let screen_sound_sections = video_sections
        .get(video_receive_slots + 1..)
        .unwrap_or_default();
    let screen_sound = if screen_sound_sections.is_empty() {
        None
    } else {
        Some(screen_sound_lines(screen_sound_sections)?)
    };
    Ok(VoiceOfferShape {
        audio_receive_slots,
        video_receive_slots,
        screen_sound,
    })
}

/// The screen sound m-lines, which end the offer: one sent, then 1 to
/// `MAXIMUM_SCREEN_SOUND_RECEIVE_SLOTS` received, each with a mid.
fn screen_sound_lines(sections: &[Section]) -> Result<ScreenSoundLines, InvalidVoiceOffer> {
    let receive_slots = count_send_then_receive_lines(
        sections,
        SectionKind::Audio,
        MAXIMUM_SCREEN_SOUND_RECEIVE_SLOTS,
    )
    .filter(|receive_slots| receive_slots + 1 == sections.len())
    .ok_or(InvalidVoiceOffer::UnexpectedMediaLines)?;
    let mut mids = sections.iter().map(|section| section.mid.clone());
    let send_mid = mids
        .next()
        .flatten()
        .ok_or(InvalidVoiceOffer::UnexpectedMediaLines)?;
    let receive_mids = mids
        .collect::<Option<Vec<_>>>()
        .ok_or(InvalidVoiceOffer::UnexpectedMediaLines)?;
    debug_assert_eq!(receive_mids.len(), receive_slots);
    Ok(ScreenSoundLines {
        send_mid,
        receive_mids,
    })
}

/// How many people a voice channel holds: its user limit, never above the hard cap.
#[must_use]
pub(crate) fn voice_channel_capacity(user_limit: Option<u8>) -> usize {
    usize::from(
        user_limit.map_or(MAXIMUM_VOICE_CHANNEL_PARTICIPANTS, |limit| {
            limit.min(MAXIMUM_VOICE_CHANNEL_PARTICIPANTS)
        }),
    )
}

/// Whether someone may join a channel already holding `other_participants` people (the joiner
/// not counted, so moving inside a full channel or reconnecting is always possible).
#[must_use]
pub(crate) fn has_room_for_one_more(other_participants: usize, user_limit: Option<u8>) -> bool {
    other_participants < voice_channel_capacity(user_limit)
}

/// The voice channel as `domain::guilds::Guilds::voice_access` finds it for one user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VoiceAccess {
    /// The channel is gone, is not a voice channel, or is not in a guild.
    ChannelGone,
    /// The user is not a member of the channel's guild.
    NotMember,
    /// The user lacks `VIEW_CHANNEL` or `CONNECT` there.
    Denied,
    Allowed {
        guild_id: Snowflake,
        user_limit: Option<u8>,
        /// Holds `SPEAK`: their audio is forwarded.
        may_speak: bool,
        /// May play soundboard sounds there (`domain::permissions::may_use_soundboard`).
        may_use_soundboard: bool,
        /// May turn their camera on there (`domain::permissions::may_send_video`).
        may_send_video: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AssignedSlot<Slot> {
    slot: Slot,
    last_packet_at: Instant,
}

/// One listener's receive m-lines and the speakers they carry (ADR-0030). A speaker keeps their
/// slot while they talk; once every slot is taken, a new speaker takes the slot of the one silent
/// the longest, if silent for more than `SILENCE_BEFORE_SLOT_REUSE`, otherwise is not heard. This
/// approximates "the 16 loudest voices" with "the 16 most recent", which needs no decoding.
#[derive(Debug, Clone)]
pub(crate) struct ReceiveSlots<Slot, Speaker> {
    free: Vec<Slot>,
    assigned: HashMap<Speaker, AssignedSlot<Slot>>,
}

impl<Slot, Speaker> Default for ReceiveSlots<Slot, Speaker> {
    fn default() -> Self {
        Self {
            free: Vec::new(),
            assigned: HashMap::new(),
        }
    }
}

impl<Slot: Copy, Speaker: Copy + Eq + Hash> ReceiveSlots<Slot, Speaker> {
    pub(crate) fn add_free_slot(&mut self, slot: Slot) {
        self.free.push(slot);
    }

    /// The slot to write `speaker`'s packet arriving at `now` on, or `None` when every slot
    /// carries someone who spoke within `SILENCE_BEFORE_SLOT_REUSE`.
    pub(crate) fn slot_for(&mut self, speaker: Speaker, now: Instant) -> Option<Slot> {
        if let Some(assigned) = self.assigned.get_mut(&speaker) {
            assigned.last_packet_at = now;
            return Some(assigned.slot);
        }
        let slot = if let Some(slot) = self.free.pop() {
            slot
        } else {
            let (silent_speaker, silent) = self
                .assigned
                .iter()
                .filter(|(_, assigned)| {
                    now.saturating_duration_since(assigned.last_packet_at)
                        > SILENCE_BEFORE_SLOT_REUSE
                })
                .min_by_key(|(_, assigned)| assigned.last_packet_at)
                .map(|(speaker, assigned)| (*speaker, *assigned))?;
            self.assigned.remove(&silent_speaker);
            silent.slot
        };
        self.assigned.insert(
            speaker,
            AssignedSlot {
                slot,
                last_packet_at: now,
            },
        );
        Some(slot)
    }

    /// Whether `speaker` already has a slot, so a new assignment can be announced.
    pub(crate) fn has_slot(&self, speaker: Speaker) -> bool {
        self.assigned.contains_key(&speaker)
    }

    /// Frees the slot of a speaker who left.
    pub(crate) fn release(&mut self, speaker: Speaker) {
        if let Some(assigned) = self.assigned.remove(&speaker) {
            self.free.push(assigned.slot);
        }
    }
}

/// Turns the audio level of someone's packets into "started" and "stopped speaking" edges.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SpeakingDetector {
    is_speaking: bool,
    last_voiced_at: Option<Instant>,
}

impl SpeakingDetector {
    /// A packet arrived at `now` with this audio level. A packet without the level counts as
    /// voiced: with DTX, a client sends packets mostly while it hears a voice. Returns `Some(true)`
    /// when this starts a speech.
    pub(crate) fn on_packet(&mut self, audio_level_dbov: Option<i8>, now: Instant) -> Option<bool> {
        let is_voiced =
            audio_level_dbov.is_none_or(|level| level > SPEAKING_AUDIO_LEVEL_THRESHOLD_DBOV);
        if !is_voiced {
            return None;
        }
        self.last_voiced_at = Some(now);
        (!std::mem::replace(&mut self.is_speaking, true)).then_some(true)
    }

    /// Time passed: returns `Some(false)` once `SPEAKING_HANGOVER` went by without a voiced packet.
    pub(crate) fn on_tick(&mut self, now: Instant) -> Option<bool> {
        let is_quiet = self
            .last_voiced_at
            .is_none_or(|last| now.saturating_duration_since(last) >= SPEAKING_HANGOVER);
        if self.is_speaking && is_quiet {
            self.is_speaking = false;
            return Some(false);
        }
        None
    }

    /// The person was muted or suppressed: returns `Some(false)` if they were speaking.
    pub(crate) fn stop(&mut self) -> Option<bool> {
        self.last_voiced_at = None;
        std::mem::replace(&mut self.is_speaking, false).then_some(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_are_two_seconds_apart_at_least() {
        let start = Instant::now();
        assert!(may_play_another_sound(None, start));
        assert!(!may_play_another_sound(Some(start), start));
        assert!(!may_play_another_sound(
            Some(start),
            start + Duration::from_millis(1_999)
        ));
        assert!(may_play_another_sound(
            Some(start),
            start + Duration::from_secs(2)
        ));
    }

    fn offer(media: &[(&str, &str)]) -> String {
        use std::fmt::Write as _;
        let mut sdp = String::from("v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n");
        for (index, (kind, direction)) in media.iter().enumerate() {
            write!(
                sdp,
                "m={kind} 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:{index}\r\na={direction}\r\n"
            )
            .unwrap();
        }
        sdp
    }

    fn audio_only(audio_receive_slots: usize) -> VoiceOfferShape {
        VoiceOfferShape {
            audio_receive_slots,
            video_receive_slots: 0,
            screen_sound: None,
        }
    }

    #[test]
    fn offers_need_one_voice_line_then_1_to_16_receive_lines_of_audio() {
        let mut lines = vec![("audio", "sendonly"), ("audio", "recvonly")];
        assert_eq!(validate_voice_offer(&offer(&lines)), Ok(audio_only(1)));
        lines[0].1 = "sendrecv";
        lines.extend(std::iter::repeat_n(("audio", "recvonly"), 15));
        assert_eq!(validate_voice_offer(&offer(&lines)), Ok(audio_only(16)));
        lines.push(("audio", "recvonly"));
        assert_eq!(
            validate_voice_offer(&offer(&lines)),
            Err(InvalidVoiceOffer::UnexpectedMediaLines),
            "17 receive slots"
        );
        for refused in [
            vec![("audio", "sendonly")],
            vec![("audio", "recvonly"), ("audio", "recvonly")],
            vec![("audio", "sendonly"), ("audio", "sendonly")],
            vec![("audio", "inactive"), ("audio", "recvonly")],
            vec![],
        ] {
            assert_eq!(
                validate_voice_offer(&offer(&refused)),
                Err(InvalidVoiceOffer::UnexpectedMediaLines),
                "{refused:?}"
            );
        }
        assert_eq!(
            validate_voice_offer(&offer(&[
                ("audio", "sendonly"),
                ("application", "recvonly")
            ])),
            Err(InvalidVoiceOffer::UnexpectedMediaKind)
        );
    }

    #[test]
    fn video_lines_follow_the_audio_ones_one_camera_then_1_to_9_receive_lines() {
        let audio = [("audio", "sendonly"), ("audio", "recvonly")];
        let with_video = |video: &[(&'static str, &'static str)]| {
            validate_voice_offer(&offer(&[&audio[..], video].concat()))
        };
        let camera_and_watched = |watched: usize| {
            let mut video = vec![("video", "sendonly")];
            video.extend(std::iter::repeat_n(("video", "recvonly"), watched));
            video
        };
        assert_eq!(
            with_video(&camera_and_watched(1)),
            Ok(VoiceOfferShape {
                audio_receive_slots: 1,
                video_receive_slots: 1,
                screen_sound: None,
            })
        );
        assert_eq!(
            with_video(&camera_and_watched(9)),
            Ok(VoiceOfferShape {
                audio_receive_slots: 1,
                video_receive_slots: 9,
                screen_sound: None,
            })
        );
        assert_eq!(
            with_video(&[("video", "sendrecv"), ("video", "recvonly")])
                .map(|shape| shape.video_receive_slots),
            Ok(1)
        );
        for refused in [
            camera_and_watched(10),
            camera_and_watched(0),
            vec![("video", "recvonly"), ("video", "recvonly")],
            vec![("video", "sendonly"), ("video", "sendonly")],
            vec![
                ("video", "sendonly"),
                ("video", "recvonly"),
                ("audio", "recvonly"),
            ],
            vec![("video", "sendonly"), ("audio", "recvonly")],
            vec![
                ("video", "sendonly"),
                ("audio", "sendonly"),
                ("audio", "recvonly"),
            ],
        ] {
            assert_eq!(
                with_video(&refused),
                Err(InvalidVoiceOffer::UnexpectedMediaLines),
                "{refused:?}"
            );
        }
        assert_eq!(
            validate_voice_offer(&offer(&[
                ("video", "sendonly"),
                ("video", "recvonly"),
                ("audio", "sendonly"),
                ("audio", "recvonly")
            ])),
            Err(InvalidVoiceOffer::UnexpectedMediaLines),
            "audio comes first"
        );
    }

    #[test]
    fn screen_sound_lines_end_the_offer_one_sent_then_1_to_3_received() {
        let mut lines = vec![
            ("audio", "sendonly"),
            ("audio", "recvonly"),
            ("video", "sendonly"),
            ("video", "recvonly"),
            ("audio", "sendonly"),
            ("audio", "recvonly"),
        ];
        assert_eq!(
            validate_voice_offer(&offer(&lines)),
            Ok(VoiceOfferShape {
                audio_receive_slots: 1,
                video_receive_slots: 1,
                screen_sound: Some(ScreenSoundLines {
                    send_mid: "4".into(),
                    receive_mids: vec!["5".into()],
                }),
            })
        );
        lines.extend([("audio", "recvonly"), ("audio", "recvonly")]);
        assert_eq!(
            validate_voice_offer(&offer(&lines))
                .map(|shape| shape.screen_sound.map(|sound| sound.receive_mids.len())),
            Ok(Some(3))
        );
        for refused in [
            [&lines[..], &[("audio", "recvonly")]].concat(),
            [&lines[..], &[("video", "recvonly")]].concat(),
            lines[..5].to_vec(),
        ] {
            assert_eq!(
                validate_voice_offer(&offer(&refused)),
                Err(InvalidVoiceOffer::UnexpectedMediaLines),
                "{refused:?}"
            );
        }
        let without_mid = offer(&lines[..6]).replace("a=mid:4\r\n", "");
        assert_eq!(
            validate_voice_offer(&without_mid),
            Err(InvalidVoiceOffer::UnexpectedMediaLines)
        );
    }

    #[test]
    fn a_section_without_direction_takes_the_session_default_and_huge_offers_are_refused() {
        let without_direction = "v=0\r\na=recvonly\r\nm=audio 9 RTP 111\r\na=sendonly\r\n\
                                 m=audio 9 RTP 111\r\n";
        assert_eq!(validate_voice_offer(without_direction), Ok(audio_only(1)));
        let sendrecv_by_default = "v=0\r\nm=audio 9 RTP 111\r\nm=audio 9 RTP 111\r\n";
        assert_eq!(
            validate_voice_offer(sendrecv_by_default),
            Err(InvalidVoiceOffer::UnexpectedMediaLines)
        );
        let mut huge = offer(&[("audio", "sendonly"), ("audio", "recvonly")]);
        huge.push_str(&"a=x\r\n".repeat(MAXIMUM_VOICE_OFFER_BYTES / 5));
        assert_eq!(
            validate_voice_offer(&huge),
            Err(InvalidVoiceOffer::TooLarge)
        );
    }

    #[test]
    fn channels_hold_their_limit_capped_at_25_and_the_joiner_is_not_counted() {
        assert_eq!(voice_channel_capacity(None), 25);
        assert_eq!(voice_channel_capacity(Some(3)), 3);
        assert_eq!(voice_channel_capacity(Some(200)), 25);
        assert!(has_room_for_one_more(2, Some(3)));
        assert!(!has_room_for_one_more(3, Some(3)));
        assert!(has_room_for_one_more(24, None));
        assert!(!has_room_for_one_more(25, None));
    }

    #[test]
    fn speakers_keep_their_slot_and_take_the_longest_silent_one_when_all_are_busy() {
        let start = Instant::now();
        let at = |milliseconds| start + Duration::from_millis(milliseconds);
        let mut slots: ReceiveSlots<u8, char> = ReceiveSlots::default();
        slots.add_free_slot(1);
        slots.add_free_slot(2);

        assert!(!slots.has_slot('a'));
        let a = slots.slot_for('a', at(0)).unwrap();
        assert!(slots.has_slot('a'), "a new assignment is visible");
        let b = slots.slot_for('b', at(10)).unwrap();
        assert_ne!(a, b);
        assert_eq!(slots.slot_for('a', at(20)), Some(a), "a keeps its slot");
        assert_eq!(
            slots.slot_for('c', at(200)),
            None,
            "both spoke within 300 ms"
        );
        // a spoke last at 20 ms, b at 10 ms: at 400 ms both are silent, b the longest.
        assert_eq!(slots.slot_for('c', at(400)), Some(b));
        assert_eq!(
            slots.slot_for('a', at(401)),
            Some(a),
            "a still holds its slot"
        );
        assert_eq!(slots.slot_for('b', at(402)), None, "a and c are recent");
        slots.release('a');
        assert_eq!(
            slots.slot_for('b', at(403)),
            Some(a),
            "a left: its slot is free"
        );
    }

    #[test]
    fn speaking_starts_on_a_voiced_packet_and_stops_after_the_hangover() {
        let start = Instant::now();
        let at = |milliseconds| start + Duration::from_millis(milliseconds);
        let mut detector = SpeakingDetector::default();

        assert_eq!(
            detector.on_packet(Some(-90), at(0)),
            None,
            "background noise"
        );
        assert_eq!(detector.on_tick(at(10)), None);
        assert_eq!(detector.on_packet(Some(-30), at(20)), Some(true));
        assert_eq!(
            detector.on_packet(Some(-25), at(40)),
            None,
            "already speaking"
        );
        assert_eq!(detector.on_packet(Some(-90), at(60)), None);
        assert_eq!(detector.on_tick(at(339)), None, "a pause between words");
        assert_eq!(detector.on_tick(at(340)), Some(false));
        assert_eq!(detector.on_tick(at(1000)), None, "stopped once");
        assert_eq!(detector.on_packet(None, at(1100)), Some(true), "no level");
        assert_eq!(detector.stop(), Some(false), "muted");
        assert_eq!(detector.stop(), None);
    }
}
