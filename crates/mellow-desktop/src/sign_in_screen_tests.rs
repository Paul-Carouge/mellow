//! The sign-in flow in a headless GPUI window against a real in-process `mellow-server`.

// The server refuses Windows until its master key file can be protected there (ADR-0006).
#![cfg(not(windows))]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{Entity, TestAppContext, VisualTestContext, WindowAppearance};
use mellow_client_core::{GatewayUpdate, InstanceAddress, RestClient};
use mellow_protocol::{EnableTwoFactorRequest, RegisterRequest, StartTwoFactorEnrolmentRequest};
use mellow_server::commands::{InitializeOptions, initialize_instance, serve_instance_on_listener};
use mellow_server::domain::account_rules::RegistrationMode;
use tokio::runtime::Runtime;
use tokio::sync::oneshot;

use super::*;
use crate::main_window::MainWindow;
use crate::session::{ConnectionStatus, SessionPhase};
use crate::strings_pending_i18n::{ActiveStrings, ENGLISH};
use crate::theme::ActiveTheme;
use crate::tokio_bridge::{build_network_runtime, install_network_runtime};

const PASSWORD: &str = "correct horse battery staple";
const MOCK_KLIPY_APP_KEY: &str = "mock-key";
/// How long a step may take against the local server before the test fails; never a measure.
const STEP_DEADLINE: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

struct RunningInstance {
    _directory: tempfile::TempDir,
    address: InstanceAddress,
    stop: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl RunningInstance {
    async fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        // Kept bound and handed to the server: a probed port could be taken by another test first.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_address = listener.local_addr().unwrap();
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
        // The outbox test creates a server with a plain account.
        config.guild_creation = mellow_server::domain::account_rules::GuildCreationPolicy::Everyone;
        // GIFs on (ADR-0042): the session asks for Klipy's key at `Ready`; nothing reaches Klipy.
        config.gifs.klipy_app_key = Some(MOCK_KLIPY_APP_KEY.to_owned());
        let (stop, stopped) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            serve_instance_on_listener(&config, listener, async {
                let _ = stopped.await;
            })
            .await
        });
        Self {
            _directory: directory,
            address: InstanceAddress::parse(&format!("http://{listen_address}")).unwrap(),
            stop,
            server,
        }
    }

    /// Waits for the database to close: SQLCipher crashes if it is still closing at exit.
    async fn stop(self) {
        let _ = self.stop.send(());
        self.server.await.unwrap().unwrap();
    }
}

struct Harness {
    runtime: Runtime,
    instance: RunningInstance,
    session: Entity<Session>,
    main_window: Entity<MainWindow>,
}

fn start(cx: &mut TestAppContext) -> (Harness, &mut VisualTestContext) {
    let runtime = build_network_runtime().unwrap();
    let instance = runtime.block_on(RunningInstance::start());
    // Answers come from Tokio threads, outside GPUI's deterministic scheduler.
    cx.executor().allow_parking();
    let handle = runtime.handle().clone();
    cx.update(|cx| {
        gpui_component::init(cx);
        install_network_runtime(handle, cx);
        cx.set_global(ActiveStrings(&ENGLISH));
        cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
    });
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    let window_session = session.clone();
    let (main_window, cx) =
        cx.add_window_view(|window, cx| MainWindow::new(window_session, window, cx));
    (
        Harness {
            runtime,
            instance,
            session,
            main_window,
        },
        cx,
    )
}

impl Harness {
    fn screen(&self, cx: &mut VisualTestContext) -> Entity<SignInScreen> {
        self.main_window
            .read_with(cx, |main_window, _| main_window.sign_in_screen().cloned())
            .expect("the sign-in screen shows")
    }

    fn type_into(&self, field: SignInField, text: &str, cx: &mut VisualTestContext) {
        let screen = self.screen(cx);
        screen.update_in(cx, |screen, window, cx| {
            screen.set_value(field, text, window, cx);
        });
    }

    fn submit(&self, cx: &mut VisualTestContext) {
        let screen = self.screen(cx);
        screen.update_in(cx, SignInScreen::submit);
    }

    fn read_screen<R>(
        &self,
        cx: &mut VisualTestContext,
        read: impl FnOnce(&SignInScreen) -> R,
    ) -> R {
        self.screen(cx).read_with(cx, |screen, _| read(screen))
    }

    fn stop(self) {
        self.runtime.block_on(self.instance.stop());
        self.runtime.shutdown_background();
    }
}

