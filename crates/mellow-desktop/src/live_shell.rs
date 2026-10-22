//! The shell connected to a real instance through the client core (ADR-0011).
//!
//! State lives on the UI thread; network calls run on a Tokio runtime and their results come
//! back through Slint's event loop (`slint::spawn_local`), so no lock is ever shared with the UI.

mod account_settings;
mod message_actions;
mod server_actions;
mod server_settings;

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mellow_client_core::state::{ClientState, HISTORY_PAGE_SIZE, StateChange};
use mellow_client_core::{
    ClientError, GatewayUpdate, InstanceAddress, RestClient, generate_message_nonce, run_gateway,
};
use mellow_protocol::{
    ApiErrorCode, GatewayEvent, LoginRequest, Message, MessageHistoryQuery, Permissions,
    PublicUser, RegisterRequest, SendMessageRequest, Snowflake,
};
use slint::{ComponentHandle as _, Model as _, ModelRc, SharedString, VecModel, Weak};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use crate::generated_ui::{AppWindow, MellowComponent, MessageRow, QuickSwitcherKind};
use crate::live_rows::ConnectionStatus;
use crate::mentions::MentionablePeople;
use crate::preferences::Preferences;
use crate::role_settings::MoveDirection;
use crate::session_store::{self, StoredSession};
use crate::{clipboard, live_rows, quick_switcher, shell_rows};

const GATEWAY_UPDATE_QUEUE_CAPACITY: usize = 256;
const LOGOUT_ON_EXIT_TIMEOUT: Duration = Duration::from_secs(3);
/// A Secret Service waiting on an unlock prompt nobody sees would otherwise keep the sign-in form
/// busy forever.
const SECRET_STORE_TIMEOUT: Duration = Duration::from_secs(10);
/// Never the server's error text (development notes: "Server text in the UI").
const MESSAGE_NOT_SENT_ANNOUNCEMENT: &str = "Message not sent.";
const CONNECTING_NOTICE: &str = "Connecting…";
const RECONNECTING_NOTICE: &str = "Connection lost. Reconnecting…";
const SESSION_ENDED_MESSAGE: &str = "Your session ended. Sign in again.";
/// Shown when the rail's home entry (index -1 in `server_rail.slint`) is selected.
const HOME_SERVER_NAME: &str = "Direct messages";
/// How often the typing indicator checks whether typing has expired.
const TYPING_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
/// Someone typing whose name this client does not know yet.
const SOMEONE: &str = "Someone";

struct Connection {
    rest: RestClient,
    gateway: tokio::task::AbortHandle,
    /// False when the secret store refused it: the session then lives only as long as the app.
    is_session_stored: bool,
}

pub struct LiveShell {
    window: Weak<AppWindow>,
    runtime: Handle,
    connection: Option<Connection>,
    state: ClientState,
    selected_guild_id: Option<Snowflake>,
    selected_channel_id: Option<Snowflake>,
    collapsed_category_ids: BTreeSet<Snowflake>,
    /// Fetched once per `Ready`; the gateway keeps them current afterwards.
    requested_history_channel_ids: HashSet<Snowflake>,
    requested_member_list_guild_ids: HashSet<Snowflake>,
    /// Channels with a request for older history in flight, so scrolling asks for one page at a time.
    loading_older_history_channel_ids: HashSet<Snowflake>,
    /// Channels whose last history request failed: they wait for "Try again" or the next `Ready`
    /// instead of asking again on every scroll frame.
    failed_history_channel_ids: HashSet<Snowflake>,
    /// Bumped by every `Ready`, which resets the history bookkeeping: answers to history requests
    /// sent before it are dropped, so a late failure cannot mark a channel failed again.
    history_generation: u64,
    messages: Rc<VecModel<MessageRow>>,
    /// The message id behind each row of `messages`, in the same order.
    displayed_message_ids: Vec<Snowflake>,
    /// The outbox nonce behind each row after those of `displayed_message_ids`.
    displayed_pending_nonces: Vec<String>,
    /// What the message rows were built with: rows are rebuilt only when it changes.
    displayed_can_manage_messages: bool,
    /// Role colours the message rows were built with: rows are rebuilt only when they change.
    displayed_role_colors: HashMap<Snowflake, u32>,
    /// The open channel's read position when it was opened, until its messages are known and the
    /// marker below can be set. Acknowledging on open moves the stored one at once.
    read_position_at_open: Option<Snowflake>,
    /// What this visit counts as new (DESIGN.md §5.4); gone once read or marked as read.
    unread_marker: Option<live_rows::UnreadMarker>,
    /// Ticks while someone types in the open channel, so the indicator clears when typing expires.
    typing_timer: slint::Timer,
    /// What each result of the open quick switcher leads to, by option index.
    quick_switcher_destinations: Vec<quick_switcher::Destination>,
    /// For the typing timer: the shell owns the timer, so the timer must not own the shell.
    self_reference: std::rc::Weak<RefCell<LiveShell>>,
    /// The message the composer answers, shown as "Replying to …".
    reply_to_message_id: Option<Snowflake>,
    /// See `clipboard::copy_text`.
    clipboard: Option<arboard::Clipboard>,
    /// Bumped whenever a session starts or ends, so late results of an earlier session (a 401
    /// after signing out, buffered gateway updates) never act on the current one.
    session_generation: u64,
    /// Awaited before exit: the runtime drops unfinished tasks, so the revocation might never leave.
    /// Every sign-out still reaching its instance: signing out twice must not orphan the first.
    pending_logouts: Vec<tokio::task::JoinHandle<()>>,
    /// Open while server settings show the Roles section of the selected guild.
    role_editor: Option<server_settings::RoleEditor>,
    /// One role request at a time, across reopenings of server settings.
    is_role_request_in_flight: bool,
    /// Bumped whenever a server dialog closes, so a late answer never lands in a later dialog.
    dialog_request_generation: u64,
    connection_status: ConnectionStatus,
    /// Shared with the Appearance callback in `main`; the member sheet is remembered per server.
    preferences: Rc<RefCell<Preferences>>,
}

pub type SharedShell = Rc<RefCell<LiveShell>>;

pub fn start(
    window: &AppWindow,
    runtime: Handle,
    preferences: Rc<RefCell<Preferences>>,
) -> SharedShell {
    let shell = Rc::new(RefCell::new(LiveShell {
        window: window.as_weak(),
        runtime,
        connection: None,
        state: ClientState::default(),
        selected_guild_id: None,
        selected_channel_id: None,
        collapsed_category_ids: BTreeSet::new(),
        requested_history_channel_ids: HashSet::new(),
        requested_member_list_guild_ids: HashSet::new(),
        loading_older_history_channel_ids: HashSet::new(),
        failed_history_channel_ids: HashSet::new(),
        history_generation: 0,
        messages: Rc::new(VecModel::default()),
        displayed_message_ids: Vec::new(),
        displayed_pending_nonces: Vec::new(),
        displayed_can_manage_messages: false,
        displayed_role_colors: HashMap::new(),
        read_position_at_open: None,
        unread_marker: None,
        typing_timer: slint::Timer::default(),
        quick_switcher_destinations: Vec::new(),
        self_reference: std::rc::Weak::new(),
        reply_to_message_id: None,
        clipboard: None,
        session_generation: 0,
        pending_logouts: Vec::new(),
        role_editor: None,
        is_role_request_in_flight: false,
        dialog_request_generation: 0,
        connection_status: ConnectionStatus::SignedOut,
        preferences,
    }));
    shell.borrow_mut().self_reference = Rc::downgrade(&shell);
    window.set_is_signed_in(false);
    window.set_messages(shell.borrow().messages.clone().into());
    connect_callbacks(window, &shell);
    restore_stored_session(&shell);
    shell
}

/// Resumes the session saved in the secret store, if any; the sign-in form stays busy meanwhile.
fn restore_stored_session(shell: &SharedShell) {
    if let Some(window) = shell.borrow().window.upgrade() {
        window.set_login_is_busy(true);
    }
    let request = async {
        let load = tokio::task::spawn_blocking(session_store::load_session);
        let stored_session = match tokio::time::timeout(SECRET_STORE_TIMEOUT, load).await {
            Ok(Ok(Ok(stored_session))) => stored_session,
            Ok(Ok(Err(error))) => {
                tracing::warn!(%error, "cannot read the stored session");
                None
            }
            Ok(Err(error)) => {
                tracing::error!(%error, "reading the stored session failed");
                None
            }
            Err(_elapsed) => {
                tracing::warn!("the secret store did not answer; starting signed out");
                None
            }
        };
        let Some(StoredSession { instance, token }) = stored_session else {
            return Ok(None);
        };
        let mut rest = RestClient::new(instance)?;
        rest.set_token(token);
        Ok(Some(rest))
    };
    run_request(shell, request, |shell, result| {
        if let Some(window) = shell.borrow().window.upgrade() {
            window.set_login_is_busy(false);
        }
        match result {
            Ok(Some(rest)) => connect_gateway(shell, rest, true),
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "cannot resume the stored session"),
        }
    });
}

