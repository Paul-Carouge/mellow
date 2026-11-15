//! Private calls (ADR-0062) through the real gateway and UDP port, with the headless `str0m`
//! bots of the voice tests: ringing, answering on one device, media, the call's message, the end
//! rules, and access changes.

use std::time::Duration;

use mellow_protocol::{
    Call, ClientFrame, CreateGroupConversationRequest, GatewayEvent, Message, MessageCall,
    OpenDirectMessageRequest, SendFriendRequest, SetRelationshipKind, Snowflake,
    VoiceDisconnectReason, VoiceJoinRefusal,
};

use super::CallError;
use super::tests::{Fixture, fixture, join_and_start, now, pending_bot};
use crate::gateway::tests::{ClientSocket, identified, receive_dispatch, send};

/// Ringing never ends by itself during a test: the tests end it (`end_ringing`).
const RING_TIMEOUT_FOR_TESTS: Duration = Duration::from_secs(3_600);

async fn call_fixture() -> Fixture {
    let fixture = fixture().await;
    fixture.state.voice.set_ring_timeout(RING_TIMEOUT_FOR_TESTS);
    fixture
}

/// The one-to-one conversation of the owner and the member (who share the fixture's guild).
async fn owner_and_member_conversation(fixture: &Fixture) -> Snowflake {
    fixture
        .state
        .messages
        .open_direct_message(
            fixture.user_id("owner"),
            OpenDirectMessageRequest {
                recipient_id: fixture.user_id("member"),
            },
            now(),
        )
        .await
        .unwrap()
        .id
}

async fn start(fixture: &Fixture, caller: &str, channel_id: Snowflake) -> Call {
    fixture
        .state
        .voice
        .start_call(fixture.user_id(caller), channel_id, now())
        .await
        .unwrap()
}

async fn next_call_event(socket: &mut ClientSocket, channel_id: Snowflake) -> GatewayEvent {
    receive_dispatch(socket, |event| match event {
        GatewayEvent::CallCreate(call) | GatewayEvent::CallUpdate(call) => {
            call.channel_id == channel_id
        }
        GatewayEvent::CallDelete { channel_id: ended } => *ended == channel_id,
        _ => false,
    })
    .await
    .1
}

/// Reads call events until one shows the call as `matches` wants it.
async fn call_update_where(
    socket: &mut ClientSocket,
    channel_id: Snowflake,
    matches: impl Fn(&Call) -> bool,
) -> Call {
    loop {
        match next_call_event(socket, channel_id).await {
            GatewayEvent::CallCreate(call) | GatewayEvent::CallUpdate(call) if matches(&call) => {
                return call;
            }
            GatewayEvent::CallDelete { .. } => panic!("the call ended"),
            _ => {}
        }
    }
}

async fn wait_for_call_delete(socket: &mut ClientSocket, channel_id: Snowflake) {
    receive_dispatch(socket, |event| {
        matches!(event, GatewayEvent::CallDelete { channel_id: ended } if *ended == channel_id)
    })
    .await;
}

async fn ended_call_message(socket: &mut ClientSocket, message_id: Snowflake) -> MessageCall {
    match receive_dispatch(
        socket,
        |event| matches!(event, GatewayEvent::MessageUpdate(message) if message.id == message_id),
    )
    .await
    .1
    {
        GatewayEvent::MessageUpdate(Message {
            call: Some(call), ..
        }) => call,
        other => panic!("not a call's message: {other:?}"),
    }
}

async fn wait_for_disconnection(
    socket: &mut ClientSocket,
    channel_id: Snowflake,
) -> VoiceDisconnectReason {
    match receive_dispatch(socket, |event| {
        matches!(event, GatewayEvent::VoiceDisconnected { channel_id: left, .. } if *left == channel_id)
    })
    .await
    .1
    {
        GatewayEvent::VoiceDisconnected { reason, .. } => reason,
        other => unreachable!("{other:?}"),
    }
}