/// Runs GPUI until `condition` holds, polling while the network answers on Tokio threads.
fn wait_until(
    cx: &mut VisualTestContext,
    mut condition: impl FnMut(&mut VisualTestContext) -> bool,
) {
    let deadline = std::time::Instant::now() + STEP_DEADLINE;
    loop {
        cx.run_until_parked();
        if condition(cx) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the step did not finish in time"
        );
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn wait_until_idle(harness: &Harness, cx: &mut VisualTestContext) {
    wait_until(cx, |cx| !harness.read_screen(cx, |screen| screen.is_busy));
}

#[test]
fn the_instance_field_keeps_http_but_not_https() {
    let loopback = InstanceAddress::parse("http://127.0.0.1:8080").unwrap();
    let public = InstanceAddress::parse("chat.example.org").unwrap();
    assert_eq!(instance_field_value(&loopback), "http://127.0.0.1:8080");
    assert_eq!(instance_field_value(&public), "chat.example.org");
    // What the field shows parses back to the same instance.
    assert_eq!(
        InstanceAddress::parse(instance_field_value(&loopback)).unwrap(),
        loopback
    );
}

fn is_on_credentials_step(screen: &SignInScreen) -> bool {
    matches!(screen.step, Step::Credentials { .. })
}

#[gpui::test]
fn the_instance_address_is_checked_then_fields_then_credentials(cx: &mut TestAppContext) {
    let (harness, cx) = start(cx);
    assert!(harness.read_screen(cx, |screen| matches!(screen.step, Step::InstanceAddress)));

    harness.submit(cx);
    assert_eq!(
        harness.read_screen(cx, |screen| screen.field_errors.clone()),
        [(
            SignInField::InstanceAddress,
            ENGLISH.instance_address_missing
        )]
    );
    harness.type_into(SignInField::InstanceAddress, "http://chat.example.org", cx);
    harness.submit(cx);
    assert_eq!(
        harness.read_screen(cx, |screen| screen.field_errors.clone()),
        [(
            SignInField::InstanceAddress,
            ENGLISH.instance_address_invalid
        )]
    );

    let address = harness.instance.address.as_str().to_owned();
    harness.type_into(SignInField::InstanceAddress, &address, cx);
    harness.submit(cx);
    wait_until_idle(&harness, cx);
    assert!(harness.read_screen(cx, is_on_credentials_step));

    harness.submit(cx);
    assert_eq!(
        harness.read_screen(cx, |screen| screen.field_errors.clone()),
        [
            (SignInField::Login, ENGLISH.login_missing),
            (SignInField::Password, ENGLISH.password_missing)
        ]
    );

    harness.type_into(SignInField::Login, "nobody", cx);
    harness.type_into(SignInField::Password, "wrong password", cx);
    harness.submit(cx);
    wait_until_idle(&harness, cx);
    assert_eq!(
        harness.read_screen(cx, |screen| screen.error_message.clone()),
        Some("Couldn't sign in: the login or the password is wrong.".to_owned())
    );

    // Registering signs in: the gateway's `Ready` connects the session and the card goes.
    harness.screen(cx).update_in(cx, SignInScreen::switch_mode);
    harness.type_into(SignInField::Email, "ada", cx);
    harness.type_into(SignInField::Username, "ada", cx);
    harness.type_into(SignInField::NewPassword, PASSWORD, cx);
    harness.submit(cx);
    assert_eq!(
        harness.read_screen(cx, |screen| screen.field_errors.clone()),
        [(SignInField::Email, ENGLISH.email_malformed)]
    );
    harness.type_into(SignInField::Email, "ada@example.org", cx);
    harness.submit(cx);
    wait_until(cx, |cx| {
        harness.session.read_with(cx, |session, _| {
            session.status() == ConnectionStatus::Connected
        })
    });
    assert!(
        harness
            .main_window
            .read_with(cx, |main_window, _| main_window.sign_in_screen().is_none())
    );
    // The instance has GIFs: the GIF button and Klipy addresses as GIFs follow.
    wait_until(cx, |cx| {
        harness
            .session
            .read_with(cx, |session, _| session.gif_settings().is_some())
    });
    harness.session.read_with(cx, |session, _| {
        assert_eq!(
            session
                .gif_settings()
                .map(|settings| settings.klipy_app_key.as_str()),
            Some(MOCK_KLIPY_APP_KEY)
        );
    });
    harness.session.read_with(cx, |session, _| {
        assert_eq!(
            session
                .state()
                .current_user()
                .map(|user| user.username.as_str()),
            Some("ada")
        );
    });

    // Signing out reaches the instance and comes back to the sign-in card of the same instance.
    harness.session.update(cx, Session::sign_out);
    wait_until(cx, |cx| {
        harness
            .session
            .read_with(cx, |session, _| session.phase() == SessionPhase::SignedOut)
    });
    assert!(harness.read_screen(cx, |screen| {
        is_on_credentials_step(screen) && !screen.has_session_ended_by_itself
    }));
    harness.stop();
}

/// The authenticator code of `secret` (RFC 6238); the instance accepts one step of drift.
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
        bits = (bits << 5) | u64::try_from(value).unwrap();
        bit_count += 5;
        if bit_count >= 8 {
            bit_count -= 8;
            bytes.push(u8::try_from((bits >> bit_count) & 0xFF).unwrap());
        }
    }
    bytes
}