/// Saves the session for the next launch; false when the secret store refused it.
async fn store_session(rest: &RestClient) -> bool {
    let Some(token) = rest.token().map(|token| Zeroizing::new(token.to_owned())) else {
        return false;
    };
    let instance = rest.instance().clone();
    if instance.is_loopback() && !session_store::SHOULD_STORE_LOOPBACK_SESSIONS {
        return false;
    }
    let save = tokio::task::spawn_blocking(move || session_store::save_session(&instance, &token));
    match tokio::time::timeout(SECRET_STORE_TIMEOUT, save).await {
        Ok(Ok(Ok(()))) => true,
        Ok(Ok(Err(error))) => {
            tracing::warn!(%error, "cannot store the session; it will end when the app exits");
            false
        }
        Ok(Err(error)) => {
            tracing::error!(%error, "storing the session failed");
            false
        }
        // A save that completes later stores a token revoked at exit; the next launch then gets a
        // 401 and forgets it.
        Err(_elapsed) => {
            tracing::warn!(
                "the secret store did not answer; the session will end when the app exits"
            );
            false
        }
    }
}

/// Removes the stored session in the background; a failure only means the next launch tries a
/// token the server rejects, which leads back to the sign-in screen.
/// The handle tells whether the entry is gone, for a sign-out that must not claim it is.
fn forget_stored_session(runtime: &Handle) -> tokio::task::JoinHandle<bool> {
    let forget = runtime.spawn_blocking(session_store::forget_session);
    runtime.spawn(async move {
        match forget.await {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                tracing::warn!(%error, "cannot remove the stored session");
                false
            }
            Err(error) => {
                tracing::error!(%error, "removing the stored session failed");
                false
            }
        }
    })
}

/// Lets a pending sign-out reach the server. A stored session stays open for the next launch; one
/// the secret store refused is revoked instead: nobody could use it again, so it would stay valid
/// for nothing.
pub fn end_connection_before_exit(shell: &SharedShell) {
    let (runtime, connection, pending_logouts) = {
        let mut shell = shell.borrow_mut();
        (
            shell.runtime.clone(),
            shell.connection.take(),
            std::mem::take(&mut shell.pending_logouts),
        )
    };
    let logouts_deadline = tokio::time::Instant::now() + LOGOUT_ON_EXIT_TIMEOUT;
    for pending_logout in pending_logouts {
        if runtime
            .block_on(tokio::time::timeout_at(logouts_deadline, pending_logout))
            .is_err()
        {
            tracing::warn!("a sign-out did not reach the instance before exiting");
        }
    }
    let Some(Connection {
        mut rest,
        gateway,
        is_session_stored,
    }) = connection
    else {
        return;
    };
    gateway.abort();
    if is_session_stored {
        return;
    }
    let logout = runtime.block_on(tokio::time::timeout(LOGOUT_ON_EXIT_TIMEOUT, rest.logout()));
    if !matches!(logout, Ok(Ok(()))) {
        tracing::warn!("could not end the session on the instance before exiting");
    }
}

fn connect_callbacks(window: &AppWindow, shell: &SharedShell) {
    let shell_for_callback = shell.clone();
    window.on_sign_in(move || submit_credentials(&shell_for_callback, false));
    let shell_for_callback = shell.clone();
    window.on_register(move || submit_credentials(&shell_for_callback, true));
    let shell_for_callback = shell.clone();
    window.on_sign_out(move || sign_out(&shell_for_callback));

    let shell_for_callback = shell.clone();
    window.on_select_server(move |server_index| {
        let guild_id = usize::try_from(server_index).ok().and_then(|index| {
            shell_for_callback
                .borrow()
                .state
                .guilds()
                .get(index)
                .map(|guild| guild.id)
        });
        select_guild(&shell_for_callback, guild_id, None);
    });

    connect_layout_callbacks(window, shell);

    let shell_for_callback = shell.clone();
    window.on_select_channel(move |channel_id: SharedString| {
        if let Ok(channel_id) = channel_id.parse() {
            open_channel(&shell_for_callback, Some(channel_id));
        }
    });

    let shell_for_callback = shell.clone();
    window.on_toggle_category(move |category_index| {
        {
            let mut shell = shell_for_callback.borrow_mut();
            let category_id = shell
                .selected_guild_id
                .and_then(|guild_id| shell.state.guild(guild_id))
                .map(live_rows::sorted_category_ids)
                .and_then(|ids| {
                    usize::try_from(category_index)
                        .ok()
                        .and_then(|index| ids.get(index).copied())
                });
            let Some(category_id) = category_id else {
                return;
            };
            if !shell.collapsed_category_ids.remove(&category_id) {
                shell.collapsed_category_ids.insert(category_id);
            }
        }
        refresh_channel_rows(&shell_for_callback.borrow());
    });

    let shell_for_callback = shell.clone();
    window.on_load_older_messages(move || load_older_messages(&shell_for_callback));
    let shell_for_callback = shell.clone();
    window.on_retry_loading_history(move || retry_loading_history(&shell_for_callback));

    let shell_for_callback = shell.clone();
    window.on_send_message(move |text: SharedString| send_message(&shell_for_callback, text));

    let shell_for_callback = shell.clone();
    window.on_retry_message(move |row_index| retry_message(&shell_for_callback, row_index));
    let shell_for_callback = shell.clone();
    window.on_discard_failed_message(move |row_index| {
        discard_failed_message(&shell_for_callback, row_index);
    });

    let shell_for_callback = shell.clone();
    window.on_copy_message_text(move |message_index| {
        let mut shell = shell_for_callback.borrow_mut();
        let Some(message) = usize::try_from(message_index)
            .ok()
            .and_then(|index| shell.messages.row_data(index))
        else {
            return;
        };
        clipboard::copy_text(&mut shell.clipboard, &message.body_plain);
    });

    let shell_for_callback = shell.clone();
    window.on_reply_to_message(move |row_index| {
        message_actions::reply_to_message(&shell_for_callback, row_index);
    });
    let shell_for_callback = shell.clone();
    window.on_cancel_reply(move || message_actions::cancel_reply(&shell_for_callback));
    let shell_for_callback = shell.clone();
    window.on_edit_last_own_message(move || {
        message_actions::edit_last_own_message(&shell_for_callback);
    });
    let shell_for_callback = shell.clone();
    window.on_save_message_edit(move |row_index, text| {
        message_actions::save_message_edit(&shell_for_callback, row_index, &text);
    });
    let shell_for_callback = shell.clone();
    window.on_delete_message(move |row_index| {
        message_actions::delete_message(&shell_for_callback, row_index);
    });

    connect_server_settings_callbacks(window, shell);
    connect_server_action_callbacks(window, shell);
    connect_account_callbacks(window, shell);
    let shell_for_callback = shell.clone();
    window.on_open_direct_message(move |user_id| {
        if let Ok(user_id) = user_id.parse() {
            open_direct_message(&shell_for_callback, user_id);
        }
    });
    window.on_link_clicked(|_url| {
        // shortcut: links are not opened yet; opening needs the confirmation dialog of DESIGN.md §5.4.
    });
}

enum Credentials {
    Login(LoginRequest),
    Register(RegisterRequest),
}

fn connect_server_settings_callbacks(window: &AppWindow, shell: &SharedShell) {
    let shell_for_callback = shell.clone();
    window.on_open_server_settings(move || server_settings::open(&shell_for_callback));
    let shell_for_callback = shell.clone();
    window.on_close_server_settings(move || {
        server_settings::close(&mut shell_for_callback.borrow_mut());
    });
    let shell_for_callback = shell.clone();
    window.on_select_role(move |row_index| {
        server_settings::select_role(&shell_for_callback, row_index);
    });
    let shell_for_callback = shell.clone();
    window.on_role_draft_name_edited(move || server_settings::edit_draft_name(&shell_for_callback));
    let shell_for_callback = shell.clone();
    window.on_choose_role_color(move |preset_index| {
        server_settings::choose_color(&shell_for_callback, preset_index);
    });
    let shell_for_callback = shell.clone();
    window.on_toggle_role_permission(move |row_index| {
        server_settings::toggle_permission(&shell_for_callback, row_index);
    });
    let shell_for_callback = shell.clone();
    window.on_save_role_changes(move || server_settings::save_changes(&shell_for_callback));
    let shell_for_callback = shell.clone();
    window.on_reset_role_changes(move || server_settings::reset_changes(&shell_for_callback));
    let shell_for_callback = shell.clone();
    window.on_create_role(move || server_settings::create_role(&shell_for_callback));
    let shell_for_callback = shell.clone();
    window.on_delete_role(move |row_index| {
        server_settings::delete_role(&shell_for_callback, row_index);
    });
    let shell_for_callback = shell.clone();
    window.on_move_role_up(move |row_index| {
        server_settings::move_role(&shell_for_callback, row_index, MoveDirection::Up);
    });
    let shell_for_callback = shell.clone();
    window.on_move_role_down(move |row_index| {
        server_settings::move_role(&shell_for_callback, row_index, MoveDirection::Down);
    });
}

