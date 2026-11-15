//! The voice panel (`VoicePanel.svelte`, `voice-panel.ts`, DESIGN.md §5.3, ADR-0030): a card
//! like the user card, directly above it. While in a call: the state with its signal bars, the
//! channel and server (a link to the server), and Disconnect; after a problem: what happened,
//! Try again when it may pass, and Close. Its announcements (`VoiceAnnouncements.svelte`) always
//! exist, so their first change is read out. Also the user card's microphone and headset
//! toggles, and a participant's volume and "Mute for me" popover.

use gpui::{
    AnyElement, App, Context, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, PathBuilder, Pixels, Point, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Toggled, Window, anchored, canvas, deferred, div,
    point, prelude::FluentBuilder as _, px,
};
use mellow_client_core::shared_logic::voice_panel::{
    LatencyQuality, VoiceProblem, latency_quality,
};
use mellow_client_core::shared_logic::voice_settings::{
    MAXIMUM_VOLUME_PERCENT, MINIMUM_VOLUME_PERCENT, PersonVoiceSetting, VoiceInputMode,
};
use mellow_protocol::{Snowflake, VoiceDisconnectReason, VoiceJoinRefusal};

use super::DuskShell;
use super::screen_share_host::screen_share_problem_sentence;
use crate::components::{
    Button, ButtonVariant, IconButton, IconButtonTone, RangeBounds, RangeField, Switch, Tooltip,
    child_id, focus_ring, icon, is_focus_visible, polite_live_region, remembered_focus_handle,
    remembered_hover, shadow, with_type,
};
use crate::i18n::{Strings, strings};
use crate::preferences::preferences;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CONTEXT_MENU_BORDER_WIDTH, COMPONENT_CONTEXT_MENU_MIN_WIDTH,
    COMPONENT_CONTEXT_MENU_PADDING, COMPONENT_CONTEXT_MENU_RADIUS,
    COMPONENT_CONTEXT_MENU_VIEWPORT_MARGIN, COMPONENT_USER_PANEL_BUTTON_ICON_SIZE,
    COMPONENT_USER_PANEL_BUTTON_SIZE, COMPONENT_USER_PANEL_TOGGLE_STRIKE_CUT_WIDTH,
    COMPONENT_USER_PANEL_TOGGLE_STRIKE_LENGTH, COMPONENT_USER_PANEL_TOGGLE_STRIKE_WIDTH,
    COMPONENT_VOICE_PANEL_MARGIN, COMPONENT_VOICE_PANEL_PADDING_BLOCK,
    COMPONENT_VOICE_PANEL_PADDING_INLINE, COMPONENT_VOICE_PANEL_RADIUS,
    COMPONENT_VOICE_PANEL_SIGNAL_ICON_SIZE, RADIUS_SM, RADIUS_XS, SPACE_2XS, SPACE_3XS, SPACE_XS,
    TYPE_BODY_SM, TYPE_USER_PANEL_NAME,
};
use crate::theme::{color, length, theme};
use crate::voice::global_push_to_talk::GlobalPushToTalkStatus;
use crate::voice::{CallPhase, Voice, current_settings};

/// A participant's popover: their volume and "Mute for me", for the reader only.
pub(super) struct VoicePersonPopover {
    user_id: Snowflake,
    name: SharedString,
    /// Their account, once their member is known: "Profile" opens their profile modal.
    user: Option<mellow_protocol::PublicUser>,
    position: Point<Pixels>,
    focus: FocusHandle,
    /// The participant row, which gets focus back.
    opener: Option<FocusHandle>,
}

/// What the problem card offers: joining again, or calling again (ADR-0062).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelRecovery {
    TryAgain,
    CallAgain(Snowflake),
}

/// What the panel's live regions say: the connection, then the latest join or leave.
fn phase_sentence(strings: Strings, phase: CallPhase, channel_name: &str, cx: &App) -> String {
    match phase {
        CallPhase::Connecting => strings.voice().connecting_to(channel_name),
        CallPhase::Reconnecting => strings.voice().reconnecting_to(channel_name),
        CallPhase::Connected => {
            let connected = strings.voice().connected_to(channel_name);
            let settings = preferences(cx).map(|preferences| &preferences.voice_settings);
            // Push-to-talk said once on connecting: otherwise silence would be a mystery.
            match settings.filter(|settings| settings.input_mode == VoiceInputMode::PushToTalk) {
                Some(settings) => {
                    format!("{connected} {}", push_to_talk_sentence(strings, settings))
                }
                None => connected,
            }
        }
    }
}

fn push_to_talk_sentence(
    strings: Strings,
    settings: &mellow_client_core::shared_logic::voice_settings::VoiceSettings,
) -> String {
    settings.push_to_talk_key.as_ref().map_or_else(
        || strings.voice().push_to_talk_no_key().to_owned(),
        |key| format!("{}.", strings.voice().push_to_talk_hint(&key.label)),
    )
}