/// Registers `ada` and turns two-factor sign-in on; the recovery codes.
async fn account_with_two_factor(instance: &RunningInstance) -> Vec<String> {
    let mut ada = RestClient::new(instance.address.clone()).unwrap();
    ada.register(&RegisterRequest {
        username: "ada".into(),
        display_name: None,
        email: "ada@example.org".into(),
        password: PASSWORD.into(),
        device_name: "test".into(),
    })
    .await
    .unwrap();
    let enrolment = ada
        .start_two_factor_enrolment(&StartTwoFactorEnrolmentRequest {
            current_password: PASSWORD.into(),
        })
        .await
        .unwrap();
    let code = current_totp_code(&base32_decode(&enrolment.secret));
    ada.enable_two_factor(&EnableTwoFactorRequest { code })
        .await
        .unwrap()
        .recovery_codes
}

#[gpui::test]
fn two_factor_sign_in_asks_for_a_code_then_signs_in_with_a_recovery_code(cx: &mut TestAppContext) {
    let (harness, cx) = start(cx);
    let recovery_codes = harness
        .runtime
        .block_on(account_with_two_factor(&harness.instance));
    let address = harness.instance.address.as_str().to_owned();
    harness.type_into(SignInField::InstanceAddress, &address, cx);
    harness.submit(cx);
    wait_until_idle(&harness, cx);

    harness.type_into(SignInField::Login, "ada", cx);
    harness.type_into(SignInField::Password, PASSWORD, cx);
    harness.submit(cx);
    wait_until_idle(&harness, cx);
    assert!(harness.read_screen(cx, |screen| matches!(
        screen.step,
        Step::TwoFactor {
            is_using_recovery_code: false,
            ..
        }
    )));
    // The password is dropped from the form once it has done its part.
    let screen = harness.screen(cx);
    assert!(screen.read_with(cx, |screen, cx| {
        screen.value(SignInField::Password, cx).is_empty()
    }));

    harness.submit(cx);
    assert_eq!(
        harness.read_screen(cx, |screen| screen.field_errors.clone()),
        [(SignInField::Code, ENGLISH.two_factor_code_missing)]
    );
    screen.update_in(cx, SignInScreen::switch_code_kind);
    harness.type_into(SignInField::Code, "aaaa-bbbb-cccc-dddd", cx);
    harness.submit(cx);
    wait_until_idle(&harness, cx);
    assert_eq!(
        harness.read_screen(cx, |screen| screen.error_message.clone()),
        Some("Couldn't sign in: the code is wrong, already used or too old.".to_owned())
    );

    harness.type_into(SignInField::Code, &recovery_codes[0], cx);
    harness.submit(cx);
    wait_until(cx, |cx| {
        harness.session.read_with(cx, |session, _| {
            session.status() == ConnectionStatus::Connected
        })
    });
    harness.stop();
}

