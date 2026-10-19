//! Guild, channel, invite and membership use cases. Every action checks permissions through
//! `permissions::guild_permissions` (ADR-0007).

use std::sync::Arc;

use mellow_protocol::{
    Channel, ChannelKind, CreateChannelRequest, CreateGuildRequest, CreateInviteRequest,
    GatewayEvent, Guild, GuildSummary, Invite, Member, Permissions, PresenceStatus, PublicUser,
    Snowflake, UpdateChannelRequest,
};
use sqlx::SqlitePool;

use crate::domain::guild_rules::{
    GuildRuleViolation, MAXIMUM_ACTIVE_INVITES_PER_GUILD, MAXIMUM_CHANNELS_PER_GUILD,
    MAXIMUM_MEMBERS_LISTED, MAXIMUM_OWNED_GUILDS_PER_USER, generate_invite_code,
    normalize_category_name, normalize_guild_name, normalize_text_channel_name, normalize_topic,
    validate_invite_limits,
};
use crate::domain::permissions::guild_permissions;
use crate::domain::snowflake_generator::SnowflakeGenerator;
use crate::events::EventHub;
use crate::storage::guilds::{
    self as guild_storage, GuildStorageError, Membership, NewGuild, StoredChannel, StoredInvite,
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
    #[error("limit reached: {0}")]
    LimitReached(&'static str),
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
        },
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
    }
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

    /// The member's permissions, or `NotFound` for non-members.
    async fn membership_and_permissions(
        &self,
        guild_id: Snowflake,
        user_id: Snowflake,
    ) -> Result<(Membership, Permissions), GuildError> {
        let membership = guild_storage::find_membership(&self.pool, guild_id, user_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        let permissions = guild_permissions(
            user_id,
            membership.owner_id,
            membership.everyone_permissions,
        );
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
            },
            StoredChannel {
                id: self.ids.next_id(now_unix_milliseconds)?,
                guild_id,
                kind: ChannelKind::Text,
                name: DEFAULT_CHANNEL_NAME.to_owned(),
                topic: String::new(),
                parent_id: Some(category_id),
                position: 1,
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
        let channels = guild_storage::list_guild_channels(&self.pool, guild_id)
            .await?
            .into_iter()
            .map(channel_from_stored)
            .collect();
        Ok(Guild {
            id: guild_id,
            name: membership.guild_name,
            owner_id: membership.owner_id,
            my_permissions,
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
            let parent = guild_storage::find_guild_channel(&self.pool, parent_id).await?;
            let is_category_of_same_guild = parent.is_some_and(|parent| {
                parent.guild_id == guild_id && parent.kind == ChannelKind::Category
            });
            if request.kind == ChannelKind::Category || !is_category_of_same_guild {
                return Err(GuildRuleViolation::Parent.into());
            }
        }
        let channel = guild_storage::insert_channel_last(
            &self.pool,
            &StoredChannel {
                id: self.ids.next_id(now_unix_milliseconds)?,
                guild_id,
                kind: request.kind,
                name,
                topic,
                parent_id: request.parent_id,
                position: 0,
            },
            MAXIMUM_CHANNELS_PER_GUILD,
        )
        .await
        .map_err(limit_reached("a guild has at most 500 channels"))?;
        let channel = channel_from_stored(channel);
        self.events
            .publish_to_guild(guild_id, &GatewayEvent::ChannelCreate(channel.clone()));
        Ok(channel)
    }

    /// The guild channel and `user_id`'s permissions in its guild, or `NotFound` for non-members.
    pub(crate) async fn channel_and_member_permissions(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
    ) -> Result<(StoredChannel, Permissions), GuildError> {
        let channel = guild_storage::find_guild_channel(&self.pool, channel_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        let (_, permissions) = self
            .membership_and_permissions(channel.guild_id, user_id)
            .await?;
        Ok((channel, permissions))
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
        let updated = guild_storage::update_channel_fields(
            &self.pool,
            channel.id,
            name.as_deref(),
            topic.as_deref(),
        )
        .await?
        .ok_or(GuildError::NotFound)?;
        let updated = channel_from_stored(updated);
        self.events.publish_to_guild(
            updated.guild_id,
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
        guild_storage::delete_channel(&self.pool, channel.id).await?;
        self.events.publish_to_guild(
            channel.guild_id,
            &GatewayEvent::ChannelDelete {
                guild_id: channel.guild_id,
                id: channel.id,
            },
        );
        Ok(())
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
            guild_storage::join_with_invite(&self.pool, code, user_id, now_unix_milliseconds)
                .await?
                .ok_or(GuildError::InvalidInvite)?;
        if is_new_member {
            // Subscribed before the snapshot, so nothing between the two is missed.
            self.events.subscribe_user_to_guild(user_id, guild_id);
        }
        let guild = self.get_guild(user_id, guild_id).await?;
        if is_new_member {
            if let Some(member) = guild_storage::find_member(&self.pool, guild_id, user_id).await? {
                self.events.publish_to_guild(
                    guild_id,
                    &GatewayEvent::GuildMemberAdd {
                        guild_id,
                        member: member_from_stored(member),
                    },
                );
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

    /// Members only. shortcut: unpaginated; fine for M1 community sizes, paginate with the member list UI.
    pub(crate) async fn list_members(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
    ) -> Result<Vec<Member>, GuildError> {
        self.membership_and_permissions(guild_id, user_id).await?;
        Ok(
            guild_storage::list_members(&self.pool, guild_id, MAXIMUM_MEMBERS_LISTED)
                .await?
                .into_iter()
                .map(member_from_stored)
                .collect(),
        )
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
        guild_storage::delete_member(&self.pool, guild_id, user_id).await?;
        self.events.unsubscribe_user_from_guild(user_id, guild_id);
        self.events.publish_to_guild(
            guild_id,
            &GatewayEvent::GuildMemberRemove { guild_id, user_id },
        );
        self.events
            .publish_to_user(user_id, &GatewayEvent::GuildDelete { id: guild_id });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::RegisterRequest;

    use super::*;
    use crate::domain::account_rules::RegistrationMode;
    use crate::domain::accounts::Accounts;
    use crate::domain::accounts::tests::{NOW, migrated_pool, register_request};

    struct Fixture {
        _directory: tempfile::TempDir,
        pool: SqlitePool,
        guilds: Guilds,
        owner: Snowflake,
        other: Snowflake,
        third: Snowflake,
    }

    async fn fixture() -> Fixture {
        let (directory, pool) = migrated_pool().await;
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
            guilds: Guilds::new(pool.clone(), ids, Arc::default()),
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
}
