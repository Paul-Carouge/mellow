//! Presences, custom statuses, profiles shown everywhere, and relationships (ADR-0025).

use mellow_protocol::{
    ChosenStatus, CurrentUser, Presence, PresenceStatus, PublicUser, Relationship,
    RelationshipKind, Snowflake,
};

use super::pins::Pins;
use super::{ClientState, StateChange};

impl ClientState {
    /// Whether `user_id` shows as online, idle or do not disturb.
    #[must_use]
    pub fn is_online(&self, user_id: Snowflake) -> bool {
        self.presence_of(user_id) != PresenceStatus::Offline
    }

    /// What this client shows for `user_id`. The signed-in user sees their own pick, invisible as
    /// offline, even before the instance echoes it.
    #[must_use]
    pub fn presence_of(&self, user_id: Snowflake) -> PresenceStatus {
        if let Some(user) = self.current_user.as_ref().filter(|user| user.id == user_id) {
            return match user.status {
                ChosenStatus::Online => PresenceStatus::Online,
                ChosenStatus::Idle => PresenceStatus::Idle,
                ChosenStatus::DoNotDisturb => PresenceStatus::DoNotDisturb,
                ChosenStatus::Invisible => PresenceStatus::Offline,
            };
        }
        self.presence_by_user_id
            .get(&user_id)
            .map_or(PresenceStatus::Offline, |presence| presence.status)
    }

    /// `None` for offline and invisible users, and for the signed-in user without one.
    #[must_use]
    pub fn custom_status_of(&self, user_id: Snowflake) -> Option<&str> {
        if let Some(user) = self.current_user.as_ref().filter(|user| user.id == user_id) {
            return match user.status {
                ChosenStatus::Invisible => None,
                _ => user.custom_status.as_deref(),
            };
        }
        self.presence_by_user_id
            .get(&user_id)
            .and_then(|presence| presence.custom_status.as_deref())
    }

    /// `PRESENCE_UPDATE`. For the signed-in user it keeps their own pick in step with their other
    /// devices, so a status chosen here never sends back a custom status another device already
    /// changed. While this session is live they can only look offline by going invisible; the
    /// custom status then stays as this client knew it.
    pub(super) fn apply_presence(&mut self, presence: Presence) -> StateChange {
        let user_id = presence.user_id;
        if let Some(user) = self.current_user.as_mut().filter(|user| user.id == user_id) {
            match presence.status {
                PresenceStatus::Offline => user.status = ChosenStatus::Invisible,
                PresenceStatus::Online => user.status = ChosenStatus::Online,
                PresenceStatus::Idle => user.status = ChosenStatus::Idle,
                PresenceStatus::DoNotDisturb => user.status = ChosenStatus::DoNotDisturb,
            }
            if presence.status != PresenceStatus::Offline {
                user.custom_status.clone_from(&presence.custom_status);
            }
        }
        if presence.status == PresenceStatus::Offline {
            self.presence_by_user_id.remove(&user_id);
        } else {
            self.presence_by_user_id.insert(user_id, presence);
        }
        StateChange::Presence { user_id }
    }

    /// The instance's answer to a change of the user's own account (a status chosen in the user
    /// card): the user everywhere they appear, as `UserUpdate` does, and their own pick at once,
    /// before the instance echoes it (the web's `applyCurrentUser`).
    pub fn apply_current_user(&mut self, user: CurrentUser) -> StateChange {
        let public = PublicUser {
            id: user.id,
            username: user.username.clone(),
            display_name: user.display_name.clone(),
            avatar: user.avatar,
            banner: user.banner,
        };
        let user_id = user.id;
        self.current_user = Some(user);
        self.apply_user_update(&public);
        StateChange::Presence { user_id }
    }

