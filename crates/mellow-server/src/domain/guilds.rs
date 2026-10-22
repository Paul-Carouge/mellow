//! Guild, channel, role, invite and membership use cases. Every action checks permissions through
//! `domain::permissions` (ADR-0007).

use std::collections::HashSet;
use std::sync::Arc;

use mellow_protocol::{
    AuditAction, AuditChange, AuditChangeKey, AuditLogEntry, AuditLogQuery, Ban, Channel,
    ChannelKind, CreateBanRequest, CreateChannelRequest, CreateGuildRequest, CreateInviteRequest,
    CreateRoleRequest, GatewayEvent, Guild, GuildSummary, Invite, Member, OverwriteKind,
    PermissionOverwrite, Permissions, PresenceStatus, PublicUser, ReorderRolesRequest, Role,
    SetPermissionOverwriteRequest, Snowflake, UpdateChannelRequest, UpdateRoleRequest,
};
use sqlx::SqlitePool;

use crate::domain::clock::now_unix_milliseconds;
use crate::domain::guild_rules::{
    GuildRuleViolation, MAXIMUM_ACTIVE_INVITES_PER_GUILD, MAXIMUM_BANS_LISTED,
    MAXIMUM_CHANNELS_PER_GUILD, MAXIMUM_MEMBERS_LISTED, MAXIMUM_OVERWRITES_PER_CHANNEL,
    MAXIMUM_OWNED_GUILDS_PER_USER, MAXIMUM_ROLES_PER_GUILD, audit_log_page_size,
    generate_invite_code, normalize_ban_reason, normalize_category_name, normalize_guild_name,
    normalize_role_name, normalize_text_channel_name, normalize_topic, validate_invite_limits,
    validate_known_permissions, validate_role_color, validate_role_order,
};
use crate::domain::permissions::{
    RoleManager, channel_permissions, guild_permissions, is_visible_to_every_member,
};
use crate::domain::profile_images::fill_profile_images;
use crate::domain::snowflake_generator::SnowflakeGenerator;
use crate::events::EventHub;
use crate::storage::audit_log::{self as audit_log_storage, ChannelEntryFilter, NewAuditEntry};
use crate::storage::guilds::{
    self as guild_storage, GuildStorageError, JoinOutcome, Membership, NewGuild, RoleFieldChanges,
    StoredChannel, StoredInvite,
};

const DEFAULT_CATEGORY_NAME: &str = "Text channels";
const DEFAULT_CHANNEL_NAME: &str = "general";
const MILLISECONDS_PER_SECOND: u64 = 1_000;

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

