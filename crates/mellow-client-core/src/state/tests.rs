use mellow_protocol::{
    Channel, ChannelKind, GuildSummary, Message, OverwriteKind, PermissionOverwrite,
    PresenceStatus, PublicUser, ReadState, Role,
};

use super::*;

pub(super) const GUILD: Snowflake = Snowflake::from_raw(100);
pub(super) const CATEGORY: Snowflake = Snowflake::from_raw(101);
pub(super) const GENERAL: Snowflake = Snowflake::from_raw(102);
pub(super) const ME: Snowflake = Snowflake::from_raw(1);
pub(super) const OTHER: Snowflake = Snowflake::from_raw(2);
pub(super) const NOW: u64 = 1_000_000;

pub(super) fn channel(
    id: Snowflake,
    kind: ChannelKind,
    parent_id: Option<Snowflake>,
    position: u32,
) -> Channel {
    Channel {
        icon: None,
        id,
        guild_id: GUILD,
        kind,
        name: format!("c{}", id.raw()),
        topic: String::new(),
        parent_id,
        position,
        permission_overwrites: Vec::new(),
        user_limit: None,
    }
}

pub(super) fn user(id: Snowflake) -> PublicUser {
    PublicUser {
        id,
        username: format!("user{}", id.raw()),
        display_name: format!("User {}", id.raw()),
        avatar: None,
        banner: None,
    }
}

pub(super) fn message(id: u64, author: Snowflake) -> Message {
    Message {
        id: Snowflake::from_raw(id),
        channel_id: GENERAL,
        author: user(author),
        content: format!("message {id}"),
        reply_to_id: None,
        edited_at_unix_milliseconds: None,
        nonce: None,
        attachments: Vec::new(),
        reactions: Vec::new(),
        pinned: false,
        mention_ids: Vec::new(),
        mention_role_ids: Vec::new(),
        link_previews: Vec::new(),
        poll: None,
        checklist: None,
        progress_bar: None,
        forwarded_from: None,
    }
}

pub(super) fn ready_state() -> ClientState {
    let mut state = ClientState::default();
    state.apply_ready(Ready {
        may_create_guilds: true,
        session_id: "s".into(),
        user: CurrentUser {
            id: ME,
            username: "me".into(),
            display_name: "Me".into(),
            email: "me@example.org".into(),
            is_instance_administrator: false,
            avatar: None,
            banner: None,
            status: mellow_protocol::ChosenStatus::Online,
            custom_status: None,
            friend_code: None,
        },
        guilds: vec![Guild {
            icon: None,
            emojis: Vec::new(),
            sounds: Vec::new(),
            my_join_sound_id: None,
            id: GUILD,
            name: "Club".into(),
            owner_id: ME,
            my_permissions: Permissions::all(),
            my_role_ids: Vec::new(),
            roles: Vec::new(),
            channels: vec![
                channel(CATEGORY, ChannelKind::Category, None, 0),
                channel(GENERAL, ChannelKind::Text, Some(CATEGORY), 1),
            ],
        }],
        read_states: vec![ReadState {
            channel_id: GENERAL,
            last_read_message_id: Snowflake::from_raw(10),
        }],
        online_user_ids: vec![ME],
        direct_message_channels: Vec::new(),
        channel_unreads: Vec::new(),
        presences: Vec::new(),
        relationships: Vec::new(),
        voice_states: Vec::new(),
        voice_enabled: false,
        maximum_attachment_bytes: None,
    });
    state
}

#[test]
fn messages_stay_sorted_deduplicated_and_capped() {
    let mut state = ready_state();
    state.insert_messages(GENERAL, vec![message(30, OTHER), message(10, OTHER)]);
    state.apply_event(GatewayEvent::MessageCreate(message(20, OTHER)), NOW);
    state.apply_event(GatewayEvent::MessageCreate(message(20, OTHER)), NOW);
    let ids: Vec<u64> = state
        .messages(GENERAL)
        .iter()
        .map(|message| message.id.raw())
        .collect();
    assert_eq!(ids, [10, 20, 30]);

    let many = (1..=(MAXIMUM_CACHED_MESSAGES_PER_CHANNEL as u64 + 10))
        .map(|id| message(1_000 + id, OTHER))
        .collect();
    state.insert_messages(GENERAL, many);
    assert_eq!(
        state.messages(GENERAL).len(),
        MAXIMUM_CACHED_MESSAGES_PER_CHANNEL
    );
    assert_eq!(
        state.messages(GENERAL).last().unwrap().id.raw(),
        1_000 + MAXIMUM_CACHED_MESSAGES_PER_CHANNEL as u64 + 10,
        "the newest are kept"
    );
}

