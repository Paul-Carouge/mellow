//! Every family of REST routes the client core offers, against a real in-process `mellow-server`:
//! one answer that works and one refusal each.

// The server refuses Windows until its master key file can be protected there (ADR-0006).
#![cfg(not(windows))]
// Test helpers outside `#[test]` functions: failing loudly is the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mellow_client_core::{
    ClientError, Download, DownloadableFile, InstanceAddress, RestClient, SignIn, UploadProgress,
};
use mellow_protocol::{
    ApiErrorCode, BannerCropQuery, ChannelKind, ChannelPlacement, ChosenStatus,
    CompleteTwoFactorSignInRequest, CreateGroupConversationRequest, CreateInviteRequest,
    CreateRoleRequest, CreateSoundQuery, EnableTwoFactorRequest, Guild, HealthStatus,
    IconCropQuery, LoginRequest, MAXIMUM_SEARCH_QUERY_LENGTH, Message, MessageSearchQuery,
    NewChecklist, NewPoll, NewProfileLink, NewProgressBar, POLL_DURATIONS_IN_HOURS, Permissions,
    ProveTwoFactorRequest, ReactionUsersQuery, RegisterRequest, RelationshipKind,
    SendFriendRequest, SendMessageRequest, SetRelationshipKind, Snowflake,
    StartTwoFactorEnrolmentRequest, UpdateSoundRequest, UpdateUserProfileRequest,
};
use mellow_server::commands::{InitializeOptions, initialize_instance, serve_instance};
use mellow_server::domain::account_rules::{GuildCreationPolicy, RegistrationMode};
use tokio::sync::oneshot;

const PASSWORD: &str = "correct horse battery staple";
/// Small, so a test can go over it without sending megabytes.
const MAXIMUM_ATTACHMENT_BYTES: u64 = 256 * 1024;

struct RunningInstance {
    _directory: tempfile::TempDir,
    address: InstanceAddress,
    stop: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl RunningInstance {
    async fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        // shortcut: the port is free when probed but could be taken before the server binds it.
        let listen_address = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let mut config = initialize_instance(&InitializeOptions {
            config_path: directory.path().join("mellow.toml"),
            data_directory: directory.path().join("data"),
            master_key_file: directory.path().join("master.key"),
            listen_address,
            registration_mode: RegistrationMode::Open,
            tls: None,
        })
        .await
        .unwrap();
        config.guild_creation = GuildCreationPolicy::Everyone;
        config.maximum_attachment_bytes = MAXIMUM_ATTACHMENT_BYTES;
        // The instance only hands the key to clients: nothing here calls Klipy.
        config.gifs.klipy_app_key = Some("test-key".into());
        let (stop, stopped) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            serve_instance(&config, async {
                let _ = stopped.await;
            })
            .await
        });
        let address = InstanceAddress::parse(&format!("http://{listen_address}")).unwrap();
        for _ in 0..100 {
            if tokio::net::TcpStream::connect(listen_address).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Self {
            _directory: directory,
            address,
            stop,
            server,
        }
    }

    /// Waits for the database to close: SQLCipher crashes if it is still closing at exit.
    async fn stop(self) {
        let _ = self.stop.send(());
        self.server.await.unwrap().unwrap();
    }

    async fn registered(&self, username: &str) -> RestClient {
        let mut client = RestClient::new(self.address.clone()).unwrap();
        client
            .register(&RegisterRequest {
                username: username.into(),
                display_name: None,
                email: format!("{username}@example.org"),
                password: PASSWORD.into(),
                device_name: "test".into(),
            })
            .await
            .unwrap();
        client
    }

    /// `owner`'s guild with `member` joined through an invite, and its first text channel.
    async fn guild_with_member(
        owner: &RestClient,
        member: &RestClient,
    ) -> (Guild, Snowflake, String) {
        let guild = owner.create_guild("Hideout").await.unwrap();
        let invite = owner
            .create_invite(guild.id, &CreateInviteRequest::default())
            .await
            .unwrap();
        member.join_with_invite(&invite.code).await.unwrap();
        let general = guild
            .channels
            .iter()
            .find(|channel| channel.kind == ChannelKind::Text)
            .unwrap()
            .id;
        (guild, general, invite.code)
    }
}

fn assert_refused<T: std::fmt::Debug>(result: Result<T, ClientError>, expected: ApiErrorCode) {
    match result {
        Err(ClientError::Api { code, .. }) if code == expected => {}
        other => panic!("expected {expected:?}, got {other:?}"),
    }
}

