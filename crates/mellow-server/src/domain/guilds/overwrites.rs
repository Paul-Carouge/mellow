//! Channel permission overwrites, set only within the actor's role hierarchy and held
//! permissions.

use std::collections::HashSet;

use mellow_protocol::{
    AuditAction, AuditChangeKey, Channel, GatewayEvent, OverwriteKind, PermissionOverwrite,
    Permissions, SetPermissionOverwriteRequest, Snowflake,
};

use super::audience::{AffectedMembers, viewer_ids};
use super::{GuildError, Guilds, channel_from_stored, limit_reached, permissions_change};
use crate::domain::guild_rules::{
    GuildRuleViolation, MAXIMUM_OVERWRITES_PER_CHANNEL, validate_known_permissions,
};
use crate::domain::permissions::RoleManager;
use crate::storage::audit_log::NewAuditEntry;
use crate::storage::guild_channels::{self as channel_storage, StoredChannel};
use crate::storage::guild_members as member_storage;
use crate::storage::guild_roles as role_storage;
use crate::storage::guilds as guild_storage;

impl Guilds {
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
        channel_storage::set_overwrite(
            &self.database,
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
            let role = role_storage::list_roles(&self.database, channel.guild_id)
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
        if !channel_storage::delete_overwrite(&self.database, channel.id, target_id, held, &audit)
            .await?
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
        let membership = guild_storage::find_membership(&self.database, guild_id, user_id)
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
                let role = role_storage::list_roles(&self.database, guild_id)
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
                member_storage::find_member(&self.database, guild_id, wanted.id)
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
        let updated = channel_storage::find_guild_channel(&self.database, channel.id)
            .await?
            .ok_or(GuildError::NotFound)?;
        let snapshot = self.permission_snapshot(channel.guild_id).await?;
        let viewers_after = viewer_ids(channel.guild_id, &snapshot, &updated.permission_overwrites);
        let still_viewing: HashSet<Snowflake> = viewers_before
            .intersection(&viewers_after)
            .copied()
            .collect();
        let updated_channel = channel_from_stored(updated);
        // CONNECT or SPEAK may have changed without changing who sees the channel.
        self.events.note_guild_access_changed(channel.guild_id);
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
        let snapshot = self.permission_snapshot(channel.guild_id).await?;
        Ok(viewer_ids(
            channel.guild_id,
            &snapshot,
            &channel.permission_overwrites,
        ))
    }
}