#[test]
fn history_pages_extend_the_open_channel_until_its_start() {
    let page_size = u64::from(HISTORY_PAGE_SIZE);
    let page = |newest: u64| {
        (newest + 1 - page_size..=newest)
            .map(|id| message(id, OTHER))
            .collect::<Vec<_>>()
    };
    let mut state = ready_state();
    assert!(state.has_older_history(GENERAL), "never loaded yet");

    // Full pages, newest first, until the channel holds more than the cap.
    let mut newest = 100_000;
    while state.messages(GENERAL).len() <= MAXIMUM_CACHED_MESSAGES_PER_CHANNEL {
        state.insert_history_page(GENERAL, page(newest));
        newest -= page_size;
        assert!(
            state.has_older_history(GENERAL),
            "a full page may not be the last"
        );
    }
    let loaded = state.messages(GENERAL).len();
    state.apply_event(GatewayEvent::MessageCreate(message(200_000, OTHER)), NOW);
    assert_eq!(
        state.messages(GENERAL).len(),
        loaded + 1,
        "a new message never drops the older pages being read"
    );

    state.insert_history_page(GENERAL, vec![message(1, OTHER), message(2, OTHER)]);
    assert!(
        !state.has_older_history(GENERAL),
        "a short page is the last"
    );
    assert_eq!(state.messages(GENERAL).first().unwrap().id.raw(), 1);

    state.trim_history(GENERAL);
    assert_eq!(
        state.messages(GENERAL).len(),
        MAXIMUM_CACHED_MESSAGES_PER_CHANNEL
    );
    assert_eq!(state.messages(GENERAL).last().unwrap().id.raw(), 200_000);
    assert!(
        state.has_older_history(GENERAL),
        "trimmed messages must be fetched again"
    );
}

#[test]
fn an_open_channel_at_exactly_the_cap_keeps_its_older_pages() {
    let page_size = u64::from(HISTORY_PAGE_SIZE);
    let mut state = ready_state();
    let cap = MAXIMUM_CACHED_MESSAGES_PER_CHANNEL as u64;
    for newest in (page_size..=cap).rev().step_by(HISTORY_PAGE_SIZE as usize) {
        let page = (newest + 1 - page_size..=newest)
            .map(|id| message(id, OTHER))
            .collect();
        state.insert_history_page(GENERAL, page);
    }
    assert_eq!(
        state.messages(GENERAL).len(),
        MAXIMUM_CACHED_MESSAGES_PER_CHANNEL
    );
    state.apply_event(GatewayEvent::MessageCreate(message(cap + 1, OTHER)), NOW);
    assert_eq!(
        state.messages(GENERAL).len(),
        MAXIMUM_CACHED_MESSAGES_PER_CHANNEL + 1,
        "the oldest row the reader may be looking at stays"
    );
}

#[test]
fn a_short_latest_page_holds_the_whole_channel() {
    let mut state = ready_state();
    state.insert_history_page(GENERAL, vec![message(10, OTHER)]);
    assert!(!state.has_older_history(GENERAL));
    state.apply_event(GatewayEvent::MessageCreate(message(20, OTHER)), NOW);
    assert!(
        !state.has_older_history(GENERAL),
        "new messages keep the start"
    );
    state.trim_history(GENERAL);
    assert!(!state.has_older_history(GENERAL), "nothing was trimmed");
}