fn connect_account_callbacks(window: &AppWindow, shell: &SharedShell) {
    let shell_for_callback = shell.clone();
    window.on_save_display_name(move |name| {
        account_settings::save_display_name(&shell_for_callback, &name);
    });
    let shell_for_callback = shell.clone();
    window.on_save_username(move |username, password| {
        account_settings::save_username(&shell_for_callback, &username, &password);
    });
    let shell_for_callback = shell.clone();
    window.on_change_password(move |current, new| {
        account_settings::change_password(&shell_for_callback, &current, &new);
    });
}

fn connect_server_action_callbacks(window: &AppWindow, shell: &SharedShell) {
    let shell_for_callback = shell.clone();
    window.on_create_server(move |name| server_actions::create_server(&shell_for_callback, &name));
    let shell_for_callback = shell.clone();
    window.on_join_server(move |text| server_actions::join_server(&shell_for_callback, &text));
    let shell_for_callback = shell.clone();
    window.on_create_invite(move || server_actions::create_invite(&shell_for_callback));
    let shell_for_callback = shell.clone();
    window.on_copy_invite_code(move || server_actions::copy_invite_code(&shell_for_callback));
    let shell_for_callback = shell.clone();
    window.on_create_channel(move |name| {
        server_actions::create_channel(&shell_for_callback, &name);
    });
    let shell_for_callback = shell.clone();
    window.on_leave_server(move || server_actions::leave_server(&shell_for_callback));
    let shell_for_callback = shell.clone();
    window.on_dialog_closed(move || {
        server_actions::reset_dialog(&mut shell_for_callback.borrow_mut());
    });
}

/// Opens the conversation with a member (member list → Message), then shows it at home.
fn open_direct_message(shell: &SharedShell, user_id: Snowflake) {
    let Some(rest) = authenticated_rest(shell) else {
        return;
    };
    let request = async move { rest.open_direct_message(user_id).await };
    run_request(shell, request, |shell, result| match result {
        Ok(conversation) => {
            let conversation_id = conversation.id;
            let change = shell
                .borrow_mut()
                .state
                .upsert_direct_message_channel(conversation);
            apply_state_change(shell, change);
            select_guild(shell, None, Some(conversation_id));
            if let Some(window) = shell.borrow().window.upgrade() {
                window.invoke_focus_composer();
            }
        }
        Err(error) => {
            report_request_error(shell, &error, "open conversation");
            if !error.needs_new_login()
                && let Some(window) = shell.borrow().window.upgrade()
            {
                window
                    .set_message_action_error("Couldn't open the conversation. Try again.".into());
            }
        }
    });
}

fn device_name() -> String {
    format!("Mellow desktop ({})", std::env::consts::OS)
}

fn submit_credentials(shell: &SharedShell, is_registering: bool) {
    let Some(window) = shell.borrow().window.upgrade() else {
        return;
    };
    let Ok(instance) = InstanceAddress::parse(&window.get_login_instance_address()) else {
        show_login_error(
            &window,
            "That doesn't look like an instance address. Try something like chat.example.org.",
        );
        return;
    };
    let host = instance.host().to_owned();
    // Cleared from the UI right away; the request owns the only copy until it is sent.
    let password = Zeroizing::new(window.get_login_password().to_string());
    window.set_login_password(SharedString::default());
    let credentials = if is_registering {
        Credentials::Register(RegisterRequest {
            username: window.get_login_username().trim().to_owned(),
            display_name: None,
            email: window.get_login_email().trim().to_owned(),
            password: password.to_string(),
            device_name: device_name(),
        })
    } else {
        Credentials::Login(LoginRequest {
            login: window.get_login_login().trim().to_owned(),
            password: password.to_string(),
            device_name: device_name(),
        })
    };
    window.set_login_error_message(SharedString::default());
    window.set_login_is_busy(true);

    let request = async move {
        let mut rest = RestClient::new(instance)?;
        match credentials {
            Credentials::Login(mut request) => {
                let result = rest.login(&request).await;
                zeroize::Zeroize::zeroize(&mut request.password);
                result?;
            }
            Credentials::Register(mut request) => {
                let result = rest.register(&request).await;
                zeroize::Zeroize::zeroize(&mut request.password);
                result?;
            }
        }
        let is_session_stored = store_session(&rest).await;
        Ok((rest, is_session_stored))
    };
    run_request(shell, request, move |shell, result| {
        let Some(window) = shell.borrow().window.upgrade() else {
            return;
        };
        window.set_login_is_busy(false);
        match result {
            Ok((rest, is_session_stored)) => connect_gateway(shell, rest, is_session_stored),
            Err(error) => show_login_error(&window, &sign_in_error_message(&error, &host)),
        }
    });
}

/// Cleared, then set on the next tick: an identical message set twice in one go would leave the
/// property unchanged, so screen readers would not announce it and focus would not move.
fn show_login_error(window: &AppWindow, message: &str) {
    window.set_login_error_message(SharedString::default());
    let weak_window = window.as_weak();
    let message = SharedString::from(message);
    slint::Timer::single_shot(Duration::ZERO, move || {
        if let Some(window) = weak_window.upgrade() {
            window.set_login_error_message(message);
        }
    });
}

/// The copy of DESIGN.md §5.1; the server's own message for cases it explains best.
fn sign_in_error_message(error: &ClientError, host: &str) -> String {
    match error {
        ClientError::Api {
            code: ApiErrorCode::InvalidCredentials,
            ..
        } => "That username and password don't match. Check them and try again.".into(),
        ClientError::Api {
            code: ApiErrorCode::AccountDisabled,
            ..
        } => format!(
            "This account is suspended on {host}. Contact the instance admin if you think this is a mistake."
        ),
        ClientError::Api {
            code: ApiErrorCode::RegistrationClosed,
            ..
        } => {
            format!("Registrations are closed on {host}. Ask the instance admin for an account.")
        }
        ClientError::Api { message, .. } => message.clone(),
        ClientError::Network(_) => format!(
            "We couldn't reach {host}. Check the address, or try again if your connection is down."
        ),
        ClientError::UnexpectedResponse(_) => format!(
            "{host} is online but doesn't look like a Mellow instance. Check the address with the person who invited you."
        ),
        ClientError::InvalidInstanceAddress(_)
        | ClientError::NotSignedIn
        | ClientError::RandomGeneratorFailed(_) => error.to_string(),
    }
}

/// Runs `request` on the network runtime, then `on_done` on the UI thread with its result.
fn run_request<T: Send + 'static>(
    shell: &SharedShell,
    request: impl Future<Output = Result<T, ClientError>> + Send + 'static,
    on_done: impl FnOnce(&SharedShell, Result<T, ClientError>) + 'static,
) {
    let (task, generation) = {
        let shell = shell.borrow();
        (shell.runtime.spawn(request), shell.session_generation)
    };
    let shell = shell.clone();
    let scheduled = slint::spawn_local(async move {
        let result = task.await;
        if shell.borrow().session_generation != generation {
            return;
        }
        match result {
            Ok(result) => on_done(&shell, result),
            Err(error) => tracing::error!(%error, "a network task failed"),
        }
    });
    if let Err(error) = scheduled {
        tracing::error!(%error, "cannot schedule work on the UI thread");
    }
}

/// Signs out locally when the server no longer accepts the token; otherwise only logs.
fn report_request_error(shell: &SharedShell, error: &ClientError, action: &str) {
    if error.needs_new_login() {
        end_session(shell, SESSION_ENDED_MESSAGE);
    } else {
        tracing::warn!(%error, action, "request failed");
    }
}

fn authenticated_rest(shell: &SharedShell) -> Option<RestClient> {
    shell
        .borrow()
        .connection
        .as_ref()
        .map(|connection| connection.rest.clone())
}