#[gpui::test]
fn failed_messages_are_sent_again_when_the_gateway_comes_back(cx: &mut TestAppContext) {
    let (harness, cx) = start(cx);
    let rest = harness.runtime.block_on(async {
        let mut rest = RestClient::new(harness.instance.address.clone()).unwrap();
        rest.register(&RegisterRequest {
            username: "ada".into(),
            display_name: None,
            email: "ada@example.org".into(),
            password: PASSWORD.into(),
            device_name: "test".into(),
        })
        .await
        .unwrap();
        rest.create_guild("Home").await.unwrap();
        rest
    });
    harness
        .session
        .update(cx, |session, cx| session.begin_signed_in_session(rest, cx));
    wait_until(cx, |cx| {
        harness.session.read_with(cx, |session, _| {
            session.status() == ConnectionStatus::Connected
        })
    });
    let channel_id = harness.session.read_with(cx, |session, _| {
        session.state().guilds()[0]
            .channels
            .iter()
            .find(|channel| channel.kind == mellow_protocol::ChannelKind::Text)
            .unwrap()
            .id
    });

    // A message whose send failed while the connection was down.
    harness.session.update(cx, |session, _| {
        let state = session.state_mut();
        state.queue_message(channel_id, "hello again".into(), None, "nonce-1".into());
        state.mark_send_failed("nonce-1");
    });
    harness.session.update(cx, |session, cx| {
        session.apply_gateway_updates_of_this_session(
            vec![GatewayUpdate::Disconnected, GatewayUpdate::Resumed],
            cx,
        );
    });
    wait_until(cx, |cx| {
        harness.session.read_with(cx, |session, _| {
            session
                .state()
                .pending_messages(channel_id)
                .next()
                .is_none()
        })
    });
    harness.session.read_with(cx, |session, _| {
        let contents: Vec<&str> = session
            .state()
            .messages(channel_id)
            .iter()
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(contents, ["hello again"]);
    });
    harness.stop();
}

#[gpui::test]
fn a_signed_in_reader_opens_their_channel_writes_and_sends(cx: &mut TestAppContext) {
    let (harness, cx) = start(cx);
    cx.update(|window, cx| {
        crate::components::bind_tab_navigation(cx);
        crate::composer::bind_composer_keys(cx);
        // Only a window in front marks what it shows as read.
        window.activate_window();
    });
    let rest = harness.runtime.block_on(async {
        let mut rest = RestClient::new(harness.instance.address.clone()).unwrap();
        rest.register(&RegisterRequest {
            username: "grace".into(),
            display_name: None,
            email: "grace@example.org".into(),
            password: PASSWORD.into(),
            device_name: "test".into(),
        })
        .await
        .unwrap();
        rest.create_guild("Home").await.unwrap();
        rest
    });
    harness
        .session
        .update(cx, |session, cx| session.begin_signed_in_session(rest, cx));
    let conversation = |cx: &mut VisualTestContext| {
        harness.main_window.read_with(cx, |main_window, cx| {
            main_window
                .shell()
                .map(|shell| shell.read(cx).conversation().clone())
        })
    };
    // The shell opens the server's first text channel, whose history comes from the instance.
    wait_until(cx, |cx| {
        conversation(cx)
            .is_some_and(|view| view.read_with(cx, |view, _| view.open_channel_id().is_some()))
    });
    let view = conversation(cx).unwrap();
    let channel_id = view
        .read_with(cx, |view, _| view.open_channel_id())
        .unwrap();
    wait_until(cx, |cx| {
        harness.session.read_with(cx, |session, _| {
            !session.state().has_older_history(channel_id)
        })
    });

    let composer = view.read_with(cx, |view, _| view.composer().clone());
    cx.update(|window, cx| {
        composer.update(cx, |composer, cx| {
            composer.type_text("hello from the desktop", window, cx);
        });
    });
    cx.simulate_keystrokes("enter");
    wait_until(cx, |cx| {
        harness.session.read_with(cx, |session, _| {
            let state = session.state();
            state.pending_messages(channel_id).next().is_none()
                && state
                    .messages(channel_id)
                    .iter()
                    .any(|message| message.content == "hello from the desktop")
        })
    });

    // Shown at the latest message, the channel is read, and the position goes out once.
    wait_until(cx, |cx| {
        harness.session.read_with(cx, |session, _| {
            session.is_read_state_send_scheduled()
                || session.state().last_read_message_id(channel_id)
                    == session
                        .state()
                        .messages(channel_id)
                        .last()
                        .map(|message| message.id)
        })
    });
    cx.executor()
        .advance_clock(crate::session::READ_STATE_SEND_DELAY);
    wait_until(cx, |cx| {
        harness
            .session
            .read_with(cx, |session, _| !session.is_read_state_send_scheduled())
    });

    // A reconnection past the resume window ends in a fresh `Ready`, which empties the cache:
    // the open channel asks for its history again instead of loading forever.
    let ready = harness.session.read_with(cx, |session, _| {
        fresh_ready(&harness.runtime, session.rest_client().unwrap())
    });
    harness.session.update(cx, |session, cx| {
        session.apply_gateway_updates_of_this_session(vec![ready], cx);
    });
    wait_until(cx, |cx| {
        harness.session.read_with(cx, |session, _| {
            session
                .state()
                .messages(channel_id)
                .iter()
                .any(|message| message.content == "hello from the desktop")
        })
    });
    harness.stop();
}

