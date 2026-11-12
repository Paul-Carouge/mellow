//! The conversation view (`ChatPane.svelte`, `MessageList.svelte`, DESIGN.md §5.4–§5.5): the
//! open channel's or conversation's messages in a virtual list that opens at the latest message,
//! sticks to it and keeps the rows being read in place while older pages load; the "New" divider
//! and unread bar, the jump bar, the typing line and the composer; each message's hover bar and
//! menu; and the pinned messages panel. It turns the composer's events into the session's
//! requests, and shows what failed above the composer.
//!
//! The shell fills its conversation slot with this view and tells it the place
//! ([`ConversationView::set_place`]); the view applies it in its next render, which has the window.

mod card_dialogs;
mod cards;
#[cfg(test)]
mod cards_tests;
mod checklist_editor;
mod chrome;
mod emoji_reactions;
mod forward_dialog;
mod forwarding;
mod gif_picker_host;
mod pins_panel;
mod poll_voters_dialog;
mod reactions_dialog;
mod rows;
pub mod rules;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use gpui::{
    AnyView, App, AppContext as _, ClipboardItem, Context, Entity, FocusHandle, KeyDownEvent,
    ListAlignment, ListOffset, ListState, SharedString, Subscription, Task, Window, px,
};
use mellow_client_core::shared_logic::context_menus_people::{
    MenuLine, MessageMenuAction, MessageMenuFacts, message_menu_actions,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::mentions::MentionablePeople;
use mellow_client_core::shared_logic::role_mentions::roles_writer_may_mention;
use mellow_client_core::shared_logic::slash_commands::SlashCommandId;
use mellow_client_core::state::{
    ClientState, MAXIMUM_HISTORY_PAGE_SIZE, MAXIMUM_PAGES_LOADED_TO_REACH_A_MESSAGE, StateChange,
    UnreadMarker, is_poll_ended,
};
use mellow_protocol::{CustomEmoji, Message, Permissions, Role, Snowflake};

use crate::components::{
    ContextMenu, MenuAlignment, MenuAnchor, MenuEntry, MenuItem, MenuItemTone, MenuOrigin,
    ModalDialog, ModalDialogEvent, Tooltip, is_context_menu_keystroke,
};
use crate::composer::{
    BlockedRecipient, Cancelled, Composer, ComposerEvent, ReplyTarget, SavedDraftsFile,
};
use crate::i18n::strings;
use crate::messages::{MessageBodyCache, ReactionPill};
use crate::session::{
    HistoryReach, HistoryStatus, SendRefused, Session, StateChanged, now_unix_milliseconds,
};
use crate::theme::delay;
use crate::theme::icons::IconName;
use crate::theme::tokens::{COMPONENT_MESSAGE_HISTORY_LOAD_DISTANCE, MOTION_DELAY_JUMP_HIGHLIGHT};

use self::card_dialogs::CardDialogKind;
use self::emoji_reactions::is_reaction_given;
pub use self::forwarding::OpenForwardOrigin;
use self::rows::Entry;
use self::rules::reactor_sentence;

/// Rows measured beyond the visible part, so scrolling and older pages do not pop in.
const LIST_OVERDRAW: f32 = COMPONENT_MESSAGE_HISTORY_LOAD_DISTANCE;
/// Page Up and Page Down scroll this share of the list's height.
const PAGE_SCROLL_SHARE: f32 = 0.9;
/// Who-reacted sentences kept before the cache starts over: one per pill hovered in a visit.
const MAXIMUM_REACTOR_SENTENCES: usize = 256;
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
    /// The newest row already read out: rows that come after it are new to announce.
    last_announced_key: Option<String>,
}

/// What a confirmation dialog will do once confirmed.
#[derive(Debug, Clone)]
enum Confirmation {
    Delete(Message),
    Pin(Message, bool),
    RemoveAllReactions(Message),
}

