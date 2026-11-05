//! Guild, channel, role, invite and membership use cases. Every action checks permissions through
//! `domain::permissions` (ADR-0007).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use crate::storage::Database;
use mellow_protocol::{
    AuditAction, AuditChange, AuditChangeKey, AuditLogEntry, AuditLogQuery, Channel, ChannelKind,
    CreateGuildRequest, CustomEmoji, GatewayEvent, Guild, GuildSummary, InitialCategory,
    InitialTextChannel, Member, Permissions, PublicUser, Role, Snowflake, SoundboardSound,
    UpdateGuildRequest,
};

use crate::domain::account_rules::GuildCreationPolicy;
use crate::domain::clock::now_unix_milliseconds;
use crate::domain::guild_rules::{
    GuildRuleViolation, MAXIMUM_OWNED_GUILDS_PER_USER, audit_log_page_size, normalize_guild_name,
    normalize_initial_categories,
};
use crate::domain::permissions::{channel_permissions, guild_permissions};
use crate::domain::snowflake_generator::SnowflakeGenerator;
use crate::events::EventHub;
use crate::storage::audit_log::{self as audit_log_storage, ChannelEntryFilter, NewAuditEntry};
use crate::storage::guild_channels::{self as channel_storage, StoredChannel};
use crate::storage::guild_emojis as guild_emojis_storage;
use crate::storage::guild_members as member_storage;
use crate::storage::guild_roles as role_storage;
use crate::storage::guild_sounds as guild_sounds_storage;
use crate::storage::guilds::{
    self as guild_storage, GuildRename, GuildStorageError, Membership, NewGuild,
    StoredPermissionSnapshot,
};

mod audience;
mod channels;
mod emojis;
mod icons;
mod invites;
mod members;
mod overwrites;
mod role_icons;
mod roles;
mod sounds;
#[cfg(test)]
mod tests;

pub(crate) use audience::ChannelAudience;
pub(crate) use icons::IconUpload;
pub(crate) use role_icons::RoleIconRequest;

const DEFAULT_CATEGORY_NAME: &str = "Text channels";
const DEFAULT_CHANNEL_NAME: &str = "general";