    /// A user's name or pictures changed: wherever they appear. An absent avatar or banner was
    /// removed.
    pub(super) fn apply_user_update(&mut self, user: &PublicUser) -> StateChange {
        if let Some(current_user) = self
            .current_user
            .as_mut()
            .filter(|current_user| current_user.id == user.id)
        {
            current_user.username.clone_from(&user.username);
            current_user.display_name.clone_from(&user.display_name);
            current_user.avatar = user.avatar;
            current_user.banner = user.banner;
        }
        for members in self.members_by_guild.values_mut() {
            if let Some(member) = members.iter_mut().find(|member| member.user.id == user.id) {
                member.user.clone_from(user);
                members.sort_by(|left, right| left.user.username.cmp(&right.user.username));
            }
        }
        let messages = self
            .history_by_channel
            .values_mut()
            .flat_map(|history| history.messages.iter_mut())
            .chain(
                self.pins_by_channel
                    .values_mut()
                    .flat_map(Pins::messages_mut),
            );
        for message in messages {
            if message.author.id == user.id {
                message.author.clone_from(user);
            }
        }
        for channel in &mut self.direct_message_channels {
            for recipient in &mut channel.recipients {
                if recipient.id == user.id {
                    recipient.clone_from(user);
                }
            }
        }
        if let Some(relationship) = self.relationship_by_user_id.get_mut(&user.id) {
            relationship.user.clone_from(user);
        }
        StateChange::UserRenamed { user_id: user.id }
    }

    #[must_use]
    pub fn relationship(&self, user_id: Snowflake) -> Option<&Relationship> {
        self.relationship_by_user_id.get(&user_id)
    }

    /// The relationships of `kind`, by name.
    /// shortcut: names compare by lowercase code points, not by locale; use a collator once the
    /// app has one.
    #[must_use]
    pub fn relationships_of_kind(&self, kind: RelationshipKind) -> Vec<&Relationship> {
        let shown_name = |relationship: &Relationship| {
            let user = &relationship.user;
            let name = if user.display_name.is_empty() {
                &user.username
            } else {
                &user.display_name
            };
            name.to_lowercase()
        };
        let mut relationships: Vec<&Relationship> = self
            .relationship_by_user_id
            .values()
            .filter(|relationship| relationship.kind == kind)
            .collect();
        relationships
            .sort_by_cached_key(|relationship| (shown_name(relationship), relationship.user.id));
        relationships
    }

    /// Requests waiting for the user's answer: the Friends entry's badge.
    #[must_use]
    pub fn incoming_request_count(&self) -> usize {
        self.relationship_by_user_id
            .values()
            .filter(|relationship| relationship.kind == RelationshipKind::IncomingRequest)
            .count()
    }

    #[must_use]
    pub fn is_blocked(&self, user_id: Snowflake) -> bool {
        self.relationship(user_id)
            .is_some_and(|relationship| relationship.kind == RelationshipKind::Blocked)
    }

    /// `RELATIONSHIP_ADD`, or the instance's answer to the user's own change (a request, an
    /// acceptance, a block), applied at once.
    pub fn apply_relationship(&mut self, relationship: Relationship) -> StateChange {
        self.relationship_by_user_id
            .insert(relationship.user.id, relationship);
        StateChange::Relationships
    }