#[test]
fn a_message_update_keeps_the_cached_reactions_and_takes_the_pin() {
    let mut state = ready_state();
    let mut reacted = message(11, OTHER);
    reacted.reactions = vec![mellow_protocol::ReactionSummary {
        emoji: "👍".into(),
        count: 2,
        me: true,
    }];
    state.apply_event(GatewayEvent::MessageCreate(reacted.clone()), NOW);
    let mut pinned = message(11, OTHER);
    pinned.pinned = true;
    state.apply_event(GatewayEvent::MessageUpdate(pinned), NOW);
    assert_eq!(state.messages(GENERAL)[0].reactions, reacted.reactions);
    assert!(state.messages(GENERAL)[0].pinned);
}

#[test]
fn edits_deletes_and_unread_markers() {
    let mut state = ready_state();
    state.apply_event(GatewayEvent::MessageCreate(message(11, OTHER)), NOW);
    assert!(state.has_unread(GENERAL));
    state.mark_read(GENERAL, Snowflake::from_raw(11));
    state.mark_read(GENERAL, Snowflake::from_raw(5));
    assert!(!state.has_unread(GENERAL), "read position never moves back");

    let mut edited = message(11, OTHER);
    edited.content = "edited".into();
    edited.edited_at_unix_milliseconds = Some(NOW);
    let mut edited_later = edited.clone();
    edited_later.content = "edited later".into();
    edited_later.edited_at_unix_milliseconds = Some(NOW + 1);
    state.apply_event(GatewayEvent::MessageUpdate(edited_later), NOW);
    assert_eq!(
        state.apply_event(GatewayEvent::MessageUpdate(edited), NOW),
        StateChange::Nothing,
        "a late answer for an older edit is ignored"
    );
    assert_eq!(state.messages(GENERAL)[0].content, "edited later");
    let mut not_cached = message(3, OTHER);
    not_cached.channel_id = Snowflake::from_raw(999);
    assert_eq!(
        state.apply_event(GatewayEvent::MessageUpdate(not_cached), NOW),
        StateChange::Nothing
    );
    assert!(!state.has_unread(Snowflake::from_raw(999)));
    state.apply_event(
        GatewayEvent::MessageDelete {
            guild_id: Some(GUILD),
            channel_id: GENERAL,
            id: Snowflake::from_raw(11),
        },
        NOW,
    );
    assert_eq!(state.messages(GENERAL), &[]);
}

#[test]
fn channels_guilds_and_members_follow_events() {
    let mut state = ready_state();
    state.apply_event(
        GatewayEvent::ChannelCreate(channel(
            Snowflake::from_raw(103),
            ChannelKind::Text,
            Some(CATEGORY),
            2,
        )),
        NOW,
    );
    assert_eq!(state.guild(GUILD).unwrap().channels.len(), 3);
    state.apply_event(
        GatewayEvent::ChannelDelete {
            guild_id: GUILD,
            id: CATEGORY,
        },
        NOW,
    );
    let channels = &state.guild(GUILD).unwrap().channels;
    assert_eq!(channels.len(), 2);
    assert!(channels.iter().all(|channel| channel.parent_id.is_none()));

    state.set_members(
        GUILD,
        vec![Member {
            user: user(ME),
            role_ids: Vec::new(),
            joined_at_unix_milliseconds: 0,
        }],
    );
    state.apply_event(
        GatewayEvent::GuildMemberAdd {
            guild_id: GUILD,
            member: Member {
                user: user(OTHER),
                role_ids: Vec::new(),
                joined_at_unix_milliseconds: 1,
            },
        },
        NOW,
    );
    assert_eq!(state.members(GUILD).len(), 2);
    state.apply_event(
        GatewayEvent::GuildMemberRemove {
            guild_id: GUILD,
            user_id: OTHER,
        },
        NOW,
    );
    assert_eq!(state.members(GUILD).len(), 1);

    let change = state.apply_event(
        GatewayEvent::GuildUpdate(GuildSummary {
            icon: None,
            id: GUILD,
            name: "Renamed".into(),
        }),
        NOW,
    );
    assert_eq!(change, StateChange::Guilds);
    assert_eq!(state.guilds()[0].name, "Renamed");

    state.apply_event(GatewayEvent::GuildDelete { id: GUILD }, NOW);
    assert_eq!(state.guilds(), &[]);
}

