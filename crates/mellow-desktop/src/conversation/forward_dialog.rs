//! "Forward message" (`ForwardMessageDialog.svelte`, DESIGN.md §5.4, ADR-0039): the places the
//! reader may post in, searchable, up to five; the forward as it will arrive; an optional message
//! sent after it in each place. Each place gets its own requests: those that worked leave the
//! selection, those that failed stay picked with their reason, so trying again only sends what is
//! missing. Places, search, order and nonces are client-core's (`forward_messages`).

use std::collections::HashMap;

use gpui::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, EventEmitter,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Toggled, WeakEntity, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::input::{InputEvent, InputState};
use mellow_client_core::shared_logic::forward_messages::{
    ForwardAttempts, ForwardDestination, ForwardDestinationKind, ForwardDestinationSources,
    MAXIMUM_FORWARD_DESTINATIONS, destinations_matching, forward_destinations, in_opening_order,
};
use mellow_client_core::shared_logic::message_length::MAXIMUM_MESSAGE_LENGTH;
use mellow_client_core::shared_logic::role_mentions::message_as_plain_text;
use mellow_client_core::{ClientError, generate_message_nonce};
use mellow_protocol::{ApiErrorCode, ForwardOrigin, Message, Role as ServerRole, Snowflake};

use crate::components::{
    Avatar, AvatarPerson, GroupAvatar, LiveRegion as _, ModalDialog, ModalDialogEvent, TextField,
    icon, inset_focus_ring, is_focus_visible, remembered_focus_handle, remembered_hover, with_type,
};
use crate::home::conversation_display_name;
use crate::i18n::{Strings, strings};
use crate::session::Session;
use crate::shell::profile_picture;
use crate::sign_in_form::{describe_error, error_reason};
use crate::strings_pending_i18n;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_BUTTON_DISABLED_OPACITY, COMPONENT_DIALOG_ERROR_GAP,
    COMPONENT_DIALOG_ERROR_ICON_SIZE, COMPONENT_DIALOG_PREVIEW_BODY_MAX_HEIGHT,
    COMPONENT_MESSAGE_QUOTE_BAR_WIDTH, COMPONENT_MESSAGE_STATUS_ICON_SIZE,
    COMPONENT_PEOPLE_PICKER_AVATAR_SIZE, COMPONENT_PEOPLE_PICKER_CHECKBOX_SIZE,
    COMPONENT_PEOPLE_PICKER_LIST_MAX_HEIGHT, COMPONENT_PEOPLE_PICKER_ROW_GAP,
    COMPONENT_PEOPLE_PICKER_ROW_HEIGHT, COMPONENT_PEOPLE_PICKER_ROW_PADDING_INLINE,
    COMPONENT_PEOPLE_PICKER_ROW_RADIUS, COMPONENT_TEXT_FIELD_BORDER_WIDTH,
    COMPONENT_TEXT_FIELD_ICON_SIZE, RADIUS_MD, RADIUS_XS, SPACE_2XS, SPACE_SM, TYPE_BODY,
    TYPE_BODY_LG, TYPE_BODY_SM, TYPE_MEMBER_NAME, TYPE_MESSAGE, TYPE_MESSAGE_TIMESTAMP,
};
use crate::theme::{color, length, theme};

/// What the dialog asks of its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardDialogEvent {
    /// Closed without forwarding everything; focus is back on the opener.
    Closed,
    /// Every place got the forward: "Forwarded to #general and Ana.", for the chat's status line.
    Forwarded(String),
}

/// One try, while its answers come back.
struct Round {
    /// The places tried, in the list's order.
    targets: Vec<ForwardDestination>,
    /// A place's refusal: the sentence for the alert and the reason for its row.
    refusals: HashMap<Snowflake, (String, String)>,
    remaining: usize,
}