    /// `RELATIONSHIP_REMOVE`, or the answer to removing one. A removal answered `NOT_FOUND` counts
    /// as done too: another session or the other person already removed it.
    pub fn forget_relationship(&mut self, user_id: Snowflake) -> StateChange {
        self.relationship_by_user_id.remove(&user_id);
        StateChange::Relationships
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::GatewayEvent;

    use super::super::tests::{ME, NOW, OTHER, ready_state, user};
    use super::*;

    fn relationship(id: u64, display_name: &str, kind: RelationshipKind) -> Relationship {
        Relationship {
            user: PublicUser {
                display_name: display_name.into(),
                username: display_name.to_lowercase(),
                ..user(Snowflake::from_raw(id))
            },
            kind,
            since_unix_milliseconds: 0,
        }
    }

    fn presence(user_id: Snowflake, status: PresenceStatus, custom: Option<&str>) -> Presence {
        Presence {
            user_id,
            status,
            custom_status: custom.map(Into::into),
        }
    }

    #[test]
    fn lists_each_kind_by_name_and_counts_the_requests_to_answer() {
        let mut state = ready_state();
        let zoe = relationship(2, "Zoé", RelationshipKind::Friend);
        let ana = relationship(3, "Ana", RelationshipKind::Friend);
        let bob = relationship(4, "Bob", RelationshipKind::IncomingRequest);
        for relationship in [zoe.clone(), bob, ana.clone()] {
            state.apply_relationship(relationship);
        }
        assert_eq!(
            state.relationships_of_kind(RelationshipKind::Friend),
            [&ana, &zoe]
        );
        assert_eq!(state.incoming_request_count(), 1);
    }

    #[test]
    fn follows_the_gateways_adds_removes_and_profile_changes() {
        let mut state = ready_state();
        let zoe = relationship(2, "Zoé", RelationshipKind::Friend);
        state.apply_event(GatewayEvent::RelationshipAdd(zoe.clone()), NOW);
        state.apply_event(
            GatewayEvent::RelationshipAdd(Relationship {
                kind: RelationshipKind::Blocked,
                ..zoe.clone()
            }),
            NOW,
        );
        assert!(state.is_blocked(zoe.user.id));
        let renamed = PublicUser {
            display_name: "Zoé B.".into(),
            ..zoe.user.clone()
        };
        state.apply_event(GatewayEvent::UserUpdate(renamed), NOW);
        assert_eq!(
            state.relationship(zoe.user.id).unwrap().user.display_name,
            "Zoé B."
        );
        state.apply_event(
            GatewayEvent::RelationshipRemove {
                user_id: zoe.user.id,
            },
            NOW,
        );
        assert!(!state.is_blocked(zoe.user.id));
    }

    #[test]
    fn presences_keep_their_kind_and_custom_status() {
        let mut state = ready_state();
        let change = state.apply_event(
            GatewayEvent::PresenceUpdate(presence(
                OTHER,
                PresenceStatus::DoNotDisturb,
                Some("focus"),
            )),
            NOW,
        );
        assert_eq!(change, StateChange::Presence { user_id: OTHER });
        assert_eq!(state.presence_of(OTHER), PresenceStatus::DoNotDisturb);
        assert_eq!(state.custom_status_of(OTHER), Some("focus"));
        state.apply_event(
            GatewayEvent::PresenceUpdate(presence(OTHER, PresenceStatus::Offline, None)),
            NOW,
        );
        assert_eq!(state.presence_of(OTHER), PresenceStatus::Offline);
        assert_eq!(state.custom_status_of(OTHER), None);
    }

    #[test]
    fn the_users_own_pick_follows_their_other_devices() {
        let mut state = ready_state();
        state.apply_event(
            GatewayEvent::PresenceUpdate(presence(ME, PresenceStatus::Idle, Some("away"))),
            NOW,
        );
        assert_eq!(state.current_user().unwrap().status, ChosenStatus::Idle);
        assert_eq!(state.custom_status_of(ME), Some("away"));
        state.apply_event(
            GatewayEvent::PresenceUpdate(presence(ME, PresenceStatus::Offline, None)),
            NOW,
        );
        assert_eq!(
            state.current_user().unwrap().status,
            ChosenStatus::Invisible
        );
        assert_eq!(state.presence_of(ME), PresenceStatus::Offline);
        assert_eq!(state.custom_status_of(ME), None, "hidden while invisible");
        assert_eq!(
            state.current_user().unwrap().custom_status.as_deref(),
            Some("away"),
            "kept for when they show again"
        );
    }

    #[test]
    fn the_users_own_answer_shows_their_pick_at_once() {
        let mut state = ready_state();
        let mut user = state.current_user().unwrap().clone();
        user.status = ChosenStatus::DoNotDisturb;
        user.display_name = "Me again".into();
        let change = state.apply_current_user(user);
        assert_eq!(change, StateChange::Presence { user_id: ME });
        assert_eq!(
            state.current_user().unwrap().status,
            ChosenStatus::DoNotDisturb
        );
        assert_eq!(state.presence_of(ME), PresenceStatus::DoNotDisturb);
        assert_eq!(state.current_user().unwrap().display_name, "Me again");
    }
}
