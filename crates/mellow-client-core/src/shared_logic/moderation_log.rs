//! The moderation log as people read it (`auditEntrySentence` and `auditChangeLines` of
//! `web/src/components/server-settings.ts`, DESIGN.md §5.8): who did what to whom, with names
//! resolved from the server, its members and its bans, and what is gone said as such. The client's
//! catalogue puts each [`AuditSentence`] and [`AuditChangeLine`] into words.

use mellow_protocol::{
    AuditAction, AuditChange, AuditChangeKey, AuditLogEntry, Guild, Permissions, PublicUser,
    Snowflake,
};

use super::custom_emoji::{custom_emoji_of_reaction_key, display_emoji};
use super::format::display_name;
use super::server_settings::{PERMISSION_GROUPS, PermissionKey};

/// The log's page size (`AUDIT_LOG_PAGE_SIZE`): a shorter page is the log's start.
pub const AUDIT_LOG_PAGE_SIZE: usize = 50;

/// The catalogue's words for what no longer exists.
#[derive(Debug, Clone, Copy)]
pub struct AuditPlaceholders<'words> {
    pub deleted_account: &'words str,
    pub former_member: &'words str,
    pub deleted_role: &'words str,
    pub deleted_channel: &'words str,
    pub deleted_emoji: &'words str,
}

/// What the log can name.
#[derive(Debug, Clone, Copy)]
pub struct AuditContext<'context> {
    pub guild: &'context Guild,
    /// The members, and the banned users when the reader may list them.
    pub people: &'context [PublicUser],
    pub placeholders: AuditPlaceholders<'context>,
}

impl AuditContext<'_> {
    fn person(&self, user_id: Option<Snowflake>) -> Option<&PublicUser> {
        user_id.and_then(|user_id| self.people.iter().find(|person| person.id == user_id))
    }

    fn person_name(&self, user_id: Option<Snowflake>) -> String {
        match (user_id, self.person(user_id)) {
            (None, _) => self.placeholders.deleted_account.to_owned(),
            (Some(_), Some(person)) => display_name(person).to_owned(),
            (Some(_), None) => self.placeholders.former_member.to_owned(),
        }
    }

    fn role_name(&self, role_id: Option<Snowflake>) -> String {
        self.guild
            .roles
            .iter()
            .find(|role| Some(role.id) == role_id)
            .map_or_else(
                || self.placeholders.deleted_role.to_owned(),
                |role| role.name.clone(),
            )
    }

    fn channel_name(&self, channel_id: Option<Snowflake>) -> String {
        self.guild
            .channels
            .iter()
            .find(|channel| Some(channel.id) == channel_id)
            .map_or_else(
                || self.placeholders.deleted_channel.to_owned(),
                |channel| channel.name.clone(),
            )
    }

    /// A role, `@everyone` or a member: the target of a channel overwrite.
    fn role_or_person_name(&self, target_id: Option<Snowflake>) -> String {
        if self
            .guild
            .roles
            .iter()
            .any(|role| Some(role.id) == target_id)
        {
            self.role_name(target_id)
        } else {
            self.person_name(target_id)
        }
    }

    /// A sound by its name in the change, else as the server lists it now.
    fn sound_name(&self, sound_id: Option<Snowflake>, changed_name: Option<&str>) -> String {
        changed_name
            .map(str::to_owned)
            .or_else(|| {
                self.guild
                    .sounds
                    .iter()
                    .find(|sound| Some(sound.id) == sound_id)
                    .map(|sound| sound.name.clone())
            })
            .unwrap_or_default()
    }

    /// A reaction key as people read it: the emoji itself, or a server emoji's `:name:`.
    fn emoji_key_label(&self, key: &str) -> String {
        if key.is_empty() {
            return String::new();
        }
        let Some(emoji_id) = custom_emoji_of_reaction_key(key) else {
            return display_emoji(key);
        };
        self.guild
            .emojis
            .iter()
            .find(|emoji| emoji.id == emoji_id)
            .map_or_else(
                || self.placeholders.deleted_emoji.to_owned(),
                |emoji| format!(":{}:", emoji.name),
            )
    }

    /// The person who did it: their profile when the log knows them.
    #[must_use]
    pub fn actor_of(&self, entry: &AuditLogEntry) -> Option<&PublicUser> {
        self.person(entry.actor_id)
    }
}