pub struct ForwardDialog {
    session: Entity<Session>,
    message: Message,
    /// The channel on screen: a read place elsewhere stays read after the forward lands.
    open_channel_id: Option<Snowflake>,
    roles: HashMap<Snowflake, ServerRole>,
    dialog: WeakEntity<ModalDialog>,
    search: Entity<InputState>,
    note: Entity<InputState>,
    picked_ids: Vec<Snowflake>,
    /// The rows' order when the dialog opened: a forward sent must not reshuffle them.
    order_at_opening: Vec<Snowflake>,
    /// Why a place refused, shown on its row until the next try.
    refusal_by_channel_id: HashMap<Snowflake, String>,
    /// The places reached when others refused, said apart from the failures.
    partial_success: Option<String>,
    attempts: ForwardAttempts,
    round: Option<Round>,
    subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for ForwardDialog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ForwardDialog")
            .field("message", &self.message.id)
            .field("picked_ids", &self.picked_ids)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<ForwardDialogEvent> for ForwardDialog {}

impl ForwardDialog {
    /// Opens over the window; returns the dialog the host draws and the body that drives it.
    pub fn open(
        session: Entity<Session>,
        message: Message,
        open_channel_id: Option<Snowflake>,
        roles: HashMap<Snowflake, ServerRole>,
        window: &mut Window,
        cx: &mut App,
    ) -> (Entity<ModalDialog>, Entity<Self>) {
        let body = cx.new(|cx| Self::new(session, message, open_channel_id, roles, window, cx));
        let words = strings(cx).forward();
        let subtitle = words.subtitle(i64::try_from(MAXIMUM_FORWARD_DESTINATIONS).unwrap_or(0));
        let view = body.clone().into();
        let dialog = cx.new(|cx| {
            ModalDialog::new(words.title(), strings(cx).common().cancel(), window, cx)
                .subtitle(subtitle)
                .body(view)
        });
        body.update(cx, |body, cx| {
            let subscription = cx.subscribe_in(&dialog, window, Self::follow_dialog);
            body.subscriptions.push(subscription);
            body.dialog = dialog.downgrade();
            body.sync_dialog(cx);
            // The search takes focus, as the web's `data-initial-focus`.
            if !body.destinations(cx).is_empty() {
                body.search
                    .update(cx, |search, cx| search.focus(window, cx));
            }
        });
        (dialog, body)
    }

