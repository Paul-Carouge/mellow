//! Realistic messages for the component gallery, laid out by client-core's `build_message_rows`
//! as the message list will: author groups, a day divider, the unread line, a reply, pictures,
//! reactions and the outbox. Developer-facing, so its words are English and not translated.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{AnyElement, App, IntoElement, SharedString, Window, prelude::FluentBuilder as _};
use mellow_client_core::shared_logic::markdown::{MarkdownSources, RoleMentionContext};
use mellow_client_core::shared_logic::mentions::{MentionablePeople, mentionable_people};
use mellow_client_core::shared_logic::message_rows::{
    MessageRowKind, PendingRowFacts, build_message_rows,
};
use mellow_client_core::shared_logic::role_colors::readable_role_colour;
use mellow_client_core::state::{PendingMessageStatus, add_reaction, remove_reaction};
use mellow_protocol::{
    Attachment, CustomEmoji, ForwardOrigin, GitHubRepositoryEmbed, LinkEmbed, LinkPreview,
    LinkPreviewImage, MELLOW_EPOCH_UNIX_MILLISECONDS, Message, Permissions, PublicUser,
    ReactionSummary, Role, Snowflake,
};

use super::forward::ForwardOrigin as ForwardOriginShown;
use super::{
    DayDivider, MessageAuthor, MessageBodyCache, MessageBodyKey, MessageItem, MessageItemContent,
    MessageLinkPreviews, UnreadDivider, day_divider_label, local_date_time, reaction_pills,
    reply_reference, time_of_day_label,
};
use crate::i18n;
use crate::theme::{color, theme};

const MINUTE: u64 = 60 * 1000;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;
const MODERATORS_ROLE_ID: u64 = 7;
const MODERATORS_COLOUR: u32 = 0x004F_B39A;
const GUILD_ID: u64 = 900;
const PARTY_EMOJI_ID: u64 = 500;

/// Where the gallery hears what a sample row asks for.
pub(crate) type SampleActionHandler = Rc<dyn Fn(SampleAction, &mut Window, &mut App)>;

/// What a sample row asks the gallery to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SampleAction {
    ToggleReaction { message_id: Snowflake, key: String },
    Retry(String),
    Discard(String),
}

struct PendingSample {
    nonce: String,
    content: String,
    status: PendingMessageStatus,
}

pub(crate) struct MessageSamples {
    reader: PublicUser,
    people: MentionablePeople,
    custom_emojis: HashMap<Snowflake, CustomEmoji>,
    roles: HashMap<Snowflake, Role>,
    messages: Vec<Message>,
    pending: Vec<PendingSample>,
    unread_marker: Snowflake,
    cache: MessageBodyCache,
}

fn person(id: u64, username: &str, display_name: &str) -> PublicUser {
    PublicUser {
        id: Snowflake::from_raw(id),
        username: username.to_owned(),
        display_name: display_name.to_owned(),
        avatar: None,
        banner: None,
    }
}

fn id_at(unix_milliseconds: u64) -> Snowflake {
    Snowflake::from_parts(
        unix_milliseconds.saturating_sub(MELLOW_EPOCH_UNIX_MILLISECONDS),
        0,
    )
    .unwrap_or(Snowflake::from_raw(1))
}

fn message(author: &PublicUser, sent_at: u64, content: &str) -> Message {
    Message {
        id: id_at(sent_at),
        channel_id: Snowflake::from_raw(1),
        author: author.clone(),
        content: content.to_owned(),
        reply_to_id: None,
        edited_at_unix_milliseconds: None,
        nonce: None,
        attachments: Vec::new(),
        reactions: Vec::new(),
        pinned: false,
        mention_ids: Vec::new(),
        mention_role_ids: Vec::new(),
        link_previews: Vec::new(),
        poll: None,
        checklist: None,
        progress_bar: None,
        forwarded_from: None,
    }
}

const CODE_SAMPLE: &str = "```rust
/// Keeps the newest messages of a channel, oldest first.
pub struct History {
    messages: Vec<Message>,
    has_reached_start: bool,
}