/// An entry as one sentence; every name is already resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditSentence {
    ChannelCreate {
        actor: String,
        channel: String,
    },
    ChannelUpdate {
        actor: String,
        channel: String,
    },
    ChannelDelete {
        actor: String,
    },
    OverwriteSet {
        actor: String,
        target: String,
        channel: String,
    },
    OverwriteDelete {
        actor: String,
        target: String,
        channel: String,
    },
    RoleCreate {
        actor: String,
        role: String,
    },
    RoleUpdate {
        actor: String,
        role: String,
    },
    RoleDelete {
        actor: String,
    },
    RolesReorder {
        actor: String,
    },
    ChannelsReorder {
        actor: String,
    },
    MemberRoleAdd {
        actor: String,
        role: String,
        member: String,
    },
    MemberRoleRemove {
        actor: String,
        role: String,
        member: String,
    },
    MemberKick {
        actor: String,
        member: String,
    },
    MemberBanAdd {
        actor: String,
        member: String,
    },
    MemberBanRemove {
        actor: String,
        member: String,
    },
    MessageDelete {
        actor: String,
        author: String,
        channel: String,
    },
    MessagePin {
        actor: String,
        channel: String,
    },
    MessageUnpin {
        actor: String,
        channel: String,
    },
    MessageReactionRemove {
        actor: String,
        channel: String,
    },
    GuildRename {
        actor: String,
        before: String,
        after: String,
    },
    GuildUpdate {
        actor: String,
    },
    InviteCreate {
        actor: String,
        code: String,
    },
    InviteDelete {
        actor: String,
        code: String,
    },
    EmojiCreate {
        actor: String,
        name: String,
    },
    EmojiRename {
        actor: String,
        before: String,
        after: String,
    },
    EmojiDelete {
        actor: String,
        name: String,
    },
    SoundCreate {
        actor: String,
        name: String,
    },
    SoundUpdate {
        actor: String,
        name: String,
    },
    SoundDelete {
        actor: String,
        name: String,
    },
    /// An action of a newer server that this client cannot name yet.
    Unknown {
        actor: String,
    },
}

fn change_of(entry: &AuditLogEntry, key: AuditChangeKey) -> Option<&AuditChange> {
    entry.changes.iter().find(|change| change.key == key)
}

fn new_value_of(entry: &AuditLogEntry, key: AuditChangeKey) -> String {
    change_of(entry, key)
        .map(|change| change.new_value.clone())
        .unwrap_or_default()
}

fn old_value_of(entry: &AuditLogEntry, key: AuditChangeKey) -> String {
    change_of(entry, key)
        .map(|change| change.old_value.clone())
        .unwrap_or_default()
}

