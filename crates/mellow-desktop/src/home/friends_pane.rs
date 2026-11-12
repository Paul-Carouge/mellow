//! The Friends space (`FriendsPane.svelte`, `AddFriendForm.svelte`, `FriendCode.svelte`,
//! `RelationshipConfirmDialog.svelte`, DESIGN.md §5.6.1, ADR-0025): Home's page when no
//! conversation is open. Tabs Online, All, Pending, Blocked and Add friend, as on Discord; each row
//! has its actions as round buttons. The shell draws the tab list ([`FriendsPane::tab_list`]) in
//! its header, or under it in the drawer layout; this view draws the panel.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, Context, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable as _, InteractiveElement as _, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, ParentElement as _, Pixels, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, canvas, div,
    prelude::FluentBuilder as _,
};
use gpui_component::input::{Input, InputEvent, InputState};
use mellow_client_core::ClientError;
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::friends::{
    FriendRequestOutcome, FriendsGroupKind, FriendsTab, friend_request_outcome, friends_groups_in,
};
use mellow_protocol::{
    ApiErrorCode, PresenceStatus, PublicUser, Relationship, RelationshipKind, SendFriendRequest,
    SetRelationshipKind, Snowflake,
};

use super::{PersonMenuRequest, presence_name, shown_presence};
use crate::components::{
    Avatar, AvatarPerson, Button, ButtonVariant, ContextMenu, FriendCode, FriendCodeCopyState,
    MentionBadge, MenuAlignment, MenuAnchor, MenuEntry, MenuItem, MenuItemTone, MenuOrigin,
    ModalDialog, ModalDialogEvent, Tooltip, bounds_recorder, focus_ring, icon,
    is_context_menu_keystroke, is_focus_visible, menu_origin_at_pointer,
    menu_origin_beside_remembered, next_tab_index, remembered_bounds, remembered_focus_handle,
    remembered_hover, with_type,
};
use crate::i18n::{Strings, strings};
use crate::session::Session;
use crate::shell::profile_picture;
use crate::sign_in_form::describe_error;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_BUTTON_DISABLED_OPACITY, COMPONENT_FRIENDS_ACTION_GAP,
    COMPONENT_FRIENDS_ACTION_ICON_SIZE, COMPONENT_FRIENDS_ACTION_SIZE,
    COMPONENT_FRIENDS_AVATAR_SIZE, COMPONENT_FRIENDS_CONTENT_MAX_WIDTH,
    COMPONENT_FRIENDS_EMPTY_GAP, COMPONENT_FRIENDS_GROUP_HEIGHT,
    COMPONENT_FRIENDS_LIST_PADDING_INLINE, COMPONENT_FRIENDS_RESULT_ICON_SIZE,
    COMPONENT_FRIENDS_ROW_GAP, COMPONENT_FRIENDS_ROW_HEIGHT, COMPONENT_FRIENDS_ROW_PADDING_INLINE,
    COMPONENT_FRIENDS_ROW_RADIUS, COMPONENT_FRIENDS_TAB_GAP, COMPONENT_FRIENDS_TAB_HEIGHT,
    COMPONENT_FRIENDS_TAB_PADDING_INLINE, COMPONENT_FRIENDS_TAB_RADIUS,
    COMPONENT_FRIENDS_USERNAME_MIN_WIDTH, COMPONENT_TEXT_FIELD_BORDER_WIDTH,
    COMPONENT_TEXT_FIELD_PADDING_INLINE, COMPONENT_TEXT_FIELD_RADIUS, SPACE_2XS, SPACE_LG,
    SPACE_MD, SPACE_SM, SPACE_XL, SPACE_XS, TYPE_BODY, TYPE_BODY_LG, TYPE_BODY_SM, TYPE_BUTTON,
    TYPE_CATEGORY, TYPE_HEADING_LG, TYPE_MEMBER_NAME,
};
use crate::theme::{color, length, theme};

/// What the pane asks of its host.
#[derive(Debug, Clone, PartialEq)]
pub enum FriendsPaneEvent {
    /// Message opened (or found) this conversation: it shows, the composer focused.
    OpenConversation(Snowflake),
    /// A right-click, Shift+F10 or the Menu key on a row: the person's menu.
    OpenPersonMenu(PersonMenuRequest),
}

/// How an "Add friend" form looks the person up (ADR-0048).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lookup {
    Name,
    FriendCode,
}

struct AddFriendForm {
    input: Entity<InputState>,
    is_sending: bool,
    /// The answer said under the field: whether it worked, and in words.
    result: Option<(bool, String)>,
}

/// A row action's look under the pointer (`actionClass`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionTone {
    Neutral,
    Positive,
    Danger,
}

/// What a confirmed dialog does to a friend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Confirmed {
    Remove,
    Block,
}