#[test]
fn a_rename_updates_me_members_in_username_order_and_cached_authors() {
    let mut state = ready_state();
    let member = |id| Member {
        user: user(id),
        role_ids: Vec::new(),
        joined_at_unix_milliseconds: 0,
    };
    state.set_members(GUILD, vec![member(ME), member(OTHER)]);
    state.insert_messages(GENERAL, vec![message(11, ME), message(12, OTHER)]);
    let renamed = PublicUser {
        id: ME,
        username: "zoe".into(),
        display_name: "Zoé".into(),
        avatar: None,
        banner: None,
    };

    let change = state.apply_event(GatewayEvent::UserUpdate(renamed.clone()), NOW);

    assert_eq!(change, StateChange::UserRenamed { user_id: ME });
    let me = state.current_user().unwrap();
    assert_eq!(
        (me.username.as_str(), me.display_name.as_str()),
        ("zoe", "Zoé")
    );
    assert_eq!(me.email, "me@example.org");
    assert_eq!(state.current_user_as_public_user(), Some(renamed.clone()));
    let member_ids: Vec<_> = state.members(GUILD).iter().map(|m| m.user.id).collect();
    assert_eq!(member_ids, [OTHER, ME]);
    assert_eq!(state.members(GUILD)[1].user, renamed);
    let authors: Vec<_> = state
        .messages(GENERAL)
        .iter()
        .map(|message| message.author.display_name.as_str())
        .collect();
    assert_eq!(authors, ["Zoé", "User 2"]);
}

#[test]
fn role_events_update_roles_member_role_ids_and_my_permissions() {
    const CLUB: Snowflake = Snowflake::from_raw(300);
    const MODERATOR: Snowflake = Snowflake::from_raw(301);
    let role = |id, position, permissions| Role {
        id,
        name: "role".into(),
        color: 0,
        position,
        permissions,
        is_displayed_separately: false,
        is_mentionable: false,
        icon: None,
        unicode_emoji: None,
    };
    let everyone = role(CLUB, 0, Permissions::DEFAULT_EVERYONE);
    let mut state = ready_state();
    state.apply_event(
        GatewayEvent::GuildCreate(Box::new(Guild {
            icon: None,
            emojis: Vec::new(),
            sounds: Vec::new(),
            my_join_sound_id: None,
            id: CLUB,
            name: "Other's club".into(),
            owner_id: OTHER,
            my_permissions: Permissions::DEFAULT_EVERYONE,
            my_role_ids: Vec::new(),
            roles: vec![everyone.clone()],
            channels: Vec::new(),
        })),
        NOW,
    );
    state.set_members(
        CLUB,
        vec![Member {
            user: user(ME),
            role_ids: Vec::new(),
            joined_at_unix_milliseconds: 0,
        }],
    );
    let roles_with_moderator = vec![
        everyone.clone(),
        role(MODERATOR, 1, Permissions::MANAGE_ROLES),
    ];
    state.apply_event(
        GatewayEvent::GuildRolesUpdate {
            guild_id: CLUB,
            roles: roles_with_moderator,
        },
        NOW,
    );
    assert_eq!(state.guild(CLUB).unwrap().roles.len(), 2);
    assert_eq!(
        state.guild(CLUB).unwrap().my_permissions,
        Permissions::DEFAULT_EVERYONE,
        "a role nobody gave me changes nothing"
    );

    assert_eq!(
        state.apply_event(
            GatewayEvent::GuildMemberUpdate {
                guild_id: CLUB,
                user_id: ME,
                role_ids: vec![MODERATOR],
            },
            NOW,
        ),
        StateChange::Guilds
    );
    let club = state.guild(CLUB).unwrap();
    assert_eq!(club.my_role_ids, vec![MODERATOR]);
    assert!(club.my_permissions.contains(Permissions::MANAGE_ROLES));
    assert_eq!(state.members(CLUB)[0].role_ids, vec![MODERATOR]);

    state.apply_event(
        GatewayEvent::GuildRolesUpdate {
            guild_id: CLUB,
            roles: vec![everyone],
        },
        NOW,
    );
    let club = state.guild(CLUB).unwrap();
    assert_eq!(
        club.my_role_ids,
        Vec::<Snowflake>::new(),
        "a deleted role leaves its members"
    );
    assert_eq!(club.my_permissions, Permissions::DEFAULT_EVERYONE);
    assert_eq!(state.members(CLUB)[0].role_ids, Vec::<Snowflake>::new());
}

