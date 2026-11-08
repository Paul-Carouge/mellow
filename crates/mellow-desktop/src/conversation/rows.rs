//! The list's rows: built from client-core's cached messages and outbox with its row rules
//! (author groups, day and unread dividers), each drawn as a [`MessageItem`] with its hover bar,
//! right-click menu and keyboard ring; and the history start above them (the channel's hero, the
//! skeletons while a page loads, or the failure with "Try again").
//!
//! shortcut: runs of a blocked author are not folded yet (client-core's `collapse_blocked_runs`,
//! ADR-0025), and new messages are not read out to screen readers; both come with the
//! conversations step (M3 step 4).

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher as _};

use gpui::{
    AnyElement, App, Context, FontWeight, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, ParentElement as _, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _, relative,
};
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::markdown::{MarkdownSources, RoleMentionContext};
use mellow_client_core::shared_logic::mentions::mentionable_people;
use mellow_client_core::shared_logic::message_rows::{
    BlockedRun, MessageListEntry, MessageListRow, MessageRowKind, PendingRowFacts, UnreadMarker,
    build_message_rows, collapse_blocked_runs, is_unread_start_loaded,
};
use mellow_client_core::shared_logic::role_colors::{highest_coloured_role, readable_role_colour};
use mellow_protocol::{Message, PublicUser, Snowflake};

use super::rules::{HoverBarAction, hover_bar_actions};
use super::{ConversationView, MessageMarkupSources};
use crate::components::{
    Button, IconButton, IconButtonTone, MenuAlignment, MenuAnchor, MenuOrigin, icon,
    inset_focus_ring, shadow, with_type,
};
use crate::i18n::strings;
use crate::messages::{
    DayDivider, MessageAuthor, MessageBodyKey, MessageItem, MessageItemContent, UnreadDivider,
    day_divider_label, local_date_time, local_day_of, message_summary, picture_animation,
    picture_for_box, reaction_pills, reply_reference, time_of_day_label,
};
use crate::session::now_unix_milliseconds;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_ACTION_BAR_BUTTON_SIZE, COMPONENT_ACTION_BAR_ICON_SIZE,
    COMPONENT_ACTION_BAR_OFFSET_TOP, COMPONENT_ACTION_BAR_OFFSET_TRAILING,
    COMPONENT_ACTION_BAR_RADIUS, COMPONENT_MESSAGE_AVATAR_INSET, COMPONENT_MESSAGE_AVATAR_SIZE,
    COMPONENT_MESSAGE_BLOCKED_ICON_SIZE, COMPONENT_MESSAGE_CHANNEL_START_GAP,
    COMPONENT_MESSAGE_CHANNEL_START_ICON_CIRCLE_SIZE, COMPONENT_MESSAGE_CHANNEL_START_ICON_SIZE,
    COMPONENT_MESSAGE_CHANNEL_START_PADDING_BLOCK, COMPONENT_MESSAGE_GROUP_GAP,
    COMPONENT_MESSAGE_GUTTER, COMPONENT_MESSAGE_PADDING_BLOCK, COMPONENT_MESSAGE_PADDING_TRAILING,
    COMPONENT_MESSAGE_REPLY_AVATAR_SIZE, COMPONENT_MESSAGE_SKELETON_BAR_HEIGHT,
    COMPONENT_MESSAGE_SKELETON_NAME_MAX_WIDTH, MOTION_DELAY_SKELETON, SPACE_MD, SPACE_SM, SPACE_XS,
    TYPE_BODY, TYPE_BODY_LG, TYPE_DISPLAY, TYPE_MESSAGE,
};
use crate::theme::{color, delay, length, theme};

const HISTORY_START_KEY: &str = "history-start";
const INITIAL_SKELETON_COUNT: usize = 5;
const OLDER_HISTORY_SKELETON_COUNT: usize = 3;
/// Varied on purpose, so placeholders read as messages rather than a grid (`SKELETON_SHAPES`).
const SKELETON_SHAPES: [(f32, &[f32]); 5] = [
    (0.75, &[0.72, 0.46]),
    (0.5, &[0.58]),
    (0.9, &[0.86, 0.78, 0.34]),
    (0.6, &[0.4]),
    (0.8, &[0.66, 0.52]),
];

/// One item of the virtual list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Entry {
    /// Above the oldest message: the hero, the skeletons or the failure.
    HistoryStart,
    Row(MessageListRow),
    /// A blocked author's messages, folded (ADR-0025).
    BlockedRun(BlockedRun),
}

impl Entry {
    pub(super) fn key(&self) -> String {
        match self {
            Self::HistoryStart => HISTORY_START_KEY.to_owned(),
            Self::Row(row) => row.key.clone(),
            Self::BlockedRun(run) => run.key.clone(),
        }
    }