fn member_from_stored(member: guild_storage::StoredMember) -> Member {
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

fn channel_from_stored(channel: StoredChannel) -> Channel {
    Channel {
        id: channel.id,
        guild_id: channel.guild_id,
        kind: channel.kind,
        name: channel.name,
        topic: channel.topic,
        parent_id: channel.parent_id,
        position: channel.position,
        permission_overwrites: channel.permission_overwrites,
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

/// Who receives events about a channel (ADR-0015).
pub(crate) enum ChannelAudience {
    EveryMember,
    Members(HashSet<Snowflake>),
}

/// Which online members need a fresh guild snapshot after a permission change (ADR-0015).
enum AffectedMembers {
    Everyone,
    Some(HashSet<Snowflake>),
}

#[derive(Debug)]
pub(crate) struct Guilds {
    pool: SqlitePool,
    ids: Arc<SnowflakeGenerator>,
    events: Arc<EventHub>,
}

impl Guilds {
    pub(crate) fn new(
        pool: SqlitePool,
        ids: Arc<SnowflakeGenerator>,
        events: Arc<EventHub>,
    ) -> Self {
        Self { pool, ids, events }
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
        let membership = guild_storage::find_membership(&self.pool, guild_id, user_id)
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

    /// Creates a guild owned by `owner_id`, with a "Text channels" category and `#general`.
    pub(crate) async fn create_guild(
        &self,
        owner_id: Snowflake,
        request: CreateGuildRequest,
        now_unix_milliseconds: u64,
    ) -> Result<Guild, GuildError> {
        let name = normalize_guild_name(&request.name)?;
        let guild_id = self.ids.next_id(now_unix_milliseconds)?;
        let category_id = self.ids.next_id(now_unix_milliseconds)?;
        let initial_channels = vec![
            StoredChannel {
                id: category_id,
                guild_id,
                kind: ChannelKind::Category,
                name: DEFAULT_CATEGORY_NAME.to_owned(),
                topic: String::new(),
                parent_id: None,
                position: 0,
                permission_overwrites: Vec::new(),
            },
            StoredChannel {
                id: self.ids.next_id(now_unix_milliseconds)?,
                guild_id,
                kind: ChannelKind::Text,
                name: DEFAULT_CHANNEL_NAME.to_owned(),
                topic: String::new(),
                parent_id: Some(category_id),
                position: 1,
                permission_overwrites: Vec::new(),
            },
        ];
        guild_storage::insert_guild(
            &self.pool,
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
        Ok(guild_storage::list_user_guilds(&self.pool, user_id)
            .await?
            .into_iter()
            .map(|(id, name)| GuildSummary { id, name })
            .collect())
    }

    pub(crate) async fn get_guild(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
    ) -> Result<Guild, GuildError> {
        let (membership, my_permissions) =
            self.membership_and_permissions(guild_id, user_id).await?;
        // Channels the member cannot view are left out entirely (ADR-0015).
        let channels = guild_storage::list_guild_channels(&self.pool, guild_id)
            .await?
            .into_iter()
            .filter(|channel| {
                channel_permissions(
                    user_id,
                    membership.owner_id,
                    guild_id,
                    &membership.member_roles,
                    &channel.permission_overwrites,
                )
                .contains(Permissions::VIEW_CHANNEL)
            })
            .map(channel_from_stored)
            .collect();
        Ok(Guild {
            id: guild_id,
            my_role_ids: membership.member_role_ids(guild_id),
            name: membership.guild_name,
            owner_id: membership.owner_id,
            my_permissions,
            roles: guild_storage::list_roles(&self.pool, guild_id).await?,
            channels,
        })
    }

    /// Needs `MANAGE_CHANNELS`. A text channel may sit in a category of the same guild.
    pub(crate) async fn create_channel(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        request: CreateChannelRequest,
        now_unix_milliseconds: u64,
    ) -> Result<Channel, GuildError> {
        self.require_member_permissions(guild_id, user_id, Permissions::MANAGE_CHANNELS)
            .await?;
        let name = match request.kind {
            ChannelKind::Category => normalize_category_name(&request.name)?,
            ChannelKind::Text => normalize_text_channel_name(&request.name)?,
        };
        let topic = normalize_topic(&request.topic)?;
        if let Some(parent_id) = request.parent_id {
            // A category the creator cannot view is answered like a missing one.
            let parent = match self
                .channel_and_member_permissions(user_id, parent_id)
                .await
            {
                Ok((parent, _)) => Some(parent),
                Err(GuildError::NotFound) => None,
                Err(error) => return Err(error),
            };
            let is_category_of_same_guild = parent.is_some_and(|parent| {
                parent.guild_id == guild_id && parent.kind == ChannelKind::Category
            });
            if request.kind == ChannelKind::Category || !is_category_of_same_guild {
                return Err(GuildRuleViolation::Parent.into());
            }
        }
        // A new channel has no overwrites; its audience is known before it is written.
        let audience = self.audience_with_overwrites(guild_id, &[]).await?;
        let channel_id = self.ids.next_id(now_unix_milliseconds)?;
        let audit = self.audit_entry(
            guild_id,
            user_id,
            AuditAction::ChannelCreate,
            Some(channel_id),
            None,
        )?;
        let channel = guild_storage::insert_channel_last(
            &self.pool,
            &StoredChannel {
                id: channel_id,
                guild_id,
                kind: request.kind,
                name,
                topic,
                parent_id: request.parent_id,
                position: 0,
                permission_overwrites: Vec::new(),
            },
            MAXIMUM_CHANNELS_PER_GUILD,
            &audit,
        )
        .await
        .map_err(limit_reached("a guild has at most 500 channels"))?;
        let channel = channel_from_stored(channel);
        self.publish_to_audience(
            guild_id,
            &audience,
            &GatewayEvent::ChannelCreate(channel.clone()),
        );
        Ok(channel)
    }

    /// The guild channel and `user_id`'s permissions in it (overwrites applied), or `NotFound`
    /// for non-members and for members who cannot view it, so hidden channels stay hidden.
    pub(crate) async fn channel_and_member_permissions(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
    ) -> Result<(StoredChannel, Permissions), GuildError> {
        let channel = guild_storage::find_guild_channel(&self.pool, channel_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        let membership = guild_storage::find_membership(&self.pool, channel.guild_id, user_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        let permissions = channel_permissions(
            user_id,
            membership.owner_id,
            channel.guild_id,
            &membership.member_roles,
            &channel.permission_overwrites,
        );
        if !permissions.contains(Permissions::VIEW_CHANNEL) {
            return Err(GuildError::NotFound);
        }
        Ok((channel, permissions))
    }

    /// Who can view `channel` as stored now: its overwrites are read again, so a change committed
    /// since the caller's check is taken into account. Callers compute it before writing, so a
    /// failure here leaves nothing half done (ADR-0015).
    pub(crate) async fn channel_audience(
        &self,
        channel: &StoredChannel,
    ) -> Result<ChannelAudience, GuildError> {
        let overwrites = guild_storage::find_guild_channel(&self.pool, channel.id)
            .await?
            .map(|current| current.permission_overwrites)
            .unwrap_or_default();
        self.audience_with_overwrites(channel.guild_id, &overwrites)
            .await
    }

    /// `EveryMember` without looking at members when no one can be hidden from a channel with
    /// these overwrites, otherwise the members who can view it.
    async fn audience_with_overwrites(
        &self,
        guild_id: Snowflake,
        overwrites: &[PermissionOverwrite],
    ) -> Result<ChannelAudience, GuildError> {
        let roles = guild_storage::list_roles(&self.pool, guild_id).await?;
        let everyone_permissions = roles
            .iter()
            .find(|role| role.id == guild_id)
            .map_or(Permissions::empty(), |everyone| everyone.permissions);
        if is_visible_to_every_member(everyone_permissions, overwrites) {
            return Ok(ChannelAudience::EveryMember);
        }
        Ok(ChannelAudience::Members(
            self.viewer_ids(guild_id, &roles, overwrites).await?,
        ))
    }

    /// The members who can view a channel with these overwrites.
    /// shortcut: two queries over every member and role assignment of the guild per call; cache
    /// per guild if the M4 load test shows the need.
    async fn viewer_ids(
        &self,
        guild_id: Snowflake,
        roles: &[Role],
        overwrites: &[PermissionOverwrite],
    ) -> Result<HashSet<Snowflake>, GuildError> {
        let owner_id = guild_storage::find_guild_owner_id(&self.pool, guild_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        Ok(
            guild_storage::member_ids_with_role_ids(&self.pool, guild_id)
                .await?
                .into_iter()
                .filter(|(member_id, role_ids)| {
                    let member_roles: Vec<Role> = roles
                        .iter()
                        .filter(|role| role.id == guild_id || role_ids.contains(&role.id))
                        .cloned()
                        .collect();
                    channel_permissions(*member_id, owner_id, guild_id, &member_roles, overwrites)
                        .contains(Permissions::VIEW_CHANNEL)
                })
                .map(|(member_id, _)| member_id)
                .collect(),
        )
    }

    /// Delivers an event about a channel of `guild_id` to its audience; never fails, so it can
    /// follow a committed write.
    pub(crate) fn publish_to_audience(
        &self,
        guild_id: Snowflake,
        audience: &ChannelAudience,
        event: &GatewayEvent,
    ) {
        match audience {
            ChannelAudience::EveryMember => self.events.publish_to_guild(guild_id, event),
            ChannelAudience::Members(viewers) => {
                self.events
                    .publish_to_guild_members(guild_id, viewers, event);
            }
        }
    }

    /// Direct-message events, which have no guild to fan out through.
    pub(crate) fn publish_to_users(&self, user_ids: &[Snowflake], event: &GatewayEvent) {
        for user_id in user_ids {
            self.events.publish_to_user(*user_id, event);
        }
    }

    /// Sends each affected online member a fresh snapshot, since a permission change may have
    /// hidden or revealed channels for them (ADR-0015). Never fails: the change is committed, so a
    /// member whose snapshot cannot be built is skipped and resynchronises at their next `Ready`.
    async fn resynchronize(&self, guild_id: Snowflake, affected: AffectedMembers) {
        let online = self.events.online_user_ids_of_guild(guild_id);
        let recipients: Vec<Snowflake> = match affected {
            AffectedMembers::Everyone => online.into_iter().collect(),
            AffectedMembers::Some(user_ids) => online.intersection(&user_ids).copied().collect(),
        };
        for user_id in recipients {
            match self.get_guild(user_id, guild_id).await {
                Ok(guild) => self
                    .events
                    .publish_to_user(user_id, &GatewayEvent::GuildCreate(Box::new(guild))),
                // Left meanwhile: their GUILD_DELETE is already on its way.
                Err(GuildError::NotFound) => {}
                Err(error) => {
                    tracing::warn!(%error, "cannot resynchronize a member after a permission change");
                }
            }
        }
    }

    /// The ids of every channel `user_id` can view, across their guilds.
    pub(crate) async fn visible_channel_ids(
        &self,
        user_id: Snowflake,
    ) -> Result<HashSet<Snowflake>, GuildError> {
        let mut visible = HashSet::new();
        for (guild_id, _) in guild_storage::list_user_guilds(&self.pool, user_id).await? {
            match self.get_guild(user_id, guild_id).await {
                Ok(guild) => visible.extend(guild.channels.iter().map(|channel| channel.id)),
                Err(GuildError::NotFound) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(visible)
    }

    /// Needs `MANAGE_CHANNELS`.
    pub(crate) async fn update_channel(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        request: UpdateChannelRequest,
    ) -> Result<Channel, GuildError> {
        let (channel, permissions) = self
            .channel_and_member_permissions(user_id, channel_id)
            .await?;
        if !permissions.contains(Permissions::MANAGE_CHANNELS) {
            return Err(GuildError::MissingPermissions);
        }
        let name = request
            .name
            .map(|name| match channel.kind {
                ChannelKind::Category => normalize_category_name(&name),
                ChannelKind::Text => normalize_text_channel_name(&name),
            })
            .transpose()?;
        let topic = request
            .topic
            .map(|topic| normalize_topic(&topic))
            .transpose()?;
        let changes: Vec<AuditChange> = [
            text_change(AuditChangeKey::Name, &channel.name, name.as_deref()),
            text_change(AuditChangeKey::Topic, &channel.topic, topic.as_deref()),
        ]
        .into_iter()
        .flatten()
        .collect();
        // Nothing to write, record or send.
        if changes.is_empty() {
            return Ok(channel_from_stored(channel));
        }
        let audience = self.channel_audience(&channel).await?;
        let audit = NewAuditEntry {
            changes,
            ..self.audit_entry(
                channel.guild_id,
                user_id,
                AuditAction::ChannelUpdate,
                Some(channel.id),
                None,
            )?
        };
        let updated = guild_storage::update_channel_fields(
            &self.pool,
            channel.id,
            name.as_deref(),
            topic.as_deref(),
            &audit,
        )
        .await?
        .ok_or(GuildError::NotFound)?;
        let updated = channel_from_stored(updated);
        self.publish_to_audience(
            updated.guild_id,
            &audience,
            &GatewayEvent::ChannelUpdate(updated.clone()),
        );
        Ok(updated)
    }

    /// Needs `MANAGE_CHANNELS`. A deleted category's channels stay, uncategorised.
    pub(crate) async fn delete_channel(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
    ) -> Result<(), GuildError> {
        let (channel, permissions) = self
            .channel_and_member_permissions(user_id, channel_id)
            .await?;
        if !permissions.contains(Permissions::MANAGE_CHANNELS) {
            return Err(GuildError::MissingPermissions);
        }
        // Who could view it is decided before it disappears.
        let audience = self.channel_audience(&channel).await?;
        let audit = self.audit_entry(
            channel.guild_id,
            user_id,
            AuditAction::ChannelDelete,
            Some(channel.id),
            None,
        )?;
        guild_storage::delete_channel(&self.pool, channel.id, &audit).await?;
        self.publish_to_audience(
            channel.guild_id,
            &audience,
            &GatewayEvent::ChannelDelete {
                guild_id: channel.guild_id,
                id: channel.id,
            },
        );
        Ok(())
    }

    /// Needs `MANAGE_ROLES` in the channel, and only permissions the member holds there may be
    /// granted or taken away. A role target must be `@everyone` or below the member's highest
    /// role (the owner sets any); a member target must belong to the guild, whatever their rank,
    /// as on Discord. Only `Permissions::CHANNEL_SCOPED` bits go in an overwrite.
    pub(crate) async fn set_permission_overwrite(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        target_id: Snowflake,
        request: SetPermissionOverwriteRequest,
    ) -> Result<Channel, GuildError> {
        let allow = validate_known_permissions(request.allow)?;
        let deny = validate_known_permissions(request.deny)?;
        if allow.intersects(deny) {
            return Err(GuildRuleViolation::OverwriteAllowsAndDeniesOneBit.into());
        }
        if !Permissions::CHANNEL_SCOPED.contains(allow | deny) {
            return Err(GuildRuleViolation::OverwriteOutsideChannelScope.into());
        }
        let wanted = PermissionOverwrite {
            id: target_id,
            kind: request.kind,
            allow,
            deny,
        };
        let (channel, held) = self.channel_managed_by(user_id, channel_id).await?;
        self.require_overwrite_target(user_id, &channel, &wanted)
            .await?;
        let (current_allow, current_deny) = channel
            .permission_overwrites
            .iter()
            .find(|overwrite| overwrite.id == target_id)
            .map_or((Permissions::empty(), Permissions::empty()), |current| {
                (current.allow, current.deny)
            });
        let changed =
            current_allow.symmetric_difference(allow) | current_deny.symmetric_difference(deny);
        if !held.contains(changed) {
            return Err(GuildError::MissingPermissions);
        }
        let exists = channel
            .permission_overwrites
            .iter()
            .any(|overwrite| overwrite.id == target_id);
        if exists && changed.is_empty() {
            return Ok(channel_from_stored(channel));
        }
        let viewers_before = self.viewers_before_overwrite_change(&channel).await?;
        let audit = NewAuditEntry {
            changes: [
                permissions_change(AuditChangeKey::Allow, current_allow, allow),
                permissions_change(AuditChangeKey::Deny, current_deny, deny),
            ]
            .into_iter()
            .flatten()
            .collect(),
            ..self.audit_entry(
                channel.guild_id,
                user_id,
                AuditAction::OverwriteSet,
                Some(target_id),
                Some(channel.id),
            )?
        };
        guild_storage::set_overwrite(
            &self.pool,
            channel.id,
            &wanted,
            held,
            MAXIMUM_OVERWRITES_PER_CHANNEL,
            &audit,
        )
        .await
        .map_err(limit_reached(
            "a channel has at most 100 permission overwrites",
        ))?;
        self.publish_overwrite_change(&channel, &viewers_before)
            .await
    }

    /// Needs `MANAGE_ROLES` in the channel and every permission of the overwrite. Works for the
    /// overwrite of a member who left or of a role deleted meanwhile, so none is stuck.
    pub(crate) async fn delete_permission_overwrite(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        target_id: Snowflake,
    ) -> Result<Channel, GuildError> {
        let (channel, held) = self.channel_managed_by(user_id, channel_id).await?;
        let current = channel
            .permission_overwrites
            .iter()
            .find(|overwrite| overwrite.id == target_id)
            .cloned()
            .ok_or(GuildError::NotFound)?;
        if current.kind == OverwriteKind::Role {
            let role = guild_storage::list_roles(&self.pool, channel.guild_id)
                .await?
                .into_iter()
                .find(|role| role.id == target_id);
            if let Some(role) = role
                && !self
                    .role_manager(user_id, channel.guild_id)
                    .await?
                    .can_target_role_in_overwrites(&role, channel.guild_id)
            {
                return Err(GuildError::MissingPermissions);
            }
        }
        if !held.contains(current.allow | current.deny) {
            return Err(GuildError::MissingPermissions);
        }
        let viewers_before = self.viewers_before_overwrite_change(&channel).await?;
        let audit = NewAuditEntry {
            changes: [
                permissions_change(AuditChangeKey::Allow, current.allow, Permissions::empty()),
                permissions_change(AuditChangeKey::Deny, current.deny, Permissions::empty()),
            ]
            .into_iter()
            .flatten()
            .collect(),
            ..self.audit_entry(
                channel.guild_id,
                user_id,
                AuditAction::OverwriteDelete,
                Some(target_id),
                Some(channel.id),
            )?
        };
        // Refused if the overwrite gained a permission the member lacks since it was read.
        if !guild_storage::delete_overwrite(&self.pool, channel.id, target_id, held, &audit).await?
        {
            return Err(GuildError::Conflict);
        }
        self.publish_overwrite_change(&channel, &viewers_before)
            .await
    }

    /// The channel and the member's permissions in it, provided they hold `MANAGE_ROLES` there.
    async fn channel_managed_by(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
    ) -> Result<(StoredChannel, Permissions), GuildError> {
        let (channel, permissions) = self
            .channel_and_member_permissions(user_id, channel_id)
            .await?;
        if permissions.contains(Permissions::MANAGE_ROLES) {
            Ok((channel, permissions))
        } else {
            Err(GuildError::MissingPermissions)
        }
    }

    async fn role_manager(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
    ) -> Result<RoleManager, GuildError> {
        let membership = guild_storage::find_membership(&self.pool, guild_id, user_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        Ok(RoleManager::new(
            user_id,
            membership.owner_id,
            &membership.member_roles,
        ))
    }

    /// A role target must be a role of the guild within `user_id`'s reach; a member target must
    /// be a member of the guild.
    async fn require_overwrite_target(
        &self,
        user_id: Snowflake,
        channel: &StoredChannel,
        wanted: &PermissionOverwrite,
    ) -> Result<(), GuildError> {
        let guild_id = channel.guild_id;
        match wanted.kind {
            OverwriteKind::Role => {
                let role = guild_storage::list_roles(&self.pool, guild_id)
                    .await?
                    .into_iter()
                    .find(|role| role.id == wanted.id)
                    .ok_or(GuildError::NotFound)?;
                if !self
                    .role_manager(user_id, guild_id)
                    .await?
                    .can_target_role_in_overwrites(&role, guild_id)
                {
                    return Err(GuildError::MissingPermissions);
                }
            }
            OverwriteKind::Member => {
                guild_storage::find_member(&self.pool, guild_id, wanted.id)
                    .await?
                    .ok_or(GuildError::NotFound)?;
            }
        }
        Ok(())
    }

    /// Members who start or stop seeing the channel get a fresh snapshot; those who keep seeing
    /// it get a `CHANNEL_UPDATE` with its new overwrites (ADR-0015).
    async fn publish_overwrite_change(
        &self,
        channel: &StoredChannel,
        viewers_before: &HashSet<Snowflake>,
    ) -> Result<Channel, GuildError> {
        let updated = guild_storage::find_guild_channel(&self.pool, channel.id)
            .await?
            .ok_or(GuildError::NotFound)?;
        let roles = guild_storage::list_roles(&self.pool, channel.guild_id).await?;
        let viewers_after = self
            .viewer_ids(channel.guild_id, &roles, &updated.permission_overwrites)
            .await?;
        let still_viewing: HashSet<Snowflake> = viewers_before
            .intersection(&viewers_after)
            .copied()
            .collect();
        let updated_channel = channel_from_stored(updated);
        self.events.publish_to_guild_members(
            channel.guild_id,
            &still_viewing,
            &GatewayEvent::ChannelUpdate(updated_channel.clone()),
        );
        self.resynchronize(
            channel.guild_id,
            AffectedMembers::Some(
                viewers_before
                    .symmetric_difference(&viewers_after)
                    .copied()
                    .collect(),
            ),
        )
        .await;
        Ok(updated_channel)
    }

    /// Who can view `channel` now, before an overwrite change.
    async fn viewers_before_overwrite_change(
        &self,
        channel: &StoredChannel,
    ) -> Result<HashSet<Snowflake>, GuildError> {
        let roles = guild_storage::list_roles(&self.pool, channel.guild_id).await?;
        self.viewer_ids(channel.guild_id, &roles, &channel.permission_overwrites)
            .await
    }

    /// Needs `CREATE_INVITE`.
    pub(crate) async fn create_invite(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        request: CreateInviteRequest,
        now_unix_milliseconds: u64,
    ) -> Result<Invite, GuildError> {
        self.require_member_permissions(guild_id, user_id, Permissions::CREATE_INVITE)
            .await?;
        validate_invite_limits(request.max_uses, request.expires_in_seconds)?;
        let expires_at_unix_milliseconds = request.expires_in_seconds.map(|seconds| {
            now_unix_milliseconds.saturating_add(u64::from(seconds) * MILLISECONDS_PER_SECOND)
        });
        let invite = StoredInvite {
            code: generate_invite_code()?,
            guild_id,
            creator_id: user_id,
            max_uses: request.max_uses,
            uses: 0,
            expires_at_unix_milliseconds,
            created_at_unix_milliseconds: now_unix_milliseconds,
        };
        guild_storage::insert_invite(&self.pool, &invite, MAXIMUM_ACTIVE_INVITES_PER_GUILD)
            .await
            .map_err(limit_reached("a guild has at most 1000 active invites"))?;
        Ok(Invite {
            code: invite.code,
            guild_id,
            uses: invite.uses,
            max_uses: invite.max_uses,
            expires_at_unix_milliseconds,
        })
    }

    /// Joins (or, for an existing member, simply opens) the guild an invite points to.
    pub(crate) async fn join_with_invite(
        &self,
        user_id: Snowflake,
        code: &str,
        now_unix_milliseconds: u64,
    ) -> Result<Guild, GuildError> {
        let (guild_id, is_new_member) =
            match guild_storage::join_with_invite(&self.pool, code, user_id, now_unix_milliseconds)
                .await?
            {
                JoinOutcome::Joined {
                    guild_id,
                    is_new_member,
                } => (guild_id, is_new_member),
                JoinOutcome::InvalidInvite => return Err(GuildError::InvalidInvite),
                JoinOutcome::Banned => return Err(GuildError::Banned),
            };
        if is_new_member {
            // Subscribed before the snapshot, so nothing between the two is missed.
            self.events.subscribe_user_to_guild(user_id, guild_id);
            // A kick or ban committed between the join and the subscription found nothing to
            // unsubscribe: check again, so the removed user hears nothing more.
            if guild_storage::find_membership(&self.pool, guild_id, user_id)
                .await?
                .is_none()
            {
                self.events.unsubscribe_user_from_guild(user_id, guild_id);
                return Err(GuildError::NotFound);
            }
        }
        let guild = self.get_guild(user_id, guild_id).await?;
        if is_new_member {
            if let Some(member) = guild_storage::find_member(&self.pool, guild_id, user_id).await? {
                let mut member = member_from_stored(member);
                // The join is stored: announce it even without the pictures.
                if let Err(error) = fill_profile_images(&self.pool, [&mut member.user]).await {
                    tracing::warn!(%error, "cannot read a new member's pictures");
                }
                self.events
                    .publish_to_guild(guild_id, &GatewayEvent::GuildMemberAdd { guild_id, member });
            }
            if self.events.is_online(user_id) {
                self.events.publish_to_guild(
                    guild_id,
                    &GatewayEvent::PresenceUpdate {
                        user_id,
                        status: PresenceStatus::Online,
                    },
                );
            }
            self.events
                .publish_to_user(user_id, &GatewayEvent::GuildCreate(Box::new(guild.clone())));
        }
        Ok(guild)
    }

    /// The member acting on roles and the role they target, or `NotFound` for non-members and
    /// roles of other guilds, `MissingPermissions` outside the hierarchy (`RoleManager`).
    async fn manager_and_manageable_role(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        role_id: Snowflake,
    ) -> Result<(RoleManager, Role), GuildError> {
        let membership = guild_storage::find_membership(&self.pool, guild_id, user_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        let manager = RoleManager::new(user_id, membership.owner_id, &membership.member_roles);
        if !manager.permissions.contains(Permissions::MANAGE_ROLES) {
            return Err(GuildError::MissingPermissions);
        }
        let role = guild_storage::list_roles(&self.pool, guild_id)
            .await?
            .into_iter()
            .find(|role| role.id == role_id)
            .ok_or(GuildError::NotFound)?;
        if manager.can_manage_role(&role, guild_id) {
            Ok((manager, role))
        } else {
            Err(GuildError::MissingPermissions)
        }
    }

    fn publish_roles(&self, guild_id: Snowflake, roles: Vec<Role>) {
        self.events.publish_to_guild(
            guild_id,
            &GatewayEvent::GuildRolesUpdate { guild_id, roles },
        );
    }

    /// Needs `MANAGE_ROLES`, and only permissions the creator holds. The role starts just above
    /// `@everyone`, so its creator can manage it whatever their own rank.
    pub(crate) async fn create_role(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        request: CreateRoleRequest,
        now_unix_milliseconds: u64,
    ) -> Result<Role, GuildError> {
        let name = normalize_role_name(&request.name)?;
        let color = validate_role_color(request.color)?;
        let permissions = validate_known_permissions(request.permissions)?;
        let membership = guild_storage::find_membership(&self.pool, guild_id, user_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        let manager = RoleManager::new(user_id, membership.owner_id, &membership.member_roles);
        if !manager.permissions.contains(Permissions::MANAGE_ROLES)
            || !manager.can_change_permissions(Permissions::empty(), permissions)
        {
            return Err(GuildError::MissingPermissions);
        }
        let role_id = self.ids.next_id(now_unix_milliseconds)?;
        let audit = self.audit_entry(
            guild_id,
            user_id,
            AuditAction::RoleCreate,
            Some(role_id),
            None,
        )?;
        let roles = guild_storage::insert_role_above_everyone(
            &self.pool,
            guild_id,
            &Role {
                id: role_id,
                name,
                color,
                position: 1,
                permissions,
            },
            MAXIMUM_ROLES_PER_GUILD,
            &audit,
        )
        .await
        .map_err(limit_reached("a guild has at most 250 roles"))?;
        let role = roles
            .iter()
            .find(|role| role.id == role_id)
            .cloned()
            .ok_or(GuildError::NotFound)?;
        self.publish_roles(guild_id, roles);
        Ok(role)
    }

    /// Needs `MANAGE_ROLES` over this role (`RoleManager`). `@everyone` changes only its
    /// permissions.
    pub(crate) async fn update_role(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        role_id: Snowflake,
        request: UpdateRoleRequest,
    ) -> Result<Role, GuildError> {
        if role_id == guild_id && (request.name.is_some() || request.color.is_some()) {
            return Err(GuildRuleViolation::EveryoneRole.into());
        }
        let name = request
            .name
            .map(|name| normalize_role_name(&name))
            .transpose()?;
        let color = request.color.map(validate_role_color).transpose()?;
        let permissions = request
            .permissions
            .map(validate_known_permissions)
            .transpose()?;
        let (manager, role) = self
            .manager_and_manageable_role(user_id, guild_id, role_id)
            .await?;
        if let Some(permissions) = permissions
            && !manager.can_change_permissions(role.permissions, permissions)
        {
            return Err(GuildError::MissingPermissions);
        }
        // What storage will write: only the bits the manager holds take the new value.
        let effective_permissions = permissions.map(|wanted| {
            role.permissions.difference(manager.permissions)
                | wanted.intersection(manager.permissions)
        });
        let changes: Vec<AuditChange> = [
            text_change(AuditChangeKey::Name, &role.name, name.as_deref()),
            color.and_then(|color| {
                text_change(
                    AuditChangeKey::Color,
                    &role.color.to_string(),
                    Some(&color.to_string()),
                )
            }),
            effective_permissions.and_then(|permissions| {
                permissions_change(AuditChangeKey::Permissions, role.permissions, permissions)
            }),
        ]
        .into_iter()
        .flatten()
        .collect();
        // Nothing to write, record or send.
        if changes.is_empty() {
            return Ok(role);
        }
        let audit = NewAuditEntry {
            changes,
            ..self.audit_entry(
                guild_id,
                user_id,
                AuditAction::RoleUpdate,
                Some(role_id),
                None,
            )?
        };
        let updated = guild_storage::update_role_fields(
            &self.pool,
            guild_id,
            role_id,
            &RoleFieldChanges {
                name: name.as_deref(),
                color,
                permissions,
            },
            manager.permissions,
            &audit,
        )
        .await?
        .ok_or(GuildError::NotFound)?;
        self.publish_roles(
            guild_id,
            guild_storage::list_roles(&self.pool, guild_id).await?,
        );
        if updated.permissions != role.permissions {
            self.resynchronize(guild_id, AffectedMembers::Everyone)
                .await;
        }
        Ok(updated)
    }

    /// Needs `MANAGE_ROLES`; anyone but the owner moves only roles they manage
    /// (`RoleManager::can_reorder_roles`). Returns the guild's new role list.
    pub(crate) async fn reorder_roles(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        request: ReorderRolesRequest,
    ) -> Result<Vec<Role>, GuildError> {
        let membership = guild_storage::find_membership(&self.pool, guild_id, user_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        let roles = guild_storage::list_roles(&self.pool, guild_id).await?;
        // The manager's rank comes from the same read as the order the storage guard checks, so
        // a reordering committed between the two reads cannot leave them a stale, higher rank.
        let member_role_ids = membership.member_role_ids(guild_id);
        let member_roles: Vec<Role> = roles
            .iter()
            .filter(|role| role.id == guild_id || member_role_ids.contains(&role.id))
            .cloned()
            .collect();
        let manager = RoleManager::new(user_id, membership.owner_id, &member_roles);
        if !manager.permissions.contains(Permissions::MANAGE_ROLES) {
            return Err(GuildError::MissingPermissions);
        }
        let named_roles: Vec<Role> = roles
            .iter()
            .filter(|role| role.id != guild_id)
            .cloned()
            .collect();
        let current_order: Vec<Snowflake> = named_roles.iter().map(|role| role.id).collect();
        validate_role_order(&current_order, &request.role_ids)?;
        if !manager.can_reorder_roles(&named_roles, &request.role_ids, guild_id) {
            return Err(GuildError::MissingPermissions);
        }
        if request.role_ids == current_order {
            return Ok(roles);
        }
        let audit = self.audit_entry(guild_id, user_id, AuditAction::RolesReorder, None, None)?;
        let roles = guild_storage::reorder_roles(
            &self.pool,
            guild_id,
            &current_order,
            &request.role_ids,
            &audit,
        )
        .await?
        .ok_or(GuildError::Conflict)?;
        self.publish_roles(guild_id, roles.clone());
        Ok(roles)
    }

    /// Needs `MANAGE_ROLES` over this role (`RoleManager`). `@everyone` cannot be deleted.
    pub(crate) async fn delete_role(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        role_id: Snowflake,
    ) -> Result<(), GuildError> {
        if role_id == guild_id {
            return Err(GuildRuleViolation::EveryoneRole.into());
        }
        self.manager_and_manageable_role(user_id, guild_id, role_id)
            .await?;
        let audit = self.audit_entry(
            guild_id,
            user_id,
            AuditAction::RoleDelete,
            Some(role_id),
            None,
        )?;
        if !guild_storage::delete_role(&self.pool, guild_id, role_id, &audit).await? {
            return Err(GuildError::NotFound);
        }
        self.publish_roles(
            guild_id,
            guild_storage::list_roles(&self.pool, guild_id).await?,
        );
        self.resynchronize(guild_id, AffectedMembers::Everyone)
            .await;
        Ok(())
    }

    /// Needs `MANAGE_ROLES` over this role (`RoleManager`), whoever the member is.
    pub(crate) async fn add_member_role(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        member_id: Snowflake,
        role_id: Snowflake,
    ) -> Result<(), GuildError> {
        if role_id == guild_id {
            return Err(GuildRuleViolation::EveryoneRole.into());
        }
        let (manager, _) = self
            .manager_and_manageable_role(user_id, guild_id, role_id)
            .await?;
        let audit = self.audit_entry(
            guild_id,
            user_id,
            AuditAction::MemberRoleAdd,
            Some(member_id),
            Some(role_id),
        )?;
        let role_ids = guild_storage::add_member_role(
            &self.pool,
            guild_id,
            member_id,
            role_id,
            manager.permissions,
            &audit,
        )
        .await?
        .ok_or(GuildError::NotFound)?;
        self.publish_member_roles(guild_id, member_id, role_ids);
        self.resynchronize(guild_id, AffectedMembers::Some(HashSet::from([member_id])))
            .await;
        Ok(())
    }

    /// Needs `MANAGE_ROLES` over this role (`RoleManager`), whoever the member is.
    pub(crate) async fn remove_member_role(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        member_id: Snowflake,
        role_id: Snowflake,
    ) -> Result<(), GuildError> {
        if role_id == guild_id {
            return Err(GuildRuleViolation::EveryoneRole.into());
        }
        self.manager_and_manageable_role(user_id, guild_id, role_id)
            .await?;
        let audit = self.audit_entry(
            guild_id,
            user_id,
            AuditAction::MemberRoleRemove,
            Some(member_id),
            Some(role_id),
        )?;
        let role_ids =
            guild_storage::remove_member_role(&self.pool, guild_id, member_id, role_id, &audit)
                .await?
                .ok_or(GuildError::NotFound)?;
        self.publish_member_roles(guild_id, member_id, role_ids);
        self.resynchronize(guild_id, AffectedMembers::Some(HashSet::from([member_id])))
            .await;
        Ok(())
    }

    fn publish_member_roles(
        &self,
        guild_id: Snowflake,
        user_id: Snowflake,
        role_ids: Vec<Snowflake>,
    ) {
        self.events.publish_to_guild(
            guild_id,
            &GatewayEvent::GuildMemberUpdate {
                guild_id,
                user_id,
                role_ids,
            },
        );
    }

    /// Members only. shortcut: unpaginated; fine for M1 community sizes, paginate with the member list UI.
    pub(crate) async fn list_members(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
    ) -> Result<Vec<Member>, GuildError> {
        self.membership_and_permissions(guild_id, user_id).await?;
        let mut members: Vec<Member> =
            guild_storage::list_members(&self.pool, guild_id, MAXIMUM_MEMBERS_LISTED)
                .await?
                .into_iter()
                .map(member_from_stored)
                .collect();
        fill_profile_images(
            &self.pool,
            members.iter_mut().map(|member| &mut member.user),
        )
        .await
        .map_err(|error| GuildError::Internal(Box::new(error)))?;
        Ok(members)
    }

    pub(crate) async fn leave_guild(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
    ) -> Result<(), GuildError> {
        let (membership, _) = self.membership_and_permissions(guild_id, user_id).await?;
        if membership.owner_id == user_id {
            return Err(GuildError::OwnerCannotLeave);
        }
        guild_storage::delete_member(&self.pool, guild_id, user_id, None).await?;
        self.publish_member_removed(guild_id, user_id);
        Ok(())
    }

    /// The member's sessions stop receiving the guild, which disappears for them, and the other
    /// members see them go.
    fn publish_member_removed(&self, guild_id: Snowflake, user_id: Snowflake) {
        self.events.unsubscribe_user_from_guild(user_id, guild_id);
        self.events.publish_to_guild(
            guild_id,
            &GatewayEvent::GuildMemberRemove { guild_id, user_id },
        );
        self.events
            .publish_to_user(user_id, &GatewayEvent::GuildDelete { id: guild_id });
    }

    /// The moderator, once `needed` is checked and `target_id` is neither them nor the owner.
    async fn require_moderator_over_target(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        target_id: Snowflake,
        needed: Permissions,
    ) -> Result<(Membership, RoleManager), GuildError> {
        let membership = guild_storage::find_membership(&self.pool, guild_id, user_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        let manager = RoleManager::new(user_id, membership.owner_id, &membership.member_roles);
        if !manager.permissions.contains(needed) {
            return Err(GuildError::MissingPermissions);
        }
        if target_id == user_id {
            return Err(GuildRuleViolation::ModerationTarget.into());
        }
        if target_id == membership.owner_id {
            return Err(GuildError::MissingPermissions);
        }
        Ok((membership, manager))
    }

    /// Returns whether `target_id` is a member, after refusing (`MissingPermissions`) a member
    /// ranked at or above the moderator.
    async fn require_rank_below_moderator_if_member(
        &self,
        manager: &RoleManager,
        membership: &Membership,
        guild_id: Snowflake,
        target_id: Snowflake,
    ) -> Result<bool, GuildError> {
        let Some(target) = guild_storage::find_membership(&self.pool, guild_id, target_id).await?
        else {
            return Ok(false);
        };
        let target_is_owner = target_id == membership.owner_id;
        if manager.outranks(target_is_owner, &target.member_roles) {
            Ok(true)
        } else {
            Err(GuildError::MissingPermissions)
        }
    }

    /// Needs `KICK_MEMBERS`, over a member ranked strictly below the moderator; never the owner
    /// or oneself. The member may join again with an invite.
    pub(crate) async fn kick_member(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        target_id: Snowflake,
    ) -> Result<(), GuildError> {
        let (membership, manager) = self
            .require_moderator_over_target(user_id, guild_id, target_id, Permissions::KICK_MEMBERS)
            .await?;
        if !self
            .require_rank_below_moderator_if_member(&manager, &membership, guild_id, target_id)
            .await?
        {
            return Err(GuildError::NotFound);
        }
        let audit = self.audit_entry(
            guild_id,
            user_id,
            AuditAction::MemberKick,
            Some(target_id),
            None,
        )?;
        guild_storage::delete_member(&self.pool, guild_id, target_id, Some(&audit)).await?;
        self.publish_member_removed(guild_id, target_id);
        Ok(())
    }

    /// Needs `BAN_MEMBERS`. A member must rank strictly below the moderator; any other existing
    /// user may be banned ahead of time. Never the owner or oneself.
    pub(crate) async fn ban_member(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        target_id: Snowflake,
        request: CreateBanRequest,
        now_unix_milliseconds: u64,
    ) -> Result<(), GuildError> {
        let reason = normalize_ban_reason(&request.reason)?;
        let (membership, manager) = self
            .require_moderator_over_target(user_id, guild_id, target_id, Permissions::BAN_MEMBERS)
            .await?;
        if !self
            .require_rank_below_moderator_if_member(&manager, &membership, guild_id, target_id)
            .await?
            && !guild_storage::user_exists(&self.pool, target_id).await?
        {
            return Err(GuildError::NotFound);
        }
        let audit = NewAuditEntry {
            reason: reason.clone(),
            ..self.audit_entry(
                guild_id,
                user_id,
                AuditAction::MemberBanAdd,
                Some(target_id),
                None,
            )?
        };
        let was_member = guild_storage::ban_and_remove_member(
            &self.pool,
            guild_id,
            target_id,
            &reason,
            user_id,
            now_unix_milliseconds,
            &audit,
        )
        .await?;
        if was_member {
            self.publish_member_removed(guild_id, target_id);
        }
        Ok(())
    }

    /// Needs `BAN_MEMBERS`.
    pub(crate) async fn unban_member(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        target_id: Snowflake,
    ) -> Result<(), GuildError> {
        self.require_member_permissions(guild_id, user_id, Permissions::BAN_MEMBERS)
            .await?;
        let audit = self.audit_entry(
            guild_id,
            user_id,
            AuditAction::MemberBanRemove,
            Some(target_id),
            None,
        )?;
        if guild_storage::delete_ban(&self.pool, guild_id, target_id, &audit).await? {
            Ok(())
        } else {
            Err(GuildError::NotFound)
        }
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
            &self.pool,
            guild_id,
            query.before,
            limit,
            &filter,
        )
        .await?)
    }

    /// Needs `BAN_MEMBERS`. shortcut: capped, not paginated; paginate with the bans screen.
    pub(crate) async fn list_bans(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
    ) -> Result<Vec<Ban>, GuildError> {
        self.require_member_permissions(guild_id, user_id, Permissions::BAN_MEMBERS)
            .await?;
        let mut bans: Vec<Ban> =
            guild_storage::list_bans(&self.pool, guild_id, MAXIMUM_BANS_LISTED)
                .await?
                .into_iter()
                .map(|ban| Ban {
                    user: PublicUser {
                        id: ban.user_id,
                        username: ban.username,
                        display_name: ban.display_name,
                        avatar: None,
                        banner: None,
                    },
                    reason: ban.reason,
                    created_at_unix_milliseconds: ban.created_at_unix_milliseconds,
                })
                .collect();
        fill_profile_images(&self.pool, bans.iter_mut().map(|ban| &mut ban.user))
            .await
            .map_err(|error| GuildError::Internal(Box::new(error)))?;
        Ok(bans)
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::RegisterRequest;

    use super::*;
    use crate::domain::account_rules::RegistrationMode;
    use crate::domain::accounts::Accounts;
    use crate::domain::accounts::tests::{NOW, migrated_pool, register_request};
    use crate::domain::guild_rules::MAXIMUM_BAN_REASON_LENGTH;

    struct Fixture {
        _directory: tempfile::TempDir,
        pool: SqlitePool,
        guilds: Guilds,
        events: Arc<EventHub>,
        owner: Snowflake,
        other: Snowflake,
        third: Snowflake,
    }

    async fn fixture() -> Fixture {
        let (directory, pool) = migrated_pool().await;
        let events = Arc::new(EventHub::default());
        let ids = Arc::new(SnowflakeGenerator::default());
        let accounts = Accounts::new(
            pool.clone(),
            RegistrationMode::Open,
            ids.clone(),
            Arc::default(),
        );
        let register = |request: RegisterRequest| accounts.register(request, NOW);
        let owner = register(register_request("owner", "owner@example.org"))
            .await
            .unwrap()
            .user
            .id;
        let other = register(register_request("other", "other@example.org"))
            .await
            .unwrap()
            .user
            .id;
        let third = register(register_request("third", "third@example.org"))
            .await
            .unwrap()
            .user
            .id;
        Fixture {
            _directory: directory,
            third,
            guilds: Guilds::new(pool.clone(), ids, events.clone()),
            events,
            pool,
            owner,
            other,
        }
    }

    fn create_request(name: &str) -> CreateGuildRequest {
        CreateGuildRequest { name: name.into() }
    }

    fn text_channel(name: &str, parent_id: Option<Snowflake>) -> CreateChannelRequest {
        CreateChannelRequest {
            kind: ChannelKind::Text,
            name: name.into(),
            topic: String::new(),
            parent_id,
        }
    }

    #[tokio::test]
    async fn new_guild_has_a_default_category_and_general_and_the_owner_has_everything() {
        let fixture = fixture().await;
        let guild = fixture
            .guilds
            .create_guild(fixture.owner, create_request(" Mellow Dev "), NOW)
            .await
            .unwrap();
        fixture.pool.close().await;

        assert_eq!(guild.name, "Mellow Dev");
        assert_eq!(guild.owner_id, fixture.owner);
        assert_eq!(guild.my_permissions, Permissions::all());
        let names: Vec<_> = guild
            .channels
            .iter()
            .map(|channel| channel.name.as_str())
            .collect();
        assert_eq!(names, ["Text channels", "general"]);
        assert_eq!(guild.channels[1].parent_id, Some(guild.channels[0].id));
    }

    #[tokio::test]
    async fn non_members_get_not_found_and_members_without_permission_are_refused() {
        let fixture = fixture().await;
        let guild = fixture
            .guilds
            .create_guild(fixture.owner, create_request("Private"), NOW)
            .await
            .unwrap();
        let outsider_view = fixture.guilds.get_guild(fixture.other, guild.id).await;
        let invite = fixture
            .guilds
            .create_invite(fixture.owner, guild.id, CreateInviteRequest::default(), NOW)
            .await
            .unwrap();
        let joined = fixture
            .guilds
            .join_with_invite(fixture.other, &invite.code, NOW)
            .await
            .unwrap();
        let member_creates_channel = fixture
            .guilds
            .create_channel(fixture.other, guild.id, text_channel("nope", None), NOW)
            .await;
        fixture.pool.close().await;

        assert!(matches!(outsider_view, Err(GuildError::NotFound)));
        assert_eq!(joined.my_permissions, Permissions::DEFAULT_EVERYONE);
        assert!(matches!(
            member_creates_channel,
            Err(GuildError::MissingPermissions)
        ));
    }

    #[tokio::test]
    async fn invites_respect_max_uses_and_expiry_and_rejoining_costs_nothing() {
        let fixture = fixture().await;
        let guild = fixture
            .guilds
            .create_guild(fixture.owner, create_request("Club"), NOW)
            .await
            .unwrap();
        let single_use = fixture
            .guilds
            .create_invite(
                fixture.owner,
                guild.id,
                CreateInviteRequest {
                    max_uses: Some(1),
                    expires_in_seconds: Some(60),
                },
                NOW,
            )
            .await
            .unwrap();

        // The owner is already a member: opening the invite does not consume its only use.
        fixture
            .guilds
            .join_with_invite(fixture.owner, &single_use.code, NOW)
            .await
            .unwrap();
        let expired = fixture
            .guilds
            .join_with_invite(fixture.other, &single_use.code, NOW + 60_000)
            .await;
        fixture
            .guilds
            .join_with_invite(fixture.other, &single_use.code, NOW)
            .await
            .unwrap();
        let members = fixture
            .guilds
            .list_members(fixture.owner, guild.id)
            .await
            .unwrap();
        let unknown = fixture
            .guilds
            .join_with_invite(fixture.other, "doesnotexist", NOW)
            .await;
        fixture.pool.close().await;

        assert!(matches!(expired, Err(GuildError::InvalidInvite)));
        assert_eq!(members.len(), 2);
        assert!(matches!(unknown, Err(GuildError::InvalidInvite)));
    }

    #[tokio::test]
    async fn channels_are_created_under_categories_renamed_and_deleted() {
        let fixture = fixture().await;
        let guild = fixture
            .guilds
            .create_guild(fixture.owner, create_request("Club"), NOW)
            .await
            .unwrap();
        let category_id = guild.channels[0].id;
        let general_id = guild.channels[1].id;

        let created = fixture
            .guilds
            .create_channel(
                fixture.owner,
                guild.id,
                text_channel("Off Topic", Some(category_id)),
                NOW,
            )
            .await
            .unwrap();
        let text_as_parent = fixture
            .guilds
            .create_channel(
                fixture.owner,
                guild.id,
                text_channel("x", Some(general_id)),
                NOW,
            )
            .await;
        let renamed = fixture
            .guilds
            .update_channel(
                fixture.owner,
                created.id,
                UpdateChannelRequest {
                    name: Some("Random Stuff".into()),
                    topic: Some("Anything goes".into()),
                },
            )
            .await
            .unwrap();
        fixture
            .guilds
            .delete_channel(fixture.owner, category_id)
            .await
            .unwrap();
        let after = fixture
            .guilds
            .get_guild(fixture.owner, guild.id)
            .await
            .unwrap();
        fixture.pool.close().await;

        assert_eq!(created.name, "off-topic");
        assert_eq!(created.position, 2);
        assert!(matches!(
            text_as_parent,
            Err(GuildError::InvalidInput(GuildRuleViolation::Parent))
        ));
        assert_eq!(
            (renamed.name.as_str(), renamed.topic.as_str()),
            ("random-stuff", "Anything goes")
        );
        assert!(
            after
                .channels
                .iter()
                .all(|channel| channel.parent_id.is_none())
        );
        assert_eq!(after.channels.len(), 2);
    }

    #[tokio::test]
    async fn members_can_leave_but_the_owner_cannot() {
        let fixture = fixture().await;
        let guild = fixture
            .guilds
            .create_guild(fixture.owner, create_request("Club"), NOW)
            .await
            .unwrap();
        let invite = fixture
            .guilds
            .create_invite(fixture.owner, guild.id, CreateInviteRequest::default(), NOW)
            .await
            .unwrap();
        fixture
            .guilds
            .join_with_invite(fixture.other, &invite.code, NOW)
            .await
            .unwrap();

        let owner_leaves = fixture.guilds.leave_guild(fixture.owner, guild.id).await;
        fixture
            .guilds
            .leave_guild(fixture.other, guild.id)
            .await
            .unwrap();
        let my_guilds = fixture.guilds.list_my_guilds(fixture.other).await.unwrap();
        fixture.pool.close().await;

        assert!(matches!(owner_leaves, Err(GuildError::OwnerCannotLeave)));
        assert_eq!(my_guilds, Vec::new());
    }

    async fn guild_with_member(fixture: &Fixture) -> Guild {
        let guild = fixture
            .guilds
            .create_guild(fixture.owner, create_request("Club"), NOW)
            .await
            .unwrap();
        let invite = fixture
            .guilds
            .create_invite(fixture.owner, guild.id, CreateInviteRequest::default(), NOW)
            .await
            .unwrap();
        fixture
            .guilds
            .join_with_invite(fixture.other, &invite.code, NOW)
            .await
            .unwrap();
        guild
    }

    #[tokio::test]
    async fn a_used_up_invite_is_refused() {
        let fixture = fixture().await;
        let guild = fixture
            .guilds
            .create_guild(fixture.owner, create_request("Club"), NOW)
            .await
            .unwrap();
        let single_use = fixture
            .guilds
            .create_invite(
                fixture.owner,
                guild.id,
                CreateInviteRequest {
                    max_uses: Some(1),
                    expires_in_seconds: None,
                },
                NOW,
            )
            .await
            .unwrap();
        fixture
            .guilds
            .join_with_invite(fixture.other, &single_use.code, NOW)
            .await
            .unwrap();
        let second_use = fixture
            .guilds
            .join_with_invite(fixture.third, &single_use.code, NOW)
            .await;
        let uses: i64 = sqlx::query_scalar("SELECT uses FROM invites WHERE code = ?")
            .bind(&single_use.code)
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
        fixture.pool.close().await;
        assert!(matches!(second_use, Err(GuildError::InvalidInvite)));
        assert_eq!(uses, 1);
    }

    #[tokio::test]
    async fn every_guild_action_checks_membership_then_permission() {
        let fixture = fixture().await;
        let guild = guild_with_member(&fixture).await;
        let channel_id = guild.channels[1].id;
        let rename = || UpdateChannelRequest {
            name: Some("renamed".into()),
            topic: None,
        };
        let guilds = &fixture.guilds;

        // `third` is not a member: everything is NotFound.
        assert!(matches!(
            guilds
                .update_channel(fixture.third, channel_id, rename())
                .await,
            Err(GuildError::NotFound)
        ));
        assert!(matches!(
            guilds.delete_channel(fixture.third, channel_id).await,
            Err(GuildError::NotFound)
        ));
        assert!(matches!(
            guilds
                .create_invite(fixture.third, guild.id, CreateInviteRequest::default(), NOW)
                .await,
            Err(GuildError::NotFound)
        ));
        assert!(matches!(
            guilds.list_members(fixture.third, guild.id).await,
            Err(GuildError::NotFound)
        ));
        assert!(matches!(
            guilds.leave_guild(fixture.third, guild.id).await,
            Err(GuildError::NotFound)
        ));

        // `other` is a member without MANAGE_CHANNELS.
        assert!(matches!(
            guilds
                .update_channel(fixture.other, channel_id, rename())
                .await,
            Err(GuildError::MissingPermissions)
        ));
        assert!(matches!(
            guilds.delete_channel(fixture.other, channel_id).await,
            Err(GuildError::MissingPermissions)
        ));
        fixture.pool.close().await;
    }

    #[tokio::test]
    async fn parents_must_be_categories_of_the_same_guild_and_categories_cannot_nest() {
        let fixture = fixture().await;
        let guild = guild_with_member(&fixture).await;
        let other_guild = fixture
            .guilds
            .create_guild(fixture.owner, create_request("Elsewhere"), NOW)
            .await
            .unwrap();
        let foreign_category = other_guild.channels[0].id;
        let own_category = guild.channels[0].id;

        let cross_guild = fixture
            .guilds
            .create_channel(
                fixture.owner,
                guild.id,
                text_channel("x", Some(foreign_category)),
                NOW,
            )
            .await;
        let nested_category = fixture
            .guilds
            .create_channel(
                fixture.owner,
                guild.id,
                CreateChannelRequest {
                    kind: ChannelKind::Category,
                    name: "Nested".into(),
                    topic: String::new(),
                    parent_id: Some(own_category),
                },
                NOW,
            )
            .await;
        let out_of_range_parent = fixture
            .guilds
            .create_channel(
                fixture.owner,
                guild.id,
                text_channel("x", Some(Snowflake::from_raw(u64::MAX))),
                NOW,
            )
            .await;
        let out_of_range_guild = fixture
            .guilds
            .get_guild(fixture.owner, Snowflake::from_raw(u64::MAX))
            .await;
        fixture.pool.close().await;

        for result in [cross_guild, nested_category, out_of_range_parent] {
            assert!(
                matches!(
                    result,
                    Err(GuildError::InvalidInput(GuildRuleViolation::Parent))
                ),
                "{result:?}"
            );
        }
        assert!(matches!(out_of_range_guild, Err(GuildError::NotFound)));
    }

    #[tokio::test]
    async fn a_guild_holds_at_most_the_channel_quota() {
        let fixture = fixture().await;
        let guild = fixture
            .guilds
            .create_guild(fixture.owner, create_request("Busy"), NOW)
            .await
            .unwrap();
        let already_present = u32::try_from(guild.channels.len()).unwrap();
        for index in already_present..MAXIMUM_CHANNELS_PER_GUILD {
            fixture
                .guilds
                .create_channel(
                    fixture.owner,
                    guild.id,
                    text_channel(&format!("c{index}"), None),
                    NOW,
                )
                .await
                .unwrap();
        }
        let one_too_many = fixture
            .guilds
            .create_channel(fixture.owner, guild.id, text_channel("overflow", None), NOW)
            .await;
        fixture.pool.close().await;
        assert!(matches!(one_too_many, Err(GuildError::LimitReached(_))));
    }

    fn role_request(name: &str, permissions: Permissions) -> CreateRoleRequest {
        CreateRoleRequest {
            name: name.into(),
            color: 0,
            permissions,
        }
    }

    fn permissions_update(permissions: Permissions) -> UpdateRoleRequest {
        UpdateRoleRequest {
            permissions: Some(permissions),
            ..UpdateRoleRequest::default()
        }
    }

    async fn join(fixture: &Fixture, guild_id: Snowflake, user_id: Snowflake) {
        let invite = fixture
            .guilds
            .create_invite(fixture.owner, guild_id, CreateInviteRequest::default(), NOW)
            .await
            .unwrap();
        fixture
            .guilds
            .join_with_invite(user_id, &invite.code, NOW)
            .await
            .unwrap();
    }

    fn role_names_by_position(guild: &Guild) -> Vec<(&str, u32)> {
        guild
            .roles
            .iter()
            .map(|role| (role.name.as_str(), role.position))
            .collect()
    }

    #[tokio::test]
    async fn new_roles_start_above_everyone_and_deleting_one_closes_the_gap() {
        let fixture = fixture().await;
        let guild = guild_with_member(&fixture).await;
        let create = |name: &'static str| {
            fixture.guilds.create_role(
                fixture.owner,
                guild.id,
                role_request(name, Permissions::empty()),
                NOW,
            )
        };
        let first = create("first").await.unwrap();
        assert_eq!(first.position, 1);
        create("second").await.unwrap();
        let snapshot = fixture
            .guilds
            .get_guild(fixture.owner, guild.id)
            .await
            .unwrap();
        assert_eq!(
            role_names_by_position(&snapshot),
            vec![("@everyone", 0), ("second", 1), ("first", 2)]
        );

        fixture
            .guilds
            .add_member_role(fixture.owner, guild.id, fixture.other, first.id)
            .await
            .unwrap();
        fixture
            .guilds
            .delete_role(fixture.owner, guild.id, snapshot.roles[1].id)
            .await
            .unwrap();
        let snapshot = fixture
            .guilds
            .get_guild(fixture.other, guild.id)
            .await
            .unwrap();
        assert_eq!(
            role_names_by_position(&snapshot),
            vec![("@everyone", 0), ("first", 1)]
        );
        assert_eq!(snapshot.my_role_ids, vec![first.id]);
        fixture
            .guilds
            .delete_role(fixture.owner, guild.id, first.id)
            .await
            .unwrap();
        let snapshot = fixture
            .guilds
            .get_guild(fixture.other, guild.id)
            .await
            .unwrap();
        fixture.pool.close().await;
        assert_eq!(
            snapshot.my_role_ids,
            Vec::<Snowflake>::new(),
            "a deleted role leaves its members"
        );
    }

    #[tokio::test]
    async fn roles_grant_their_permissions_to_members_who_hold_them() {
        let fixture = fixture().await;
        let guild = guild_with_member(&fixture).await;
        let channel_managers = fixture
            .guilds
            .create_role(
                fixture.owner,
                guild.id,
                role_request("Channel managers", Permissions::MANAGE_CHANNELS),
                NOW,
            )
            .await
            .unwrap();
        let create_channel = || {
            fixture
                .guilds
                .create_channel(fixture.other, guild.id, text_channel("news", None), NOW)
        };
        assert!(matches!(
            create_channel().await,
            Err(GuildError::MissingPermissions)
        ));
        fixture
            .guilds
            .add_member_role(fixture.owner, guild.id, fixture.other, channel_managers.id)
            .await
            .unwrap();
        create_channel().await.unwrap();
        let members = fixture
            .guilds
            .list_members(fixture.owner, guild.id)
            .await
            .unwrap();
        let other = members
            .iter()
            .find(|member| member.user.id == fixture.other)
            .unwrap();
        assert_eq!(other.role_ids, vec![channel_managers.id]);

        fixture
            .guilds
            .remove_member_role(fixture.owner, guild.id, fixture.other, channel_managers.id)
            .await
            .unwrap();
        let after_removal = create_channel().await;
        let removing_again = fixture
            .guilds
            .remove_member_role(fixture.owner, guild.id, fixture.other, channel_managers.id)
            .await;
        fixture.pool.close().await;
        assert!(matches!(after_removal, Err(GuildError::MissingPermissions)));
        assert!(matches!(removing_again, Err(GuildError::NotFound)));
    }

    /// Roles of `guild_with_member`'s guild, `other` holding `moderator` (`MANAGE_ROLES` and
    /// `KICK_MEMBERS`) between `helper` below and `admin` above; `third` is a plain member.
    struct ModeratedGuild {
        guild: Guild,
        helper: Role,
        moderator: Role,
        admin: Role,
    }

    async fn moderated_guild(fixture: &Fixture) -> ModeratedGuild {
        let guild = guild_with_member(fixture).await;
        join(fixture, guild.id, fixture.third).await;
        let create = |name: &'static str, permissions| {
            fixture.guilds.create_role(
                fixture.owner,
                guild.id,
                role_request(name, permissions),
                NOW,
            )
        };
        // Each new role starts at position 1, so the last one created ends up lowest.
        let admin = create("Admin", Permissions::empty()).await.unwrap();
        let moderator = create(
            "Moderator",
            Permissions::MANAGE_ROLES | Permissions::KICK_MEMBERS,
        )
        .await
        .unwrap();
        let helper = create("Helper", Permissions::empty()).await.unwrap();
        let snapshot = fixture
            .guilds
            .get_guild(fixture.owner, guild.id)
            .await
            .unwrap();
        assert_eq!(
            role_names_by_position(&snapshot),
            vec![
                ("@everyone", 0),
                ("Helper", 1),
                ("Moderator", 2),
                ("Admin", 3)
            ]
        );
        fixture
            .guilds
            .add_member_role(fixture.owner, guild.id, fixture.other, moderator.id)
            .await
            .unwrap();
        ModeratedGuild {
            guild,
            helper,
            moderator,
            admin,
        }
    }

    fn assert_all_refused<T: std::fmt::Debug>(results: Vec<Result<T, GuildError>>) {
        for refused in results {
            assert!(
                matches!(refused, Err(GuildError::MissingPermissions)),
                "{refused:?}"
            );
        }
    }

    #[tokio::test]
    async fn managers_act_only_on_roles_below_their_highest() {
        let fixture = fixture().await;
        let ModeratedGuild {
            guild,
            helper,
            moderator,
            admin,
        } = moderated_guild(&fixture).await;
        let (guilds, other) = (&fixture.guilds, fixture.other);
        let kick = || permissions_update(Permissions::KICK_MEMBERS);

        guilds
            .add_member_role(other, guild.id, fixture.third, helper.id)
            .await
            .unwrap();
        guilds
            .update_role(other, guild.id, helper.id, kick())
            .await
            .unwrap();
        let higher_role = guilds.update_role(other, guild.id, admin.id, kick()).await;
        let give_higher_role = guilds
            .add_member_role(other, guild.id, fixture.third, admin.id)
            .await;
        let own_role = guilds
            .update_role(other, guild.id, moderator.id, kick())
            .await;
        let delete_own_role = guilds.delete_role(other, guild.id, moderator.id).await;
        let without_manage_roles = guilds
            .add_member_role(fixture.third, guild.id, fixture.third, helper.id)
            .await;
        fixture.pool.close().await;

        assert_all_refused(vec![higher_role.map(|_| ()), own_role.map(|_| ())]);
        assert_all_refused(vec![
            give_higher_role,
            delete_own_role,
            without_manage_roles,
        ]);
    }

    #[tokio::test]
    async fn managers_grant_only_permissions_they_hold() {
        let fixture = fixture().await;
        let ModeratedGuild { guild, helper, .. } = moderated_guild(&fixture).await;
        let (guilds, other) = (&fixture.guilds, fixture.other);

        let grant_unheld = guilds
            .update_role(
                other,
                guild.id,
                helper.id,
                permissions_update(Permissions::BAN_MEMBERS),
            )
            .await;
        let create_with_unheld = guilds
            .create_role(
                other,
                guild.id,
                role_request("Banners", Permissions::BAN_MEMBERS),
                NOW,
            )
            .await;
        let created_below = guilds
            .create_role(
                other,
                guild.id,
                role_request("Kickers", Permissions::KICK_MEMBERS),
                NOW,
            )
            .await
            .unwrap();
        let everyone_edit = guilds
            .update_role(
                other,
                guild.id,
                guild.id,
                permissions_update(Permissions::DEFAULT_EVERYONE | Permissions::KICK_MEMBERS),
            )
            .await;
        fixture.pool.close().await;

        assert_eq!(created_below.position, 1);
        assert!(everyone_edit.is_ok(), "{everyone_edit:?}");
        assert_all_refused(vec![grant_unheld, create_with_unheld]);
    }

    #[tokio::test]
    async fn managers_cannot_give_take_or_delete_lower_roles_holding_permissions_they_lack() {
        let fixture = fixture().await;
        let ModeratedGuild { guild, .. } = moderated_guild(&fixture).await;
        let (guilds, other) = (&fixture.guilds, fixture.other);
        // Created after Moderator, so it starts below it (position 1).
        let administrators = guilds
            .create_role(
                fixture.owner,
                guild.id,
                role_request("Administrators", Permissions::ADMINISTRATOR),
                NOW,
            )
            .await
            .unwrap();
        guilds
            .add_member_role(fixture.owner, guild.id, fixture.third, administrators.id)
            .await
            .unwrap();

        let give_to_self = guilds
            .add_member_role(other, guild.id, other, administrators.id)
            .await;
        let take_away = guilds
            .remove_member_role(other, guild.id, fixture.third, administrators.id)
            .await;
        let delete = guilds.delete_role(other, guild.id, administrators.id).await;
        let rename = guilds
            .update_role(
                other,
                guild.id,
                administrators.id,
                UpdateRoleRequest {
                    name: Some("Mine now".into()),
                    ..UpdateRoleRequest::default()
                },
            )
            .await;
        let still_moderator_only = guilds.get_guild(other, guild.id).await.unwrap();
        fixture.pool.close().await;

        assert_eq!(administrators.position, 1);
        assert_all_refused(vec![give_to_self, take_away, delete]);
        assert_all_refused(vec![rename]);
        assert!(
            !still_moderator_only
                .my_permissions
                .contains(Permissions::ADMINISTRATOR)
        );
    }

    fn order_of(roles: &[&Role]) -> ReorderRolesRequest {
        ReorderRolesRequest {
            role_ids: roles.iter().map(|role| role.id).collect(),
        }
    }

    #[tokio::test]
    async fn roles_are_reordered_within_the_hierarchy() {
        let fixture = fixture().await;
        let ModeratedGuild {
            guild,
            helper,
            moderator,
            admin,
        } = moderated_guild(&fixture).await;
        let (guilds, other) = (&fixture.guilds, fixture.other);
        let kickers = guilds
            .create_role(
                other,
                guild.id,
                role_request("Kickers", Permissions::KICK_MEMBERS),
                NOW,
            )
            .await
            .unwrap();
        // Now: Kickers 1, Helper 2, Moderator 3, Admin 4.
        let moved_below = guilds
            .reorder_roles(
                other,
                guild.id,
                order_of(&[&helper, &kickers, &moderator, &admin]),
            )
            .await
            .unwrap();
        let own_role_moved = guilds
            .reorder_roles(
                other,
                guild.id,
                order_of(&[&helper, &moderator, &kickers, &admin]),
            )
            .await;
        let above_moved = guilds
            .reorder_roles(
                other,
                guild.id,
                order_of(&[&helper, &kickers, &admin, &moderator]),
            )
            .await;
        let incomplete = guilds
            .reorder_roles(other, guild.id, order_of(&[&helper, &kickers, &moderator]))
            .await;
        let with_everyone = guilds
            .reorder_roles(
                fixture.owner,
                guild.id,
                ReorderRolesRequest {
                    role_ids: vec![guild.id, helper.id, kickers.id, moderator.id, admin.id],
                },
            )
            .await;
        let by_owner = guilds
            .reorder_roles(
                fixture.owner,
                guild.id,
                order_of(&[&moderator, &helper, &kickers, &admin]),
            )
            .await
            .unwrap();
        let by_plain_member = guilds
            .reorder_roles(
                fixture.third,
                guild.id,
                order_of(&[&moderator, &helper, &kickers, &admin]),
            )
            .await;
        guilds.leave_guild(fixture.third, guild.id).await.unwrap();
        let by_non_member = guilds
            .reorder_roles(
                fixture.third,
                guild.id,
                order_of(&[&moderator, &helper, &kickers, &admin]),
            )
            .await;
        fixture.pool.close().await;

        let names = |roles: &[Role]| {
            roles
                .iter()
                .map(|role| role.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(&moved_below),
            ["@everyone", "Helper", "Kickers", "Moderator", "Admin"]
        );
        assert_eq!(
            names(&by_owner),
            ["@everyone", "Moderator", "Helper", "Kickers", "Admin"]
        );
        assert_all_refused(vec![own_role_moved, above_moved, by_plain_member]);
        assert!(matches!(by_non_member, Err(GuildError::NotFound)));
        for invalid in [incomplete, with_everyone] {
            assert!(
                matches!(
                    invalid,
                    Err(GuildError::InvalidInput(GuildRuleViolation::RoleOrder))
                ),
                "{invalid:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_reordering_based_on_an_outdated_order_writes_nothing() {
        let fixture = fixture().await;
        let ModeratedGuild {
            guild,
            helper,
            moderator,
            admin,
        } = moderated_guild(&fixture).await;
        let seen = [helper.id, moderator.id, admin.id];
        // Someone else reordered after the check read `seen`.
        fixture
            .guilds
            .reorder_roles(
                fixture.owner,
                guild.id,
                order_of(&[&moderator, &helper, &admin]),
            )
            .await
            .unwrap();
        let audit = fixture
            .guilds
            .audit_entry(
                guild.id,
                fixture.owner,
                AuditAction::RolesReorder,
                None,
                None,
            )
            .unwrap();
        let outdated = guild_storage::reorder_roles(
            &fixture.pool,
            guild.id,
            &seen,
            &[admin.id, helper.id, moderator.id],
            &audit,
        )
        .await
        .unwrap();
        let roles = guild_storage::list_roles(&fixture.pool, guild.id)
            .await
            .unwrap();
        let audit_log = audit_log_storage::list_audit_entries(
            &fixture.pool,
            guild.id,
            None,
            100,
            &ChannelEntryFilter::All,
        )
        .await
        .unwrap();
        fixture.pool.close().await;
        assert_eq!(outdated, None);
        assert!(
            audit_log.iter().all(|entry| entry.id != audit.id),
            "nothing written, nothing recorded"
        );
        assert_eq!(
            roles.iter().map(|role| role.id).collect::<Vec<_>>(),
            [guild.id, moderator.id, helper.id, admin.id]
        );
    }

    fn overwrite_request(
        kind: OverwriteKind,
        allow: Permissions,
        deny: Permissions,
    ) -> SetPermissionOverwriteRequest {
        SetPermissionOverwriteRequest { kind, allow, deny }
    }

    #[tokio::test]
    async fn overwrites_are_set_only_within_the_hierarchy_and_held_permissions() {
        let fixture = fixture().await;
        let ModeratedGuild {
            guild,
            helper,
            admin,
            ..
        } = moderated_guild(&fixture).await;
        let (guilds, other) = (&fixture.guilds, fixture.other);
        let general = guild.channels[1].id;
        let role = |allow, deny| overwrite_request(OverwriteKind::Role, allow, deny);
        // Moderator holds SEND_MESSAGES (from @everyone) and MANAGE_ROLES, not MANAGE_MESSAGES.
        let held = Permissions::SEND_MESSAGES;

        let updated = guilds
            .set_permission_overwrite(other, general, helper.id, role(held, Permissions::empty()))
            .await
            .unwrap();
        guilds
            .set_permission_overwrite(
                other,
                general,
                fixture.third,
                overwrite_request(
                    OverwriteKind::Member,
                    Permissions::empty(),
                    Permissions::SEND_MESSAGES,
                ),
            )
            .await
            .unwrap();
        let unheld = guilds
            .set_permission_overwrite(
                other,
                general,
                helper.id,
                role(Permissions::MANAGE_MESSAGES, Permissions::empty()),
            )
            .await;
        let above = guilds
            .set_permission_overwrite(other, general, admin.id, role(held, Permissions::empty()))
            .await;
        let without_manage_roles = guilds
            .set_permission_overwrite(
                fixture.third,
                general,
                helper.id,
                role(Permissions::empty(), Permissions::empty()),
            )
            .await;
        let both = guilds
            .set_permission_overwrite(other, general, helper.id, role(held, held))
            .await;
        let not_a_member = guilds
            .set_permission_overwrite(
                other,
                general,
                Snowflake::from_raw(424_242),
                overwrite_request(OverwriteKind::Member, Permissions::empty(), held),
            )
            .await;
        let unknown_overwrite = guilds
            .delete_permission_overwrite(other, general, admin.id)
            .await;
        guilds
            .delete_role(fixture.owner, guild.id, helper.id)
            .await
            .unwrap();
        let after_role_deletion = guilds.get_guild(fixture.owner, guild.id).await.unwrap();
        let member_muted = guilds
            .channel_and_member_permissions(fixture.third, general)
            .await
            .unwrap()
            .1;
        fixture.pool.close().await;

        assert_eq!(updated.permission_overwrites.len(), 1);
        assert!(!member_muted.contains(Permissions::SEND_MESSAGES));
        assert_all_refused(vec![unheld, above, without_manage_roles]);
        assert!(
            matches!(
                both,
                Err(GuildError::InvalidInput(
                    GuildRuleViolation::OverwriteAllowsAndDeniesOneBit
                ))
            ),
            "{both:?}"
        );
        assert!(matches!(not_a_member, Err(GuildError::NotFound)));
        assert!(matches!(unknown_overwrite, Err(GuildError::NotFound)));
        let general_after = after_role_deletion
            .channels
            .iter()
            .find(|channel| channel.id == general)
            .unwrap();
        assert!(
            general_after
                .permission_overwrites
                .iter()
                .all(|overwrite| overwrite.id != helper.id),
            "deleting a role drops its overwrites"
        );
    }

    #[tokio::test]
    async fn a_role_allow_reveals_a_channel_hidden_from_everyone() {
        let fixture = fixture().await;
        let ModeratedGuild { guild, helper, .. } = moderated_guild(&fixture).await;
        let guilds = &fixture.guilds;
        let general = guild.channels[1].id;
        let view = Permissions::VIEW_CHANNEL;
        guilds
            .set_permission_overwrite(
                fixture.owner,
                general,
                guild.id,
                overwrite_request(OverwriteKind::Role, Permissions::empty(), view),
            )
            .await
            .unwrap();
        guilds
            .set_permission_overwrite(
                fixture.owner,
                general,
                helper.id,
                overwrite_request(OverwriteKind::Role, view, Permissions::empty()),
            )
            .await
            .unwrap();
        let hidden = guilds
            .channel_and_member_permissions(fixture.third, general)
            .await;
        guilds
            .add_member_role(fixture.owner, guild.id, fixture.third, helper.id)
            .await
            .unwrap();
        let revealed = guilds
            .channel_and_member_permissions(fixture.third, general)
            .await;
        let snapshot = guilds.get_guild(fixture.third, guild.id).await.unwrap();
        let owner_creates_in_category = guilds
            .create_channel(
                fixture.owner,
                guild.id,
                text_channel("child", Some(guild.channels[0].id)),
                NOW,
            )
            .await;
        fixture.pool.close().await;

        assert!(matches!(hidden, Err(GuildError::NotFound)));
        assert!(revealed.is_ok());
        assert!(
            snapshot
                .channels
                .iter()
                .any(|channel| channel.id == general)
        );
        assert!(owner_creates_in_category.is_ok());
    }

    /// A live session for the owner and for `other`, subscribed to `guild_id`, with nothing
    /// queued yet (presence updates of the sessions opening included).
    fn owner_and_other_sessions(
        fixture: &Fixture,
        guild_id: Snowflake,
    ) -> [tokio::sync::mpsc::Receiver<crate::events::SharedEvent>; 2] {
        let mut sessions =
            [(fixture.owner, [1; 32]), (fixture.other, [2; 32])].map(|(user_id, token)| {
                let (key, events) = fixture.events.register_session(user_id, token, 4).unwrap();
                fixture.events.activate_session(key, &[guild_id]);
                events
            });
        for events in &mut sessions {
            event_types(events);
        }
        sessions
    }

    fn event_types(
        events: &mut tokio::sync::mpsc::Receiver<crate::events::SharedEvent>,
    ) -> Vec<String> {
        let mut types = Vec::new();
        while let Ok(event) = events.try_recv() {
            let event: serde_json::Value = serde_json::from_str(&event.payload_json).unwrap();
            types.push(event["t"].as_str().unwrap().to_owned());
        }
        types
    }

    fn hide_from_everyone() -> SetPermissionOverwriteRequest {
        overwrite_request(
            OverwriteKind::Role,
            Permissions::empty(),
            Permissions::VIEW_CHANNEL,
        )
    }

    #[tokio::test]
    async fn only_members_whose_view_changes_get_a_snapshot() {
        let fixture = fixture().await;
        let guild = guild_with_member(&fixture).await;
        let general = guild.channels[1].id;
        let [mut owner_events, mut other_events] = owner_and_other_sessions(&fixture, guild.id);
        let guilds = &fixture.guilds;

        guilds
            .set_permission_overwrite(fixture.owner, general, guild.id, hide_from_everyone())
            .await
            .unwrap();
        let after_hiding = (
            event_types(&mut owner_events),
            event_types(&mut other_events),
        );
        guilds
            .set_permission_overwrite(fixture.owner, general, guild.id, hide_from_everyone())
            .await
            .unwrap();
        let after_identical = (
            event_types(&mut owner_events),
            event_types(&mut other_events),
        );
        let role = guilds
            .create_role(
                fixture.owner,
                guild.id,
                role_request("Viewers", Permissions::empty()),
                NOW,
            )
            .await
            .unwrap();
        guilds
            .set_permission_overwrite(
                fixture.owner,
                general,
                role.id,
                overwrite_request(
                    OverwriteKind::Role,
                    Permissions::VIEW_CHANNEL,
                    Permissions::empty(),
                ),
            )
            .await
            .unwrap();
        event_types(&mut owner_events);
        event_types(&mut other_events);
        guilds
            .add_member_role(fixture.owner, guild.id, fixture.other, role.id)
            .await
            .unwrap();
        let after_role_given = (
            event_types(&mut owner_events),
            event_types(&mut other_events),
        );
        fixture.pool.close().await;

        let types = |names: &[&str]| {
            names
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            after_hiding,
            (types(&["CHANNEL_UPDATE"]), types(&["GUILD_CREATE"]))
        );
        assert_eq!(
            after_identical,
            (Vec::new(), Vec::new()),
            "nothing changed, nothing sent"
        );
        assert_eq!(
            after_role_given,
            (
                types(&["GUILD_MEMBER_UPDATE"]),
                types(&["GUILD_MEMBER_UPDATE", "GUILD_CREATE"])
            ),
            "only the member whose roles changed gets a snapshot"
        );
    }

    #[tokio::test]
    async fn deleting_a_hidden_channel_tells_only_its_viewers() {
        let fixture = fixture().await;
        let guild = guild_with_member(&fixture).await;
        let staff = fixture
            .guilds
            .create_channel(fixture.owner, guild.id, text_channel("staff", None), NOW)
            .await
            .unwrap();
        fixture
            .guilds
            .set_permission_overwrite(fixture.owner, staff.id, guild.id, hide_from_everyone())
            .await
            .unwrap();
        let [mut owner_events, mut other_events] = owner_and_other_sessions(&fixture, guild.id);
        fixture
            .guilds
            .delete_channel(fixture.owner, staff.id)
            .await
            .unwrap();
        fixture.pool.close().await;
        assert_eq!(event_types(&mut owner_events), ["CHANNEL_DELETE"]);
        assert_eq!(event_types(&mut other_events), Vec::<String>::new());
    }

    #[tokio::test]
    async fn manage_roles_in_a_channel_decides_who_edits_its_overwrites() {
        let fixture = fixture().await;
        let guild = guild_with_member(&fixture).await;
        let general = guild.channels[1].id;
        let guilds = &fixture.guilds;
        let send = Permissions::SEND_MESSAGES;
        let mute_owner_overwrite =
            || overwrite_request(OverwriteKind::Member, Permissions::empty(), send);
        let before = guilds
            .set_permission_overwrite(
                fixture.other,
                general,
                fixture.other,
                mute_owner_overwrite(),
            )
            .await;
        guilds
            .set_permission_overwrite(
                fixture.owner,
                general,
                fixture.other,
                overwrite_request(
                    OverwriteKind::Member,
                    Permissions::MANAGE_ROLES,
                    Permissions::empty(),
                ),
            )
            .await
            .unwrap();
        let with_channel_manage_roles = guilds
            .set_permission_overwrite(
                fixture.other,
                general,
                guild.id,
                overwrite_request(OverwriteKind::Role, Permissions::empty(), send),
            )
            .await;
        let guild_wide = guilds
            .create_role(
                fixture.other,
                guild.id,
                role_request("Nope", Permissions::empty()),
                NOW,
            )
            .await;
        let out_of_scope = guilds
            .set_permission_overwrite(
                fixture.owner,
                general,
                guild.id,
                overwrite_request(
                    OverwriteKind::Role,
                    Permissions::KICK_MEMBERS,
                    Permissions::empty(),
                ),
            )
            .await;
        fixture.pool.close().await;
        assert!(
            matches!(before, Err(GuildError::MissingPermissions)),
            "{before:?}"
        );
        assert!(
            with_channel_manage_roles.is_ok(),
            "{with_channel_manage_roles:?}"
        );
        assert!(
            matches!(guild_wide, Err(GuildError::MissingPermissions)),
            "a channel overwrite grants nothing guild-wide"
        );
        assert!(matches!(
            out_of_scope,
            Err(GuildError::InvalidInput(
                GuildRuleViolation::OverwriteOutsideChannelScope
            ))
        ));
    }

    #[tokio::test]
    async fn overwrites_of_departed_members_can_be_deleted_only_with_their_permissions() {
        let fixture = fixture().await;
        let ModeratedGuild { guild, .. } = moderated_guild(&fixture).await;
        let general = guild.channels[1].id;
        let guilds = &fixture.guilds;
        guilds
            .set_permission_overwrite(
                fixture.owner,
                general,
                fixture.third,
                overwrite_request(
                    OverwriteKind::Member,
                    Permissions::MANAGE_MESSAGES,
                    Permissions::empty(),
                ),
            )
            .await
            .unwrap();
        guilds.leave_guild(fixture.third, guild.id).await.unwrap();
        let without_the_permission = guilds
            .delete_permission_overwrite(fixture.other, general, fixture.third)
            .await;
        let by_owner = guilds
            .delete_permission_overwrite(fixture.owner, general, fixture.third)
            .await
            .unwrap();
        fixture.pool.close().await;
        assert!(
            matches!(without_the_permission, Err(GuildError::MissingPermissions)),
            "{without_the_permission:?}"
        );
        assert_eq!(by_owner.permission_overwrites, Vec::new());
    }

    #[tokio::test]
    async fn a_channel_holds_at_most_the_overwrite_quota() {
        let fixture = fixture().await;
        let guild = guild_with_member(&fixture).await;
        let general = guild.channels[1].id;
        let send = Permissions::SEND_MESSAGES;
        let mut role_ids = Vec::new();
        for index in 0..MAXIMUM_OVERWRITES_PER_CHANNEL {
            let role = fixture
                .guilds
                .create_role(
                    fixture.owner,
                    guild.id,
                    role_request(&format!("r{index}"), Permissions::empty()),
                    NOW,
                )
                .await
                .unwrap();
            fixture
                .guilds
                .set_permission_overwrite(
                    fixture.owner,
                    general,
                    role.id,
                    overwrite_request(OverwriteKind::Role, send, Permissions::empty()),
                )
                .await
                .unwrap();
            role_ids.push(role.id);
        }
        let replace_at_quota = fixture
            .guilds
            .set_permission_overwrite(
                fixture.owner,
                general,
                role_ids[0],
                overwrite_request(OverwriteKind::Role, Permissions::empty(), send),
            )
            .await;
        let one_too_many = fixture
            .guilds
            .set_permission_overwrite(fixture.owner, general, guild.id, hide_from_everyone())
            .await;
        fixture.pool.close().await;
        assert!(replace_at_quota.is_ok(), "{replace_at_quota:?}");
        assert!(matches!(one_too_many, Err(GuildError::LimitReached(_))));
    }

    #[tokio::test]
    async fn a_channel_cannot_be_created_in_a_category_hidden_from_its_creator() {
        let fixture = fixture().await;
        let guild = guild_with_member(&fixture).await;
        let category = guild.channels[0].id;
        let guilds = &fixture.guilds;
        let managers = guilds
            .create_role(
                fixture.owner,
                guild.id,
                role_request("Managers", Permissions::MANAGE_CHANNELS),
                NOW,
            )
            .await
            .unwrap();
        guilds
            .add_member_role(fixture.owner, guild.id, fixture.other, managers.id)
            .await
            .unwrap();
        guilds
            .set_permission_overwrite(fixture.owner, category, guild.id, hide_from_everyone())
            .await
            .unwrap();
        let in_hidden_category = guilds
            .create_channel(
                fixture.other,
                guild.id,
                text_channel("x", Some(category)),
                NOW,
            )
            .await;
        fixture.pool.close().await;
        assert!(matches!(
            in_hidden_category,
            Err(GuildError::InvalidInput(GuildRuleViolation::Parent))
        ));
    }

    #[tokio::test]
    async fn moderators_kick_only_members_ranked_below_them() {
        let fixture = fixture().await;
        let ModeratedGuild { guild, admin, .. } = moderated_guild(&fixture).await;
        let guilds = &fixture.guilds;
        let other = fixture.other;
        let kick_owner = guilds.kick_member(other, guild.id, fixture.owner).await;
        let kick_self = guilds.kick_member(other, guild.id, other).await;
        let kick_outsider = guilds
            .kick_member(other, guild.id, Snowflake::from_raw(424_242))
            .await;
        let without_kick = guilds.kick_member(fixture.third, guild.id, other).await;
        guilds
            .kick_member(other, guild.id, fixture.third)
            .await
            .unwrap();
        let kicked_snapshot = guilds.get_guild(fixture.third, guild.id).await;
        // A kicked member may come back with an invite.
        join(&fixture, guild.id, fixture.third).await;
        guilds
            .add_member_role(fixture.owner, guild.id, fixture.third, admin.id)
            .await
            .unwrap();
        let kick_higher = guilds.kick_member(other, guild.id, fixture.third).await;
        fixture.pool.close().await;

        assert!(matches!(kicked_snapshot, Err(GuildError::NotFound)));
        assert_all_refused(vec![kick_owner, without_kick, kick_higher]);
        assert!(matches!(
            kick_self,
            Err(GuildError::InvalidInput(
                GuildRuleViolation::ModerationTarget
            ))
        ));
        assert!(matches!(kick_outsider, Err(GuildError::NotFound)));
    }

    fn ban_request(reason: &str) -> CreateBanRequest {
        CreateBanRequest {
            reason: reason.into(),
        }
    }

    #[tokio::test]
    async fn banned_users_cannot_join_until_unbanned() {
        let fixture = fixture().await;
        let ModeratedGuild { guild, .. } = moderated_guild(&fixture).await;
        let guilds = &fixture.guilds;
        let owner = fixture.owner;
        let without_ban_members = guilds
            .ban_member(fixture.other, guild.id, fixture.third, ban_request(""), NOW)
            .await;
        guilds
            .ban_member(owner, guild.id, fixture.third, ban_request("  spam  "), NOW)
            .await
            .unwrap();
        let banned_snapshot = guilds.get_guild(fixture.third, guild.id).await;
        let invite = guilds
            .create_invite(owner, guild.id, CreateInviteRequest::default(), NOW)
            .await
            .unwrap();
        let rejoin_while_banned = guilds
            .join_with_invite(fixture.third, &invite.code, NOW)
            .await;
        let bans = guilds.list_bans(owner, guild.id).await.unwrap();
        let bans_without_permission = guilds.list_bans(fixture.other, guild.id).await;
        let unknown_user = guilds
            .ban_member(
                owner,
                guild.id,
                Snowflake::from_raw(424_242),
                ban_request(""),
                NOW,
            )
            .await;
        let too_long = guilds
            .ban_member(
                owner,
                guild.id,
                fixture.other,
                ban_request(&"x".repeat(MAXIMUM_BAN_REASON_LENGTH + 1)),
                NOW,
            )
            .await;
        guilds
            .unban_member(owner, guild.id, fixture.third)
            .await
            .unwrap();
        let unban_again = guilds.unban_member(owner, guild.id, fixture.third).await;
        let rejoin_after_unban = guilds
            .join_with_invite(fixture.third, &invite.code, NOW)
            .await;
        fixture.pool.close().await;

        assert!(
            matches!(banned_snapshot, Err(GuildError::NotFound)),
            "a ban removes the member"
        );
        assert!(matches!(rejoin_while_banned, Err(GuildError::Banned)));
        assert_eq!(bans.len(), 1);
        assert_eq!(
            (bans[0].user.id, bans[0].reason.as_str()),
            (fixture.third, "spam")
        );
        assert_all_refused(vec![without_ban_members]);
        assert!(matches!(
            bans_without_permission,
            Err(GuildError::MissingPermissions)
        ));
        assert!(matches!(unknown_user, Err(GuildError::NotFound)));
        assert!(matches!(
            too_long,
            Err(GuildError::InvalidInput(GuildRuleViolation::BanReason))
        ));
        assert!(matches!(unban_again, Err(GuildError::NotFound)));
        assert!(rejoin_after_unban.is_ok(), "{rejoin_after_unban:?}");
    }

    #[tokio::test]
    async fn moderators_ban_only_members_ranked_below_them() {
        let fixture = fixture().await;
        let guild = guild_with_member(&fixture).await;
        join(&fixture, guild.id, fixture.third).await;
        let guilds = &fixture.guilds;
        let banners = guilds
            .create_role(
                fixture.owner,
                guild.id,
                role_request("Banners", Permissions::BAN_MEMBERS),
                NOW,
            )
            .await
            .unwrap();
        for member in [fixture.other, fixture.third] {
            guilds
                .add_member_role(fixture.owner, guild.id, member, banners.id)
                .await
                .unwrap();
        }
        let other = fixture.other;
        let equal_rank = guilds
            .ban_member(other, guild.id, fixture.third, ban_request(""), NOW)
            .await;
        let owner = guilds
            .ban_member(other, guild.id, fixture.owner, ban_request(""), NOW)
            .await;
        let self_ban = guilds
            .ban_member(other, guild.id, other, ban_request(""), NOW)
            .await;
        fixture.pool.close().await;
        assert_all_refused(vec![equal_rank, owner]);
        assert!(matches!(
            self_ban,
            Err(GuildError::InvalidInput(
                GuildRuleViolation::ModerationTarget
            ))
        ));
    }

    #[tokio::test]
    async fn users_can_be_banned_before_they_join() {
        let fixture = fixture().await;
        let guild = fixture
            .guilds
            .create_guild(fixture.owner, create_request("Club"), NOW)
            .await
            .unwrap();
        fixture
            .guilds
            .ban_member(fixture.owner, guild.id, fixture.other, ban_request(""), NOW)
            .await
            .unwrap();
        let invite = fixture
            .guilds
            .create_invite(fixture.owner, guild.id, CreateInviteRequest::default(), NOW)
            .await
            .unwrap();
        let joined = fixture
            .guilds
            .join_with_invite(fixture.other, &invite.code, NOW)
            .await;
        fixture.pool.close().await;
        assert!(matches!(joined, Err(GuildError::Banned)));
    }

    fn actions(entries: &[AuditLogEntry]) -> Vec<AuditAction> {
        entries.iter().map(|entry| entry.action).collect()
    }

    #[tokio::test]
    async fn moderation_and_administration_are_recorded_in_the_audit_log() {
        let fixture = fixture().await;
        let guild = guild_with_member(&fixture).await;
        join(&fixture, guild.id, fixture.third).await;
        let guilds = &fixture.guilds;
        let owner = fixture.owner;
        let channel = guilds
            .create_channel(owner, guild.id, text_channel("news", None), NOW)
            .await
            .unwrap();
        let role = guilds
            .create_role(
                owner,
                guild.id,
                role_request("Mods", Permissions::empty()),
                NOW,
            )
            .await
            .unwrap();
        guilds
            .add_member_role(owner, guild.id, fixture.other, role.id)
            .await
            .unwrap();
        let without_permission = guilds
            .list_audit_log(fixture.other, guild.id, AuditLogQuery::default())
            .await;
        // Refused or unchanged: nothing recorded.
        let refused = guilds
            .create_role(
                fixture.other,
                guild.id,
                role_request("No", Permissions::empty()),
                NOW,
            )
            .await;
        guilds
            .add_member_role(owner, guild.id, fixture.other, role.id)
            .await
            .unwrap();
        guilds
            .set_permission_overwrite(
                owner,
                channel.id,
                role.id,
                overwrite_request(
                    OverwriteKind::Role,
                    Permissions::SEND_MESSAGES,
                    Permissions::empty(),
                ),
            )
            .await
            .unwrap();
        guilds
            .kick_member(owner, guild.id, fixture.third)
            .await
            .unwrap();
        guilds
            .ban_member(owner, guild.id, fixture.other, ban_request("spam"), NOW)
            .await
            .unwrap();
        let everything = guilds
            .list_audit_log(owner, guild.id, AuditLogQuery::default())
            .await
            .unwrap();
        fixture.pool.close().await;

        assert_all_refused(vec![refused.map(|_| ())]);
        assert_eq!(
            actions(&everything),
            [
                AuditAction::MemberBanAdd,
                AuditAction::MemberKick,
                AuditAction::OverwriteSet,
                AuditAction::MemberRoleAdd,
                AuditAction::RoleCreate,
                AuditAction::ChannelCreate,
            ],
            "newest first; joins and repeated grants are not moderation"
        );
        let ban = &everything[0];
        assert_eq!(
            (ban.actor_id, ban.target_id, ban.reason.as_str()),
            (Some(owner), Some(fixture.other), "spam")
        );
        let overwrite = &everything[2];
        assert_eq!(
            (overwrite.target_id, overwrite.context_id),
            (Some(role.id), Some(channel.id))
        );
        assert!(matches!(
            without_permission,
            Err(GuildError::MissingPermissions)
        ));
    }

    #[tokio::test]
    async fn the_audit_log_pages_back_from_the_newest_entry() {
        let fixture = fixture().await;
        let guild = guild_with_member(&fixture).await;
        let guilds = &fixture.guilds;
        for name in ["a", "b", "c"] {
            guilds
                .create_channel(fixture.owner, guild.id, text_channel(name, None), NOW)
                .await
                .unwrap();
        }
        let page = |before, limit| {
            guilds.list_audit_log(fixture.owner, guild.id, AuditLogQuery { before, limit })
        };
        let newest = page(None, Some(1)).await.unwrap();
        let older = page(Some(newest[0].id), Some(5)).await.unwrap();
        let beyond_every_id = page(Some(Snowflake::from_raw(u64::MAX)), None)
            .await
            .unwrap();
        let bad_limit = page(None, Some(0)).await;
        fixture.pool.close().await;

        assert_eq!(newest.len(), 1);
        assert_eq!(older.len(), 2);
        assert!(older.iter().all(|entry| entry.id < newest[0].id));
        assert_eq!(beyond_every_id.len(), 3);
        assert!(matches!(
            bad_limit,
            Err(GuildError::InvalidInput(GuildRuleViolation::AuditLogLimit))
        ));
    }

    async fn audit_actions(fixture: &Fixture, guild_id: Snowflake) -> Vec<AuditAction> {
        actions(
            &fixture
                .guilds
                .list_audit_log(fixture.owner, guild_id, AuditLogQuery::default())
                .await
                .unwrap(),
        )
    }

    /// `guild_with_member`'s guild where `first` (with a `SEND_MESSAGES` overwrite on `general`)
    /// is given to `other`, `second` exists, and `third` is banned.
    struct AuditedGuild {
        guild: Guild,
        general: Snowflake,
        first: Role,
        second: Role,
    }

    async fn audited_guild(fixture: &Fixture) -> AuditedGuild {
        let guild = guild_with_member(fixture).await;
        let owner = fixture.owner;
        let general = guild.channels[1].id;
        let send = Permissions::SEND_MESSAGES;
        let guilds = &fixture.guilds;
        let first = guilds
            .create_role(
                owner,
                guild.id,
                role_request("First", Permissions::empty()),
                NOW,
            )
            .await
            .unwrap();
        let second = guilds
            .create_role(
                owner,
                guild.id,
                role_request("Second", Permissions::empty()),
                NOW,
            )
            .await
            .unwrap();
        guilds
            .set_permission_overwrite(
                owner,
                general,
                first.id,
                overwrite_request(OverwriteKind::Role, send, Permissions::empty()),
            )
            .await
            .unwrap();
        guilds
            .add_member_role(owner, guild.id, fixture.other, first.id)
            .await
            .unwrap();
        guilds
            .ban_member(owner, guild.id, fixture.third, ban_request(""), NOW)
            .await
            .unwrap();
        AuditedGuild {
            guild,
            general,
            first,
            second,
        }
    }

    #[tokio::test]
    async fn unchanged_or_refused_actions_record_nothing() {
        let fixture = fixture().await;
        let AuditedGuild {
            guild,
            general,
            first,
            second,
        } = audited_guild(&fixture).await;
        let guilds = &fixture.guilds;
        let owner = fixture.owner;
        let before = audit_actions(&fixture, guild.id).await.len();

        // Nothing changes: nothing is recorded.
        let rename_to_same = UpdateChannelRequest {
            name: Some("general".into()),
            topic: Some(String::new()),
        };
        guilds
            .update_channel(owner, general, rename_to_same)
            .await
            .unwrap();
        guilds
            .update_role(
                owner,
                guild.id,
                first.id,
                UpdateRoleRequest {
                    name: Some("First".into()),
                    color: Some(0),
                    permissions: Some(Permissions::empty()),
                },
            )
            .await
            .unwrap();
        let not_held = guilds
            .remove_member_role(owner, guild.id, fixture.other, second.id)
            .await;
        let not_banned = guilds.unban_member(owner, guild.id, fixture.other).await;
        assert_eq!(audit_actions(&fixture, guild.id).await.len(), before);
        assert!(matches!(not_held, Err(GuildError::NotFound)));
        assert!(matches!(not_banned, Err(GuildError::NotFound)));
        fixture.pool.close().await;
    }

    #[tokio::test]
    async fn every_other_audited_action_is_recorded_with_its_changes() {
        let fixture = fixture().await;
        let AuditedGuild {
            guild,
            general,
            first,
            second,
        } = audited_guild(&fixture).await;
        let guilds = &fixture.guilds;
        let owner = fixture.owner;
        let send = Permissions::SEND_MESSAGES;
        guilds
            .update_channel(
                owner,
                general,
                UpdateChannelRequest {
                    name: Some("lobby".into()),
                    topic: None,
                },
            )
            .await
            .unwrap();
        guilds
            .update_role(owner, guild.id, first.id, permissions_update(send))
            .await
            .unwrap();
        guilds
            .reorder_roles(owner, guild.id, order_of(&[&first, &second]))
            .await
            .unwrap();
        guilds
            .remove_member_role(owner, guild.id, fixture.other, first.id)
            .await
            .unwrap();
        guilds
            .delete_permission_overwrite(owner, general, first.id)
            .await
            .unwrap();
        guilds
            .delete_role(owner, guild.id, second.id)
            .await
            .unwrap();
        guilds
            .unban_member(owner, guild.id, fixture.third)
            .await
            .unwrap();
        guilds.leave_guild(fixture.other, guild.id).await.unwrap();
        guilds.delete_channel(owner, general).await.unwrap();
        let entries = guilds
            .list_audit_log(owner, guild.id, AuditLogQuery::default())
            .await
            .unwrap();
        fixture.pool.close().await;

        assert_eq!(
            actions(&entries[..9]),
            [
                AuditAction::ChannelDelete,
                AuditAction::MemberBanRemove,
                AuditAction::RoleDelete,
                AuditAction::OverwriteDelete,
                AuditAction::MemberRoleRemove,
                AuditAction::RolesReorder,
                AuditAction::RoleUpdate,
                AuditAction::ChannelUpdate,
                AuditAction::MemberBanAdd,
            ],
            "leaving records nothing"
        );
        let change = |entry: &AuditLogEntry| {
            entry
                .changes
                .iter()
                .map(|change| {
                    (
                        change.key,
                        change.old_value.clone(),
                        change.new_value.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            change(&entries[7]),
            [(AuditChangeKey::Name, "general".into(), "lobby".into())]
        );
        assert_eq!(
            change(&entries[6]),
            [(
                AuditChangeKey::Permissions,
                "0".into(),
                send.bits().to_string()
            )]
        );
        assert_eq!(
            change(&entries[3]),
            [(AuditChangeKey::Allow, send.bits().to_string(), "0".into())]
        );
    }

    #[tokio::test]
    async fn readers_see_no_entries_about_channels_they_cannot_view() {
        let fixture = fixture().await;
        let guild = guild_with_member(&fixture).await;
        let guilds = &fixture.guilds;
        let owner = fixture.owner;
        let auditors = guilds
            .create_role(
                owner,
                guild.id,
                role_request("Auditors", Permissions::VIEW_AUDIT_LOG),
                NOW,
            )
            .await
            .unwrap();
        guilds
            .add_member_role(owner, guild.id, fixture.other, auditors.id)
            .await
            .unwrap();
        let staff = guilds
            .create_channel(owner, guild.id, text_channel("staff", None), NOW)
            .await
            .unwrap();
        guilds
            .set_permission_overwrite(owner, staff.id, guild.id, hide_from_everyone())
            .await
            .unwrap();
        let gone = guilds
            .create_channel(owner, guild.id, text_channel("gone", None), NOW)
            .await
            .unwrap();
        guilds.delete_channel(owner, gone.id).await.unwrap();
        let seen_by_auditor = guilds
            .list_audit_log(fixture.other, guild.id, AuditLogQuery::default())
            .await
            .unwrap();
        let seen_by_owner = guilds
            .list_audit_log(owner, guild.id, AuditLogQuery::default())
            .await
            .unwrap();
        fixture.pool.close().await;

        let about = |entries: &[AuditLogEntry], channel_id: Snowflake| {
            entries.iter().any(|entry| {
                entry.target_id == Some(channel_id) || entry.context_id == Some(channel_id)
            })
        };
        assert!(!about(&seen_by_auditor, staff.id), "hidden channel");
        assert!(!about(&seen_by_auditor, gone.id), "deleted channel");
        assert_eq!(
            actions(&seen_by_auditor),
            [AuditAction::MemberRoleAdd, AuditAction::RoleCreate]
        );
        assert!(about(&seen_by_owner, staff.id) && about(&seen_by_owner, gone.id));
    }

    #[tokio::test]
    async fn everyone_cannot_be_renamed_deleted_given_or_taken_away() {
        let fixture = fixture().await;
        let guild = guild_with_member(&fixture).await;
        let (guilds, owner) = (&fixture.guilds, fixture.owner);
        let rename = guilds
            .update_role(
                owner,
                guild.id,
                guild.id,
                UpdateRoleRequest {
                    name: Some("everyone".into()),
                    ..UpdateRoleRequest::default()
                },
            )
            .await;
        let delete = guilds.delete_role(owner, guild.id, guild.id).await;
        let give = guilds
            .add_member_role(owner, guild.id, fixture.other, guild.id)
            .await;
        let take = guilds
            .remove_member_role(owner, guild.id, fixture.other, guild.id)
            .await;
        fixture.pool.close().await;
        for refused in [rename.map(|_| ()), delete, give, take] {
            assert!(
                matches!(
                    refused,
                    Err(GuildError::InvalidInput(GuildRuleViolation::EveryoneRole))
                ),
                "{refused:?}"
            );
        }
    }

    #[tokio::test]
    async fn role_requests_are_validated_and_other_guilds_roles_are_not_found() {
        let fixture = fixture().await;
        let guild = guild_with_member(&fixture).await;
        let elsewhere = fixture
            .guilds
            .create_guild(fixture.other, create_request("Elsewhere"), NOW)
            .await
            .unwrap();
        let foreign_role = fixture
            .guilds
            .create_role(
                fixture.other,
                elsewhere.id,
                role_request("Theirs", Permissions::empty()),
                NOW,
            )
            .await
            .unwrap();
        let unknown_bits = fixture
            .guilds
            .create_role(
                fixture.owner,
                guild.id,
                role_request("x", Permissions::from_bits_retain(1 << 63)),
                NOW,
            )
            .await;
        let bad_color = fixture
            .guilds
            .create_role(
                fixture.owner,
                guild.id,
                CreateRoleRequest {
                    color: 0x100_0000,
                    ..role_request("x", Permissions::empty())
                },
                NOW,
            )
            .await;
        let foreign = fixture
            .guilds
            .add_member_role(fixture.owner, guild.id, fixture.other, foreign_role.id)
            .await;
        let non_member = fixture
            .guilds
            .create_role(
                fixture.third,
                guild.id,
                role_request("x", Permissions::empty()),
                NOW,
            )
            .await;
        let own = fixture
            .guilds
            .create_role(
                fixture.owner,
                guild.id,
                role_request("Mine", Permissions::empty()),
                NOW,
            )
            .await
            .unwrap();
        let outsider_target = fixture
            .guilds
            .add_member_role(fixture.owner, guild.id, fixture.third, own.id)
            .await;
        fixture.pool.close().await;
        assert!(matches!(
            unknown_bits,
            Err(GuildError::InvalidInput(
                GuildRuleViolation::UnknownPermissions
            ))
        ));
        assert!(matches!(
            bad_color,
            Err(GuildError::InvalidInput(GuildRuleViolation::RoleColor))
        ));
        for not_found in [foreign, non_member.map(|_| ()), outsider_target] {
            assert!(
                matches!(not_found, Err(GuildError::NotFound)),
                "{not_found:?}"
            );
        }
    }

    #[tokio::test]
    async fn leaving_drops_roles_and_a_guild_holds_at_most_the_role_quota() {
        let fixture = fixture().await;
        let guild = guild_with_member(&fixture).await;
        let role = fixture
            .guilds
            .create_role(
                fixture.owner,
                guild.id,
                role_request("r", Permissions::empty()),
                NOW,
            )
            .await
            .unwrap();
        fixture
            .guilds
            .add_member_role(fixture.owner, guild.id, fixture.other, role.id)
            .await
            .unwrap();
        fixture
            .guilds
            .leave_guild(fixture.other, guild.id)
            .await
            .unwrap();
        join(&fixture, guild.id, fixture.other).await;
        let rejoined = fixture
            .guilds
            .get_guild(fixture.other, guild.id)
            .await
            .unwrap();
        assert_eq!(rejoined.my_role_ids, Vec::<Snowflake>::new());

        for index in 2..MAXIMUM_ROLES_PER_GUILD {
            fixture
                .guilds
                .create_role(
                    fixture.owner,
                    guild.id,
                    role_request(&format!("r{index}"), Permissions::empty()),
                    NOW,
                )
                .await
                .unwrap();
        }
        let one_too_many = fixture
            .guilds
            .create_role(
                fixture.owner,
                guild.id,
                role_request("overflow", Permissions::empty()),
                NOW,
            )
            .await;
        fixture.pool.close().await;
        assert!(matches!(one_too_many, Err(GuildError::LimitReached(_))));
    }
}