pub struct FriendsPane {
    session: Entity<Session>,
    tab: FriendsTab,
    /// One per tab of `FriendsTab::ALL`; only the chosen one is a Tab stop (roving).
    tab_focus: Vec<FocusHandle>,
    /// The people whose rows wait for the instance: their actions dim, the others stay usable.
    busy_user_ids: HashSet<Snowflake>,
    action_error: Option<String>,
    /// Read out by the status region: what the last action did.
    announcement: String,
    name_form: AddFriendForm,
    code_form: AddFriendForm,
    friend_code_copy: Entity<FriendCodeCopyState>,
    menu: Option<(Entity<ContextMenu>, Subscription)>,
    confirmation: Option<(Entity<ModalDialog>, Subscription)>,
    /// Where each row's More button is, for its menu to open under it.
    more_button_bounds: Rc<RefCell<HashMap<Snowflake, Bounds<Pixels>>>>,
    /// The first action of each row drawn last, so focus lands on a row near one that left.
    row_first_actions: Vec<FocusHandle>,
    /// A row the reader acted on left: focus goes to the one at this index once drawn.
    pending_focus_row: Option<usize>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for FriendsPane {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FriendsPane")
            .field("tab", &self.tab)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<FriendsPaneEvent> for FriendsPane {}

impl FriendsPane {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let catalogue = strings(cx).friends();
        let name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(catalogue.username_placeholder()));
        let code_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(catalogue.friend_code_placeholder()));
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            Self::follow_form_input(&name_input, Lookup::Name, window, cx),
            Self::follow_form_input(&code_input, Lookup::FriendCode, window, cx),
        ];
        Self {
            session,
            tab: FriendsTab::Online,
            tab_focus: FriendsTab::ALL.iter().map(|_| cx.focus_handle()).collect(),
            busy_user_ids: HashSet::new(),
            action_error: None,
            announcement: String::new(),
            name_form: AddFriendForm {
                input: name_input,
                is_sending: false,
                result: None,
            },
            code_form: AddFriendForm {
                input: code_input,
                is_sending: false,
                result: None,
            },
            friend_code_copy: cx.new(|_| FriendCodeCopyState::default()),
            menu: None,
            confirmation: None,
            more_button_bounds: Rc::new(RefCell::new(HashMap::new())),
            row_first_actions: Vec::new(),
            pending_focus_row: None,
            _subscriptions: subscriptions,
        }
    }

    fn follow_form_input(
        input: &Entity<InputState>,
        lookup: Lookup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            input,
            window,
            move |pane, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    pane.form_mut(lookup).result = None;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => pane.send_friend_request(lookup, window, cx),
                InputEvent::Focus | InputEvent::Blur => cx.notify(),
            },
        )
    }

    fn form_mut(&mut self, lookup: Lookup) -> &mut AddFriendForm {
        match lookup {
            Lookup::Name => &mut self.name_form,
            Lookup::FriendCode => &mut self.code_form,
        }
    }

    #[must_use]
    pub fn tab(&self) -> FriendsTab {
        self.tab
    }

    /// What the last action did, read out by Home's status region.
    pub(super) fn announcement(&self) -> &str {
        &self.announcement
    }

    /// The Remove friend or Block dialog, drawn by Home over the whole window.
    pub(super) fn confirmation_dialog(&self) -> Option<&Entity<ModalDialog>> {
        self.confirmation.as_ref().map(|(dialog, _)| dialog)
    }

    /// Chooses a tab; `should_focus_field`: a click or Enter on Add friend goes to its field,
    /// while arrows stay on the tabs.
    pub fn choose_tab(
        &mut self,
        tab: FriendsTab,
        should_focus_field: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tab = tab;
        self.action_error = None;
        // A row of the tab left behind must not pull focus back once it answers.
        self.pending_focus_row = None;
        if tab == FriendsTab::Add && should_focus_field {
            self.name_form
                .input
                .update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    /// The selected tab, or Add friend's field: where focus starts in the pane.
    pub fn focus_start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab == FriendsTab::Add {
            self.name_form
                .input
                .update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.focus_selected_tab(window, cx);
        }
    }

    fn focus_selected_tab(&self, window: &mut Window, cx: &mut App) {
        if let Some(handle) = self.tab_focus.get(tab_index(self.tab)) {
            window.focus(handle, cx);
        }
    }

    /// The first action of the row at `index` (or the last row), else the selected tab: where
    /// focus goes once the row it was on has left.
    pub(super) fn focus_row_at(&self, index: usize, window: &mut Window, cx: &mut App) {
        match self
            .row_first_actions
            .get(index.min(self.row_first_actions.len().saturating_sub(1)))
        {
            Some(handle) => window.focus(handle, cx),
            None => self.focus_selected_tab(window, cx),
        }
    }

    /// The row index of `user_id` as drawn last.
    fn row_index_of(&self, user_id: Snowflake, cx: &App) -> usize {
        friends_groups_in(self.session.read(cx).state(), self.tab)
            .iter()
            .flat_map(|group| group.relationships.iter())
            .position(|relationship| relationship.user.id == user_id)
            .unwrap_or(0)
    }

    // ---- Row actions ----

    /// Runs one row action; the row may leave the list, and focus moves to the one now at its
    /// place if it was on it.
    fn act(
        &mut self,
        user: &PublicUser,
        action: RowAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let user_id = user.id;
        let person = display_name(user).to_owned();
        let row_index = self.row_index_of(user_id, cx);
        self.busy_user_ids.insert(user_id);
        self.action_error = None;
        let pane = cx.entity().downgrade();
        let window_handle = window.window_handle();
        let focused_on_press = window.focused(cx);
        let on_done = move |answer: Result<(), ClientError>, cx: &mut App| {
            let _ = window_handle.update(cx, |_, window, cx| {
                // No dialog holds the reader here: focus only follows them if they stayed put
                // (the web's `isFocusLost`), never out of a field they moved to meanwhile.
                let has_focus_stayed = window.focused(cx) == focused_on_press;
                let _ = pane.update(cx, |pane, cx| {
                    pane.busy_user_ids.remove(&user_id);
                    let words = strings(cx).friends();
                    match answer {
                        Ok(()) => {
                            pane.announcement = action.done(words, &person);
                            if has_focus_stayed {
                                pane.focus_row_after_change(row_index, cx);
                            }
                        }
                        Err(error) => {
                            pane.action_error = Some(describe_error(
                                &error,
                                &action.failed(words, &person),
                                crate::strings_pending_i18n::strings(cx),
                            ));
                        }
                    }
                    cx.notify();
                });
            });
        };
        self.session.update(cx, |session, cx| match action {
            RowAction::Accept => {
                session.set_relationship(user_id, SetRelationshipKind::Friend, cx, on_done);
            }
            RowAction::Ignore | RowAction::Cancel | RowAction::Unblock => {
                session.remove_relationship(user_id, cx, on_done);
            }
        });
        cx.notify();
    }

    /// After the list changed under a row the reader acted on: focus lands on the row now at its
    /// place once the list without it is drawn, else on the selected tab.
    pub(super) fn focus_row_after_change(&mut self, index: usize, cx: &mut Context<Self>) {
        self.pending_focus_row = Some(index);
        cx.notify();
    }

    fn message(&mut self, user: &PublicUser, cx: &mut Context<Self>) {
        let user_id = user.id;
        self.busy_user_ids.insert(user_id);
        self.action_error = None;
        let failure = strings(cx).members().open_conversation_failed().to_owned();
        let pane = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.open_direct_message(user_id, failure, cx, move |answer, cx| {
                let _ = pane.update(cx, |pane, cx| {
                    pane.busy_user_ids.remove(&user_id);
                    match answer {
                        Ok(conversation_id) => {
                            cx.emit(FriendsPaneEvent::OpenConversation(conversation_id));
                        }
                        Err(message) => pane.action_error = Some(message),
                    }
                    cx.notify();
                });
            });
        });
        cx.notify();
    }

    /// The More button's menu, under the button: Remove friend and Block, both confirmed.
    fn open_more_menu(
        &mut self,
        user: PublicUser,
        opener: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.menu.take().is_some() {
            cx.notify();
            return;
        }
        let Some(bounds) = self.more_button_bounds.borrow().get(&user.id).copied() else {
            return;
        };
        let words = strings(cx).friends();
        let person = display_name(&user).to_owned();
        let pane = cx.entity().downgrade();
        let confirm = move |confirmed: Confirmed| {
            let pane = pane.clone();
            let user = user.clone();
            move |window: &mut Window, cx: &mut App| {
                let _ = pane.update(cx, |pane, cx| {
                    pane.confirm(&user, confirmed, true, window, cx);
                });
            }
        };
        let entries = vec![
            MenuEntry::Item(
                MenuItem::new(
                    words.remove_friend(),
                    IconName::UserMinus,
                    confirm(Confirmed::Remove),
                )
                .tone(MenuItemTone::Danger),
            ),
            MenuEntry::Item(
                MenuItem::new(words.block(), IconName::Prohibit, confirm(Confirmed::Block))
                    .tone(MenuItemTone::Danger),
            ),
        ];
        let origin = MenuOrigin::Anchor {
            bounds: MenuAnchor {
                top: f32::from(bounds.top()),
                bottom: f32::from(bounds.bottom()),
                left: f32::from(bounds.left()),
                right: f32::from(bounds.right()),
            },
            alignment: MenuAlignment::End,
        };
        let menu = cx.new(|cx| {
            ContextMenu::open(
                words.menu(&person),
                entries,
                origin,
                Some(opener),
                window,
                cx,
            )
        });
        let subscription = cx.subscribe(&menu, |pane, _, _: &gpui::DismissEvent, cx| {
            pane.menu = None;
            cx.notify();
        });
        self.menu = Some((menu, subscription));
        cx.notify();
    }

    /// "Remove friend" and "Block", the destructive pattern of DESIGN.md §8: Cancel takes focus.
    /// Blocking says what it does, and that the other person is never told (ADR-0025). Home draws
    /// the dialog, so a person's menu anywhere asks the same way; `should_focus_row_after`: asked
    /// from a row here, whose place focus moves to once it changed.
    pub(super) fn confirm(
        &mut self,
        user: &PublicUser,
        confirmed: Confirmed,
        should_focus_row_after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let catalogue = strings(cx);
        let words = catalogue.friends();
        let person = display_name(user).to_owned();
        let row_index = self.row_index_of(user.id, cx);
        let dialog = cx.new(|cx| match confirmed {
            Confirmed::Remove => ModalDialog::new(
                words.remove_title(&person),
                catalogue.common().cancel(),
                window,
                cx,
            )
            .subtitle(words.remove_body(&person))
            .submit(words.remove_friend(), words.removing(), true),
            Confirmed::Block => ModalDialog::new(
                words.block_title(&person),
                catalogue.common().cancel(),
                window,
                cx,
            )
            .subtitle(words.block_body(&person))
            .submit(words.block_submit(&person), words.blocking(), true),
        });
        let user_id = user.id;
        let subscription = cx.subscribe_in(
            &dialog,
            window,
            move |pane, dialog, event: &ModalDialogEvent, window, cx| match event {
                ModalDialogEvent::Closed => {
                    pane.confirmation = None;
                    cx.notify();
                }
                ModalDialogEvent::Submitted => {
                    dialog.update(cx, |dialog, cx| {
                        dialog.set_error(None, cx);
                        dialog.set_busy(true, cx);
                    });
                    let failure = match confirmed {
                        Confirmed::Remove => words.remove_failed(&person),
                        Confirmed::Block => words.block_failed(&person),
                    };
                    let done = match confirmed {
                        Confirmed::Remove => words.removed(&person),
                        Confirmed::Block => words.blocked(&person),
                    };
                    let dialog = dialog.downgrade();
                    let pane_handle = cx.entity().downgrade();
                    let window_handle = window.window_handle();
                    let on_done = move |answer: Result<(), ClientError>, cx: &mut App| {
                        finish_confirmation(
                            &FinishedConfirmation {
                                window_handle,
                                pane: pane_handle,
                                dialog,
                                row_index: should_focus_row_after.then_some(row_index),
                                done,
                                failure,
                            },
                            &answer,
                            cx,
                        );
                    };
                    pane.session.update(cx, |session, cx| match confirmed {
                        Confirmed::Remove => session.remove_relationship(user_id, cx, on_done),
                        Confirmed::Block => session.set_relationship(
                            user_id,
                            SetRelationshipKind::Blocked,
                            cx,
                            on_done,
                        ),
                    });
                }
            },
        );
        self.confirmation = Some((dialog, subscription));
        cx.notify();
    }

    // ---- Add friend ----

    /// Sends the request typed in the form; the answer is said under its field, which keeps focus.
    fn send_friend_request(&mut self, lookup: Lookup, window: &mut Window, cx: &mut Context<Self>) {
        let form = self.form_mut(lookup);
        let typed = form.input.read(cx).value().trim().to_owned();
        if typed.is_empty() || form.is_sending {
            return;
        }
        form.is_sending = true;
        form.result = None;
        let state = self.session.read(cx).state();
        let kinds_before: HashMap<Snowflake, RelationshipKind> = [
            RelationshipKind::Friend,
            RelationshipKind::IncomingRequest,
            RelationshipKind::OutgoingRequest,
            RelationshipKind::Blocked,
        ]
        .into_iter()
        .flat_map(|kind| state.relationships_of_kind(kind))
        .map(|relationship| (relationship.user.id, relationship.kind))
        .collect();
        let request = match lookup {
            Lookup::Name => SendFriendRequest {
                username: typed,
                friend_code: None,
            },
            Lookup::FriendCode => SendFriendRequest {
                username: String::new(),
                friend_code: Some(typed),
            },
        };
        let pane = cx.entity().downgrade();
        let window_handle = window.window_handle();
        self.session.update(cx, |session, cx| {
            session.send_friend_request(request, cx, move |answer, cx| {
                let _ = window_handle.update(cx, |_, window, cx| {
                    let _ = pane.update(cx, |pane, cx| {
                        let words = strings(cx).friends();
                        let result = match answer {
                            Ok(relationship) => {
                                let person = display_name(&relationship.user);
                                let outcome = friend_request_outcome(
                                    kinds_before.get(&relationship.user.id).copied(),
                                    relationship.kind,
                                );
                                (true, outcome_text(words, outcome, person))
                            }
                            Err(error) => (false, refusal_text(&error, words, cx)),
                        };
                        let form = pane.form_mut(lookup);
                        form.is_sending = false;
                        if result.0 {
                            form.input
                                .update(cx, |input, cx| input.set_value("", window, cx));
                        }
                        // After the reset above, whose change event clears the result.
                        let form = pane.form_mut(lookup);
                        form.result = Some(result);
                        form.input.update(cx, |input, cx| input.focus(window, cx));
                        cx.notify();
                    });
                });
            });
        });
        cx.notify();
    }

    // ---- Rendering ----

    /// The tab list, drawn by the shell in the chat header or under it.
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    pub fn tab_list(pane: &Entity<Self>, window: &mut Window, cx: &mut App) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.friends();
        let (selected, focus_handles, incoming, has_friends) = {
            let pane = pane.read(cx);
            let state = pane.session.read(cx).state();
            (
                pane.tab,
                pane.tab_focus.clone(),
                state.incoming_request_count(),
                !state
                    .relationships_of_kind(RelationshipKind::Friend)
                    .is_empty(),
            )
        };
        let is_invitation_shown = !has_friends
            && matches!(selected, FriendsTab::Online | FriendsTab::All)
            && friends_groups_in(pane.read(cx).session.read(cx).state(), selected).is_empty();
        let tabs: Vec<AnyElement> = FriendsTab::ALL
            .iter()
            .zip(focus_handles.iter())
            .map(|(&candidate, focus_handle)| {
                let id = ElementId::from(("friends-tab", tab_index(candidate)));
                let focus_handle = focus_handle.clone().tab_stop(candidate == selected);
                let hover = remembered_hover(&id, window, cx);
                let is_hovered = *hover.read(cx);
                let is_selected = candidate == selected;
                let (fill, ink) = if candidate == FriendsTab::Add {
                    if is_selected || is_invitation_shown {
                        (
                            is_hovered.then_some(tokens.component_friends_tab_background_hover),
                            tokens.component_friends_add_tab_text_selected,
                        )
                    } else {
                        (
                            Some(if is_hovered {
                                tokens.component_friends_add_tab_background_hover
                            } else {
                                tokens.component_friends_add_tab_background
                            }),
                            tokens.component_friends_add_tab_text,
                        )
                    }
                } else if is_selected {
                    (
                        Some(tokens.component_friends_tab_background_selected),
                        tokens.component_friends_tab_text_selected,
                    )
                } else if is_hovered {
                    (
                        Some(tokens.component_friends_tab_background_hover),
                        tokens.component_friends_tab_text_hover,
                    )
                } else {
                    (None, tokens.component_friends_tab_text)
                };
                let label = tab_label(catalogue, candidate);
                let accessible_label = if candidate == FriendsTab::Pending && incoming > 0 {
                    words.pending_with_count(i64::try_from(incoming).unwrap_or(i64::MAX))
                } else {
                    label.to_owned()
                };
                let is_ring_shown = is_focus_visible(&focus_handle, window);
                let pane = pane.clone();
                with_type(div(), TYPE_BUTTON)
                    .id(id)
                    .role(Role::Tab)
                    .aria_label(accessible_label)
                    .aria_selected(is_selected)
                    .aria_position_in_set(tab_index(candidate) + 1)
                    .aria_size_of_set(FriendsTab::ALL.len())
                    .track_focus(&focus_handle)
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .gap(length(SPACE_2XS))
                    .h(length(COMPONENT_FRIENDS_TAB_HEIGHT))
                    .px(length(COMPONENT_FRIENDS_TAB_PADDING_INLINE))
                    .rounded(length(COMPONENT_FRIENDS_TAB_RADIUS))
                    .when_some(fill, |tab, fill| tab.bg(color(fill)))
                    .text_color(color(ink))
                    .cursor_pointer()
                    .on_hover(move |is_hovered, _, cx| {
                        hover.update(cx, |remembered, cx| {
                            *remembered = *is_hovered;
                            cx.notify();
                        });
                    })
                    .on_click(move |_, window, cx| {
                        pane.update(cx, |pane, cx| pane.choose_tab(candidate, true, window, cx));
                    })
                    .child(label)
                    .when(candidate == FriendsTab::Pending && incoming > 0, |tab| {
                        tab.child(MentionBadge::new(
                            u32::try_from(incoming).unwrap_or(u32::MAX),
                        ))
                    })
                    .when(is_ring_shown, |tab| {
                        tab.child(focus_ring(COMPONENT_FRIENDS_TAB_RADIUS, cx))
                    })
                    .into_any_element()
            })
            .collect();
        let key_pane = pane.clone();
        div()
            .id("friends-tabs")
            .role(Role::TabList)
            .aria_label(words.title())
            // The tabs wrap rather than hide one off the edge (long French labels, large text).
            .flex()
            .flex_wrap()
            .items_center()
            .gap(length(COMPONENT_FRIENDS_TAB_GAP))
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.modified() {
                    return;
                }
                let current = tab_index(key_pane.read(cx).tab);
                let Some(next) =
                    next_tab_index(&event.keystroke.key, current, FriendsTab::ALL.len())
                else {
                    return;
                };
                cx.stop_propagation();
                key_pane.update(cx, |pane, cx| {
                    pane.choose_tab(FriendsTab::ALL[next], false, window, cx);
                    pane.focus_selected_tab(window, cx);
                });
            })
            .children(tabs)
            .into_any_element()
    }

    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render_row(
        &self,
        relationship: &Relationship,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (AnyElement, FocusHandle) {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.friends();
        let state = self.session.read(cx).state();
        let user = relationship.user.clone();
        let person = display_name(&user).to_owned();
        let presence = state.presence_of(user.id);
        let is_friend = relationship.kind == RelationshipKind::Friend;
        let is_busy = self.busy_user_ids.contains(&user.id);
        let detail = match relationship.kind {
            RelationshipKind::Friend => state.custom_status_of(user.id).map_or_else(
                || presence_name(catalogue, presence).to_owned(),
                str::to_owned,
            ),
            RelationshipKind::IncomingRequest => words.incoming_request().to_owned(),
            RelationshipKind::OutgoingRequest => words.outgoing_request().to_owned(),
            RelationshipKind::Blocked => words.blocked_detail().to_owned(),
        };
        let row_id = ElementId::from(("friend-row", user.id.raw()));
        let hover = remembered_hover(&row_id, window, cx);
        let picture = user
            .avatar
            .map(|image| profile_picture(image, COMPONENT_FRIENDS_AVATAR_SIZE, window));
        let mut avatar = Avatar::new(
            AvatarPerson::from_user(&user, picture),
            COMPONENT_FRIENDS_AVATAR_SIZE,
        );
        if is_friend && presence != PresenceStatus::Offline {
            avatar = avatar.presence(shown_presence(presence), color(tokens.color_surface_chat));
        }
        let avatar = avatar.dimmed(is_friend && presence == PresenceStatus::Offline);

        let pane = cx.entity();
        let mut actions: Vec<AnyElement> = Vec::new();
        let mut first_action = None;
        let mut push = |(element, focus): (AnyElement, FocusHandle)| {
            first_action.get_or_insert(focus);
            actions.push(element);
        };
        let action_id = |name: &'static str| ElementId::from((name, user.id.raw()));
        match relationship.kind {
            RelationshipKind::Friend => {
                let message_user = user.clone();
                let message_pane = pane.clone();
                push(round_action(
                    action_id("friend-message"),
                    IconName::ChatCircle,
                    words.message(&person),
                    ActionTone::Neutral,
                    is_busy,
                    move |_, cx| {
                        message_pane.update(cx, |pane, cx| pane.message(&message_user, cx));
                    },
                    window,
                    cx,
                ));
                let more_id = action_id("friend-more");
                let more_focus = remembered_focus_handle(&more_id, window, cx);
                let more_user = user.clone();
                let more_pane = pane.clone();
                let opener = more_focus.clone();
                let (more, _) = round_action(
                    more_id,
                    IconName::DotsThree,
                    words.more(&person),
                    ActionTone::Neutral,
                    is_busy,
                    move |window, cx| {
                        more_pane.update(cx, |pane, cx| {
                            pane.open_more_menu(more_user.clone(), opener.clone(), window, cx);
                        });
                    },
                    window,
                    cx,
                );
                let bounds = self.more_button_bounds.clone();
                let user_id = user.id;
                push((
                    div()
                        .relative()
                        .child(more)
                        // Records where the button is, for its menu to open under it.
                        .child(
                            canvas(
                                move |button_bounds, _, _| {
                                    bounds.borrow_mut().insert(user_id, button_bounds);
                                },
                                |_, (), _, _| {},
                            )
                            .absolute()
                            .size_full(),
                        )
                        .into_any_element(),
                    more_focus,
                ));
            }
            RelationshipKind::IncomingRequest => {
                for (name, icon_name, label, tone, row_action) in [
                    (
                        "friend-accept",
                        IconName::Check,
                        words.accept(&person),
                        ActionTone::Positive,
                        RowAction::Accept,
                    ),
                    (
                        "friend-ignore",
                        IconName::X,
                        words.ignore(&person),
                        ActionTone::Danger,
                        RowAction::Ignore,
                    ),
                ] {
                    let user = user.clone();
                    let pane = pane.clone();
                    push(round_action(
                        action_id(name),
                        icon_name,
                        label,
                        tone,
                        is_busy,
                        move |window, cx| {
                            pane.update(cx, |pane, cx| pane.act(&user, row_action, window, cx));
                        },
                        window,
                        cx,
                    ));
                }
            }
            RelationshipKind::OutgoingRequest => {
                let user = user.clone();
                let pane = pane.clone();
                push(round_action(
                    action_id("friend-cancel"),
                    IconName::X,
                    words.cancel(&person),
                    ActionTone::Danger,
                    is_busy,
                    move |window, cx| {
                        pane.update(cx, |pane, cx| {
                            pane.act(&user, RowAction::Cancel, window, cx);
                        });
                    },
                    window,
                    cx,
                ));
            }
            RelationshipKind::Blocked => {
                let id = action_id("friend-unblock");
                let focus = remembered_focus_handle(&id, window, cx);
                let user = user.clone();
                let pane = pane.clone();
                push((
                    Button::new(id, words.unblock())
                        .accessible_label(words.unblock_label(&person))
                        .variant(ButtonVariant::Secondary)
                        .disabled(is_busy)
                        .track_focus(&focus)
                        .on_click(move |window, cx| {
                            pane.update(cx, |pane, cx| {
                                pane.act(&user, RowAction::Unblock, window, cx);
                            });
                        })
                        .into_any_element(),
                    focus,
                ));
            }
        }
        let first_action = first_action.unwrap_or_else(|| cx.focus_handle());
        let row_bounds = remembered_bounds(&row_id, window, cx);
        let pointer_user = user.clone();
        let key_user = user.clone();
        let key_bounds = row_bounds.clone();
        let row = div()
            .id(row_id)
            .role(Role::ListItem)
            .aria_label(format!("{person}, {detail}"))
            .mt(length(COMPONENT_FRIENDS_ROW_GAP))
            .h(length(COMPONENT_FRIENDS_ROW_HEIGHT))
            .px(length(COMPONENT_FRIENDS_ROW_PADDING_INLINE))
            .flex()
            .items_center()
            .gap(length(SPACE_SM))
            .rounded(length(COMPONENT_FRIENDS_ROW_RADIUS))
            .when(*hover.read(cx), |row| {
                row.bg(color(tokens.component_friends_row_background_hover))
            })
            .on_hover(move |is_hovered, _, cx| {
                hover.update(cx, |remembered, cx| {
                    *remembered = *is_hovered;
                    cx.notify();
                });
            })
            // The person's menu (`PersonContextMenu`): at the pointer, or beside the row from the
            // keyboard while focus is on one of its actions.
            .relative()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |pane, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    cx.emit(FriendsPaneEvent::OpenPersonMenu(PersonMenuRequest {
                        friends_row_index: Some(pane.row_index_of(pointer_user.id, cx)),
                        ..PersonMenuRequest::new(
                            pointer_user.clone(),
                            menu_origin_at_pointer(event.position),
                        )
                    }));
                }),
            )
            .on_key_down(cx.listener(move |pane, event: &KeyDownEvent, _, cx| {
                if !is_context_menu_keystroke(&event.keystroke) {
                    return;
                }
                if let Some(origin) = menu_origin_beside_remembered(&key_bounds, cx) {
                    cx.stop_propagation();
                    cx.emit(FriendsPaneEvent::OpenPersonMenu(PersonMenuRequest {
                        friends_row_index: Some(pane.row_index_of(key_user.id, cx)),
                        ..PersonMenuRequest::new(key_user.clone(), origin)
                    }));
                }
            }))
            .child(bounds_recorder(row_bounds))
            .child(avatar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        with_type(div(), TYPE_MEMBER_NAME)
                            .truncate()
                            .text_color(color(tokens.component_friends_name_text))
                            .child(person),
                    )
                    .child(
                        with_type(div(), TYPE_BODY_SM)
                            .truncate()
                            .text_color(color(tokens.component_friends_detail_text))
                            .child(detail),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(length(COMPONENT_FRIENDS_ACTION_GAP))
                    .children(actions),
            );
        (row.into_any_element(), first_action)
    }

    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render_add_friend_form(
        &self,
        lookup: Lookup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let words = strings(cx).friends();
        let form = match lookup {
            Lookup::Name => &self.name_form,
            Lookup::FriendCode => &self.code_form,
        };
        let host = self
            .session
            .read(cx)
            .last_instance()
            .map(|instance| instance.host().to_owned())
            .unwrap_or_default();
        let (title, hint, label) = match lookup {
            Lookup::Name => (
                words.add_title().to_owned(),
                words.add_subtitle(&host),
                words.username(),
            ),
            Lookup::FriendCode => (
                words.add_by_code_title().to_owned(),
                words.add_by_code_subtitle().to_owned(),
                words.friend_code(),
            ),
        };
        let can_send = !form.input.read(cx).value().trim().is_empty() && !form.is_sending;
        let is_field_focused = form.input.read(cx).focus_handle(cx).is_focused(window);
        let result = form.result.clone();
        let id_prefix = match lookup {
            Lookup::Name => "add-friend-by-name",
            Lookup::FriendCode => "add-friend-by-code",
        };
        let description = match &result {
            Some((_, text)) => format!("{label}, {hint} {text}"),
            None => format!("{label}, {hint}"),
        };
        let pane = cx.entity();
        div()
            .id(id_prefix)
            .role(Role::Form)
            .aria_label(title.clone())
            .max_w(length(COMPONENT_FRIENDS_CONTENT_MAX_WIDTH))
            .flex()
            .flex_col()
            .gap(length(SPACE_XS))
            .child(
                with_type(div(), TYPE_HEADING_LG)
                    .text_color(color(tokens.color_text_heading))
                    .child(title),
            )
            .child(
                with_type(div(), TYPE_BODY)
                    .text_color(color(tokens.color_text_secondary))
                    .child(hint),
            )
            .child(
                // The button sits inside the field, as on Discord; narrow, it wraps under it.
                div()
                    .relative()
                    .mt(length(SPACE_XS))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(length(SPACE_XS))
                    .rounded(length(COMPONENT_TEXT_FIELD_RADIUS))
                    .border(length(COMPONENT_TEXT_FIELD_BORDER_WIDTH))
                    .border_color(color(tokens.component_text_field_border))
                    .bg(color(tokens.component_text_field_background))
                    .p(length(SPACE_2XS))
                    .pl(length(COMPONENT_TEXT_FIELD_PADDING_INLINE))
                    .child(
                        with_type(div(), TYPE_BODY_LG)
                            .flex_1()
                            .min_w(length(COMPONENT_FRIENDS_USERNAME_MIN_WIDTH))
                            .text_color(color(tokens.component_text_field_text))
                            .child(
                                Input::new(&form.input)
                                    .appearance(false)
                                    .disabled(form.is_sending)
                                    .aria_label(SharedString::from(description)),
                            ),
                    )
                    .child(
                        Button::new(
                            ElementId::from(SharedString::from(format!("{id_prefix}-send"))),
                            if form.is_sending {
                                words.sending()
                            } else {
                                words.send()
                            },
                        )
                        .variant(ButtonVariant::Primary)
                        .disabled(!can_send)
                        .on_click(move |window, cx| {
                            pane.update(cx, |pane, cx| {
                                pane.send_friend_request(lookup, window, cx);
                            });
                        }),
                    )
                    .when(is_field_focused, |field| {
                        field.child(focus_ring(COMPONENT_TEXT_FIELD_RADIUS, cx))
                    }),
            )
            .child(
                // Polite: the answer is read once it arrives, whichever it is.
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "{id_prefix}-result"
                    ))))
                    .role(Role::Status)
                    .when_some(result.as_ref(), |line, (_, text)| {
                        line.aria_label(SharedString::from(text.clone()))
                    })
                    .min_h(length(TYPE_BODY.font_size * TYPE_BODY.line_height))
                    .when_some(result, |line, (is_success, text)| {
                        let ink = color(if is_success {
                            tokens.component_friends_result_positive_text
                        } else {
                            tokens.component_friends_result_error_text
                        });
                        line.child(
                            with_type(div(), TYPE_BODY)
                                .flex()
                                .items_start()
                                .gap(length(SPACE_2XS))
                                .text_color(ink)
                                .child(icon(
                                    if is_success {
                                        IconName::Check
                                    } else {
                                        IconName::WarningCircle
                                    },
                                    COMPONENT_FRIENDS_RESULT_ICON_SIZE,
                                    ink,
                                ))
                                .child(text),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_empty_state(
        sentence: &'static str,
        with_add_friend: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let words = strings(cx).friends();
        let add_friend_pane = cx.entity();
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(length(COMPONENT_FRIENDS_EMPTY_GAP))
            .p(length(SPACE_XL))
            .child(
                with_type(div(), TYPE_BODY_LG)
                    .max_w(length(COMPONENT_FRIENDS_CONTENT_MAX_WIDTH))
                    .text_center()
                    .text_color(color(tokens.color_text_secondary))
                    .child(sentence),
            )
            // shortcut: the web's "Add a server" beside it, for an account in no server, comes
            // with server management (M3 step 6).
            .when(with_add_friend, |empty| {
                empty.child(
                    Button::new("friends-empty-add-friend", words.add_friend())
                        .variant(ButtonVariant::Primary)
                        .on_click(move |window, cx| {
                            add_friend_pane.update(cx, |pane, cx| {
                                pane.choose_tab(FriendsTab::Add, true, window, cx);
                            });
                        }),
                )
            })
            .into_any_element()
    }
}

/// A row action Accept, Ignore, Cancel or Unblock: no confirmation, as on the web.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowAction {
    Accept,
    Ignore,
    Cancel,
    Unblock,
}

impl RowAction {
    fn done(self, words: crate::i18n::catalogue::FriendsStrings, person: &str) -> String {
        match self {
            Self::Accept => words.accepted(person),
            Self::Ignore => words.ignored(person),
            Self::Cancel => words.cancelled(person),
            Self::Unblock => words.unblocked(person),
        }
    }

    fn failed(self, words: crate::i18n::catalogue::FriendsStrings, person: &str) -> String {
        match self {
            Self::Accept => words.accept_failed(person),
            Self::Ignore => words.ignore_failed(person),
            Self::Cancel => words.cancel_failed(person),
            Self::Unblock => words.unblock_failed(person),
        }
    }
}

/// What a confirmed Remove or Block needs once the instance answered.
struct FinishedConfirmation {
    window_handle: AnyWindowHandle,
    pane: gpui::WeakEntity<FriendsPane>,
    dialog: gpui::WeakEntity<ModalDialog>,
    /// The row focus moves to, when asked from the pane.
    row_index: Option<usize>,
    done: String,
    failure: String,
}

/// Success closes the dialog and moves focus to the row now at the friend's place; a failure
/// stays in the dialog, said with its reason.
fn finish_confirmation(
    finished: &FinishedConfirmation,
    answer: &Result<(), ClientError>,
    cx: &mut App,
) {
    let _ = finished.window_handle.update(cx, |_, window, cx| {
        let _ = finished.dialog.update(cx, |dialog, cx| {
            dialog.set_busy(false, cx);
            match answer {
                Ok(()) => dialog.request_close(window, cx),
                Err(error) => dialog.set_error(
                    Some(SharedString::from(describe_error(
                        error,
                        &finished.failure,
                        crate::strings_pending_i18n::strings(cx),
                    ))),
                    cx,
                ),
            }
        });
        if answer.is_ok() {
            let _ = finished.pane.update(cx, |pane, cx| {
                pane.announcement.clone_from(&finished.done);
                if let Some(row_index) = finished.row_index {
                    pane.focus_row_after_change(row_index, cx);
                }
                cx.notify();
            });
        }
    });
}

fn tab_index(tab: FriendsTab) -> usize {
    FriendsTab::ALL
        .iter()
        .position(|candidate| *candidate == tab)
        .unwrap_or(0)
}

fn tab_label(catalogue: Strings, tab: FriendsTab) -> &'static str {
    let tabs = catalogue.friends().tabs();
    match tab {
        FriendsTab::Online => tabs.online(),
        FriendsTab::All => tabs.all(),
        FriendsTab::Pending => tabs.pending(),
        FriendsTab::Blocked => tabs.blocked(),
        FriendsTab::Add => tabs.add(),
    }
}