fn connect_gateway(shell: &SharedShell, rest: RestClient, is_session_stored: bool) {
    let Some(token) = rest.token().map(|token| Zeroizing::new(token.to_owned())) else {
        return;
    };
    let (sender, mut updates) = mpsc::channel(GATEWAY_UPDATE_QUEUE_CAPACITY);
    let generation = {
        let mut shell_mut = shell.borrow_mut();
        if let Some(previous) = shell_mut.connection.take() {
            previous.gateway.abort();
        }
        shell_mut.session_generation += 1;
        let gateway = shell_mut
            .runtime
            .spawn(run_gateway(rest.instance().clone(), token, sender))
            .abort_handle();
        shell_mut.connection = Some(Connection {
            rest,
            gateway,
            is_session_stored,
        });
        if let Some(window) = shell_mut.window.upgrade() {
            window.set_is_signed_in(true);
        }
        show_connection_status(&mut shell_mut, ConnectionStatus::Connecting);
        shell_mut.session_generation
    };
    let shell = shell.clone();
    let scheduled = slint::spawn_local(async move {
        while let Some(update) = updates.recv().await {
            if shell.borrow().session_generation != generation {
                break;
            }
            apply_gateway_update(&shell, update);
        }
    });
    if let Err(error) = scheduled {
        tracing::error!(%error, "cannot schedule gateway updates on the UI thread");
    }
}

fn now_unix_milliseconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// The connection notice says what the connection is doing, and Melli in the Home button shows it.
fn show_connection_status(shell: &mut LiveShell, status: ConnectionStatus) {
    shell.connection_status = status;
    if let Some(window) = shell.window.upgrade() {
        window.set_connection_notice(
            match status {
                ConnectionStatus::Connecting => CONNECTING_NOTICE,
                ConnectionStatus::Connected | ConnectionStatus::SignedOut => "",
                ConnectionStatus::Reconnecting => RECONNECTING_NOTICE,
            }
            .into(),
        );
    }
    refresh_home_melli_state(shell);
}

fn refresh_home_melli_state(shell: &LiveShell) {
    if let Some(window) = shell.window.upgrade() {
        window.set_home_melli_state(live_rows::home_melli_state(
            shell.connection_status,
            live_rows::has_unread_direct_messages(&shell.state),
        ));
    }
}

fn apply_gateway_update(shell: &SharedShell, update: GatewayUpdate) {
    let set_status =
        |status: ConnectionStatus| show_connection_status(&mut shell.borrow_mut(), status);
    match update {
        GatewayUpdate::Ready(ready) => {
            let (previous_guild_id, previous_channel_id, was_home) = {
                let mut shell = shell.borrow_mut();
                // Home after an earlier `Ready`, not the empty selection before the first one.
                let was_home =
                    shell.state.current_user().is_some() && shell.selected_guild_id.is_none();
                shell.state.apply_ready(*ready);
                shell.requested_history_channel_ids.clear();
                shell.requested_member_list_guild_ids.clear();
                // Requests sent before this `Ready` are ignored when they come back
                // (`history_generation`), so nothing else would free their channel.
                shell.loading_older_history_channel_ids.clear();
                shell.failed_history_channel_ids.clear();
                shell.history_generation += 1;
                refresh_user_panel(&shell);
                (shell.selected_guild_id, shell.selected_channel_id, was_home)
            };
            set_status(ConnectionStatus::Connected);
            let guild_id = {
                let shell = shell.borrow();
                previous_guild_id
                    .filter(|id| shell.state.guild(*id).is_some())
                    .or_else(|| {
                        if was_home {
                            None
                        } else {
                            shell.state.guilds().first().map(|guild| guild.id)
                        }
                    })
            };
            // After a re-identify the reader stays where they were, as after a resume.
            select_guild(shell, guild_id, previous_channel_id);
            // The edited guild may be gone, or its roles changed while disconnected.
            server_settings::refresh(&mut shell.borrow_mut());
            resend_failed_messages(shell);
        }
        GatewayUpdate::Event(event) => {
            let created_message = match &event {
                GatewayEvent::MessageCreate(message) => {
                    Some((message.channel_id, message.author.id))
                }
                _ => None,
            };
            let change = shell
                .borrow_mut()
                .state
                .apply_event(event, now_unix_milliseconds());
            apply_state_change(shell, change);
            let shell = shell.borrow();
            // Your own messages are not "new", as on Discord, including those resent after a reconnect.
            let is_new_message_from_someone_else =
                created_message.is_some_and(|(channel_id, author_id)| {
                    shell.selected_channel_id == Some(channel_id)
                        && shell
                            .state
                            .current_user()
                            .is_none_or(|user| user.id != author_id)
                });
            if is_new_message_from_someone_else && let Some(window) = shell.window.upgrade() {
                window.invoke_note_new_message();
            }
        }
        GatewayUpdate::Resumed => {
            set_status(ConnectionStatus::Connected);
            resend_failed_messages(shell);
        }
        GatewayUpdate::Disconnected => set_status(ConnectionStatus::Reconnecting),
        GatewayUpdate::SignedOut => {
            end_session(shell, SESSION_ENDED_MESSAGE);
        }
    }
}

fn apply_state_change(shell: &SharedShell, change: StateChange) {
    let (selected_guild_id, selected_channel_id) = {
        let shell = shell.borrow();
        (shell.selected_guild_id, shell.selected_channel_id)
    };
    match change {
        StateChange::Nothing => return,
        StateChange::Typing { channel_id } => {
            if Some(channel_id) == selected_channel_id {
                refresh_typing(&shell.borrow());
            }
            return;
        }
        StateChange::Guilds => {
            // The selected guild or channel may be gone; `None` is home, which never goes away.
            let guild_is_gone =
                selected_guild_id.is_some_and(|id| shell.borrow().state.guild(id).is_none());
            if guild_is_gone {
                let first_guild_id = shell.borrow().state.guilds().first().map(|guild| guild.id);
                select_guild(shell, first_guild_id, None);
                // After the switch, so focus goes to what the next server shows.
                server_actions::close_dialog_of_gone_guild(shell);
                // Closes server settings of the guild that is gone.
                server_settings::refresh(&mut shell.borrow_mut());
                return;
            }
            let channel_is_gone = {
                let shell = shell.borrow();
                selected_guild_id
                    .and_then(|id| shell.state.guild(id))
                    .is_some_and(|guild| {
                        !guild
                            .channels
                            .iter()
                            .any(|channel| Some(channel.id) == selected_channel_id)
                    })
            };
            if channel_is_gone {
                let first_channel_id = selected_guild_id.and_then(|id| {
                    shell
                        .borrow()
                        .state
                        .guild(id)
                        .and_then(live_rows::first_text_channel_id)
                });
                open_channel(shell, first_channel_id);
            }
            // A rename, a new topic or a permission change of the open guild or channel.
            refresh_header(&shell.borrow());
            // Rebuilding rows recreates an open inline editor, so only when Delete or a role
            // colour changes.
            let are_message_rows_outdated = {
                let shell = shell.borrow();
                can_manage_messages(&shell) != shell.displayed_can_manage_messages
                    || selected_role_colors(&shell) != shell.displayed_role_colors
            };
            if are_message_rows_outdated {
                refresh_messages(&mut shell.borrow_mut());
            }
            server_settings::refresh(&mut shell.borrow_mut());
        }
        StateChange::Messages { channel_id } if Some(channel_id) == selected_channel_id => {
            refresh_messages(&mut shell.borrow_mut());
            acknowledge_selected_channel(shell);
        }
        StateChange::Members { guild_id } if Some(guild_id) == selected_guild_id => {
            refresh_member_rows(&shell.borrow());
            // Author names take their role colour once the members are known.
            let are_role_colors_outdated = {
                let shell = shell.borrow();
                selected_role_colors(&shell) != shell.displayed_role_colors
            };
            if are_role_colors_outdated {
                refresh_messages(&mut shell.borrow_mut());
            }
            // Member counts per role.
            server_settings::refresh(&mut shell.borrow_mut());
        }
        StateChange::Presence => refresh_member_rows(&shell.borrow()),
        StateChange::UserRenamed { user_id } => {
            refresh_user_panel(&shell.borrow());
            // A conversation is named after its recipients.
            refresh_header(&shell.borrow());
            refresh_member_rows(&shell.borrow());
            // Rebuilding rows recreates an open inline editor, so only when a shown author changed.
            let shows_renamed_author = selected_channel_id.is_some_and(|channel_id| {
                shell
                    .borrow()
                    .state
                    .messages(channel_id)
                    .iter()
                    .any(|message| message.author.id == user_id)
            });
            if shows_renamed_author {
                refresh_messages(&mut shell.borrow_mut());
            }
        }
        StateChange::DirectMessages => refresh_header(&shell.borrow()),
        StateChange::Messages { .. } | StateChange::Members { .. } | StateChange::ReadStates => {}
    }
    let shell = shell.borrow();
    refresh_server_items(&shell);
    refresh_channel_rows(&shell);
}