fn participant_ids(call: &Call) -> Vec<Snowflake> {
    call.participants
        .iter()
        .map(|participant| participant.user_id)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_rings_every_device_is_answered_on_one_and_its_message_records_it() {
    let fixture = call_fixture().await;
    let (owner_id, member_id) = (fixture.user_id("owner"), fixture.user_id("member"));
    let conversation = owner_and_member_conversation(&fixture).await;
    let (mut owner, _) = identified(fixture.address, fixture.token("owner")).await;
    let (mut member_phone, _) = identified(fixture.address, fixture.token("member")).await;
    let (mut member_laptop, _) = identified(fixture.address, fixture.token("member")).await;

    let started = start(&fixture, "owner", conversation).await;
    // The conversation reaches the callee with the call's message, its first one.
    let announced = receive_dispatch(&mut member_phone, |event| {
        matches!(event, GatewayEvent::DirectMessageChannelCreate(channel) if channel.id == conversation)
    })
    .await
    .1;
    let posted = receive_dispatch(&mut member_phone, |event| {
        matches!(event, GatewayEvent::MessageCreate(message) if message.channel_id == conversation)
    })
    .await
    .1;
    let ringing = next_call_event(&mut member_laptop, conversation).await;
    let again = start(&fixture, "member", conversation).await;
    let (_, ready) = identified(fixture.address, fixture.token("member")).await;

    let owner_bot = join_and_start(&mut owner, conversation, 1).await;
    call_update_where(&mut member_laptop, conversation, |call| {
        participant_ids(call) == [owner_id]
    })
    .await;
    let member_bot = join_and_start(&mut member_phone, conversation, 2).await;
    let answered = call_update_where(&mut member_laptop, conversation, |call| {
        participant_ids(call) == [owner_id, member_id]
    })
    .await;
    owner_bot.wait_until_hearing(2).await;
    member_bot.wait_until_hearing(1).await;

    send(&mut owner, &ClientFrame::VoiceLeave).await;
    let after_owner_left = call_update_where(&mut member_laptop, conversation, |call| {
        participant_ids(call) == [member_id]
    })
    .await;
    send(&mut member_phone, &ClientFrame::VoiceLeave).await;
    wait_for_call_delete(&mut member_laptop, conversation).await;
    let record = ended_call_message(&mut owner, started.message_id).await;
    drop((owner_bot, member_bot));
    fixture.close().await;

    assert!(matches!(
        announced,
        GatewayEvent::DirectMessageChannelCreate(_)
    ));
    assert!(
        matches!(&posted, GatewayEvent::MessageCreate(message)
            if message.id == started.message_id && message.content.is_empty()
                && message.author.id == owner_id
                && message.call.as_ref().is_some_and(|call| call.ended_at_unix_milliseconds.is_none())),
        "{posted:?}"
    );
    assert_eq!(started.caller_id, owner_id);
    assert_eq!(started.ringing_user_ids, [member_id]);
    assert_eq!(started.participants, []);
    assert_eq!(ringing, GatewayEvent::CallCreate(started.clone()));
    assert_eq!(
        again.message_id, started.message_id,
        "one call per conversation"
    );
    assert!(ready.calls_enabled);
    assert_eq!(ready.calls, [started]);
    assert_eq!(
        answered.ringing_user_ids,
        [],
        "answering on one device stops every device ringing"
    );
    assert_eq!(after_owner_left.ringing_user_ids, []);
    assert_eq!(record.participant_ids, {
        let mut both = vec![owner_id, member_id];
        both.sort_unstable_by_key(|id| id.raw());
        both
    });
    assert!(record.ended_at_unix_milliseconds.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unanswered_call_ends_with_its_ringing_and_a_decline_ends_a_one_to_one_call() {
    let fixture = call_fixture().await;
    let (owner_id, member_id) = (fixture.user_id("owner"), fixture.user_id("member"));
    let conversation = owner_and_member_conversation(&fixture).await;
    let (mut owner, _) = identified(fixture.address, fixture.token("owner")).await;
    let (mut member, _) = identified(fixture.address, fixture.token("member")).await;

    let missed = start(&fixture, "owner", conversation).await;
    let _owner_bot = join_and_start(&mut owner, conversation, 1).await;
    call_update_where(&mut member, conversation, |call| {
        participant_ids(call) == [owner_id]
    })
    .await;
    fixture.state.voice.end_ringing(conversation).await;
    let caller_cut_off = wait_for_disconnection(&mut owner, conversation).await;
    wait_for_call_delete(&mut member, conversation).await;
    let missed_record = ended_call_message(&mut member, missed.message_id).await;

    let declined = start(&fixture, "owner", conversation).await;
    next_call_event(&mut member, conversation).await;
    fixture
        .state
        .voice
        .decline_call(member_id, conversation)
        .await;
    wait_for_call_delete(&mut member, conversation).await;
    let declined_record = ended_call_message(&mut owner, declined.message_id).await;
    fixture.close().await;

    assert_eq!(caller_cut_off, VoiceDisconnectReason::CallEnded);
    assert_eq!(
        missed_record.participant_ids,
        [owner_id],
        "missed by the member"
    );
    assert!(
        declined_record.participant_ids.is_empty(),
        "the caller never joined"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_participants_who_may_write_call_and_a_block_ends_the_call() {
    let fixture = call_fixture().await;
    let (owner_id, member_id) = (fixture.user_id("owner"), fixture.user_id("member"));
    let conversation = owner_and_member_conversation(&fixture).await;
    let outsider = fixture
        .state
        .voice
        .start_call(fixture.user_id("outsider"), conversation, now())
        .await;
    let guild_channel = fixture
        .state
        .voice
        .start_call(owner_id, fixture.lounge, now())
        .await;
    let (mut owner, _) = identified(fixture.address, fixture.token("owner")).await;
    let (mut member, _) = identified(fixture.address, fixture.token("member")).await;
    // No call going on: the conversation is no voice channel.
    send(
        &mut owner,
        &ClientFrame::VoiceJoin {
            channel_id: conversation,
            offer_id: 1,
            sdp_offer: pending_bot().offer,
            self_mute: false,
            self_deaf: false,
        },
    )
    .await;
    let refused = receive_dispatch(&mut owner, |event| {
        matches!(event, GatewayEvent::VoiceJoinRefused { .. })
    })
    .await
    .1;

    start(&fixture, "owner", conversation).await;
    let _owner_bot = join_and_start(&mut owner, conversation, 1).await;
    let _member_bot = join_and_start(&mut member, conversation, 2).await;
    call_update_where(&mut owner, conversation, |call| {
        call.participants.len() == 2
    })
    .await;
    fixture
        .state
        .relationships
        .set(member_id, owner_id, SetRelationshipKind::Blocked, now())
        .await
        .unwrap();
    let owner_cut_off = wait_for_disconnection(&mut owner, conversation).await;
    let member_cut_off = wait_for_disconnection(&mut member, conversation).await;
    wait_for_call_delete(&mut owner, conversation).await;
    let after_block = fixture
        .state
        .voice
        .start_call(owner_id, conversation, now())
        .await;
    fixture.close().await;

    assert!(matches!(outsider, Err(CallError::NotFound)), "{outsider:?}");
    assert!(
        matches!(guild_channel, Err(CallError::NotFound)),
        "{guild_channel:?}"
    );
    assert!(
        matches!(
            refused,
            GatewayEvent::VoiceJoinRefused {
                reason: VoiceJoinRefusal::MissingPermissions,
                ..
            }
        ),
        "{refused:?}"
    );
    assert_eq!(owner_cut_off, VoiceDisconnectReason::PermissionsChanged);
    assert_eq!(member_cut_off, VoiceDisconnectReason::PermissionsChanged);
    assert!(
        matches!(after_block, Err(CallError::NotFound)),
        "{after_block:?}"
    );
}

async fn befriend(fixture: &Fixture, asker: &str, answerer: &str) {
    fixture
        .state
        .relationships
        .send_friend_request(
            fixture.user_id(asker),
            SendFriendRequest {
                username: answerer.into(),
                friend_code: None,
            },
            now(),
        )
        .await
        .unwrap();
    fixture
        .state
        .relationships
        .set(
            fixture.user_id(answerer),
            fixture.user_id(asker),
            SetRelationshipKind::Friend,
            now(),
        )
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_group_call_never_rings_across_a_block_and_goes_on_while_someone_stays() {
    let fixture = call_fixture().await;
    let (owner_id, member_id, muted_id) = (
        fixture.user_id("owner"),
        fixture.user_id("member"),
        fixture.user_id("muted"),
    );
    befriend(&fixture, "owner", "member").await;
    befriend(&fixture, "owner", "muted").await;
    let group = fixture
        .state
        .messages
        .create_group_conversation(
            owner_id,
            CreateGroupConversationRequest {
                recipient_ids: vec![member_id, muted_id],
                name: String::new(),
            },
            now(),
        )
        .await
        .unwrap()
        .id;
    // A block inside a group leaves it open (ADR-0026), but the call never rings across it.
    fixture
        .state
        .relationships
        .set(muted_id, owner_id, SetRelationshipKind::Blocked, now())
        .await
        .unwrap();
    let (mut owner, _) = identified(fixture.address, fixture.token("owner")).await;
    let (mut member, _) = identified(fixture.address, fixture.token("member")).await;
    let (mut muted, _) = identified(fixture.address, fixture.token("muted")).await;

    let started = start(&fixture, "owner", group).await;
    let seen_by_muted = next_call_event(&mut muted, group).await;
    let _owner_bot = join_and_start(&mut owner, group, 1).await;
    let member_bot = join_and_start(&mut member, group, 2).await;
    // Muted was not rung but may still join: everyone in a group takes part.
    let muted_bot = join_and_start(&mut muted, group, 3).await;
    member_bot.wait_until_hearing(3).await;
    muted_bot.wait_until_hearing(2).await;
    send(&mut owner, &ClientFrame::VoiceLeave).await;
    let without_caller = call_update_where(&mut member, group, |call| {
        participant_ids(call) == [member_id, muted_id]
    })
    .await;
    // The owner removes Muted from the group: their part of the call ends with it.
    fixture
        .state
        .messages
        .remove_group_participant(owner_id, group, muted_id)
        .await
        .unwrap();
    let muted_cut_off = wait_for_disconnection(&mut muted, group).await;
    wait_for_call_delete(&mut muted, group).await;
    let after_removal = call_update_where(&mut member, group, |call| {
        participant_ids(call) == [member_id]
    })
    .await;
    drop((member_bot, muted_bot));
    fixture.close().await;

    // Muted's block shows to no one: they are listed as rung, until ringing ends, like anyone
    // who does not answer; only their own view leaves them out, so their devices stay silent.
    let mut rung = vec![member_id, muted_id];
    rung.sort_unstable_by_key(|id| id.raw());
    assert_eq!(started.ringing_user_ids, rung);
    assert_eq!(
        seen_by_muted,
        GatewayEvent::CallCreate(Call {
            ringing_user_ids: vec![member_id],
            ..started
        })
    );
    assert!(
        without_caller.participants.len() == 2,
        "the group call goes on"
    );
    assert_eq!(muted_cut_off, VoiceDisconnectReason::PermissionsChanged);
    assert_eq!(participant_ids(&after_removal), [member_id]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_message_reads_in_history_and_cannot_be_edited_or_forwarded() {
    let fixture = call_fixture().await;
    let owner_id = fixture.user_id("owner");
    let conversation = owner_and_member_conversation(&fixture).await;
    let call = start(&fixture, "owner", conversation).await;
    let history = fixture
        .state
        .messages
        .list_messages(
            fixture.user_id("member"),
            conversation,
            mellow_protocol::MessageHistoryQuery {
                before: None,
                after: None,
                limit: None,
            },
        )
        .await
        .unwrap();
    let edit = fixture
        .state
        .messages
        .edit(
            owner_id,
            conversation,
            call.message_id,
            mellow_protocol::EditMessageRequest {
                content: "not a call".into(),
            },
            now(),
        )
        .await;
    let forward = fixture
        .state
        .messages
        .send(
            owner_id,
            conversation,
            mellow_protocol::SendMessageRequest {
                content: String::new(),
                reply_to_id: None,
                nonce: None,
                attachment_ids: Vec::new(),
                forwarded_from: Some(mellow_protocol::ForwardOrigin {
                    channel_id: conversation,
                    message_id: call.message_id,
                }),
                poll: None,
                checklist: None,
                progress_bar: None,
            },
            now(),
        )
        .await;
    fixture.close().await;

    assert_eq!(history.len(), 1);
    assert_eq!(
        history[0].call,
        Some(MessageCall {
            participant_ids: Vec::new(),
            ended_at_unix_milliseconds: None,
        })
    );
    let is_call_change = |result: &Result<Message, crate::domain::messages::MessageError>| {
        matches!(
            result,
            Err(crate::domain::messages::MessageError::InvalidInput(
                crate::domain::message_rules::MessageRuleViolation::CallChange
            ))
        )
    };
    assert!(is_call_change(&edit), "{edit:?}");
    assert!(is_call_change(&forward), "{forward:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_block_placed_while_a_group_call_rings_stops_it_ringing_across_the_block() {
    let fixture = call_fixture().await;
    let (owner_id, member_id, muted_id) = (
        fixture.user_id("owner"),
        fixture.user_id("member"),
        fixture.user_id("muted"),
    );
    befriend(&fixture, "owner", "member").await;
    befriend(&fixture, "owner", "muted").await;
    let group = fixture
        .state
        .messages
        .create_group_conversation(
            owner_id,
            CreateGroupConversationRequest {
                recipient_ids: vec![member_id, muted_id],
                name: String::new(),
            },
            now(),
        )
        .await
        .unwrap()
        .id;
    let (mut owner, _) = identified(fixture.address, fixture.token("owner")).await;
    let (mut member, _) = identified(fixture.address, fixture.token("member")).await;
    let started = start(&fixture, "owner", group).await;
    let _owner_bot = join_and_start(&mut owner, group, 1).await;
    call_update_where(&mut member, group, |call| {
        participant_ids(call) == [owner_id]
    })
    .await;
    fixture
        .state
        .relationships
        .set(member_id, owner_id, SetRelationshipKind::Blocked, now())
        .await
        .unwrap();
    // The member's devices stop ringing; the owner learns nothing of the block.
    let seen_by_member = call_update_where(&mut member, group, |call| {
        !call.ringing_user_ids.contains(&member_id)
    })
    .await;
    let (_, owner_ready) = identified(fixture.address, fixture.token("owner")).await;
    fixture.close().await;

    let mut rung = vec![member_id, muted_id];
    rung.sort_unstable_by_key(|id| id.raw());
    assert_eq!(started.ringing_user_ids, rung);
    assert_eq!(seen_by_member.ringing_user_ids, [muted_id]);
    assert_eq!(owner_ready.calls[0].ringing_user_ids, rung);
}

/// A group of the owner and the member alone: Muted was in it and left.
async fn group_of_owner_and_member(fixture: &Fixture) -> Snowflake {
    let (owner_id, member_id, muted_id) = (
        fixture.user_id("owner"),
        fixture.user_id("member"),
        fixture.user_id("muted"),
    );
    befriend(fixture, "owner", "member").await;
    befriend(fixture, "owner", "muted").await;
    let group = fixture
        .state
        .messages
        .create_group_conversation(
            owner_id,
            CreateGroupConversationRequest {
                recipient_ids: vec![member_id, muted_id],
                name: String::new(),
            },
            now(),
        )
        .await
        .unwrap()
        .id;
    fixture
        .state
        .messages
        .remove_group_participant(muted_id, group, muted_id)
        .await
        .unwrap();
    group
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_nobody_can_hear_ring_still_ends_with_its_ringing() {
    let fixture = call_fixture().await;
    let (owner_id, member_id) = (fixture.user_id("owner"), fixture.user_id("member"));
    let group = group_of_owner_and_member(&fixture).await;
    fixture
        .state
        .relationships
        .set(member_id, owner_id, SetRelationshipKind::Blocked, now())
        .await
        .unwrap();
    let (mut owner, _) = identified(fixture.address, fixture.token("owner")).await;
    let started = start(&fixture, "owner", group).await;
    fixture.state.voice.end_ringing(group).await;
    wait_for_call_delete(&mut owner, group).await;
    let record = ended_call_message(&mut owner, started.message_id).await;
    fixture.close().await;

    assert_eq!(record.participant_ids, []);
    assert!(record.ended_at_unix_milliseconds.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn calls_from_someone_not_a_friend_ring_a_bounded_number_of_times() {
    use crate::http::rate_limit::CALL_RINGS_RECEIVED_FROM_STRANGERS;

    let fixture = call_fixture().await;
    let member_id = fixture.user_id("member");
    // The owner and the member share a server, nothing more.
    let conversation = owner_and_member_conversation(&fixture).await;
    let (mut member, _) = identified(fixture.address, fixture.token("member")).await;
    for _ in 0..CALL_RINGS_RECEIVED_FROM_STRANGERS.maximum_events {
        start(&fixture, "owner", conversation).await;
        fixture
            .state
            .voice
            .decline_call(member_id, conversation)
            .await;
        wait_for_call_delete(&mut member, conversation).await;
    }
    let over_the_budget = start(&fixture, "owner", conversation).await;
    let seen_by_member = match next_call_event(&mut member, conversation).await {
        GatewayEvent::CallCreate(call) => call,
        other => panic!("{other:?}"),
    };
    fixture.close().await;

    assert_eq!(
        over_the_budget.ringing_user_ids,
        [member_id],
        "the caller cannot tell"
    );
    assert_eq!(
        seen_by_member.ringing_user_ids,
        [],
        "the member's devices stay silent"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_caller_rings_one_conversation_at_a_time() {
    let fixture = call_fixture().await;
    let owner_id = fixture.user_id("owner");
    let with_member = owner_and_member_conversation(&fixture).await;
    let with_muted = fixture
        .state
        .messages
        .open_direct_message(
            owner_id,
            OpenDirectMessageRequest {
                recipient_id: fixture.user_id("muted"),
            },
            now(),
        )
        .await
        .unwrap()
        .id;
    start(&fixture, "owner", with_member).await;
    let second = fixture
        .state
        .voice
        .start_call(owner_id, with_muted, now())
        .await;
    fixture.close().await;

    assert!(
        matches!(second, Err(CallError::AlreadyCalling)),
        "{second:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn who_joined_is_recorded_at_once_and_a_stop_ends_the_calls_going_on() {
    let fixture = call_fixture().await;
    let (owner_id, member_id) = (fixture.user_id("owner"), fixture.user_id("member"));
    let conversation = owner_and_member_conversation(&fixture).await;
    let (mut owner, _) = identified(fixture.address, fixture.token("owner")).await;
    let (mut member, _) = identified(fixture.address, fixture.token("member")).await;
    let started = start(&fixture, "owner", conversation).await;
    let _owner_bot = join_and_start(&mut owner, conversation, 1).await;
    let _member_bot = join_and_start(&mut member, conversation, 2).await;
    call_update_where(&mut owner, conversation, |call| {
        call.participants.len() == 2
    })
    .await;
    let record = || async {
        crate::storage::call_messages::calls_of_messages(
            &fixture.state.database,
            &[started.message_id],
        )
        .await
        .unwrap()
        .remove(&started.message_id)
        .unwrap()
    };
    // Written beside the join, without holding it up: read until it lands.
    let deadline = std::time::Instant::now() + super::tests::HEARING_TIMEOUT;
    let mut while_going_on = record().await;
    while while_going_on.participant_ids.len() < 2 && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
        while_going_on = record().await;
    }
    fixture.state.voice.end_all_calls().await;
    let after_stop = record().await;
    let (_, ready) = identified(fixture.address, fixture.token("member")).await;
    fixture.close().await;

    let mut both = vec![owner_id, member_id];
    both.sort_unstable_by_key(|id| id.raw());
    assert_eq!(while_going_on.participant_ids, both);
    assert_eq!(while_going_on.ended_at_unix_milliseconds, None);
    assert_eq!(after_stop.participant_ids, both);
    assert!(
        after_stop.ended_at_unix_milliseconds.is_some(),
        "ended before the database closes"
    );
    assert_eq!(ready.calls, []);
}