/// Whether push-to-talk works in other apps, and whether that is a limit worth a warning icon;
/// `None` while there is nothing to say.
fn global_push_to_talk_sentence(
    strings: Strings,
    status: &GlobalPushToTalkStatus,
) -> Option<(bool, String)> {
    let desktop = strings.desktop();
    let (is_limited, sentence) = match status {
        GlobalPushToTalkStatus::Off => return None,
        GlobalPushToTalkStatus::Listening {
            trigger: Some(trigger),
            ..
        } => (false, desktop.global_push_to_talk_listening_as(trigger)),
        GlobalPushToTalkStatus::Listening {
            trigger: None,
            is_key_taken_from_other_apps: true,
        } => (
            false,
            desktop.global_push_to_talk_listening_key_taken().to_owned(),
        ),
        GlobalPushToTalkStatus::Listening {
            trigger: None,
            is_key_taken_from_other_apps: false,
        } => (false, desktop.global_push_to_talk_listening().to_owned()),
        GlobalPushToTalkStatus::PortalBoundWithoutKey => (
            true,
            desktop.global_push_to_talk_bound_without_key().to_owned(),
        ),
        GlobalPushToTalkStatus::KeyNotOnKeyboardLayout => (
            true,
            desktop.global_push_to_talk_key_not_on_layout().to_owned(),
        ),
        GlobalPushToTalkStatus::PortalFailed => {
            (true, desktop.global_push_to_talk_portal_failed().to_owned())
        }
        GlobalPushToTalkStatus::ListeningWhileX11WindowFocused => {
            (true, desktop.global_push_to_talk_x11_only().to_owned())
        }
        GlobalPushToTalkStatus::ModifierKeyWindowOnly => (
            true,
            desktop
                .global_push_to_talk_modifier_window_only()
                .to_owned(),
        ),
        GlobalPushToTalkStatus::KeyUnavailable => (
            true,
            desktop.global_push_to_talk_key_unavailable().to_owned(),
        ),
        GlobalPushToTalkStatus::PortalRefused => {
            (true, desktop.global_push_to_talk_refused().to_owned())
        }
        GlobalPushToTalkStatus::NoPortal => {
            (true, desktop.global_push_to_talk_no_portal().to_owned())
        }
    };
    Some((is_limited, sentence))
}

/// The latency in words, and its icon; `None` before the first measure.
fn signal_look(
    strings: Strings,
    round_trip_milliseconds: Option<u64>,
) -> (IconName, Option<LatencyQuality>, String) {
    let Some(milliseconds) = round_trip_milliseconds else {
        return (
            IconName::CellSignalHigh,
            None,
            strings.voice().latency_measuring().to_owned(),
        );
    };
    let quality = latency_quality(milliseconds);
    let (icon, words) = match quality {
        LatencyQuality::Good => (
            IconName::CellSignalHigh,
            strings.voice().latency_quality().good(),
        ),
        LatencyQuality::Medium => (
            IconName::CellSignalMedium,
            strings.voice().latency_quality().medium(),
        ),
        LatencyQuality::Poor => (
            IconName::CellSignalLow,
            strings.voice().latency_quality().poor(),
        ),
    };
    let milliseconds = i64::try_from(milliseconds).unwrap_or(i64::MAX);
    (
        icon,
        Some(quality),
        format!("{}, {words}", strings.voice().latency(milliseconds)),
    )
}

/// A problem's title and sentence (`problemLook`).
fn problem_words(
    strings: Strings,
    problem: VoiceProblem,
    channel_name: &str,
) -> (String, &'static str) {
    let voice = strings.voice();
    match problem {
        VoiceProblem::Refused { reason, .. } => {
            let refusals = voice.refusals();
            let sentence = match reason {
                VoiceJoinRefusal::MissingPermissions => refusals.missing_permissions(),
                VoiceJoinRefusal::ChannelFull => refusals.channel_full(),
                VoiceJoinRefusal::InstanceFull => refusals.instance_full(),
                VoiceJoinRefusal::InvalidOffer => refusals.invalid_offer(),
                VoiceJoinRefusal::VoiceUnavailable => refusals.voice_unavailable(),
                VoiceJoinRefusal::RateLimited => refusals.rate_limited(),
            };
            (voice.could_not_join(channel_name), sentence)
        }
        VoiceProblem::NoAnswer { .. } => (voice.could_not_join(channel_name), voice.no_answer()),
        VoiceProblem::Unreachable { .. } => {
            (voice.could_not_join(channel_name), voice.unreachable())
        }
        VoiceProblem::ConnectionLost { .. } => (
            voice.disconnected_from(channel_name),
            voice.connection_lost(),
        ),
        VoiceProblem::Disconnected { reason, .. } => {
            let disconnections = voice.disconnections();
            let sentence = match reason {
                VoiceDisconnectReason::MovedElsewhere => disconnections.moved_elsewhere(),
                VoiceDisconnectReason::PermissionsChanged => disconnections.permissions_changed(),
                VoiceDisconnectReason::ChannelDeleted => disconnections.channel_deleted(),
                VoiceDisconnectReason::RemovedFromGuild => disconnections.removed_from_guild(),
                VoiceDisconnectReason::MediaTimeout => disconnections.media_timeout(),
                VoiceDisconnectReason::CallEnded => disconnections.call_ended(),
                VoiceDisconnectReason::ServerStopping => disconnections.server_stopping(),
            };
            let title = if reason == VoiceDisconnectReason::MovedElsewhere {
                voice.moved_elsewhere().to_owned()
            } else {
                voice.disconnected_from(channel_name)
            };
            (title, sentence)
        }
        VoiceProblem::CouldNotStart { .. } => (
            strings.desktop().voice_could_not_start_title().to_owned(),
            strings.desktop().voice_could_not_start(),
        ),
    }
}

