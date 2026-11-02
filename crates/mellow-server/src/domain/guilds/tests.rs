use std::collections::HashSet;

use mellow_protocol::{
    CreateBanRequest, CreateChannelRequest, CreateInviteRequest, CreateRoleRequest, GuildInvite,
    Invite, OverwriteKind, RegisterRequest, ReorderChannelsRequest, ReorderRolesRequest,
    SetPermissionOverwriteRequest, UpdateChannelRequest, UpdateRoleRequest, UserLimitChange,
};

use super::invites::{INVITE_CODE_HINT_LENGTH, invite_code_hint};
use super::*;
use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::Accounts;
use crate::domain::accounts::tests::{NOW, migrated_database, register_request};
use crate::domain::guild_rules::{
    MAXIMUM_BAN_REASON_LENGTH, MAXIMUM_CHANNELS_PER_GUILD, MAXIMUM_OVERWRITES_PER_CHANNEL,
    MAXIMUM_ROLES_PER_GUILD,
};
use mellow_protocol::ChannelPlacement;

struct Fixture {
    _directory: tempfile::TempDir,
    database: Database,
    guilds: Guilds,
    events: Arc<EventHub>,
    owner: Snowflake,
    other: Snowflake,
    third: Snowflake,
}

async fn fixture() -> Fixture {
    let (directory, database) = migrated_database().await;
    let events = Arc::new(EventHub::default());
    let ids = Arc::new(SnowflakeGenerator::default());
    let accounts = Accounts::new(
        database.clone(),
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
        guilds: Guilds::new(
            database.clone(),
            ids,
            events.clone(),
            GuildCreationPolicy::Everyone,
        ),
        events,
        database,
        owner,
        other,
    }
}

fn create_request(name: &str) -> CreateGuildRequest {
    CreateGuildRequest {
        name: name.into(),
        initial_categories: None,
    }
}

fn text_channel(name: &str, parent_id: Option<Snowflake>) -> CreateChannelRequest {
    CreateChannelRequest {
        kind: ChannelKind::Text,
        name: name.into(),
        topic: String::new(),
        parent_id,
        user_limit: None,
    }
}

#[tokio::test]
async fn new_guild_has_a_default_category_and_general_and_the_owner_has_everything() {
    let fixture = fixture().await;
    let guild = fixture
        .guilds
        .create_guild(fixture.owner, false, create_request(" Mellow Dev "), NOW)
        .await
        .unwrap();
    fixture.database.close().await;

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
        .create_guild(fixture.owner, false, create_request("Private"), NOW)
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
    fixture.database.close().await;

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
        .create_guild(fixture.owner, false, create_request("Club"), NOW)
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
    fixture.database.close().await;

    assert!(matches!(expired, Err(GuildError::InvalidInvite)));
    assert_eq!(members.len(), 2);
    assert!(matches!(unknown, Err(GuildError::InvalidInvite)));
}

#[tokio::test]
async fn voice_channels_keep_no_topic_and_take_a_user_limit_that_null_clears() {
    let fixture = fixture().await;
    let guild = fixture
        .guilds
        .create_guild(fixture.owner, false, create_request("Club"), NOW)
        .await
        .unwrap();
    let category_id = guild.channels[0].id;
    let create = |kind, user_limit| CreateChannelRequest {
        kind,
        name: "Lounge".into(),
        topic: "ignored".into(),
        parent_id: Some(category_id),
        user_limit,
    };
    let voice = fixture
        .guilds
        .create_channel(
            fixture.owner,
            guild.id,
            create(ChannelKind::Voice, Some(5)),
            NOW,
        )
        .await
        .unwrap();
    let text_with_limit = fixture
        .guilds
        .create_channel(
            fixture.owner,
            guild.id,
            create(ChannelKind::Text, Some(5)),
            NOW,
        )
        .await
        .map(|_| ());
    let too_many = fixture
        .guilds
        .create_channel(
            fixture.owner,
            guild.id,
            create(ChannelKind::Voice, Some(26)),
            NOW,
        )
        .await
        .map(|_| ());
    let update = |user_limit| UpdateChannelRequest {
        name: None,
        topic: Some("still ignored".into()),
        user_limit,
    };
    let raised = fixture
        .guilds
        .update_channel(fixture.owner, voice.id, update(UserLimitChange::Set(25)))
        .await
        .unwrap();
    let kept = fixture
        .guilds
        .update_channel(fixture.owner, voice.id, update(UserLimitChange::Keep))
        .await
        .unwrap();
    let cleared = fixture
        .guilds
        .update_channel(fixture.owner, voice.id, update(UserLimitChange::Clear))
        .await
        .unwrap();
    let general_limited = fixture
        .guilds
        .update_channel(
            fixture.owner,
            guild.channels[1].id,
            update(UserLimitChange::Set(3)),
        )
        .await
        .map(|_| ());
    let stored = fixture
        .guilds
        .get_guild(fixture.owner, guild.id)
        .await
        .unwrap();
    fixture.database.close().await;

    assert_eq!(voice.kind, ChannelKind::Voice);
    assert_eq!(voice.topic, "");
    assert_eq!(voice.user_limit, Some(5));
    assert_eq!(voice.parent_id, Some(category_id));
    for refused in [text_with_limit, too_many, general_limited] {
        assert!(matches!(
            refused,
            Err(GuildError::InvalidInput(GuildRuleViolation::UserLimit))
        ));
    }
    assert_eq!((raised.user_limit, raised.topic.as_str()), (Some(25), ""));
    assert_eq!(kept.user_limit, Some(25));
    assert_eq!(cleared.user_limit, None);
    let stored_voice = stored
        .channels
        .iter()
        .find(|channel| channel.id == voice.id);
    assert_eq!(stored_voice.map(|channel| channel.user_limit), Some(None));
}

