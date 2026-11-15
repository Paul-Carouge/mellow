//! "Appel en cours · Rejoindre" (`CallBanner.svelte`, DESIGN.md §5.6.3, ADR-0062): over the
//! conversation, under its header like the unread bar (which moves under it), so the rows being
//! read do not move. It shows while the conversation's call goes on without the reader (declined,
//! missed, or left), and while they are in it with the call view closed ("Afficher l'appel"). A
//! call ringing them has its own card.

use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Role, StatefulInteractiveElement as _, Styled as _, div,
};
use mellow_client_core::state::CallPhase;

use super::ConversationView;
use crate::components::{Button, ButtonSize, ButtonVariant, icon, with_type};
use crate::i18n::strings;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CALL_BANNER_HEIGHT, COMPONENT_CALL_BANNER_ICON_SIZE, COMPONENT_CALL_BANNER_MARGIN,
    COMPONENT_CALL_BANNER_PADDING_INLINE, COMPONENT_CALL_BANNER_RADIUS,
    COMPONENT_CALL_MESSAGE_ICON_SIZE, SPACE_2XS, SPACE_3XS, SPACE_XS, TYPE_BODY,
};
use crate::theme::{color, length, theme};

/// Where the banner sits under the header, and where the unread bar goes under it.
pub(super) const CALL_BANNER_TOP: f32 = SPACE_3XS;
pub(super) const UNREAD_BAR_TOP_UNDER_CALL_BANNER: f32 =
    SPACE_3XS + COMPONENT_CALL_BANNER_HEIGHT + SPACE_2XS;

impl ConversationView {
    /// Whether the banner shows for the open conversation.
    pub(super) fn is_call_banner_shown(&self, cx: &gpui::App) -> bool {
        let Some(channel_id) = self.open_channel_id() else {
            return false;
        };
        let phase = self.session.read(cx).state().call_phase(channel_id);
        let is_in_it_with_the_view_closed = crate::voice::voice(cx).is_some_and(|voice| {
            let voice = voice.read(cx);
            voice.call_conversation_id() == Some(channel_id)
                && !voice.is_call_view_open()
                && voice.watched_stream_user_id().is_none()
        });
        phase == Some(CallPhase::Joinable) || (phase.is_some() && is_in_it_with_the_view_closed)
    }

    pub(super) fn render_call_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.is_call_banner_shown(cx) {
            return None;
        }
        let channel_id = self.open_channel_id()?;
        let call = self.session.read(cx).state().call(channel_id)?;
        let people = i64::try_from(call.participants.len()).unwrap_or(i64::MAX);
        let is_in_it = crate::voice::voice(cx)
            .is_some_and(|voice| voice.read(cx).call_conversation_id() == Some(channel_id));
        let tokens = theme(cx);
        let strings = strings(cx);
        let key = if is_in_it {
            Button::new("call-banner-show", strings.calls().show_call())
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Small)
                .on_click(|_, cx| {
                    if let Some(voice) = crate::voice::voice(cx) {
                        voice.update(cx, crate::voice::Voice::open_call_view);
                    }
                })
        } else {
            let is_joining =
                crate::calls::calls(cx).is_some_and(|calls| calls.read(cx).is_starting(channel_id));
            Button::new("call-banner-join", strings.calls().join())
                .variant(ButtonVariant::Primary)
                .size(ButtonSize::Small)
                .leading_icon(IconName::Phone, COMPONENT_CALL_MESSAGE_ICON_SIZE)
                .disabled(is_joining)
                .on_click(move |_, cx| {
                    if let Some(calls) = crate::calls::calls(cx) {
                        calls.update(cx, |calls, cx| calls.join_call(channel_id, cx));
                    }
                })
        };
        Some(
            div()
                .id("call-banner")
                .role(Role::Region)
                .aria_label(strings.calls().ongoing())
                .absolute()
                .top(length(CALL_BANNER_TOP))
                .left(length(COMPONENT_CALL_BANNER_MARGIN))
                .right(length(COMPONENT_CALL_BANNER_MARGIN))
                .h(length(COMPONENT_CALL_BANNER_HEIGHT))
                .flex()
                .items_center()
                .gap(length(SPACE_XS))
                .pl(length(COMPONENT_CALL_BANNER_PADDING_INLINE))
                .pr(length(SPACE_2XS))
                .rounded(length(COMPONENT_CALL_BANNER_RADIUS))
                .bg(color(tokens.component_call_banner_background))
                .child(icon(
                    IconName::PhoneCall,
                    COMPONENT_CALL_BANNER_ICON_SIZE,
                    color(tokens.component_call_banner_icon),
                ))
                .child(
                    with_type(div(), TYPE_BODY)
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .gap(length(SPACE_2XS))
                        .truncate()
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(color(tokens.component_call_banner_text))
                                .child(strings.calls().ongoing()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(color(tokens.component_call_banner_secondary_text))
                                .child(format!("· {}", strings.calls().people_in_call(people))),
                        ),
                )
                .child(key)
                .into_any_element(),
        )
    }
}
