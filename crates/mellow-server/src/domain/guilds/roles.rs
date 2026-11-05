//! Roles and who holds them, managed only below the actor's highest role.

use std::collections::HashSet;

use mellow_protocol::{
    AuditAction, AuditChange, AuditChangeKey, CreateRoleRequest, GatewayEvent, Permissions,
    ReorderRolesRequest, Role, Snowflake, UpdateRoleRequest,
};

use super::audience::AffectedMembers;
use super::{GuildError, Guilds, limit_reached, permissions_change, text_change};
use crate::domain::guild_rules::{
    GuildRuleViolation, MAXIMUM_ROLES_PER_GUILD, normalize_role_name, validate_known_permissions,
    validate_role_color, validate_role_order,
};
use crate::domain::permissions::RoleManager;
use crate::storage::audit_log::NewAuditEntry;
use crate::storage::guild_roles::{self as role_storage, RoleFieldChanges};
use crate::storage::guilds as guild_storage;

impl Guilds {
    /// The member acting on roles and the role they target, or `NotFound` for non-members and
    /// roles of other guilds, `MissingPermissions` outside the hierarchy (`RoleManager`).
    pub(super) async fn manager_and_manageable_role(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        role_id: Snowflake,
    ) -> Result<(RoleManager, Role), GuildError> {
        let membership = guild_storage::find_membership(&self.database, guild_id, user_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        let manager = RoleManager::new(user_id, membership.owner_id, &membership.member_roles);
        if !manager.permissions.contains(Permissions::MANAGE_ROLES) {
            return Err(GuildError::MissingPermissions);
        }
        let role = role_storage::list_roles(&self.database, guild_id)
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

    pub(super) fn publish_roles(&self, guild_id: Snowflake, roles: Vec<Role>) {
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
        let membership = guild_storage::find_membership(&self.database, guild_id, user_id)
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
        let roles = role_storage::insert_role_above_everyone(
            &self.database,
            guild_id,
            &Role {
                id: role_id,
                name,
                color,
                position: 1,
                permissions,
                is_displayed_separately: request.is_displayed_separately,
                is_mentionable: request.is_mentionable,
                icon: None,
                unicode_emoji: None,
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
    /// permissions: it is never displayed separately, since everyone holds it, and never
    /// mentionable, since mentioning everyone always needs `MENTION_EVERYONE` (ADR-0040).
    pub(crate) async fn update_role(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        role_id: Snowflake,
        request: UpdateRoleRequest,
    ) -> Result<Role, GuildError> {
        if role_id == guild_id
            && (request.name.is_some()
                || request.color.is_some()
                || request.is_displayed_separately.is_some()
                || request.is_mentionable.is_some())
        {
            return Err(GuildRuleViolation::EveryoneRole.into());
        }
        let name = request
            .name
            .as_deref()
            .map(normalize_role_name)
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
        let changes = role_audit_changes(
            &role,
            name.as_deref(),
            color,
            effective_permissions,
            &request,
        );
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
        let updated = role_storage::update_role_fields(
            &self.database,
            guild_id,
            role_id,
            &RoleFieldChanges {
                name: name.as_deref(),
                color,
                permissions,
                is_displayed_separately: request.is_displayed_separately,
                is_mentionable: request.is_mentionable,
            },
            manager.permissions,
            &audit,
        )
        .await?
        .ok_or(GuildError::NotFound)?;
        self.publish_roles(
            guild_id,
            role_storage::list_roles(&self.database, guild_id).await?,
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
        let membership = guild_storage::find_membership(&self.database, guild_id, user_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        let roles = role_storage::list_roles(&self.database, guild_id).await?;
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
        let roles = role_storage::reorder_roles(
            &self.database,
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
        if !role_storage::delete_role(&self.database, guild_id, role_id, &audit).await? {
            return Err(GuildError::NotFound);
        }
        self.publish_roles(
            guild_id,
            role_storage::list_roles(&self.database, guild_id).await?,
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
        let role_ids = role_storage::add_member_role(
            &self.database,
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
            role_storage::remove_member_role(&self.database, guild_id, member_id, role_id, &audit)
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
}

/// What an update changes in `role`, for the moderation log; empty when nothing changes.
fn role_audit_changes(
    role: &Role,
    name: Option<&str>,
    color: Option<u32>,
    effective_permissions: Option<Permissions>,
    request: &UpdateRoleRequest,
) -> Vec<AuditChange> {
    [
        text_change(AuditChangeKey::Name, &role.name, name),
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
        request
            .is_displayed_separately
            .and_then(|is_displayed_separately| {
                text_change(
                    AuditChangeKey::DisplayedSeparately,
                    &role.is_displayed_separately.to_string(),
                    Some(&is_displayed_separately.to_string()),
                )
            }),
        request.is_mentionable.and_then(|is_mentionable| {
            text_change(
                AuditChangeKey::Mentionable,
                &role.is_mentionable.to_string(),
                Some(&is_mentionable.to_string()),
            )
        }),
    ]
    .into_iter()
    .flatten()
    .collect()
}