#[tokio::test]
async fn channels_are_created_under_categories_renamed_and_deleted() {
    let fixture = fixture().await;
    let guild = fixture
        .guilds
        .create_guild(fixture.owner, false, create_request("Club"), NOW)
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
                user_limit: mellow_protocol::UserLimitChange::Keep,
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
    fixture.database.close().await;

    assert_eq!(created.name, "Off Topic");
    assert_eq!(created.position, 2);
    assert!(matches!(
        text_as_parent,
        Err(GuildError::InvalidInput(GuildRuleViolation::Parent))
    ));
    assert_eq!(
        (renamed.name.as_str(), renamed.topic.as_str()),
        ("Random Stuff", "Anything goes")
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
        .create_guild(fixture.owner, false, create_request("Club"), NOW)
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
    fixture.database.close().await;

    assert!(matches!(owner_leaves, Err(GuildError::OwnerCannotLeave)));
    assert_eq!(my_guilds, Vec::new());
}

async fn guild_with_member(fixture: &Fixture) -> Guild {
    let guild = fixture
        .guilds
        .create_guild(fixture.owner, false, create_request("Club"), NOW)
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
        .create_guild(fixture.owner, false, create_request("Club"), NOW)
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
        .fetch_one(fixture.database.writer())
        .await
        .unwrap();
    fixture.database.close().await;
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
        user_limit: mellow_protocol::UserLimitChange::Keep,
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
    fixture.database.close().await;
}

#[tokio::test]
async fn parents_must_be_categories_of_the_same_guild_and_categories_cannot_nest() {
    let fixture = fixture().await;
    let guild = guild_with_member(&fixture).await;
    let other_guild = fixture
        .guilds
        .create_guild(fixture.owner, false, create_request("Elsewhere"), NOW)
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
                user_limit: None,
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
    fixture.database.close().await;

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
        .create_guild(fixture.owner, false, create_request("Busy"), NOW)
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
    fixture.database.close().await;
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
    fixture.database.close().await;
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
    fixture.database.close().await;
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
    fixture.database.close().await;

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
    fixture.database.close().await;

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
    fixture.database.close().await;

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
    fixture.database.close().await;

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
    let outdated = role_storage::reorder_roles(
        &fixture.database,
        guild.id,
        &seen,
        &[admin.id, helper.id, moderator.id],
        &audit,
    )
    .await
    .unwrap();
    let roles = role_storage::list_roles(&fixture.database, guild.id)
        .await
        .unwrap();
    let audit_log = audit_log_storage::list_audit_entries(
        &fixture.database,
        guild.id,
        None,
        100,
        &ChannelEntryFilter::All,
    )
    .await
    .unwrap();
    fixture.database.close().await;
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
async fn a_restricted_audience_follows_role_changes_made_outside_this_process() {
    let fixture = fixture().await;
    let ModeratedGuild { guild, helper, .. } = moderated_guild(&fixture).await;
    let general = guild.channels[1].id;
    // Only the owner and Helper members can view #general.
    for (target, request) in [
        (
            guild.id,
            overwrite_request(
                OverwriteKind::Role,
                Permissions::empty(),
                Permissions::VIEW_CHANNEL,
            ),
        ),
        (
            helper.id,
            overwrite_request(
                OverwriteKind::Role,
                Permissions::VIEW_CHANNEL,
                Permissions::empty(),
            ),
        ),
    ] {
        fixture
            .guilds
            .set_permission_overwrite(fixture.owner, general, target, request)
            .await
            .unwrap();
    }
    let channel = channel_storage::find_guild_channel(&fixture.database, general)
        .await
        .unwrap()
        .unwrap();
    let audience = || async {
        match fixture.guilds.channel_audience(&channel).await.unwrap() {
            ChannelAudience::Members(viewers) => viewers,
            ChannelAudience::EveryMember => panic!("#general is restricted"),
        }
    };
    let integer = |id: Snowflake| i64::try_from(id.raw()).unwrap();
    // Written as another process (`admin`) would, so only the database knows.
    let write = |statement: &'static str, ids: &[Snowflake]| {
        let mut query = sqlx::query(statement);
        for id in ids {
            query = query.bind(integer(*id));
        }
        query.execute(fixture.database.writer())
    };

    let before = audience().await;
    write(
        "INSERT INTO member_roles (guild_id, user_id, role_id) VALUES (?, ?, ?)",
        &[guild.id, fixture.third, helper.id],
    )
    .await
    .unwrap();
    let after_assignment = audience().await;
    write(
        "DELETE FROM roles WHERE guild_id = ? AND id = ?",
        &[guild.id, helper.id],
    )
    .await
    .unwrap();
    let after_role_deletion = audience().await;
    fixture.database.close().await;

    assert_eq!(before, HashSet::from([fixture.owner]));
    assert_eq!(
        after_assignment,
        HashSet::from([fixture.owner, fixture.third])
    );
    assert_eq!(after_role_deletion, HashSet::from([fixture.owner]));
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
    fixture.database.close().await;

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
    fixture.database.close().await;

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

#[tokio::test]
async fn a_guild_starts_with_the_requested_categories_and_channels_in_order() {
    let fixture = fixture().await;
    let request = CreateGuildRequest {
        name: "Club".into(),
        initial_categories: Some(vec![
            InitialCategory {
                name: "Infos".into(),
                channels: vec![InitialTextChannel {
                    name: "Bienvenue".into(),
                    topic: "Dis bonjour".into(),
                }],
            },
            InitialCategory {
                name: "Discussions".into(),
                channels: vec![
                    InitialTextChannel {
                        name: "Général".into(),
                        topic: String::new(),
                    },
                    InitialTextChannel {
                        name: "Jeux vidéo".into(),
                        topic: String::new(),
                    },
                ],
            },
        ]),
    };
    let guild = fixture
        .guilds
        .create_guild(fixture.owner, false, request, NOW)
        .await
        .unwrap();
    let too_many = fixture
        .guilds
        .create_guild(
            fixture.owner,
            false,
            CreateGuildRequest {
                name: "Club".into(),
                initial_categories: Some(vec![
                    InitialCategory {
                        name: "a".into(),
                        channels: Vec::new(),
                    };
                    11
                ]),
            },
            NOW,
        )
        .await;
    fixture.database.close().await;

    let layout: Vec<(&str, Option<&str>, u32)> = guild
        .channels
        .iter()
        .map(|channel| {
            let parent = channel.parent_id.map(|parent_id| {
                guild
                    .channels
                    .iter()
                    .find(|candidate| candidate.id == parent_id)
                    .unwrap()
                    .name
                    .as_str()
            });
            (channel.name.as_str(), parent, channel.position)
        })
        .collect();
    assert_eq!(
        layout,
        [
            ("Infos", None, 0),
            ("Bienvenue", Some("Infos"), 1),
            ("Discussions", None, 2),
            ("Général", Some("Discussions"), 3),
            ("Jeux vidéo", Some("Discussions"), 4),
        ]
    );
    assert_eq!(guild.channels[1].topic, "Dis bonjour");
    assert!(matches!(
        too_many,
        Err(GuildError::InvalidInput(
            GuildRuleViolation::InitialChannels
        ))
    ));
}

#[tokio::test]
async fn channels_are_reordered_and_each_member_gets_their_own_snapshot() {
    let fixture = fixture().await;
    let guild = guild_with_member(&fixture).await;
    let [text_channels, general] = [guild.channels[0].id, guild.channels[1].id];
    let category = |name: &str| CreateChannelRequest {
        kind: ChannelKind::Category,
        name: name.into(),
        topic: String::new(),
        parent_id: None,
        user_limit: None,
    };
    let second = fixture
        .guilds
        .create_channel(fixture.owner, guild.id, category("Second"), NOW)
        .await
        .unwrap()
        .id;
    let secret = fixture
        .guilds
        .create_channel(
            fixture.owner,
            guild.id,
            text_channel("secret", Some(text_channels)),
            NOW,
        )
        .await
        .unwrap()
        .id;
    fixture
        .guilds
        .set_permission_overwrite(fixture.owner, secret, guild.id, hide_from_everyone())
        .await
        .unwrap();
    let [mut owner_events, mut other_events] = owner_and_other_sessions(&fixture, guild.id);
    let placement = |id, parent_id| ChannelPlacement { id, parent_id };
    let new_order = vec![
        placement(second, None),
        placement(secret, Some(second)),
        placement(general, Some(second)),
        placement(text_channels, None),
    ];

    let reorder = |user_id, channels: &[ChannelPlacement]| {
        fixture.guilds.reorder_channels(
            user_id,
            guild.id,
            ReorderChannelsRequest {
                channels: channels.to_vec(),
            },
        )
    };
    let member_reorders = reorder(fixture.other, &new_order).await;
    let incomplete = reorder(fixture.owner, &new_order[..3]).await;
    let reordered = reorder(fixture.owner, &new_order).await.unwrap();
    let seen_by_other = fixture.guilds.get_guild(fixture.other, guild.id).await;
    fixture.database.close().await;

    assert!(matches!(
        member_reorders,
        Err(GuildError::MissingPermissions)
    ));
    assert!(matches!(
        incomplete,
        Err(GuildError::InvalidInput(GuildRuleViolation::ChannelOrder))
    ));
    let placements: Vec<ChannelPlacement> = reordered
        .iter()
        .map(|channel| placement(channel.id, channel.parent_id))
        .collect();
    assert_eq!(placements, new_order);
    assert_eq!(
        reordered
            .iter()
            .map(|channel| channel.position)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    let other_ids: Vec<Snowflake> = seen_by_other
        .as_ref()
        .unwrap()
        .channels
        .iter()
        .map(|channel| channel.id)
        .collect();
    assert_eq!(other_ids, [second, general, text_channels]);
    let seen_by_other = seen_by_other.unwrap();
    assert_eq!(received_guilds(&mut other_events), [seen_by_other]);
    assert_eq!(event_types(&mut owner_events), ["GUILD_CREATE"]);
}

/// `other` manages channels everywhere but in `general`. Returns the guild and the ids of
/// "Text channels", `general` and a second, empty category, in that order.
async fn guild_where_other_manages_channels_but_general(
    fixture: &Fixture,
) -> (Guild, [Snowflake; 3]) {
    let guild = guild_with_member(fixture).await;
    let [text_channels, general] = [guild.channels[0].id, guild.channels[1].id];
    let guilds = &fixture.guilds;
    guilds
        .update_role(
            fixture.owner,
            guild.id,
            guild.id,
            UpdateRoleRequest {
                permissions: Some(Permissions::DEFAULT_EVERYONE | Permissions::MANAGE_CHANNELS),
                ..UpdateRoleRequest::default()
            },
        )
        .await
        .unwrap();
    let second = guilds
        .create_channel(
            fixture.owner,
            guild.id,
            CreateChannelRequest {
                kind: ChannelKind::Category,
                name: "Second".into(),
                topic: String::new(),
                parent_id: None,
                user_limit: None,
            },
            NOW,
        )
        .await
        .unwrap()
        .id;
    guilds
        .set_permission_overwrite(
            fixture.owner,
            general,
            fixture.other,
            overwrite_request(
                OverwriteKind::Member,
                Permissions::empty(),
                Permissions::MANAGE_CHANNELS,
            ),
        )
        .await
        .unwrap();
    (guild, [text_channels, general, second])
}

#[tokio::test]
async fn moving_a_channel_to_another_category_needs_manage_channels_in_it() {
    let fixture = fixture().await;
    let (guild, [text_channels, general, second]) =
        guild_where_other_manages_channels_but_general(&fixture).await;
    let guilds = &fixture.guilds;
    let placement = |id, parent_id| ChannelPlacement { id, parent_id };
    let reorder = |channels: Vec<ChannelPlacement>| {
        guilds.reorder_channels(fixture.other, guild.id, ReorderChannelsRequest { channels })
    };

    let moved_out = reorder(vec![
        placement(text_channels, None),
        placement(second, None),
        placement(general, Some(second)),
    ])
    .await;
    let categories_swapped = reorder(vec![
        placement(second, None),
        placement(text_channels, None),
        placement(general, Some(text_channels)),
    ])
    .await;
    let audit = guilds
        .audit_entry(
            guild.id,
            fixture.owner,
            AuditAction::ChannelsReorder,
            None,
            None,
        )
        .unwrap();
    // Someone else reordered after the check read the original order.
    let outdated = channel_storage::reorder_channels(
        &fixture.database,
        guild.id,
        &[
            (text_channels, None),
            (general, Some(text_channels)),
            (second, None),
        ],
        &[(general, None), (text_channels, None), (second, None)],
        &[audit],
    )
    .await
    .unwrap();
    let after = guilds.get_guild(fixture.owner, guild.id).await.unwrap();
    let log = guilds
        .list_audit_log(fixture.owner, guild.id, AuditLogQuery::default())
        .await;
    fixture.database.close().await;

    assert!(matches!(moved_out, Err(GuildError::MissingPermissions)));
    assert!(categories_swapped.is_ok(), "{categories_swapped:?}");
    assert!(!outdated);
    let placements: Vec<ChannelPlacement> = after
        .channels
        .iter()
        .map(|channel| placement(channel.id, channel.parent_id))
        .collect();
    assert_eq!(
        placements,
        [
            placement(second, None),
            placement(text_channels, None),
            placement(general, Some(text_channels)),
        ]
    );
    let reorders = log
        .unwrap()
        .iter()
        .filter(|entry| entry.action == AuditAction::ChannelsReorder)
        .count();
    assert_eq!(reorders, 1);
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

/// The guild of each `GUILD_CREATE` received, in order.
fn received_guilds(
    events: &mut tokio::sync::mpsc::Receiver<crate::events::SharedEvent>,
) -> Vec<Guild> {
    let mut guilds = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let GatewayEvent::GuildCreate(guild) = serde_json::from_str(&event.payload_json).unwrap()
        {
            guilds.push(*guild);
        }
    }
    guilds
}

#[tokio::test]
async fn a_resynchronized_member_receives_exactly_what_get_guild_answers_them() {
    let fixture = fixture().await;
    let guild = guild_with_member(&fixture).await;
    let general = guild.channels[1].id;
    let guilds = &fixture.guilds;
    let viewers = guilds
        .create_role(
            fixture.owner,
            guild.id,
            role_request("Viewers", Permissions::empty()),
            NOW,
        )
        .await
        .unwrap();
    guilds
        .add_member_role(fixture.owner, guild.id, fixture.other, viewers.id)
        .await
        .unwrap();
    // #general is hidden from everyone but Viewers; the other channels stay public.
    for (target, request) in [
        (guild.id, hide_from_everyone()),
        (
            viewers.id,
            overwrite_request(
                OverwriteKind::Role,
                Permissions::VIEW_CHANNEL,
                Permissions::empty(),
            ),
        ),
    ] {
        guilds
            .set_permission_overwrite(fixture.owner, general, target, request)
            .await
            .unwrap();
    }
    let [mut owner_events, mut other_events] = owner_and_other_sessions(&fixture, guild.id);

    // Changing a role's permissions resynchronizes every online member.
    guilds
        .update_role(
            fixture.owner,
            guild.id,
            viewers.id,
            UpdateRoleRequest {
                name: None,
                color: None,
                permissions: Some(Permissions::ADD_REACTIONS),
            },
        )
        .await
        .unwrap();
    let received = [
        received_guilds(&mut owner_events),
        received_guilds(&mut other_events),
    ];
    let answered = [
        guilds.get_guild(fixture.owner, guild.id).await.unwrap(),
        guilds.get_guild(fixture.other, guild.id).await.unwrap(),
    ];
    fixture.database.close().await;

    for (received, answered) in received.into_iter().zip(answered) {
        assert_eq!(received, [answered]);
    }
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
    fixture.database.close().await;

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
    fixture.database.close().await;
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
    fixture.database.close().await;
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
    fixture.database.close().await;
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
    fixture.database.close().await;
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
    fixture.database.close().await;
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
    fixture.database.close().await;

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
    fixture.database.close().await;

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
    fixture.database.close().await;
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
        .create_guild(fixture.owner, false, create_request("Club"), NOW)
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
    fixture.database.close().await;
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
    fixture.database.close().await;

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
            AuditAction::InviteCreate,
            AuditAction::InviteCreate,
        ],
        "newest first; joins and repeated grants are not moderation, invites are"
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
    fixture.database.close().await;

    assert_eq!(newest.len(), 1);
    assert_eq!(older.len(), 3, "two more channels and the invite");
    assert!(older.iter().all(|entry| entry.id < newest[0].id));
    assert_eq!(beyond_every_id.len(), 4);
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
        user_limit: mellow_protocol::UserLimitChange::Keep,
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
    fixture.database.close().await;
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
                user_limit: mellow_protocol::UserLimitChange::Keep,
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
    fixture.database.close().await;

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
    fixture.database.close().await;

    let about = |entries: &[AuditLogEntry], channel_id: Snowflake| {
        entries.iter().any(|entry| {
            entry.target_id == Some(channel_id) || entry.context_id == Some(channel_id)
        })
    };
    assert!(!about(&seen_by_auditor, staff.id), "hidden channel");
    assert!(!about(&seen_by_auditor, gone.id), "deleted channel");
    assert_eq!(
        actions(&seen_by_auditor),
        [
            AuditAction::MemberRoleAdd,
            AuditAction::RoleCreate,
            AuditAction::InviteCreate,
        ]
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
    fixture.database.close().await;
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
        .create_guild(fixture.other, false, create_request("Elsewhere"), NOW)
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
    fixture.database.close().await;
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
    fixture.database.close().await;
    assert!(matches!(one_too_many, Err(GuildError::LimitReached(_))));
}

#[tokio::test]
async fn managers_rename_a_guild_and_every_member_hears_it() {
    let fixture = fixture().await;
    let guild = guild_with_member(&fixture).await;
    let [mut owner_events, mut other_events] = owner_and_other_sessions(&fixture, guild.id);
    let rename = |name: &str| UpdateGuildRequest {
        name: Some(name.into()),
    };
    let refused = [
        fixture
            .guilds
            .update_guild(fixture.other, guild.id, rename("Mine"))
            .await,
        fixture
            .guilds
            .update_guild(fixture.third, guild.id, rename("Mine"))
            .await,
        fixture
            .guilds
            .update_guild(fixture.owner, guild.id, rename("  "))
            .await,
    ];
    let renamed = fixture
        .guilds
        .update_guild(fixture.owner, guild.id, rename("  Book club "))
        .await
        .unwrap();
    let unchanged = fixture
        .guilds
        .update_guild(fixture.owner, guild.id, rename("Book club"))
        .await
        .unwrap();
    let entries = fixture
        .guilds
        .list_audit_log(fixture.owner, guild.id, AuditLogQuery::default())
        .await
        .unwrap();
    let other_sees = fixture.guilds.list_my_guilds(fixture.other).await.unwrap();
    fixture.database.close().await;
    assert!(matches!(refused[0], Err(GuildError::MissingPermissions)));
    assert!(matches!(refused[1], Err(GuildError::NotFound)));
    assert!(matches!(refused[2], Err(GuildError::InvalidInput(_))));
    assert_eq!(renamed.name, "Book club");
    assert_eq!(unchanged.name, "Book club");
    assert_eq!(other_sees[0].name, "Book club");
    assert_eq!(event_types(&mut owner_events), ["GUILD_UPDATE"]);
    assert_eq!(event_types(&mut other_events), ["GUILD_UPDATE"]);
    let guild_updates: Vec<&AuditLogEntry> = entries
        .iter()
        .filter(|entry| entry.action == AuditAction::GuildUpdate)
        .collect();
    assert_eq!(guild_updates.len(), 1, "an unchanged name records nothing");
    assert_eq!(
        guild_updates[0].changes,
        [AuditChange {
            key: AuditChangeKey::Name,
            old_value: "Club".into(),
            new_value: "Book club".into(),
        }]
    );
    assert_eq!(guild_updates[0].actor_id, Some(fixture.owner));
}

#[tokio::test]
async fn only_the_owner_deletes_a_guild_and_every_member_sees_it_go() {
    let fixture = fixture().await;
    let guild = guild_with_member(&fixture).await;
    let administrator = fixture
        .guilds
        .create_role(
            fixture.owner,
            guild.id,
            role_request("admin", Permissions::ADMINISTRATOR),
            NOW,
        )
        .await
        .unwrap();
    fixture
        .guilds
        .add_member_role(fixture.owner, guild.id, fixture.other, administrator.id)
        .await
        .unwrap();
    let [mut owner_events, mut other_events] = owner_and_other_sessions(&fixture, guild.id);
    let by_administrator = fixture.guilds.delete_guild(fixture.other, guild.id).await;
    let by_outsider = fixture.guilds.delete_guild(fixture.third, guild.id).await;
    let by_owner = fixture.guilds.delete_guild(fixture.owner, guild.id).await;
    let again = fixture.guilds.delete_guild(fixture.owner, guild.id).await;
    let snapshot = fixture.guilds.get_guild(fixture.owner, guild.id).await;
    let other_guilds = fixture.guilds.list_my_guilds(fixture.other).await.unwrap();
    fixture.database.close().await;
    assert!(matches!(
        by_administrator,
        Err(GuildError::MissingPermissions)
    ));
    assert!(matches!(by_outsider, Err(GuildError::NotFound)));
    assert!(by_owner.is_ok());
    assert!(matches!(again, Err(GuildError::NotFound)));
    assert!(matches!(snapshot, Err(GuildError::NotFound)));
    assert_eq!(other_guilds, []);
    assert_eq!(event_types(&mut owner_events), ["GUILD_DELETE"]);
    assert_eq!(event_types(&mut other_events), ["GUILD_DELETE"]);
    // Unsubscribed: nothing about the guild reaches them any more.
    fixture
        .events
        .publish_to_guild(guild.id, &GatewayEvent::GuildDelete { id: guild.id });
    assert_eq!(event_types(&mut other_events), Vec::<String>::new());
}

const SECOND: u64 = 1_000;

/// The member joined with the first invite; the second lasts a minute from `NOW + SECOND`.
async fn guild_with_expiring_invite(fixture: &Fixture) -> (Guild, Invite) {
    let guild = guild_with_member(fixture).await;
    let expiring = fixture
        .guilds
        .create_invite(
            fixture.owner,
            guild.id,
            CreateInviteRequest {
                max_uses: Some(5),
                expires_in_seconds: Some(60),
            },
            NOW + SECOND,
        )
        .await
        .unwrap();
    (guild, expiring)
}

fn invite_codes(invites: &[GuildInvite]) -> Vec<String> {
    invites.iter().map(|invite| invite.code.clone()).collect()
}

#[tokio::test]
async fn managers_list_usable_invites_newest_first() {
    let fixture = fixture().await;
    let (guild, expiring) = guild_with_expiring_invite(&fixture).await;
    let list = |user_id, now| fixture.guilds.list_invites(user_id, guild.id, now);
    let listed_now = list(fixture.owner, NOW + SECOND).await.unwrap();
    let listed_later = list(fixture.owner, NOW + 120 * SECOND).await.unwrap();
    let by_member = list(fixture.other, NOW).await;
    let by_outsider = list(fixture.third, NOW).await;
    fixture.database.close().await;

    assert_eq!(listed_now.len(), 2);
    assert_eq!(listed_now[0].code, expiring.code, "newest first");
    assert_eq!(listed_now[0].max_uses, Some(5));
    assert_eq!(listed_now[1].uses, 1);
    assert_eq!(listed_now[1].creator.username, "owner");
    assert_eq!(
        invite_codes(&listed_later),
        [listed_now[1].code.clone()],
        "expired invites are left out"
    );
    assert!(matches!(by_member, Err(GuildError::MissingPermissions)));
    assert!(matches!(by_outsider, Err(GuildError::NotFound)));
}

#[tokio::test]
async fn managers_revoke_invites_and_creating_and_revoking_are_audited() {
    let fixture = fixture().await;
    let (guild, expiring) = guild_with_expiring_invite(&fixture).await;
    let revoke = |user_id, code| fixture.guilds.delete_invite(user_id, code);
    let by_member = revoke(fixture.other, &expiring.code).await;
    let by_outsider = revoke(fixture.third, &expiring.code).await;
    let unknown = revoke(fixture.owner, "unknown").await;
    revoke(fixture.owner, &expiring.code).await.unwrap();
    let joined_with_revoked = fixture
        .guilds
        .join_with_invite(fixture.third, &expiring.code, NOW + SECOND)
        .await;
    let after_revoking = fixture
        .guilds
        .list_invites(fixture.owner, guild.id, NOW + SECOND)
        .await
        .unwrap();
    let entries = fixture
        .guilds
        .list_audit_log(fixture.owner, guild.id, AuditLogQuery::default())
        .await
        .unwrap();
    fixture.database.close().await;

    assert!(matches!(by_member, Err(GuildError::MissingPermissions)));
    assert!(matches!(by_outsider, Err(GuildError::NotFound)));
    assert!(matches!(unknown, Err(GuildError::NotFound)));
    assert!(matches!(
        joined_with_revoked,
        Err(GuildError::InvalidInvite)
    ));
    assert!(!invite_codes(&after_revoking).contains(&expiring.code));
    assert_eq!(
        actions(&entries),
        [
            AuditAction::InviteDelete,
            AuditAction::InviteCreate,
            AuditAction::InviteCreate,
        ]
    );
    let hint = invite_code_hint(&expiring.code);
    assert_eq!(entries[0].changes[0].key, AuditChangeKey::Code);
    assert_eq!(entries[0].changes[0].old_value, hint);
    assert_eq!(entries[1].changes[0].new_value, hint);
    assert_eq!(entries[0].target_id, None);
}

#[tokio::test]
async fn auditors_never_read_a_usable_invite_code() {
    let fixture = fixture().await;
    let guild = guild_with_member(&fixture).await;
    let guilds = &fixture.guilds;
    let auditors = guilds
        .create_role(
            fixture.owner,
            guild.id,
            role_request("Auditors", Permissions::VIEW_AUDIT_LOG),
            NOW,
        )
        .await
        .unwrap();
    guilds
        .add_member_role(fixture.owner, guild.id, fixture.other, auditors.id)
        .await
        .unwrap();
    let invite = guilds
        .create_invite(fixture.owner, guild.id, CreateInviteRequest::default(), NOW)
        .await
        .unwrap();
    let seen_by_auditor = guilds
        .list_audit_log(fixture.other, guild.id, AuditLogQuery::default())
        .await
        .unwrap();
    fixture.database.close().await;

    let created = seen_by_auditor
        .iter()
        .find(|entry| entry.action == AuditAction::InviteCreate)
        .unwrap();
    let prefix: String = invite.code.chars().take(INVITE_CODE_HINT_LENGTH).collect();
    assert_eq!(created.changes[0].new_value, format!("{prefix}…"));
    assert!(
        seen_by_auditor
            .iter()
            .flat_map(|entry| &entry.changes)
            .all(|change| !change.new_value.contains(&invite.code)
                && !change.old_value.contains(&invite.code)),
        "no entry holds a whole code"
    );
}

#[tokio::test]
async fn each_rename_records_the_name_it_replaced() {
    let fixture = fixture().await;
    let guild = guild_with_member(&fixture).await;
    let audit = || {
        fixture
            .guilds
            .audit_entry(
                guild.id,
                fixture.owner,
                AuditAction::GuildUpdate,
                None,
                None,
            )
            .unwrap()
    };
    let rename = |name: &'static str, audit: NewAuditEntry| {
        let database = fixture.database.clone();
        async move { guild_storage::update_guild_name(&database, guild.id, name, &audit).await }
    };
    let first = rename("Book club", audit()).await.unwrap();
    let second = rename("Reading room", audit()).await.unwrap();
    let unchanged = rename("Reading room", audit()).await.unwrap();
    let missing = guild_storage::update_guild_name(
        &fixture.database,
        Snowflake::from_raw(42),
        "Nowhere",
        &audit(),
    )
    .await
    .unwrap();
    let entries = fixture
        .guilds
        .list_audit_log(fixture.owner, guild.id, AuditLogQuery::default())
        .await
        .unwrap();
    fixture.database.close().await;

    assert_eq!(
        [first, second, unchanged, missing],
        [
            GuildRename::Renamed { icon: None },
            GuildRename::Renamed { icon: None },
            GuildRename::Unchanged,
            GuildRename::NotFound,
        ]
    );
    let names: Vec<(&str, &str)> = entries
        .iter()
        .filter(|entry| entry.action == AuditAction::GuildUpdate)
        .map(|entry| {
            (
                entry.changes[0].old_value.as_str(),
                entry.changes[0].new_value.as_str(),
            )
        })
        .collect();
    assert_eq!(
        names,
        [("Book club", "Reading room"), ("Club", "Book club")]
    );
}

#[tokio::test]
async fn deleting_a_guild_removes_what_it_holds_and_keeps_conversations() {
    let fixture = fixture().await;
    let guild = guild_with_member(&fixture).await;
    let general = guild.channels[1].id;
    fixture
        .guilds
        .ban_member(fixture.owner, guild.id, fixture.third, ban_request(""), NOW)
        .await
        .unwrap();
    let (conversation, _) = crate::storage::direct_messages::find_or_create_one_to_one(
        &fixture.database,
        fixture.owner,
        fixture.other,
        Snowflake::from_raw(900),
    )
    .await
    .unwrap();
    let database = &fixture.database;
    for (message_id, channel_id) in [(901, general), (902, conversation)] {
        sqlx::query(
            "INSERT INTO messages (id, channel_id, author_id, content) VALUES (?, ?, ?, 'hi')",
        )
        .bind(message_id)
        .bind(i64::try_from(channel_id.raw()).unwrap())
        .bind(i64::try_from(fixture.owner.raw()).unwrap())
        .execute(database.writer())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO read_states (user_id, channel_id, last_read_message_id) VALUES (?, ?, ?)",
        )
        .bind(i64::try_from(fixture.other.raw()).unwrap())
        .bind(i64::try_from(channel_id.raw()).unwrap())
        .bind(message_id)
        .execute(database.writer())
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO attachments (id, channel_id, uploader_id, message_id, file_name, \
         content_type, size_bytes, created_at_unix_milliseconds) \
         VALUES (903, ?, ?, 901, 'notes.txt', 'application/octet-stream', 5, 0)",
    )
    .bind(i64::try_from(general.raw()).unwrap())
    .bind(i64::try_from(fixture.owner.raw()).unwrap())
    .execute(database.writer())
    .await
    .unwrap();

    let member_ids = guild_storage::delete_guild(database, guild.id, fixture.owner)
        .await
        .unwrap();
    let guild_id = i64::try_from(guild.id.raw()).unwrap();
    let mut left_in_guild = Vec::new();
    for query in [
        "SELECT COUNT(*) FROM guilds WHERE id = ?1",
        "SELECT COUNT(*) FROM roles WHERE guild_id = ?1",
        "SELECT COUNT(*) FROM guild_members WHERE guild_id = ?1",
        "SELECT COUNT(*) FROM channels WHERE guild_id = ?1",
        "SELECT COUNT(*) FROM invites WHERE guild_id = ?1",
        "SELECT COUNT(*) FROM bans WHERE guild_id = ?1",
        "SELECT COUNT(*) FROM audit_log WHERE guild_id = ?1",
        "SELECT COUNT(*) FROM messages WHERE id = 901",
        "SELECT COUNT(*) FROM attachments WHERE id = 903",
        "SELECT COUNT(*) FROM read_states WHERE last_read_message_id = 901",
    ] {
        let count: i64 = sqlx::query_scalar(query)
            .bind(guild_id)
            .fetch_one(database.writer())
            .await
            .unwrap();
        left_in_guild.push(count);
    }
    let mut kept_in_conversation = Vec::new();
    for query in [
        "SELECT COUNT(*) FROM channels WHERE id = ?1",
        "SELECT COUNT(*) FROM messages WHERE id = 902",
        "SELECT COUNT(*) FROM read_states WHERE last_read_message_id = 902",
        "SELECT COUNT(*) FROM direct_message_participants WHERE channel_id = ?1",
    ] {
        let count: i64 = sqlx::query_scalar(query)
            .bind(i64::try_from(conversation.raw()).unwrap())
            .fetch_one(database.writer())
            .await
            .unwrap();
        kept_in_conversation.push(count);
    }
    fixture.database.close().await;

    let mut member_ids = member_ids.unwrap();
    member_ids.sort_unstable();
    let mut expected = vec![fixture.owner, fixture.other];
    expected.sort_unstable();
    assert_eq!(member_ids, expected);
    assert_eq!(left_in_guild, [0; 10]);
    assert_eq!(kept_in_conversation, [1, 1, 1, 2]);
}