/// The device sentences under the panel's first row.
fn device_sentences(voice: &Voice, strings: Strings) -> Vec<&'static str> {
    let notices = voice.device_notices();
    [
        (
            notices.is_microphone_unavailable,
            strings.voice().microphone_problems().missing(),
        ),
        (
            notices.is_input_device_missing && !notices.is_microphone_unavailable,
            strings.voice().input_device_missing(),
        ),
        (
            notices.is_output_unavailable,
            strings.desktop().voice_output_unavailable(),
        ),
        (
            notices.is_output_device_missing && !notices.is_output_unavailable,
            strings.voice().output_device_missing(),
        ),
    ]
    .into_iter()
    .filter_map(|(is_shown, sentence)| is_shown.then_some(sentence))
    .collect()
}

impl DuskShell {
    /// A voice channel's name, or a conversation's for its call (ADR-0062).
    fn channel_name(&self, channel_id: Snowflake, cx: &App) -> String {
        let state = self.state(cx);
        if let Some(conversation) = state.direct_message_channel(channel_id) {
            let current_user_id = state.current_user().map(|user| user.id);
            return crate::home::conversation_display_name(
                conversation,
                current_user_id,
                strings(cx),
            );
        }
        state.guild_channel(channel_id).map_or_else(
            || strings(cx).voice().voice_channel().to_owned(),
            |channel| channel.name.clone(),
        )
    }

    /// The panel's live regions: always drawn while voice exists, so the first change is heard.
    pub(super) fn render_voice_announcements(&self, cx: &App) -> Option<AnyElement> {
        let voice = self.voice.as_ref()?.read(cx);
        let strings = strings(cx);
        let phase = voice
            .phase()
            .zip(voice.channel_id())
            .map(|(phase, channel_id)| {
                phase_sentence(strings, phase, &self.channel_name(channel_id, cx), cx)
            })
            .unwrap_or_default();
        let problem = voice
            .problem()
            .map(|problem| {
                let (title, sentence) = problem_words(
                    strings,
                    problem,
                    &self.channel_name(problem.channel_id(), cx),
                );
                format!("{title}. {sentence}")
            })
            .unwrap_or_default();
        let devices = device_sentences(voice, strings).join(" ");
        // A problem says why the share is off; otherwise its last start, end or cancel.
        let screen_share: SharedString = voice.screen_share_problem().map_or_else(
            || voice.screen_share_announcement(),
            |problem| screen_share_problem_sentence(strings, problem).into(),
        );
        // The Linux status comes after the compositor's dialog: heard once it is known.
        let global_push_to_talk =
            global_push_to_talk_sentence(strings, voice.global_push_to_talk_status())
                .map(|(_, sentence)| sentence)
                .unwrap_or_default();
        Some(
            div()
                .child(polite_live_region("voice-phase-announcement", phase.into()))
                .child(polite_live_region(
                    "voice-peer-announcement",
                    voice.announcement(),
                ))
                .child(polite_live_region(
                    "voice-problem-announcement",
                    problem.into(),
                ))
                .child(polite_live_region(
                    "voice-device-announcement",
                    devices.into(),
                ))
                .child(polite_live_region(
                    "voice-global-push-to-talk-announcement",
                    global_push_to_talk.into(),
                ))
                .child(polite_live_region(
                    "voice-screen-share-announcement",
                    screen_share,
                ))
                .into_any_element(),
        )
    }

