//! Home, the conversations entry of the server rail (DESIGN.md §5.6, ADR-0025, ADR-0026,
//! `docs/design/desktop-conversations-and-friends.md`): the Friends space, the people picker and
//! the group conversations' dialogs. The shell draws them; [`Home`] keeps their state and asks the
//! shell to show a conversation or the Friends space through [`HomeEvent`].

mod friends_pane;
pub mod group_sheet;
mod people_picker;
#[cfg(test)]
mod tests;

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::input::{InputEvent, InputState};
use mellow_client_core::ClientError;
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::group_conversations::{
    next_owner_after_leaving, other_participants,
};
use mellow_protocol::{
    DirectMessageChannel, MAXIMUM_GROUP_CONVERSATION_NAME_LENGTH, PresenceStatus, PublicUser,
    Snowflake,
};

pub use friends_pane::{FriendsPane, FriendsPaneEvent};
pub use people_picker::{PeoplePicker, PeoplePickerEvent};

use crate::components::{ModalDialog, ModalDialogEvent, TextField};
use crate::i18n::{Strings, strings};
use crate::session::Session;
use crate::sign_in_form::describe_error;

/// A group's own name when it has one, else its people's names; a one-to-one is the other person
/// (`conversationDisplayName` in `group-conversations.ts`).
#[must_use]
pub fn conversation_display_name(
    conversation: &DirectMessageChannel,
    current_user_id: Option<Snowflake>,
    strings: Strings,
) -> String {
    let own_name = if conversation.owner_id.is_some() {
        conversation.name.trim()
    } else {
        ""
    };
    if !own_name.is_empty() {
        return own_name.to_owned();
    }
    participant_names_of(conversation, current_user_id, strings)
}

/// "Ana, Bob and 2 others" from the people in it, whatever its own name.
#[must_use]
pub fn participant_names_of(
    conversation: &DirectMessageChannel,
    current_user_id: Option<Snowflake>,
    strings: Strings,
) -> String {
    let names: Vec<&str> = other_participants(conversation, current_user_id)
        .into_iter()
        .map(display_name)
        .collect();
    strings.groups().participant_names(&names)
}

pub(crate) fn presence_name(strings: Strings, status: PresenceStatus) -> &'static str {
    let presence = strings.presence();
    match status {
        PresenceStatus::Online => presence.online(),
        PresenceStatus::Idle => presence.idle(),
        PresenceStatus::DoNotDisturb => presence.do_not_disturb(),
        PresenceStatus::Offline => presence.offline(),
    }
}

/// What Home asks the shell to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeEvent {
    /// Show this conversation, the composer focused.
    OpenConversation(Snowflake),
    /// Show the Friends space (after leaving a group, or "Add friend" in the picker), focus in it.
    ShowFriends,
}

/// A group dialog: rename, leave, or the owner removing someone.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GroupQuestion {
    Rename,
    Leave,
    Remove(PublicUser),
}

pub struct Home {
    session: Entity<Session>,
    friends_pane: Entity<FriendsPane>,
    picker: Option<(Entity<PeoplePicker>, Subscription)>,
    /// The dialog and what it listens to (its own events, the rename field's Enter).
    group_dialog: Option<(Entity<ModalDialog>, Vec<Subscription>)>,
    /// Read out by the status region: what the last group change did.
    announcement: String,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for Home {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Home")
            .field("is_picker_open", &self.picker.is_some())
            .field("is_group_dialog_open", &self.group_dialog.is_some())
            .finish_non_exhaustive()
    }
}

impl EventEmitter<HomeEvent> for Home {}