fn group_label(catalogue: Strings, kind: FriendsGroupKind) -> &'static str {
    let groups = catalogue.friends().groups();
    match kind {
        FriendsGroupKind::Online => groups.online(),
        FriendsGroupKind::All => groups.all(),
        FriendsGroupKind::Incoming => groups.incoming(),
        FriendsGroupKind::Outgoing => groups.outgoing(),
        FriendsGroupKind::Blocked => groups.blocked(),
    }
}

fn outcome_text(
    words: crate::i18n::catalogue::FriendsStrings,
    outcome: FriendRequestOutcome,
    person: &str,
) -> String {
    match outcome {
        FriendRequestOutcome::Accepted => words.accepted_theirs(person),
        FriendRequestOutcome::AlreadyFriends => words.already_friends(person),
        FriendRequestOutcome::AlreadySent => words.already_sent(person),
        FriendRequestOutcome::Sent => words.sent(person),
    }
}

/// A missing account, yourself and someone who blocked you read the same: the form tells nothing
/// about blocks.
fn refusal_text(
    error: &ClientError,
    words: crate::i18n::catalogue::FriendsStrings,
    cx: &App,
) -> String {
    match error {
        ClientError::Api {
            code: ApiErrorCode::NotFound,
            ..
        } => words.not_found().to_owned(),
        ClientError::Api {
            code: ApiErrorCode::RateLimited,
            ..
        } => words.rate_limited().to_owned(),
        ClientError::Api {
            code: ApiErrorCode::LimitReached,
            ..
        } => words.limit_reached().to_owned(),
        _ => describe_error(
            error,
            words.send_failed(),
            crate::strings_pending_i18n::strings(cx),
        ),
    }
}

