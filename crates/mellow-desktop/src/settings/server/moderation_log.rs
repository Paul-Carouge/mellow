//! Moderation log (`ModerationLogSettings.svelte`, DESIGN.md §5.8): who did what, newest first,
//! one sentence each in the reader's language, with what changed and the ban reason under it.
//! Older pages load on request. Names come from the member list and the bans; anyone else reads
//! as "a former member".

use std::collections::HashMap;

use gpui::{
    Context, ElementId, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Role, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::moderation_log::{
    AUDIT_LOG_PAGE_SIZE, AuditChangeLine, AuditContext, AuditPermissionsKind, AuditPlaceholders,
    AuditSentence, audit_change_lines, audit_entry_sentence,
};
use mellow_client_core::shared_logic::server_settings::{PermissionKey, holds};
use mellow_protocol::{AuditLogEntry, AuditLogQuery, Permissions, PublicUser, Snowflake};

use super::list_rows::{
    empty_sentence, list_row, list_status, person_avatar, secondary_line, text_column,
};
use crate::components::{Button, is_picture_row_pointed_at, track_picture_row_pointer, with_type};
use crate::i18n::catalogue::PermissionsOrAllowOrDeny;
use crate::i18n::{Strings, strings};
use crate::messages::{full_timestamp_label, local_date_time};
use crate::session::Session;
use crate::settings::permission_words::permission_words;
use crate::settings::{SettingsPage, SettingsPageEvent, error_alert};
use crate::theme::tokens::{
    COMPONENT_SERVER_SETTINGS_AVATAR_SIZE, COMPONENT_SERVER_SETTINGS_TOOLBAR_GAP,
    COMPONENT_SETTINGS_SECTION_LABEL_GAP, TYPE_BODY, TYPE_BODY_LG,
};
use crate::theme::{color, length, theme};

#[allow(clippy::struct_excessive_bools)] // The web's independent flags, one each.
pub struct ServerModerationLogPage {
    session: Entity<Session>,
    guild_id: Snowflake,
    /// `None` while loading or after a failure.
    entries: Option<Vec<AuditLogEntry>>,
    has_failed: bool,
    has_reached_start: bool,
    is_loading_older: bool,
    has_older_failed: bool,
    /// Banned people left the member list; moderators who may list bans see them named too.
    banned_users: Vec<PublicUser>,
    load_generation: u64,
}

impl EventEmitter<SettingsPageEvent> for ServerModerationLogPage {}

impl SettingsPage for ServerModerationLogPage {
    fn set_attention(&mut self, _attention: u32, _cx: &mut Context<Self>) {}
}

impl std::fmt::Debug for ServerModerationLogPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerModerationLogPage")
            .field("guild_id", &self.guild_id)
            .field("has_failed", &self.has_failed)
            .finish_non_exhaustive()
    }
}

