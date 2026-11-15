//! The incoming call card (`IncomingCallCard.svelte`, DESIGN.md §5.6.3, ADR-0062): floating at the
//! top of the window's content wherever the reader is, the caller's face with Melli ringing on
//! it, then Refuser and Décrocher. It does not block the app: a call is an interruption, not a
//! question that must be answered first. It stays as it is while the same call rings (a
//! `CALL_UPDATE` neither opens it again nor takes focus again). Focus goes to Décrocher unless the
//! reader is typing (Enter would then answer by accident) or in Do not disturb, and comes back
//! where it was when the card goes; Ctrl+Enter answers from anywhere and Esc declines while focus
//! is in the card. The sentence is read out once per call. The ringtone and the notification are
//! `crate::calls`'.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Context, FocusHandle, InteractiveElement as _,
    IntoElement, Keystroke, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, anchored, deferred, div, point,
    prelude::FluentBuilder as _, px,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_protocol::{ChosenStatus, Snowflake};

use super::{DuskShell, profile_picture};
use crate::calls::words::{call_person_of, incoming_call_sentence, other_people};
use crate::components::{
    Avatar, AvatarPerson, Button, ButtonVariant, GroupAvatar, Melli, MelliState, child_id,
    colour_transition, easing, focus_ring, icon, is_focus_visible, remembered_hover, shadow,
    with_type,
};
use crate::home::conversation_display_name;
use crate::i18n::strings;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_BUTTON_HEIGHT_LARGE, COMPONENT_BUTTON_PADDING_INLINE, COMPONENT_BUTTON_RADIUS,
    COMPONENT_INCOMING_CALL_ACTION_GAP, COMPONENT_INCOMING_CALL_AVATAR_SIZE,
    COMPONENT_INCOMING_CALL_BORDER_WIDTH, COMPONENT_INCOMING_CALL_ENTER_DURATION,
    COMPONENT_INCOMING_CALL_ENTER_OFFSET, COMPONENT_INCOMING_CALL_GAP,
    COMPONENT_INCOMING_CALL_HINT_ICON_SIZE, COMPONENT_INCOMING_CALL_MARGIN,
    COMPONENT_INCOMING_CALL_MELLI_SIZE, COMPONENT_INCOMING_CALL_PADDING,
    COMPONENT_INCOMING_CALL_RADIUS, COMPONENT_INCOMING_CALL_WIDTH, MOTION_EASING_SETTLE, SPACE_2XS,
    SPACE_SM, TYPE_BODY, TYPE_BODY_SM, TYPE_BUTTON, TYPE_HEADING_MD,
};
use crate::theme::{animation_duration, color, length, theme};

/// Over dialogs, menus and the media viewer: a call reaches the reader anywhere in the app.
const INCOMING_CALL_LAYER_PRIORITY: usize = 4;
/// The key context of the focused text fields (`gpui_base`'s inputs): the reader is typing.
const TEXT_INPUT_KEY_CONTEXT: &str = "Input";

/// The call the card shows, kept while it rings.
pub(super) struct ShownIncomingCall {
    pub(super) key: (Snowflake, Snowflake),
    /// What had focus when it came, to give it back when it goes.
    opener: Option<FocusHandle>,
    /// "Ada t'appelle. Ctrl+Entrée pour décrocher.", once per call.
    pub(super) announcement: SharedString,
}

/// The card's focus targets, made once with the shell.
pub(super) struct IncomingCallFocus {
    pub(super) card: FocusHandle,
    pub(super) answer: FocusHandle,
    pub(super) decline: FocusHandle,
}

impl IncomingCallFocus {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            card: cx.focus_handle(),
            answer: cx.focus_handle().tab_stop(true),
            decline: cx.focus_handle().tab_stop(true),
        }
    }
}

/// Whether focus is in a text field: the card then leaves it there.
fn is_typing(window: &Window) -> bool {
    window
        .context_stack()
        .iter()
        .any(|context| context.contains(TEXT_INPUT_KEY_CONTEXT))
}

impl DuskShell {
    fn is_do_not_disturb(&self, cx: &App) -> bool {
        self.state(cx)
            .current_user()
            .is_some_and(|user| user.status == ChosenStatus::DoNotDisturb)
    }