#[test]
fn channel_permissions_apply_the_overwrites_of_my_roles_and_of_me() {
    const CLUB: Snowflake = Snowflake::from_raw(310);
    const ANNOUNCEMENTS: Snowflake = Snowflake::from_raw(311);
    const MUTED: Snowflake = Snowflake::from_raw(312);
    let mut state = ready_state();
    assert_eq!(state.my_channel_permissions(ANNOUNCEMENTS), None);
    let muted_role_denies_sending = PermissionOverwrite {
        id: MUTED,
        kind: OverwriteKind::Role,
        allow: Permissions::empty(),
        deny: Permissions::SEND_MESSAGES,
    };
    let mut can_send_with = |my_permissions, overwrites| {
        state.apply_event(
            GatewayEvent::GuildCreate(Box::new(Guild {
                icon: None,
                emojis: Vec::new(),
                sounds: Vec::new(),
                my_join_sound_id: None,
                id: CLUB,
                name: "Other's club".into(),
                owner_id: OTHER,
                my_permissions,
                my_role_ids: vec![MUTED],
                roles: Vec::new(),
                channels: vec![Channel {
                    icon: None,
                    id: ANNOUNCEMENTS,
                    guild_id: CLUB,
                    kind: ChannelKind::Text,
                    name: "announcements".into(),
                    topic: String::new(),
                    parent_id: None,
                    position: 0,
                    permission_overwrites: overwrites,
                    user_limit: None,
                }],
            })),
            NOW,
        );
        let permissions = state.my_channel_permissions(ANNOUNCEMENTS).unwrap();
        assert!(permissions.contains(Permissions::VIEW_CHANNEL));
        permissions.contains(Permissions::SEND_MESSAGES)
    };
    assert!(!can_send_with(
        Permissions::DEFAULT_EVERYONE,
        vec![muted_role_denies_sending.clone()]
    ));
    let member_allows_sending = PermissionOverwrite {
        id: ME,
        kind: OverwriteKind::Member,
        allow: Permissions::SEND_MESSAGES,
        deny: Permissions::empty(),
    };
    assert!(can_send_with(
        Permissions::DEFAULT_EVERYONE,
        vec![muted_role_denies_sending.clone(), member_allows_sending]
    ));
    assert!(
        can_send_with(Permissions::ADMINISTRATOR, vec![muted_role_denies_sending]),
        "administrators ignore overwrites"
    );
}

#[test]
fn a_snapshot_without_a_channel_forgets_its_messages_and_unsent_text() {
    let mut state = ready_state();
    state.insert_messages(GENERAL, vec![message(20, OTHER)]);
    state.queue_message(GENERAL, "draft".into(), None, "nonce-1".into());
    let mut snapshot = state.guild(GUILD).unwrap().clone();
    snapshot.channels.retain(|channel| channel.id != GENERAL);
    state.apply_event(GatewayEvent::GuildCreate(Box::new(snapshot)), NOW);
    assert_eq!(state.messages(GENERAL).len(), 0);
    assert_eq!(state.pending_messages(GENERAL).count(), 0);
    assert_eq!(state.guild(GUILD).unwrap().channels.len(), 1);

    let late_message = state.apply_event(GatewayEvent::MessageCreate(message(21, OTHER)), NOW);
    let mut late_update = channel(GENERAL, ChannelKind::Text, Some(CATEGORY), 1);
    late_update.name = "hidden".into();
    let late_channel_update = state.apply_event(GatewayEvent::ChannelUpdate(late_update), NOW);
    assert_eq!(late_message, StateChange::Nothing);
    assert_eq!(late_channel_update, StateChange::Nothing);
    assert_eq!(state.messages(GENERAL).len(), 0);
    assert_eq!(
        state.guild(GUILD).unwrap().channels.len(),
        1,
        "not brought back"
    );
}