impl ServerModerationLogPage {
    pub fn new(session: Entity<Session>, guild_id: Snowflake, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            session,
            guild_id,
            entries: None,
            has_failed: false,
            has_reached_start: false,
            is_loading_older: false,
            has_older_failed: false,
            banned_users: Vec::new(),
            load_generation: 0,
        };
        page.load(cx);
        page.load_banned_users(cx);
        page
    }

    // Only the tests against a real server read it; the server refuses Windows (ADR-0006).
    #[cfg(all(test, not(windows)))]
    pub(crate) fn entries(&self) -> Option<&[AuditLogEntry]> {
        self.entries.as_deref()
    }

    fn load_banned_users(&mut self, cx: &mut Context<Self>) {
        let guild_id = self.guild_id;
        let may_list_bans = self
            .session
            .read(cx)
            .state()
            .guild(guild_id)
            .is_some_and(|guild| holds(guild, Permissions::BAN_MEMBERS));
        if !may_list_bans {
            return;
        }
        let page = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_settings_request(
                "list bans for the moderation log",
                move |rest| async move { rest.list_bans(guild_id).await },
                cx,
                move |answer, cx| {
                    // A failure only leaves names out: they read as "a former member".
                    if let Ok(bans) = answer {
                        let _ = page.update(cx, |page, cx| {
                            page.banned_users = bans.into_iter().map(|ban| ban.user).collect();
                            cx.notify();
                        });
                    }
                },
            );
        });
    }

    /// The newest page, or the one before `before`; a page shorter than asked is the log's start.
    fn load_page(&mut self, before: Option<Snowflake>, cx: &mut Context<Self>) {
        self.load_generation += 1;
        let generation = self.load_generation;
        let guild_id = self.guild_id;
        let page = cx.entity().downgrade();
        let query = AuditLogQuery {
            before,
            limit: None,
        };
        self.session.update(cx, |session, cx| {
            session.send_settings_request(
                "list the moderation log",
                move |rest| async move { rest.list_audit_log(guild_id, &query).await },
                cx,
                move |answer, cx| {
                    let _ = page.update(cx, |page, cx| {
                        if page.load_generation != generation {
                            return;
                        }
                        page.is_loading_older = false;
                        match answer {
                            Ok(entries) => {
                                page.has_reached_start = entries.len() < AUDIT_LOG_PAGE_SIZE;
                                match (before, page.entries.as_mut()) {
                                    (Some(_), Some(shown)) => shown.extend(entries),
                                    _ => page.entries = Some(entries),
                                }
                            }
                            Err(_) if before.is_some() => page.has_older_failed = true,
                            Err(_) => {
                                page.entries = None;
                                page.has_failed = true;
                            }
                        }
                        cx.notify();
                    });
                },
            );
        });
        cx.notify();
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.has_failed = false;
        self.load_page(None, cx);
    }

    fn load_older(&mut self, cx: &mut Context<Self>) {
        let Some(oldest) = self
            .entries
            .as_ref()
            .and_then(|entries| entries.last())
            .map(|entry| entry.id)
        else {
            return;
        };
        if self.is_loading_older {
            return;
        }
        self.is_loading_older = true;
        self.has_older_failed = false;
        self.load_page(Some(oldest), cx);
    }
}

/// `auditEntrySentence` in the reader's words.
pub(crate) fn sentence_words(sentence: &AuditSentence, catalogue: Strings) -> String {
    let log = catalogue.server_settings().moderation_log();
    match sentence {
        AuditSentence::ChannelCreate { actor, channel } => log.channel_create(actor, channel),
        AuditSentence::ChannelUpdate { actor, channel } => log.channel_update(actor, channel),
        AuditSentence::ChannelDelete { actor } => log.channel_delete(actor),
        AuditSentence::OverwriteSet {
            actor,
            target,
            channel,
        } => log.overwrite_set(actor, target, channel),
        AuditSentence::OverwriteDelete {
            actor,
            target,
            channel,
        } => log.overwrite_delete(actor, target, channel),
        AuditSentence::RoleCreate { actor, role } => log.role_create(actor, role),
        AuditSentence::RoleUpdate { actor, role } => log.role_update(actor, role),
        AuditSentence::RoleDelete { actor } => log.role_delete(actor),
        AuditSentence::RolesReorder { actor } => log.roles_reorder(actor),
        AuditSentence::ChannelsReorder { actor } => log.channels_reorder(actor),
        AuditSentence::MemberRoleAdd {
            actor,
            role,
            member,
        } => log.member_role_add(actor, role, member),
        AuditSentence::MemberRoleRemove {
            actor,
            role,
            member,
        } => log.member_role_remove(actor, role, member),
        AuditSentence::MemberKick { actor, member } => log.member_kick(actor, member),
        AuditSentence::MemberBanAdd { actor, member } => log.member_ban_add(actor, member),
        AuditSentence::MemberBanRemove { actor, member } => log.member_ban_remove(actor, member),
        AuditSentence::MessageDelete {
            actor,
            author,
            channel,
        } => log.message_delete(actor, author, channel),
        AuditSentence::MessagePin { actor, channel } => log.message_pin(actor, channel),
        AuditSentence::MessageUnpin { actor, channel } => log.message_unpin(actor, channel),
        AuditSentence::MessageReactionRemove { actor, channel } => {
            log.message_reaction_remove(actor, channel)
        }
        AuditSentence::GuildRename {
            actor,
            before,
            after,
        } => log.guild_rename(actor, before, after),
        AuditSentence::GuildUpdate { actor } => log.guild_update(actor),
        AuditSentence::InviteCreate { actor, code } => log.invite_create(actor, code),
        AuditSentence::InviteDelete { actor, code } => log.invite_delete(actor, code),
        AuditSentence::EmojiCreate { actor, name } => log.emoji_create(actor, name),
        AuditSentence::EmojiRename {
            actor,
            before,
            after,
        } => log.emoji_rename(actor, before, after),
        AuditSentence::EmojiDelete { actor, name } => log.emoji_delete(actor, name),
        AuditSentence::SoundCreate { actor, name } => log.sound_create(actor, name),
        AuditSentence::SoundUpdate { actor, name } => log.sound_update(actor, name),
        AuditSentence::SoundDelete { actor, name } => log.sound_delete(actor, name),
        AuditSentence::Unknown { actor } => log.unknown(actor),
    }
}