    /// The ringing call changed (`Calls` notified): the card comes, goes or moves to another call.
    pub(super) fn follow_incoming_call(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ringing = crate::calls::calls(cx).and_then(|calls| calls.read(cx).ringing_call(cx));
        let key = ringing
            .as_ref()
            .map(|call| (call.channel_id, call.message_id));
        if self.incoming_call.as_ref().map(|shown| shown.key) == key {
            return;
        }
        let had_focus = self.incoming_call_focus.card.contains_focused(window, cx);
        let previous = self.incoming_call.take();
        let Some(call) = ringing else {
            if had_focus {
                let opener = previous.and_then(|shown| shown.opener);
                let focus = opener.unwrap_or_else(|| self.root_focus.clone());
                window.focus(&focus, cx);
            }
            cx.notify();
            return;
        };
        let state = self.state(cx);
        let strings = strings(cx);
        let current_user_id = state.current_user().map(|user| user.id);
        let announcement = state
            .direct_message_channel(call.channel_id)
            .map(|conversation| {
                strings.calls().announcement(&incoming_call_sentence(
                    &call,
                    conversation,
                    current_user_id,
                    strings,
                ))
            })
            .unwrap_or_default();
        // A card replacing another keeps the first opener.
        let opener = previous
            .and_then(|shown| shown.opener)
            .or_else(|| window.focused(cx).filter(|_| !had_focus));
        if !self.is_do_not_disturb(cx) && !is_typing(window) {
            window.focus(&self.incoming_call_focus.answer, cx);
        }
        self.incoming_call = Some(ShownIncomingCall {
            key: (call.channel_id, call.message_id),
            opener,
            announcement: announcement.into(),
        });
        cx.notify();
    }