fn refresh_user_panel(shell: &LiveShell) {
    account_settings::refresh(shell);
    if let (Some(window), Some(user)) = (shell.window.upgrade(), shell.state.current_user()) {
        window.set_current_user_name(user.display_name.as_str().into());
        window.set_current_user_initial(shell_rows::first_letter(&user.display_name).into());
        window.set_current_user_tint(crate::person_style::avatar_tint_index_of(user.id));
    }
}

/// Revokes the session on the server in the background and goes back to the sign-in screen.
fn sign_out(shell: &SharedShell) {
    let (runtime, connection) = {
        let mut shell = shell.borrow_mut();
        (shell.runtime.clone(), shell.connection.take())
    };
    let forget = end_session(shell, "");
    let Some(Connection {
        mut rest, gateway, ..
    }) = connection
    else {
        return;
    };
    gateway.abort();
    let host = rest.instance().host().to_owned();
    let (revocation_sender, revocation_receiver) = tokio::sync::oneshot::channel();
    let logout = runtime.spawn(async move {
        // A session the server already rejects is as good as revoked.
        let is_revoked = match rest.logout().await {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(%error, "could not end the session on the instance");
                error.needs_new_login()
            }
        };
        let is_forgotten = forget.await.unwrap_or(false);
        // The receiver is gone only when the app is exiting, with nobody left to tell.
        let _ = revocation_sender.send((is_revoked, is_forgotten));
    });
    let generation = {
        let mut shell = shell.borrow_mut();
        shell
            .pending_logouts
            .retain(|pending_logout| !pending_logout.is_finished());
        shell.pending_logouts.push(logout);
        shell.session_generation
    };
    let shell = shell.clone();
    let scheduled = slint::spawn_local(async move {
        let (is_revoked, is_forgotten) = revocation_receiver.await.unwrap_or((false, false));
        let shell = shell.borrow();
        // Revoked, a session left in the store is refused and forgotten at the next launch.
        if is_revoked || shell.session_generation != generation {
            return;
        }
        let message = if is_forgotten {
            format!(
                "Signed out on this device, but {host} couldn't be reached. The session stays valid there until it expires."
            )
        } else {
            format!(
                "Mellow couldn't remove the saved session from this computer, and {host} couldn't be reached. It will sign in again at next launch."
            )
        };
        if let Some(window) = shell.window.upgrade() {
            show_login_error(&window, &message);
        }
    });
    if let Err(error) = scheduled {
        tracing::error!(%error, "cannot schedule the sign-out result on the UI thread");
    }
}

/// Back to the sign-in screen, forgetting the stored session: the server no longer accepts it,
/// or the user signed out.
fn end_session(shell: &SharedShell, message: &str) -> tokio::task::JoinHandle<bool> {
    let mut shell = shell.borrow_mut();
    if let Some(connection) = shell.connection.take() {
        connection.gateway.abort();
    }
    shell.session_generation += 1;
    let forget = forget_stored_session(&shell.runtime);
    shell.state = ClientState::default();
    shell.selected_guild_id = None;
    shell.selected_channel_id = None;
    shell.messages.set_vec(Vec::new());
    shell.displayed_message_ids.clear();
    shell.displayed_pending_nonces.clear();
    message_actions::reset_message_actions(&mut shell);
    // The answer of a request in flight is dropped with the session, so nothing else clears it.
    shell.is_role_request_in_flight = false;
    server_actions::reset_dialog(&mut shell);
    // Nothing of the previous account may stay on screen for the next one.
    refresh_header(&shell);
    refresh_server_items(&shell);
    refresh_channel_rows(&shell);
    refresh_member_rows(&shell);
    refresh_history_start_state(&shell);
    if let Some(window) = shell.window.upgrade() {
        window.set_current_user_name(SharedString::default());
        window.set_current_user_initial(SharedString::default());
        window.set_composer_text(SharedString::default());
        window.set_is_signed_in(false);
        // Nothing of the previous account may stay, and its request never answers.
        account_settings::clear(&window);
        window.set_login_is_busy(false);
        show_login_error(&window, message);
    }
    show_connection_status(&mut shell, ConnectionStatus::SignedOut);
    // After signing out, so the focus is not sent back to the hidden shell.
    server_settings::close(&mut shell);
    forget
}

/// The unread bar and the member sheet (DESIGN.md §5.4, §5.7).
fn connect_layout_callbacks(window: &AppWindow, shell: &SharedShell) {
    let shell_for_callback = shell.clone();
    window.on_quick_switcher_query_changed(move |query| {
        let mut shell = shell_for_callback.borrow_mut();
        let destinations = live_rows::quick_switcher_destinations(&shell.state);
        let (rows, chosen) = quick_switcher::results(&destinations, &query);
        shell.quick_switcher_destinations = chosen;
        if let Some(window) = shell.window.upgrade() {
            window.set_quick_switcher_rows(ModelRc::new(VecModel::from(rows)));
        }
    });
    let shell_for_callback = shell.clone();
    window.on_quick_switcher_chosen(move |index| go_to(&shell_for_callback, index));
    let shell_for_callback = shell.clone();
    window.on_mark_channel_read(move || forget_unread_divider(&shell_for_callback));
    let shell_for_callback = shell.clone();
    window.on_unread_divider_read(move || forget_unread_divider(&shell_for_callback));
    let shell_for_callback = shell.clone();
    window.on_member_sheet_toggled(move || remember_member_sheet(&shell_for_callback));
    let shell_for_callback = shell.clone();
    // Mentions are coloured in Rust, so the rows follow the theme.
    window.on_theme_changed(move || refresh_messages(&mut shell_for_callback.borrow_mut()));
}

/// Opens the quick switcher's result at `index` (DESIGN.md §5.10).
fn go_to(shell: &SharedShell, index: i32) {
    let destination = usize::try_from(index).ok().and_then(|index| {
        shell
            .borrow()
            .quick_switcher_destinations
            .get(index)
            .cloned()
    });
    let Some(destination) = destination else {
        return;
    };
    let id = destination.id.parse::<Snowflake>().ok();
    let server_id = destination.server_id.parse::<Snowflake>().ok();
    match destination.kind {
        QuickSwitcherKind::Conversation => select_guild(shell, None, id),
        QuickSwitcherKind::Channel => select_guild(shell, server_id, id),
        QuickSwitcherKind::Server => select_guild(shell, server_id, None),
    }
}

/// "Mark as read", sending, or the new messages seen down to the latest one: the divider and the
/// bar go (the channel is already acknowledged).
fn forget_unread_divider(shell: &SharedShell) {
    let mut shell = shell.borrow_mut();
    shell.read_position_at_open = None;
    if shell.unread_marker.take().is_some() {
        refresh_messages(&mut shell);
    }
}

/// Remembers, for the selected server, whether its member sheet is open.
fn remember_member_sheet(shell: &SharedShell) {
    let shell = shell.borrow();
    let (Some(window), Some(guild_id)) = (shell.window.upgrade(), shell.selected_guild_id) else {
        return;
    };
    let mut preferences = shell.preferences.borrow_mut();
    preferences.set_member_sheet_open(&guild_id.to_string(), window.get_is_member_list_visible());
    preferences.save_in_background();
}

/// Opens `preferred_channel_id` when it belongs to the guild, otherwise its first text channel.
fn select_guild(
    shell: &SharedShell,
    guild_id: Option<Snowflake>,
    preferred_channel_id: Option<Snowflake>,
) {
    let channel_id = {
        let mut shell = shell.borrow_mut();
        shell.selected_guild_id = guild_id;
        let guild = guild_id.and_then(|id| shell.state.guild(id));
        let preferred = preferred_channel_id.filter(|channel_id| {
            guild.is_some_and(|guild| {
                guild
                    .channels
                    .iter()
                    .any(|channel| channel.id == *channel_id)
            })
        });
        let channel_id = match guild {
            Some(guild) => preferred.or_else(|| live_rows::first_text_channel_id(guild)),
            // Home opens the given conversation, or the most recent one.
            None => preferred_channel_id
                .filter(|id| shell.state.direct_message_channel(*id).is_some())
                .or_else(|| {
                    shell
                        .state
                        .direct_message_channels()
                        .first()
                        .map(|channel| channel.id)
                }),
        };
        refresh_server_items(&shell);
        refresh_member_rows(&shell);
        if let Some(window) = shell.window.upgrade() {
            // Closed by default and on Home; each server remembers its own (DESIGN.md §5.7).
            let is_member_sheet_open = guild_id.is_some_and(|id| {
                shell
                    .preferences
                    .borrow()
                    .is_member_sheet_open(&id.to_string())
            });
            window.set_is_member_list_visible(is_member_sheet_open);
        }
        channel_id
    };
    if let Some(guild_id) = guild_id {
        ensure_members_loaded(shell, guild_id);
    }
    open_channel(shell, channel_id);
}