impl Home {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let friends_pane = cx.new(|cx| FriendsPane::new(session.clone(), window, cx));
        let subscriptions = vec![cx.subscribe(
            &friends_pane,
            |_, _, event: &FriendsPaneEvent, cx| match *event {
                FriendsPaneEvent::OpenConversation(conversation_id) => {
                    cx.emit(HomeEvent::OpenConversation(conversation_id));
                }
            },
        )];
        Self {
            session,
            friends_pane,
            picker: None,
            group_dialog: None,
            announcement: String::new(),
            _subscriptions: subscriptions,
        }
    }

    #[must_use]
    pub fn friends_pane(&self) -> &Entity<FriendsPane> {
        &self.friends_pane
    }

    #[cfg(test)]
    pub(crate) fn picker(&self) -> Option<&Entity<PeoplePicker>> {
        self.picker.as_ref().map(|(picker, _)| picker)
    }

    /// Whether a picker or group dialog is open over the window.
    #[must_use]
    pub fn is_dialog_open(&self) -> bool {
        self.picker.is_some() || self.group_dialog.is_some()
    }

    /// "New message" (`None`) or a group's "Add friends".
    pub fn open_people_picker(
        &mut self,
        group_id: Option<Snowflake>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_dialog_open() {
            return;
        }
        let picker = PeoplePicker::open(self.session.clone(), group_id, window, cx);
        let subscription = cx.subscribe_in(
            &picker,
            window,
            |home, _, event: &PeoplePickerEvent, window, cx| match event {
                PeoplePickerEvent::Closed => {
                    home.picker = None;
                    cx.notify();
                }
                PeoplePickerEvent::OpenConversation(conversation_id) => {
                    cx.emit(HomeEvent::OpenConversation(*conversation_id));
                }
                PeoplePickerEvent::AddFriend => {
                    home.picker = None;
                    home.friends_pane.update(cx, |pane, cx| {
                        pane.choose_tab(
                            mellow_client_core::shared_logic::friends::FriendsTab::Add,
                            true,
                            window,
                            cx,
                        );
                    });
                    cx.emit(HomeEvent::ShowFriends);
                    cx.notify();
                }
            },
        );
        self.picker = Some((picker, subscription));
        cx.notify();
    }

    pub fn ask_to_rename_group(
        &mut self,
        group_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ask(group_id, GroupQuestion::Rename, window, cx);
    }

    pub fn ask_to_leave_group(
        &mut self,
        group_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ask(group_id, GroupQuestion::Leave, window, cx);
    }

    pub fn ask_to_remove_from_group(
        &mut self,
        group_id: Snowflake,
        person: PublicUser,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ask(group_id, GroupQuestion::Remove(person), window, cx);
    }

    /// The web's `RenameGroupDialog` and `GroupConfirmDialog`: Leave and Remove follow the
    /// destructive pattern (Cancel takes focus), Rename starts in its field.
    #[allow(clippy::too_many_lines)] // Three dialogs' words, as the web's two components.
    fn ask(
        &mut self,
        group_id: Snowflake,
        question: GroupQuestion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_dialog_open() {
            return;
        }
        let catalogue = strings(cx);
        let words = catalogue.groups();
        let state = self.session.read(cx).state();
        let current_user_id = state.current_user().map(|user| user.id);
        let Some(group) = state.direct_message_channel(group_id).cloned() else {
            return;
        };
        let group_name = conversation_display_name(&group, current_user_id, catalogue);
        let cancel = catalogue.common().cancel();
        let rename_input = (question == GroupQuestion::Rename).then(|| {
            let placeholder = participant_names_of(&group, current_user_id, catalogue);
            // The name as it was when the dialog opened; a rename elsewhere does not erase typing.
            let name = group.name.clone();
            cx.new(|cx| {
                let mut input = InputState::new(window, cx)
                    .placeholder(placeholder)
                    .validate(|text, _| {
                        text.chars().count() <= MAXIMUM_GROUP_CONVERSATION_NAME_LENGTH
                    });
                input.set_value(name, window, cx);
                input
            })
        });
        let dialog = cx.new(|cx| match &question {
            GroupQuestion::Rename => {
                let body = rename_input.clone().map(|input| {
                    cx.new(|_| RenameGroupBody {
                        input,
                        label: words.name_label().into(),
                        helper: words.group_name_helper().into(),
                    })
                    .into()
                });
                let dialog = ModalDialog::new(words.rename(), cancel, window, cx).submit(
                    words.save_name(),
                    catalogue.common().saving(),
                    false,
                );
                match body {
                    Some(body) => dialog.body(body),
                    None => dialog,
                }
            }
            GroupQuestion::Leave => {
                let body = if other_participants(&group, current_user_id).is_empty() {
                    words.leave_body_as_last().to_owned()
                } else {
                    next_owner_after_leaving(&group, current_user_id).map_or_else(
                        || words.leave_body().to_owned(),
                        |next_owner| words.leave_body_as_owner(display_name(next_owner)),
                    )
                };
                ModalDialog::new(words.leave_title(&group_name), cancel, window, cx)
                    .subtitle(body)
                    .submit(words.leave(), words.leaving(), true)
            }
            GroupQuestion::Remove(person) => {
                let person = display_name(person);
                ModalDialog::new(words.remove_title(person), cancel, window, cx)
                    .subtitle(words.remove_body(person))
                    .submit(words.remove_submit(person), words.removing(), true)
            }
        });
        let mut subscriptions = Vec::new();
        if let Some(input) = &rename_input {
            input.update(cx, |input, cx| input.focus(window, cx));
            let dialog = dialog.downgrade();
            subscriptions.push(cx.subscribe(input, move |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let _ = dialog.update(cx, ModalDialog::submit_now);
                }
            }));
        }
        subscriptions.push(close_when_subject_gone(
            &self.session,
            group_id,
            &question,
            &dialog,
            window,
            cx,
        ));
        let subscription = cx.subscribe_in(
            &dialog,
            window,
            move |home, dialog, event: &ModalDialogEvent, window, cx| match event {
                ModalDialogEvent::Closed => {
                    home.group_dialog = None;
                    cx.notify();
                }
                ModalDialogEvent::Submitted => {
                    let typed = rename_input
                        .as_ref()
                        .map(|input| input.read(cx).value().trim().to_owned());
                    home.answer(group_id, &question, typed.as_deref(), dialog, window, cx);
                }
            },
        );
        subscriptions.push(subscription);
        self.group_dialog = Some((dialog, subscriptions));
        cx.notify();
    }

    /// Runs the confirmed change; the dialog stays open, busy, until the instance answers.
    fn answer(
        &mut self,
        group_id: Snowflake,
        question: &GroupQuestion,
        typed_name: Option<&str>,
        dialog: &Entity<ModalDialog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let words = strings(cx).groups();
        let state = self.session.read(cx).state();
        let current_name = state
            .direct_message_channel(group_id)
            .map(|group| group.name.trim().to_owned())
            .unwrap_or_default();
        if let Some(name) = typed_name
            && name == current_name
        {
            dialog.update(cx, |dialog, cx| dialog.request_close(window, cx));
            return;
        }
        dialog.update(cx, |dialog, cx| {
            dialog.set_error(None, cx);
            dialog.set_busy(true, cx);
        });
        let (done, failure) = match (question, typed_name) {
            (GroupQuestion::Rename, Some("")) => (
                words.name_cleared().to_owned(),
                words.rename_failed().to_owned(),
            ),
            (GroupQuestion::Rename, name) => (
                words.renamed(name.unwrap_or_default()),
                words.rename_failed().to_owned(),
            ),
            (GroupQuestion::Leave, _) => (String::new(), words.leave_failed().to_owned()),
            (GroupQuestion::Remove(person), _) => (
                words.removed(display_name(person)),
                words.remove_failed(display_name(person)),
            ),
        };
        let is_leaving = *question == GroupQuestion::Leave;
        let home = cx.entity().downgrade();
        let dialog = dialog.downgrade();
        let window_handle = window.window_handle();
        let on_done = move |answer: Result<(), ClientError>, cx: &mut App| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = dialog.update(cx, |dialog, cx| {
                    dialog.set_busy(false, cx);
                    match &answer {
                        Ok(()) => dialog.request_close(window, cx),
                        Err(error) => dialog.set_error(
                            Some(SharedString::from(describe_error(
                                error,
                                &failure,
                                crate::strings_pending_i18n::strings(cx),
                            ))),
                            cx,
                        ),
                    }
                });
                if answer.is_ok() {
                    let _ = home.update(cx, |home, cx| {
                        home.announcement = done;
                        if is_leaving {
                            // The conversation is gone: Friends shows, and focus goes there.
                            cx.emit(HomeEvent::ShowFriends);
                        }
                        cx.notify();
                    });
                }
            });
        };
        self.session.update(cx, |session, cx| match question {
            GroupQuestion::Rename => session.rename_group_conversation(
                group_id,
                typed_name.unwrap_or_default(),
                cx,
                on_done,
            ),
            GroupQuestion::Leave => session.leave_group_conversation(group_id, cx, on_done),
            GroupQuestion::Remove(person) => {
                session.remove_group_participant(group_id, person.id, cx, on_done);
            }
        });
    }
}