    /// Ctrl+Enter (Cmd+Enter) answers the ringing call from anywhere, before the composer sees
    /// it; Esc declines it while focus is in the card. `true` when the key was used.
    pub(super) fn answer_or_decline_with_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((channel_id, _)) = self.incoming_call.as_ref().map(|shown| shown.key) else {
            return false;
        };
        let modifiers = keystroke.modifiers;
        let is_answer = keystroke.key == "enter"
            && modifiers.secondary(cx)
            && !modifiers.shift
            && !modifiers.alt;
        let is_decline = keystroke.key == "escape"
            && !modifiers.modified()
            && !crate::profile_surfaces::is_profile_hover_card_shown(cx)
            && self.incoming_call_focus.card.contains_focused(window, cx);
        if is_answer {
            self.answer_call(channel_id, window, cx);
        } else if is_decline {
            Self::decline_ringing_call(channel_id, cx);
        }
        is_answer || is_decline
    }

    pub(super) fn decline_ringing_call(channel_id: Snowflake, cx: &mut App) {
        if let Some(calls) = crate::calls::calls(cx) {
            calls.update(cx, |calls, cx| calls.decline_call(channel_id, cx));
        }
    }

    /// Where answering takes the reader's voice from, said before they press it.
    fn place_left_by_answering(&self, cx: &App) -> Option<String> {
        let voice = self.voice.as_ref()?.read(cx);
        let channel_id = voice.channel_id()?;
        let state = self.state(cx);
        if let Some(conversation) = state.direct_message_channel(channel_id) {
            let current_user_id = state.current_user().map(|user| user.id);
            return Some(conversation_display_name(
                conversation,
                current_user_id,
                strings(cx),
            ));
        }
        state
            .guild_channel(channel_id)
            .map(|channel| channel.name.clone())
    }

    /// The card, or nothing while no call rings the reader.
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    pub(super) fn render_incoming_call(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let calls = crate::calls::calls(cx)?;
        let call = calls.read(cx).ringing_call(cx)?;
        let tokens = theme(cx);
        let strings = strings(cx);
        let state = self.state(cx);
        let conversation = state.direct_message_channel(call.channel_id)?.clone();
        let current_user_id = state.current_user().map(|user| user.id);
        let is_group = conversation.owner_id.is_some();
        let caller = call_person_of(Some(&conversation), call.caller_id, strings);
        let is_do_not_disturb = self.is_do_not_disturb(cx);
        let place_left = self.place_left_by_answering(cx);
        let sentence = incoming_call_sentence(&call, &conversation, current_user_id, strings);
        let shown_sentence = if is_group {
            strings.calls().in_group(&conversation_display_name(
                &conversation,
                current_user_id,
                strings,
            ))
        } else {
            strings.calls().is_calling_you().to_owned()
        };
        let face = if is_group {
            GroupAvatar::new(
                other_people(&conversation, current_user_id)
                    .iter()
                    .map(|person| {
                        let picture = person.avatar.map(|image| {
                            profile_picture(
                                image,
                                COMPONENT_INCOMING_CALL_AVATAR_SIZE,
                                true,
                                window,
                                cx,
                            )
                        });
                        AvatarPerson::from_user(person, picture)
                    })
                    .collect(),
                COMPONENT_INCOMING_CALL_AVATAR_SIZE,
                color(tokens.component_incoming_call_background),
            )
            .into_any_element()
        } else {
            let picture = caller.avatar.map(|image| {
                profile_picture(image, COMPONENT_INCOMING_CALL_AVATAR_SIZE, true, window, cx)
            });
            Avatar::new(
                AvatarPerson::from_user(&caller, picture),
                COMPONENT_INCOMING_CALL_AVATAR_SIZE,
            )
            .into_any_element()
        };
        let channel_id = call.channel_id;
        let shell = cx.entity().downgrade();
        let answer = Button::new("incoming-call-answer", strings.calls().answer())
            .variant(ButtonVariant::Primary)
            .size(crate::components::ButtonSize::Large)
            .leading_icon(IconName::Phone, COMPONENT_INCOMING_CALL_HINT_ICON_SIZE)
            .full_width()
            .track_focus(&self.incoming_call_focus.answer)
            .on_click({
                let shell = shell.clone();
                move |window, cx| {
                    let _ = shell.update(cx, |shell, cx| shell.answer_call(channel_id, window, cx));
                }
            });
        let decline = self.render_decline_key(channel_id, window, cx);
        let hint = if is_do_not_disturb {
            Some((
                IconName::BellSlash,
                strings.calls().silent_for_do_not_disturb().to_owned(),
            ))
        } else {
            place_left.map(|place| (IconName::PhoneCall, strings.calls().answer_leaves(&place)))
        };

        let content = crate::window_chrome::client_content_bounds(window);
        let content_width = f32::from(content.size.width);
        let width = COMPONENT_INCOMING_CALL_WIDTH
            .min(content_width - 2.0 * COMPONENT_INCOMING_CALL_MARGIN)
            .max(0.0);
        let left = content.origin.x + px((content_width - width) / 2.0);
        let top = content.origin.y + px(COMPONENT_INCOMING_CALL_MARGIN);
        let (call_key_channel, call_key_message) = (call.channel_id, call.message_id);
        let enter = animation_duration(COMPONENT_INCOMING_CALL_ENTER_DURATION, cx)
            .max(Duration::from_millis(1));
        let card = div()
            .id(SharedString::from(format!(
                "incoming-call-{call_key_channel}-{call_key_message}"
            )))
            .debug_selector(|| "incoming-call".to_owned())
            .role(Role::Dialog)
            .aria_label(strings.calls().incoming())
            .track_focus(&self.incoming_call_focus.card)
            .occlude()
            .w(length(width))
            .flex()
            .flex_col()
            .gap(length(COMPONENT_INCOMING_CALL_GAP))
            .p(length(COMPONENT_INCOMING_CALL_PADDING))
            .rounded(length(COMPONENT_INCOMING_CALL_RADIUS))
            .border(length(COMPONENT_INCOMING_CALL_BORDER_WIDTH))
            .border_color(color(tokens.component_incoming_call_border))
            .bg(color(tokens.component_incoming_call_background))
            .shadow(shadow(tokens.component_incoming_call_shadow))
            .text_color(color(tokens.color_text_normal))
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap(length(SPACE_SM))
                    .child(div().flex_none().child(face))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                with_type(div(), TYPE_HEADING_MD)
                                    .truncate()
                                    .text_color(color(tokens.component_incoming_call_name_text))
                                    .child(display_name(&caller).to_owned()),
                            )
                            .child(
                                with_type(div(), TYPE_BODY)
                                    .id("incoming-call-sentence")
                                    .role(Role::Label)
                                    .aria_label(sentence)
                                    .truncate()
                                    .text_color(color(tokens.component_incoming_call_sentence_text))
                                    .child(shown_sentence),
                            ),
                    )
                    // Melli rings the phone: still and quiet under Do not disturb.
                    .child(
                        Melli::new(
                            "incoming-call-melli",
                            MelliState::IncomingCall,
                            COMPONENT_INCOMING_CALL_MELLI_SIZE,
                        )
                        .still(is_do_not_disturb),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(length(COMPONENT_INCOMING_CALL_ACTION_GAP))
                    .child(div().flex_1().min_w_0().child(decline))
                    .child(div().flex_1().min_w_0().child(answer)),
            )
            .when_some(hint, |card, (hint_icon, text)| {
                card.child(
                    with_type(div(), TYPE_BODY_SM)
                        .mt(length(-SPACE_2XS))
                        .flex()
                        .items_center()
                        .gap(length(SPACE_2XS))
                        .text_color(color(tokens.component_incoming_call_sentence_text))
                        .child(icon(
                            hint_icon,
                            COMPONENT_INCOMING_CALL_HINT_ICON_SIZE,
                            color(tokens.component_incoming_call_sentence_text),
                        ))
                        .child(div().min_w_0().child(text)),
                )
            })
            // GPUI cannot scale a box: the card lands from above and fades in.
            .with_animation(
                SharedString::from(format!(
                    "incoming-call-enter-{call_key_channel}-{call_key_message}"
                )),
                Animation::new(enter)
                    .with_easing(|progress| easing(MOTION_EASING_SETTLE).sample(progress)),
                |card, progress| {
                    card.opacity(progress.clamp(0.0, 1.0)).mt(length(
                        COMPONENT_INCOMING_CALL_ENTER_OFFSET * (1.0 - progress),
                    ))
                },
            );
        Some(
            deferred(anchored().position(point(left, top)).child(card))
                .with_priority(INCOMING_CALL_LAYER_PRIORITY)
                .into_any_element(),
        )
    }

    /// Refuser: quiet dusk at rest, warming to red under the pointer (declining ends nothing).
    fn render_decline_key(
        &self,
        channel_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tokens = theme(cx);
        let id = gpui::ElementId::from("incoming-call-decline");
        let hover = remembered_hover(&id, window, cx);
        let is_hovered = *hover.read(cx);
        let fill = colour_transition(
            child_id(&id, "background"),
            color(if is_hovered {
                tokens.component_incoming_call_decline_background_hover
            } else {
                tokens.component_incoming_call_decline_background
            }),
            window,
            cx,
        );
        let ink = color(if is_hovered {
            tokens.component_incoming_call_decline_text_hover
        } else {
            tokens.component_incoming_call_decline_text
        });
        let focus = self.incoming_call_focus.decline.clone();
        let is_ring_shown = is_focus_visible(&focus, window);
        with_type(div(), TYPE_BUTTON)
            .id(id)
            .role(Role::Button)
            .aria_label(strings(cx).calls().decline())
            .aria_keyshortcuts("Escape")
            .track_focus(&focus)
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .gap(length(SPACE_2XS))
            .w_full()
            .h(length(COMPONENT_BUTTON_HEIGHT_LARGE))
            .px(length(COMPONENT_BUTTON_PADDING_INLINE))
            .rounded(length(COMPONENT_BUTTON_RADIUS))
            .bg(fill)
            .text_color(ink)
            .cursor_pointer()
            .on_hover(move |is_hovered, _, cx| {
                hover.update(cx, |remembered, cx| {
                    *remembered = *is_hovered;
                    cx.notify();
                });
            })
            .on_click(move |_, _, cx| {
                DuskShell::decline_ringing_call(channel_id, cx);
            })
            .child(icon(
                IconName::PhoneDisconnect,
                COMPONENT_INCOMING_CALL_HINT_ICON_SIZE,
                ink,
            ))
            .child(strings(cx).calls().decline())
            .when(is_ring_shown, |key| {
                key.child(focus_ring(COMPONENT_BUTTON_RADIUS, cx))
            })
    }
}

#[cfg(test)]
#[path = "incoming_call_tests.rs"]
mod tests;
