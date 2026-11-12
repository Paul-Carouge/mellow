//! "New message" and "Add friends" (`PeoplePickerDialog.svelte`, DESIGN.md §5.6, ADR-0026): your
//! friends with a search, the ones picked as chips in the field, how many more fit, then one
//! action. One friend opens (or finds) your conversation with them; several make a new group,
//! with an optional name. In a group, the friends already in it are left out and the picked ones
//! are added, one request at a time.

use crate::components::LiveRegion as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, EventEmitter, Focusable as _,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Toggled, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::input::{Input, InputEvent, InputState};
use mellow_client_core::ClientError;
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::group_conversations::{
    friends_matching, participant_count, picker_room,
};
use mellow_protocol::{
    ApiErrorCode, MAXIMUM_GROUP_CONVERSATION_NAME_LENGTH, PublicUser, RelationshipKind, Snowflake,
};

use super::conversation_display_name;
use crate::components::{
    Avatar, AvatarPerson, Button, ButtonVariant, ModalDialog, ModalDialogEvent, TextField,
    focus_ring, icon, inset_focus_ring, is_focus_visible, remembered_focus_handle,
    remembered_hover, with_type,
};
use crate::i18n::strings;
use crate::session::Session;
use crate::shell::profile_picture;
use crate::sign_in_form::describe_error;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_BUTTON_DISABLED_OPACITY, COMPONENT_PEOPLE_PICKER_AVATAR_SIZE,
    COMPONENT_PEOPLE_PICKER_CHECKBOX_SIZE, COMPONENT_PEOPLE_PICKER_CHIP_AVATAR_SIZE,
    COMPONENT_PEOPLE_PICKER_CHIP_GAP, COMPONENT_PEOPLE_PICKER_CHIP_HEIGHT,
    COMPONENT_PEOPLE_PICKER_CHIP_PADDING_INLINE, COMPONENT_PEOPLE_PICKER_CHIP_RADIUS,
    COMPONENT_PEOPLE_PICKER_LIST_MAX_HEIGHT, COMPONENT_PEOPLE_PICKER_ROW_GAP,
    COMPONENT_PEOPLE_PICKER_ROW_HEIGHT, COMPONENT_PEOPLE_PICKER_ROW_PADDING_INLINE,
    COMPONENT_PEOPLE_PICKER_ROW_RADIUS, COMPONENT_PEOPLE_PICKER_SEARCH_MIN_WIDTH,
    COMPONENT_TEXT_FIELD_BORDER_WIDTH, COMPONENT_TEXT_FIELD_HEIGHT, COMPONENT_TEXT_FIELD_ICON_SIZE,
    COMPONENT_TEXT_FIELD_LABEL_GAP, COMPONENT_TEXT_FIELD_PADDING_INLINE,
    COMPONENT_TEXT_FIELD_RADIUS, RADIUS_XS, SPACE_2XS, SPACE_SM, TYPE_BODY, TYPE_BODY_LG,
    TYPE_CATEGORY, TYPE_MEMBER_NAME,
};
use crate::theme::{color, length, theme};

/// What the picker asks of its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeoplePickerEvent {
    /// Closed, done or not; focus is back on the opener.
    Closed,
    /// The conversation or new group to show, the composer focused.
    OpenConversation(Snowflake),
    /// "Add friend" with nobody to pick: the Friends space's Add friend tab.
    AddFriend,
}