    /// Holds the "New" divider: the list opens on it.
    fn is_first_unread(&self) -> bool {
        match self {
            Self::Row(row) => matches!(
                row.kind,
                MessageRowKind::Sent {
                    is_first_unread: true,
                    ..
                }
            ),
            Self::BlockedRun(run) => run.is_first_unread,
            Self::HistoryStart => false,
        }
    }

    pub(super) fn message_id(&self) -> Option<Snowflake> {
        match self {
            Self::Row(MessageListRow {
                kind: MessageRowKind::Sent { message_id, .. },
                ..
            }) => Some(*message_id),
            _ => None,
        }
    }
}

fn version_of(parts: impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    parts.hash(&mut hasher);
    hasher.finish()
}

/// What a message row shows: a change of any of these may change its height.
fn message_version(message: &Message, is_original_cached: bool) -> u64 {
    let reactions: Vec<(&str, u32, bool)> = message
        .reactions
        .iter()
        .map(|reaction| (reaction.emoji.as_str(), reaction.count, reaction.me))
        .collect();
    let attachments: Vec<Snowflake> = message
        .attachments
        .iter()
        .map(|attachment| attachment.id)
        .collect();
    version_of((
        message.edited_at_unix_milliseconds,
        &message.content,
        reactions,
        attachments,
        message.pinned,
        message.reply_to_id,
        is_original_cached,
        message.link_previews.len(),
    ))
}

/// What a folded run shows: a change of any of these may change its height.
fn blocked_run_version(run: &BlockedRun) -> u64 {
    version_of((
        run.is_revealed,
        &run.message_ids,
        run.is_first_unread,
        run.day_start,
    ))
}

/// The signed-in user as a message author.
pub(super) fn current_user_as_author(session: &crate::session::Session) -> Option<PublicUser> {
    session.state().current_user().map(|user| PublicUser {
        id: user.id,
        username: user.username.clone(),
        display_name: user.display_name.clone(),
        avatar: user.avatar,
        banner: user.banner,
    })
}

impl ConversationView {
    /// People, emoji and roles messages are read against: the open server's members, or the
    /// conversation's people, with the reader; every known server's emoji and roles.
    fn rebuild_sources(&mut self, cx: &App) {
        self.are_sources_stale = false;
        let session = self.session.read(cx);
        let state = session.state();
        let mut people: Vec<PublicUser> = current_user_as_author(session).into_iter().collect();
        let mut role_colours = HashMap::new();
        match self.open_conversation.as_ref() {
            Some(super::OpenConversation {
                guild_id: Some(guild_id),
                ..
            }) => {
                let roles = state
                    .guild(*guild_id)
                    .map_or(&[][..], |guild| guild.roles.as_slice());
                for member in state.members(*guild_id) {
                    if let Some(role) = highest_coloured_role(roles, &member.role_ids) {
                        role_colours.insert(member.user.id, role.color);
                    }
                    people.push(member.user.clone());
                }
            }
            Some(open) => {
                if let Some(conversation) = state.direct_message_channel(open.channel_id) {
                    people.extend(conversation.recipients.iter().cloned());
                }
            }
            None => {}
        }
        let mut custom_emojis = HashMap::new();
        let mut roles = HashMap::new();
        for guild in state.guilds() {
            custom_emojis.extend(guild.emojis.iter().map(|emoji| (emoji.id, emoji.clone())));
            roles.extend(guild.roles.iter().map(|role| (role.id, role.clone())));
        }
        self.sources = MessageMarkupSources {
            generation: self.sources.generation + 1,
            people: mentionable_people(&people),
            custom_emojis,
            roles,
            role_colours,
        };
        self.body_cache
            .use_sources_generation(self.sources.generation);
    }