#[derive(Debug, thiserror::Error)]
pub enum GuildError {
    #[error(transparent)]
    InvalidInput(#[from] GuildRuleViolation),
    /// Also returned to non-members, so they cannot tell a hidden guild from a missing one.
    #[error("not found")]
    NotFound,
    #[error("you do not have permission to do this")]
    MissingPermissions,
    #[error("this invite is unknown, expired or used up")]
    InvalidInvite,
    #[error("the owner cannot leave their own guild")]
    OwnerCannotLeave,
    #[error("you are banned from this guild")]
    Banned,
    #[error("limit reached: {0}")]
    LimitReached(&'static str),
    /// What the request was checked against changed before it could be written.
    #[error("changed meanwhile, refresh and try again")]
    Conflict,
    #[error("internal error")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl From<GuildStorageError> for GuildError {
    fn from(error: GuildStorageError) -> Self {
        match error {
            GuildStorageError::ReferenceMissing => GuildRuleViolation::Parent.into(),
            other => Self::Internal(Box::new(other)),
        }
    }
}

/// Maps the storage quota error to a message naming the limit that was reached.
fn limit_reached(limit: &'static str) -> impl FnOnce(GuildStorageError) -> GuildError {
    move |error| match error {
        GuildStorageError::LimitReached => GuildError::LimitReached(limit),
        other => other.into(),
    }
}

impl From<mellow_protocol::SnowflakeError> for GuildError {
    fn from(error: mellow_protocol::SnowflakeError) -> Self {
        Self::Internal(Box::new(error))
    }
}

impl From<getrandom::Error> for GuildError {
    fn from(error: getrandom::Error) -> Self {
        Self::Internal(Box::new(error))
    }
}

fn member_from_stored(member: member_storage::StoredMember) -> Member {
    Member {
        user: PublicUser {
            id: member.user_id,
            username: member.username,
            display_name: member.display_name,
            avatar: None,
            banner: None,
        },
        role_ids: member.role_ids,
        joined_at_unix_milliseconds: member.joined_at_unix_milliseconds,
    }
}

/// What every member of a guild sees the same: its custom emoji and soundboard sounds.
#[derive(Debug, Clone)]
struct GuildExpressions {
    emojis: Vec<CustomEmoji>,
    sounds: Vec<SoundboardSound>,
}

impl Guilds {
    async fn expressions_of_guild(
        &self,
        guild_id: Snowflake,
    ) -> Result<GuildExpressions, GuildError> {
        Ok(GuildExpressions {
            emojis: guild_emojis_storage::list_guild_emojis(&self.database, guild_id).await?,
            sounds: guild_sounds_storage::list_guild_sounds(&self.database, guild_id).await?,
        })
    }
}

/// What `member_id` receives as their guild: channels they cannot view are left out entirely
/// (ADR-0015). `member_roles` includes `@everyone`; `roles` is every role of the guild.
fn guild_seen_by_member(
    member_id: Snowflake,
    summary: GuildSummary,
    owner_id: Snowflake,
    member_roles: &[Role],
    roles: Vec<Role>,
    channels: &[StoredChannel],
    expressions: GuildExpressions,
) -> Guild {
    let guild_id = summary.id;
    Guild {
        id: guild_id,
        my_role_ids: member_roles
            .iter()
            .map(|role| role.id)
            .filter(|role_id| *role_id != guild_id)
            .collect(),
        name: summary.name,
        icon: summary.icon,
        owner_id,
        my_permissions: guild_permissions(member_id, owner_id, member_roles),
        roles,
        channels: channels
            .iter()
            .filter(|channel| {
                channel_permissions(
                    member_id,
                    owner_id,
                    guild_id,
                    member_roles,
                    &channel.permission_overwrites,
                )
                .contains(Permissions::VIEW_CHANNEL)
            })
            .cloned()
            .map(channel_from_stored)
            .collect(),
        emojis: expressions.emojis,
        sounds: expressions.sounds,
        // Each member's own: the callers fill it in.
        my_join_sound_id: None,
    }
}

fn channel_from_stored(channel: StoredChannel) -> Channel {
    Channel {
        id: channel.id,
        guild_id: channel.guild_id,
        kind: channel.kind,
        name: channel.name,
        topic: channel.topic,
        parent_id: channel.parent_id,
        position: channel.position,
        icon: channel.icon,
        permission_overwrites: channel.permission_overwrites,
        user_limit: channel.user_limit,
    }
}

/// `Some` when `new` is given and differs from `old`.
fn text_change(key: AuditChangeKey, old: &str, new: Option<&str>) -> Option<AuditChange> {
    new.filter(|new| *new != old).map(|new| AuditChange {
        key,
        old_value: old.to_owned(),
        new_value: new.to_owned(),
    })
}

fn permissions_change(
    key: AuditChangeKey,
    old: Permissions,
    new: Permissions,
) -> Option<AuditChange> {
    (old != new).then(|| AuditChange {
        key,
        old_value: old.bits().to_string(),
        new_value: new.bits().to_string(),
    })
}

#[derive(Debug)]
pub(crate) struct Guilds {
    database: Database,
    ids: Arc<SnowflakeGenerator>,
    events: Arc<EventHub>,
    /// Each guild's members and roles for event fan-out, trusted only while the guild's
    /// `permission_version` in the database still matches (`permission_snapshot`).
    permission_snapshots: Mutex<HashMap<Snowflake, Arc<StoredPermissionSnapshot>>>,
    creation_policy: GuildCreationPolicy,
}

impl Guilds {
    pub(crate) fn new(
        database: Database,
        ids: Arc<SnowflakeGenerator>,
        events: Arc<EventHub>,
        creation_policy: GuildCreationPolicy,
    ) -> Self {
        Self {
            database,
            ids,
            events,
            permission_snapshots: Mutex::new(HashMap::new()),
            creation_policy,
        }
    }

    /// Whether a user may create guilds here: everyone, or only instance administrators
    /// (`GuildCreationPolicy`), which guild ownership and guild roles have nothing to do with.
    pub(crate) const fn may_create_guilds(&self, is_instance_administrator: bool) -> bool {
        self.creation_policy.allows(is_instance_administrator)
    }

    /// The guild's members and roles. The cached copy is used only if the guild's
    /// `permission_version`, read now, is the one it was loaded at; triggers bump that version on
    /// every member, role, role assignment and owner change (migration 0015), so the copy is never
    /// older than the database at the time of the call.
    async fn permission_snapshot(
        &self,
        guild_id: Snowflake,
    ) -> Result<Arc<StoredPermissionSnapshot>, GuildError> {
        let Some(version) = guild_storage::permission_version(&self.database, guild_id).await?
        else {
            // Deleted, maybe by another process: drop what a racing load may have stored.
            self.cached_permission_snapshots().remove(&guild_id);
            return Err(GuildError::NotFound);
        };
        let cached = self
            .cached_permission_snapshots()
            .get(&guild_id)
            .filter(|snapshot| snapshot.version == version)
            .cloned();
        if let Some(snapshot) = cached {
            return Ok(snapshot);
        }
        let loaded = Arc::new(
            guild_storage::permission_snapshot(&self.database, guild_id)
                .await?
                .ok_or(GuildError::NotFound)?,
        );
        let mut snapshots = self.cached_permission_snapshots();
        // A concurrent load may have stored a newer version meanwhile: keep the newest.
        // shortcut: one entry per guild ever used, never evicted (one racing a deletion is dropped
        // at the next use); add an LRU bound if memory on large instances shows the need.
        if snapshots
            .get(&guild_id)
            .is_none_or(|cached| cached.version < loaded.version)
        {
            snapshots.insert(guild_id, loaded.clone());
        }
        Ok(loaded)
    }

    fn cached_permission_snapshots(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<Snowflake, Arc<StoredPermissionSnapshot>>> {
        // The map holds no invariant a panicking holder could break.
        self.permission_snapshots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// What the audit log records for an action about to be written, in the same transaction.
    pub(crate) fn audit_entry(
        &self,
        guild_id: Snowflake,
        actor_id: Snowflake,
        action: AuditAction,
        target_id: Option<Snowflake>,
        context_id: Option<Snowflake>,
    ) -> Result<NewAuditEntry, GuildError> {
        // Decided here, so no caller can forget the channel that hides an entry (ADR-0015).
        let channel_id = match action {
            AuditAction::ChannelCreate
            | AuditAction::ChannelUpdate
            | AuditAction::ChannelDelete => target_id,
            AuditAction::OverwriteSet
            | AuditAction::OverwriteDelete
            | AuditAction::MessageDelete
            | AuditAction::MessagePin
            | AuditAction::MessageUnpin
            | AuditAction::MessageReactionRemove => context_id,
            _ => None,
        };
        Ok(NewAuditEntry {
            id: self.ids.next_id(now_unix_milliseconds())?,
            guild_id,
            actor_id,
            action,
            target_id,
            context_id,
            channel_id,
            reason: String::new(),
            changes: Vec::new(),
        })
    }

    /// The member's permissions, or `NotFound` for non-members.
    async fn membership_and_permissions(
        &self,
        guild_id: Snowflake,
        user_id: Snowflake,
    ) -> Result<(Membership, Permissions), GuildError> {
        let membership = guild_storage::find_membership(&self.database, guild_id, user_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        let permissions = guild_permissions(user_id, membership.owner_id, &membership.member_roles);
        Ok((membership, permissions))
    }

    async fn require_member_permissions(
        &self,
        guild_id: Snowflake,
        user_id: Snowflake,
        needed: Permissions,
    ) -> Result<(), GuildError> {
        let (_, permissions) = self.membership_and_permissions(guild_id, user_id).await?;
        if permissions.contains(needed) {
            Ok(())
        } else {
            Err(GuildError::MissingPermissions)
        }
    }

    /// Creates a guild owned by `owner_id` with the requested categories and channels, or by
    /// default a "Text channels" category holding `general`.
    /// Refused (`MissingPermissions`) unless the instance's `GuildCreationPolicy` lets the owner.
    pub(crate) async fn create_guild(
        &self,
        owner_id: Snowflake,
        is_instance_administrator: bool,
        request: CreateGuildRequest,
        now_unix_milliseconds: u64,
    ) -> Result<Guild, GuildError> {
        if !self.may_create_guilds(is_instance_administrator) {
            return Err(GuildError::MissingPermissions);
        }
        let name = normalize_guild_name(&request.name)?;
        let initial_categories = match &request.initial_categories {
            Some(categories) => normalize_initial_categories(categories)?,
            None => vec![InitialCategory {
                name: DEFAULT_CATEGORY_NAME.to_owned(),
                channels: vec![InitialTextChannel {
                    name: DEFAULT_CHANNEL_NAME.to_owned(),
                    topic: String::new(),
                }],
            }],
        };
        let guild_id = self.ids.next_id(now_unix_milliseconds)?;
        let mut initial_channels = Vec::new();
        for category in initial_categories {
            let category_id = self.ids.next_id(now_unix_milliseconds)?;
            let channels = std::iter::once((ChannelKind::Category, category.name, String::new()))
                .chain(
                    category
                        .channels
                        .into_iter()
                        .map(|channel| (ChannelKind::Text, channel.name, channel.topic)),
                );
            for (kind, name, topic) in channels {
                let is_category = kind == ChannelKind::Category;
                initial_channels.push(StoredChannel {
                    id: if is_category {
                        category_id
                    } else {
                        self.ids.next_id(now_unix_milliseconds)?
                    },
                    guild_id,
                    kind,
                    name,
                    topic,
                    parent_id: (!is_category).then_some(category_id),
                    position: u32::try_from(initial_channels.len())
                        .map_err(|_| GuildRuleViolation::InitialChannels)?,
                    icon: None,
                    permission_overwrites: Vec::new(),
                    user_limit: None,
                });
            }
        }
        guild_storage::insert_guild(
            &self.database,
            &NewGuild {
                id: guild_id,
                name,
                owner_id,
                everyone_permissions: Permissions::DEFAULT_EVERYONE,
                created_at_unix_milliseconds: now_unix_milliseconds,
                initial_channels,
            },
            MAXIMUM_OWNED_GUILDS_PER_USER,
        )
        .await
        .map_err(limit_reached("a user owns at most 100 guilds"))?;
        // Subscribed before the snapshot, so a channel created right after is not missed.
        self.events.subscribe_user_to_guild(owner_id, guild_id);
        let guild = self.get_guild(owner_id, guild_id).await?;
        self.events.publish_to_user(
            owner_id,
            &GatewayEvent::GuildCreate(Box::new(guild.clone())),
        );
        Ok(guild)
    }

    pub(crate) async fn list_my_guilds(
        &self,
        user_id: Snowflake,
    ) -> Result<Vec<GuildSummary>, GuildError> {
        Ok(guild_storage::list_user_guilds(&self.database, user_id).await?)
    }

    pub(crate) async fn get_guild(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
    ) -> Result<Guild, GuildError> {
        let membership = guild_storage::find_membership(&self.database, guild_id, user_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        let channels = channel_storage::list_guild_channels(&self.database, guild_id).await?;
        let roles = role_storage::list_roles(&self.database, guild_id).await?;
        let expressions = self.expressions_of_guild(guild_id).await?;
        let my_join_sound_id =
            guild_sounds_storage::member_join_sound(&self.database, guild_id, user_id).await?;
        Ok(Guild {
            my_join_sound_id,
            ..guild_seen_by_member(
                user_id,
                GuildSummary {
                    id: guild_id,
                    name: membership.guild_name,
                    icon: membership.guild_icon,
                },
                membership.owner_id,
                &membership.member_roles,
                roles,
                &channels,
                expressions,
            )
        })
    }

    /// Needs `MANAGE_GUILD`. Absent fields stay unchanged; an unchanged name records nothing.
    pub(crate) async fn update_guild(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        request: UpdateGuildRequest,
    ) -> Result<Guild, GuildError> {
        self.require_member_permissions(guild_id, user_id, Permissions::MANAGE_GUILD)
            .await?;
        let name = request
            .name
            .map(|name| normalize_guild_name(&name))
            .transpose()?;
        if let Some(name) = name {
            let audit =
                self.audit_entry(guild_id, user_id, AuditAction::GuildUpdate, None, None)?;
            match guild_storage::update_guild_name(&self.database, guild_id, &name, &audit).await? {
                GuildRename::Renamed { icon } => self.events.publish_to_guild(
                    guild_id,
                    &GatewayEvent::GuildUpdate(GuildSummary {
                        id: guild_id,
                        name,
                        icon,
                    }),
                ),
                GuildRename::Unchanged => {}
                GuildRename::NotFound => return Err(GuildError::NotFound),
            }
        }
        self.get_guild(user_id, guild_id).await
    }

    /// The owner only. Everything in the guild goes with it; every member's sessions stop
    /// receiving it and see it disappear. Nothing is audited: the log goes with the guild.
    pub(crate) async fn delete_guild(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
    ) -> Result<(), GuildError> {
        let (membership, _) = self.membership_and_permissions(guild_id, user_id).await?;
        if membership.owner_id != user_id {
            return Err(GuildError::MissingPermissions);
        }
        // Detached, like a channel deletion.
        let database = self.database.clone();
        let member_ids =
            tokio::spawn(
                async move { guild_storage::delete_guild(&database, guild_id, user_id).await },
            )
            .await
            .map_err(|error| GuildError::Internal(Box::new(error)))??
            .ok_or(GuildError::NotFound)?;
        self.cached_permission_snapshots().remove(&guild_id);
        self.events.note_guild_access_changed(guild_id);
        for member_id in member_ids {
            self.events.unsubscribe_user_from_guild(member_id, guild_id);
            self.events
                .publish_to_user(member_id, &GatewayEvent::GuildDelete { id: guild_id });
        }
        tracing::info!(%guild_id, "a guild was deleted by its owner");
        Ok(())
    }

    /// Needs `VIEW_AUDIT_LOG`. Newest first, `before` pages back.
    pub(crate) async fn list_audit_log(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        query: AuditLogQuery,
    ) -> Result<Vec<AuditLogEntry>, GuildError> {
        let limit = audit_log_page_size(query.limit)?;
        self.require_member_permissions(guild_id, user_id, Permissions::VIEW_AUDIT_LOG)
            .await?;
        let guild = self.get_guild(user_id, guild_id).await?;
        // Entries about a channel the reader cannot view stay hidden, as the channel does.
        let filter = if guild.my_permissions.contains(Permissions::ADMINISTRATOR) {
            ChannelEntryFilter::All
        } else {
            ChannelEntryFilter::Only(guild.channels.iter().map(|channel| channel.id).collect())
        };
        Ok(audit_log_storage::list_audit_entries(
            &self.database,
            guild_id,
            query.before,
            limit,
            &filter,
        )
        .await?)
    }
}