fn open_channel(shell: &SharedShell, channel_id: Option<Snowflake>) {
    {
        let mut shell = shell.borrow_mut();
        if shell.selected_channel_id != channel_id {
            message_actions::reset_message_actions(&mut shell);
            if let Some(previous_channel_id) = shell.selected_channel_id {
                shell.state.trim_history(previous_channel_id);
            }
            // Reopening a channel whose history failed is a retry too.
            if let Some(channel_id) = channel_id {
                shell.failed_history_channel_ids.remove(&channel_id);
            }
            shell.read_position_at_open =
                channel_id.and_then(|id| shell.state.last_read_message_id(id));
            shell.unread_marker = None;
        }
        shell.selected_channel_id = channel_id;
        refresh_header(&shell);
        refresh_channel_rows(&shell);
        refresh_messages(&mut shell);
        refresh_typing(&shell);
        if let Some(window) = shell.window.upgrade() {
            crate::scroll_to_latest_message_after_layout(&window);
        }
    }
    if let Some(channel_id) = channel_id {
        ensure_history_loaded(shell, channel_id);
        acknowledge_selected_channel(shell);
    }
}

/// Server name, rail selection and channel header, from the current selection and state.
fn refresh_header(shell: &LiveShell) {
    let Some(window) = shell.window.upgrade() else {
        return;
    };
    let guild = shell.selected_guild_id.and_then(|id| shell.state.guild(id));
    let channel = guild.and_then(|guild| {
        guild
            .channels
            .iter()
            .find(|channel| Some(channel.id) == shell.selected_channel_id)
    });
    let conversation = shell
        .selected_channel_id
        .filter(|_| guild.is_none())
        .and_then(|id| shell.state.direct_message_channel(id));
    window.set_is_direct_message_open(conversation.is_some());
    let server_index = guild
        .and_then(|guild| {
            shell
                .state
                .guilds()
                .iter()
                .position(|candidate| candidate.id == guild.id)
        })
        .map_or(-1, shell_rows::saturating_int);
    window
        .set_can_open_server_settings(guild.is_some_and(server_settings::can_open_server_settings));
    window.set_can_invite_people(guild.is_some_and(server_actions::can_invite_people));
    window.set_can_create_channels(guild.is_some_and(server_actions::can_create_channels));
    let current_user_id = shell.state.current_user().map(|user| user.id);
    window.set_can_leave_server(
        guild.is_some_and(|guild| server_actions::can_leave_server(guild, current_user_id)),
    );
    window.set_server_name(
        guild
            .map_or(HOME_SERVER_NAME, |guild| guild.name.as_str())
            .into(),
    );
    window.set_selected_server_index(server_index);
    window.set_selected_channel_id(
        channel
            .map(|channel| channel.id)
            .or(conversation.map(|conversation| conversation.id))
            .map(|id| id.to_string())
            .unwrap_or_default()
            .into(),
    );
    window.set_selected_channel_name(
        match (channel, conversation) {
            (Some(channel), _) => channel.name.clone(),
            (None, Some(conversation)) => live_rows::direct_message_name(conversation),
            (None, None) => String::new(),
        }
        .into(),
    );
    window.set_selected_channel_topic(channel.map_or("", |channel| channel.topic.as_str()).into());
    window.set_can_send_messages(
        shell
            .selected_channel_id
            .is_none_or(|id| live_rows::can_send_messages(&shell.state, id)),
    );
}

fn ensure_history_loaded(shell: &SharedShell, channel_id: Snowflake) {
    let history_generation = {
        let mut shell = shell.borrow_mut();
        if !shell.requested_history_channel_ids.insert(channel_id) {
            return;
        }
        shell.history_generation
    };
    let Some(rest) = authenticated_rest(shell) else {
        return;
    };
    let query = MessageHistoryQuery {
        limit: Some(HISTORY_PAGE_SIZE),
        ..MessageHistoryQuery::default()
    };
    let request = async move { rest.list_messages(channel_id, &query).await };
    run_request(shell, request, move |shell, result| match result {
        _ if shell.borrow().history_generation != history_generation => {}
        Ok(page) => {
            let change = {
                let mut shell = shell.borrow_mut();
                let change = shell.state.insert_history_page(channel_id, page);
                // Left before its latest page came back: trim now, or the page lifts the cap for good.
                if shell.selected_channel_id != Some(channel_id) {
                    shell.state.trim_history(channel_id);
                }
                change
            };
            apply_state_change(shell, change);
        }
        Err(error) => {
            {
                let mut shell = shell.borrow_mut();
                shell.requested_history_channel_ids.remove(&channel_id);
                shell.failed_history_channel_ids.insert(channel_id);
                refresh_history_start_state(&shell);
            }
            report_request_error(shell, &error, "load message history");
        }
    });
}

/// Fetches the page before the oldest cached message of the open channel, one page at a time.
fn load_older_messages(shell: &SharedShell) {
    // Called on every scroll frame near the top: the cheap checks run before the client is cloned.
    let (rest, channel_id, oldest_message_id, history_generation) = {
        let mut shell = shell.borrow_mut();
        let Some(channel_id) = shell.selected_channel_id else {
            return;
        };
        // An empty channel waits for its latest page, requested when it opened.
        let Some(oldest_message_id) = shell
            .state
            .messages(channel_id)
            .first()
            .map(|message| message.id)
        else {
            return;
        };
        if !shell.state.has_older_history(channel_id)
            || shell
                .loading_older_history_channel_ids
                .contains(&channel_id)
            || shell.failed_history_channel_ids.contains(&channel_id)
        {
            return;
        }
        let Some(rest) = shell
            .connection
            .as_ref()
            .map(|connection| connection.rest.clone())
        else {
            return;
        };
        shell.loading_older_history_channel_ids.insert(channel_id);
        refresh_history_start_state(&shell);
        (
            rest,
            channel_id,
            oldest_message_id,
            shell.history_generation,
        )
    };
    let query = MessageHistoryQuery {
        before: Some(oldest_message_id),
        limit: Some(HISTORY_PAGE_SIZE),
        ..MessageHistoryQuery::default()
    };
    let request = async move { rest.list_messages(channel_id, &query).await };
    run_request(shell, request, move |shell, result| {
        {
            let mut shell = shell.borrow_mut();
            if shell.history_generation != history_generation {
                return;
            }
            shell.loading_older_history_channel_ids.remove(&channel_id);
            if result.is_err() {
                shell.failed_history_channel_ids.insert(channel_id);
            }
            refresh_history_start_state(&shell);
        }
        match result {
            Ok(page) => {
                // The page must join the cache where it was cut: a `Ready`, a trim or a deleted
                // oldest message since the request would leave a gap. The next scroll asks again.
                let is_still_adjacent = {
                    let shell = shell.borrow();
                    shell.selected_channel_id == Some(channel_id)
                        && shell
                            .state
                            .messages(channel_id)
                            .first()
                            .map(|message| message.id)
                            == Some(oldest_message_id)
                };
                if !is_still_adjacent {
                    return;
                }
                // An empty page only turns row 0 into the hero. Rebuilding the rows would recreate
                // row 0 already showing it, so it could not compensate its height, and pinning at a
                // distance while row 0 changes height makes ListView oscillate.
                if page.is_empty() {
                    let mut shell = shell.borrow_mut();
                    shell.state.insert_history_page(channel_id, page);
                    refresh_history_start_state(&shell);
                    return;
                }
                if let Some(window) = shell.borrow().window.upgrade() {
                    window.invoke_keep_distance_from_latest_message();
                }
                let change = shell
                    .borrow_mut()
                    .state
                    .insert_history_page(channel_id, page);
                apply_state_change(shell, change);
            }
            Err(error) => report_request_error(shell, &error, "load older messages"),
        }
    });
}

