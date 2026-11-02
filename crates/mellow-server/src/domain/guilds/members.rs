//! Members: listing, leaving, and moderation (kicks and bans).

use mellow_protocol::{
    AuditAction, Ban, CreateBanRequest, GatewayEvent, Member, Permissions, PublicUser, Snowflake,
};

use super::{GuildError, Guilds, member_from_stored};
use crate::domain::guild_rules::{
    GuildRuleViolation, MAXIMUM_BANS_LISTED, MAXIMUM_MEMBERS_LISTED, normalize_ban_reason,
};
use crate::domain::permissions::RoleManager;
use crate::domain::profile_images::fill_profile_images;
use crate::storage::audit_log::NewAuditEntry;
use crate::storage::guild_members as member_storage;
use crate::storage::guilds::{self as guild_storage, Membership};

impl Guilds {
    /// Members only. shortcut: unpaginated; fine for M1 community sizes, paginate with the member list UI.
    pub(crate) async fn list_members(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
    ) -> Result<Vec<Member>, GuildError> {
        self.membership_and_permissions(guild_id, user_id).await?;
        let mut members: Vec<Member> =
            member_storage::list_members(&self.database, guild_id, MAXIMUM_MEMBERS_LISTED)
                .await?
                .into_iter()
                .map(member_from_stored)
                .collect();
        fill_profile_images(
            &self.database,
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
        member_storage::delete_member(&self.database, guild_id, user_id, None).await?;
        self.publish_member_removed(guild_id, user_id);
        Ok(())
    }

    /// The member's sessions stop receiving the guild, which disappears for them, and the other
    /// members see them go.
    fn publish_member_removed(&self, guild_id: Snowflake, user_id: Snowflake) {
        self.events.unsubscribe_user_from_guild(user_id, guild_id);
        self.events.note_guild_access_changed(guild_id);
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
        let membership = guild_storage::find_membership(&self.database, guild_id, user_id)
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
        let Some(target) =
            guild_storage::find_membership(&self.database, guild_id, target_id).await?
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
        member_storage::delete_member(&self.database, guild_id, target_id, Some(&audit)).await?;
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
            && !guild_storage::user_exists(&self.database, target_id).await?
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
        let was_member = member_storage::ban_and_remove_member(
            &self.database,
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
        if member_storage::delete_ban(&self.database, guild_id, target_id, &audit).await? {
            Ok(())
        } else {
            Err(GuildError::NotFound)
        }
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
            member_storage::list_bans(&self.database, guild_id, MAXIMUM_BANS_LISTED)
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
        fill_profile_images(&self.database, bans.iter_mut().map(|ban| &mut ban.user))
            .await
            .map_err(|error| GuildError::Internal(Box::new(error)))?;
        Ok(bans)
    }
}
