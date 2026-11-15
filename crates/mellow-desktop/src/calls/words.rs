//! What the call interface says (`calls-view.ts`, DESIGN.md §5.6.3): a call's length, the line of
//! its message, who calls and who is being rung. The rules are client-core's
//! (`mellow_client_core::state::calls`).

use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::group_conversations::other_participants;
use mellow_client_core::shared_logic::voice_panel::VoiceProblem;
use mellow_client_core::state::CallMessageSummary;
use mellow_protocol::{
    Call, DirectMessageChannel, PublicUser, Snowflake, VoiceDisconnectReason, VoiceJoinRefusal,
};

use super::CallStartFailure;
use crate::home::conversation_display_name;
use crate::i18n::Strings;

const MINUTES_PER_HOUR: u64 = 60;
const MILLISECONDS_PER_MINUTE: u64 = 60_000;

/// Whole minutes of a call, as "Call · 12 min" counts them: under a minute reads 0.
#[must_use]
pub const fn call_duration_minutes(duration_milliseconds: u64) -> u64 {
    duration_milliseconds / MILLISECONDS_PER_MINUTE
}

/// "moins d'une minute", "12 min", "1 h 05".
#[must_use]
pub fn call_duration_label(duration_milliseconds: u64, strings: Strings) -> String {
    let minutes = call_duration_minutes(duration_milliseconds);
    let calls = strings.calls();
    if minutes == 0 {
        return calls.under_a_minute().to_owned();
    }
    let count = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
    if minutes < MINUTES_PER_HOUR {
        return calls.minutes(count(minutes));
    }
    calls.hours_and_minutes(
        count(minutes / MINUTES_PER_HOUR),
        &format!("{:02}", minutes % MINUTES_PER_HOUR),
    )
}

/// "Appel manqué", "Appel sans réponse", "Appel · 12 min", "Appel en cours".
#[must_use]
pub fn call_message_line(summary: CallMessageSummary, strings: Strings) -> String {
    let calls = strings.calls();
    match summary {
        CallMessageSummary::Ongoing => calls.ongoing().to_owned(),
        CallMessageSummary::Missed => calls.missed().to_owned(),
        CallMessageSummary::Unanswered => calls.unanswered().to_owned(),
        CallMessageSummary::Ended {
            duration_milliseconds,
        } => calls.ended(&call_duration_label(duration_milliseconds, strings)),
    }
}

/// Someone of the conversation, or a stand-in for an id it no longer lists.
#[must_use]
pub fn call_person_of(
    conversation: Option<&DirectMessageChannel>,
    user_id: Snowflake,
    strings: Strings,
) -> PublicUser {
    conversation
        .and_then(|conversation| {
            conversation
                .recipients
                .iter()
                .find(|recipient| recipient.id == user_id)
        })
        .cloned()
        .unwrap_or_else(|| {
            let someone = strings.voice().someone().to_owned();
            PublicUser {
                id: user_id,
                username: someone.clone(),
                display_name: someone,
                avatar: None,
                banner: None,
            }
        })
}

/// "Ada t'appelle", or "Ada t'appelle dans Atelier" for a group.
#[must_use]
pub fn incoming_call_sentence(
    call: &Call,
    conversation: &DirectMessageChannel,
    current_user_id: Option<Snowflake>,
    strings: Strings,
) -> String {
    let caller = call_person_of(Some(conversation), call.caller_id, strings);
    let caller = display_name(&caller);
    if conversation.owner_id.is_some() {
        strings.calls().calls_you_in(
            caller,
            &conversation_display_name(conversation, current_user_id, strings),
        )
    } else {
        strings.calls().calls_you(caller)
    }
}

/// The people being rung, named as a group's people are ("Ada, Bob et 2 autres").
#[must_use]
pub fn ringing_names(
    call: &Call,
    conversation: Option<&DirectMessageChannel>,
    strings: Strings,
) -> String {
    let people: Vec<PublicUser> = call
        .ringing_user_ids
        .iter()
        .map(|user_id| call_person_of(conversation, *user_id, strings))
        .collect();
    let names: Vec<&str> = people.iter().map(display_name).collect();
    strings.groups().participant_names(&names)
}