/// One of Discord's round row buttons (`actionClass`): its icon brightens under the pointer, green
/// for Accept, red for Ignore and Cancel. Busy, it dims and does nothing. Returns its focus
/// handle, for focus to come back to the row.
#[allow(clippy::too_many_arguments)] // The web's snippet takes the same.
fn round_action(
    id: ElementId,
    icon_name: IconName,
    label: String,
    tone: ActionTone,
    is_busy: bool,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> (AnyElement, FocusHandle) {
    let tokens = theme(cx);
    let focus_handle = remembered_focus_handle(&id, window, cx);
    let hover = remembered_hover(&id, window, cx);
    let is_hovered = *hover.read(cx) && !is_busy;
    let ink = if is_hovered {
        match tone {
            ActionTone::Neutral => tokens.component_friends_action_icon_hover,
            ActionTone::Positive => tokens.component_friends_action_icon_positive_hover,
            ActionTone::Danger => tokens.component_friends_action_icon_danger_hover,
        }
    } else {
        tokens.component_friends_action_icon
    };
    let is_ring_shown = is_focus_visible(&focus_handle, window);
    let tooltip = SharedString::from(label.clone());
    let element = div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .track_focus(&focus_handle)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(length(COMPONENT_FRIENDS_ACTION_SIZE))
        .rounded_full()
        .bg(color(tokens.component_friends_action_background))
        .when(is_busy, |button| {
            button.opacity(COMPONENT_BUTTON_DISABLED_OPACITY)
        })
        .when(!is_busy, gpui::Styled::cursor_pointer)
        .tooltip(move |_, cx| Tooltip::view(tooltip.clone(), cx))
        .tooltip_show_delay(Tooltip::show_delay())
        .on_hover(move |is_hovered, _, cx| {
            hover.update(cx, |remembered, cx| {
                *remembered = *is_hovered;
                cx.notify();
            });
        })
        .on_click(move |_, window, cx| {
            if !is_busy {
                on_press(window, cx);
            }
        })
        .child(icon(
            icon_name,
            COMPONENT_FRIENDS_ACTION_ICON_SIZE,
            color(ink),
        ))
        .when(is_ring_shown, |button| {
            button.child(focus_ring(COMPONENT_FRIENDS_ACTION_SIZE / 2.0, cx))
        })
        .into_any_element();
    (element, focus_handle)
}

impl Render for FriendsPane {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.friends();
        let state = self.session.read(cx).state();
        let groups: Vec<(FriendsGroupKind, Vec<Relationship>)> = friends_groups_in(state, self.tab)
            .into_iter()
            .map(|group| {
                (
                    group.kind,
                    group.relationships.into_iter().cloned().collect(),
                )
            })
            .collect();
        let has_friends = !state
            .relationships_of_kind(RelationshipKind::Friend)
            .is_empty();
        let friend_code = state
            .current_user()
            .and_then(|user| user.friend_code.clone());

        let mut panel: Vec<AnyElement> = Vec::new();
        let mut first_actions = Vec::new();
        if self.tab == FriendsTab::Add {
            panel.push(
                div()
                    .pt(length(SPACE_MD))
                    .flex()
                    .flex_col()
                    .gap(length(SPACE_LG))
                    .child(self.render_add_friend_form(Lookup::Name, window, cx))
                    .child(self.render_add_friend_form(Lookup::FriendCode, window, cx))
                    .when_some(friend_code, |forms, code| {
                        forms.child(
                            div()
                                .max_w(length(COMPONENT_FRIENDS_CONTENT_MAX_WIDTH))
                                .child(FriendCode::new(
                                    "friend-code",
                                    code,
                                    self.friend_code_copy.clone(),
                                )),
                        )
                    })
                    .into_any_element(),
            );
        } else if groups.is_empty() {
            let is_invitation_shown =
                matches!(self.tab, FriendsTab::Online | FriendsTab::All) && !has_friends;
            let sentence = if is_invitation_shown {
                words.no_friends_yet()
            } else {
                match self.tab {
                    FriendsTab::Online => words.empty_online(),
                    FriendsTab::Pending => words.empty_pending(),
                    _ => words.empty_blocked(),
                }
            };
            panel.push(Self::render_empty_state(sentence, is_invitation_shown, cx));
        } else {
            for (kind, relationships) in groups {
                let label = group_label(catalogue, kind);
                let count = relationships.len();
                // Sentence case, the count as its own number, as in the member list.
                panel.push(
                    with_type(div(), TYPE_CATEGORY)
                        .id(ElementId::from(("friends-group", kind as usize)))
                        .role(Role::Heading)
                        .aria_level(2)
                        .aria_label(
                            words.group_label(label, i64::try_from(count).unwrap_or(i64::MAX)),
                        )
                        .h(length(COMPONENT_FRIENDS_GROUP_HEIGHT))
                        .px(length(COMPONENT_FRIENDS_ROW_PADDING_INLINE))
                        .pb(length(SPACE_2XS))
                        .flex()
                        .items_end()
                        .justify_between()
                        .text_color(color(tokens.component_friends_group_text))
                        .child(label)
                        .child(count.to_string())
                        .into_any_element(),
                );
                for relationship in &relationships {
                    let (row, first_action) = self.render_row(relationship, window, cx);
                    first_actions.push(first_action);
                    panel.push(row);
                }
            }
        }
        self.row_first_actions = first_actions;
        if let Some(index) = self.pending_focus_row.take() {
            cx.defer_in(window, move |pane, window, cx| {
                pane.focus_row_at(index, window, cx);
            });
        }

        div()
            .debug_selector(|| "friends-pane".to_owned())
            .id("friends-pane")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .when_some(self.action_error.clone(), |pane, error| {
                pane.child(
                    with_type(div(), TYPE_BODY)
                        .id("friends-action-error")
                        .role(Role::Alert)
                        .aria_label(SharedString::from(error.clone()))
                        .flex()
                        .gap(length(SPACE_2XS))
                        .px(length(COMPONENT_FRIENDS_LIST_PADDING_INLINE))
                        .py(length(SPACE_XS))
                        .text_color(color(tokens.component_friends_result_error_text))
                        .child(icon(
                            IconName::WarningCircle,
                            COMPONENT_FRIENDS_RESULT_ICON_SIZE,
                            color(tokens.component_friends_result_error_text),
                        ))
                        .child(error),
                )
            })
            .child(
                div()
                    .id("friends-panel")
                    .role(Role::TabPanel)
                    .aria_label(tab_label(catalogue, self.tab))
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .px(length(COMPONENT_FRIENDS_LIST_PADDING_INLINE))
                    .pb(length(SPACE_MD))
                    .children(panel),
            )
            .when_some(self.menu.as_ref(), |pane, (menu, _)| {
                pane.child(menu.clone())
            })
    }
}