pub struct PeoplePicker {
    session: Entity<Session>,
    /// The group to add to; `None` starts a conversation.
    group_id: Option<Snowflake>,
    dialog: Option<Entity<ModalDialog>>,
    search: Entity<InputState>,
    group_name: Entity<InputState>,
    picked_ids: Vec<Snowflake>,
    /// What the group's name field shows while empty.
    group_name_placeholder: String,
    is_busy: bool,
    subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for PeoplePicker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PeoplePicker")
            .field("group_id", &self.group_id)
            .field("picked_ids", &self.picked_ids)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<PeoplePickerEvent> for PeoplePicker {}

/// What the picker offers now.
struct Choices {
    /// Every friend, by name.
    friends: Vec<PublicUser>,
    /// Friends not in the group yet.
    candidates: Vec<PublicUser>,
    /// Picked and still candidates, in the order picked: someone who stopped being a friend, or
    /// joined meanwhile, leaves the selection by themselves.
    picked: Vec<PublicUser>,
    /// Matching the search.
    shown: Vec<PublicUser>,
    room: usize,
    is_creating_group: bool,
}

impl PeoplePicker {
    /// Opens the picker over the window: for a new conversation, or to add people to `group_id`.
    pub fn open(
        session: Entity<Session>,
        group_id: Option<Snowflake>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let picker = cx.new(|cx| Self::new(session, group_id, window, cx));
        let catalogue = strings(cx);
        let words = catalogue.groups();
        let (title, subtitle) = {
            let picker = picker.read(cx);
            let state = picker.session.read(cx).state();
            let current_user_id = state.current_user().map(|user| user.id);
            match group_id.and_then(|group_id| state.direct_message_channel(group_id)) {
                Some(group) => (
                    words.add_title(&conversation_display_name(
                        group,
                        current_user_id,
                        catalogue,
                    )),
                    words.add_subtitle(),
                ),
                None => (words.new_message().to_owned(), words.picker_subtitle()),
            }
        };
        let body = picker.clone().into();
        let dialog = cx.new(|cx| {
            ModalDialog::new(title, catalogue.common().cancel(), window, cx)
                .subtitle(subtitle)
                .body(body)
        });
        picker.update(cx, |picker, cx| {
            let subscription = cx.subscribe_in(&dialog, window, Self::follow_dialog);
            picker.subscriptions.push(subscription);
            picker.dialog = Some(dialog);
            picker.sync_dialog(cx);
            // The search takes focus, as the web's `data-initial-focus`.
            if !picker.choices(cx).candidates.is_empty() {
                picker
                    .search
                    .update(cx, |search, cx| search.focus(window, cx));
            }
        });
        picker
    }