/// `auditChangeLines`, one line in the reader's words.
fn change_line_words(line: &AuditChangeLine, catalogue: Strings) -> String {
    let log = catalogue.server_settings().moderation_log();
    let names = |keys: &[PermissionKey]| {
        keys.iter()
            .map(|key| permission_words(*key, catalogue).0)
            .collect::<Vec<_>>()
            .join(catalogue.typing().list_separator())
    };
    let kind_words = |kind: AuditPermissionsKind| match kind {
        AuditPermissionsKind::Permissions => PermissionsOrAllowOrDeny::Permissions,
        AuditPermissionsKind::Allow => PermissionsOrAllowOrDeny::Allow,
        AuditPermissionsKind::Deny => PermissionsOrAllowOrDeny::Deny,
    };
    match line {
        AuditChangeLine::Name { before, after } => log.name_change(before, after),
        AuditChangeLine::Emoji { before, after } => log.emoji_change(before, after),
        AuditChangeLine::Topic(topic) => log.topic_change(topic),
        AuditChangeLine::TopicCleared => log.topic_cleared().to_owned(),
        AuditChangeLine::Color => log.color_change().to_owned(),
        AuditChangeLine::DisplayedSeparately(is_on) => log.displayed_separately_change(*is_on),
        AuditChangeLine::Mentionable(is_on) => log.mentionable_change(*is_on),
        AuditChangeLine::PermissionsAdded(kind, keys) => {
            log.permissions_added(kind_words(*kind), &names(keys))
        }
        AuditChangeLine::PermissionsRemoved(kind, keys) => {
            log.permissions_removed(kind_words(*kind), &names(keys))
        }
    }
}