/// The words, people and roles messages are read against, rebuilt when they change.
#[derive(Default)]
struct MessageMarkupSources {
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
    open_conversation: Option<OpenConversation>,
    /// Asked by the shell; applied by the next render.
    requested_place: Option<OpenConversation>,
    list: ListState,
    entries: Vec<Entry>,
    /// Each entry's key and version, to splice and remeasure only what changed.
    entry_versions: Vec<(String, u64)>,
    are_rows_stale: bool,
    /// The open channel's loading state the rows were last built for.
    shown_history_status: Option<HistoryStatus>,
    are_sources_stale: bool,
    sources: MessageMarkupSources,
    body_cache: MessageBodyCache,
    visit: Visit,
    /// A blocked author's messages the reader chose to show, for this visit only.
    revealed_blocked_message_ids: HashSet<Snowflake>,
    /// The message row under the pointer: its hover bar shows.
    hovered_message_id: Option<Snowflake>,
    /// The row keyboard focus is on, while the list has focus (one Tab stop, arrows move).
    focused_row_key: Option<String>,
    menu: Option<(Entity<ContextMenu>, Subscription, Snowflake)>,
    emoji_picker: Option<emoji_reactions::OpenEmojiPicker>,
    /// Where a click on its opener closed the picker: that click must not reopen it.
    emoji_picker_dismissal: crate::components::OutsideClickDismissal<emoji_reactions::EmojiTarget>,
    /// The open dialog: a confirmation, or who reacted.
    confirmation: Option<(Entity<ModalDialog>, Subscription)>,
    is_pins_panel_open: bool,
    /// Where a click outside closed the pins panel: the toggle under it must not reopen it.
    pins_dismissal: crate::components::OutsideClickDismissal,
    /// The GIF picker while it is open (`gif_picker_host`).
    gif_picker: Option<(Entity<crate::gif_picker::GifPicker>, Subscription)>,
    /// Where a click outside closed the GIF picker: the GIF button under it must not reopen it.
    gif_picker_dismissal: crate::components::OutsideClickDismissal,
    /// What failed, above the composer until the next action or channel.
    action_error: Option<SharedString>,
    /// What went well, said above the composer until the next channel or message sent ("Forwarded
    /// to #news.").
    notice: Option<SharedString>,
    /// A forward's original in another channel, reached once the shell opened that channel.
    pending_origin_jump: Option<(Snowflake, Snowflake)>,
    /// A refused edit's text, given back to the composer by the next render (which has the
    /// window): the message, the text and whether the editor may reopen on it.
    refused_edit: Option<(Snowflake, String, bool)>,
    /// Who reacted, by message and emoji, for the count it had: a new count asks again.
    reactor_sentences: HashMap<(Snowflake, String), (u32, SharedString)>,
    /// A pinned message reached: revealed by the next render, which has the window.
    pending_reveal: Option<Snowflake>,
    /// The message lit after a jump, and which jump lit it.
    highlight: Option<(Snowflake, u64)>,
    /// Counts lit messages, so an earlier one's timer never puts out a later one.
    highlight_generation: u64,
    /// Counts jumps and channel changes: a jump's late answer is dropped once another started.
    jump_generation: u64,
    /// Read out politely (copied, new messages).
    announcement: SharedString,
    is_shift_held: bool,
    list_focus: FocusHandle,
    pins_focus: FocusHandle,
    typing_refresh: Option<Task<()>>,
    cards: cards::CardsState,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for ConversationView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConversationView")
            .field(
                "open",
                &self.open_conversation.as_ref().map(|open| open.channel_id),
            )
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
            open_conversation: None,
            requested_place: None,
            list,
            entries: Vec::new(),
            entry_versions: Vec::new(),
            are_rows_stale: true,
            shown_history_status: None,
            are_sources_stale: true,
            sources: MessageMarkupSources::default(),
            body_cache: MessageBodyCache::new(),
            visit: Visit::default(),
            revealed_blocked_message_ids: HashSet::new(),
            hovered_message_id: None,
            focused_row_key: None,
            menu: None,
            emoji_picker: None,
            emoji_picker_dismissal: crate::components::OutsideClickDismissal::default(),
            confirmation: None,
            is_pins_panel_open: false,
            pins_dismissal: crate::components::OutsideClickDismissal::default(),
            gif_picker: None,
            gif_picker_dismissal: crate::components::OutsideClickDismissal::default(),
            action_error: None,
            notice: None,
            pending_origin_jump: None,
            refused_edit: None,
            reactor_sentences: HashMap::new(),
            pending_reveal: None,
            highlight: None,
            highlight_generation: 0,
            jump_generation: 0,
            announcement: SharedString::default(),
            is_shift_held: false,
            list_focus: cx.focus_handle().tab_stop(true),
            pins_focus: cx.focus_handle(),
            typing_refresh: None,
            cards: cards::CardsState::default(),
            _subscriptions: subscriptions,
        }
    }

    #[must_use]
    pub fn open_channel_id(&self) -> Option<Snowflake> {
        self.open_conversation.as_ref().map(|open| open.channel_id)
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
    pub fn set_place(&mut self, requested_place: Option<OpenConversation>, cx: &mut Context<Self>) {
        if self.requested_place != requested_place {
            self.requested_place = requested_place;
            cx.notify();
        }
    }

    /// The message list's focus, for the shell's `F6`.
    #[must_use]
    pub fn message_list_focus(&self) -> &FocusHandle {
        &self.list_focus
    }

    /// `F6` into the message list: its rows take arrows from there.
    pub fn focus_message_list(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.list_focus, cx);
    }

    /// Puts focus in the composer (after a place opens from the switcher, or Jump to present).
    pub fn focus_composer(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |composer, cx| composer.focus(window, cx));
    }

    /// Applies the place the shell asked for: the previous channel's visit ends, the composer
    /// switches drafts and the new channel's history is asked for.
    fn apply_requested_place(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let requested_place = self.requested_place.clone();
        if requested_place == self.open_conversation {
            return;
        }
        let previous_channel_id = self.open_channel_id();
        let next_channel_id = requested_place.as_ref().map(|open| open.channel_id);
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
            // A sound of the channel left stops and is freed, as the web's goes with the page.
            crate::attachment_audio::stop_attachment_audio(cx);
            self.revealed_blocked_message_ids.clear();
            self.forget_cards_of_channel();
            self.reactor_sentences.clear();
            self.pending_reveal = None;
            self.highlight = None;
            self.jump_generation += 1;
            self.hovered_message_id = None;
            self.focused_row_key = None;
            self.menu = None;
            self.emoji_picker = None;
            self.show_composer_picker_state(cx);
            self.confirmation = None;
            self.is_pins_panel_open = false;
            self.close_gif_picker(cx);
            self.action_error = None;
            self.notice = None;
            self.entries.clear();
            self.entry_versions.clear();
            self.list.reset(0);
            self.list.set_follow_mode(gpui::FollowMode::Tail);
            self.are_sources_stale = true;
        }
        self.open_conversation = requested_place;
        self.are_rows_stale = true;
        let label = self
            .open_conversation
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
        let current_user_id = state.current_user().map(|user| user.id);
        // `permissions` locks the conversation already; the notice names whom to unblock.
        let blocked_recipient = channel_id
            .and_then(|channel_id| state.direct_message_channel(channel_id))
            .and_then(|conversation| {
                rules::blocked_recipient_of(conversation, current_user_id, |user_id| {
                    state.is_blocked(user_id)
                })
            })
            .map(|recipient| BlockedRecipient {
                user_id: recipient.id,
                name: display_name(recipient).to_owned().into(),
            });
        let can_send = permissions.is_none_or(|permissions| permissions.can_send);
        let can_attach = permissions.is_none_or(|permissions| permissions.can_attach);
        let card_rights = channel_id
            .map(|channel_id| self.card_rights(channel_id, cx))
            .map_or((false, false), |rights| {
                (rights.can_create_polls, rights.can_create_checklists)
            });
        let people = match &self.open_conversation {
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
        let (mentionable_roles, can_mention_everyone) =
            role_mentions_for(state, self.open_conversation.as_ref(), channel_id);
        let rest = session.rest_client().cloned();
        // Per account and instance, and only where the session may keep things on this device.
        let kept_drafts_file = rest
            .as_ref()
            .zip(current_user_id)
            .filter(|_| session.is_kept_on_this_device())
            .and_then(|(rest, user_id)| SavedDraftsFile::for_account(rest.instance(), user_id));
        let maximum_attachment_bytes = state.maximum_attachment_bytes();
        self.composer.update(cx, |composer, cx| {
            composer.set_blocked_recipient(blocked_recipient, cx);
            composer.set_permissions(can_send, can_attach, cx);
            composer.set_card_permissions(card_rights.0, card_rights.1, cx);
            composer.set_mention_candidates(people, current_user_id, cx);
            composer.set_role_mentions(mentionable_roles, can_mention_everyone, cx);
            composer.set_uploads(rest, maximum_attachment_bytes, cx);
            composer.keep_drafts_in(kept_drafts_file, cx);
        });
    }

    fn follow_state(&mut self, changes: &[StateChange], cx: &mut Context<Self>) {
        let channel_id = self.open_channel_id();
        let guild_id = self
            .open_conversation
            .as_ref()
            .and_then(|open| open.guild_id);
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
        let conversation = state.direct_message_channel(channel_id);
        let is_conversation = conversation.is_some();
        let current_user_id = state.current_user().map(|user| user.id);
        let is_locked_by_block = conversation.is_some_and(|conversation| {
            rules::blocked_recipient_of(conversation, current_user_id, |user_id| {
                state.is_blocked(user_id)
            })
            .is_some()
        });
        let can_send = has(Permissions::SEND_MESSAGES) && !is_locked_by_block;
        let can_read_history = has(Permissions::READ_MESSAGE_HISTORY);
        let can_manage_messages =
            permissions.is_some_and(|bits| bits.contains(Permissions::MANAGE_MESSAGES));
        ChannelRights {
            can_send,
            can_attach: can_send && has(Permissions::ATTACH_FILES),
            can_join_reactions: can_read_history,
            can_add_reactions: can_read_history
                && permissions.is_some_and(|bits| bits.contains(Permissions::ADD_REACTIONS)),
            can_manage_messages,
            // `can_manage_messages` is only ever true in a server channel.
            can_remove_reactions: can_manage_messages && can_read_history,
            can_manage_pins: is_conversation || can_manage_messages,
        }
    }

    fn message_facts(&self, message: &Message, cx: &App) -> MessageMenuFacts {
        let rights = self.permissions(message.channel_id, cx);
        let current_user_id = self
            .session
            .read(cx)
            .state()
            .current_user()
            .map(|user| user.id);
        MessageMenuFacts {
            is_own: Some(message.author.id) == current_user_id,
            is_forward: message.forwarded_from.is_some(),
            is_rich: message.poll.is_some()
                || message.checklist.is_some()
                || message.progress_bar.is_some(),
            is_poll_open: message
                .poll
                .as_ref()
                .is_some_and(|poll| !is_poll_ended(poll, now_unix_milliseconds())),
            has_text: !message.content.trim().is_empty(),
            is_pinned: message.pinned,
            can_send_messages: rights.can_send,
            can_manage_messages: rights.can_manage_messages,
            has_reactions: !message.reactions.is_empty(),
            can_join_reactions: rights.can_join_reactions,
            can_add_reactions: rights.can_add_reactions,
            can_remove_reactions: rights.can_remove_reactions,
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
                self.notice = None;
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
            ComposerEvent::Unblock(recipient) => self.unblock(recipient, cx),
            ComposerEvent::OpenCardDialog(command) => {
                let kind = match command {
                    SlashCommandId::Poll => Some(CardDialogKind::Poll),
                    SlashCommandId::Checklist => Some(CardDialogKind::Checklist),
                    SlashCommandId::ProgressBar => Some(CardDialogKind::ProgressBar(None)),
                    _ => None,
                };
                if let Some(kind) = kind {
                    self.open_card_dialog(kind, window, cx);
                }
            }
            ComposerEvent::ToggleGifPicker => self.toggle_gif_picker(window, cx),
            ComposerEvent::OpenEmojiPicker { anchor } => self.toggle_emoji_picker(
                emoji_reactions::EmojiTarget::Composer,
                *anchor,
                window,
                cx,
            ),
        }
        cx.notify();
    }

    /// "Unblock" on the composer's notice; a failure is said above the composer.
    fn unblock(&mut self, recipient: &BlockedRecipient, cx: &mut Context<Self>) {
        self.action_error = None;
        let failure = strings(cx).friends().unblock_failed(&recipient.name);
        let view = cx.entity().downgrade();
        let user_id = recipient.user_id;
        self.session.update(cx, |session, cx| {
            session.unblock(user_id, failure, cx, move |answer, cx| {
                let _ = view.update(cx, |view, cx| {
                    let is_unblocked = answer.is_ok();
                    view.action_error = answer.err().map(SharedString::from);
                    view.composer.update(cx, |composer, cx| {
                        composer.finish_unblocking(is_unblocked, cx);
                    });
                    cx.notify();
                });
            });
        });
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
            // A poll, checklist or progress bar has no text to give back: it stays failed in the list.
            .find(|pending| {
                pending.nonce == refused.nonce
                    && pending.attachments.is_empty()
                    && pending.card.is_none()
            });
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

    /// A reaction pill's tooltip: who reacted, asked from the instance only when it shows, and
    /// kept while the count stays the same.
    fn reactors_tooltip(
        &mut self,
        message_id: Snowflake,
        pill: &ReactionPill,
        cx: &mut Context<Self>,
    ) -> AnyView {
        let catalogue = strings(cx);
        let cache_key = (message_id, pill.key.clone());
        if let Some((count, sentence)) = self.reactor_sentences.get(&cache_key)
            && *count == pill.count
        {
            return Tooltip::view(sentence.clone(), cx);
        }
        let tooltip = cx.new(|_| Tooltip::new(catalogue.reactions().loading_who()));
        let Some(channel_id) = self.open_channel_id() else {
            return tooltip.into();
        };
        let reader_id = self
            .session
            .read(cx)
            .state()
            .current_user()
            .map(|user| user.id);
        let emoji = reaction_label(&pill.key, &self.sources.custom_emojis, catalogue);
        let (count, is_mine) = (pill.count, pill.is_mine);
        let shown = tooltip.downgrade();
        let view = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.list_reacting_users(
                channel_id,
                message_id,
                cache_key.1.clone(),
                catalogue.reactions().who_failed().to_owned(),
                move |answer, cx| {
                    let catalogue = strings(cx);
                    let sentence: SharedString = match answer {
                        Ok(users) => {
                            let sentence: SharedString = reactor_sentence(
                                &users, count, is_mine, reader_id, &emoji, catalogue,
                            )
                            .into();
                            let _ = view.update(cx, |view, _| {
                                if view.reactor_sentences.len() >= MAXIMUM_REACTOR_SENTENCES {
                                    view.reactor_sentences.clear();
                                }
                                view.reactor_sentences
                                    .insert(cache_key, (count, sentence.clone()));
                            });
                            sentence
                        }
                        Err(_) => catalogue.reactions().who_failed().into(),
                    };
                    let _ = shown.update(cx, |tooltip, cx| tooltip.set_label(sentence, cx));
                },
                cx,
            );
        });
        tooltip.into()
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

    /// A moderator's "Remove all reactions", once confirmed; the gateway's event clears them.
    fn remove_all_reactions_now(&mut self, message: &Message, cx: &mut Context<Self>) {
        self.action_error = None;
        let view = cx.entity().downgrade();
        let channel_id = message.channel_id;
        let summary = strings(cx).reactions().remove_all_failed().to_owned();
        self.session.update(cx, |session, cx| {
            session.remove_all_reactions(
                message,
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
            .open_conversation
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
                Confirmation::RemoveAllReactions(_) => {
                    let words = catalogue.reactions();
                    ModalDialog::new(words.remove_all_title(), cancel, window, cx)
                        .subtitle(words.remove_all_subtitle())
                        .submit(words.remove_all_submit(), words.remove_all_submit(), true)
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
                        Confirmation::RemoveAllReactions(message) => {
                            view.remove_all_reactions_now(&message, cx);
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

    /// Who reacted with what (the menu's Reactions), in the dialog slot the confirmations use.
    fn open_reactions_dialog(
        &mut self,
        message_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.open_channel_id() else {
            return;
        };
        let catalogue = strings(cx);
        let session = self.session.clone();
        let custom_emojis = self.sources.custom_emojis.clone();
        let can_remove_others = self.permissions(channel_id, cx).can_remove_reactions;
        let body = cx.new(|cx| {
            reactions_dialog::ReactionsDialogBody::new(
                session,
                channel_id,
                message_id,
                custom_emojis,
                can_remove_others,
                cx,
            )
        });
        let dialog = cx.new(|cx| {
            ModalDialog::new(
                catalogue.reactions().reactions(),
                catalogue.common().close(),
                window,
                cx,
            )
            .body(body.into())
        });
        let subscription = cx.subscribe(&dialog, |view, _, _: &ModalDialogEvent, cx| {
            view.confirmation = None;
            cx.notify();
        });
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
        let quick_reactions = self.quick_reactions(cx);
        let view = cx.entity().downgrade();
        let entries = message_menu_actions(facts, &quick_reactions)
            .into_iter()
            .map(|line| {
                let MenuLine::Action(action) = line else {
                    return MenuEntry::Separator;
                };
                let view = view.clone();
                // A quick reaction says whether it is given, as its menuitemcheckbox does on the web.
                let is_given = match &action {
                    MessageMenuAction::ToggleReaction(emoji) => {
                        Some(is_reaction_given(&message, emoji))
                    }
                    _ => None,
                };
                let (label, icon, tone) = message_menu_item_look(&action, catalogue);
                MenuEntry::Item(
                    MenuItem::new(label, icon, move |window, cx| {
                        let _ = view.update(cx, |view, cx| {
                            view.run_menu_action(message_id, action.clone(), window, cx);
                        });
                    })
                    .tone(tone)
                    .checked(is_given),
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
            MessageMenuAction::ToggleReaction(emoji) => {
                self.react_with_quick_reaction(message_id, &emoji, cx);
            }
            MessageMenuAction::AddReaction => {
                self.open_reaction_picker_beside_row(message_id, window, cx);
            }
            MessageMenuAction::Reply => self.start_reply(message_id, window, cx),
            MessageMenuAction::Forward => self.open_forward_dialog(message_id, window, cx),
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
            MessageMenuAction::ViewReactions => self.open_reactions_dialog(message_id, window, cx),
            MessageMenuAction::CopyMessageId => {
                let copied = strings(cx).context_menu().message_id_copied();
                self.copy_to_clipboard(message_id.to_string(), Some(copied), cx);
            }
            MessageMenuAction::EndPoll => self.ask_to_end_poll(message_id, window, cx),
            MessageMenuAction::RemoveAllReactions => {
                if let Some(message) = self.cached_message(message_id, cx) {
                    self.ask_to_confirm(Confirmation::RemoveAllReactions(message), window, cx);
                }
            }
            MessageMenuAction::Delete => {
                let skip = window.modifiers().shift;
                self.delete(message_id, skip, window, cx);
            }
        }
    }

    /// The menu of the focused row, from the keyboard (Shift+F10, the Menu key), lined up with
    /// the row.
    fn open_menu_of_focused_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((index, message_id)) = self.focused_message() else {
            return;
        };
        // A row the list has not measured yet: beside the list instead, never nowhere.
        let bounds = self
            .list
            .bounds_for_item(index)
            .unwrap_or_else(|| self.list.viewport_bounds());
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

    /// The folded run keyboard focus is on: its messages and whether they show.
    fn focused_blocked_run(&self) -> Option<(Vec<Snowflake>, bool)> {
        let key = self.focused_row_key.as_ref()?;
        self.entries.iter().find_map(|entry| match entry {
            Entry::BlockedRun(run) if run.anchor_key == *key => {
                Some((run.message_ids.clone(), run.is_revealed))
            }
            _ => None,
        })
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
            "enter" | "space" if !modifiers.modified() && self.list_focus.is_focused(window) => {
                match self.focused_blocked_run() {
                    Some((message_ids, is_revealed)) => {
                        self.toggle_blocked_run(&message_ids, is_revealed, cx);
                        true
                    }
                    None => false,
                }
            }
            // Copies text selected in the messages; the message's own Copy text stays in the menu.
            "c" if is_secondary && !modifiers.shift && !modifiers.alt => {
                if gpui_base::TextSelection::has_selection(window, cx) {
                    let selected = gpui_base::TextSelection::selected_text(window, cx);
                    cx.write_to_clipboard(ClipboardItem::new_string(selected));
                    true
                } else {
                    false
                }
            }
            _ if is_context_menu_keystroke(keystroke) => {
                self.open_menu_of_focused_row(window, cx);
                true
            }
            // A printable key goes to the composer, as typing anywhere in the chat does; only from
            // the list itself, so Space still activates a focused button inside it.
            _ if self.list_focus.is_focused(window) => {
                let is_text = !modifiers.control && !modifiers.platform && !modifiers.alt;
                match keystroke.key_char.as_deref().filter(|_| is_text) {
                    Some(text) if !text.chars().any(char::is_control) => {
                        let text = text.to_owned();
                        self.composer
                            .update(cx, |composer, cx| composer.type_text(&text, window, cx))
                    }
                    _ => false,
                }
            }
            _ => false,
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
        if self
            .pins_dismissal
            .is_closing_click(window.mouse_position(), &())
        {
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

    /// Jump from the pins panel: older history loads until the message is there (the web's
    /// `loadHistoryUntilMessage`), then it scrolls into view, takes the list's focus and is lit.
    /// Jump to present, Esc or reaching the latest message again trims what was loaded.
    fn jump_to_message(
        &mut self,
        message_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_pins_panel_open = false;
        self.action_error = None;
        let Some(channel_id) = self.open_channel_id() else {
            return;
        };
        self.jump_generation += 1;
        let jump_generation = self.jump_generation;
        let view = cx.entity().downgrade();
        let wanting_view = view.clone();
        self.session.update(cx, |session, cx| {
            session.reach_message(
                channel_id,
                message_id,
                move |cx| {
                    wanting_view
                        .upgrade()
                        .is_some_and(|view| view.read(cx).jump_generation == jump_generation)
                },
                move |reach, cx| {
                    let _ = view.update(cx, |view, cx| {
                        if view.jump_generation == jump_generation {
                            view.finish_reaching(channel_id, message_id, reach, cx);
                        }
                    });
                },
                cx,
            );
        });
        window.focus(&self.list_focus, cx);
        cx.notify();
    }

    fn finish_reaching(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        reach: HistoryReach,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self
            .open_conversation
            .as_ref()
            .filter(|open| open.channel_id == channel_id)
        else {
            return;
        };
        let search = strings(cx).search();
        match reach {
            // Revealed by the next render, once the rows hold the pages just loaded.
            HistoryReach::Loaded => {
                self.pending_reveal = Some(message_id);
                self.are_rows_stale = true;
            }
            HistoryReach::Missing => self.action_error = Some(search.jump_missing().into()),
            HistoryReach::TooFar => {
                let messages = MAXIMUM_PAGES_LOADED_TO_REACH_A_MESSAGE * MAXIMUM_HISTORY_PAGE_SIZE;
                self.action_error =
                    Some(search.jump_too_far(&open.label, i64::from(messages)).into());
            }
            HistoryReach::Failed => self.action_error = Some(search.jump_failed().into()),
        }
        cx.notify();
    }

    /// The message scrolls into view, takes the list's focus and is lit briefly (DESIGN.md 5.12);
    /// a run of someone the reader blocked shows first, as Show would.
    fn reveal_message(
        &mut self,
        message_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let folded_run = self.entries.iter().find_map(|entry| match entry {
            Entry::BlockedRun(run) if !run.is_revealed && run.message_ids.contains(&message_id) => {
                Some(run.message_ids.clone())
            }
            _ => None,
        });
        if let Some(message_ids) = folded_run {
            self.toggle_blocked_run(&message_ids, false, cx);
            self.rebuild_rows(cx);
        }
        let key = message_id.raw().to_string();
        let Some(index) = self.entries.iter().position(|entry| entry.key() == key) else {
            return;
        };
        self.visit.has_reader_scrolled = true;
        self.list.scroll_to(ListOffset {
            item_ix: index,
            offset_in_item: px(0.0),
        });
        self.focused_row_key = Some(key);
        window.focus(&self.list_focus, cx);
        self.highlight_briefly(message_id, cx);
    }

    /// The row takes the match wash, which fades; under reduced motion it holds, then goes.
    fn highlight_briefly(&mut self, message_id: Snowflake, cx: &mut Context<Self>) {
        self.highlight_generation += 1;
        let generation = self.highlight_generation;
        self.highlight = Some((message_id, generation));
        cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(delay(MOTION_DELAY_JUMP_HIGHLIGHT))
                .await;
            let _ = view.update(cx, |view, cx| {
                if view.highlight.is_some_and(|(_, shown)| shown == generation) {
                    view.highlight = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
}

/// The roles the writer may mention in the open server channel, highest first, and whether they
/// may mention `@everyone` (ADR-0040); nothing in a conversation.
fn role_mentions_for(
    state: &ClientState,
    open: Option<&OpenConversation>,
    channel_id: Option<Snowflake>,
) -> (Vec<Role>, bool) {
    let (Some(guild_id), Some(channel_id)) = (open.and_then(|open| open.guild_id), channel_id)
    else {
        return (Vec::new(), false);
    };
    let can_mention_everyone = state
        .my_channel_permissions(channel_id)
        .is_some_and(|bits| bits.contains(Permissions::MENTION_EVERYONE));
    let roles = state
        .guild(guild_id)
        .map(|guild| {
            roles_writer_may_mention(&guild.roles, guild_id, can_mention_everyone)
                .into_iter()
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    (roles, can_mention_everyone)
}

/// What a channel lets the reader do here (ADR-0007 bits, only to adapt the interface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // One flag per permission, as the web's derived values.
struct ChannelRights {
    can_send: bool,
    can_attach: bool,
    can_join_reactions: bool,
    can_add_reactions: bool,
    can_manage_messages: bool,
    can_remove_reactions: bool,
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

/// The label, icon and tone of a message menu item.
fn message_menu_item_look(
    action: &MessageMenuAction,
    catalogue: crate::i18n::Strings,
) -> (String, IconName, MenuItemTone) {
    match action {
        MessageMenuAction::ToggleReaction(emoji) => (
            catalogue.reactions().react_with(emoji),
            IconName::Smiley,
            MenuItemTone::Normal,
        ),
        MessageMenuAction::AddReaction => (
            catalogue.reactions().add_reaction().to_owned(),
            IconName::Smiley,
            MenuItemTone::Normal,
        ),
        MessageMenuAction::Reply => (
            catalogue.messages().reply().to_owned(),
            IconName::ArrowBendUpLeft,
            MenuItemTone::Normal,
        ),
        MessageMenuAction::Forward => (
            catalogue.forward().forward().to_owned(),
            IconName::ArrowBendUpRight,
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
        MessageMenuAction::ViewReactions => (
            catalogue.reactions().reactions().to_owned(),
            IconName::Users,
            MenuItemTone::Normal,
        ),
        MessageMenuAction::CopyMessageId => (
            catalogue.context_menu().copy_message_id().to_owned(),
            IconName::Copy,
            MenuItemTone::Normal,
        ),
        MessageMenuAction::EndPoll => (
            catalogue.polls().end().to_owned(),
            IconName::ChartBarHorizontal,
            MenuItemTone::Danger,
        ),
        MessageMenuAction::RemoveAllReactions => (
            catalogue.reactions().remove_all().to_owned(),
            IconName::XCircle,
            MenuItemTone::Danger,
        ),
        MessageMenuAction::Delete => (
            catalogue.context_menu().delete_message().to_owned(),
            IconName::Trash,
            MenuItemTone::Danger,
        ),
    }
}
