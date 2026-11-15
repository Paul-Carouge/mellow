//! What a profile says about a person beyond their card (`profile-facts.ts`, DESIGN.md §5.7.1,
//! §5.7.2): their place in the server being read (roles, the role icon after their name, when they
//! joined it) and when they joined Mellow, read from their id (ADR-0004). Known locally: shown at
//! once, no request.

use mellow_client_core::shared_logic::role_icons::highest_role_with_icon;
use mellow_client_core::state::ClientState;
use mellow_protocol::{PresenceStatus, PublicUser, Role, Snowflake};

use crate::components::{RoleChip, RoleChips};
use crate::i18n::Strings;

/// The person as the profile shows them, from the freshest copy the state holds.
pub(super) struct ProfileFacts {
    pub user: PublicUser,
    pub presence: PresenceStatus,
    pub custom_status: Option<String>,
    pub is_self: bool,
    pub relationship: Option<mellow_protocol::RelationshipKind>,
    /// Their highest role with an icon, after their name.
    pub role_icon: Option<Role>,
    /// The server's name and its roles held, highest first; `None` outside a server, before its
    /// members arrived, or once they left it.
    pub server: Option<ServerFacts>,
    /// The account's creation, from its id.
    pub joined_mellow_at_unix_milliseconds: u64,
}

pub(super) struct ServerFacts {
    pub name: String,
    pub is_owner: bool,
    /// Without `@everyone`, highest first.
    pub roles: Vec<Role>,
    pub joined_at_unix_milliseconds: u64,
}

impl ServerFacts {
    /// The chips of §5.7.1: the owner's crown first, each role's icon in its dot's place; at most
    /// `limit` chips, the rest counted in a last "+2".
    pub fn role_chips(
        &self,
        person: &str,
        limit: Option<usize>,
        strings: Strings,
    ) -> Option<RoleChips> {
        if !self.is_owner && self.roles.is_empty() {
            return None;
        }
        let owner = self.is_owner.then(|| RoleChip::Owner {
            label: strings.server_settings().members().owner().into(),
        });
        let room = limit.map_or(usize::MAX, |limit| {
            limit.saturating_sub(usize::from(self.is_owner))
        });
        let shown = self.roles.iter().take(room).map(|role| RoleChip::Role {
            name: role.name.clone().into(),
            colour: role.color,
            icon: Some(Box::new(role.clone())),
        });
        let hidden = self.roles.len().saturating_sub(room);
        let chips = owner.into_iter().chain(shown).collect();
        Some(
            RoleChips::new(strings.profile_card().roles_of(person), chips).hidden(
                hidden,
                strings
                    .profile_card()
                    .more_roles(i64::try_from(hidden).unwrap_or(i64::MAX)),
            ),
        )
    }
}

/// `user` as the state knows them now, in `guild_id` when given.
pub(super) fn profile_facts(
    user: &PublicUser,
    guild_id: Option<Snowflake>,
    state: &ClientState,
) -> ProfileFacts {
    let user_id = user.id;
    let guild = guild_id.and_then(|guild_id| state.guild(guild_id));
    let member = guild.and_then(|guild| {
        state
            .members(guild.id)
            .iter()
            .find(|member| member.user.id == user_id)
    });
    let relationship = state.relationship(user_id);
    let is_self = state.current_user().is_some_and(|me| me.id == user_id);
    // The freshest copy: the member list, relationships and your own account follow USER_UPDATE.
    let fresh_user = if is_self {
        state.current_user_as_public_user()
    } else {
        member
            .map(|member| member.user.clone())
            .or_else(|| relationship.map(|relationship| relationship.user.clone()))
    }
    .unwrap_or_else(|| user.clone());
    let server = guild.zip(member).map(|(guild, member)| {
        let mut roles: Vec<Role> = guild
            .roles
            .iter()
            .filter(|role| role.id != guild.id && member.role_ids.contains(&role.id))
            .cloned()
            .collect();
        roles.sort_by_key(|role| std::cmp::Reverse(role.position));
        ServerFacts {
            name: guild.name.clone(),
            is_owner: guild.owner_id == user_id,
            roles,
            joined_at_unix_milliseconds: member.joined_at_unix_milliseconds,
        }
    });
    ProfileFacts {
        user: fresh_user,
        presence: state.presence_of(user_id),
        custom_status: state.custom_status_of(user_id).map(str::to_owned),
        is_self,
        relationship: relationship.map(|relationship| relationship.kind),
        role_icon: guild
            .zip(member)
            .and_then(|(guild, member)| highest_role_with_icon(&guild.roles, &member.role_ids))
            .cloned(),
        server,
        joined_mellow_at_unix_milliseconds: user_id.unix_milliseconds(),
    }
}