/// The `Ready` a new gateway session of `rest`'s account receives, as after a server restart.
fn fresh_ready(runtime: &Runtime, rest: &RestClient) -> GatewayUpdate {
    let instance = rest.instance().clone();
    let token = zeroize::Zeroizing::new(rest.token().unwrap().to_owned());
    runtime.block_on(async move {
        let (updates, mut received) = tokio::sync::mpsc::channel(8);
        let (_handle, commands) = mellow_client_core::GatewayHandle::new();
        let gateway = tokio::spawn(mellow_client_core::run_gateway(
            instance, token, updates, commands,
        ));
        let ready = tokio::time::timeout(STEP_DEADLINE, received.recv())
            .await
            .unwrap()
            .unwrap();
        gateway.abort();
        assert!(matches!(ready, GatewayUpdate::Ready(_)));
        ready
    })
}

/// Registers `name` on the instance and returns its signed-in REST client.
fn register(harness: &Harness, name: &str) -> RestClient {
    harness.runtime.block_on(async {
        let mut rest = RestClient::new(harness.instance.address.clone()).unwrap();
        rest.register(&RegisterRequest {
            username: name.into(),
            display_name: None,
            email: format!("{name}@example.org"),
            password: PASSWORD.into(),
            device_name: "test".into(),
        })
        .await
        .unwrap();
        rest
    })
}

/// Runs one session request and waits for its answer.
fn answer_of<T: 'static>(
    harness: &Harness,
    cx: &mut VisualTestContext,
    request: impl FnOnce(
        &mut Session,
        &mut gpui::Context<Session>,
        Box<dyn FnOnce(Result<T, mellow_client_core::ClientError>, &mut gpui::App)>,
    ),
) -> Result<T, mellow_client_core::ClientError> {
    let answer = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = answer.clone();
    harness.session.update(cx, |session, cx| {
        request(
            session,
            cx,
            Box::new(move |result, _| *slot.borrow_mut() = Some(result)),
        );
    });
    wait_until(cx, |_| answer.borrow().is_some());
    answer.borrow_mut().take().unwrap()
}

#[gpui::test]
fn friends_and_groups_change_through_the_session_and_stay_in_step(cx: &mut TestAppContext) {
    use mellow_protocol::{RelationshipKind, SendFriendRequest, SetRelationshipKind};

    let (harness, cx) = start(cx);
    let ada = register(&harness, "ada");
    let bob = register(&harness, "bob");
    let cleo = register(&harness, "cleo");
    harness
        .session
        .update(cx, |session, cx| session.begin_signed_in_session(ada, cx));
    wait_until(cx, |cx| {
        harness.session.read_with(cx, |session, _| {
            session.status() == ConnectionStatus::Connected
        })
    });

    // Ada asks Bob by name; Cleo asks Ada, who accepts.
    let sent = answer_of(&harness, cx, |session, cx, on_done| {
        let request = SendFriendRequest {
            username: "Bob".into(),
            friend_code: None,
        };
        session.send_friend_request(request, cx, on_done);
    })
    .unwrap();
    assert_eq!(sent.kind, RelationshipKind::OutgoingRequest);
    let ada_id = harness
        .session
        .read_with(cx, |session, _| session.state().current_user().unwrap().id);
    harness.runtime.block_on(async {
        bob.set_relationship(ada_id, SetRelationshipKind::Friend)
            .await
            .unwrap();
        cleo.send_friend_request(&SendFriendRequest {
            username: "ada".into(),
            friend_code: None,
        })
        .await
        .unwrap();
    });
    wait_until(cx, |cx| {
        harness.session.read_with(cx, |session, _| {
            session.state().incoming_request_count() == 1
                && session.state().relationship(sent.user.id).map(|r| r.kind)
                    == Some(RelationshipKind::Friend)
        })
    });
    let cleo_id = harness.session.read_with(cx, |session, _| {
        session
            .state()
            .relationships_of_kind(RelationshipKind::IncomingRequest)[0]
            .user
            .id
    });
    answer_of(&harness, cx, |session, cx, on_done| {
        session.set_relationship(cleo_id, SetRelationshipKind::Friend, cx, on_done);
    })
    .unwrap();
    // Removing a relationship that is already gone counts as done.
    answer_of(&harness, cx, |session, cx, on_done| {
        session.remove_relationship(mellow_protocol::Snowflake::from_raw(404), cx, on_done);
    })
    .unwrap();

    // A named group with both, renamed back to its people's names, then left.
    let group_id = answer_of(&harness, cx, |session, cx, on_done| {
        session.create_group_conversation(vec![sent.user.id, cleo_id], " Book club ", cx, on_done);
    })
    .unwrap();
    let name_of = |cx: &mut VisualTestContext| {
        harness.session.read_with(cx, |session, _| {
            session
                .state()
                .direct_message_channel(group_id)
                .map(|group| group.name.clone())
        })
    };
    assert_eq!(name_of(cx).as_deref(), Some("Book club"));
    answer_of(&harness, cx, |session, cx, on_done| {
        session.rename_group_conversation(group_id, "  ", cx, on_done);
    })
    .unwrap();
    assert_eq!(name_of(cx).as_deref(), Some(""));
    answer_of(&harness, cx, |session, cx, on_done| {
        session.leave_group_conversation(group_id, cx, on_done);
    })
    .unwrap();
    assert_eq!(name_of(cx), None);
    harness.stop();
}