    /// Builds the rows again from the state.
    pub(super) fn rebuild_rows(&mut self, cx: &App) {
        self.are_rows_stale = false;
        if self.are_sources_stale {
            self.rebuild_sources(cx);
        }
        let mut entries = Vec::new();
        let mut versions = Vec::new();
        if let Some(channel_id) = self.open_channel_id() {
            let session = self.session.read(cx);
            let state = session.state();
            let messages = state.messages(channel_id);
            let pending: Vec<_> = state.pending_messages(channel_id).collect();
            let pending_facts: Vec<PendingRowFacts<'_>> = pending
                .iter()
                .map(|pending| PendingRowFacts {
                    nonce: &pending.nonce,
                    is_reply: pending.reply_to_id.is_some(),
                })
                .collect();
            let has_reached_start = !state.has_older_history(channel_id);
            // The divider waits until the first new message is loaded, so it never jumps.
            let divider_after = self
                .visit
                .unread_marker
                .map(|marker| UnreadMarker {
                    last_read_message_id: marker.last_read_message_id,
                    newest_at_open_message_id: marker.newest_at_open_message_id,
                })
                .filter(|marker| is_unread_start_loaded(messages, *marker, has_reached_start))
                .map(|marker| marker.last_read_message_id);
            let rows = build_message_rows(
                messages,
                &pending_facts,
                state.current_user().map(|user| user.id),
                now_unix_milliseconds(),
                divider_after,
                has_reached_start,
                local_day_of,
            );
            let status = session.history_status(channel_id);
            entries.push(Entry::HistoryStart);
            versions.push((
                HISTORY_START_KEY.to_owned(),
                version_of((
                    has_reached_start,
                    status,
                    messages.is_empty(),
                    self.sources.generation,
                )),
            ));
            self.announce_latest_message(&rows, messages, state, cx);
            let collapsed = collapse_blocked_runs(
                &rows,
                |user_id| state.is_blocked(user_id),
                &self.revealed_blocked_message_ids,
            );
            for entry in collapsed {
                let row = match entry {
                    MessageListEntry::Row(row) => row,
                    MessageListEntry::BlockedRun(run) => {
                        versions.push((run.key.clone(), blocked_run_version(&run)));
                        entries.push(Entry::BlockedRun(run));
                        continue;
                    }
                };
                let layout = (
                    row.starts_author_group,
                    row.day_start,
                    self.sources.generation,
                );
                let version = match row.kind {
                    MessageRowKind::Sent {
                        message_index,
                        is_first_unread,
                        ..
                    } => messages.get(message_index).map_or(0, |message| {
                        let is_original_cached = message
                            .reply_to_id
                            .is_some_and(|id| state.cached_message(channel_id, id).is_some());
                        version_of((
                            layout,
                            is_first_unread,
                            message_version(message, is_original_cached),
                        ))
                    }),
                    MessageRowKind::Pending { pending_index } => version_of((
                        layout,
                        pending
                            .get(pending_index)
                            .map(|pending| pending.status as u8),
                    )),
                };
                versions.push((row.key.clone(), version));
                entries.push(Entry::Row(row));
            }
        }
        self.apply_rows(entries, versions);
    }

    /// Takes the rebuilt rows and tells the list what moved.
    fn apply_rows(&mut self, entries: Vec<Entry>, versions: Vec<(String, u64)>) {
        let update = super::rules::list_update(&self.entry_versions, &versions);
        // An older page lands while the reader is in the history start: the first old row keeps
        // its place at the top instead of being pushed down by the page.
        let scroll_top = self.list.logical_scroll_top();
        let is_reading_history_start =
            scroll_top.item_ix == 0 && !self.entry_versions.is_empty() && !self.is_at_latest();
        if let Some((range, count)) = update.splice.clone() {
            self.list.splice(range.clone(), count);
            if is_reading_history_start && range == (1..1) && count > 0 {
                self.list.scroll_to(gpui::ListOffset {
                    item_ix: 1 + count,
                    offset_in_item: gpui::px(0.0),
                });
            }
        }
        for index in update.remeasure {
            self.list.remeasure_items(index..index + 1);
        }
        self.entries = entries;
        self.entry_versions = versions;
        self.open_on_unread_divider_once();
    }

    /// New messages from others are read out politely (`announceLatestMessage`); the first page,
    /// the reader's own messages and someone they blocked (ADR-0025) are not.
    fn announce_latest_message(
        &mut self,
        rows: &[MessageListRow],
        messages: &[Message],
        state: &mellow_client_core::state::ClientState,
        cx: &App,
    ) {
        let Some(MessageListRow {
            key,
            kind: MessageRowKind::Sent { message_index, .. },
            ..
        }) = rows.last()
        else {
            return;
        };
        let Some(message) = messages.get(*message_index) else {
            return;
        };
        let is_new = self
            .visit
            .last_announced_key
            .as_ref()
            .is_some_and(|announced| announced != key);
        let is_from_someone_else = state
            .current_user()
            .is_none_or(|user| user.id != message.author.id);
        if is_new && is_from_someone_else && !state.is_blocked(message.author.id) {
            let catalogue = strings(cx);
            let author = mellow_client_core::shared_logic::format::display_name(&message.author);
            let summary = message_summary(message, &self.sources.roles, catalogue);
            self.announcement = catalogue.messages().author_said(author, &summary).into();
        }
        self.visit.last_announced_key = Some(key.clone());
    }