    fn new(
        session: Entity<Session>,
        message: Message,
        open_channel_id: Option<Snowflake>,
        roles: HashMap<Snowflake, ServerRole>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let words = strings(cx).forward();
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(words.search_placeholder()));
        let note = cx.new(|cx| {
            InputState::new(window, cx)
                .validate(|text, _| text.chars().count() <= MAXIMUM_MESSAGE_LENGTH)
        });
        let subscriptions = vec![
            cx.observe(&session, |dialog, _, cx| {
                dialog.sync_dialog(cx);
                cx.notify();
            }),
            cx.subscribe_in(
                &search,
                window,
                |dialog, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => dialog.pick_from_search(window, cx),
                    InputEvent::Change | InputEvent::Focus | InputEvent::Blur => cx.notify(),
                },
            ),
            cx.subscribe(&note, |dialog, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    dialog.submit(cx);
                }
            }),
        ];
        let mut dialog = Self {
            session,
            message,
            open_channel_id,
            roles,
            dialog: WeakEntity::new_invalid(),
            search,
            note,
            picked_ids: Vec::new(),
            order_at_opening: Vec::new(),
            refusal_by_channel_id: HashMap::new(),
            partial_success: None,
            attempts: ForwardAttempts::default(),
            round: None,
            subscriptions,
        };
        dialog.order_at_opening = dialog
            .destinations(cx)
            .iter()
            .map(|destination| destination.channel_id)
            .collect();
        dialog
    }

    #[cfg(test)]
    pub(crate) fn picked_ids(&self) -> &[Snowflake] {
        &self.picked_ids
    }

    #[cfg(test)]
    pub(crate) fn refusal_of(&self, channel_id: Snowflake) -> Option<&str> {
        self.refusal_by_channel_id
            .get(&channel_id)
            .map(String::as_str)
    }

    pub(crate) fn is_busy(&self) -> bool {
        self.round.is_some()
    }

    /// The places offered now, in the order they had when the dialog opened.
    fn destinations(&self, cx: &App) -> Vec<ForwardDestination> {
        let state = self.session.read(cx).state();
        let Some(current_user_id) = state.current_user().map(|user| user.id) else {
            return Vec::new();
        };
        let catalogue = strings(cx);
        let found = forward_destinations(&ForwardDestinationSources {
            guilds: state.guilds(),
            conversations: state.direct_message_channels(),
            current_user_id,
            is_blocked: &|user_id| state.is_blocked(user_id),
            has_attachments: !self.message.attachments.is_empty(),
            conversation_name: &|conversation| {
                conversation_display_name(conversation, Some(current_user_id), catalogue)
            },
            group_word: catalogue.groups().group(),
        });
        in_opening_order(found, &self.order_at_opening)
    }

    /// Picked and still offered, in the list's order: a place that went away leaves by itself.
    fn picked(&self, destinations: &[ForwardDestination]) -> Vec<ForwardDestination> {
        destinations
            .iter()
            .filter(|destination| self.picked_ids.contains(&destination.channel_id))
            .cloned()
            .collect()
    }

    fn room(&self, destinations: &[ForwardDestination]) -> usize {
        MAXIMUM_FORWARD_DESTINATIONS.saturating_sub(self.picked(destinations).len())
    }

    /// Forward, enabled once a place is picked; Cancel alone when there is nowhere to send.
    fn sync_dialog(&self, cx: &mut Context<Self>) {
        let Some(dialog) = self.dialog.upgrade() else {
            return;
        };
        let words = strings(cx).forward();
        let destinations = self.destinations(cx);
        let labels = (!destinations.is_empty()).then(|| {
            (
                SharedString::from(words.submit()),
                SharedString::from(words.submitting()),
            )
        });
        let is_enabled = !self.picked(&destinations).is_empty();
        let is_busy = self.is_busy();
        dialog.update(cx, |dialog, cx| {
            dialog.set_submit_labels(labels, cx);
            dialog.set_submit_enabled(is_enabled, cx);
            dialog.set_busy(is_busy, cx);
        });
    }

    #[allow(clippy::trivially_copy_pass_by_ref)] // GPUI hands events by reference.
    fn follow_dialog(
        &mut self,
        _: &Entity<ModalDialog>,
        event: &ModalDialogEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match *event {
            ModalDialogEvent::Closed => cx.emit(ForwardDialogEvent::Closed),
            ModalDialogEvent::Submitted => self.submit(cx),
        }
    }

    fn toggle_pick(&mut self, channel_id: Snowflake, cx: &mut Context<Self>) {
        // The places being sent to stay as they are until the answers are in.
        if self.is_busy() {
            return;
        }
        if let Some(position) = self.picked_ids.iter().position(|id| *id == channel_id) {
            self.picked_ids.remove(position);
        } else if self.room(&self.destinations(cx)) > 0 {
            self.picked_ids.push(channel_id);
        }
        self.sync_dialog(cx);
        cx.notify();
    }

    /// Enter after a search picks the first place it found and clears it; with nothing typed,
    /// Enter forwards.
    fn pick_from_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().trim().to_owned();
        if query.is_empty() {
            self.submit(cx);
            return;
        }
        if self.is_busy() {
            return;
        }
        let destinations = self.destinations(cx);
        let first = destinations_matching(&destinations, &query)
            .into_iter()
            .find(|destination| !self.picked_ids.contains(&destination.channel_id))
            .map(|destination| destination.channel_id);
        if let Some(first) = first.filter(|_| self.room(&destinations) > 0) {
            self.picked_ids.push(first);
            self.search
                .update(cx, |search, cx| search.set_value("", window, cx));
            self.sync_dialog(cx);
            cx.notify();
        }
    }

    fn set_error(&self, message: Option<String>, cx: &mut Context<Self>) {
        if let Some(dialog) = self.dialog.upgrade() {
            dialog.update(cx, |dialog, cx| {
                dialog.set_error(message.map(SharedString::from), cx);
            });
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let targets = self.picked(&self.destinations(cx));
        if targets.is_empty() || self.is_busy() {
            return;
        }
        self.set_error(None, cx);
        self.partial_success = None;
        let note = self.note.read(cx).value().trim().to_owned();
        self.round = Some(Round {
            remaining: targets.len(),
            targets: targets.clone(),
            refusals: HashMap::new(),
        });
        self.sync_dialog(cx);
        // Each place on its own and in parallel: one refusing does not hold the others back.
        for destination in targets {
            self.deliver(destination, note.clone(), cx);
        }
        cx.notify();
    }

    /// The forward, then the note when there is one; a place whose forward landed in an earlier
    /// try only gets its note.
    fn deliver(&mut self, destination: ForwardDestination, note: String, cx: &mut Context<Self>) {
        let channel_id = destination.channel_id;
        let nonces = match self.attempts.nonces_for(channel_id, generate_message_nonce) {
            Ok(nonces) => nonces,
            Err(error) => {
                self.refuse_forward(&destination, &error, cx);
                return;
            }
        };
        if self.attempts.is_forwarded(channel_id) {
            self.send_note(destination, note, nonces.note, cx);
            return;
        }
        let origin = ForwardOrigin {
            // The instance keeps a forward's first origin, so a forward of a forward names itself.
            channel_id: self.message.channel_id,
            message_id: self.message.id,
        };
        let dialog = cx.entity().downgrade();
        let open_channel_id = self.open_channel_id;
        self.session.update(cx, |session, cx| {
            session.forward_message(
                channel_id,
                origin,
                nonces.forward,
                open_channel_id,
                cx,
                move |answer, cx| {
                    let _ = dialog.update(cx, |dialog, cx| match answer {
                        Ok(()) => {
                            dialog.attempts.note_forwarded(channel_id);
                            dialog.send_note(destination, note, nonces.note, cx);
                        }
                        Err(error) => dialog.refuse_forward(&destination, &error, cx),
                    });
                },
            );
        });
    }

    fn send_note(
        &mut self,
        destination: ForwardDestination,
        note: String,
        nonce: String,
        cx: &mut Context<Self>,
    ) {
        if note.is_empty() {
            self.finish_one(destination.channel_id, None, cx);
            return;
        }
        let dialog = cx.entity().downgrade();
        let open_channel_id = self.open_channel_id;
        self.session.update(cx, |session, cx| {
            session.send_message_to(
                destination.channel_id,
                note,
                nonce,
                open_channel_id,
                cx,
                move |answer, cx| {
                    let _ = dialog.update(cx, |dialog, cx| {
                        let refusal = answer.err().map(|error| {
                            let pending = strings_pending_i18n::strings(cx);
                            let summary = strings(cx).forward().note_failed(&destination.label());
                            (
                                describe_error(&error, &summary, pending),
                                error_reason(&error, pending).to_owned(),
                            )
                        });
                        dialog.finish_one(destination.channel_id, refusal, cx);
                    });
                },
            );
        });
    }

    fn refuse_forward(
        &mut self,
        destination: &ForwardDestination,
        error: &ClientError,
        cx: &mut Context<Self>,
    ) {
        let catalogue = strings(cx);
        let reason = forward_refusal_reason(error, catalogue, strings_pending_i18n::strings(cx));
        let sentence = catalogue.errors().with_reason(
            &catalogue.forward().forward_failed(&destination.label()),
            &reason,
        );
        self.finish_one(destination.channel_id, Some((sentence, reason)), cx);
    }

    /// One place answered; once all did, the dialog closes or says what is missing.
    fn finish_one(
        &mut self,
        channel_id: Snowflake,
        refusal: Option<(String, String)>,
        cx: &mut Context<Self>,
    ) {
        let Some(round) = self.round.as_mut() else {
            return;
        };
        if let Some(refusal) = refusal {
            round.refusals.insert(channel_id, refusal);
        }
        round.remaining = round.remaining.saturating_sub(1);
        if round.remaining > 0 {
            return;
        }
        let Some(round) = self.round.take() else {
            return;
        };
        let words = strings(cx).forward();
        let reached: Vec<String> = round
            .targets
            .iter()
            .filter(|destination| !round.refusals.contains_key(&destination.channel_id))
            .map(ForwardDestination::label)
            .collect();
        let reached: Vec<&str> = reached.iter().map(String::as_str).collect();
        if round.refusals.is_empty() {
            self.sync_dialog(cx);
            cx.emit(ForwardDialogEvent::Forwarded(words.sent(&reached)));
            return;
        }
        // What worked leaves the selection, so trying again only sends to the places that refused.
        let refused: Vec<&ForwardDestination> = round
            .targets
            .iter()
            .filter(|destination| round.refusals.contains_key(&destination.channel_id))
            .collect();
        self.picked_ids = refused
            .iter()
            .map(|destination| destination.channel_id)
            .collect();
        self.refusal_by_channel_id = round
            .refusals
            .iter()
            .map(|(channel_id, (_, reason))| (*channel_id, reason.clone()))
            .collect();
        self.partial_success = (!reached.is_empty()).then(|| words.sent(&reached));
        let sentences: Vec<&str> = refused
            .iter()
            .filter_map(|destination| round.refusals.get(&destination.channel_id))
            .map(|(sentence, _)| sentence.as_str())
            .collect();
        self.set_error(Some(sentences.join(" ")), cx);
        self.sync_dialog(cx);
        cx.notify();
    }

    #[allow(clippy::too_many_lines)] // One row: its leading picture, words and box.
    fn render_row(
        &self,
        destination: &ForwardDestination,
        room: usize,
        room_sentence: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let is_checked = self.picked_ids.contains(&destination.channel_id);
        let is_disabled = (room == 0 && !is_checked) || self.is_busy();
        let id = ElementId::from(("forward-row", destination.channel_id.raw()));
        let focus = remembered_focus_handle(&id, window, cx);
        let hover = remembered_hover(&id, window, cx);
        let is_row_active = *hover.read(cx) || focus.is_focused(window);
        let channel_id = destination.channel_id;
        let entity = cx.entity();
        let label = destination.label();
        let refusal = self.refusal_by_channel_id.get(&destination.channel_id);
        let leading = match destination.kind {
            // A place, square like a server in the strip; people stay round.
            ForwardDestinationKind::Channel => div()
                .flex_none()
                .size(length(COMPONENT_PEOPLE_PICKER_AVATAR_SIZE))
                .flex()
                .items_center()
                .justify_center()
                .rounded(length(RADIUS_MD))
                .bg(color(tokens.color_neutral_fill))
                .child(icon(
                    IconName::Hash,
                    COMPONENT_TEXT_FIELD_ICON_SIZE,
                    color(tokens.color_text_on_neutral),
                ))
                .into_any_element(),
            ForwardDestinationKind::Group => GroupAvatar::new(
                destination
                    .people
                    .iter()
                    .map(|person| {
                        let picture = person.avatar.map(|image| {
                            profile_picture(
                                image,
                                COMPONENT_PEOPLE_PICKER_AVATAR_SIZE,
                                is_row_active,
                                window,
                                cx,
                            )
                        });
                        AvatarPerson::from_user(person, picture)
                    })
                    .collect(),
                COMPONENT_PEOPLE_PICKER_AVATAR_SIZE,
                color(tokens.component_dialog_background),
            )
            .into_any_element(),
            ForwardDestinationKind::Person => match destination.people.first() {
                Some(person) => {
                    let picture = person.avatar.map(|image| {
                        profile_picture(
                            image,
                            COMPONENT_PEOPLE_PICKER_AVATAR_SIZE,
                            is_row_active,
                            window,
                            cx,
                        )
                    });
                    Avatar::new(
                        AvatarPerson::from_user(person, picture),
                        COMPONENT_PEOPLE_PICKER_AVATAR_SIZE,
                    )
                    .into_any_element()
                }
                None => div().into_any_element(),
            },
        };
        let second_line = match refusal {
            Some(reason) => {
                let error_text = color(tokens.component_dialog_error_text);
                with_type(div(), TYPE_BODY_SM)
                    .flex()
                    .items_center()
                    .gap(length(SPACE_2XS))
                    .text_color(error_text)
                    .child(icon(
                        IconName::WarningCircle,
                        COMPONENT_MESSAGE_STATUS_ICON_SIZE,
                        error_text,
                    ))
                    .child(div().min_w_0().child(capitalised(reason)))
                    .into_any_element()
            }
            None => with_type(div(), TYPE_BODY_SM)
                .truncate()
                .text_color(color(tokens.component_people_picker_username_text))
                .child(destination.place.clone())
                .into_any_element(),
        };
        let checkbox = div()
            .flex_none()
            .size(length(COMPONENT_PEOPLE_PICKER_CHECKBOX_SIZE))
            .flex()
            .items_center()
            .justify_center()
            .rounded(length(RADIUS_XS))
            .border(length(COMPONENT_TEXT_FIELD_BORDER_WIDTH))
            .border_color(color(if is_checked {
                tokens.component_people_picker_checkbox_color
            } else {
                tokens.component_text_field_border
            }))
            .when(is_checked, |checkbox| {
                checkbox
                    .bg(color(tokens.component_people_picker_checkbox_color))
                    .child(icon(
                        IconName::Check,
                        COMPONENT_PEOPLE_PICKER_CHECKBOX_SIZE,
                        color(tokens.component_button_primary_text),
                    ))
            });
        let accessible_name = match refusal {
            Some(reason) => format!("{label}, {}, {reason}", destination.place),
            None if room == 0 && !is_checked => {
                format!("{label}, {}, {room_sentence}", destination.place)
            }
            None => format!("{label}, {}", destination.place),
        };
        div()
            .id(id)
            .role(Role::CheckBox)
            .aria_label(accessible_name)
            .aria_toggled(if is_checked {
                Toggled::True
            } else {
                Toggled::False
            })
            .track_focus(&focus)
            .relative()
            .mt(length(COMPONENT_PEOPLE_PICKER_ROW_GAP))
            .min_h(length(COMPONENT_PEOPLE_PICKER_ROW_HEIGHT))
            .px(length(COMPONENT_PEOPLE_PICKER_ROW_PADDING_INLINE))
            .flex()
            .items_center()
            .gap(length(SPACE_SM))
            .rounded(length(COMPONENT_PEOPLE_PICKER_ROW_RADIUS))
            .when(is_disabled, |row| {
                row.opacity(COMPONENT_BUTTON_DISABLED_OPACITY)
            })
            .when(!is_disabled, |row| {
                row.cursor_pointer().when(*hover.read(cx), |row| {
                    row.bg(color(tokens.component_people_picker_row_background_hover))
                })
            })
            .on_hover(move |is_hovered, _, cx| {
                hover.update(cx, |remembered, cx| {
                    *remembered = *is_hovered;
                    cx.notify();
                });
            })
            .on_click(move |_, _, cx| {
                if !is_disabled {
                    entity.update(cx, |dialog, cx| dialog.toggle_pick(channel_id, cx));
                }
            })
            .child(leading)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        with_type(div(), TYPE_MEMBER_NAME)
                            .truncate()
                            .text_color(color(tokens.component_people_picker_name_text))
                            .child(label),
                    )
                    .child(second_line),
            )
            .child(checkbox)
            .when(is_focus_visible(&focus, window), |row| {
                row.child(inset_focus_ring(COMPONENT_PEOPLE_PICKER_ROW_RADIUS, cx))
            })
            .into_any_element()
    }

    /// The forward as it will arrive: the bar and the mark, never the original's author (the
    /// forwarder sends it).
    fn render_preview(&self, cx: &App) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let muted = color(tokens.color_text_muted);
        let text = message_as_plain_text(
            &self.message.content,
            &self.roles,
            catalogue.messages().deleted_role_name(),
        );
        let attachment_count = self.message.attachments.len();
        div()
            .id("forward-preview")
            .role(Role::Group)
            .aria_label(catalogue.forward().preview())
            .flex()
            .flex_col()
            .gap(length(SPACE_2XS))
            .border_l(length(COMPONENT_MESSAGE_QUOTE_BAR_WIDTH))
            .border_color(color(tokens.component_message_quote_bar_color))
            .pl(length(SPACE_SM))
            .child(
                with_type(div(), TYPE_MESSAGE_TIMESTAMP)
                    .flex()
                    .items_center()
                    .gap(length(SPACE_2XS))
                    .text_color(muted)
                    .child(icon(
                        IconName::ArrowBendUpRight,
                        COMPONENT_MESSAGE_STATUS_ICON_SIZE,
                        muted,
                    ))
                    .child(catalogue.forward().forwarded()),
            )
            .when(!text.trim().is_empty(), |preview| {
                preview.child(
                    with_type(div(), TYPE_MESSAGE)
                        .max_h(length(COMPONENT_DIALOG_PREVIEW_BODY_MAX_HEIGHT))
                        .overflow_hidden()
                        .text_color(color(tokens.component_message_body_text))
                        .child(text),
                )
            })
            .when(attachment_count > 0, |preview| {
                let secondary = color(tokens.color_text_secondary);
                preview.child(
                    with_type(div(), TYPE_BODY)
                        .flex()
                        .items_center()
                        .gap(length(SPACE_2XS))
                        .text_color(secondary)
                        .child(icon(
                            IconName::File,
                            COMPONENT_MESSAGE_STATUS_ICON_SIZE,
                            secondary,
                        ))
                        .child(
                            catalogue.messages().attachment_count(
                                i64::try_from(attachment_count).unwrap_or(i64::MAX),
                            ),
                        ),
                )
            })
            .into_any_element()
    }
}