/// The faces a call shows for a conversation: the others in it.
#[must_use]
pub fn other_people(
    conversation: &DirectMessageChannel,
    current_user_id: Option<Snowflake>,
) -> Vec<PublicUser> {
    other_participants(conversation, current_user_id)
        .into_iter()
        .cloned()
        .collect()
}

/// A voice problem's card, in a call's own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallProblemLook {
    pub title: String,
    pub sentence: &'static str,
    pub is_notice: bool,
    /// "Rappeler": calling again may work.
    pub can_call_again: bool,
}

/// A voice problem in a conversation's call (`place` is its name), when a call says it otherwise
/// than a voice channel: nobody answered, the instance stopped, the conversation no longer open
/// to the reader. `None`: the voice channel's words fit.
#[must_use]
pub fn call_problem_look(
    problem: VoiceProblem,
    place: &str,
    strings: Strings,
) -> Option<CallProblemLook> {
    let calls = strings.calls();
    if let VoiceProblem::Refused {
        reason: VoiceJoinRefusal::MissingPermissions,
        ..
    } = problem
    {
        return Some(CallProblemLook {
            title: strings.voice().could_not_join(place),
            sentence: calls.cannot_join(),
            is_notice: false,
            can_call_again: false,
        });
    }
    let VoiceProblem::Disconnected { reason, .. } = problem else {
        return None;
    };
    match reason {
        VoiceDisconnectReason::CallEnded => Some(CallProblemLook {
            title: calls.no_answer_title().to_owned(),
            sentence: calls.no_answer(),
            is_notice: true,
            can_call_again: true,
        }),
        VoiceDisconnectReason::ServerStopping => Some(CallProblemLook {
            title: calls.interrupted_title().to_owned(),
            sentence: calls.interrupted(),
            is_notice: true,
            can_call_again: true,
        }),
        VoiceDisconnectReason::PermissionsChanged => Some(CallProblemLook {
            title: strings.voice().disconnected_from(place),
            sentence: calls.cannot_join(),
            is_notice: false,
            can_call_again: false,
        }),
        _ => None,
    }
}

/// Why a call could not start, in the voice panel's words.
#[must_use]
pub fn call_start_failure_sentence(failure: CallStartFailure, strings: Strings) -> &'static str {
    let failures = strings.calls().failures();
    match failure {
        CallStartFailure::AlreadyCalling => failures.already_calling(),
        CallStartFailure::RateLimited => failures.rate_limited(),
        CallStartFailure::Unreachable => failures.unreachable(),
        CallStartFailure::VoiceUnavailable => failures.voice_unavailable(),
        // Not said: the start already under way says how it went.
        CallStartFailure::Other | CallStartFailure::AlreadyStarting => failures.other(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn a_call_says_it_can_no_longer_be_joined_in_its_own_words() {
        let strings = Strings::new(Language::French);
        let refused = VoiceProblem::Refused {
            channel_id: Snowflake::from_raw(1),
            reason: VoiceJoinRefusal::MissingPermissions,
        };
        let look = call_problem_look(refused, "Ana", strings).unwrap();
        assert_eq!(look.sentence, "Tu ne peux plus participer à cet appel.");
        assert!(!look.can_call_again);
        let full = VoiceProblem::Refused {
            channel_id: Snowflake::from_raw(1),
            reason: VoiceJoinRefusal::ChannelFull,
        };
        assert!(call_problem_look(full, "Ana", strings).is_none());
    }

    #[test]
    fn a_call_reads_its_length_in_minutes_then_hours() {
        let strings = Strings::new(Language::French);
        assert_eq!(call_duration_label(59_000, strings), "moins d’une minute");
        assert_eq!(call_duration_label(720_000, strings), "12\u{a0}min");
        assert_eq!(call_duration_label(3_900_000, strings), "1\u{a0}h\u{a0}05");
        assert_eq!(
            call_message_line(
                CallMessageSummary::Ended {
                    duration_milliseconds: 720_000
                },
                strings
            ),
            "Appel · 12\u{a0}min"
        );
        assert_eq!(
            call_message_line(CallMessageSummary::Missed, strings),
            "Appel manqué"
        );
    }
}