/// `auditEntrySentence`: "Ana kicked Paul".
#[must_use]
#[allow(clippy::too_many_lines)] // One arm per action, as the web's switch.
pub fn audit_entry_sentence(entry: &AuditLogEntry, context: &AuditContext<'_>) -> AuditSentence {
    use AuditSentence as Sentence;
    let actor = context.person_name(entry.actor_id);
    let target = entry.target_id;
    let channel_of_context = || context.channel_name(entry.context_id);
    match entry.action {
        AuditAction::ChannelCreate => Sentence::ChannelCreate {
            actor,
            channel: context.channel_name(target),
        },
        AuditAction::ChannelUpdate => Sentence::ChannelUpdate {
            actor,
            channel: context.channel_name(target),
        },
        AuditAction::ChannelDelete => Sentence::ChannelDelete { actor },
        AuditAction::OverwriteSet => Sentence::OverwriteSet {
            actor,
            target: context.role_or_person_name(target),
            channel: channel_of_context(),
        },
        AuditAction::OverwriteDelete => Sentence::OverwriteDelete {
            actor,
            target: context.role_or_person_name(target),
            channel: channel_of_context(),
        },
        AuditAction::RoleCreate => Sentence::RoleCreate {
            actor,
            role: context.role_name(target),
        },
        AuditAction::RoleUpdate => Sentence::RoleUpdate {
            actor,
            role: context.role_name(target),
        },
        AuditAction::RoleDelete => Sentence::RoleDelete { actor },
        AuditAction::RolesReorder => Sentence::RolesReorder { actor },
        AuditAction::ChannelsReorder => Sentence::ChannelsReorder { actor },
        AuditAction::MemberRoleAdd => Sentence::MemberRoleAdd {
            actor,
            role: context.role_name(entry.context_id),
            member: context.person_name(target),
        },
        AuditAction::MemberRoleRemove => Sentence::MemberRoleRemove {
            actor,
            role: context.role_name(entry.context_id),
            member: context.person_name(target),
        },
        AuditAction::MemberKick => Sentence::MemberKick {
            actor,
            member: context.person_name(target),
        },
        AuditAction::MemberBanAdd => Sentence::MemberBanAdd {
            actor,
            member: context.person_name(target),
        },
        AuditAction::MemberBanRemove => Sentence::MemberBanRemove {
            actor,
            member: context.person_name(target),
        },
        AuditAction::MessageDelete => Sentence::MessageDelete {
            actor,
            author: context.person_name(target),
            channel: channel_of_context(),
        },
        AuditAction::MessagePin => Sentence::MessagePin {
            actor,
            channel: channel_of_context(),
        },
        AuditAction::MessageUnpin => Sentence::MessageUnpin {
            actor,
            channel: channel_of_context(),
        },
        AuditAction::MessageReactionRemove => Sentence::MessageReactionRemove {
            actor,
            channel: channel_of_context(),
        },
        AuditAction::GuildUpdate => match change_of(entry, AuditChangeKey::Name) {
            Some(name) => Sentence::GuildRename {
                actor,
                before: name.old_value.clone(),
                after: name.new_value.clone(),
            },
            None => Sentence::GuildUpdate { actor },
        },
        AuditAction::InviteCreate => Sentence::InviteCreate {
            actor,
            code: new_value_of(entry, AuditChangeKey::Code),
        },
        AuditAction::InviteDelete => Sentence::InviteDelete {
            actor,
            code: old_value_of(entry, AuditChangeKey::Code),
        },
        AuditAction::EmojiCreate => Sentence::EmojiCreate {
            actor,
            name: new_value_of(entry, AuditChangeKey::Name),
        },
        AuditAction::EmojiUpdate => Sentence::EmojiRename {
            actor,
            before: old_value_of(entry, AuditChangeKey::Name),
            after: new_value_of(entry, AuditChangeKey::Name),
        },
        AuditAction::EmojiDelete => Sentence::EmojiDelete {
            actor,
            name: old_value_of(entry, AuditChangeKey::Name),
        },
        AuditAction::SoundCreate => Sentence::SoundCreate {
            actor,
            name: new_value_of(entry, AuditChangeKey::Name),
        },
        // A rename names the sound by its new name; the change lines say what it was.
        AuditAction::SoundUpdate => Sentence::SoundUpdate {
            actor,
            name: context.sound_name(
                target,
                change_of(entry, AuditChangeKey::Name).map(|change| change.new_value.as_str()),
            ),
        },
        AuditAction::SoundDelete => Sentence::SoundDelete {
            actor,
            name: old_value_of(entry, AuditChangeKey::Name),
        },
        AuditAction::Unknown => Sentence::Unknown { actor },
    }
}

/// Which permissions a change is about: a role's, or an overwrite's allowed or denied ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditPermissionsKind {
    Permissions,
    Allow,
    Deny,
}

/// One line of what an update changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditChangeLine {
    Name { before: String, after: String },
    Emoji { before: String, after: String },
    Topic(String),
    TopicCleared,
    Color,
    DisplayedSeparately(bool),
    Mentionable(bool),
    PermissionsAdded(AuditPermissionsKind, Vec<PermissionKey>),
    PermissionsRemoved(AuditPermissionsKind, Vec<PermissionKey>),
}

/// The permissions the editors name, in their order.
fn permission_keys(bits: Permissions) -> Vec<PermissionKey> {
    PERMISSION_GROUPS
        .iter()
        .flat_map(|group| group.permissions)
        .filter(|permission| bits.intersects(permission.bit))
        .map(|permission| permission.key)
        .collect()
}

fn permissions_of(value: &str) -> Permissions {
    Permissions::from_bits_retain(value.parse().unwrap_or_default())
}