#[test]
fn presence_and_typing_indicators() {
    let mut state = ready_state();
    state.apply_event(
        GatewayEvent::PresenceUpdate(Presence {
            user_id: OTHER,
            status: PresenceStatus::Online,
            custom_status: None,
        }),
        NOW,
    );
    assert!(state.is_online(OTHER));
    state.apply_event(
        GatewayEvent::PresenceUpdate(Presence {
            user_id: OTHER,
            status: PresenceStatus::Offline,
            custom_status: None,
        }),
        NOW,
    );
    assert!(!state.is_online(OTHER));

    for user_id in [OTHER, ME] {
        state.apply_event(
            GatewayEvent::TypingStart {
                guild_id: Some(GUILD),
                channel_id: GENERAL,
                user_id,
            },
            NOW,
        );
    }
    assert_eq!(
        state.typing_user_ids(GENERAL, NOW + 1),
        [OTHER],
        "my own typing is not shown"
    );
    assert_eq!(
        state.next_typing_notice_end(GENERAL, NOW + 1),
        Some(NOW + TYPING_INDICATOR_LIFETIME_MILLISECONDS),
        "the line changes when the notice ends"
    );
    assert_eq!(
        state.typing_user_ids(GENERAL, NOW + TYPING_INDICATOR_LIFETIME_MILLISECONDS),
        Vec::<Snowflake>::new()
    );
    assert_eq!(
        state.next_typing_notice_end(GENERAL, NOW + TYPING_INDICATOR_LIFETIME_MILLISECONDS),
        None,
        "nothing left to wait for"
    );
    state.apply_event(
        GatewayEvent::TypingStart {
            guild_id: Some(GUILD),
            channel_id: GENERAL,
            user_id: OTHER,
        },
        NOW,
    );
    state.apply_event(GatewayEvent::MessageCreate(message(50, OTHER)), NOW);
    assert_eq!(
        state.typing_user_ids(GENERAL, NOW + 1),
        Vec::<Snowflake>::new(),
        "sending clears typing"
    );
}

#[test]
fn the_next_typing_notice_end_is_the_earliest_one_still_shown() {
    let mut state = ready_state();
    let later_typist = Snowflake::from_raw(3);
    let typing = |user_id| GatewayEvent::TypingStart {
        guild_id: Some(GUILD),
        channel_id: GENERAL,
        user_id,
    };
    state.apply_event(typing(OTHER), NOW);
    state.apply_event(typing(later_typist), NOW + 3_000);
    let first_end = NOW + TYPING_INDICATOR_LIFETIME_MILLISECONDS;
    assert_eq!(
        state.next_typing_notice_end(GENERAL, NOW + 3_000),
        Some(first_end)
    );
    assert_eq!(
        state.next_typing_notice_end(GENERAL, first_end),
        Some(first_end + 3_000),
        "once the first ends, the later one's end comes next"
    );
    assert_eq!(
        state.next_typing_notice_end(Snowflake::from_raw(999), NOW),
        None
    );
}

