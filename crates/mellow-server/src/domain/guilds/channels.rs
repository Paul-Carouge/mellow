//! Channel use cases: creating, reading, editing, reordering and deleting channels.

use std::collections::{HashMap, HashSet};

use mellow_protocol::{
    AuditAction, AuditChange, AuditChangeKey, Channel, ChannelKind, CreateChannelRequest,
    GatewayEvent, Permissions, ReorderChannelsRequest, Snowflake, UpdateChannelRequest,
    UserLimitChange,
};

use super::audience::AffectedMembers;
use super::{GuildError, Guilds, channel_from_stored, limit_reached, text_change};
use crate::domain::guild_rules::{
    ChannelPlace, GuildRuleViolation, MAXIMUM_CHANNELS_PER_GUILD, merge_channel_order,
    normalize_channel_name, normalize_topic, validate_user_limit,
};
use crate::domain::permissions::{
    VoiceEntitlement, channel_permissions, may_use_soundboard, voice_entitlement,
};
use crate::domain::voice_rules::VoiceAccess;
use crate::storage::audit_log::NewAuditEntry;
use crate::storage::guild_channels::{self as channel_storage, ChannelFieldChanges, StoredChannel};
use crate::storage::guilds as guild_storage;

impl Guilds {
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
        let name = normalize_channel_name(&request.name)?;
        // Only text channels show a topic.
        let topic = if request.kind == ChannelKind::Text {
            normalize_topic(&request.topic)?
        } else {
            String::new()
        };
        let user_limit = validate_user_limit(request.kind, request.user_limit)?;
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
        let channel = channel_storage::insert_channel_last(
            &self.database,
            &StoredChannel {
                id: channel_id,
                guild_id,
                kind: request.kind,
                name,
                topic,
                parent_id: request.parent_id,
                position: 0,
                icon: None,
                permission_overwrites: Vec::new(),
                user_limit,
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
        let channel = channel_storage::find_guild_channel(&self.database, channel_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        let membership = guild_storage::find_membership(&self.database, channel.guild_id, user_id)
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

    /// The text channels of the guild where `user_id` may view and read history (overwrites
    /// applied), for search; `NotFound` for non-members.
    pub(crate) async fn channels_with_readable_history(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
    ) -> Result<Vec<StoredChannel>, GuildError> {
        let membership = guild_storage::find_membership(&self.database, guild_id, user_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        let readable = Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY;
        Ok(
            channel_storage::list_guild_channels(&self.database, guild_id)
                .await?
                .into_iter()
                .filter(|channel| {
                    channel.kind == ChannelKind::Text
                        && channel_permissions(
                            user_id,
                            membership.owner_id,
                            guild_id,
                            &membership.member_roles,
                            &channel.permission_overwrites,
                        )
                        .contains(readable)
                })
                .collect(),
        )
    }

    /// What `user_id` may do in the voice channel `channel_id` now (ADR-0030), read fresh from the
    /// database: the voice service asks on every join and after every change of who sees what.
    pub(crate) async fn voice_access(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
    ) -> Result<VoiceAccess, GuildError> {
        let Some(channel) = channel_storage::find_guild_channel(&self.database, channel_id)
            .await?
            .filter(|channel| channel.kind == ChannelKind::Voice)
        else {
            return Ok(VoiceAccess::ChannelGone);
        };
        let Some(membership) =
            guild_storage::find_membership(&self.database, channel.guild_id, user_id).await?
        else {
            return Ok(VoiceAccess::NotMember);
        };
        let permissions = channel_permissions(
            user_id,
            membership.owner_id,
            channel.guild_id,
            &membership.member_roles,
            &channel.permission_overwrites,
        );
        Ok(match voice_entitlement(permissions) {
            VoiceEntitlement::Refused => VoiceAccess::Denied,
            entitlement => VoiceAccess::Allowed {
                guild_id: channel.guild_id,
                user_limit: channel.user_limit,
                may_speak: entitlement == VoiceEntitlement::Speak,
                may_use_soundboard: may_use_soundboard(permissions),
            },
        })
    }

    /// The ids of every channel `user_id` can view, across their guilds.
    pub(crate) async fn visible_channel_ids(
        &self,
        user_id: Snowflake,
    ) -> Result<HashSet<Snowflake>, GuildError> {
        let mut visible = HashSet::new();
        for summary in guild_storage::list_user_guilds(&self.database, user_id).await? {
            match self.get_guild(user_id, summary.id).await {
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
            .map(|name| normalize_channel_name(&name))
            .transpose()?;
        // Only text channels show a topic: it is ignored elsewhere.
        let topic = request
            .topic
            .filter(|_| channel.kind == ChannelKind::Text)
            .map(|topic| normalize_topic(&topic))
            .transpose()?;
        let user_limit = match request.user_limit {
            UserLimitChange::Set(limit) => {
                validate_user_limit(channel.kind, Some(limit))?;
                UserLimitChange::Set(limit)
            }
            other => other,
        };
        let new_user_limit = match user_limit {
            UserLimitChange::Keep => channel.user_limit,
            UserLimitChange::Clear => None,
            UserLimitChange::Set(limit) => Some(limit),
        };
        let user_limit_text = |limit: Option<u8>| limit.map(|limit| limit.to_string());
        let changes: Vec<AuditChange> = [
            text_change(AuditChangeKey::Name, &channel.name, name.as_deref()),
            text_change(AuditChangeKey::Topic, &channel.topic, topic.as_deref()),
            (new_user_limit != channel.user_limit).then(|| AuditChange {
                key: AuditChangeKey::UserLimit,
                old_value: user_limit_text(channel.user_limit).unwrap_or_default(),
                new_value: user_limit_text(new_user_limit).unwrap_or_default(),
            }),
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
        let updated = channel_storage::update_channel_fields(
            &self.database,
            channel.id,
            ChannelFieldChanges {
                name: name.as_deref(),
                topic: topic.as_deref(),
                user_limit,
            },
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

    /// Needs `MANAGE_CHANNELS`, and also in each channel moved to another category. Only the
    /// channels the member can view are listed and moved (`merge_channel_order`). Online members
    /// get one fresh snapshot each rather than an event per moved channel: a move renumbers
    /// nearly every channel, and hundreds of events at once would fill their session queues.
    pub(crate) async fn reorder_channels(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        request: ReorderChannelsRequest,
    ) -> Result<Vec<Channel>, GuildError> {
        let (membership, permissions) = self.membership_and_permissions(guild_id, user_id).await?;
        if !permissions.contains(Permissions::MANAGE_CHANNELS) {
            return Err(GuildError::MissingPermissions);
        }
        let channels = channel_storage::list_guild_channels(&self.database, guild_id).await?;
        let permissions_by_channel: HashMap<Snowflake, Permissions> = channels
            .iter()
            .map(|channel| {
                let permissions = channel_permissions(
                    user_id,
                    membership.owner_id,
                    guild_id,
                    &membership.member_roles,
                    &channel.permission_overwrites,
                );
                (channel.id, permissions)
            })
            .collect();
        let may = |channel_id: Snowflake, needed: Permissions| {
            permissions_by_channel
                .get(&channel_id)
                .is_some_and(|permissions| permissions.contains(needed))
        };
        let current: Vec<ChannelPlace> = channels
            .iter()
            .map(|channel| ChannelPlace {
                id: channel.id,
                kind: channel.kind,
                parent_id: channel.parent_id,
            })
            .collect();
        let merged = merge_channel_order(
            &current,
            |channel_id| may(channel_id, Permissions::VIEW_CHANNEL),
            &request.channels,
        )?;
        // Positions left with gaps by deletions are not worth a write on their own.
        if merged == current {
            return Ok(self.get_guild(user_id, guild_id).await?.channels);
        }
        let parent_before: HashMap<Snowflake, Option<Snowflake>> = current
            .iter()
            .map(|place| (place.id, place.parent_id))
            .collect();
        let mut audits =
            vec![self.audit_entry(guild_id, user_id, AuditAction::ChannelsReorder, None, None)?];
        for place in &merged {
            let Some(&before) = parent_before.get(&place.id) else {
                continue;
            };
            if before == place.parent_id {
                continue;
            }
            if !may(place.id, Permissions::MANAGE_CHANNELS) {
                return Err(GuildError::MissingPermissions);
            }
            let id_text = |id: Option<Snowflake>| id.map(|id| id.to_string()).unwrap_or_default();
            audits.push(NewAuditEntry {
                changes: vec![AuditChange {
                    key: AuditChangeKey::Parent,
                    old_value: id_text(before),
                    new_value: id_text(place.parent_id),
                }],
                ..self.audit_entry(
                    guild_id,
                    user_id,
                    AuditAction::ChannelUpdate,
                    Some(place.id),
                    None,
                )?
            });
        }
        let placements = |places: &[ChannelPlace]| -> Vec<(Snowflake, Option<Snowflake>)> {
            places
                .iter()
                .map(|place| (place.id, place.parent_id))
                .collect()
        };
        let is_written = channel_storage::reorder_channels(
            &self.database,
            guild_id,
            &placements(&current),
            &placements(&merged),
            &audits,
        )
        .await?;
        if !is_written {
            return Err(GuildError::Conflict);
        }
        self.resynchronize(guild_id, AffectedMembers::Everyone)
            .await;
        Ok(self.get_guild(user_id, guild_id).await?.channels)
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
        // Detached: its messages go in batches (ADR-0029), and a client hanging up must not stop
        // the deletion half way.
        let database = self.database.clone();
        let channel_id = channel.id;
        tokio::spawn(async move {
            channel_storage::delete_channel(&database, channel_id, &audit).await
        })
        .await
        .map_err(|error| GuildError::Internal(Box::new(error)))??;
        self.publish_to_audience(
            channel.guild_id,
            &audience,
            &GatewayEvent::ChannelDelete {
                guild_id: channel.guild_id,
                id: channel.id,
            },
        );
        self.events.note_guild_access_changed(channel.guild_id);
        Ok(())
    }
}