/// A forward's own refusals read better than the generic ones (`forwardRefusalReason`).
fn forward_refusal_reason(
    error: &ClientError,
    catalogue: Strings,
    pending: &strings_pending_i18n::Strings,
) -> String {
    match error {
        ClientError::Api {
            code: ApiErrorCode::NotFound,
            ..
        } => catalogue.forward().origin_gone().to_owned(),
        ClientError::Api {
            code: ApiErrorCode::MissingPermissions,
            ..
        } => catalogue.forward().cannot_send_there().to_owned(),
        _ => error_reason(error, pending).to_owned(),
    }
}

/// A reason starts lower case to follow a colon; alone on a row it starts upper case.
fn capitalised(reason: &str) -> String {
    let mut characters = reason.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().chain(characters).collect())
        .unwrap_or_default()
}

impl Render for ForwardDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let words = strings(cx).forward();
        let destinations = self.destinations(cx);
        if destinations.is_empty() {
            return with_type(div(), TYPE_BODY_LG)
                .text_color(color(tokens.color_text_secondary))
                .child(words.no_places())
                .into_any_element();
        }
        let room = self.room(&destinations);
        let room_sentence = words.room_left(i64::try_from(room).unwrap_or(0));
        let query = self.search.read(cx).value().trim().to_owned();
        let shown = destinations_matching(&destinations, &query);
        let rows: Vec<AnyElement> = if shown.is_empty() {
            vec![
                with_type(div(), TYPE_BODY)
                    .px(length(COMPONENT_PEOPLE_PICKER_ROW_PADDING_INLINE))
                    .text_color(color(tokens.color_text_muted))
                    .child(words.no_match(&query))
                    .into_any_element(),
            ]
        } else {
            shown
                .into_iter()
                .map(|destination| self.render_row(destination, room, &room_sentence, window, cx))
                .collect()
        };
        let is_busy = self.is_busy();
        div()
            .debug_selector(|| "forward-dialog".to_owned())
            .flex()
            .flex_col()
            .gap(length(SPACE_SM))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(length(SPACE_2XS))
                    .child(
                        TextField::new("forward-search", words.search(), &self.search)
                            .leading_icon(IconName::MagnifyingGlass),
                    )
                    .child(
                        with_type(div(), TYPE_BODY)
                            .id("forward-room")
                            .live_polite(SharedString::from(room_sentence.clone()))
                            .text_color(color(tokens.component_people_picker_counter_text))
                            .child(room_sentence),
                    ),
            )
            .child(
                div()
                    .id("forward-list")
                    .role(Role::Group)
                    .aria_label(words.places())
                    .max_h(length(COMPONENT_PEOPLE_PICKER_LIST_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .children(rows),
            )
            .child(self.render_preview(cx))
            .child(
                TextField::new("forward-note", words.note(), &self.note)
                    .helper(words.note_helper())
                    .read_only(is_busy),
            )
            .children(self.partial_success.clone().map(|sentence| {
                let secondary = color(tokens.color_text_secondary);
                with_type(div(), TYPE_BODY)
                    .id("forward-partial-success")
                    .live_polite(SharedString::from(sentence.clone()))
                    .flex()
                    .gap(length(COMPONENT_DIALOG_ERROR_GAP))
                    .text_color(secondary)
                    .child(icon(
                        IconName::Check,
                        COMPONENT_DIALOG_ERROR_ICON_SIZE,
                        secondary,
                    ))
                    .child(sentence)
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reason_alone_on_a_row_starts_with_a_capital() {
        assert_eq!(
            capitalised("you can't send messages there."),
            "You can't send messages there."
        );
        assert_eq!(capitalised("été"), "Été");
        assert_eq!(capitalised(""), "");
    }

    #[test]
    fn a_forwards_own_refusals_read_better_than_the_generic_ones() {
        let english = Strings::new(crate::i18n::Language::English);
        let pending = &strings_pending_i18n::ENGLISH;
        let refusal = |code| ClientError::Api {
            status: 400,
            code,
            message: String::new(),
        };
        assert_eq!(
            forward_refusal_reason(&refusal(ApiErrorCode::NotFound), english, pending),
            english.forward().origin_gone()
        );
        assert_eq!(
            forward_refusal_reason(&refusal(ApiErrorCode::MissingPermissions), english, pending),
            english.forward().cannot_send_there()
        );
        assert_eq!(
            forward_refusal_reason(&ClientError::NotSignedIn, english, pending),
            pending.error_unknown_reason
        );
    }
}