#[test]
fn outbox_keeps_unsent_text_until_confirmed_retried_or_discarded() {
    let mut state = ready_state();
    let request = state.queue_message(GENERAL, "hi".into(), None, "n1".into());
    assert_eq!(request.nonce.as_deref(), Some("n1"));
    state.queue_message(GENERAL, "again".into(), None, "n2".into());
    // A sending message can neither be retried (double send) nor discarded (maybe stored).
    assert_eq!(state.retry_pending_message("n1"), None);
    assert_eq!(state.discard_failed_message("n1"), StateChange::Nothing);

    state.mark_send_failed("n1");
    // A reconnect keeps the outbox.
    state.apply_ready(Ready {
        may_create_guilds: true,
        session_id: "s2".into(),
        user: state.current_user().unwrap().clone(),
        guilds: state.guilds().to_vec(),
        read_states: Vec::new(),
        online_user_ids: Vec::new(),
        direct_message_channels: Vec::new(),
        channel_unreads: Vec::new(),
        presences: Vec::new(),
        relationships: Vec::new(),
        voice_states: Vec::new(),
        voice_enabled: false,
        maximum_attachment_bytes: None,
    });
    let statuses: Vec<(&str, PendingMessageStatus)> = state
        .pending_messages(GENERAL)
        .map(|pending| (pending.nonce.as_str(), pending.status))
        .collect();
    assert_eq!(
        statuses,
        [
            ("n1", PendingMessageStatus::Failed),
            ("n2", PendingMessageStatus::Sending)
        ]
    );

    let (channel_id, retried) = state.retry_pending_message("n1").unwrap();
    assert_eq!(channel_id, GENERAL);
    assert_eq!(
        (retried.content.as_str(), retried.nonce.as_deref()),
        ("hi", Some("n1"))
    );
    assert_eq!(state.retry_pending_message("n1"), None);

    // The answer lands in the channel we sent to, even if it names another one.
    let mut answer = message(50, ME);
    answer.channel_id = Snowflake::from_raw(999);
    state.confirm_sent_message("n1", answer);
    assert_eq!(state.messages(GENERAL).last().unwrap().id.raw(), 50);
    assert_eq!(state.pending_messages(GENERAL).count(), 1);
    assert_eq!(
        state.confirm_sent_message("unknown", message(51, ME)),
        StateChange::Nothing
    );

    state.mark_send_failed("n2");
    state.discard_failed_message("n2");
    assert_eq!(state.pending_messages(GENERAL).count(), 0);
}

#[test]
fn a_reconnect_resends_only_failed_messages_in_send_order() {
    let mut state = ready_state();
    for nonce in ["n1", "n2", "n3"] {
        state.queue_message(GENERAL, nonce.into(), None, nonce.into());
    }
    state.mark_send_failed("n3");
    state.mark_send_failed("n1");
    let resent: Vec<(Snowflake, String, Option<String>)> = state
        .retry_failed_messages()
        .into_iter()
        .map(|(channel_id, nonce, request)| (channel_id, nonce, request.nonce))
        .collect();
    assert_eq!(
        resent,
        [
            (GENERAL, "n1".to_owned(), Some("n1".to_owned())),
            (GENERAL, "n3".to_owned(), Some("n3".to_owned()))
        ]
    );
    assert!(
        state
            .pending_messages(GENERAL)
            .all(|pending| pending.status == PendingMessageStatus::Sending)
    );
    // Already sending again: a second reconnect does not send them twice at once.
    assert_eq!(state.retry_failed_messages(), []);
}

#[test]
fn outbox_forgets_messages_of_deleted_channels_and_guilds() {
    let mut state = ready_state();
    state.queue_message(GENERAL, "a".into(), None, "n1".into());
    state.apply_event(
        GatewayEvent::ChannelDelete {
            guild_id: GUILD,
            id: GENERAL,
        },
        NOW,
    );
    assert_eq!(state.pending_messages(GENERAL).count(), 0);

    let mut state = ready_state();
    state.queue_message(GENERAL, "a".into(), None, "n1".into());
    state.apply_event(GatewayEvent::GuildDelete { id: GUILD }, NOW);
    assert_eq!(state.pending_messages(GENERAL).count(), 0);

    // Left or deleted while disconnected: the next `Ready` no longer lists the channel.
    let mut state = ready_state();
    state.queue_message(GENERAL, "a".into(), None, "n1".into());
    state.apply_ready(Ready {
        may_create_guilds: true,
        session_id: "s2".into(),
        user: state.current_user().unwrap().clone(),
        guilds: Vec::new(),
        read_states: Vec::new(),
        online_user_ids: Vec::new(),
        direct_message_channels: Vec::new(),
        channel_unreads: Vec::new(),
        presences: Vec::new(),
        relationships: Vec::new(),
        voice_states: Vec::new(),
        voice_enabled: false,
        maximum_attachment_bytes: None,
    });
    assert_eq!(state.pending_messages(GENERAL).count(), 0);
}

#[test]
fn a_refused_message_leaves_the_outbox_with_its_text() {
    let mut state = ready_state();
    state.queue_message(GENERAL, "too long".into(), None, "n1".into());
    let refused = state.remove_refused_message("n1").unwrap();
    assert_eq!(refused.content, "too long");
    assert_eq!(state.pending_messages(GENERAL).count(), 0);
    assert_eq!(state.remove_refused_message("n1"), None);
}