impl History {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            has_reached_start: false,
        }
    }

    /// Older messages go before the ones already loaded.
    pub fn prepend(&mut self, older: Vec<Message>) {
        let mut merged = older;
        merged.append(&mut self.messages);
        self.messages = merged;
    }

    pub fn latest(&self) -> Option<&Message> {
        self.messages.last()
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }
}
```";

fn now_unix_milliseconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

impl MessageSamples {
    #[allow(clippy::too_many_lines)] // The sample conversation, message by message.
    pub fn new() -> Self {
        let reader = person(3, "paul", "Paul");
        let ana = person(1, "ana", "Ana");
        let leo = person(2, "leo", "Léo");
        let people = mentionable_people(&[reader.clone(), ana.clone(), leo.clone()]);
        let party = CustomEmoji {
            id: Snowflake::from_raw(PARTY_EMOJI_ID),
            name: "party".into(),
            is_animated: false,
        };
        let moderators = Role {
            id: Snowflake::from_raw(MODERATORS_ROLE_ID),
            name: "Moderators".into(),
            color: MODERATORS_COLOUR,
            position: 1,
            permissions: Permissions::empty(),
            is_displayed_separately: true,
            is_mentionable: true,
            icon: None,
            unicode_emoji: None,
        };
        let now = now_unix_milliseconds();
        let yesterday = now - DAY;
        let today = now - 3 * HOUR;

        let mut announcement = message(
            &ana,
            yesterday,
            "Morning! The **new build** is up: try `mellow --version` and tell me what you see.\n\
             The notes are in the [changelog](https://example.com/changelog), bugs go to https://example.com/issues",
        );
        announcement.reactions = vec![
            ReactionSummary {
                emoji: "👍".into(),
                count: 3,
                me: true,
            },
            ReactionSummary {
                emoji: format!(":{PARTY_EMOJI_ID}"),
                count: 1,
                me: false,
            },
            ReactionSummary {
                emoji: ":404".into(),
                count: 2,
                me: false,
            },
        ];
        let changes = message(
            &ana,
            yesterday + 2 * MINUTE,
            "What changed:\n- Faster history\n- Pictures decoded in the app\n  - AVIF too\n1. Update\n2. Restart",
        );
        let mut question = message(
            &leo,
            today,
            "@Paul can you check the crash on Linux? ||the one with the camera||",
        );
        question.reply_to_id = Some(announcement.id);
        question.pinned = true;
        question.edited_at_unix_milliseconds = Some(today + 4 * MINUTE);
        let mut pictures = message(
            &leo,
            today + MINUTE,
            "> Pictures decoded in the app\nWorks here 🎉 <:party:500>",
        );
        pictures.attachments = vec![
            Attachment {
                id: Snowflake::from_raw(41),
                file_name: "harbour-at-dusk.avif".into(),
                content_type: "image/avif".into(),
                size_bytes: 214_331,
                width: Some(1600),
                height: Some(900),
            },
            Attachment {
                id: Snowflake::from_raw(42),
                file_name: "camera-crash-trace.log".into(),
                content_type: "application/octet-stream".into(),
                size_bytes: 48_213,
                width: None,
                height: None,
            },
        ];
        let code = message(&ana, today + 10 * MINUTE, CODE_SAMPLE);
        let jumbo = message(&ana, today + 11 * MINUTE, "🎉🎉 <:party:500>");
        let mut links = message(
            &ana,
            today + 12 * MINUTE,
            "The repo https://github.com/rust-lang/rust and the talk https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=90",
        );
        links.link_previews = vec![
            LinkPreview {
                id: Snowflake::from_raw(61),
                url: "https://github.com/rust-lang/rust".into(),
                title: Some("rust-lang/rust".into()),
                description: Some(
                    "Empowering everyone to build reliable and efficient software.".into(),
                ),
                site_name: Some("GitHub".into()),
                theme_color: Some(0x001e_2327),
                image: None,
                embed: Some(LinkEmbed::GitHubRepository(GitHubRepositoryEmbed {
                    owner: "rust-lang".into(),
                    name: "rust".into(),
                    stars: 104_238,
                    forks: 13_612,
                    language: Some("Rust".into()),
                    language_color: None,
                    topics: vec!["compiler".into(), "language".into(), "rust".into()],
                    licence_spdx: Some("Apache-2.0".into()),
                    is_archived: false,
                })),
            },
            LinkPreview {
                id: Snowflake::from_raw(62),
                url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=90".into(),
                title: Some("A talk about Rust in Lyon".into()),
                description: None,
                site_name: Some("YouTube".into()),
                theme_color: None,
                image: Some(LinkPreviewImage {
                    width: 480,
                    height: 360,
                }),
                embed: None,
            },
        ];
        let mut passed_on = message(&leo, today + 13 * MINUTE, &announcement.content);
        passed_on.forwarded_from = Some(ForwardOrigin {
            channel_id: announcement.channel_id,
            message_id: announcement.id,
        });
        let mut plan = message(
            &reader,
            today + 20 * MINUTE,
            "# Weekend plan\n-# shared with the team\nAsk <@&7> for a review, then @everyone celebrates *properly*",
        );
        plan.mention_role_ids = vec![
            Snowflake::from_raw(MODERATORS_ROLE_ID),
            Snowflake::from_raw(GUILD_ID),
        ];
        let unread_marker = question.id;

        Self {
            reader,
            people,
            custom_emojis: HashMap::from([(party.id, party)]),
            roles: HashMap::from([(moderators.id, moderators)]),
            messages: vec![
                announcement,
                changes,
                question,
                pictures,
                code,
                jumbo,
                links,
                passed_on,
                plan,
            ],
            pending: vec![
                PendingSample {
                    nonce: "pending-sending".into(),
                    content: "Sending this one now".into(),
                    status: PendingMessageStatus::Sending,
                },
                PendingSample {
                    nonce: "pending-failed".into(),
                    content: "This one did not make it".into(),
                    status: PendingMessageStatus::Failed,
                },
            ],
            unread_marker,
            cache: MessageBodyCache::new(),
        }
    }

    pub fn apply(&mut self, action: &SampleAction) {
        match action {
            SampleAction::ToggleReaction { message_id, key } => {
                if let Some(message) = self
                    .messages
                    .iter_mut()
                    .find(|message| message.id == *message_id)
                {
                    let is_mine = message
                        .reactions
                        .iter()
                        .any(|reaction| reaction.emoji == *key && reaction.me);
                    if is_mine {
                        remove_reaction(&mut message.reactions, key, true);
                    } else {
                        add_reaction(&mut message.reactions, key, true);
                    }
                }
            }
            SampleAction::Retry(nonce) => {
                if let Some(pending) = self
                    .pending
                    .iter_mut()
                    .find(|pending| pending.nonce == *nonce)
                {
                    pending.status = PendingMessageStatus::Sending;
                }
            }
            SampleAction::Discard(nonce) => self.pending.retain(|pending| pending.nonce != *nonce),
        }
    }

    /// The rows as the list will draw them; `on_action` receives what the rows ask for.
    #[allow(clippy::too_many_lines)] // Wiring every part of a row, as the list will.
    pub fn rows(&mut self, on_action: &SampleActionHandler, cx: &App) -> Vec<AnyElement> {
        let strings = i18n::strings(cx);
        let tokens = theme(cx);
        let now = now_unix_milliseconds();
        // Another language words the quotes again.
        self.cache
            .use_sources_generation(u64::from(strings.language() == i18n::Language::French));
        let pending_facts: Vec<PendingRowFacts<'_>> = self
            .pending
            .iter()
            .map(|pending| PendingRowFacts {
                nonce: &pending.nonce,
                is_reply: false,
            })
            .collect();
        let rows = build_message_rows(
            &self.messages,
            &pending_facts,
            Some(self.reader.id),
            now,
            Some(self.unread_marker),
            true,
            super::local_day_of,
        );
        let by_id: HashMap<Snowflake, &Message> = self
            .messages
            .iter()
            .map(|message| (message.id, message))
            .collect();
        let moderator_colour = color(
            (readable_role_colour(MODERATORS_COLOUR, tokens.color_surface_chat >> 8) << 8) | 0xFF,
        );
        let mut elements = Vec::new();
        for row in rows {
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
                .map(|sent_at| time_of_day_label(local_date_time(sent_at), strings).into());
            if let Some(day) = row.day_start {
                elements.push(
                    DayDivider::new(day_divider_label(day, strings))
                        .new_since(time_of_first_unread)
                        .into_any_element(),
                );
            } else if let Some(time) = time_of_first_unread {
                elements.push(UnreadDivider::new(time).into_any_element());
            }
            let item = match row.kind {
                MessageRowKind::Sent { message_index, .. } => {
                    let Some(message) = self.messages.get(message_index) else {
                        continue;
                    };
                    let is_everyone_mentioned = message
                        .mention_role_ids
                        .contains(&Snowflake::from_raw(GUILD_ID));
                    let body = self.cache.body(
                        &MessageBodyKey::Sent {
                            id: message.id,
                            edited_at_unix_milliseconds: message.edited_at_unix_milliseconds,
                            is_everyone_mentioned,
                        },
                        &message.content,
                        MarkdownSources {
                            people: &self.people,
                            custom_emojis: &self.custom_emojis,
                            role_mentions: RoleMentionContext {
                                roles: &self.roles,
                                is_everyone_mentioned,
                            },
                        },
                        Some(self.reader.id),
                    );
                    let reply = reply_reference(
                        message,
                        &by_id,
                        |_| false,
                        &self.roles,
                        &mut self.cache,
                        strings,
                    )
                    .map(|reference| (reference, None));
                    let is_moderator = message.author.username == "ana";
                    let message_id = message.id;
                    let toggle = Rc::clone(on_action);
                    MessageItem::new(
                        SharedString::from(row.key.clone()),
                        MessageItemContent {
                            author: MessageAuthor {
                                user: message.author.clone(),
                                picture: None,
                                colour: is_moderator.then_some(moderator_colour),
                            },
                            sent_at_unix_milliseconds: sent_at,
                            now_unix_milliseconds: now,
                            is_mentioning_reader: message.author.id != self.reader.id
                                && body.names_reader,
                            body,
                            attachments: message.attachments.clone(),
                            reactions: reaction_pills(&message.reactions, &self.custom_emojis),
                            reply,
                            edited_at_unix_milliseconds: message.edited_at_unix_milliseconds,
                            is_pinned: message.pinned,
                            starts_author_group: row.starts_author_group,
                            pending: None,
                        },
                    )
                    .on_toggle_reaction(move |key, window, cx| {
                        toggle(
                            SampleAction::ToggleReaction {
                                message_id,
                                key: key.to_owned(),
                            },
                            window,
                            cx,
                        );
                    })
                    .map(|item| match message.forwarded_from {
                        Some(origin) => item.forward(
                            ForwardOriginShown {
                                place: Some("#general".into()),
                                sent_at_unix_milliseconds: origin.message_id.unix_milliseconds(),
                            },
                            Some(|_: &mut Window, _: &mut App| {}),
                        ),
                        None if !message.link_previews.is_empty() => {
                            item.link_previews(MessageLinkPreviews::new(
                                SharedString::from(format!("previews-{}", message.id.raw())),
                                message.channel_id,
                                message.id,
                                message.link_previews.clone(),
                            ))
                        }
                        None => item,
                    })
                }
                MessageRowKind::Pending { pending_index } => {
                    let Some(pending) = self.pending.get(pending_index) else {
                        continue;
                    };
                    let body = self.cache.body(
                        &MessageBodyKey::Pending {
                            nonce: pending.nonce.clone(),
                        },
                        &pending.content,
                        MarkdownSources {
                            people: &self.people,
                            custom_emojis: &self.custom_emojis,
                            role_mentions: RoleMentionContext {
                                roles: &self.roles,
                                is_everyone_mentioned: false,
                            },
                        },
                        Some(self.reader.id),
                    );
                    let retry = Rc::clone(on_action);
                    let discard = Rc::clone(on_action);
                    let retry_nonce = pending.nonce.clone();
                    let discard_nonce = pending.nonce.clone();
                    MessageItem::new(
                        SharedString::from(row.key.clone()),
                        MessageItemContent {
                            author: MessageAuthor {
                                user: self.reader.clone(),
                                picture: None,
                                colour: None,
                            },
                            sent_at_unix_milliseconds: None,
                            now_unix_milliseconds: now,
                            body,
                            attachments: Vec::new(),
                            reactions: Vec::new(),
                            reply: None,
                            edited_at_unix_milliseconds: None,
                            is_pinned: false,
                            starts_author_group: row.starts_author_group,
                            is_mentioning_reader: false,
                            pending: Some(pending.status),
                        },
                    )
                    .on_retry(move |window, cx| {
                        retry(SampleAction::Retry(retry_nonce.clone()), window, cx);
                    })
                    .on_discard(move |window, cx| {
                        discard(SampleAction::Discard(discard_nonce.clone()), window, cx);
                    })
                }
            };
            elements.push(item.into_any_element());
        }
        elements
    }
}

impl std::fmt::Debug for MessageSamples {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MessageSamples")
            .field("messages", &self.messages.len())
            .field("pending", &self.pending.len())
            .finish_non_exhaustive()
    }
}