/// Closes a group dialog whose group, or whose person to remove, left meanwhile (the web's
/// `GroupMembersSheet`); a dialog waiting for its own answer stays (`request_close` is a no-op).
fn close_when_subject_gone(
    session: &Entity<Session>,
    group_id: Snowflake,
    question: &GroupQuestion,
    dialog: &Entity<ModalDialog>,
    window: &mut Window,
    cx: &mut Context<Home>,
) -> Subscription {
    let removed_id = match question {
        GroupQuestion::Remove(person) => Some(person.id),
        GroupQuestion::Rename | GroupQuestion::Leave => None,
    };
    let dialog = dialog.downgrade();
    cx.observe_in(session, window, move |_, session, window, cx| {
        let is_subject_present = session
            .read(cx)
            .state()
            .direct_message_channel(group_id)
            .is_some_and(|group| {
                removed_id.is_none_or(|id| group.recipients.iter().any(|user| user.id == id))
            });
        if !is_subject_present {
            let _ = dialog.update(cx, |dialog, cx| dialog.request_close(window, cx));
        }
    })
}

/// The rename dialog's field.
struct RenameGroupBody {
    input: Entity<InputState>,
    label: SharedString,
    helper: SharedString,
}

impl Render for RenameGroupBody {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        TextField::new("rename-group-name", self.label.clone(), &self.input)
            .helper(self.helper.clone())
    }
}

impl Render for Home {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let picker_dialog = self
            .picker
            .as_ref()
            .and_then(|(picker, _)| picker.read(cx).dialog().cloned());
        div()
            .id("home-layers")
            .child(
                div()
                    .id("home-announcement")
                    .role(Role::Status)
                    .aria_label(SharedString::from(self.announcement.clone()))
                    .h_0()
                    .overflow_hidden(),
            )
            .when_some(picker_dialog, gpui::ParentElement::child)
            .when_some(self.group_dialog.as_ref(), |layers, (dialog, _)| {
                layers.child(dialog.clone())
            })
    }
}