    fn new(
        session: Entity<Session>,
        group_id: Option<Snowflake>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let words = strings(cx).groups();
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(words.search_placeholder()));
        let group_name = cx.new(|cx| {
            InputState::new(window, cx)
                .validate(|text, _| text.chars().count() <= MAXIMUM_GROUP_CONVERSATION_NAME_LENGTH)
        });
        let subscriptions = vec![
            cx.observe(&session, |picker, _, cx| {
                picker.sync_dialog(cx);
                cx.notify();
            }),
            cx.subscribe_in(
                &search,
                window,
                |picker, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => picker.pick_from_search(window, cx),
                    InputEvent::Change | InputEvent::Focus | InputEvent::Blur => cx.notify(),
                },
            ),
            cx.subscribe(&group_name, |picker, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    picker.submit(cx);
                }
            }),
        ];
        Self {
            session,
            group_id,
            dialog: None,
            search,
            group_name,
            picked_ids: Vec::new(),
            group_name_placeholder: String::new(),
            is_busy: false,
            subscriptions,
        }
    }

    #[cfg(test)]
    pub(crate) fn picked_ids(&self) -> &[Snowflake] {
        &self.picked_ids
    }

    /// The dialog holding the picker, drawn by the host.
    #[must_use]
    pub fn dialog(&self) -> Option<&Entity<ModalDialog>> {
        self.dialog.as_ref()
    }

    fn choices(&self, cx: &App) -> Choices {
        let state = self.session.read(cx).state();
        let current_user_id = state.current_user().map(|user| user.id);
        let friends: Vec<PublicUser> = state
            .relationships_of_kind(RelationshipKind::Friend)
            .into_iter()
            .map(|relationship| relationship.user.clone())
            .collect();
        let group = self
            .group_id
            .and_then(|group_id| state.direct_message_channel(group_id));
        let already_in: Vec<Snowflake> = group
            .map(|group| {
                group
                    .recipients
                    .iter()
                    .map(|recipient| recipient.id)
                    .collect()
            })
            .unwrap_or_default();
        let candidates: Vec<PublicUser> = friends
            .iter()
            .filter(|friend| !already_in.contains(&friend.id))
            .cloned()
            .collect();
        let picked: Vec<PublicUser> = self
            .picked_ids
            .iter()
            .filter_map(|user_id| {
                candidates
                    .iter()
                    .find(|candidate| candidate.id == *user_id)
                    .cloned()
            })
            .collect();
        let query = self.search.read(cx).value().to_string();
        let friend_refs: Vec<&PublicUser> = friends.iter().collect();
        let shown = friends_matching(&friend_refs, &query, &already_in)
            .into_iter()
            .cloned()
            .collect();
        let participants_already_in =
            group.map_or(1, |group| participant_count(group, current_user_id));
        let room = picker_room(participants_already_in, picked.len());
        let is_creating_group = self.group_id.is_none() && picked.len() >= 2;
        Choices {
            friends,
            candidates,
            picked,
            shown,
            room,
            is_creating_group,
        }
    }

    /// The dialog's action follows the picker: Message, Create group or Add to group, enabled
    /// once someone is picked; none while there is nobody to pick.
    fn sync_dialog(&self, cx: &mut Context<Self>) {
        let Some(dialog) = self.dialog.clone() else {
            return;
        };
        let words = strings(cx).groups();
        let choices = self.choices(cx);
        let labels = if choices.candidates.is_empty() {
            None
        } else if self.group_id.is_some() {
            Some((words.add_to_group(), words.adding()))
        } else if choices.is_creating_group {
            Some((words.create_group(), words.creating()))
        } else {
            Some((words.message(), strings(cx).members().opening()))
        };
        let is_enabled = !choices.picked.is_empty();
        let is_busy = self.is_busy;
        dialog.update(cx, |dialog, cx| {
            dialog.set_submit_labels(
                labels.map(|(label, busy)| (SharedString::from(label), SharedString::from(busy))),
                cx,
            );
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
            ModalDialogEvent::Closed => cx.emit(PeoplePickerEvent::Closed),
            ModalDialogEvent::Submitted => self.submit(cx),
        }
    }

    fn toggle_pick(&mut self, user_id: Snowflake, cx: &mut Context<Self>) {
        if let Some(position) = self.picked_ids.iter().position(|id| *id == user_id) {
            self.picked_ids.remove(position);
        } else if self.choices(cx).room > 0 {
            self.picked_ids.push(user_id);
        }
        self.sync_dialog(cx);
        cx.notify();
    }

    /// Enter after a search picks the first friend it found and clears it, so names can be typed
    /// one after the other; with nothing typed, Enter does the dialog's action.
    fn pick_from_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().trim().to_owned();
        if query.is_empty() {
            self.submit(cx);
            return;
        }
        let choices = self.choices(cx);
        let first = choices
            .shown
            .iter()
            .find(|person| !self.picked_ids.contains(&person.id));
        if let Some(first) = first.filter(|_| choices.room > 0) {
            self.picked_ids.push(first.id);
            self.search
                .update(cx, |search, cx| search.set_value("", window, cx));
            self.sync_dialog(cx);
            cx.notify();
        }
    }

    /// Backspace in an empty search takes back the last friend picked, as in Discord's field.
    fn on_search_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key != "backspace" || event.keystroke.modifiers.modified() {
            return;
        }
        if !self.search.read(cx).value().is_empty() {
            return;
        }
        let last = self.choices(cx).picked.last().map(|person| person.id);
        if let Some(last) = last {
            cx.stop_propagation();
            self.picked_ids.retain(|id| *id != last);
            self.sync_dialog(cx);
            cx.notify();
        }
    }

    fn set_error(&self, message: Option<String>, cx: &mut Context<Self>) {
        if let Some(dialog) = self.dialog.clone() {
            dialog.update(cx, |dialog, cx| {
                dialog.set_error(message.map(SharedString::from), cx);
            });
        }
    }

    fn set_busy(&mut self, is_busy: bool, cx: &mut Context<Self>) {
        self.is_busy = is_busy;
        self.sync_dialog(cx);
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let choices = self.choices(cx);
        if choices.picked.is_empty() || self.is_busy {
            return;
        }
        self.set_error(None, cx);
        self.set_busy(true, cx);
        let picker = cx.entity().downgrade();
        let words = strings(cx).groups();
        let picked_ids: Vec<Snowflake> = choices.picked.iter().map(|person| person.id).collect();
        match (self.group_id, picked_ids.as_slice()) {
            (Some(group_id), _) => self.add_next(group_id, choices.picked, Vec::new(), cx),
            (None, [only]) => {
                let failure = strings(cx).members().open_conversation_failed().to_owned();
                let only = *only;
                self.session.update(cx, |session, cx| {
                    session.open_direct_message(only, failure, cx, move |answer, cx| {
                        let _ = picker.update(cx, |picker, cx| match answer {
                            Ok(conversation_id) => picker.finish(conversation_id, cx),
                            Err(message) => {
                                picker.set_busy(false, cx);
                                picker.set_error(Some(message), cx);
                            }
                        });
                    });
                });
            }
            (None, _) => {
                let name = self.group_name.read(cx).value().to_string();
                self.session.update(cx, |session, cx| {
                    session.create_group_conversation(picked_ids, &name, cx, move |answer, cx| {
                        let _ = picker.update(cx, |picker, cx| match answer {
                            Ok(group_id) => picker.finish(group_id, cx),
                            Err(error) => {
                                let reason = matches!(
                                    error,
                                    ClientError::Api {
                                        code: ApiErrorCode::NotFound,
                                        ..
                                    }
                                )
                                .then(|| words.cannot_join());
                                picker.set_busy(false, cx);
                                picker.set_error(
                                    Some(refusal(&error, words.create_failed(), reason, cx)),
                                    cx,
                                );
                            }
                        });
                    });
                });
            }
        }
    }

    /// One request per person, in the order picked; on a refusal the rest stay picked and who
    /// joined is said in the dialog's own alert.
    fn add_next(
        &mut self,
        group_id: Snowflake,
        mut remaining: Vec<PublicUser>,
        added: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if remaining.is_empty() {
            self.finish(group_id, cx);
            return;
        }
        let person = remaining.remove(0);
        let picker = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.add_group_participant(group_id, person.id, cx, move |answer, cx| {
                let _ = picker.update(cx, |picker, cx| {
                    let mut added = added;
                    match answer {
                        Ok(()) => {
                            picker.picked_ids.retain(|id| *id != person.id);
                            added.push(display_name(&person).to_owned());
                            // The answer comes while the session is being updated: the next
                            // request waits until that update is over.
                            let picker = cx.entity().downgrade();
                            cx.defer(move |cx| {
                                let _ = picker.update(cx, |picker, cx| {
                                    picker.add_next(group_id, remaining, added, cx);
                                });
                            });
                        }
                        Err(error) => {
                            let words = strings(cx).groups();
                            let reason = match &error {
                                ClientError::Api {
                                    code: ApiErrorCode::NotFound,
                                    ..
                                } => Some(words.add_not_found()),
                                ClientError::Api {
                                    code: ApiErrorCode::LimitReached,
                                    ..
                                } => Some(words.add_full()),
                                _ => None,
                            };
                            let refusal = refusal(
                                &error,
                                &words.add_failed(display_name(&person)),
                                reason,
                                cx,
                            );
                            let names: Vec<&str> = added.iter().map(String::as_str).collect();
                            let message = if added.is_empty() {
                                refusal
                            } else {
                                format!("{} {refusal}", words.added(&names))
                            };
                            picker.set_busy(false, cx);
                            picker.set_error(Some(message), cx);
                        }
                    }
                });
            });
        });
    }

    /// Done: the dialog closes and the host shows the conversation.
    fn finish(&mut self, conversation_id: Snowflake, cx: &mut Context<Self>) {
        self.is_busy = false;
        if let Some(dialog) = self.dialog.take() {
            dialog.update(cx, |dialog, cx| dialog.set_busy(false, cx));
        }
        cx.emit(PeoplePickerEvent::OpenConversation(conversation_id));
        cx.emit(PeoplePickerEvent::Closed);
    }

    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render_choices(
        &self,
        choices: &Choices,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let words = strings(cx).groups();
        let entity = cx.entity();
        let is_search_focused = self.search.read(cx).focus_handle(cx).is_focused(window);
        let chips: Vec<AnyElement> = choices
            .picked
            .iter()
            .map(|person| {
                let id = ElementId::from(("picked-chip", person.id.raw()));
                let focus = remembered_focus_handle(&id, window, cx);
                let is_ring_shown = is_focus_visible(&focus, window);
                let user_id = person.id;
                let entity = entity.clone();
                let search = self.search.clone();
                let picture = person.avatar.map(|image| {
                    profile_picture(image, COMPONENT_PEOPLE_PICKER_CHIP_AVATAR_SIZE, window)
                });
                with_type(div(), TYPE_BODY)
                    .id(id)
                    .role(Role::Button)
                    .aria_label(words.unpick(display_name(person)))
                    .track_focus(&focus)
                    .relative()
                    .flex()
                    .items_center()
                    .gap(length(SPACE_2XS))
                    .max_w_full()
                    .h(length(COMPONENT_PEOPLE_PICKER_CHIP_HEIGHT))
                    .pl(length(COMPONENT_PEOPLE_PICKER_CHIP_PADDING_INLINE))
                    .pr(length(SPACE_2XS))
                    .rounded(length(COMPONENT_PEOPLE_PICKER_CHIP_RADIUS))
                    .bg(color(tokens.component_people_picker_chip_background))
                    .text_color(color(tokens.component_people_picker_chip_text))
                    .cursor_pointer()
                    .on_click(move |_, window, cx| {
                        entity.update(cx, |picker, cx| picker.toggle_pick(user_id, cx));
                        // The chip pressed is gone: the field keeps the keyboard in the picker.
                        search.update(cx, |search, cx| search.focus(window, cx));
                    })
                    .child(Avatar::new(
                        AvatarPerson::from_user(person, picture),
                        COMPONENT_PEOPLE_PICKER_CHIP_AVATAR_SIZE,
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(display_name(person).to_owned()),
                    )
                    .child(icon(
                        IconName::X,
                        COMPONENT_TEXT_FIELD_ICON_SIZE,
                        color(tokens.component_people_picker_chip_icon),
                    ))
                    .when(is_ring_shown, |chip| {
                        chip.child(focus_ring(COMPONENT_PEOPLE_PICKER_CHIP_RADIUS, cx))
                    })
                    .into_any_element()
            })
            .collect();
        let room_sentence = words.room_left(i64::try_from(choices.room).unwrap_or(i64::MAX));
        let rows: Vec<AnyElement> = if choices.shown.is_empty() {
            vec![
                with_type(div(), TYPE_BODY)
                    .px(length(COMPONENT_PEOPLE_PICKER_ROW_PADDING_INLINE))
                    .text_color(color(tokens.color_text_muted))
                    .child(words.no_match(self.search.read(cx).value().trim()))
                    .into_any_element(),
            ]
        } else {
            choices
                .shown
                .iter()
                .map(|person| {
                    let is_checked = self.picked_ids.contains(&person.id);
                    let is_full = choices.room == 0 && !is_checked;
                    let id = ElementId::from(("picker-row", person.id.raw()));
                    let focus = remembered_focus_handle(&id, window, cx);
                    let hover = remembered_hover(&id, window, cx);
                    let is_ring_shown = is_focus_visible(&focus, window);
                    let user_id = person.id;
                    let entity = entity.clone();
                    let name = display_name(person).to_owned();
                    let picture = person.avatar.map(|image| {
                        profile_picture(image, COMPONENT_PEOPLE_PICKER_AVATAR_SIZE, window)
                    });
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
                    div()
                        .id(id)
                        .role(Role::CheckBox)
                        .aria_label(if is_full {
                            format!("{name}, {room_sentence}")
                        } else {
                            name.clone()
                        })
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
                        .when(is_full, |row| {
                            row.opacity(COMPONENT_BUTTON_DISABLED_OPACITY)
                        })
                        .when(!is_full, |row| {
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
                            if !is_full {
                                entity.update(cx, |picker, cx| picker.toggle_pick(user_id, cx));
                            }
                        })
                        .child(Avatar::new(
                            AvatarPerson::from_user(person, picture),
                            COMPONENT_PEOPLE_PICKER_AVATAR_SIZE,
                        ))
                        .child(
                            with_type(div(), TYPE_MEMBER_NAME)
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(color(tokens.component_people_picker_name_text))
                                .child(name),
                        )
                        .child(checkbox)
                        .when(is_ring_shown, |row| {
                            row.child(inset_focus_ring(COMPONENT_PEOPLE_PICKER_ROW_RADIUS, cx))
                        })
                        .into_any_element()
                })
                .collect()
        };
        div()
            .debug_selector(|| "people-picker".to_owned())
            .flex()
            .flex_col()
            .gap(length(SPACE_SM))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_TEXT_FIELD_LABEL_GAP))
                    .child(
                        with_type(div(), TYPE_CATEGORY)
                            .text_color(color(tokens.component_text_field_label_text))
                            .child(words.search()),
                    )
                    .child(
                        // Discord's field: the friends picked sit in it as chips, before the typing.
                        div()
                            .id("picker-search")
                            .relative()
                            .min_h(length(COMPONENT_TEXT_FIELD_HEIGHT))
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(length(COMPONENT_PEOPLE_PICKER_CHIP_GAP))
                            .px(length(COMPONENT_TEXT_FIELD_PADDING_INLINE))
                            .py(length(SPACE_2XS))
                            .rounded(length(COMPONENT_TEXT_FIELD_RADIUS))
                            .border(length(COMPONENT_TEXT_FIELD_BORDER_WIDTH))
                            .border_color(color(tokens.component_text_field_border))
                            .bg(color(tokens.component_text_field_background))
                            .capture_key_down(cx.listener(Self::on_search_key_down))
                            .children(chips)
                            .child(
                                with_type(div(), TYPE_BODY_LG)
                                    .flex_1()
                                    .min_w(length(COMPONENT_PEOPLE_PICKER_SEARCH_MIN_WIDTH))
                                    .h(length(COMPONENT_PEOPLE_PICKER_CHIP_HEIGHT))
                                    .text_color(color(tokens.component_text_field_text))
                                    .child(Input::new(&self.search).appearance(false).aria_label(
                                        SharedString::from(format!(
                                            "{}, {room_sentence}",
                                            words.search()
                                        )),
                                    )),
                            )
                            .when(is_search_focused, |field| {
                                field.child(focus_ring(COMPONENT_TEXT_FIELD_RADIUS, cx))
                            }),
                    )
                    .child(
                        with_type(div(), TYPE_BODY)
                            .id("picker-room")
                            .live_polite(SharedString::from(room_sentence.clone()))
                            .text_color(color(tokens.component_people_picker_counter_text))
                            .child(room_sentence.clone()),
                    ),
            )
            .child(
                div()
                    .id("picker-list")
                    .role(Role::Group)
                    .aria_label(words.picked())
                    .max_h(length(COMPONENT_PEOPLE_PICKER_LIST_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .children(rows),
            )
            .when(choices.is_creating_group, |body| {
                body.child(
                    TextField::new("picker-group-name", words.group_name(), &self.group_name)
                        .helper(words.group_name_helper()),
                )
            })
            .into_any_element()
    }
}

