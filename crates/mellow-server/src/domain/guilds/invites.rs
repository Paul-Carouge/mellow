//! Invites: creating, listing, revoking and joining through them.

use mellow_protocol::{
    AuditAction, AuditChange, AuditChangeKey, CreateInviteRequest, GatewayEvent, Guild,
    GuildInvite, Invite, Permissions, PublicUser, Snowflake,
};

use super::{GuildError, Guilds, limit_reached, member_from_stored};
use crate::domain::guild_rules::{
    MAXIMUM_ACTIVE_INVITES_PER_GUILD, generate_invite_code, normalize_invite_code,
    validate_invite_limits,
};
use crate::domain::profile_images::fill_profile_images;
use crate::storage::audit_log::NewAuditEntry;
use crate::storage::guild_invites::{self as invite_storage, JoinOutcome, StoredInvite};
use crate::storage::guild_members as member_storage;
use crate::storage::guilds as guild_storage;

const MILLISECONDS_PER_SECOND: u64 = 1_000;

/// Characters of an invite code kept in the audit log.
pub(super) const INVITE_CODE_HINT_LENGTH: usize = 4;

/// What the audit log keeps of an invite code: enough to tell invites apart, too little to join
/// with, since readers of the log need not be allowed to invite.
pub(super) fn invite_code_hint(code: &str) -> String {
    let prefix: String = code.chars().take(INVITE_CODE_HINT_LENGTH).collect();
    format!("{prefix}…")
}

impl Guilds {
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
        let audit = NewAuditEntry {
            changes: vec![AuditChange {
                key: AuditChangeKey::Code,
                old_value: String::new(),
                new_value: invite_code_hint(&invite.code),
            }],
            ..self.audit_entry(guild_id, user_id, AuditAction::InviteCreate, None, None)?
        };
        invite_storage::insert_invite(
            &self.database,
            &invite,
            MAXIMUM_ACTIVE_INVITES_PER_GUILD,
            &audit,
        )
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

    /// Needs `MANAGE_GUILD`. Only invites that can still be used, newest first.
    pub(crate) async fn list_invites(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        now_unix_milliseconds: u64,
    ) -> Result<Vec<GuildInvite>, GuildError> {
        self.require_member_permissions(guild_id, user_id, Permissions::MANAGE_GUILD)
            .await?;
        let mut invites: Vec<GuildInvite> = invite_storage::list_usable_invites(
            &self.database,
            guild_id,
            now_unix_milliseconds,
            MAXIMUM_ACTIVE_INVITES_PER_GUILD,
        )
        .await?
        .into_iter()
        .map(|listed| GuildInvite {
            code: listed.invite.code,
            creator: PublicUser {
                id: listed.invite.creator_id,
                username: listed.creator_username,
                display_name: listed.creator_display_name,
                avatar: None,
                banner: None,
            },
            uses: listed.invite.uses,
            max_uses: listed.invite.max_uses,
            expires_at_unix_milliseconds: listed.invite.expires_at_unix_milliseconds,
            created_at_unix_milliseconds: listed.invite.created_at_unix_milliseconds,
        })
        .collect();
        fill_profile_images(
            &self.database,
            invites.iter_mut().map(|invite| &mut invite.creator),
        )
        .await
        .map_err(|error| GuildError::Internal(Box::new(error)))?;
        Ok(invites)
    }

    /// Needs `MANAGE_GUILD` in the invite's guild. An unknown code and an invite of a guild the
    /// user is not in are both `NotFound`, so codes cannot be probed.
    pub(crate) async fn delete_invite(
        &self,
        user_id: Snowflake,
        code: &str,
    ) -> Result<(), GuildError> {
        let guild_id = invite_storage::find_invite_guild_id(&self.database, code)
            .await?
            .ok_or(GuildError::NotFound)?;
        self.require_member_permissions(guild_id, user_id, Permissions::MANAGE_GUILD)
            .await?;
        let audit = NewAuditEntry {
            changes: vec![AuditChange {
                key: AuditChangeKey::Code,
                old_value: invite_code_hint(code),
                new_value: String::new(),
            }],
            ..self.audit_entry(guild_id, user_id, AuditAction::InviteDelete, None, None)?
        };
        if invite_storage::delete_invite(&self.database, guild_id, code, &audit).await? {
            Ok(())
        } else {
            Err(GuildError::NotFound)
        }
    }

    /// Joins (or, for an existing member, simply opens) the guild an invite points to.
    pub(crate) async fn join_with_invite(
        &self,
        user_id: Snowflake,
        code: &str,
        now_unix_milliseconds: u64,
    ) -> Result<Guild, GuildError> {
        let code = normalize_invite_code(code);
        let (guild_id, is_new_member) = match invite_storage::join_with_invite(
            &self.database,
            &code,
            user_id,
            now_unix_milliseconds,
        )
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
            if guild_storage::find_membership(&self.database, guild_id, user_id)
                .await?
                .is_none()
            {
                self.events.unsubscribe_user_from_guild(user_id, guild_id);
                return Err(GuildError::NotFound);
            }
        }
        let guild = self.get_guild(user_id, guild_id).await?;
        if is_new_member {
            if let Some(member) =
                member_storage::find_member(&self.database, guild_id, user_id).await?
            {
                let mut member = member_from_stored(member);
                // The join is stored: announce it even without the pictures.
                if let Err(error) = fill_profile_images(&self.database, [&mut member.user]).await {
                    tracing::warn!(%error, "cannot read a new member's pictures");
                }
                self.events
                    .publish_to_guild(guild_id, &GatewayEvent::GuildMemberAdd { guild_id, member });
            }
            if let Some(presence) = self.events.visible_presence_of(user_id) {
                self.events
                    .publish_to_guild(guild_id, &GatewayEvent::PresenceUpdate(presence));
            }
            self.events
                .publish_to_user(user_id, &GatewayEvent::GuildCreate(Box::new(guild.clone())));
            // The voice service then announces who is connected, the newcomer included.
            self.events.note_guild_access_changed(guild_id);
        }
        Ok(guild)
    }
}