#[gpui::test]
fn open_direct_conversation_shows_it_with_the_composer_focused_or_says_why_not(
    cx: &mut TestAppContext,
) {
    use mellow_protocol::{SendFriendRequest, SetRelationshipKind, Snowflake};

    use crate::home::FriendsPane;
    use crate::navigation::Place;
    use crate::shell::OpenDirectConversation;

    let (harness, cx) = start(cx);
    let ada = register(&harness, "ada");
    let bob = register(&harness, "bob");
    let bob_id = harness.runtime.block_on(async {
        let bob_id = bob.current_user().await.unwrap().id;
        bob.send_friend_request(&SendFriendRequest {
            username: "ada".into(),
            friend_code: None,
        })
        .await
        .unwrap();
        ada.set_relationship(bob_id, SetRelationshipKind::Friend)
            .await
            .unwrap();
        bob_id
    });
    harness
        .session
        .update(cx, |session, cx| session.begin_signed_in_session(ada, cx));
    wait_until(cx, |cx| {
        harness
            .main_window
            .read_with(cx, |main_window, _| main_window.shell().is_some())
            && harness.session.read_with(cx, |session, _| {
                session.status() == ConnectionStatus::Connected
            })
    });
    let shell = harness
        .main_window
        .read_with(cx, |main_window, _| main_window.shell().cloned())
        .unwrap();
    // As from a menu in the Friends space: the action bubbles up to the shell.
    let pane = shell.read_with(cx, |shell, cx| shell.home().read(cx).friends_pane().clone());
    pane.update_in(cx, FriendsPane::focus_start);

    // A person nobody can reach: the notice bar says so, and Home stays.
    cx.dispatch_action(OpenDirectConversation {
        user_id: Snowflake::from_raw(404),
    });
    wait_until(cx, |cx| cx.debug_bounds("shell-notice-bar").is_some());
    assert_eq!(
        shell.read_with(cx, |shell, _| shell.navigation().place()),
        Some(Place::Home {
            conversation_id: None
        })
    );

    // A friend: the conversation opens and typing goes to its composer.
    pane.update_in(cx, FriendsPane::focus_start);
    cx.dispatch_action(OpenDirectConversation { user_id: bob_id });
    wait_until(cx, |cx| {
        matches!(
            shell.read_with(cx, |shell, _| shell.navigation().place()),
            Some(Place::Home {
                conversation_id: Some(_)
            })
        )
    });
    cx.simulate_input("hi");
    let typed = cx.update(|_, cx| {
        let composer = shell.read(cx).conversation().read(cx).composer().clone();
        composer.read(cx).text_in_field(cx)
    });
    assert_eq!(typed, "hi");
    harness.stop();
}