impl Render for ServerModerationLogPage {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalogue = strings(cx);
        let words = catalogue.server_settings().moderation_log();
        let Some(entries) = self.entries.as_ref() else {
            let page = cx.entity().downgrade();
            return list_status(
                "moderation-log-status",
                self.has_failed,
                words.load_failed(),
                move |_, cx| {
                    let _ = page.update(cx, ServerModerationLogPage::load);
                },
                cx,
            )
            .into_any_element();
        };
        if entries.is_empty() {
            return empty_sentence(words.empty(), cx).into_any_element();
        }
        let state = self.session.read(cx).state();
        let Some(guild) = state.guild(self.guild_id) else {
            return div().into_any_element();
        };
        // Borrowed, not cloned: the page redraws on every session change.
        let people: HashMap<Snowflake, &PublicUser> = state
            .members(self.guild_id)
            .iter()
            .map(|member| &member.user)
            .chain(self.banned_users.iter())
            .map(|person| (person.id, person))
            .collect();
        let context = AuditContext {
            guild,
            people: &people,
            placeholders: AuditPlaceholders {
                deleted_account: words.deleted_account(),
                former_member: words.former_member(),
                deleted_role: words.deleted_role(),
                deleted_channel: words.deleted_channel(),
                deleted_emoji: catalogue.reactions().deleted_emoji(),
            },
        };
        let tokens = theme(cx);
        let body_color = color(tokens.component_settings_body_text);
        let mut rows = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            let row_id = ElementId::from(("audit-entry", index));
            let is_row_pointed_at = is_picture_row_pointed_at(&row_id, cx);
            let avatar = match context.actor_of(entry) {
                Some(actor) => {
                    person_avatar(actor, is_row_pointed_at, window, cx).into_any_element()
                }
                None => div()
                    .flex_none()
                    .size(length(COMPONENT_SERVER_SETTINGS_AVATAR_SIZE))
                    .rounded_full()
                    .bg(color(tokens.component_server_settings_skeleton_fill))
                    .into_any_element(),
            };
            let details: Vec<_> = audit_change_lines(entry, &context)
                .iter()
                .map(|line| change_line_words(line, catalogue))
                .chain((!entry.reason.is_empty()).then(|| words.reason(&entry.reason)))
                .map(|line| {
                    with_type(div(), TYPE_BODY)
                        .text_color(body_color)
                        .child(line)
                })
                .collect();
            rows.push(
                track_picture_row_pointer(list_row(cx).id(row_id.clone()), row_id, window)
                    .items_start()
                    .role(Role::ListItem)
                    .aria_position_in_set(index + 1)
                    .aria_size_of_set(entries.len())
                    .child(avatar)
                    .child(
                        text_column()
                            .child(
                                with_type(div(), TYPE_BODY_LG)
                                    .text_color(color(tokens.color_text_heading))
                                    .child(sentence_words(
                                        &audit_entry_sentence(entry, &context),
                                        catalogue,
                                    )),
                            )
                            .children(details)
                            .child(secondary_line(
                                full_timestamp_label(
                                    local_date_time(entry.id.unix_milliseconds()),
                                    catalogue,
                                ),
                                cx,
                            )),
                    ),
            );
        }
        let page = cx.entity().downgrade();
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("moderation-log-list")
                    .role(Role::List)
                    .aria_label(catalogue.server_settings().sections().moderation_log())
                    .children(rows),
            )
            .when(!self.has_reached_start, |column| {
                column.child(
                    div()
                        .mt(length(COMPONENT_SERVER_SETTINGS_TOOLBAR_GAP))
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                        .child(
                            Button::new(
                                "moderation-log-older",
                                if self.is_loading_older {
                                    words.loading_older()
                                } else {
                                    words.load_older()
                                },
                            )
                            .disabled(self.is_loading_older)
                            .on_click(move |_, cx| {
                                let _ = page.update(cx, ServerModerationLogPage::load_older);
                            }),
                        )
                        .child(error_alert(
                            "moderation-log-older-error",
                            self.has_older_failed
                                .then(|| words.load_older_failed().into()),
                            cx,
                        )),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use mellow_client_core::shared_logic::moderation_log::{AuditChangeLine, AuditPermissionsKind};
    use mellow_client_core::shared_logic::server_settings::PermissionKey;

    use super::*;

    #[test]
    fn says_entries_and_permission_changes_in_the_readers_words() {
        let catalogue = Strings::default();
        assert_eq!(
            sentence_words(
                &AuditSentence::MemberKick {
                    actor: "Ana".into(),
                    member: "Paul".into()
                },
                catalogue
            ),
            "Ana kicked Paul"
        );
        assert_eq!(
            change_line_words(
                &AuditChangeLine::PermissionsAdded(
                    AuditPermissionsKind::Deny,
                    vec![PermissionKey::SendMessages, PermissionKey::AddReactions]
                ),
                catalogue
            ),
            "Now denied: Send messages, Add reactions"
        );
    }
}