    /// The card above the user card: the call, or the last problem.
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    pub(super) fn render_voice_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let voice_entity = self.voice.clone()?;
        let screen_share_section = self.render_screen_share_section(&voice_entity, window, cx);
        let camera_button = self.render_camera_button(cx);
        let voice = voice_entity.read(cx);
        let global_push_to_talk_status = voice.global_push_to_talk_status().clone();
        let strings = strings(cx);
        let (phase, place, round_trip, devices, problem) = (
            voice.phase(),
            voice.connected_place(),
            voice.round_trip(),
            device_sentences(voice, strings),
            voice.problem(),
        );
        let tokens = theme(cx);
        let card = div()
            .id("voice-panel")
            .role(Role::Region)
            .aria_label(strings.voice().panel())
            .flex_none()
            .mx(length(COMPONENT_VOICE_PANEL_MARGIN))
            .mb(length(SPACE_2XS))
            .px(length(COMPONENT_VOICE_PANEL_PADDING_INLINE))
            .py(length(COMPONENT_VOICE_PANEL_PADDING_BLOCK))
            .rounded(length(COMPONENT_VOICE_PANEL_RADIUS))
            .bg(color(tokens.component_voice_panel_background))
            .flex()
            .flex_col();
        let hint_line = |icon_name: Option<IconName>, text: SharedString, ink| {
            with_type(div(), TYPE_BODY_SM)
                .mt(length(SPACE_XS))
                .flex()
                .gap(length(SPACE_2XS))
                .text_color(ink)
                .when_some(icon_name, |line, icon_name| {
                    line.child(div().mt(length(SPACE_3XS)).child(icon(
                        icon_name,
                        COMPONENT_VOICE_PANEL_SIGNAL_ICON_SIZE,
                        ink,
                    )))
                })
                .child(div().min_w_0().child(text))
        };
        let conversation_id = voice.call_conversation_id();
        let panel_place = place
            .map(|(guild_id, channel_id)| (Some(guild_id), channel_id))
            .or(conversation_id.map(|conversation_id| (None, conversation_id)));
        if let (Some(phase), Some((guild_id, channel_id))) = (phase, panel_place) {
            let channel_name = self.channel_name(channel_id, cx);
            let guild_name = guild_id
                .and_then(|guild_id| self.state(cx).guild(guild_id))
                .map(|guild| guild.name.clone())
                .unwrap_or_default();
            // A call rings the others: its second line says so instead of the link.
            let ringing_sentence = guild_id
                .is_none()
                .then(|| {
                    let state = self.state(cx);
                    let call = state.call(channel_id)?;
                    (state.call_phase(channel_id)
                        == Some(mellow_client_core::state::CallPhase::Outgoing))
                    .then(|| {
                        strings.calls().ringing(&crate::calls::words::ringing_names(
                            call,
                            state.direct_message_channel(channel_id),
                            strings,
                        ))
                    })
                })
                .flatten();
            let round_trip = round_trip
                .map(|round_trip| u64::try_from(round_trip.as_millis()).unwrap_or(u64::MAX));
            let (signal_icon, quality, signal_description) = signal_look(strings, round_trip);
            let is_connected = phase == CallPhase::Connected;
            let signal_ink = color(match quality.filter(|_| is_connected) {
                None => tokens.component_voice_panel_signal_unknown,
                Some(LatencyQuality::Good) => tokens.component_voice_panel_signal_good,
                Some(LatencyQuality::Medium) => tokens.component_voice_panel_signal_medium,
                Some(LatencyQuality::Poor) => tokens.component_voice_panel_signal_poor,
            });
            let status = match phase {
                CallPhase::Connected => strings.voice().connected(),
                CallPhase::Connecting => strings.voice().connecting(),
                CallPhase::Reconnecting => strings.voice().reconnecting(),
            };
            let status_label: SharedString = if is_connected {
                format!("{status}, {signal_description}").into()
            } else {
                status.into()
            };
            let link_id = gpui::ElementId::from("voice-panel-channel");
            let link_focus = remembered_focus_handle(&link_id, window, cx);
            let link_hover = remembered_hover(&link_id, window, cx);
            let is_link_hovered = *link_hover.read(cx);
            let is_link_ring_shown = is_focus_visible(&link_focus, window);
            let settings = current_settings(cx);
            let push_to_talk = (settings.input_mode == VoiceInputMode::PushToTalk).then(|| {
                (
                    settings.push_to_talk_key.is_none(),
                    push_to_talk_sentence(strings, &settings),
                )
            });
            // Only beside a key that works in the window: no key, nothing to say about others.
            let global_push_to_talk = push_to_talk
                .as_ref()
                .filter(|(is_missing_key, _)| !is_missing_key)
                .and_then(|_| global_push_to_talk_sentence(strings, &global_push_to_talk_status));
            let tooltip_text: SharedString = signal_description.clone().into();
            let shell = cx.entity().downgrade();
            let leave_voice = voice_entity.downgrade();
            let row = div()
                .flex()
                .min_w_0()
                .items_center()
                .gap(length(SPACE_XS))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            with_type(div(), TYPE_USER_PANEL_NAME)
                                .id("voice-panel-status")
                                .role(Role::Label)
                                .aria_label(status_label)
                                .flex()
                                .min_w_0()
                                .items_center()
                                .gap(length(SPACE_2XS))
                                .text_color(color(if is_connected {
                                    tokens.component_voice_panel_status_connected_text
                                } else {
                                    tokens.component_voice_panel_status_pending_text
                                }))
                                .when(is_connected, |line| {
                                    line.tooltip(move |_, cx| {
                                        Tooltip::view(tooltip_text.clone(), cx)
                                    })
                                    .tooltip_show_delay(Tooltip::show_delay())
                                })
                                .child(icon(
                                    signal_icon,
                                    COMPONENT_VOICE_PANEL_SIGNAL_ICON_SIZE,
                                    signal_ink,
                                ))
                                .child(div().truncate().child(status)),
                        )
                        .when_some(ringing_sentence.clone(), |column, sentence| {
                            column.child(
                                with_type(div(), TYPE_BODY_SM)
                                    .min_w_0()
                                    .truncate()
                                    .text_color(color(tokens.component_voice_panel_channel_text))
                                    .child(sentence),
                            )
                        })
                        .when(ringing_sentence.is_none(), |column| {
                            column.child(
                                with_type(div(), TYPE_BODY_SM)
                                    .id(link_id)
                                    .role(Role::Link)
                                    .aria_label(if guild_id.is_some() {
                                        strings.voice().open_server(&channel_name, &guild_name)
                                    } else {
                                        strings.calls().open_conversation(&channel_name)
                                    })
                                    .track_focus(&link_focus)
                                    .relative()
                                    .self_start()
                                    .min_w_0()
                                    .truncate()
                                    .rounded(length(RADIUS_XS))
                                    .cursor_pointer()
                                    .text_color(color(if is_link_hovered {
                                        tokens.component_voice_panel_channel_text_hover
                                    } else {
                                        tokens.component_voice_panel_channel_text
                                    }))
                                    .when(is_link_hovered, gpui::Styled::underline)
                                    .on_hover(move |is_hovered, _, cx| {
                                        link_hover.update(cx, |remembered, cx| {
                                            *remembered = *is_hovered;
                                            cx.notify();
                                        });
                                    })
                                    .on_click(move |_, window, cx| {
                                        let _ = shell.update(cx, |shell, cx| match guild_id {
                                            Some(guild_id) => {
                                                shell.open_voice_server(guild_id, window, cx);
                                            }
                                            None => shell.open_channel(channel_id, window, cx),
                                        });
                                    })
                                    .child(if guild_id.is_some() {
                                        format!("{channel_name} / {guild_name}")
                                    } else {
                                        strings.calls().call_with(&channel_name)
                                    })
                                    .when(is_link_ring_shown, |link| {
                                        link.child(focus_ring(RADIUS_XS, cx))
                                    }),
                            )
                        }),
                )
                // A conversation's call has no soundboard (ADR-0062).
                .when(guild_id.is_some(), |row| {
                    row.child(self.render_soundboard_button(cx))
                })
                .child(
                    IconButton::new(
                        "voice-disconnect",
                        IconName::PhoneDisconnect,
                        if guild_id.is_some() {
                            strings.voice().disconnect()
                        } else {
                            strings.calls().hang_up()
                        },
                    )
                    .sizes(
                        COMPONENT_USER_PANEL_BUTTON_SIZE,
                        COMPONENT_USER_PANEL_BUTTON_ICON_SIZE,
                    )
                    .tone(IconButtonTone::Danger)
                    .on_click(move |_, cx| {
                        let _ = leave_voice.update(cx, Voice::leave);
                    }),
                );
            let hint_ink = color(tokens.component_voice_panel_hint_text);
            let microphone_ink = color(tokens.component_voice_panel_microphone_problem_text);
            // The second row, as on the web: the camera, then the screen share and its quality.
            let media_row = div()
                .flex()
                .items_start()
                .gap(length(SPACE_2XS))
                .child(div().mt(length(SPACE_XS)).child(camera_button))
                .child(div().flex_1().min_w_0().children(screen_share_section));
            return Some(
                card.child(row)
                    .child(media_row)
                    .when_some(push_to_talk, |card, (is_missing_key, sentence)| {
                        card.child(hint_line(
                            is_missing_key.then_some(IconName::WarningCircle),
                            sentence.into(),
                            hint_ink,
                        ))
                    })
                    .when_some(global_push_to_talk, |card, (is_limited, sentence)| {
                        card.child(hint_line(
                            is_limited.then_some(IconName::WarningCircle),
                            sentence.into(),
                            hint_ink,
                        ))
                    })
                    .children(devices.into_iter().map(|sentence| {
                        hint_line(
                            Some(IconName::WarningCircle),
                            sentence.into(),
                            if sentence == strings.voice().microphone_problems().missing() {
                                microphone_ink
                            } else {
                                hint_ink
                            },
                        )
                    }))
                    .children(self.render_camera_notice(cx))
                    .into_any_element(),
            );
        }
        // What went wrong last: the voice's problem, or a call that could not start (ADR-0062).
        let start_problem =
            crate::calls::calls(cx).and_then(|calls| calls.read(cx).start_problem());
        let (title, sentence, is_notice, problem_icon, recovery) = if let Some(problem) = problem {
            let place = self.channel_name(problem.channel_id(), cx);
            let is_call = self
                .state(cx)
                .direct_message_channel(problem.channel_id())
                .is_some();
            let call_look = is_call
                .then(|| crate::calls::words::call_problem_look(problem, &place, strings))
                .flatten();
            if let Some(look) = call_look {
                // A call says some problems in its own words: nobody answered is not an error.
                (
                    look.title,
                    look.sentence,
                    look.is_notice,
                    if look.is_notice {
                        IconName::PhoneX
                    } else {
                        IconName::WarningCircle
                    },
                    look.can_call_again
                        .then_some(PanelRecovery::CallAgain(problem.channel_id())),
                )
            } else {
                let (title, sentence) = problem_words(
                    strings,
                    problem,
                    &self.channel_name(problem.channel_id(), cx),
                );
                let is_notice = problem.is_notice();
                (
                    title,
                    sentence,
                    is_notice,
                    if is_notice {
                        IconName::SpeakerHigh
                    } else {
                        IconName::WarningCircle
                    },
                    problem.can_retry().then_some(PanelRecovery::TryAgain),
                )
            }
        } else {
            let start_problem = start_problem?;
            (
                strings
                    .calls()
                    .could_not_call(&self.channel_name(start_problem.channel_id, cx)),
                crate::calls::words::call_start_failure_sentence(start_problem.failure, strings),
                false,
                IconName::WarningCircle,
                start_problem
                    .failure
                    .may_work_again()
                    .then_some(PanelRecovery::CallAgain(start_problem.channel_id)),
            )
        };
        let retry_voice = voice_entity.downgrade();
        let dismiss_voice = voice_entity.downgrade();
        Some(
            card.child(
                div()
                    .flex()
                    .min_w_0()
                    .items_start()
                    .gap(length(SPACE_XS))
                    .child(div().mt(length(SPACE_3XS)).child(icon(
                        problem_icon,
                        COMPONENT_VOICE_PANEL_SIGNAL_ICON_SIZE,
                        color(if is_notice {
                            tokens.component_voice_panel_notice_icon
                        } else {
                            tokens.component_voice_panel_problem_icon
                        }),
                    )))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(length(SPACE_3XS))
                            .child(
                                with_type(div(), TYPE_USER_PANEL_NAME)
                                    .text_color(color(
                                        tokens.component_voice_panel_problem_title_text,
                                    ))
                                    .child(title),
                            )
                            .child(
                                with_type(div(), TYPE_BODY_SM)
                                    .text_color(color(tokens.component_voice_panel_problem_text))
                                    .child(sentence),
                            ),
                    )
                    .child(
                        IconButton::new(
                            "voice-problem-close",
                            IconName::X,
                            strings.common().close(),
                        )
                        .sizes(
                            COMPONENT_USER_PANEL_BUTTON_SIZE,
                            COMPONENT_VOICE_PANEL_SIGNAL_ICON_SIZE,
                        )
                        .on_click(move |_, cx| {
                            let _ = dismiss_voice.update(cx, Voice::dismiss_problem);
                            if let Some(calls) = crate::calls::calls(cx) {
                                calls.update(cx, crate::calls::Calls::dismiss_start_problem);
                            }
                        }),
                    ),
            )
            .when_some(recovery, |card, recovery| {
                let label = match recovery {
                    PanelRecovery::TryAgain => strings.common().try_again(),
                    PanelRecovery::CallAgain(_) => strings.calls().call_again(),
                };
                card.child(
                    div().mt(length(SPACE_XS)).flex().child(
                        Button::new("voice-retry", label)
                            .variant(ButtonVariant::Secondary)
                            .full_width()
                            .on_click(move |_, cx| match recovery {
                                PanelRecovery::TryAgain => {
                                    let _ = retry_voice.update(cx, Voice::retry);
                                }
                                PanelRecovery::CallAgain(channel_id) => {
                                    let _ = retry_voice.update(cx, Voice::dismiss_problem);
                                    if let Some(calls) = crate::calls::calls(cx) {
                                        calls.update(cx, |calls, cx| {
                                            calls.start_call(channel_id, cx, |_, _| {});
                                        });
                                    }
                                }
                            }),
                    ),
                )
            })
            .into_any_element(),
        )
    }

    /// The panel's channel line opens the server, at the open channel when it is there already.
    fn open_voice_server(
        &mut self,
        guild_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let channel_id = if self.navigation.selected_guild_id() == Some(guild_id) {
            self.navigation.selected_channel_id()
        } else {
            None
        };
        self.open_place(Some(guild_id), channel_id, window, cx);
    }

    /// The microphone and headset toggles of the user card, before the gear.
    pub(super) fn render_voice_toggles(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(voice_entity) = self.voice.clone() else {
            return Vec::new();
        };
        let toggles = voice_entity.read(cx).toggles();
        let strings = strings(cx);
        let words = strings.user_card();
        let modifier = if cfg!(target_os = "macos") {
            "Meta"
        } else {
            "Control"
        };
        let microphone = voice_entity.downgrade();
        let headset = voice_entity.downgrade();
        vec![
            voice_toggle(
                VoiceToggleLook {
                    id: "voice-microphone-toggle",
                    icon: IconName::Microphone,
                    label: words.mute_microphone(),
                    tooltip: if toggles.is_muted() {
                        words.unmute_microphone()
                    } else {
                        words.mute_microphone()
                    },
                    is_pressed: toggles.is_muted(),
                    key_shortcuts: format!("{modifier}+Shift+M").into(),
                },
                move |cx| {
                    let _ = microphone.update(cx, Voice::toggle_microphone);
                },
                window,
                cx,
            ),
            voice_toggle(
                VoiceToggleLook {
                    id: "voice-headset-toggle",
                    icon: IconName::Headphones,
                    label: words.deafen(),
                    tooltip: if toggles.is_deafened() {
                        words.undeafen()
                    } else {
                        words.deafen()
                    },
                    is_pressed: toggles.is_deafened(),
                    key_shortcuts: format!("{modifier}+Shift+D").into(),
                },
                move |cx| {
                    let _ = headset.update(cx, Voice::toggle_deafened);
                },
                window,
                cx,
            ),
        ]
    }

    // ---- A participant's popover ----

    /// A participant's row was pressed: their volume and "Mute for me" open where it was.
    pub(super) fn open_voice_person_popover(
        &mut self,
        user_id: Snowflake,
        name: SharedString,
        user: Option<mellow_protocol::PublicUser>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = cx.focus_handle();
        let opener = window.focused(cx);
        window.focus(&focus, cx);
        self.voice_person_popover = Some(VoicePersonPopover {
            user_id,
            name,
            user,
            position,
            focus,
            opener,
        });
        cx.notify();
    }

    fn close_voice_person_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(popover) = self.voice_person_popover.take() {
            if let Some(opener) = popover.opener {
                window.focus(&opener, cx);
            }
            cx.notify();
        }
    }

    #[allow(clippy::too_many_lines)] // One popover, in the order it is drawn.
    pub(super) fn render_voice_person_popover(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let popover = self.voice_person_popover.as_ref()?;
        let voice = self.voice.clone()?;
        let tokens = theme(cx);
        let strings = strings(cx);
        let user_id = popover.user_id;
        let setting = self
            .voice
            .as_ref()
            .map_or_else(PersonVoiceSetting::default, |voice| {
                voice.read(cx).effective_person_setting(user_id, cx)
            });
        let volume_voice = voice.downgrade();
        let mute_voice = voice.downgrade();
        let bounds = RangeBounds {
            minimum: f32::from(u16::try_from(MINIMUM_VOLUME_PERCENT).unwrap_or(0)),
            maximum: f32::from(u16::try_from(MAXIMUM_VOLUME_PERCENT).unwrap_or(200)),
            step: 1.0,
        };
        // Whole percents from 0 to 200: exact in `f32`.
        #[allow(clippy::cast_precision_loss)]
        let volume = setting.volume_percent as f32;
        let card = div()
            .id("voice-person-popover")
            .role(Role::Dialog)
            .aria_label(popover.name.clone())
            .track_focus(&popover.focus)
            .on_key_down(cx.listener(|shell, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    shell.close_voice_person_popover(window, cx);
                }
            }))
            .on_mouse_down_out(cx.listener(|shell, _, window, cx| {
                shell.close_voice_person_popover(window, cx);
            }))
            .min_w(length(COMPONENT_CONTEXT_MENU_MIN_WIDTH))
            .p(length(COMPONENT_CONTEXT_MENU_PADDING))
            .rounded(length(COMPONENT_CONTEXT_MENU_RADIUS))
            .border(length(COMPONENT_CONTEXT_MENU_BORDER_WIDTH))
            .border_color(color(tokens.component_context_menu_border))
            .bg(color(tokens.component_context_menu_background))
            .shadow(shadow(tokens.component_context_menu_shadow))
            .flex()
            .flex_col()
            .gap(length(SPACE_XS))
            // Their profile, as the person menu's « Profil » (DESIGN.md §5.7.3).
            .children(popover.user.clone().map(|user| {
                let shell = cx.entity().downgrade();
                Button::new(
                    child_id(&"voice-person-popover".into(), "profile"),
                    strings.members().profile(),
                )
                .variant(ButtonVariant::Ghost)
                .full_width()
                .on_click(move |window, cx| {
                    let opener = shell
                        .update(cx, |shell, cx| {
                            let opener = shell
                                .voice_person_popover
                                .as_ref()
                                .and_then(|popover| popover.opener.clone());
                            shell.close_voice_person_popover(window, cx);
                            opener
                        })
                        .ok()
                        .flatten();
                    crate::profile_surfaces::open_profile_modal(user.clone(), opener, window, cx);
                })
            }))
            .child(
                RangeField::new(
                    child_id(&"voice-person-popover".into(), "volume"),
                    strings.voice().user_volume(),
                    volume,
                    bounds,
                    move |value| {
                        // Whole percents within 0 to 200.
                        #[allow(clippy::cast_possible_truncation)]
                        let percent = value.round() as i64;
                        strings.voice_settings().percent(percent).into()
                    },
                )
                .on_change(move |value, _, cx| {
                    // Snapped to whole percents within 0 to 200 by the slider.
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let percent = value.round().max(0.0) as u32;
                    let _ = volume_voice.update(cx, |voice, cx| {
                        voice.set_person_volume(user_id, percent, cx);
                    });
                }),
            )
            .child(
                Switch::new(
                    child_id(&"voice-person-popover".into(), "mute"),
                    setting.is_locally_muted,
                    strings.voice().mute_for_me(),
                )
                .without_divider()
                .on_change(move |_, _, cx| {
                    let _ =
                        mute_voice.update(cx, |voice, cx| voice.toggle_person_muted(user_id, cx));
                }),
            );
        let margin = px(COMPONENT_CONTEXT_MENU_VIEWPORT_MARGIN);
        Some(
            deferred(
                anchored()
                    .position(popover.position)
                    .snap_to_window_with_margin(margin)
                    .child(card),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}

struct VoiceToggleLook {
    id: &'static str,
    icon: IconName,
    /// The accessible name stays; the state is `aria-pressed`, the strike and the colour.
    label: &'static str,
    tooltip: &'static str,
    is_pressed: bool,
    key_shortcuts: SharedString,
}

/// A user card toggle (`UserCard.svelte`'s `.voice-toggle`): the strike is cut out of the surface
/// behind it, so it reads across the glyph, never by colour alone.
fn voice_toggle(
    look: VoiceToggleLook,
    on_click: impl Fn(&mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let id = gpui::ElementId::from(look.id);
    let focus = remembered_focus_handle(&id, window, cx);
    let hover = remembered_hover(&id, window, cx);
    let is_hovered = *hover.read(cx);
    let surface = color(if is_hovered {
        tokens.component_user_panel_button_background_hover
    } else {
        tokens.component_user_panel_background
    });
    let ink = color(if look.is_pressed {
        tokens.component_user_panel_toggle_muted_icon
    } else {
        tokens.component_user_panel_button_icon
    });
    let strike = color(tokens.component_user_panel_toggle_muted_icon);
    let is_ring_shown = is_focus_visible(&focus, window);
    let tooltip: SharedString = look.tooltip.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(look.label)
        .aria_toggled(if look.is_pressed {
            Toggled::True
        } else {
            Toggled::False
        })
        .aria_keyshortcuts(look.key_shortcuts)
        .track_focus(&focus)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(length(COMPONENT_USER_PANEL_BUTTON_SIZE))
        .rounded(length(RADIUS_SM))
        .bg(surface)
        .cursor_pointer()
        .tooltip(move |_, cx| Tooltip::view(tooltip.clone(), cx))
        .tooltip_show_delay(Tooltip::show_delay())
        .on_hover(move |is_hovered, _, cx| {
            hover.update(cx, |remembered, cx| {
                *remembered = *is_hovered;
                cx.notify();
            });
        })
        .on_click(move |_, _, cx| on_click(cx))
        .child(icon(look.icon, COMPONENT_USER_PANEL_BUTTON_ICON_SIZE, ink))
        .when(look.is_pressed, |button| {
            button.child(
                canvas(
                    |_, _, _| {},
                    move |bounds, (), window, _| {
                        let rem_size = window.rem_size();
                        let to_pixels = |token: f32| f32::from(length(token).to_pixels(rem_size));
                        // A diagonal of the strike's length, top right to bottom left, as -45°.
                        let half = to_pixels(COMPONENT_USER_PANEL_TOGGLE_STRIKE_LENGTH)
                            / 2.0
                            / std::f32::consts::SQRT_2;
                        let centre = bounds.center();
                        let from = centre + point(px(half), px(-half));
                        let to = centre + point(px(-half), px(half));
                        let width = to_pixels(COMPONENT_USER_PANEL_TOGGLE_STRIKE_WIDTH);
                        let cut = to_pixels(COMPONENT_USER_PANEL_TOGGLE_STRIKE_CUT_WIDTH);
                        for (stroke, fill) in [(width + 2.0 * cut, surface), (width, strike)] {
                            let mut path = PathBuilder::stroke(px(stroke));
                            path.move_to(from);
                            path.line_to(to);
                            // Two points never make a malformed path; drawing nothing is the
                            // fallback.
                            if let Ok(path) = path.build() {
                                window.paint_path(path, fill);
                            }
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
        })
        .when(is_ring_shown, |button| {
            button.child(focus_ring(RADIUS_SM, cx))
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_signal_drops_a_bar_as_the_round_trip_grows_and_says_it_in_words() {
        let strings = Strings::default();
        let (icon, quality, words) = signal_look(strings, Some(42));
        assert_eq!(icon, IconName::CellSignalHigh);
        assert_eq!(quality, Some(LatencyQuality::Good));
        assert_eq!(words, "Latency: 42 ms, good connection");
        assert_eq!(
            signal_look(strings, Some(180)).0,
            IconName::CellSignalMedium
        );
        assert_eq!(signal_look(strings, Some(400)).0, IconName::CellSignalLow);
        assert_eq!(
            signal_look(strings, None),
            (
                IconName::CellSignalHigh,
                None,
                "Measuring latency…".to_owned()
            )
        );
    }

    #[test]
    fn a_problem_says_what_happened_and_a_move_reads_as_a_notice() {
        let strings = Strings::default();
        let channel_id = Snowflake::from_raw(1);
        assert_eq!(
            problem_words(
                strings,
                VoiceProblem::Refused {
                    channel_id,
                    reason: VoiceJoinRefusal::ChannelFull
                },
                "lounge"
            ),
            (
                "Couldn't join lounge".to_owned(),
                "The channel is full. Try again when someone leaves."
            )
        );
        assert_eq!(
            problem_words(
                strings,
                VoiceProblem::Disconnected {
                    channel_id,
                    reason: VoiceDisconnectReason::MovedElsewhere
                },
                "lounge"
            )
            .0,
            "Voice moved"
        );
    }
}
