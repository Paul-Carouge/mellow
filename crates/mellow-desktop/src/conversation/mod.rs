//! The conversation view (`ChatPane.svelte`, `MessageList.svelte`, DESIGN.md §5.4–§5.5): the
//! open channel's or conversation's messages in a virtual list that opens at the latest message,
//! sticks to it and keeps the rows being read in place while older pages load; the "New" divider
//! and unread bar, the jump bar, the typing line and the composer; each message's hover bar and
//! menu; and the pinned messages panel. It turns the composer's events into the session's
//! requests, and shows what failed above the composer.
//!
//! The shell fills its conversation slot with this view and tells it the place
//! ([`ConversationView::set_place`]); the view applies it in its next render, which has the window.

mod chrome;
mod pins_panel;
mod rows;
pub mod rules;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use gpui::{
    App, AppContext as _, ClipboardItem, Context, Entity, FocusHandle, KeyDownEvent, ListAlignment,
    ListOffset, ListState, SharedString, Subscription, Task, Window, px,
};
use mellow_client_core::shared_logic::mentions::MentionablePeople;
use mellow_client_core::state::{StateChange, UnreadMarker};
use mellow_protocol::{CustomEmoji, Message, Permissions, Role, Snowflake};

use crate::components::{
    ContextMenu, MenuAlignment, MenuAnchor, MenuEntry, MenuItem, MenuItemTone, MenuOrigin,
    ModalDialog, ModalDialogEvent,
};
use crate::composer::{Cancelled, Composer, ComposerEvent, ReplyTarget};
use crate::i18n::strings;
use crate::messages::MessageBodyCache;
use crate::session::{HistoryStatus, SendRefused, Session, StateChanged};
use crate::theme::icons::IconName;
use crate::theme::tokens::COMPONENT_MESSAGE_HISTORY_LOAD_DISTANCE;

use self::rows::Entry;
use self::rules::{MenuLine, MessageFacts, MessageMenuAction, message_menu_lines};

/// Rows measured beyond the visible part, so scrolling and older pages do not pop in.
const LIST_OVERDRAW: f32 = COMPONENT_MESSAGE_HISTORY_LOAD_DISTANCE;
/// Page Up and Page Down scroll this share of the list's height.
const PAGE_SCROLL_SHARE: f32 = 0.9;
/// The typing line forgets people whose notice expired; refreshed this often while it shows.
const TYPING_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// What the shell opens, and how it is named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenConversation {
    pub channel_id: Snowflake,
    /// The server of a server channel; `None` in a conversation.
    pub guild_id: Option<Snowflake>,
    /// "#general" or "Ana": names the list and its history start.
    pub label: String,
    /// "Message #general": the composer's name and placeholder.
    pub composer_label: String,
    pub hero_icon: IconName,
    pub hero_title: String,
    pub hero_sentence: String,
    pub topic: String,
}

/// One visit to a channel: what was unread when it opened, and where the reader went.
#[derive(Debug, Default)]
struct Visit {
    /// The "New" divider and the unread bar, until dismissed (DESIGN.md §5.4).
    unread_marker: Option<UnreadMarker>,
    /// Read once the channel's messages are here, before marking it read.
    is_unread_marker_pending: bool,
    /// The newest message when the reader left the present: "· 3 new" counts what came after.
    latest_id_when_leaving_present: Option<Snowflake>,
    /// Scrolls the list makes itself (opening, new messages) do not dismiss the unread bar.
    has_reader_scrolled: bool,
    /// The list opened on the "New" divider once; later rows never move it again.
    has_opened_on_divider: bool,
}

/// What a confirmation dialog will do once confirmed.
#[derive(Debug, Clone)]
enum Confirmation {
    Delete(Message),
    Pin(Message, bool),
}

/// The words, people and roles messages are read against, rebuilt when they change.
#[derive(Default)]
struct Sources {
    generation: u64,
    people: MentionablePeople,
    custom_emojis: HashMap<Snowflake, CustomEmoji>,
    roles: HashMap<Snowflake, Role>,
    /// Each author's highest coloured role colour (0xRRGGBB), made readable when drawn.
    role_colours: HashMap<Snowflake, u32>,
}