#[gpui::test]
fn the_people_picker_adds_several_friends_to_a_group_one_after_another(cx: &mut TestAppContext) {
    use mellow_protocol::{SendFriendRequest, SetRelationshipKind};

    let (harness, cx) = start(cx);
    let ada = register(&harness, "ada");
    let others = ["bob", "cleo", "dan", "eve"].map(|name| register(&harness, name));
    let ids = harness.runtime.block_on(async {
        let mut ids = Vec::new();
        for other in &others {
            let id = other.current_user().await.unwrap().id;
            other
                .send_friend_request(&SendFriendRequest {
                    username: "ada".into(),
                    friend_code: None,
                })
                .await
                .unwrap();
            ada.set_relationship(id, SetRelationshipKind::Friend)
                .await
                .unwrap();
            ids.push(id);
        }
        ids
    });
    harness
        .session
        .update(cx, |session, cx| session.begin_signed_in_session(ada, cx));
    wait_until(cx, |cx| {
        harness.session.read_with(cx, |session, _| {
            session.status() == ConnectionStatus::Connected
        })
    });
    let group_id = answer_of(&harness, cx, |session, cx, on_done| {
        session.create_group_conversation(ids[..2].to_vec(), "", cx, on_done);
    })
    .unwrap();

    // Dan and Eve picked together: one request each, the second sent once the first answered.
    let home = harness
        .main_window
        .read_with(cx, |main_window, cx| {
            main_window
                .shell()
                .map(|shell| shell.read(cx).home().clone())
        })
        .unwrap();
    home.update_in(cx, |home, window, cx| {
        home.open_people_picker(Some(group_id), window, cx);
    });
    cx.run_until_parked();
    let picker = home
        .read_with(cx, |home, _| home.picker().cloned())
        .unwrap();
    for name in ["dan", "eve"] {
        cx.simulate_input(name);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
    }
    assert_eq!(
        picker.read_with(cx, |picker, _| picker.picked_ids().to_vec()),
        ids[2..].to_vec()
    );
    let dialog = picker
        .read_with(cx, |picker, _| picker.dialog().cloned())
        .unwrap();
    dialog.update(cx, crate::components::ModalDialog::submit_now);
    wait_until(cx, |cx| {
        harness.session.read_with(cx, |session, _| {
            session
                .state()
                .direct_message_channel(group_id)
                .is_some_and(|group| group.recipients.len() == 4)
        })
    });
    harness.stop();
}
#[gpui::test]
fn a_moderator_revokes_a_ban_manages_invites_and_reads_the_log(cx: &mut TestAppContext) {
    use mellow_protocol::{AuditAction, CreateBanRequest, CreateInviteRequest};

    use crate::settings::server::{BansPage, InvitesPage, ModerationLogPage};

    let (harness, cx) = start(cx);
    let ada = register(&harness, "ada");
    let bob = register(&harness, "bob");
    let (guild_id, bob_id) = harness.runtime.block_on(async {
        let guild = ada.create_guild("Home").await.unwrap();
        let invite = ada
            .create_invite(
                guild.id,
                &CreateInviteRequest {
                    max_uses: None,
                    expires_in_seconds: None,
                },
            )
            .await
            .unwrap();
        bob.join_with_invite(&invite.code).await.unwrap();
        let bob_id = bob.current_user().await.unwrap().id;
        ada.ban_member(
            guild.id,
            bob_id,
            &CreateBanRequest {
                reason: "spam".into(),
            },
        )
        .await
        .unwrap();
        (guild.id, bob_id)
    });
    harness
        .session
        .update(cx, |session, cx| session.begin_signed_in_session(ada, cx));
    wait_until(cx, |cx| {
        harness
            .session
            .read_with(cx, |session, _| session.state().guild(guild_id).is_some())
    });

    // Bans: Bob is listed with the reason given, then can join again.
    let session = harness.session.clone();
    let bans = cx.new(|cx| BansPage::new(session, guild_id, cx));
    wait_until(cx, |cx| {
        bans.read_with(cx, |page, _| {
            page.bans()
                .is_some_and(|bans| bans.len() == 1 && bans[0].reason == "spam")
        })
    });
    bans.update(cx, |page, cx| page.revoke(bob_id, cx));
    wait_until(cx, |cx| {
        bans.read_with(cx, |page, _| page.bans().is_some_and(<[_]>::is_empty))
    });

    // Invites: the one Bob used is listed; "Create invite" adds a second with the default limits.
    let session = harness.session.clone();
    let invites = cx.new(|cx| InvitesPage::new(session, guild_id, cx));
    wait_until(cx, |cx| {
        invites.read_with(cx, |page, _| {
            page.invites().is_some_and(|list| list.len() == 1)
        })
    });
    invites.update_in(cx, InvitesPage::open_create_dialog);
    invites.update(cx, InvitesPage::create);
    wait_until(cx, |cx| {
        invites.read_with(cx, |page, _| {
            page.invites().is_some_and(|list| {
                list.len() == 2
                    && list
                        .iter()
                        .any(|invite| invite.expires_at_unix_milliseconds.is_some())
            })
        })
    });

    // The log tells the ban and its revocation, newest first.
    let session = harness.session.clone();
    let log = cx.new(|cx| ModerationLogPage::new(session, guild_id, cx));
    wait_until(cx, |cx| {
        log.read_with(cx, |page, _| {
            page.entries().is_some_and(|entries| {
                let actions: Vec<AuditAction> = entries.iter().map(|entry| entry.action).collect();
                actions.contains(&AuditAction::MemberBanAdd)
                    && actions.contains(&AuditAction::MemberBanRemove)
                    && actions
                        .iter()
                        .position(|action| *action == AuditAction::MemberBanRemove)
                        < actions
                            .iter()
                            .position(|action| *action == AuditAction::MemberBanAdd)
            })
        })
    });
    harness.stop();
}
#[gpui::test]
#[allow(clippy::too_many_lines)] // One story against the instance, step by step.
fn a_search_finds_a_servers_message_with_its_words_lit_and_opens_it(cx: &mut TestAppContext) {
    use mellow_client_core::shared_logic::message_search::MessageSearchStatus;
    use mellow_protocol::{ChannelKind, SendMessageRequest};

    let (harness, cx) = start(cx);
    cx.update(|_, cx| {
        crate::components::bind_tab_navigation(cx);
        crate::composer::bind_composer_keys(cx);
    });
    let rest = register(&harness, "hedy");
    let (guild_id, channel_id, pizza_id) = harness.runtime.block_on(async {
        let guild = rest.create_guild("Kitchen").await.unwrap();
        let channel_id = guild
            .channels
            .iter()
            .find(|channel| channel.kind == ChannelKind::Text)
            .unwrap()
            .id;
        let mut sent = Vec::new();
        for content in ["pizza party tonight", "nothing to see", "bring drinks"] {
            let message = rest
                .send_message(
                    channel_id,
                    &SendMessageRequest {
                        content: content.into(),
                        reply_to_id: None,
                        nonce: None,
                        attachment_ids: Vec::new(),
                        forwarded_from: None,
                        poll: None,
                        checklist: None,
                        progress_bar: None,
                    },
                )
                .await
                .unwrap();
            sent.push(message.id);
        }
        (guild.id, channel_id, sent[0])
    });
    harness
        .session
        .update(cx, |session, cx| session.begin_signed_in_session(rest, cx));
    let shell = |cx: &mut VisualTestContext| {
        harness
            .main_window
            .read_with(cx, |main_window, _| main_window.shell().cloned())
    };
    wait_until(cx, |cx| {
        shell(cx).is_some_and(|shell| {
            shell.read_with(cx, |shell, _| {
                shell.navigation().selected_guild_id() == Some(guild_id)
            })
        })
    });
    let shell = shell(cx).unwrap();
    let search = shell.read_with(cx, |shell, _| shell.message_search().clone());
    // The shell scopes the search to the open server on its next frame.
    wait_until(cx, |cx| {
        search.read_with(cx, |search, _| {
            search.search().scope_guild_id() == Some(guild_id)
        })
    });

    search.update(cx, |search, cx| search.search_now("pizza".into(), cx));
    wait_until(cx, |cx| {
        search.read_with(cx, |search, _| {
            search.search().status() == MessageSearchStatus::Loaded
        })
    });
    let (hit_ids, highlights) = search.read_with(cx, |search, _| {
        let hits = search.search().hits();
        (
            hits.iter().map(|hit| hit.message.id).collect::<Vec<_>>(),
            hits.first().map(|hit| hit.highlights.clone()),
        )
    });
    assert_eq!(hit_ids, vec![pizza_id]);
    assert_eq!(
        highlights,
        Some(vec![mellow_protocol::HighlightRange { start: 0, end: 5 }])
    );

    // Opening the result reaches the message in its channel.
    search.update(cx, |search, cx| {
        search.open_result(channel_id, pizza_id, cx);
    });
    let conversation = shell.read_with(cx, |shell, _| shell.conversation().clone());
    wait_until(cx, |cx| {
        conversation.read_with(cx, |view, _| view.open_channel_id() == Some(channel_id))
            && harness.session.read_with(cx, |session, _| {
                session
                    .state()
                    .messages(channel_id)
                    .iter()
                    .any(|message| message.id == pizza_id)
            })
    });

    // At Home, the search is across the conversations, of which there are none.
    shell.update_in(cx, |shell, window, cx| {
        shell.open_place(None, None, window, cx);
    });
    wait_until(cx, |cx| {
        search.read_with(cx, |search, _| search.search().scope_guild_id().is_none())
    });
    search.update(cx, |search, cx| search.search_now("pizza".into(), cx));
    wait_until(cx, |cx| {
        search.read_with(cx, |search, _| {
            search.search().status() == MessageSearchStatus::Loaded
        })
    });
    assert!(search.read_with(cx, |search, _| search.search().hits().is_empty()));
    harness.stop();
}