/// "Try again" on a failed history load: the latest page if that one failed, else the older one.
fn retry_loading_history(shell: &SharedShell) {
    let (channel_id, is_latest_page_missing) = {
        let mut shell = shell.borrow_mut();
        let Some(channel_id) = shell.selected_channel_id else {
            return;
        };
        if !shell.failed_history_channel_ids.remove(&channel_id) {
            return;
        }
        refresh_history_start_state(&shell);
        (
            channel_id,
            !shell.requested_history_channel_ids.contains(&channel_id),
        )
    };
    if is_latest_page_missing {
        ensure_history_loaded(shell, channel_id);
    } else {
        load_older_messages(shell);
    }
}

fn ensure_members_loaded(shell: &SharedShell, guild_id: Snowflake) {
    if !shell
        .borrow_mut()
        .requested_member_list_guild_ids
        .insert(guild_id)
    {
        return;
    }
    let Some(rest) = authenticated_rest(shell) else {
        return;
    };
    let request = async move { rest.list_members(guild_id).await };
    run_request(shell, request, move |shell, result| match result {
        Ok(members) => {
            let change = shell.borrow_mut().state.set_members(guild_id, members);
            apply_state_change(shell, change);
        }
        Err(error) => {
            shell
                .borrow_mut()
                .requested_member_list_guild_ids
                .remove(&guild_id);
            report_request_error(shell, &error, "load members");
        }
    });
}

/// The open channel counts as read, as on Discord.
fn acknowledge_selected_channel(shell: &SharedShell) {
    let acknowledged = {
        let mut shell = shell.borrow_mut();
        let Some(channel_id) = shell.selected_channel_id else {
            return;
        };
        if !shell.state.has_unread(channel_id) {
            return;
        }
        let Some(last_message_id) = shell
            .state
            .messages(channel_id)
            .last()
            .map(|message| message.id)
        else {
            return;
        };
        shell.state.mark_read(channel_id, last_message_id);
        (channel_id, last_message_id)
    };
    let Some(rest) = authenticated_rest(shell) else {
        return;
    };
    let (channel_id, last_message_id) = acknowledged;
    let request = async move { rest.acknowledge(channel_id, last_message_id).await };
    run_request(shell, request, |shell, result| {
        if let Err(error) = result {
            report_request_error(shell, &error, "acknowledge");
        }
    });
    let shell = shell.borrow();
    refresh_server_items(&shell);
    refresh_channel_rows(&shell);
}

fn send_message(shell: &SharedShell, text: SharedString) {
    forget_unread_divider(shell);
    let content = text.trim().to_owned();
    let Some(channel_id) = shell.borrow().selected_channel_id else {
        return;
    };
    if content.is_empty() || authenticated_rest(shell).is_none() {
        return;
    }
    if let Some(window) = shell.borrow().window.upgrade() {
        window.set_message_action_error(SharedString::default());
    }
    let nonce = match generate_message_nonce() {
        Ok(nonce) => nonce,
        Err(error) => {
            tracing::error!(%error, "cannot generate a message nonce");
            // Nothing was queued: the text goes back so it is not lost.
            if let Some(window) = shell.borrow().window.upgrade() {
                window.set_composer_text(text);
                window.set_latest_announcement(MESSAGE_NOT_SENT_ANNOUNCEMENT.into());
            }
            return;
        }
    };
    let reply_to_id = message_actions::take_reply(shell);
    let request =
        shell
            .borrow_mut()
            .state
            .queue_message(channel_id, content, reply_to_id, nonce.clone());
    apply_state_change(shell, StateChange::Messages { channel_id });
    if let Some(window) = shell.borrow().window.upgrade() {
        window.invoke_scroll_to_latest_message();
    }
    deliver_pending_message(shell, channel_id, nonce, request);
}

/// Sends a queued message; the server returns the stored one when this nonce already reached it.
fn deliver_pending_message(
    shell: &SharedShell,
    channel_id: Snowflake,
    nonce: String,
    request: SendMessageRequest,
) {
    let Some(rest) = authenticated_rest(shell) else {
        let change = shell.borrow_mut().state.mark_send_failed(&nonce);
        apply_state_change(shell, change);
        return;
    };
    let send_message_call = async move { rest.send_message(channel_id, &request).await };
    run_request(shell, send_message_call, move |shell, result| {
        apply_send_result(shell, channel_id, &nonce, result);
    });
}

/// After a reconnect, sends every failed message again, one after another so they keep their
/// order. The first failure the instance does not refuse stops the batch: the rest stay failed.
fn resend_failed_messages(shell: &SharedShell) {
    if authenticated_rest(shell).is_none() {
        return;
    }
    let batch = shell.borrow_mut().state.retry_failed_messages();
    let channel_ids: BTreeSet<Snowflake> =
        batch.iter().map(|(channel_id, _, _)| *channel_id).collect();
    for channel_id in channel_ids {
        apply_state_change(shell, StateChange::Messages { channel_id });
    }
    resend_next_message(shell, batch.into_iter());
}

/// One request at a time, each through `run_request`, so a sign-out stops the rest of the batch.
fn resend_next_message(
    shell: &SharedShell,
    mut remaining: std::vec::IntoIter<(Snowflake, String, SendMessageRequest)>,
) {
    let Some((channel_id, nonce, request)) = remaining.next() else {
        return;
    };
    let Some(rest) = authenticated_rest(shell) else {
        return;
    };
    let send_message_call = async move { rest.send_message(channel_id, &request).await };
    run_request(shell, send_message_call, move |shell, result| {
        let should_continue = result
            .as_ref()
            .map_or_else(ClientError::refuses_message_for_good, |_| true);
        apply_send_result(shell, channel_id, &nonce, result);
        if should_continue {
            resend_next_message(shell, remaining);
            return;
        }
        for (_, nonce, _) in remaining {
            let change = shell.borrow_mut().state.mark_send_failed(&nonce);
            apply_state_change(shell, change);
        }
    });
}

fn apply_send_result(
    shell: &SharedShell,
    channel_id: Snowflake,
    nonce: &str,
    result: Result<Message, ClientError>,
) {
    let change = match result {
        Ok(message) => shell
            .borrow_mut()
            .state
            .confirm_sent_message(nonce, message),
        Err(error) if error.refuses_message_for_good() => {
            report_request_error(shell, &error, "send message");
            let window = shell.borrow().window.upgrade();
            if let Some(window) = &window {
                window.set_message_action_error(message_actions::send_refusal_text(&error).into());
            }
            // One composer for every channel: the text goes back only into an empty one showing
            // its channel. Otherwise the row stays failed so the user can copy or discard it.
            let composer = window.filter(|window| {
                shell.borrow().selected_channel_id == Some(channel_id)
                    && window.get_composer_text().is_empty()
            });
            match composer {
                Some(window) => {
                    let refused = shell.borrow_mut().state.remove_refused_message(nonce);
                    if let Some(refused) = refused {
                        window.set_composer_text(refused.content.into());
                    }
                    StateChange::Messages { channel_id }
                }
                None => shell.borrow_mut().state.mark_send_failed(nonce),
            }
        }
        Err(error) => {
            if let Some(window) = shell.borrow().window.upgrade() {
                window.set_latest_announcement(MESSAGE_NOT_SENT_ANNOUNCEMENT.into());
            }
            report_request_error(shell, &error, "send message");
            shell.borrow_mut().state.mark_send_failed(nonce)
        }
    };
    apply_state_change(shell, change);
}

fn pending_nonce_at_row(shell: &LiveShell, row_index: i32) -> Option<String> {
    message_actions::pending_nonce_at_row(
        &shell.displayed_message_ids,
        &shell.displayed_pending_nonces,
        row_index,
    )
    .map(str::to_owned)
}

fn retry_message(shell: &SharedShell, row_index: i32) {
    let retried = {
        let mut shell = shell.borrow_mut();
        pending_nonce_at_row(&shell, row_index).and_then(|nonce| {
            let (channel_id, request) = shell.state.retry_pending_message(&nonce)?;
            Some((channel_id, nonce, request))
        })
    };
    let Some((channel_id, nonce, request)) = retried else {
        return;
    };
    apply_state_change(shell, StateChange::Messages { channel_id });
    deliver_pending_message(shell, channel_id, nonce, request);
}

fn discard_failed_message(shell: &SharedShell, row_index: i32) {
    let change = {
        let mut shell = shell.borrow_mut();
        match pending_nonce_at_row(&shell, row_index) {
            Some(nonce) => shell.state.discard_failed_message(&nonce),
            None => StateChange::Nothing,
        }
    };
    apply_state_change(shell, change);
}

fn refresh_server_items(shell: &LiveShell) {
    if let Some(window) = shell.window.upgrade() {
        window.set_servers(ModelRc::new(VecModel::from(live_rows::server_items(
            &shell.state,
        ))));
    }
}