/// "Couldn't create the group: …", with the picker's own reason for a refusal it knows.
fn refusal(error: &ClientError, summary: &str, reason: Option<&str>, cx: &App) -> String {
    let pending = crate::strings_pending_i18n::strings(cx);
    match reason {
        Some(reason) => (pending.error_with_reason)(summary, reason),
        None => describe_error(error, summary, pending),
    }
}

impl Render for PeoplePicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let words = strings(cx).groups();
        let choices = self.choices(cx);
        if choices.friends.is_empty() {
            // Nobody to pick yet: the way to get someone.
            let entity = cx.entity();
            return div()
                .flex()
                .flex_col()
                .items_start()
                .gap(length(SPACE_SM))
                .child(
                    with_type(div(), TYPE_BODY_LG)
                        .text_color(color(tokens.color_text_secondary))
                        .child(words.no_friends()),
                )
                .child(
                    Button::new("picker-add-friend", words.add_friend())
                        .variant(ButtonVariant::Primary)
                        .on_click(move |_, cx| {
                            entity.update(cx, |_, cx| cx.emit(PeoplePickerEvent::AddFriend));
                        }),
                )
                .into_any_element();
        }
        if choices.candidates.is_empty() {
            return with_type(div(), TYPE_BODY_LG)
                .text_color(color(tokens.color_text_secondary))
                .child(words.everyone_is_in())
                .into_any_element();
        }
        // The group's name field shows the names it would have without one.
        if choices.is_creating_group {
            let picked_names: Vec<&str> = choices.picked.iter().map(display_name).collect();
            let placeholder = words.participant_names(&picked_names);
            if placeholder != self.group_name_placeholder {
                self.group_name_placeholder.clone_from(&placeholder);
                self.group_name.update(cx, |input, cx| {
                    input.set_placeholder(placeholder, window, cx);
                });
            }
        }
        self.render_choices(&choices, window, cx)
    }
}