fn text(content: &str) -> SendMessageRequest {
    SendMessageRequest {
        content: content.into(),
        reply_to_id: None,
        nonce: None,
        attachment_ids: Vec::new(),
        forwarded_from: None,
        poll: None,
        checklist: None,
        progress_bar: None,
    }
}

fn png(width: u32, height: u32) -> Vec<u8> {
    use image::ImageEncoder as _;
    let pixels: Vec<u8> = (0..width * height)
        .flat_map(|index| [u8::try_from(index % 256).unwrap(), 40, 90, 255])
        .collect();
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&pixels, width, height, image::ExtendedColorType::Rgba8)
        .unwrap();
    bytes
}

/// A canonical PCM WAV (mono, 48 kHz, 16-bit) of `samples` samples, as ADR-0032 stores sounds.
fn canonical_wav(samples: u32) -> Vec<u8> {
    let data_bytes = samples * 2;
    let mut file = Vec::new();
    file.extend_from_slice(b"RIFF");
    file.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    file.extend_from_slice(b"WAVEfmt ");
    file.extend_from_slice(&16_u32.to_le_bytes());
    file.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    file.extend_from_slice(&1_u16.to_le_bytes()); // mono
    file.extend_from_slice(&48_000_u32.to_le_bytes());
    file.extend_from_slice(&96_000_u32.to_le_bytes());
    file.extend_from_slice(&2_u16.to_le_bytes());
    file.extend_from_slice(&16_u16.to_le_bytes());
    file.extend_from_slice(b"data");
    file.extend_from_slice(&data_bytes.to_le_bytes());
    file.resize(file.len() + data_bytes as usize, 0x7F);
    file
}

fn base32_decode(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut bits: u64 = 0;
    let mut bit_count = 0;
    let mut bytes = Vec::new();
    for character in text.bytes() {
        let value = ALPHABET
            .iter()
            .position(|&letter| letter == character)
            .unwrap();
        bits = (bits << 5) | value as u64;
        bit_count += 5;
        if bit_count >= 8 {
            bit_count -= 8;
            bytes.push(u8::try_from((bits >> bit_count) & 0xFF).unwrap());
        }
    }
    bytes
}

/// The authenticator code of `secret` (RFC 6238: HMAC-SHA-1, 6 digits, 30-second steps). The
/// instance accepts one step of drift either way, so a step boundary between here and its check
/// cannot fail the test.
fn current_totp_code(secret: &[u8]) -> String {
    use aws_lc_rs::hmac;
    let step = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / 30;
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret);
    let digest = hmac::sign(&key, &step.to_be_bytes());
    let digest = digest.as_ref();
    let offset = usize::from(digest[19] & 0x0F);
    let truncated =
        u32::from_be_bytes(digest[offset..offset + 4].try_into().unwrap()) & 0x7FFF_FFFF;
    format!("{:06}", truncated % 1_000_000)
}