/// `auditChangeLines`: one line per field, empty when the entry carries no changes.
#[must_use]
pub fn audit_change_lines(
    entry: &AuditLogEntry,
    context: &AuditContext<'_>,
) -> Vec<AuditChangeLine> {
    let mut lines = Vec::new();
    for change in &entry.changes {
        match change.key {
            AuditChangeKey::Name => {
                // These sentences already name both; a new or deleted sound's sentence names it.
                let is_named_by_sentence = matches!(
                    entry.action,
                    AuditAction::GuildUpdate
                        | AuditAction::EmojiCreate
                        | AuditAction::EmojiUpdate
                        | AuditAction::EmojiDelete
                        | AuditAction::SoundCreate
                        | AuditAction::SoundDelete
                );
                if !is_named_by_sentence {
                    lines.push(AuditChangeLine::Name {
                        before: change.old_value.clone(),
                        after: change.new_value.clone(),
                    });
                }
            }
            AuditChangeKey::Emoji => {
                if entry.action == AuditAction::SoundUpdate {
                    lines.push(AuditChangeLine::Emoji {
                        before: context.emoji_key_label(&change.old_value),
                        after: context.emoji_key_label(&change.new_value),
                    });
                }
            }
            AuditChangeKey::Topic => lines.push(if change.new_value.is_empty() {
                AuditChangeLine::TopicCleared
            } else {
                AuditChangeLine::Topic(change.new_value.clone())
            }),
            AuditChangeKey::Color => lines.push(AuditChangeLine::Color),
            AuditChangeKey::DisplayedSeparately => {
                lines.push(AuditChangeLine::DisplayedSeparately(
                    change.new_value == "true",
                ));
            }
            AuditChangeKey::Mentionable => {
                lines.push(AuditChangeLine::Mentionable(change.new_value == "true"));
            }
            AuditChangeKey::Permissions | AuditChangeKey::Allow | AuditChangeKey::Deny => {
                let kind = match change.key {
                    AuditChangeKey::Allow => AuditPermissionsKind::Allow,
                    AuditChangeKey::Deny => AuditPermissionsKind::Deny,
                    _ => AuditPermissionsKind::Permissions,
                };
                let before = permissions_of(&change.old_value);
                let after = permissions_of(&change.new_value);
                let granted = permission_keys(after.difference(before));
                let removed = permission_keys(before.difference(after));
                if !granted.is_empty() {
                    lines.push(AuditChangeLine::PermissionsAdded(kind, granted));
                }
                if !removed.is_empty() {
                    lines.push(AuditChangeLine::PermissionsRemoved(kind, removed));
                }
            }
            // The invite code is in the sentence; a category change reads as the channel's
            // update without detail, and a field of a newer server is left out.
            AuditChangeKey::Code
            | AuditChangeKey::Icon
            | AuditChangeKey::Parent
            | AuditChangeKey::UserLimit
            | AuditChangeKey::Unknown => {}
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{Channel, ChannelKind, CustomEmoji, Role, SoundboardSound};

    use super::*;

    // Ported from `server-settings.test.ts` ("moderation log").

    const OWNER: Snowflake = Snowflake::from_raw(10);
    const OTHER: Snowflake = Snowflake::from_raw(11);
    const GUILD_ID: Snowflake = Snowflake::from_raw(100);
    const HELPER: Snowflake = Snowflake::from_raw(201);
    const GENERAL: Snowflake = Snowflake::from_raw(300);

    const PLACEHOLDERS: AuditPlaceholders<'static> = AuditPlaceholders {
        deleted_account: "A deleted account",
        former_member: "a former member",
        deleted_role: "a deleted role",
        deleted_channel: "a deleted channel",
        deleted_emoji: "a deleted emoji",
    };

    fn user(id: Snowflake, name: &str) -> PublicUser {
        PublicUser {
            id,
            username: name.to_lowercase(),
            display_name: name.to_owned(),
            avatar: None,
            banner: None,
        }
    }

    fn role(id: Snowflake, name: &str) -> Role {
        Role {
            id,
            name: name.to_owned(),
            color: 0,
            position: 0,
            permissions: Permissions::empty(),
            is_displayed_separately: false,
            is_mentionable: false,
            icon: None,
            unicode_emoji: None,
        }
    }

    fn sample_guild() -> Guild {
        Guild {
            id: GUILD_ID,
            name: "Rust".to_owned(),
            owner_id: OWNER,
            icon: None,
            my_permissions: Permissions::ADMINISTRATOR,
            my_role_ids: Vec::new(),
            roles: vec![role(GUILD_ID, "role 100"), role(HELPER, "role 201")],
            channels: vec![Channel {
                id: GENERAL,
                guild_id: GUILD_ID,
                kind: ChannelKind::Text,
                name: "general".to_owned(),
                topic: String::new(),
                parent_id: None,
                position: 0,
                icon: None,
                permission_overwrites: Vec::new(),
                user_limit: None,
            }],
            emojis: vec![CustomEmoji {
                id: Snowflake::from_raw(77),
                name: "parrot".to_owned(),
                is_animated: false,
            }],
            sounds: vec![SoundboardSound {
                id: Snowflake::from_raw(900),
                name: "horn".to_owned(),
                emoji: None,
                duration_milliseconds: 800,
            }],
            my_join_sound_id: None,
        }
    }

    fn entry(action: AuditAction, target: Option<u64>, context: Option<u64>) -> AuditLogEntry {
        AuditLogEntry {
            id: Snowflake::from_raw(1),
            actor_id: Some(OWNER),
            action,
            target_id: target.map(Snowflake::from_raw),
            context_id: context.map(Snowflake::from_raw),
            reason: String::new(),
            changes: Vec::new(),
        }
    }

    fn change(key: AuditChangeKey, old_value: &str, new_value: &str) -> AuditChange {
        AuditChange {
            key,
            old_value: old_value.to_owned(),
            new_value: new_value.to_owned(),
        }
    }

    fn with_context<Answer>(run: impl FnOnce(&AuditContext<'_>) -> Answer) -> Answer {
        let guild = sample_guild();
        let people = [user(OWNER, "Ana"), user(OTHER, "Paul")];
        run(&AuditContext {
            guild: &guild,
            people: &people,
            placeholders: PLACEHOLDERS,
        })
    }

    #[test]
    fn names_people_roles_and_channels_and_what_is_gone() {
        with_context(|context| {
            let kick = entry(AuditAction::MemberKick, Some(11), None);
            assert_eq!(
                audit_entry_sentence(&kick, context),
                AuditSentence::MemberKick {
                    actor: "Ana".into(),
                    member: "Paul".into()
                }
            );
            let gone = entry(AuditAction::MemberKick, Some(999), None);
            assert_eq!(
                audit_entry_sentence(&gone, context),
                AuditSentence::MemberKick {
                    actor: "Ana".into(),
                    member: "a former member".into()
                }
            );
            let by_deleted = AuditLogEntry {
                actor_id: None,
                ..kick
            };
            assert_eq!(
                audit_entry_sentence(&by_deleted, context),
                AuditSentence::MemberKick {
                    actor: "A deleted account".into(),
                    member: "Paul".into()
                }
            );
            assert_eq!(
                audit_entry_sentence(
                    &entry(AuditAction::MemberRoleAdd, Some(11), Some(201)),
                    context
                ),
                AuditSentence::MemberRoleAdd {
                    actor: "Ana".into(),
                    role: "role 201".into(),
                    member: "Paul".into()
                }
            );
            assert_eq!(
                audit_entry_sentence(
                    &entry(AuditAction::OverwriteSet, Some(100), Some(300)),
                    context
                ),
                AuditSentence::OverwriteSet {
                    actor: "Ana".into(),
                    target: "role 100".into(),
                    channel: "general".into()
                }
            );
            assert_eq!(
                audit_entry_sentence(
                    &entry(AuditAction::MessagePin, Some(500), Some(404)),
                    context
                ),
                AuditSentence::MessagePin {
                    actor: "Ana".into(),
                    channel: "a deleted channel".into()
                }
            );
            assert_eq!(
                audit_entry_sentence(&entry(AuditAction::RoleUpdate, Some(404), None), context),
                AuditSentence::RoleUpdate {
                    actor: "Ana".into(),
                    role: "a deleted role".into()
                }
            );
            assert_eq!(
                audit_entry_sentence(&entry(AuditAction::Unknown, None, None), context),
                AuditSentence::Unknown {
                    actor: "Ana".into()
                }
            );
        });
    }

    #[test]
    fn tells_renames_invites_and_emoji_from_their_changes() {
        with_context(|context| {
            let rename = AuditLogEntry {
                changes: vec![change(AuditChangeKey::Name, "Club", "Book club")],
                ..entry(AuditAction::GuildUpdate, None, None)
            };
            assert_eq!(
                audit_entry_sentence(&rename, context),
                AuditSentence::GuildRename {
                    actor: "Ana".into(),
                    before: "Club".into(),
                    after: "Book club".into()
                }
            );
            assert_eq!(audit_change_lines(&rename, context), []);
            let revoked = AuditLogEntry {
                changes: vec![change(AuditChangeKey::Code, "abc", "")],
                ..entry(AuditAction::InviteDelete, None, None)
            };
            assert_eq!(
                audit_entry_sentence(&revoked, context),
                AuditSentence::InviteDelete {
                    actor: "Ana".into(),
                    code: "abc".into()
                }
            );
            let emoji_rename = AuditLogEntry {
                changes: vec![change(AuditChangeKey::Name, "party", "fiesta")],
                ..entry(AuditAction::EmojiUpdate, None, None)
            };
            assert_eq!(
                audit_entry_sentence(&emoji_rename, context),
                AuditSentence::EmojiRename {
                    actor: "Ana".into(),
                    before: "party".into(),
                    after: "fiesta".into()
                }
            );
            assert_eq!(audit_change_lines(&emoji_rename, context), []);
        });
    }

    #[test]
    fn tells_sound_changes_with_their_emoji_as_people_read_them() {
        with_context(|context| {
            let edited = AuditLogEntry {
                changes: vec![
                    change(AuditChangeKey::Name, "hron", "horn"),
                    change(AuditChangeKey::Emoji, "", ":77"),
                ],
                ..entry(AuditAction::SoundUpdate, Some(900), None)
            };
            assert_eq!(
                audit_entry_sentence(&edited, context),
                AuditSentence::SoundUpdate {
                    actor: "Ana".into(),
                    name: "horn".into()
                }
            );
            assert_eq!(
                audit_change_lines(&edited, context),
                [
                    AuditChangeLine::Name {
                        before: "hron".into(),
                        after: "horn".into()
                    },
                    AuditChangeLine::Emoji {
                        before: String::new(),
                        after: ":parrot:".into()
                    }
                ]
            );
            let gone_emoji = AuditLogEntry {
                changes: vec![change(AuditChangeKey::Emoji, ":78", "")],
                ..entry(AuditAction::SoundUpdate, Some(900), None)
            };
            assert_eq!(
                audit_change_lines(&gone_emoji, context),
                [AuditChangeLine::Emoji {
                    before: "a deleted emoji".into(),
                    after: String::new()
                }]
            );
        });
    }

    #[test]
    fn says_role_flags_topics_and_permissions_granted_or_removed() {
        with_context(|context| {
            let role_update = AuditLogEntry {
                changes: vec![
                    change(AuditChangeKey::DisplayedSeparately, "false", "true"),
                    change(AuditChangeKey::Mentionable, "true", "false"),
                    change(AuditChangeKey::Color, "0", "5"),
                    change(
                        AuditChangeKey::Permissions,
                        &Permissions::SEND_MESSAGES.bits().to_string(),
                        &(Permissions::KICK_MEMBERS | Permissions::VIEW_CHANNEL)
                            .bits()
                            .to_string(),
                    ),
                ],
                ..entry(AuditAction::RoleUpdate, Some(201), None)
            };
            assert_eq!(
                audit_change_lines(&role_update, context),
                [
                    AuditChangeLine::DisplayedSeparately(true),
                    AuditChangeLine::Mentionable(false),
                    AuditChangeLine::Color,
                    AuditChangeLine::PermissionsAdded(
                        AuditPermissionsKind::Permissions,
                        vec![PermissionKey::ViewChannels, PermissionKey::KickMembers]
                    ),
                    AuditChangeLine::PermissionsRemoved(
                        AuditPermissionsKind::Permissions,
                        vec![PermissionKey::SendMessages]
                    ),
                ]
            );
            let channel_update = AuditLogEntry {
                changes: vec![
                    change(AuditChangeKey::Topic, "", "Rust talk"),
                    change(AuditChangeKey::Topic, "Rust talk", ""),
                    change(AuditChangeKey::Parent, "", "5"),
                ],
                ..entry(AuditAction::ChannelUpdate, Some(300), None)
            };
            assert_eq!(
                audit_change_lines(&channel_update, context),
                [
                    AuditChangeLine::Topic("Rust talk".into()),
                    AuditChangeLine::TopicCleared
                ]
            );
        });
    }
}