fn refresh_channel_rows(shell: &LiveShell) {
    let Some(window) = shell.window.upgrade() else {
        return;
    };
    let rows = shell
        .selected_guild_id
        .and_then(|id| shell.state.guild(id))
        .map(|guild| {
            live_rows::channel_rows(
                guild,
                &shell.state,
                &shell.collapsed_category_ids,
                shell.selected_channel_id,
            )
        })
        .unwrap_or_default();
    window.set_channel_rows(ModelRc::new(VecModel::from(rows)));
    window.set_is_home(shell.selected_guild_id.is_none());
    window.set_direct_message_rows(ModelRc::new(VecModel::from(
        live_rows::direct_message_rows(&shell.state, shell.selected_channel_id),
    )));
    window.set_has_unread_direct_messages(live_rows::has_unread_direct_messages(&shell.state));
    refresh_home_melli_state(shell);
}

fn refresh_member_rows(shell: &LiveShell) {
    let Some(window) = shell.window.upgrade() else {
        return;
    };
    let members = shell
        .selected_guild_id
        .map(|id| shell.state.members(id))
        .unwrap_or_default();
    let guild = shell.selected_guild_id.and_then(|id| shell.state.guild(id));
    window.set_member_rows(ModelRc::new(VecModel::from(live_rows::member_rows(
        members,
        &shell.state,
        guild,
    ))));
}

/// Each member's highest role colour in the selected guild; none on Home.
fn selected_role_colors(shell: &LiveShell) -> HashMap<Snowflake, u32> {
    let guild = shell.selected_guild_id.and_then(|id| shell.state.guild(id));
    let members = guild
        .map(|guild| shell.state.members(guild.id))
        .unwrap_or_default();
    live_rows::role_colors_by_user(guild, members)
}

/// "Ana is typing…" above the composer (DESIGN.md §6.28), from who types in the open channel.
/// While anyone types, a timer checks again each second, since typing expires without an event.
fn refresh_typing(shell: &LiveShell) {
    let Some(window) = shell.window.upgrade() else {
        return;
    };
    let typing_user_ids = shell
        .selected_channel_id
        .map(|id| shell.state.typing_user_ids(id, now_unix_milliseconds()))
        .unwrap_or_default();
    let names: Vec<String> = typing_user_ids
        .iter()
        .map(|user_id| person_name(shell, *user_id))
        .collect();
    window.set_typing_line(live_rows::typing_line(&names));
    if names.is_empty() {
        shell.typing_timer.stop();
    } else if !shell.typing_timer.running() {
        let weak_shell = shell.self_reference.clone();
        shell.typing_timer.start(
            slint::TimerMode::Repeated,
            TYPING_REFRESH_INTERVAL,
            move || {
                if let Some(shell) = weak_shell.upgrade()
                    && let Ok(shell) = shell.try_borrow()
                {
                    refresh_typing(&shell);
                }
            },
        );
    }
}

/// A person's display name from the open server's members or the open conversation.
fn person_name(shell: &LiveShell, user_id: Snowflake) -> String {
    let members = shell
        .selected_guild_id
        .map(|id| shell.state.members(id))
        .unwrap_or_default()
        .iter()
        .map(|member| &member.user);
    let recipients = shell
        .selected_channel_id
        .and_then(|id| shell.state.direct_message_channel(id))
        .map(|channel| channel.recipients.as_slice())
        .unwrap_or_default();
    members
        .chain(recipients)
        .find(|user| user.id == user_id)
        .map_or_else(|| SOMEONE.to_owned(), |user| user.display_name.clone())
}

/// Who a mention in the open channel can name: the server's members, or the conversation's
/// participants, and the signed-in user.
fn selected_mentionable_people(shell: &LiveShell) -> MentionablePeople {
    let members = shell
        .selected_guild_id
        .map(|id| shell.state.members(id))
        .unwrap_or_default()
        .iter()
        .map(|member| &member.user);
    let recipients = shell
        .selected_channel_id
        .and_then(|id| shell.state.direct_message_channel(id))
        .map(|channel| channel.recipients.as_slice())
        .unwrap_or_default();
    let current_user = shell.state.current_user().map(|user| PublicUser {
        id: user.id,
        username: user.username.clone(),
        display_name: user.display_name.clone(),
        avatar: None,
        banner: None,
    });
    let people: Vec<&PublicUser> = members
        .chain(recipients)
        .chain(current_user.as_ref())
        .collect();
    MentionablePeople::new(people.iter().copied())
}

/// `component.mention.chip-text` in the current theme, as `#RRGGBB` for the message markup.
fn mention_color(shell: &LiveShell) -> String {
    shell.window.upgrade().map_or_else(String::new, |window| {
        let color = window.global::<MellowComponent>().get_mention_chip_text();
        format!(
            "#{:02X}{:02X}{:02X}",
            color.red(),
            color.green(),
            color.blue()
        )
    })
}

/// Whether the signed-in user may delete other people's messages in the open channel.
fn can_manage_messages(shell: &LiveShell) -> bool {
    shell
        .selected_channel_id
        .and_then(|id| shell.state.my_channel_permissions(id))
        .is_some_and(|permissions| permissions.contains(Permissions::MANAGE_MESSAGES))
}

fn refresh_messages(shell: &mut LiveShell) {
    let messages = shell
        .selected_channel_id
        .map(|id| shell.state.messages(id))
        .unwrap_or_default();
    let pending_messages: Vec<_> = shell
        .selected_channel_id
        .map(|id| shell.state.pending_messages(id).collect())
        .unwrap_or_default();
    let current_user = shell.state.current_user();
    let can_manage_messages = can_manage_messages(shell);
    shell.displayed_can_manage_messages = can_manage_messages;
    let role_colors = selected_role_colors(shell);
    let current_user_id = current_user.map(|user| user.id);
    // The visit's marker, once the channel's newest message is known: what lies between the read
    // position and it is new; nothing after it ever is.
    if shell.unread_marker.is_none()
        && let Some(last_read_message_id) = shell.read_position_at_open
        && let Some(newest) = messages.last()
    {
        shell.unread_marker =
            (newest.id > last_read_message_id).then_some(live_rows::UnreadMarker {
                last_read_message_id,
                newest_at_open_message_id: newest.id,
            });
        shell.read_position_at_open = None;
    }
    let has_reached_start = shell
        .selected_channel_id
        .is_some_and(|id| !shell.state.has_older_history(id));
    let unread_marker = shell.unread_marker;
    // The divider waits until the first new message is loaded, so it never jumps as pages arrive.
    let unread_divider_after_message_id = unread_marker
        .filter(|marker| live_rows::is_unread_start_loaded(messages, marker, has_reached_start))
        .map(|marker| marker.last_read_message_id);
    let unread_bar_text = unread_marker.map_or_else(String::new, |marker| {
        let summary =
            live_rows::unread_summary(messages, &marker, current_user_id, has_reached_start);
        if summary.count == 0 {
            String::new()
        } else {
            live_rows::unread_bar_label(&summary, marker.last_read_message_id.unix_milliseconds())
        }
    });
    let mentionable = selected_mentionable_people(shell);
    let mention_color = mention_color(shell);
    let people = live_rows::PeopleContext {
        role_colors: &role_colors,
        mentionable: &mentionable,
        mention_color: &mention_color,
    };
    let mut rows = live_rows::message_rows(
        messages,
        current_user_id,
        can_manage_messages,
        &people,
        unread_divider_after_message_id,
    );
    if let Some(current_user) = current_user {
        rows.extend(live_rows::pending_message_rows(
            &pending_messages,
            messages,
            current_user,
            &role_colors,
        ));
    }
    let message_ids = messages.iter().map(|message| message.id).collect();
    shell.displayed_pending_nonces = pending_messages
        .iter()
        .map(|pending| pending.nonce.clone())
        .collect();
    let previous_message_ids = std::mem::replace(&mut shell.displayed_message_ids, message_ids);
    shell.displayed_role_colors = role_colors;
    if let Some(window) = shell.window.upgrade() {
        window.set_unread_bar_text(unread_bar_text.into());
    }
    shell.messages.set_vec(rows);
    message_actions::remap_open_message_rows(shell, &previous_message_ids);
    refresh_history_start_state(shell);
}

fn refresh_history_start_state(shell: &LiveShell) {
    if let Some(window) = shell.window.upgrade() {
        let channel_id = shell.selected_channel_id;
        window.set_history_start_state(live_rows::history_start_state(
            &shell.state,
            channel_id,
            channel_id.is_some_and(|id| shell.failed_history_channel_ids.contains(&id)),
            channel_id.is_some_and(|id| shell.loading_older_history_channel_ids.contains(&id)),
        ));
    }
}
