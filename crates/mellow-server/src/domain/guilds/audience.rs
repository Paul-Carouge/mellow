//! Who hears about a channel and who gets a fresh guild snapshot (ADR-0015).

use std::collections::HashSet;

use mellow_protocol::{GatewayEvent, Guild, PermissionOverwrite, Permissions, Role, Snowflake};

use super::{GuildError, Guilds, guild_seen_by_member};
use crate::domain::permissions::{channel_permissions_with_role_ids, is_visible_to_every_member};
use crate::storage::guild_channels::{self as channel_storage, StoredChannel};
use crate::storage::guild_sounds as guild_sounds_storage;
use crate::storage::guilds::{self as guild_storage, StoredPermissionSnapshot};

/// The roles of a member holding `role_ids`, `@everyone` included, by position.
fn roles_with_ids_and_everyone(
    guild_id: Snowflake,
    snapshot: &StoredPermissionSnapshot,
    role_ids: &[Snowflake],
) -> Vec<Role> {
    snapshot
        .roles
        .iter()
        .filter(|role| role.id == guild_id || role_ids.contains(&role.id))
        .cloned()
        .collect()
}

/// The members who can view a channel with these overwrites.
pub(super) fn viewer_ids(
    guild_id: Snowflake,
    snapshot: &StoredPermissionSnapshot,
    overwrites: &[PermissionOverwrite],
) -> HashSet<Snowflake> {
    snapshot
        .role_ids_by_member
        .iter()
        .filter(|(member_id, role_ids)| {
            channel_permissions_with_role_ids(
                **member_id,
                snapshot.owner_id,
                guild_id,
                &snapshot.roles,
                role_ids,
                overwrites,
            )
            .contains(Permissions::VIEW_CHANNEL)
        })
        .map(|(member_id, _)| *member_id)
        .collect()
}

/// Who receives events about a channel (ADR-0015).
pub(crate) enum ChannelAudience {
    EveryMember,
    Members(HashSet<Snowflake>),
}

/// Which online members need a fresh guild snapshot after a permission change (ADR-0015).
pub(super) enum AffectedMembers {
    Everyone,
    Some(HashSet<Snowflake>),
}

impl Guilds {
    /// Who can view `channel` as stored now: its overwrites are read again, so a change committed
    /// since the caller's check is taken into account. Callers compute it before writing, so a
    /// failure here leaves nothing half done (ADR-0015).
    pub(crate) async fn channel_audience(
        &self,
        channel: &StoredChannel,
    ) -> Result<ChannelAudience, GuildError> {
        let overwrites = channel_storage::find_guild_channel(&self.database, channel.id)
            .await?
            .map(|current| current.permission_overwrites)
            .unwrap_or_default();
        self.audience_with_overwrites(channel.guild_id, &overwrites)
            .await
    }

    /// `EveryMember` without looking at members when no one can be hidden from a channel with
    /// these overwrites, otherwise the members who can view it.
    pub(super) async fn audience_with_overwrites(
        &self,
        guild_id: Snowflake,
        overwrites: &[PermissionOverwrite],
    ) -> Result<ChannelAudience, GuildError> {
        // One primary-key read, so public channels never load the member list.
        let everyone_permissions = guild_storage::everyone_permissions(&self.database, guild_id)
            .await?
            .unwrap_or(Permissions::empty());
        if is_visible_to_every_member(everyone_permissions, overwrites) {
            return Ok(ChannelAudience::EveryMember);
        }
        let snapshot = self.permission_snapshot(guild_id).await?;
        Ok(ChannelAudience::Members(viewer_ids(
            guild_id, &snapshot, overwrites,
        )))
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
        self.events.publish_to_users(user_ids, event);
    }

    /// Sends each affected online member a fresh snapshot, since a permission change may have
    /// hidden or revealed channels for them (ADR-0015). Never fails: the change is committed, so a
    /// member whose snapshot cannot be built is skipped and resynchronises at their next `Ready`.
    pub(super) async fn resynchronize(&self, guild_id: Snowflake, affected: AffectedMembers) {
        self.resynchronize_snapshots(guild_id, affected).await;
        // After the snapshots: the voice service re-checks and re-announces who is connected,
        // and a member who now sees a voice channel must know it first.
        self.events.note_guild_access_changed(guild_id);
    }

    async fn resynchronize_snapshots(&self, guild_id: Snowflake, affected: AffectedMembers) {
        let online = self.events.online_user_ids_of_guild(guild_id);
        let recipients: Vec<Snowflake> = match affected {
            AffectedMembers::Everyone => online.into_iter().collect(),
            AffectedMembers::Some(user_ids) => online.intersection(&user_ids).copied().collect(),
        };
        if recipients.is_empty() {
            return;
        }
        match self.guild_seen_by_each(guild_id, &recipients).await {
            Ok(guilds) => {
                for (user_id, guild) in guilds {
                    self.events
                        .publish_to_user(user_id, &GatewayEvent::GuildCreate(Box::new(guild)));
                }
            }
            // Deleted meanwhile: every member's GUILD_DELETE is already on its way.
            Err(GuildError::NotFound) => {}
            Err(error) => {
                tracing::warn!(%error, "cannot resynchronize members after a permission change");
            }
        }
    }

    /// The guild as each of `user_ids` sees it, read once for all of them; users who are not
    /// members (left meanwhile, so their `GUILD_DELETE` is on its way) are left out.
    async fn guild_seen_by_each(
        &self,
        guild_id: Snowflake,
        user_ids: &[Snowflake],
    ) -> Result<Vec<(Snowflake, Guild)>, GuildError> {
        let snapshot = self.permission_snapshot(guild_id).await?;
        let summary = guild_storage::find_guild_summary(&self.database, guild_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        let channels = channel_storage::list_guild_channels(&self.database, guild_id).await?;
        let expressions = self.expressions_of_guild(guild_id).await?;
        let join_sounds =
            guild_sounds_storage::join_sounds_of_guild(&self.database, guild_id).await?;
        Ok(user_ids
            .iter()
            .filter_map(|user_id| {
                let role_ids = snapshot.role_ids_by_member.get(user_id)?;
                let roles_of_member = roles_with_ids_and_everyone(guild_id, &snapshot, role_ids);
                Some((
                    *user_id,
                    Guild {
                        my_join_sound_id: join_sounds.get(user_id).copied(),
                        ..guild_seen_by_member(
                            *user_id,
                            summary.clone(),
                            snapshot.owner_id,
                            &roles_of_member,
                            snapshot.roles.clone(),
                            &channels,
                            expressions.clone(),
                        )
                    },
                ))
            })
            .collect())
    }

    /// Who can view the guild channel `channel_id` now, or `None` once it is gone (its viewers
    /// were told by `CHANNEL_DELETE`).
    pub(crate) async fn audience_of_guild_channel(
        &self,
        channel_id: Snowflake,
    ) -> Result<Option<ChannelAudience>, GuildError> {
        let Some(channel) = channel_storage::find_guild_channel(&self.database, channel_id).await?
        else {
            return Ok(None);
        };
        self.audience_with_overwrites(channel.guild_id, &channel.permission_overwrites)
            .await
            .map(Some)
    }
}