fn conversation(id: u64, with: Snowflake, last_message_id: Option<u64>) -> DirectMessageChannel {
    DirectMessageChannel {
        id: Snowflake::from_raw(id),
        recipients: vec![user(with)],
        last_message_id: last_message_id.map(Snowflake::from_raw),
        owner_id: None,
        name: String::new(),
    }
}

#[test]
fn conversations_are_ordered_by_last_activity_and_follow_renames() {
    let mut state = ready_state();
    state.upsert_direct_message_channel(conversation(300, OTHER, Some(310)));
    let change = state.apply_event(
        GatewayEvent::DirectMessageChannelCreate(conversation(400, Snowflake::from_raw(3), None)),
        NOW,
    );
    assert_eq!(change, StateChange::DirectMessages);
    let order = |state: &ClientState| {
        state
            .direct_message_channels()
            .iter()
            .map(|channel| channel.id.raw())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        order(&state),
        [400, 300],
        "a new conversation counts as activity"
    );

    let mut reply = message(500, OTHER);
    reply.channel_id = Snowflake::from_raw(300);
    state.apply_event(GatewayEvent::MessageCreate(reply), NOW);
    assert_eq!(order(&state), [300, 400]);
    assert_eq!(state.messages(Snowflake::from_raw(300)).len(), 1);
    assert_eq!(
        state.my_channel_permissions(Snowflake::from_raw(300)),
        Some(DIRECT_MESSAGE_PERMISSIONS)
    );

    let mut renamed = user(OTHER);
    renamed.display_name = "Renamed".into();
    state.apply_event(GatewayEvent::UserUpdate(renamed), NOW);
    assert_eq!(
        state.direct_message_channels()[0].recipients[0].display_name,
        "Renamed"
    );
}

#[test]
fn an_empty_conversation_with_an_unsent_message_survives_ready() {
    let mut state = ready_state();
    let empty = conversation(300, OTHER, None);
    let abandoned = conversation(400, Snowflake::from_raw(3), None);
    state.upsert_direct_message_channel(empty.clone());
    state.upsert_direct_message_channel(abandoned);
    state.queue_message(empty.id, "first".into(), None, "n".into());
    let user = state.current_user().cloned().unwrap();
    let guilds = state.guilds().to_vec();
    state.apply_ready(Ready {
        may_create_guilds: true,
        session_id: "s2".into(),
        user,
        guilds,
        read_states: Vec::new(),
        online_user_ids: Vec::new(),
        direct_message_channels: Vec::new(),
        channel_unreads: Vec::new(),
        presences: Vec::new(),
        relationships: Vec::new(),
        voice_states: Vec::new(),
        voice_enabled: false,
        maximum_attachment_bytes: None,
    });
    assert_eq!(
        state.direct_message_channels(),
        std::slice::from_ref(&empty)
    );
    assert_eq!(state.pending_messages(empty.id).count(), 1);
}

#[test]
fn a_conversation_with_messages_received_offline_is_unread_before_any_is_cached() {
    let mut state = ready_state();
    state.upsert_direct_message_channel(conversation(300, OTHER, Some(310)));
    assert!(state.has_unread(Snowflake::from_raw(300)));
    state.mark_read(Snowflake::from_raw(300), Snowflake::from_raw(310));
    assert!(!state.has_unread(Snowflake::from_raw(300)));
}

#[test]
fn a_participant_reacts_and_sends_polls_and_checklists_in_a_conversation_as_the_server_allows() {
    // The server's `direct_message_permissions` for a participant who may write there.
    let expected = Permissions::VIEW_CHANNEL
        | Permissions::SEND_MESSAGES
        | Permissions::READ_MESSAGE_HISTORY
        | Permissions::ATTACH_FILES
        | Permissions::ADD_REACTIONS
        | Permissions::SEND_POLLS
        | Permissions::SEND_CHECKLISTS;
    assert_eq!(DIRECT_MESSAGE_PERMISSIONS, expected);
}