#[allow(clippy::struct_excessive_bools)] // Independent flags of one view, each read on its own.
pub struct ConversationView {
    session: Entity<Session>,
    composer: Entity<Composer>,
    open: Option<OpenConversation>,
    /// Asked by the shell; applied by the next render.
    wanted: Option<OpenConversation>,
    list: ListState,
    entries: Vec<Entry>,
    /// Each entry's key and version, to splice and remeasure only what changed.
    entry_versions: Vec<(String, u64)>,
    are_rows_stale: bool,
    /// The open channel's loading state the rows were last built for.
    shown_history_status: Option<HistoryStatus>,
    are_sources_stale: bool,
    sources: Sources,
    body_cache: MessageBodyCache,
    visit: Visit,
    /// The message row under the pointer: its hover bar shows.
    hovered_message_id: Option<Snowflake>,
    /// The row keyboard focus is on, while the list has focus (one Tab stop, arrows move).
    focused_row_key: Option<String>,
    menu: Option<(Entity<ContextMenu>, Subscription, Snowflake)>,
    confirmation: Option<(Entity<ModalDialog>, Subscription)>,
    is_pins_panel_open: bool,
    /// Where a click outside closed the pins panel: the toggle under it must not reopen it.
    pins_dismissed_by_pointer_at: Option<gpui::Point<gpui::Pixels>>,
    /// What failed, above the composer until the next action or channel.
    action_error: Option<SharedString>,
    /// A refused edit's text, given back to the composer by the next render (which has the
    /// window): the message, the text and whether the editor may reopen on it.
    refused_edit: Option<(Snowflake, String, bool)>,
    /// Read out politely (copied, new messages).
    announcement: SharedString,
    is_shift_held: bool,
    list_focus: FocusHandle,
    pins_focus: FocusHandle,
    typing_refresh: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for ConversationView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConversationView")
            .field("open", &self.open.as_ref().map(|open| open.channel_id))
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl ConversationView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| Composer::new(window, cx));
        let list = ListState::new(0, ListAlignment::Bottom, px(LIST_OVERDRAW));
        list.set_follow_mode(gpui::FollowMode::Tail);
        let view = cx.entity().downgrade();
        list.set_scroll_handler(move |event, _, cx| {
            let _ = view.update(cx, |view, cx| {
                view.note_reader_scrolled(!event.is_scrolled, cx);
            });
        });
        let subscriptions = vec![
            cx.subscribe_in(&composer, window, Self::follow_composer),
            cx.subscribe(&session, |view, _, changes: &StateChanged, cx| {
                view.follow_state(&changes.0, cx);
            }),
            cx.subscribe_in(&session, window, Self::give_back_refused_send),
            // Messages that came while the window was in the background are read once it is back.
            cx.observe_window_activation(window, |view, window, cx| {
                view.follow_reading(window, cx);
            }),
            // Loading and failures only notify: the history start is drawn again when they move.
            cx.observe(&session, |view, session, cx| {
                let status = view
                    .open_channel_id()
                    .map(|channel_id| session.read(cx).history_status(channel_id));
                if status != view.shown_history_status {
                    view.shown_history_status = status;
                    view.are_rows_stale = true;
                }
                cx.notify();
            }),
        ];
        Self {
            session,
            composer,
            open: None,
            wanted: None,
            list,
            entries: Vec::new(),
            entry_versions: Vec::new(),
            are_rows_stale: true,
            shown_history_status: None,
            are_sources_stale: true,
            sources: Sources::default(),
            body_cache: MessageBodyCache::new(),
            visit: Visit::default(),
            hovered_message_id: None,
            focused_row_key: None,
            menu: None,
            confirmation: None,
            is_pins_panel_open: false,
            pins_dismissed_by_pointer_at: None,
            action_error: None,
            refused_edit: None,
            announcement: SharedString::default(),
            is_shift_held: false,
            list_focus: cx.focus_handle().tab_stop(true),
            pins_focus: cx.focus_handle(),
            typing_refresh: None,
            _subscriptions: subscriptions,
        }
    }

    #[must_use]
    pub fn open_channel_id(&self) -> Option<Snowflake> {
        self.open.as_ref().map(|open| open.channel_id)
    }

    #[must_use]
    pub fn composer(&self) -> &Entity<Composer> {
        &self.composer
    }

    #[must_use]
    pub fn is_pins_panel_open(&self) -> bool {
        self.is_pins_panel_open
    }

    /// The place the shell shows; `None` when no channel or conversation is open.
    pub fn set_place(&mut self, wanted: Option<OpenConversation>, cx: &mut Context<Self>) {
        if self.wanted != wanted {
            self.wanted = wanted;
            cx.notify();
        }
    }

    /// Puts focus in the composer (after a place opens from the switcher, or Jump to present).
    pub fn focus_composer(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |composer, cx| composer.focus(window, cx));
    }

    /// Applies the place the shell asked for: the previous channel's visit ends, the composer
    /// switches drafts and the new channel's history is asked for.
    fn apply_wanted_place(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.wanted.clone();
        if wanted == self.open {
            return;
        }
        let previous_channel_id = self.open_channel_id();
        let next_channel_id = wanted.as_ref().map(|open| open.channel_id);
        if previous_channel_id != next_channel_id {
            if let Some(previous) = previous_channel_id {
                self.session.update(cx, |session, cx| {
                    session.trim_history(previous, cx);
                    session.send_read_states(cx);
                });
            }
            self.visit = Visit {
                is_unread_marker_pending: true,
                ..Visit::default()
            };
            self.hovered_message_id = None;
            self.focused_row_key = None;
            self.menu = None;
            self.confirmation = None;
            self.is_pins_panel_open = false;
            self.action_error = None;
            self.entries.clear();
            self.entry_versions.clear();
            self.list.reset(0);
            self.list.set_follow_mode(gpui::FollowMode::Tail);
            self.are_sources_stale = true;
        }
        self.open = wanted;
        self.are_rows_stale = true;
        let label = self
            .open
            .as_ref()
            .map(|open| SharedString::from(open.composer_label.clone()))
            .unwrap_or_default();
        self.composer.update(cx, |composer, cx| {
            composer.set_channel(next_channel_id, label, window, cx);
        });
        if let Some(channel_id) = next_channel_id {
            self.session.update(cx, |session, cx| {
                session.ensure_history_loaded(channel_id, cx);
            });
        }
        self.feed_composer(cx);
        cx.notify();
    }

    /// What the composer needs from the session: permissions, people to mention, uploads.
    fn feed_composer(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let state = session.state();
        let channel_id = self.open_channel_id();
        let permissions = channel_id.map(|channel_id| self.permissions(channel_id, cx));
        let can_send = permissions.is_none_or(|permissions| permissions.can_send);
        let can_attach = permissions.is_none_or(|permissions| permissions.can_attach);
        let people = match &self.open {
            Some(OpenConversation {
                guild_id: Some(guild_id),
                ..
            }) => state
                .members(*guild_id)
                .iter()
                .map(|member| member.user.clone())
                .collect(),
            Some(open) => state
                .direct_message_channel(open.channel_id)
                .map(|conversation| conversation.recipients.clone())
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let current_user_id = state.current_user().map(|user| user.id);
        let rest = session.rest_client().cloned();
        let maximum_attachment_bytes = state.maximum_attachment_bytes();
        self.composer.update(cx, |composer, cx| {
            composer.set_permissions(can_send, can_attach, cx);
            composer.set_mention_candidates(people, current_user_id, cx);
            composer.set_uploads(rest, maximum_attachment_bytes, cx);
        });
    }

    fn follow_state(&mut self, changes: &[StateChange], cx: &mut Context<Self>) {
        let channel_id = self.open_channel_id();
        let guild_id = self.open.as_ref().and_then(|open| open.guild_id);
        let mut have_rows_changed = false;
        for change in changes {
            match *change {
                StateChange::Guilds
                | StateChange::DirectMessages
                | StateChange::Relationships
                | StateChange::UserRenamed { .. }
                | StateChange::GuildEmojis { .. } => {
                    self.are_sources_stale = true;
                }
                StateChange::Members {
                    guild_id: changed_guild_id,
                } if Some(changed_guild_id) == guild_id => {
                    self.are_sources_stale = true;
                }
                StateChange::Messages {
                    channel_id: changed_channel_id,
                }
                | StateChange::Message {
                    channel_id: changed_channel_id,
                    ..
                } if Some(changed_channel_id) == channel_id => {
                    have_rows_changed = true;
                }
                StateChange::ReadStates => have_rows_changed = true,
                StateChange::Typing {
                    channel_id: changed_channel_id,
                } if Some(changed_channel_id) == channel_id => {
                    self.keep_typing_line_fresh(cx);
                }
                _ => {}
            }
        }
        if self.are_sources_stale {
            self.feed_composer(cx);
        }
        // `Ready` emptied the history: the open channel asks again (once per `Ready`).
        if let Some(channel_id) = channel_id {
            self.session.update(cx, |session, cx| {
                session.ensure_history_loaded(channel_id, cx);
            });
        }
        if have_rows_changed || self.are_sources_stale {
            self.are_rows_stale = true;
        }
        cx.notify();
    }

    /// While someone types, the line is drawn again every second so expired notices leave it.
    fn keep_typing_line_fresh(&mut self, cx: &mut Context<Self>) {
        if self.typing_refresh.is_some() {
            return;
        }
        self.typing_refresh = Some(cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(TYPING_REFRESH_INTERVAL)
                    .await;
                let is_anyone_typing = view
                    .update(cx, |view, cx| {
                        cx.notify();
                        let is_typing = !view.typing_user_ids(cx).is_empty();
                        if !is_typing {
                            view.typing_refresh = None;
                        }
                        is_typing
                    })
                    .unwrap_or(false);
                if !is_anyone_typing {
                    break;
                }
            }
        }));
    }

    fn typing_user_ids(&self, cx: &App) -> Vec<Snowflake> {
        self.open_channel_id().map_or_else(Vec::new, |channel_id| {
            self.session
                .read(cx)
                .state()
                .typing_user_ids(channel_id, crate::session::now_unix_milliseconds())
        })
    }

    // ---- Permissions ----

    fn permissions(&self, channel_id: Snowflake, cx: &App) -> ChannelRights {
        let state = self.session.read(cx).state();
        let permissions = state.my_channel_permissions(channel_id);
        let has =
            |permission: Permissions| permissions.is_none_or(|bits| bits.contains(permission));
        let is_conversation = state.direct_message_channel(channel_id).is_some();
        let can_send = has(Permissions::SEND_MESSAGES);
        let can_read_history = has(Permissions::READ_MESSAGE_HISTORY);
        let can_manage_messages =
            permissions.is_some_and(|bits| bits.contains(Permissions::MANAGE_MESSAGES));
        ChannelRights {
            can_send,
            can_attach: can_send && has(Permissions::ATTACH_FILES),
            can_join_reactions: can_read_history,
            can_manage_messages,
            can_manage_pins: is_conversation || can_manage_messages,
        }
    }

    fn message_facts(&self, message: &Message, cx: &App) -> MessageFacts {
        let rights = self.permissions(message.channel_id, cx);
        let current_user_id = self
            .session
            .read(cx)
            .state()
            .current_user()
            .map(|user| user.id);
        MessageFacts {
            is_own: Some(message.author.id) == current_user_id,
            is_forward: message.forwarded_from.is_some(),
            is_rich: message.poll.is_some()
                || message.checklist.is_some()
                || message.progress_bar.is_some(),
            has_text: !message.content.trim().is_empty(),
            is_pinned: message.pinned,
            can_send_messages: rights.can_send,
            can_manage_messages: rights.can_manage_messages,
            can_join_reactions: rights.can_join_reactions,
            can_manage_pins: rights.can_manage_pins,
        }
    }

    fn cached_message(&self, message_id: Snowflake, cx: &App) -> Option<Message> {
        let channel_id = self.open_channel_id()?;
        self.session
            .read(cx)
            .state()
            .cached_message(channel_id, message_id)
            .cloned()
    }

    // ---- Reading: the list's position, unread marker and read state ----

    /// The list is at its newest message, following it.
    fn is_at_latest(&self) -> bool {
        self.list.is_following_tail()
            || self.list.logical_scroll_top().item_ix >= self.list.item_count()
    }

    /// The reader scrolled (wheel, trackpad, scroll bar).
    fn note_reader_scrolled(&mut self, is_at_latest: bool, cx: &mut Context<Self>) {
        self.visit.has_reader_scrolled = true;
        let channel_id = self.open_channel_id();
        if is_at_latest {
            self.visit.latest_id_when_leaving_present = None;
            if let Some(channel_id) = channel_id {
                self.session
                    .update(cx, |session, cx| session.trim_history(channel_id, cx));
            }
            // The unread messages were scrolled through down to the latest: the bar goes.
            if self.visit.unread_marker.take().is_some() {
                self.are_rows_stale = true;
            }
        } else if self.visit.latest_id_when_leaving_present.is_none() {
            self.visit.latest_id_when_leaving_present = channel_id.and_then(|channel_id| {
                self.session
                    .read(cx)
                    .state()
                    .messages(channel_id)
                    .last()
                    .map(|message| message.id)
            });
        }
        cx.notify();
    }

    /// Runs after each rows rebuild: reads the unread marker once the messages are here, marks
    /// the channel read while its latest message shows, and asks for older history near the top.
    fn follow_reading(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(channel_id) = self.open_channel_id() else {
            return;
        };
        let state = self.session.read(cx).state();
        let has_messages = !state.messages(channel_id).is_empty();
        if self.visit.is_unread_marker_pending && has_messages {
            // Read before anything is marked read, and the list opens on its divider before it
            // decides whether the latest message shows.
            self.visit.is_unread_marker_pending = false;
            self.visit.unread_marker = state.unread_marker(channel_id);
            // Now, not at the next render: a deferred read check could run first, while the
            // list still follows the latest message, and mark the whole run read.
            self.rebuild_rows(cx);
            cx.notify();
            return;
        }
        let newest_id = state.messages(channel_id).last().map(|message| message.id);
        let is_behind = newest_id
            .is_some_and(|newest| state.last_read_message_id(channel_id) < Some(newest))
            || state.mention_count(channel_id) > 0;
        if is_behind && self.is_at_latest() && window.is_window_active() {
            let session = self.session.clone();
            cx.defer(move |cx| {
                session.update(cx, |session, cx| session.mark_channel_read(channel_id, cx));
            });
        }
        let status = self.session.read(cx).history_status(channel_id);
        let distance_from_top = -f32::from(self.list.scroll_px_offset_for_scrollbar().y);
        let has_older = self.session.read(cx).state().has_older_history(channel_id);
        if has_messages
            && rules::should_load_older_history(
                distance_from_top,
                COMPONENT_MESSAGE_HISTORY_LOAD_DISTANCE,
                has_older,
                status.is_loading_older || status.has_failed,
            )
        {
            let session = self.session.clone();
            cx.defer(move |cx| {
                session.update(cx, |session, cx| {
                    session.load_older_messages(channel_id, cx);
                });
            });
        }
    }

    /// Esc in an empty composer, or the unread bar's "Mark as read": the bar and divider go and
    /// the channel is read.
    fn mark_as_read(&mut self, cx: &mut Context<Self>) {
        self.visit.unread_marker = None;
        self.are_rows_stale = true;
        if let Some(channel_id) = self.open_channel_id() {
            self.session
                .update(cx, |session, cx| session.mark_channel_read(channel_id, cx));
        }
        cx.notify();
    }

    /// "Jump to present", Esc in the list, Ctrl+End.
    fn jump_to_present(&mut self, cx: &mut Context<Self>) {
        self.list.set_follow_mode(gpui::FollowMode::Tail);
        self.visit.latest_id_when_leaving_present = None;
        if self.visit.has_reader_scrolled {
            self.visit.unread_marker = None;
        }
        if let Some(channel_id) = self.open_channel_id() {
            self.session
                .update(cx, |session, cx| session.trim_history(channel_id, cx));
        }
        self.are_rows_stale = true;
        cx.notify();
    }

    // ---- The composer ----

    fn follow_composer(
        &mut self,
        _: &Entity<Composer>,
        event: &ComposerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ComposerEvent::Send(message) => {
                self.action_error = None;
                let answer = self.session.update(cx, |session, cx| {
                    session.send_message(
                        message.channel_id,
                        message.text.clone(),
                        message.reply_to,
                        message.attachments.clone(),
                        cx,
                    )
                });
                if let Err(error) = answer {
                    // Nothing was queued: the text goes back rather than vanishing.
                    tracing::warn!(%error, "cannot queue a message");
                    let text = message.text.clone();
                    let channel_id = message.channel_id;
                    self.composer.update(cx, |composer, cx| {
                        composer.give_back_refused_send(channel_id, text, window, cx);
                    });
                    self.action_error = Some(strings(cx).chat().send_failed().into());
                }
                self.jump_to_present(cx);
            }
            ComposerEvent::Edit { message_id, text } => {
                self.save_edit(*message_id, text.clone(), window, cx);
            }
            ComposerEvent::Typing { channel_id } => {
                let channel_id = *channel_id;
                self.session
                    .update(cx, |session, cx| session.start_typing(channel_id, cx));
            }
            ComposerEvent::EditLast => self.edit_last_own_message(window, cx),
            ComposerEvent::Cancel(Cancelled::Nothing) => self.mark_as_read(cx),
            ComposerEvent::Cancel(Cancelled::Editing { .. } | Cancelled::Reply) => {}
        }
        cx.notify();
    }

    /// A send refused for good: its text goes back to an empty composer with the reason; when the
    /// composer holds something else, the message stays failed in the list (Retry, Delete).
    fn give_back_refused_send(
        &mut self,
        _: &Entity<Session>,
        refused: &SendRefused,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_channel_id() != Some(refused.channel_id) {
            return;
        }
        let pending = self
            .session
            .read(cx)
            .state()
            .pending_messages(refused.channel_id)
            .find(|pending| pending.nonce == refused.nonce && pending.attachments.is_empty());
        if let Some(text) = pending.map(|pending| pending.content.clone()) {
            let is_given_back = self.composer.update(cx, |composer, cx| {
                composer.give_back_refused_send(refused.channel_id, text, window, cx)
            });
            if is_given_back {
                let nonce = refused.nonce.clone();
                self.session.update(cx, |session, cx| {
                    session.take_refused_message(&nonce, cx);
                });
            }
        }
        self.action_error = Some(refused.reason.clone().into());
        cx.notify();
    }

    fn edit_last_own_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(channel_id) = self.open_channel_id() else {
            return;
        };
        let state = self.session.read(cx).state();
        let current_user_id = state.current_user().map(|user| user.id);
        // A forward, a poll, a checklist or a progress bar cannot be edited as text.
        let last_own = state
            .messages(channel_id)
            .iter()
            .rev()
            .find(|message| {
                Some(message.author.id) == current_user_id
                    && message.forwarded_from.is_none()
                    && message.poll.is_none()
                    && message.checklist.is_none()
                    && message.progress_bar.is_none()
            })
            .map(|message| (message.id, message.content.clone()));
        if let Some((message_id, content)) = last_own {
            self.composer.update(cx, |composer, cx| {
                composer.start_editing(message_id, &content, window, cx);
            });
        }
    }

    fn save_edit(
        &mut self,
        message_id: Snowflake,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(message) = self.cached_message(message_id, cx) else {
            return;
        };
        // Saving an empty message asks to delete it (DESIGN.md §5.5).
        if text.is_empty() {
            self.ask_to_confirm(Confirmation::Delete(message), window, cx);
            return;
        }
        if text == message.content {
            return;
        }
        self.action_error = None;
        let view = cx.entity().downgrade();
        let summary = strings(cx).chat().edit_failed().to_owned();
        let channel_id = message.channel_id;
        let typed = text.clone();
        self.session.update(cx, |session, cx| {
            session.edit_message(
                channel_id,
                message_id,
                text,
                summary,
                move |sentence, cx| {
                    let _ = view.update(cx, |view, cx| {
                        view.show_action_error(channel_id, sentence, cx);
                        // The typed text is not lost: the editor reopens with it, or keeps it.
                        let can_reopen = view.open_channel_id() == Some(channel_id);
                        view.refused_edit = Some((message_id, typed, can_reopen));
                        cx.notify();
                    });
                },
                cx,
            );
        });
    }

    /// An answer that comes back after the reader moved on is not told under another channel.
    fn show_action_error(
        &mut self,
        channel_id: Snowflake,
        sentence: String,
        cx: &mut Context<Self>,
    ) {
        if self.open_channel_id() == Some(channel_id) {
            self.action_error = Some(sentence.into());
            cx.notify();
        }
    }

    // ---- Message actions ----

    fn start_reply(&mut self, message_id: Snowflake, window: &mut Window, cx: &mut Context<Self>) {
        let Some(message) = self.cached_message(message_id, cx) else {
            return;
        };
        let author_name = mellow_client_core::shared_logic::format::display_name(&message.author);
        let target = ReplyTarget {
            message_id,
            author_name: author_name.to_owned().into(),
        };
        self.composer.update(cx, |composer, cx| {
            composer.set_reply_target(Some(target), window, cx);
        });
    }

    fn start_edit(&mut self, message_id: Snowflake, window: &mut Window, cx: &mut Context<Self>) {
        let Some(message) = self.cached_message(message_id, cx) else {
            return;
        };
        self.composer.update(cx, |composer, cx| {
            composer.start_editing(message_id, &message.content, window, cx);
        });
    }

    fn copy_to_clipboard(
        &mut self,
        text: String,
        announcement: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        if let Some(announcement) = announcement {
            self.announcement = announcement.to_owned().into();
        }
        cx.notify();
    }

    /// Delete from the hover bar or the menu: Shift skips the confirmation, as on the web.
    fn delete(
        &mut self,
        message_id: Snowflake,
        skip_confirmation: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(message) = self.cached_message(message_id, cx) else {
            return;
        };
        if skip_confirmation {
            self.delete_now(&message, cx);
        } else {
            self.ask_to_confirm(Confirmation::Delete(message), window, cx);
        }
    }

    fn delete_now(&mut self, message: &Message, cx: &mut Context<Self>) {
        self.action_error = None;
        let view = cx.entity().downgrade();
        let channel_id = message.channel_id;
        let summary = strings(cx).chat().delete_failed().to_owned();
        self.session.update(cx, |session, cx| {
            session.delete_message(
                channel_id,
                message.id,
                summary,
                move |sentence, cx| {
                    let _ = view.update(cx, |view, cx| {
                        view.show_action_error(channel_id, sentence, cx);
                    });
                },
                cx,
            );
        });
    }

    fn toggle_reaction(&mut self, message_id: Snowflake, key: &str, cx: &mut Context<Self>) {
        let Some(channel_id) = self.open_channel_id() else {
            return;
        };
        self.action_error = None;
        let view = cx.entity().downgrade();
        let summary = strings(cx).reactions().failed().to_owned();
        let key = key.to_owned();
        self.session.update(cx, |session, cx| {
            session.toggle_reaction(
                channel_id,
                message_id,
                &key,
                summary,
                move |sentence, cx| {
                    let _ = view.update(cx, |view, cx| {
                        view.show_action_error(channel_id, sentence, cx);
                    });
                },
                cx,
            );
        });
    }

    fn set_pinned_now(&mut self, message: Message, is_pinned: bool, cx: &mut Context<Self>) {
        self.action_error = None;
        let view = cx.entity().downgrade();
        let channel_id = message.channel_id;
        let pins = strings(cx).pins();
        let summary = if is_pinned {
            pins.pin_failed()
        } else {
            pins.unpin_failed()
        }
        .to_owned();
        self.session.update(cx, |session, cx| {
            session.set_pinned(
                message,
                is_pinned,
                summary,
                move |sentence, cx| {
                    let _ = view.update(cx, |view, cx| {
                        view.show_action_error(channel_id, sentence, cx);
                    });
                },
                cx,
            );
        });
    }

    fn retry_message(&mut self, nonce: &str, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, cx| session.retry_message(nonce, cx));
    }

    fn discard_message(&mut self, nonce: &str, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, cx| session.discard_message(nonce, cx));
    }

    /// shortcut: the web's delete dialog previews the message; this one names the action only.
    fn ask_to_confirm(
        &mut self,
        confirmation: Confirmation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let catalogue = strings(cx);
        let channel_label = self
            .open
            .as_ref()
            .map(|open| open.label.clone())
            .unwrap_or_default();
        let dialog = cx.new(|cx| {
            let cancel = catalogue.common().cancel();
            match &confirmation {
                Confirmation::Delete(_) => {
                    let words = catalogue.delete_message();
                    ModalDialog::new(words.title(), cancel, window, cx)
                        .subtitle(format!("{} {}", words.subtitle(), words.tip()))
                        .submit(words.submit(), words.submit(), true)
                }
                Confirmation::Pin(_, true) => {
                    let words = catalogue.pins();
                    ModalDialog::new(words.pin_title(), cancel, window, cx)
                        .subtitle(words.pin_subtitle(&channel_label))
                        .submit(words.pin_submit(), words.pin_submit(), false)
                }
                Confirmation::Pin(_, false) => {
                    let words = catalogue.pins();
                    ModalDialog::new(words.unpin_title(), cancel, window, cx)
                        .subtitle(words.unpin_subtitle())
                        .submit(words.unpin_submit(), words.unpin_submit(), true)
                }
            }
        });
        let subscription = cx.subscribe_in(
            &dialog,
            window,
            move |view, dialog, event: &ModalDialogEvent, window, cx| {
                if *event == ModalDialogEvent::Submitted {
                    // Closed at once, as on the web: a failure shows above the composer.
                    dialog.update(cx, |dialog, cx| dialog.request_close(window, cx));
                    match confirmation.clone() {
                        Confirmation::Delete(message) => view.delete_now(&message, cx),
                        Confirmation::Pin(message, is_pinned) => {
                            view.set_pinned_now(message, is_pinned, cx);
                        }
                    }
                }
                view.confirmation = None;
                cx.notify();
            },
        );
        self.confirmation = Some((dialog, subscription));
        cx.notify();
    }

    /// The message's menu (More, right-click, Shift+F10), with the same items as the web.
    fn open_menu(
        &mut self,
        message_id: Snowflake,
        origin: MenuOrigin,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(message) = self.cached_message(message_id, cx) else {
            return;
        };
        let catalogue = strings(cx);
        let facts = self.message_facts(&message, cx);
        let reaction_keys: Vec<String> = message
            .reactions
            .iter()
            .map(|reaction| reaction.emoji.clone())
            .collect();
        let view = cx.entity().downgrade();
        let entries = message_menu_lines(facts, &reaction_keys)
            .into_iter()
            .map(|line| {
                let MenuLine::Action(action) = line else {
                    return MenuEntry::Separator;
                };
                let view = view.clone();
                let (label, icon, tone) = match &action {
                    MessageMenuAction::ToggleReaction(key) => {
                        let shown = reaction_label(key, &self.sources.custom_emojis, catalogue);
                        (
                            catalogue.reactions().react_with(&shown),
                            IconName::Smiley,
                            MenuItemTone::Normal,
                        )
                    }
                    MessageMenuAction::Reply => (
                        catalogue.messages().reply().to_owned(),
                        IconName::ArrowBendUpLeft,
                        MenuItemTone::Normal,
                    ),
                    MessageMenuAction::Edit => (
                        catalogue.common().edit().to_owned(),
                        IconName::PencilSimple,
                        MenuItemTone::Normal,
                    ),
                    MessageMenuAction::CopyText => (
                        catalogue.messages().copy_text().to_owned(),
                        IconName::Copy,
                        MenuItemTone::Normal,
                    ),
                    MessageMenuAction::Pin => (
                        catalogue.pins().pin().to_owned(),
                        IconName::PushPin,
                        MenuItemTone::Normal,
                    ),
                    MessageMenuAction::Unpin => (
                        catalogue.pins().unpin().to_owned(),
                        IconName::PushPinSlash,
                        MenuItemTone::Normal,
                    ),
                    MessageMenuAction::CopyMessageId => (
                        catalogue.context_menu().copy_message_id().to_owned(),
                        IconName::Copy,
                        MenuItemTone::Normal,
                    ),
                    MessageMenuAction::Delete => (
                        catalogue.context_menu().delete_message().to_owned(),
                        IconName::Trash,
                        MenuItemTone::Danger,
                    ),
                };
                MenuEntry::Item(
                    MenuItem::new(label, icon, move |window, cx| {
                        let _ = view.update(cx, |view, cx| {
                            view.run_menu_action(message_id, action.clone(), window, cx);
                        });
                    })
                    .tone(tone),
                )
            })
            .collect();
        let opener = window.focused(cx);
        let menu = cx.new(|cx| {
            ContextMenu::open(
                catalogue.messages().actions(),
                entries,
                origin,
                opener,
                window,
                cx,
            )
        });
        let subscription = cx.subscribe(&menu, |view, _, _: &gpui::DismissEvent, cx| {
            view.menu = None;
            cx.notify();
        });
        self.menu = Some((menu, subscription, message_id));
        cx.notify();
    }

    fn run_menu_action(
        &mut self,
        message_id: Snowflake,
        action: MessageMenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            MessageMenuAction::ToggleReaction(key) => self.toggle_reaction(message_id, &key, cx),
            MessageMenuAction::Reply => self.start_reply(message_id, window, cx),
            MessageMenuAction::Edit => self.start_edit(message_id, window, cx),
            MessageMenuAction::CopyText => {
                if let Some(message) = self.cached_message(message_id, cx) {
                    self.copy_to_clipboard(message.content, None, cx);
                }
            }
            MessageMenuAction::Pin | MessageMenuAction::Unpin => {
                if let Some(message) = self.cached_message(message_id, cx) {
                    let is_pinned = action == MessageMenuAction::Pin;
                    self.ask_to_confirm(Confirmation::Pin(message, is_pinned), window, cx);
                }
            }
            MessageMenuAction::CopyMessageId => {
                let copied = strings(cx).context_menu().message_id_copied();
                self.copy_to_clipboard(message_id.to_string(), Some(copied), cx);
            }
            MessageMenuAction::Delete => {
                let skip = window.modifiers().shift;
                self.delete(message_id, skip, window, cx);
            }
        }
    }

    /// The menu of the focused row, from the keyboard (Shift+F10), lined up with the row.
    fn open_menu_of_focused_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((index, message_id)) = self.focused_message() else {
            return;
        };
        let Some(bounds) = self.list.bounds_for_item(index) else {
            return;
        };
        let origin = MenuOrigin::Anchor {
            bounds: MenuAnchor {
                top: f32::from(bounds.top()),
                bottom: f32::from(bounds.bottom()),
                left: f32::from(bounds.left()),
                right: f32::from(bounds.right()),
            },
            alignment: MenuAlignment::End,
        };
        self.open_menu(message_id, origin, window, cx);
    }

    // ---- Keyboard in the list ----

    fn focused_message(&self) -> Option<(usize, Snowflake)> {
        let key = self.focused_row_key.as_ref()?;
        self.entries
            .iter()
            .position(|entry| entry.key() == *key)
            .and_then(|index| self.entries[index].message_id().map(|id| (index, id)))
    }

    /// Up and Down move between messages, as on Discord; the list keeps one Tab stop.
    fn move_row_focus(&mut self, step: isize, cx: &mut Context<Self>) {
        let rows: Vec<usize> = (0..self.entries.len())
            .filter(|&index| !matches!(self.entries[index], Entry::HistoryStart))
            .collect();
        let Some(&last) = rows.last() else {
            return;
        };
        let current = self.focused_row_key.as_ref().and_then(|key| {
            rows.iter()
                .position(|&index| self.entries[index].key() == *key)
        });
        let next_position = match current {
            None => rows.len() - 1,
            Some(position) => position.saturating_add_signed(step).min(rows.len() - 1),
        };
        let index = rows.get(next_position).copied().unwrap_or(last);
        self.focused_row_key = Some(self.entries[index].key());
        self.list.scroll_to_reveal_item(index);
        cx.notify();
    }

    /// Page Up and Page Down: most of a screen, so a line of context stays; reaching the end
    /// follows the latest message again.
    fn scroll_page(&mut self, direction: f32, cx: &mut Context<Self>) {
        let page = self.list.viewport_bounds().size.height * PAGE_SCROLL_SHARE;
        let top = -self.list.scroll_px_offset_for_scrollbar().y;
        self.list
            .set_offset_from_scrollbar(gpui::point(px(0.0), -(top + page * direction)));
        let is_at_latest = self.is_at_latest();
        self.note_reader_scrolled(is_at_latest, cx);
    }

    fn on_list_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let is_secondary = if cfg!(target_os = "macos") {
            modifiers.platform
        } else {
            modifiers.control
        };
        let handled = match keystroke.key.as_str() {
            "pageup" if !modifiers.modified() => {
                self.scroll_page(-1.0, cx);
                true
            }
            "pagedown" if !modifiers.modified() => {
                self.scroll_page(1.0, cx);
                true
            }
            "home" if is_secondary => {
                self.visit.has_reader_scrolled = true;
                self.list.scroll_to(ListOffset::default());
                true
            }
            "end" if is_secondary => {
                self.jump_to_present(cx);
                true
            }
            "up" if !modifiers.modified() => {
                self.move_row_focus(-1, cx);
                true
            }
            "down" if !modifiers.modified() => {
                self.move_row_focus(1, cx);
                true
            }
            "escape" if !modifiers.modified() => {
                self.jump_to_present(cx);
                self.focus_composer(window, cx);
                true
            }
            "f10" if modifiers.shift && !modifiers.control && !modifiers.alt => {
                self.open_menu_of_focused_row(window, cx);
                true
            }
            _ => {
                // A printable key goes to the composer, as typing anywhere in the chat does.
                let is_text = !modifiers.control && !modifiers.platform && !modifiers.alt;
                match keystroke.key_char.as_deref().filter(|_| is_text) {
                    Some(text) if !text.chars().any(char::is_control) => {
                        let text = text.to_owned();
                        self.composer.update(cx, |composer, cx| {
                            composer.type_text(&text, window, cx);
                        });
                        true
                    }
                    _ => false,
                }
            }
        };
        if handled {
            cx.stop_propagation();
            cx.notify();
        }
    }

    // ---- Pins ----

    /// The chat header's pin button: opens the panel and asks for the pins, or closes it.
    pub fn toggle_pins_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(channel_id) = self.open_channel_id() else {
            return;
        };
        let is_same_click_as_dismissal = self
            .pins_dismissed_by_pointer_at
            .take()
            .is_some_and(|position| position == window.mouse_position());
        if is_same_click_as_dismissal {
            return;
        }
        self.is_pins_panel_open = !self.is_pins_panel_open;
        if self.is_pins_panel_open {
            self.session
                .update(cx, |session, cx| session.load_pins(channel_id, false, cx));
            window.focus(&self.pins_focus, cx);
        } else {
            self.focus_composer(window, cx);
        }
        cx.notify();
    }

    fn close_pins_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_pins_panel_open {
            self.is_pins_panel_open = false;
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    /// Jump from the pins panel: the message scrolls into view and takes the list's focus.
    /// shortcut: the panel offers Jump only for loaded messages; loading history up to an older
    /// pin (`loadHistoryUntilMessage`) comes with search (M3 step 6).
    fn jump_to_message(
        &mut self,
        message_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_pins_panel_open = false;
        let key = message_id.raw().to_string();
        if let Some(index) = self.entries.iter().position(|entry| entry.key() == key) {
            self.visit.has_reader_scrolled = true;
            self.list.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: px(0.0),
            });
            self.focused_row_key = Some(key);
            window.focus(&self.list_focus, cx);
        }
        cx.notify();
    }
}

/// What a channel lets the reader do here (ADR-0007 bits, only to adapt the interface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // One flag per permission, as the web's derived values.
struct ChannelRights {
    can_send: bool,
    can_attach: bool,
    can_join_reactions: bool,
    can_manage_messages: bool,
    can_manage_pins: bool,
}

/// A reaction's emoji as the menu names it: itself, `:name:`, or "a deleted emoji".
fn reaction_label(
    key: &str,
    custom_emojis: &HashMap<Snowflake, CustomEmoji>,
    catalogue: crate::i18n::Strings,
) -> String {
    use mellow_client_core::shared_logic::custom_emoji::{ReactionAppearance, reaction_appearance};
    match reaction_appearance(key, custom_emojis) {
        ReactionAppearance::Unicode(emoji) => emoji,
        ReactionAppearance::Custom(emoji) => format!(":{}:", emoji.name),
        ReactionAppearance::Deleted => catalogue.reactions().deleted_emoji().to_owned(),
    }
}