#[tokio::test(flavor = "multi_thread")]
async fn health_presence_profiles_gifs_and_read_states_round_trip() {
    let instance = RunningInstance::start().await;
    let anonymous = RestClient::new(instance.address.clone()).unwrap();
    let health = anonymous.health().await.unwrap();
    assert_eq!(health.status, HealthStatus::Ok);
    assert_eq!(health.version, env!("CARGO_PKG_VERSION"));
    assert!(matches!(
        anonymous.list_my_guilds().await,
        Err(ClientError::NotSignedIn)
    ));

    let ada = instance.registered("ada").await;
    let bob = instance.registered("bob").await;
    assert!(ada.name_availability("cleo").await.unwrap().is_available);
    assert!(!ada.name_availability("bob").await.unwrap().is_available);
    assert_refused(
        ada.name_availability("").await,
        ApiErrorCode::ValidationFailed,
    );

    let me = ada
        .update_presence(ChosenStatus::DoNotDisturb, Some("Reading".into()))
        .await
        .unwrap();
    assert_eq!(me.status, ChosenStatus::DoNotDisturb);
    assert_eq!(me.custom_status.as_deref(), Some("Reading"));

    let profile = ada
        .update_my_profile(&UpdateUserProfileRequest {
            about_me: "Mostly harmless".into(),
            links: vec![NewProfileLink {
                url: "https://example.org/".into(),
                label: Some("Site".into()),
            }],
        })
        .await
        .unwrap();
    assert_eq!(ada.my_profile().await.unwrap(), profile);
    assert_refused(
        ada.update_my_profile(&UpdateUserProfileRequest {
            about_me: String::new(),
            links: vec![NewProfileLink {
                url: "http://example.org/".into(),
                label: None,
            }],
        })
        .await,
        ApiErrorCode::ValidationFailed,
    );
    // Ada and Bob share nothing yet: their profiles are hidden from each other.
    assert_refused(
        bob.user_profile(profile.user_id).await,
        ApiErrorCode::NotFound,
    );

    let gif_settings = ada.gif_settings().await.unwrap();
    assert_eq!(gif_settings.klipy_app_key, "test-key");
    ada.add_gif_favourite("happy-cat").await.unwrap();
    ada.add_gif_favourite("sad-dog").await.unwrap();
    assert_eq!(
        ada.list_gif_favourites().await.unwrap(),
        ["sad-dog", "happy-cat"]
    );
    // Encoded as one path segment, so the instance sees the slash and refuses the slug.
    assert_refused(
        ada.add_gif_favourite("sad dog/1").await,
        ApiErrorCode::ValidationFailed,
    );
    ada.remove_gif_favourite("sad-dog").await.unwrap();
    assert_eq!(ada.list_gif_favourites().await.unwrap(), ["happy-cat"]);

    let (guild, general, _) = RunningInstance::guild_with_member(&ada, &bob).await;
    let summaries = bob.list_my_guilds().await.unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].id, guild.id);
    let sent = ada.send_message(general, &text("hello")).await.unwrap();
    bob.acknowledge(general, sent.id).await.unwrap();
    let read_states = bob.list_my_read_states().await.unwrap();
    assert!(
        read_states
            .iter()
            .any(|state| state.channel_id == general && state.last_read_message_id == sent.id)
    );
    // Now they share a guild, Bob sees Ada's profile.
    assert_eq!(bob.user_profile(profile.user_id).await.unwrap(), profile);
    instance.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_factor_sign_in_is_enrolled_completed_and_turned_off() {
    let instance = RunningInstance::start().await;
    let ada = instance.registered("ada").await;
    assert!(!ada.two_factor_status().await.unwrap().is_enabled);
    assert_refused(
        ada.start_two_factor_enrolment(&StartTwoFactorEnrolmentRequest {
            current_password: "wrong password".into(),
        })
        .await,
        ApiErrorCode::InvalidCredentials,
    );
    let enrolment = ada
        .start_two_factor_enrolment(&StartTwoFactorEnrolmentRequest {
            current_password: PASSWORD.into(),
        })
        .await
        .unwrap();
    let secret = base32_decode(&enrolment.secret);
    let code = current_totp_code(&secret);
    // Six digits that are no code of the last, current or next step.
    let wrong_code = format!(
        "{:06}",
        (code.parse::<u32>().unwrap() + 500_000) % 1_000_000
    );
    assert_refused(
        ada.enable_two_factor(&EnableTwoFactorRequest { code: wrong_code })
            .await,
        ApiErrorCode::InvalidTwoFactorCode,
    );
    let recovery_codes = ada
        .enable_two_factor(&EnableTwoFactorRequest { code })
        .await
        .unwrap()
        .recovery_codes;
    assert_eq!(recovery_codes.len(), 10);

    let mut phone = RestClient::new(instance.address.clone()).unwrap();
    let login = LoginRequest {
        login: "ada".into(),
        password: PASSWORD.into(),
        device_name: "phone".into(),
    };
    let Ok(SignIn::TwoFactorRequired(challenge)) = phone.login(&login).await else {
        panic!("the password alone must not sign in");
    };
    assert!(phone.token().is_none());
    assert_refused(
        phone
            .complete_two_factor_sign_in(&CompleteTwoFactorSignInRequest {
                two_factor_token: "mlw_not-a-challenge".into(),
                code: recovery_codes[0].clone(),
            })
            .await,
        ApiErrorCode::Unauthorized,
    );
    phone
        .complete_two_factor_sign_in(&CompleteTwoFactorSignInRequest {
            two_factor_token: challenge.two_factor_token,
            code: recovery_codes[0].clone(),
        })
        .await
        .unwrap();
    assert_eq!(phone.current_user().await.unwrap().username, "ada");

    let new_codes = phone
        .regenerate_recovery_codes(&ProveTwoFactorRequest {
            current_password: PASSWORD.into(),
            code: recovery_codes[1].clone(),
        })
        .await
        .unwrap()
        .recovery_codes;
    // The old codes stopped working with the regeneration.
    assert_refused(
        phone
            .disable_two_factor(&ProveTwoFactorRequest {
                current_password: PASSWORD.into(),
                code: recovery_codes[2].clone(),
            })
            .await,
        ApiErrorCode::InvalidTwoFactorCode,
    );
    phone
        .disable_two_factor(&ProveTwoFactorRequest {
            current_password: PASSWORD.into(),
            code: new_codes[0].clone(),
        })
        .await
        .unwrap();
    assert!(!phone.two_factor_status().await.unwrap().is_enabled);
    let mut laptop = RestClient::new(instance.address.clone()).unwrap();
    assert!(matches!(
        laptop.login(&login).await.unwrap(),
        SignIn::SignedIn(_)
    ));
    instance.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn friends_and_group_conversations_round_trip() {
    let instance = RunningInstance::start().await;
    let ada = instance.registered("ada").await;
    let bob = instance.registered("bob").await;
    let cleo = instance.registered("cleo").await;
    let ada_id = ada.current_user().await.unwrap().id;
    let bob_id = bob.current_user().await.unwrap().id;
    let cleo_user = cleo.current_user().await.unwrap();

    assert_refused(
        ada.send_friend_request(&SendFriendRequest {
            username: "nobody".into(),
            friend_code: None,
        })
        .await,
        ApiErrorCode::NotFound,
    );
    let request = ada
        .send_friend_request(&SendFriendRequest {
            username: "BOB".into(),
            friend_code: None,
        })
        .await
        .unwrap();
    assert_eq!(request.kind, RelationshipKind::OutgoingRequest);
    let accepted = bob
        .set_relationship(ada_id, SetRelationshipKind::Friend)
        .await
        .unwrap();
    assert_eq!(accepted.kind, RelationshipKind::Friend);
    // By friend code this time (ADR-0048).
    ada.send_friend_request(&SendFriendRequest {
        username: String::new(),
        friend_code: cleo_user.friend_code.clone(),
    })
    .await
    .unwrap();
    cleo.set_relationship(ada_id, SetRelationshipKind::Friend)
        .await
        .unwrap();
    let friends = ada.list_relationships().await.unwrap();
    assert_eq!(friends.len(), 2);
    assert!(
        friends
            .iter()
            .all(|friend| friend.kind == RelationshipKind::Friend)
    );

    let group = ada
        .create_group_conversation(&CreateGroupConversationRequest {
            recipient_ids: vec![bob_id, cleo_user.id],
            name: "Trio".into(),
        })
        .await
        .unwrap();
    assert_eq!(group.owner_id, Some(ada_id));
    let renamed = ada
        .rename_group_conversation(group.id, "Quartet minus one")
        .await
        .unwrap();
    assert_eq!(renamed.name, "Quartet minus one");
    // Only the owner removes others.
    assert_refused(
        bob.remove_group_participant(group.id, cleo_user.id).await,
        ApiErrorCode::MissingPermissions,
    );
    ada.remove_group_participant(group.id, cleo_user.id)
        .await
        .unwrap();
    let back = ada
        .add_group_participant(group.id, cleo_user.id)
        .await
        .unwrap();
    // Everyone but the reader.
    assert_eq!(back.recipients.len(), 2);
    bob.remove_group_participant(group.id, bob_id)
        .await
        .unwrap();

    ada.remove_relationship(bob_id).await.unwrap();
    assert_eq!(ada.list_relationships().await.unwrap().len(), 1);
    // Bob is no friend any more: he cannot be added back.
    assert_refused(
        ada.add_group_participant(group.id, bob_id).await,
        ApiErrorCode::NotFound,
    );
    instance.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn guild_settings_invites_search_and_deletion_round_trip() {
    let instance = RunningInstance::start().await;
    let owner = instance.registered("owner").await;
    let member = instance.registered("member").await;
    let (guild, general, code) = RunningInstance::guild_with_member(&owner, &member).await;

    assert_refused(
        member.rename_guild(guild.id, "Mine now").await,
        ApiErrorCode::MissingPermissions,
    );
    assert_eq!(
        owner.rename_guild(guild.id, "Den").await.unwrap().name,
        "Den"
    );

    // `general` leaves its category for the top level, after the category.
    let placements: Vec<ChannelPlacement> = guild
        .channels
        .iter()
        .map(|channel| ChannelPlacement {
            id: channel.id,
            parent_id: if channel.id == general {
                None
            } else {
                channel.parent_id
            },
        })
        .collect();
    let reordered = owner.reorder_channels(guild.id, &placements).await.unwrap();
    let moved = reordered
        .iter()
        .find(|channel| channel.id == general)
        .unwrap();
    assert_eq!(moved.parent_id, None);
    assert_refused(
        owner.reorder_channels(guild.id, &[]).await,
        ApiErrorCode::ValidationFailed,
    );

    let invites = owner.list_invites(guild.id).await.unwrap();
    assert!(invites.iter().any(|invite| invite.code == code));
    assert_refused(
        member.list_invites(guild.id).await,
        ApiErrorCode::MissingPermissions,
    );
    assert_refused(owner.revoke_invite("../x").await, ApiErrorCode::NotFound);
    owner.revoke_invite(&code).await.unwrap();
    assert_eq!(owner.list_invites(guild.id).await.unwrap().len(), 0);

    owner
        .send_message(general, &text("the quick brown fox"))
        .await
        .unwrap();
    owner
        .send_message(general, &text("a lazy dog"))
        .await
        .unwrap();
    let results = member
        .search_guild_messages(
            guild.id,
            &MessageSearchQuery {
                q: "fox".into(),
                sort: None,
                offset: None,
                limit: None,
                utc_offset_minutes: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(results.total_count, 1);
    assert_eq!(results.hits[0].message.content, "the quick brown fox");
    assert_refused(
        member
            .search_guild_messages(
                guild.id,
                &MessageSearchQuery {
                    q: "x".repeat(MAXIMUM_SEARCH_QUERY_LENGTH + 1),
                    sort: None,
                    offset: None,
                    limit: None,
                    utc_offset_minutes: None,
                },
            )
            .await,
        ApiErrorCode::ValidationFailed,
    );

    assert_refused(
        member.delete_guild(guild.id).await,
        ApiErrorCode::MissingPermissions,
    );
    owner.delete_guild(guild.id).await.unwrap();
    assert_refused(owner.guild(guild.id).await, ApiErrorCode::NotFound);
    instance.stop().await;
}

/// Reactions, pins and searching one's conversations.
async fn reactions_pins_and_conversation_search_round_trip(
    owner: &RestClient,
    member: &RestClient,
    general: Snowflake,
) {
    let member_id = member.current_user().await.unwrap().id;
    let owner_id = owner.current_user().await.unwrap().id;
    let sent = owner.send_message(general, &text("react")).await.unwrap();
    member.add_reaction(general, sent.id, "👍").await.unwrap();
    owner.add_reaction(general, sent.id, "👍").await.unwrap();
    let reacting = member
        .list_reacting_users(general, sent.id, "👍", &ReactionUsersQuery::default())
        .await
        .unwrap();
    assert_eq!(reacting.len(), 2);
    assert_refused(
        member
            .remove_user_reaction(general, sent.id, "👍", owner_id)
            .await,
        ApiErrorCode::MissingPermissions,
    );
    owner
        .remove_user_reaction(general, sent.id, "👍", member_id)
        .await
        .unwrap();
    owner
        .remove_own_reaction(general, sent.id, "👍")
        .await
        .unwrap();
    member.add_reaction(general, sent.id, "🎉").await.unwrap();
    owner
        .remove_emoji_reactions(general, sent.id, "🎉")
        .await
        .unwrap();
    member.add_reaction(general, sent.id, "🎉").await.unwrap();
    owner.remove_all_reactions(general, sent.id).await.unwrap();
    let remaining = owner
        .list_reacting_users(general, sent.id, "🎉", &ReactionUsersQuery::default())
        .await
        .unwrap();
    assert_eq!(remaining.len(), 0);

    owner.pin_message(general, sent.id).await.unwrap();
    assert_eq!(member.list_pins(general).await.unwrap()[0].id, sent.id);
    assert_refused(
        member.unpin_message(general, sent.id).await,
        ApiErrorCode::MissingPermissions,
    );
    owner.unpin_message(general, sent.id).await.unwrap();
    assert_eq!(member.list_pins(general).await.unwrap().len(), 0);

    let conversation = member.open_direct_message(owner_id).await.unwrap();
    member
        .send_message(conversation.id, &text("our secret plan"))
        .await
        .unwrap();
    let query = MessageSearchQuery {
        q: "secret".into(),
        sort: None,
        offset: None,
        limit: None,
        utc_offset_minutes: None,
    };
    let results = owner.search_my_conversation_messages(&query).await.unwrap();
    assert_eq!(results.total_count, 1);
    assert_refused(
        owner
            .search_my_conversation_messages(&MessageSearchQuery {
                limit: Some(0),
                ..query
            })
            .await,
        ApiErrorCode::ValidationFailed,
    );
}

/// Polls, checklists and progress bars (ADR-0043, ADR-0047).
#[allow(clippy::too_many_lines)] // One scenario per kind of rich message, in order.
async fn polls_checklists_and_progress_bars_round_trip(
    owner: &RestClient,
    member: &RestClient,
    general: Snowflake,
) {
    let poll_message: Message = owner
        .send_message(
            general,
            &SendMessageRequest {
                poll: Some(NewPoll {
                    question: "Tea or coffee?".into(),
                    answers: vec!["Tea".into(), "Coffee".into()],
                    allows_multiple_answers: false,
                    duration_hours: POLL_DURATIONS_IN_HOURS[0],
                }),
                ..text("")
            },
        )
        .await
        .unwrap();
    let tea = poll_message.poll.as_ref().unwrap().answers[0].id;
    member
        .add_poll_vote(general, poll_message.id, tea)
        .await
        .unwrap();
    let voters = owner
        .list_poll_voters(
            general,
            poll_message.id,
            tea,
            &ReactionUsersQuery::default(),
        )
        .await
        .unwrap();
    assert_eq!(voters.len(), 1);
    member
        .remove_poll_vote(general, poll_message.id, tea)
        .await
        .unwrap();
    assert_refused(
        member.end_poll(general, poll_message.id).await,
        ApiErrorCode::MissingPermissions,
    );
    owner.end_poll(general, poll_message.id).await.unwrap();
    assert!(
        member
            .add_poll_vote(general, poll_message.id, tea)
            .await
            .is_err(),
        "an ended poll takes no vote"
    );

    let checklist_message = owner
        .send_message(
            general,
            &SendMessageRequest {
                checklist: Some(NewChecklist {
                    title: "Chores".into(),
                    items: vec!["Dishes".into()],
                }),
                ..text("")
            },
        )
        .await
        .unwrap();
    let dishes = checklist_message.checklist.as_ref().unwrap().items[0].id;
    let checklist = owner
        .add_checklist_item(general, checklist_message.id, "Laundry")
        .await
        .unwrap();
    assert_eq!(checklist.items.len(), 2);
    let laundry = checklist.items[1].id;
    let ticked = member
        .set_checklist_item_ticked(general, checklist_message.id, dishes, true)
        .await
        .unwrap();
    assert!(ticked.items[0].checked.is_some());
    let unticked = member
        .set_checklist_item_ticked(general, checklist_message.id, dishes, false)
        .await
        .unwrap();
    assert!(unticked.items[0].checked.is_none());
    let renamed = owner
        .rename_checklist_item(general, checklist_message.id, laundry, "Ironing")
        .await
        .unwrap();
    assert_eq!(renamed.items[1].text, "Ironing");
    owner
        .remove_checklist_item(general, checklist_message.id, laundry)
        .await
        .unwrap();
    assert_refused(
        owner
            .remove_checklist_item(general, checklist_message.id, dishes)
            .await,
        ApiErrorCode::ValidationFailed,
    );

    let bar_message = owner
        .send_message(
            general,
            &SendMessageRequest {
                progress_bar: Some(NewProgressBar {
                    label: "Saving".into(),
                    value: 1,
                    maximum: 10,
                }),
                ..text("")
            },
        )
        .await
        .unwrap();
    let bar = owner
        .update_progress_bar(
            general,
            bar_message.id,
            &NewProgressBar {
                label: "Saving".into(),
                value: 5,
                maximum: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(bar.value, 5);
    assert_refused(
        owner
            .update_progress_bar(
                general,
                bar_message.id,
                &NewProgressBar {
                    label: "Saving".into(),
                    value: 11,
                    maximum: 10,
                },
            )
            .await,
        ApiErrorCode::ValidationFailed,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rich_messages_round_trip() {
    let instance = RunningInstance::start().await;
    let owner = instance.registered("owner").await;
    let member = instance.registered("member").await;
    let (_, general, _) = RunningInstance::guild_with_member(&owner, &member).await;
    reactions_pins_and_conversation_search_round_trip(&owner, &member, general).await;
    polls_checklists_and_progress_bars_round_trip(&owner, &member, general).await;
    instance.stop().await;
}

/// Attachments: progress, download, revalidation, the caller's cap and the instance's limit.
async fn attachments_round_trip(owner: &RestClient, stranger: &RestClient, general: Snowflake) {
    let file: Vec<u8> = (0..=u8::MAX).cycle().take(200 * 1024).collect();
    let reports = Arc::new(Mutex::new(Vec::<UploadProgress>::new()));
    let recorder = Arc::clone(&reports);
    let attachment = owner
        .upload_attachment_reporting_progress(
            general,
            "notes été.bin",
            file.clone(),
            move |progress| {
                recorder.lock().unwrap().push(progress);
            },
        )
        .await
        .unwrap();
    assert_eq!(attachment.file_name, "notes été.bin");
    assert_eq!(attachment.size_bytes, file.len() as u64);
    let reports = reports.lock().unwrap().clone();
    assert!(reports.len() > 1, "one report per chunk");
    assert!(
        reports
            .windows(2)
            .all(|pair| pair[0].sent_bytes < pair[1].sent_bytes)
    );
    assert_eq!(reports.last().unwrap().sent_bytes, file.len() as u64);
    // Readable by the channel's members once a message carries it.
    owner
        .send_message(
            general,
            &SendMessageRequest {
                attachment_ids: vec![attachment.id],
                ..text("")
            },
        )
        .await
        .unwrap();

    let Download::Fetched(downloaded) = owner
        .download(
            DownloadableFile::Attachment(attachment.id),
            None,
            1024 * 1024,
        )
        .await
        .unwrap()
    else {
        panic!("a first download has no cached copy");
    };
    assert_eq!(downloaded.bytes, file);
    let entity_tag = downloaded.entity_tag.unwrap();
    assert_eq!(
        owner
            .download(
                DownloadableFile::Attachment(attachment.id),
                Some(&entity_tag),
                1024 * 1024
            )
            .await
            .unwrap(),
        Download::NotModified
    );
    assert!(matches!(
        owner
            .download(DownloadableFile::Attachment(attachment.id), None, 1024)
            .await,
        Err(ClientError::FileTooLarge)
    ));
    assert_refused(
        stranger
            .download(
                DownloadableFile::Attachment(attachment.id),
                None,
                1024 * 1024,
            )
            .await,
        ApiErrorCode::NotFound,
    );
    let too_large = vec![0_u8; usize::try_from(MAXIMUM_ATTACHMENT_BYTES).unwrap() + 1];
    assert!(matches!(
        owner.upload_attachment(general, "big.bin", too_large).await,
        Err(ClientError::FileTooLarge)
    ));
}

/// Avatar, banner, guild, channel and role icons: set, served, removed.
#[allow(clippy::too_many_lines)] // Every kind of picture: set, served, refused, removed.
async fn pictures_round_trip(owner: &RestClient, stranger: &RestClient, guild: &Guild) {
    let me = owner
        .set_avatar(png(64, 64), &IconCropQuery::default())
        .await
        .unwrap();
    let avatar = me.avatar.unwrap();
    for file in [
        DownloadableFile::ProfileImage(avatar.id),
        DownloadableFile::ProfileImageStill(avatar.id),
    ] {
        let Download::Fetched(picture) = owner.download(file, None, 1024 * 1024).await.unwrap()
        else {
            panic!("profile images carry no entity tag to revalidate");
        };
        assert_ne!(picture.bytes.len(), 0);
    }
    assert_refused(
        owner
            .set_avatar(
                png(64, 64),
                &IconCropQuery {
                    crop_x: Some(0),
                    crop_y: None,
                    crop_size: None,
                },
            )
            .await,
        ApiErrorCode::ValidationFailed,
    );
    assert!(owner.remove_avatar().await.unwrap().avatar.is_none());
    let banner_owner = owner
        .set_banner(png(250, 100), &BannerCropQuery::default())
        .await
        .unwrap();
    assert!(banner_owner.banner.is_some());
    assert!(owner.remove_banner().await.unwrap().banner.is_none());

    let with_icon = owner
        .set_guild_icon(guild.id, png(64, 64), &IconCropQuery::default())
        .await
        .unwrap();
    let icon = with_icon.icon.unwrap();
    assert!(matches!(
        owner
            .download(DownloadableFile::Icon(icon.id), None, 1024 * 1024)
            .await
            .unwrap(),
        Download::Fetched(_)
    ));
    assert_refused(
        stranger
            .download(DownloadableFile::Icon(icon.id), None, 1024 * 1024)
            .await,
        ApiErrorCode::NotFound,
    );
    assert!(
        owner
            .remove_guild_icon(guild.id)
            .await
            .unwrap()
            .icon
            .is_none()
    );

    let channel = guild
        .channels
        .iter()
        .find(|channel| channel.kind == ChannelKind::Text)
        .unwrap();
    assert!(
        owner
            .set_channel_icon(channel.id, png(64, 64), &IconCropQuery::default())
            .await
            .unwrap()
            .icon
            .is_some()
    );
    assert!(
        owner
            .remove_channel_icon(channel.id)
            .await
            .unwrap()
            .icon
            .is_none()
    );

    let role = owner
        .create_role(
            guild.id,
            &CreateRoleRequest {
                name: "Artists".into(),
                color: 0,
                permissions: Permissions::empty(),
                is_displayed_separately: false,
                is_mentionable: false,
            },
        )
        .await
        .unwrap();
    let pictured = owner
        .set_role_icon_picture(guild.id, role.id, png(64, 64), &IconCropQuery::default())
        .await
        .unwrap();
    assert!(pictured.icon.is_some());
    let emoji_role = owner
        .set_role_icon_emoji(guild.id, role.id, "🎨")
        .await
        .unwrap();
    assert_eq!(emoji_role.unicode_emoji.as_deref(), Some("🎨"));
    assert!(emoji_role.icon.is_none());
    assert_refused(
        stranger.remove_role_icon(guild.id, role.id).await,
        ApiErrorCode::NotFound,
    );
    let plain = owner.remove_role_icon(guild.id, role.id).await.unwrap();
    assert!(plain.icon.is_none() && plain.unicode_emoji.is_none());
}

/// Custom emoji and soundboard sounds (ADR-0028, ADR-0032).
async fn emoji_and_sounds_round_trip(owner: &RestClient, member: &RestClient, guild_id: Snowflake) {
    let emoji = owner
        .create_emoji(guild_id, "party", png(32, 32))
        .await
        .unwrap();
    assert!(matches!(
        member
            .download(DownloadableFile::IconStill(emoji.id), None, 1024 * 1024)
            .await
            .unwrap(),
        Download::Fetched(_)
    ));
    assert_eq!(
        member.list_emojis(guild_id).await.unwrap(),
        std::slice::from_ref(&emoji)
    );
    assert_eq!(
        owner
            .rename_emoji(guild_id, emoji.id, "fiesta")
            .await
            .unwrap()
            .name,
        "fiesta"
    );
    assert_refused(
        member.delete_emoji(guild_id, emoji.id).await,
        ApiErrorCode::MissingPermissions,
    );
    owner.delete_emoji(guild_id, emoji.id).await.unwrap();
    let huge = owner
        .create_emoji(guild_id, "huge", vec![0; 300 * 1024])
        .await;
    assert!(matches!(huge, Err(ClientError::FileTooLarge)), "{huge:?}");

    let wav = canonical_wav(4_800);
    let sound = owner
        .create_sound(
            guild_id,
            &CreateSoundQuery {
                name: "boop".into(),
                emoji: Some("🔔".into()),
            },
            wav.clone(),
        )
        .await
        .unwrap();
    assert_eq!(sound.duration_milliseconds, 100);
    let Download::Fetched(served) = member
        .download(DownloadableFile::Sound(sound.id), None, 1024 * 1024)
        .await
        .unwrap()
    else {
        panic!("sounds carry no entity tag to revalidate");
    };
    assert_eq!(served.bytes, wav);
    assert_eq!(served.content_type.as_deref(), Some("audio/wav"));
    assert_eq!(member.list_sounds(guild_id).await.unwrap().len(), 1);
    let updated = owner
        .update_sound(
            guild_id,
            sound.id,
            &UpdateSoundRequest {
                name: "beep".into(),
                emoji: None,
            },
        )
        .await
        .unwrap();
    assert_eq!((updated.name.as_str(), updated.emoji), ("beep", None));
    member
        .set_join_sound(guild_id, Some(sound.id))
        .await
        .unwrap();
    member.set_join_sound(guild_id, None).await.unwrap();
    assert_refused(
        owner
            .create_sound(
                guild_id,
                &CreateSoundQuery {
                    name: "noise".into(),
                    emoji: None,
                },
                b"not a wav file".to_vec(),
            )
            .await,
        ApiErrorCode::ValidationFailed,
    );
    owner.delete_sound(guild_id, sound.id).await.unwrap();
    assert_refused(
        member.set_join_sound(guild_id, Some(sound.id)).await,
        ApiErrorCode::NotFound,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn files_upload_with_progress_and_download_with_entity_tags() {
    let instance = RunningInstance::start().await;
    let owner = instance.registered("owner").await;
    let member = instance.registered("member").await;
    let stranger = instance.registered("stranger").await;
    let (guild, general, _) = RunningInstance::guild_with_member(&owner, &member).await;
    attachments_round_trip(&owner, &stranger, general).await;
    pictures_round_trip(&owner, &stranger, &guild).await;
    // Another owner: every picture change counts against one budget per user (10 in 10 minutes).
    let (sound_guild, _, _) = RunningInstance::guild_with_member(&stranger, &member).await;
    emoji_and_sounds_round_trip(&stranger, &member, sound_guild.id).await;
    instance.stop().await;
}