    /// Opening a channel with unread messages shows the first of them (DESIGN.md §5.4); when they
    /// all fit, the list stays at the latest message.
    fn open_on_unread_divider_once(&mut self) {
        if self.visit.has_opened_on_divider || self.visit.unread_marker.is_none() {
            return;
        }
        let divider_index = self.entries.iter().position(Entry::is_first_unread);
        if let Some(index) = divider_index {
            self.visit.has_opened_on_divider = true;
            self.list.scroll_to(gpui::ListOffset {
                item_ix: index,
                offset_in_item: gpui::px(0.0),
            });
        }
    }

    /// The list's item at `index`.
    pub(super) fn render_entry(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.entries.get(index).cloned() {
            Some(Entry::HistoryStart) => self.render_history_start(window, cx),
            Some(Entry::Row(row)) => self.render_row(index, &row, window, cx),
            Some(Entry::BlockedRun(run)) => self.render_blocked_run(&run, window, cx),
            None => div().into_any_element(),
        }
    }

    #[allow(clippy::too_many_lines)] // One item and its four states, as the web's markup.
    fn render_history_start(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let Some(open) = self.open_conversation.clone() else {
            return div().into_any_element();
        };
        let session = self.session.read(cx);
        let state = session.state();
        let channel_id = open.channel_id;
        let has_reached_start = !state.has_older_history(channel_id);
        let is_initial = state.messages(channel_id).is_empty();
        let status = session.history_status(channel_id);
        let container = div().w_full().pt(length(SPACE_MD));
        if has_reached_start {
            return container
                .id(HISTORY_START_KEY)
                .role(Role::Group)
                .aria_label(SharedString::from(
                    catalogue.messages().start_of(&open.label),
                ))
                .flex()
                .flex_col()
                .gap(length(COMPONENT_MESSAGE_CHANNEL_START_GAP))
                .px(length(SPACE_MD))
                .pb(length(SPACE_XS))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(length(COMPONENT_MESSAGE_CHANNEL_START_ICON_CIRCLE_SIZE))
                        .rounded_full()
                        .bg(color(
                            tokens.component_message_channel_start_icon_background,
                        ))
                        .child(icon(
                            open.hero_icon,
                            COMPONENT_MESSAGE_CHANNEL_START_ICON_SIZE,
                            color(tokens.component_message_channel_start_icon),
                        )),
                )
                .child(
                    with_type(div(), TYPE_DISPLAY)
                        .mt(length(SPACE_XS))
                        .id("history-start-title")
                        .role(Role::Heading)
                        .aria_level(2)
                        .text_color(color(tokens.component_message_channel_start_title_text))
                        .child(open.hero_title.clone()),
                )
                .child(
                    with_type(div(), TYPE_BODY_LG)
                        .text_color(color(
                            tokens.component_message_channel_start_description_text,
                        ))
                        .child(open.hero_sentence.clone()),
                )
                .when(!open.topic.is_empty(), |hero| {
                    hero.child(
                        with_type(div(), TYPE_BODY_LG)
                            .text_color(color(
                                tokens.component_message_channel_start_description_text,
                            ))
                            .child(open.topic.clone()),
                    )
                })
                .into_any_element();
        }
        if status.has_failed {
            let session = self.session.clone();
            let sentence = if is_initial {
                catalogue.messages().load_failed(&open.label)
            } else {
                catalogue.messages().load_older_failed().to_owned()
            };
            return container
                .id(HISTORY_START_KEY)
                .role(Role::Alert)
                .flex()
                .flex_col()
                .items_start()
                .gap(length(SPACE_SM))
                .pl(length(COMPONENT_MESSAGE_AVATAR_INSET))
                .pr(length(COMPONENT_MESSAGE_PADDING_TRAILING))
                .py(length(COMPONENT_MESSAGE_CHANNEL_START_PADDING_BLOCK))
                .child(
                    with_type(div(), TYPE_MESSAGE)
                        .text_color(color(tokens.color_text_muted))
                        .child(sentence),
                )
                .child(
                    Button::new("history-try-again", catalogue.common().try_again()).on_click(
                        move |_, cx| {
                            session.update(cx, |session, cx| {
                                session.retry_loading_history(channel_id, cx);
                            });
                        },
                    ),
                )
                .into_any_element();
        }
        let is_loading = status.is_loading_latest || status.is_loading_older || is_initial;
        // Skeletons only after a short delay, so a fast page does not flash them.
        let is_delay_elapsed = *window
            .use_keyed_state(
                SharedString::from(format!("skeleton-delay-{}", channel_id.raw())),
                cx,
                |_, cx| {
                    cx.spawn(async move |elapsed, cx| {
                        cx.background_executor()
                            .timer(delay(MOTION_DELAY_SKELETON))
                            .await;
                        let _ = elapsed.update(cx, |elapsed, cx| {
                            *elapsed = true;
                            cx.notify();
                        });
                    })
                    .detach();
                    false
                },
            )
            .read(cx);
        if !is_loading || !is_delay_elapsed {
            return container.into_any_element();
        }
        let count = if is_initial {
            INITIAL_SKELETON_COUNT
        } else {
            OLDER_HISTORY_SKELETON_COUNT
        };
        let fill = color(tokens.component_message_skeleton_fill);
        let line_height = TYPE_MESSAGE.font_size * TYPE_MESSAGE.line_height;
        let bar = |width: gpui::DefiniteLength| {
            div().h(length(line_height)).flex().items_center().child(
                div()
                    .h(length(COMPONENT_MESSAGE_SKELETON_BAR_HEIGHT))
                    .w(width)
                    .rounded_full()
                    .bg(fill),
            )
        };
        container
            .id(HISTORY_START_KEY)
            .role(Role::Status)
            .aria_label(catalogue.messages().loading())
            .children(SKELETON_SHAPES.iter().take(count).map(|(name, lines)| {
                div()
                    .relative()
                    .mt(length(COMPONENT_MESSAGE_GROUP_GAP))
                    .pl(length(COMPONENT_MESSAGE_GUTTER))
                    .pr(length(COMPONENT_MESSAGE_PADDING_TRAILING))
                    .py(length(COMPONENT_MESSAGE_PADDING_BLOCK))
                    .child(
                        div()
                            .absolute()
                            .left(length(COMPONENT_MESSAGE_AVATAR_INSET))
                            .size(length(COMPONENT_MESSAGE_AVATAR_SIZE))
                            .rounded_full()
                            .bg(fill),
                    )
                    .child(bar(length(
                        COMPONENT_MESSAGE_SKELETON_NAME_MAX_WIDTH * name,
                    )
                    .into()))
                    .children(lines.iter().map(|line| bar(relative(*line))))
            }))
            .into_any_element()
    }

    /// A user's picture at `side` token pixels, when they have one.
    fn author_picture(
        user: &PublicUser,
        side: f32,
        window: &Window,
        cx: &App,
    ) -> Option<gpui::ImageSource> {
        user.avatar.as_ref().map(|avatar| {
            picture_for_box(
                DownloadableFile::ProfileImage(avatar.id),
                side,
                side,
                picture_animation(cx),
                window,
            )
        })
    }

    fn author_colour(&self, user_id: Snowflake, cx: &App) -> Option<Hsla> {
        let surface = theme(cx).color_surface_chat >> 8;
        self.sources
            .role_colours
            .get(&user_id)
            .map(|rgb| color((readable_role_colour(*rgb, surface) << 8) | 0xFF))
    }

    #[allow(clippy::too_many_lines)] // One row: its dividers, the message and its hover bar.
    fn render_row(
        &mut self,
        index: usize,
        row: &MessageListRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let catalogue = strings(cx);
        let Some(open) = self.open_conversation.clone() else {
            return div().into_any_element();
        };
        let channel_id = open.channel_id;
        let now = now_unix_milliseconds();
        let view = cx.entity().downgrade();
        let session = self.session.clone();
        let session = session.read(cx);
        let state = session.state();
        let reader_id = state.current_user().map(|user| user.id);

        let (sent_at, is_first_unread) = match row.kind {
            MessageRowKind::Sent {
                sent_at,
                is_first_unread,
                ..
            } => (Some(sent_at), is_first_unread),
            MessageRowKind::Pending { .. } => (None, false),
        };
        let time_of_first_unread: Option<SharedString> = is_first_unread
            .then_some(sent_at)
            .flatten()
            .map(|sent_at| time_of_day_label(local_date_time(sent_at), catalogue).into());
        let divider: Option<AnyElement> = if let Some(day) = row.day_start {
            Some(
                DayDivider::new(day_divider_label(day, catalogue))
                    .new_since(time_of_first_unread)
                    .into_any_element(),
            )
        } else {
            time_of_first_unread.map(|time| UnreadDivider::new(time).into_any_element())
        };

        let row_id = SharedString::from(row.key.clone());
        let (item, message_id) = match row.kind {
            MessageRowKind::Sent { message_index, .. } => {
                let Some(message) = state.messages(channel_id).get(message_index) else {
                    return div().into_any_element();
                };
                let is_everyone_mentioned = open
                    .guild_id
                    .is_some_and(|guild_id| message.mention_role_ids.contains(&guild_id));
                let body = self.body_cache.body(
                    &MessageBodyKey::Sent {
                        id: message.id,
                        edited_at_unix_milliseconds: message.edited_at_unix_milliseconds,
                        is_everyone_mentioned,
                    },
                    &message.content,
                    MarkdownSources {
                        people: &self.sources.people,
                        custom_emojis: &self.sources.custom_emojis,
                        role_mentions: RoleMentionContext {
                            roles: &self.sources.roles,
                            is_everyone_mentioned,
                        },
                    },
                    reader_id,
                );
                let originals: HashMap<Snowflake, &Message> = message
                    .reply_to_id
                    .and_then(|id| state.cached_message(channel_id, id))
                    .map(|original| (original.id, original))
                    .into_iter()
                    .collect();
                let reply = reply_reference(
                    message,
                    &originals,
                    |user_id| state.is_blocked(user_id),
                    &self.sources.roles,
                    &mut self.body_cache,
                    catalogue,
                )
                .map(|reference| {
                    let picture = reference.author.as_ref().and_then(|author| {
                        Self::author_picture(
                            author,
                            COMPONENT_MESSAGE_REPLY_AVATAR_SIZE,
                            window,
                            cx,
                        )
                    });
                    (reference, picture)
                });
                let is_mentioning_reader = Some(message.author.id) != reader_id
                    && (state.does_message_mention_current_user(message) || body.names_reader);
                let rights = self.permissions(channel_id, cx);
                let message_id = message.id;
                let toggle_view = view.clone();
                let item = MessageItem::new(
                    row_id.clone(),
                    MessageItemContent {
                        author: MessageAuthor {
                            user: message.author.clone(),
                            picture: Self::author_picture(
                                &message.author,
                                COMPONENT_MESSAGE_AVATAR_SIZE,
                                window,
                                cx,
                            ),
                            colour: self.author_colour(message.author.id, cx),
                        },
                        sent_at_unix_milliseconds: sent_at,
                        now_unix_milliseconds: now,
                        body,
                        attachments: message.attachments.clone(),
                        reactions: reaction_pills(&message.reactions, &self.sources.custom_emojis),
                        reply,
                        edited_at_unix_milliseconds: message.edited_at_unix_milliseconds,
                        is_pinned: message.pinned,
                        starts_author_group: row.starts_author_group,
                        is_mentioning_reader,
                        pending: None,
                    },
                )
                .can_join_reactions(rights.can_join_reactions)
                .on_toggle_reaction(move |key, _, cx| {
                    let _ = toggle_view
                        .update(cx, |view, cx| view.toggle_reaction(message_id, key, cx));
                });
                (item, Some(message_id))
            }
            MessageRowKind::Pending { pending_index } => {
                let Some(pending) = state.pending_messages(channel_id).nth(pending_index) else {
                    return div().into_any_element();
                };
                let Some(author) = current_user_as_author(session) else {
                    return div().into_any_element();
                };
                let body = self.body_cache.body(
                    &MessageBodyKey::Pending {
                        nonce: pending.nonce.clone(),
                    },
                    &pending.content,
                    MarkdownSources {
                        people: &self.sources.people,
                        custom_emojis: &self.sources.custom_emojis,
                        role_mentions: RoleMentionContext {
                            roles: &self.sources.roles,
                            is_everyone_mentioned: false,
                        },
                    },
                    reader_id,
                );
                let retry_nonce = pending.nonce.clone();
                let discard_nonce = pending.nonce.clone();
                let retry_view = view.clone();
                let discard_view = view.clone();
                let picture =
                    Self::author_picture(&author, COMPONENT_MESSAGE_AVATAR_SIZE, window, cx);
                let item = MessageItem::new(
                    row_id.clone(),
                    MessageItemContent {
                        author: MessageAuthor {
                            colour: self.author_colour(author.id, cx),
                            user: author,
                            picture,
                        },
                        sent_at_unix_milliseconds: None,
                        now_unix_milliseconds: now,
                        body,
                        attachments: pending.attachments.clone(),
                        reactions: Vec::new(),
                        reply: None,
                        edited_at_unix_milliseconds: None,
                        is_pinned: false,
                        starts_author_group: row.starts_author_group,
                        is_mentioning_reader: false,
                        pending: Some(pending.status),
                    },
                )
                .on_retry(move |_, cx| {
                    let _ = retry_view.update(cx, |view, cx| view.retry_message(&retry_nonce, cx));
                })
                .on_discard(move |_, cx| {
                    let _ = discard_view
                        .update(cx, |view, cx| view.discard_message(&discard_nonce, cx));
                });
                (item, None)
            }
        };

        let is_focused_row = self.focused_row_key.as_deref() == Some(row.key.as_str())
            && window.last_input_was_keyboard();
        // The bar stays while focus is in it, so Tab can reach its buttons.
        let has_keyboard_focus_within =
            is_focused_row && self.list_focus.contains_focused(window, cx);
        let is_keyboard_focused = is_focused_row && self.list_focus.is_focused(window);
        let is_menu_open = message_id.is_some_and(|id| {
            self.menu
                .as_ref()
                .is_some_and(|(_, _, menu_message_id)| *menu_message_id == id)
        });
        let hover_bar = message_id
            .filter(|id| {
                self.hovered_message_id == Some(*id) || is_menu_open || has_keyboard_focus_within
            })
            .and_then(|id| self.render_hover_bar(index, id, cx));
        let hover_id = message_id;
        let menu_id = message_id;
        div()
            .w_full()
            .flex()
            .flex_col()
            .children(divider)
            .child(
                div()
                    .id(SharedString::from(format!("row-{}", row.key)))
                    .relative()
                    .w_full()
                    .on_hover(cx.listener(move |view, is_hovered: &bool, _, cx| {
                        if *is_hovered {
                            view.hovered_message_id = hover_id;
                        } else if view.hovered_message_id == hover_id {
                            view.hovered_message_id = None;
                        }
                        cx.notify();
                    }))
                    .when_some(menu_id, |row, message_id| {
                        row.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                                let origin = MenuOrigin::Point {
                                    x: f32::from(event.position.x),
                                    y: f32::from(event.position.y),
                                };
                                view.open_menu(message_id, origin, window, cx);
                                cx.stop_propagation();
                            }),
                        )
                    })
                    .child(item)
                    .children(hover_bar)
                    .when(is_keyboard_focused, |row| {
                        row.child(inset_focus_ring(0.0, cx))
                    }),
            )
            .into_any_element()
    }

    /// A blocked author's run, folded (ADR-0025): one row with Show and Hide. It holds the list's
    /// one Tab stop like a message, where Enter or Space toggles it, so its button stays out of
    /// the Tab order, as on the web.
    fn render_blocked_run(
        &mut self,
        run: &BlockedRun,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let time_of_first_unread: Option<SharedString> = run
            .is_first_unread
            .then(|| time_of_day_label(local_date_time(run.sent_at), catalogue).into());
        let divider: Option<AnyElement> = if let Some(day) = run.day_start {
            Some(
                DayDivider::new(day_divider_label(day, catalogue))
                    .new_since(time_of_first_unread)
                    .into_any_element(),
            )
        } else {
            time_of_first_unread.map(|time| UnreadDivider::new(time).into_any_element())
        };
        let sentence = catalogue
            .messages()
            .blocked_run(i64::try_from(run.message_ids.len()).unwrap_or(i64::MAX));
        let action = if run.is_revealed {
            catalogue.messages().hide_blocked()
        } else {
            catalogue.messages().show_blocked()
        };
        let is_keyboard_focused = self.focused_row_key.as_deref() == Some(run.key.as_str())
            && window.last_input_was_keyboard()
            && self.list_focus.is_focused(window);
        let message_ids = run.message_ids.clone();
        let is_revealed = run.is_revealed;
        div()
            .w_full()
            .flex()
            .flex_col()
            .children(divider)
            .child(
                with_type(div(), TYPE_BODY)
                    .id(SharedString::from(format!("row-{}", run.key)))
                    .role(Role::ListItem)
                    .aria_label(SharedString::from(sentence.clone()))
                    .aria_expanded(is_revealed)
                    .relative()
                    .mt(length(COMPONENT_MESSAGE_GROUP_GAP))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x(length(SPACE_XS))
                    .pl(length(COMPONENT_MESSAGE_GUTTER))
                    .pr(length(COMPONENT_MESSAGE_PADDING_TRAILING))
                    .py(length(COMPONENT_MESSAGE_PADDING_BLOCK))
                    .text_color(color(tokens.component_message_blocked_text))
                    .child(
                        div()
                            .absolute()
                            .left(length(COMPONENT_MESSAGE_AVATAR_INSET))
                            .w(length(COMPONENT_MESSAGE_AVATAR_SIZE))
                            .flex()
                            .justify_center()
                            .child(icon(
                                IconName::Prohibit,
                                COMPONENT_MESSAGE_BLOCKED_ICON_SIZE,
                                color(tokens.component_message_blocked_text),
                            )),
                    )
                    .child(sentence)
                    .child(
                        div()
                            .id(SharedString::from(format!("toggle-{}", run.key)))
                            .role(Role::Button)
                            .aria_expanded(is_revealed)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color(tokens.component_message_blocked_action_text))
                            .cursor_pointer()
                            .hover(gpui::Styled::underline)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.toggle_blocked_run(&message_ids, is_revealed, cx);
                            }))
                            .child(action),
                    )
                    .when(is_keyboard_focused, |row| {
                        row.child(inset_focus_ring(0.0, cx))
                    }),
            )
            .into_any_element()
    }

    /// Show reveals a run's messages for this visit, by message, so a run an older page extends
    /// stays shown; Hide folds it again.
    pub(super) fn toggle_blocked_run(
        &mut self,
        message_ids: &[Snowflake],
        is_revealed: bool,
        cx: &mut Context<Self>,
    ) {
        for message_id in message_ids {
            if is_revealed {
                self.revealed_blocked_message_ids.remove(message_id);
            } else {
                self.revealed_blocked_message_ids.insert(*message_id);
            }
        }
        self.are_rows_stale = true;
        cx.notify();
    }

    /// Where a menu opened from the hover bar's More goes: by the row from the keyboard, at the
    /// pointer otherwise. Never called while the list draws, which holds its state.
    fn menu_origin_for_row(&self, index: usize, window: &Window) -> MenuOrigin {
        if window.last_input_was_keyboard()
            && let Some(bounds) = self.list.bounds_for_item(index)
        {
            return MenuOrigin::Anchor {
                bounds: MenuAnchor {
                    top: f32::from(bounds.top()),
                    bottom: f32::from(bounds.bottom()),
                    left: f32::from(bounds.left()),
                    right: f32::from(bounds.right()),
                },
                alignment: MenuAlignment::End,
            };
        }
        let pointer = window.mouse_position();
        MenuOrigin::Point {
            x: f32::from(pointer.x),
            y: f32::from(pointer.y),
        }
    }

    fn render_hover_bar(
        &self,
        index: usize,
        message_id: Snowflake,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let message = self.cached_message(message_id, cx)?;
        let facts = self.message_facts(&message, cx);
        let actions = hover_bar_actions(facts, self.is_shift_held);
        let id = |name: &str| SharedString::from(format!("hover-{name}-{}", message_id.raw()));
        let view = cx.entity().downgrade();
        let buttons = actions.into_iter().map(|action| {
            let view = view.clone();
            let (name, icon_name, label, tone) = match action {
                HoverBarAction::Edit => (
                    "edit",
                    IconName::PencilSimple,
                    catalogue.common().edit(),
                    IconButtonTone::Standard,
                ),
                HoverBarAction::Reply => (
                    "reply",
                    IconName::ArrowBendUpLeft,
                    catalogue.messages().reply(),
                    IconButtonTone::Standard,
                ),
                HoverBarAction::CopyText => (
                    "copy",
                    IconName::Copy,
                    catalogue.messages().copy_text(),
                    IconButtonTone::Standard,
                ),
                HoverBarAction::Delete => (
                    "delete",
                    IconName::Trash,
                    catalogue.common().delete(),
                    IconButtonTone::Danger,
                ),
                HoverBarAction::More => (
                    "more",
                    IconName::DotsThree,
                    catalogue.messages().more(),
                    IconButtonTone::Standard,
                ),
            };
            IconButton::new(id(name), icon_name, label)
                .sizes(
                    COMPONENT_ACTION_BAR_BUTTON_SIZE,
                    COMPONENT_ACTION_BAR_ICON_SIZE,
                )
                .tone(tone)
                .on_click(move |window, cx| {
                    let _ = view.update(cx, |view, cx| match action {
                        HoverBarAction::Edit => view.start_edit(message_id, window, cx),
                        HoverBarAction::Reply => view.start_reply(message_id, window, cx),
                        HoverBarAction::CopyText => {
                            if let Some(message) = view.cached_message(message_id, cx) {
                                view.copy_to_clipboard(message.content, None, cx);
                            }
                        }
                        // Held Shift (the reason Delete shows) skips the confirmation.
                        HoverBarAction::Delete => {
                            let skip = window.modifiers().shift;
                            view.delete(message_id, skip, window, cx);
                        }
                        HoverBarAction::More => {
                            {
                                // Read at click time: the list's layout is not readable while it draws.
                                let origin = view.menu_origin_for_row(index, window);
                                view.open_menu(message_id, origin, window, cx);
                            }
                        }
                    });
                })
        });
        Some(
            div()
                .id(id("bar"))
                .role(Role::Toolbar)
                .aria_label(catalogue.messages().actions())
                .absolute()
                .top(length(COMPONENT_ACTION_BAR_OFFSET_TOP))
                .right(length(COMPONENT_ACTION_BAR_OFFSET_TRAILING))
                .flex()
                .rounded(length(COMPONENT_ACTION_BAR_RADIUS))
                .border_1()
                .border_color(color(tokens.component_action_bar_border))
                .bg(color(tokens.component_action_bar_background))
                .shadow(shadow(tokens.component_action_bar_shadow))
                .children(buttons)
                .into_any_element(),
        )
    }
}
