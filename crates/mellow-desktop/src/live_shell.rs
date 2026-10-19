//! The shell connected to a real instance through the client core (ADR-0011).
//!
//! State lives on the UI thread; network calls run on a Tokio runtime and their results come
//! back through Slint's event loop (`slint::spawn_local`), so no lock is ever shared with the UI.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashSet};
use std::future::Future;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mellow_client_core::state::{ClientState, StateChange};
use mellow_client_core::{ClientError, GatewayUpdate, InstanceAddress, RestClient, run_gateway};
use mellow_protocol::{
    ApiErrorCode, LoginRequest, MessageHistoryQuery, RegisterRequest, SendMessageRequest, Snowflake,
};
use slint::{ComponentHandle as _, Model as _, ModelRc, SharedString, VecModel, Weak};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use crate::generated_ui::{AppWindow, MessageRow};
use crate::{clipboard, live_rows, shell_rows};

const GATEWAY_UPDATE_QUEUE_CAPACITY: usize = 256;
const LOGOUT_ON_EXIT_TIMEOUT: Duration = Duration::from_secs(3);
const RECONNECTING_NOTICE: &str = "Connection lost. Reconnecting…";
const SESSION_ENDED_MESSAGE: &str = "Your session ended. Sign in again.";
/// Shown when the rail's home entry (index -1 in `server_rail.slint`) is selected.
const HOME_SERVER_NAME: &str = "Direct messages";

struct Connection {
    rest: RestClient,
    gateway: tokio::task::AbortHandle,
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
    messages: Rc<VecModel<MessageRow>>,
    /// See `clipboard::copy_text`.
    clipboard: Option<arboard::Clipboard>,
}

pub type SharedShell = Rc<RefCell<LiveShell>>;

pub fn start(window: &AppWindow, runtime: Handle) -> SharedShell {
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
        messages: Rc::new(VecModel::default()),
        clipboard: None,
    }));
    window.set_is_signed_in(false);
    window.set_messages(shell.borrow().messages.clone().into());
    connect_callbacks(window, &shell);
    shell
}

/// Revokes the session token on the server: it is never stored on disk, so a session left open
/// would stay valid for nothing until it expires.
pub fn sign_out_before_exit(shell: &SharedShell) {
    let (runtime, connection) = {
        let mut shell = shell.borrow_mut();
        (shell.runtime.clone(), shell.connection.take())
    };
    let Some(Connection { mut rest, gateway }) = connection else {
        return;
    };
    gateway.abort();
    let logout = runtime.block_on(tokio::time::timeout(LOGOUT_ON_EXIT_TIMEOUT, rest.logout()));
    if !matches!(logout, Ok(Ok(()))) {
        tracing::warn!("could not end the session on the instance before exiting");
    }
}

fn connect_callbacks(window: &AppWindow, shell: &SharedShell) {
    let shared = shell.clone();
    window.on_sign_in(move || submit_credentials(&shared, false));
    let shared = shell.clone();
    window.on_register(move || submit_credentials(&shared, true));

    let shared = shell.clone();
    window.on_select_server(move |server_index| {
        let guild_id = usize::try_from(server_index).ok().and_then(|index| {
            shared
                .borrow()
                .state
                .guilds()
                .get(index)
                .map(|guild| guild.id)
        });
        select_guild(&shared, guild_id);
    });

    let shared = shell.clone();
    window.on_select_channel(move |channel_id: SharedString| {
        if let Ok(channel_id) = channel_id.parse() {
            open_channel(&shared, Some(channel_id));
        }
    });

    let shared = shell.clone();
    window.on_toggle_category(move |category_index| {
        {
            let mut shell = shared.borrow_mut();
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
        refresh_channel_rows(&shared.borrow());
    });

    let shared = shell.clone();
    window.on_send_message(move |text: SharedString| send_message(&shared, text));

    let shared = shell.clone();
    window.on_copy_message_text(move |message_index| {
        let mut shell = shared.borrow_mut();
        let Some(message) = usize::try_from(message_index)
            .ok()
            .and_then(|index| shell.messages.row_data(index))
        else {
            return;
        };
        clipboard::copy_text(&mut shell.clipboard, &message.body_plain);
    });

    window.on_link_clicked(|_url| {
        // shortcut: links are not opened yet; opening needs the confirmation dialog of DESIGN.md §5.4.
    });
}

enum Credentials {
    Login(LoginRequest),
    Register(RegisterRequest),
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
        Ok(rest)
    };
    run_request(shell, request, move |shell, result| {
        let Some(window) = shell.borrow().window.upgrade() else {
            return;
        };
        window.set_login_is_busy(false);
        match result {
            Ok(rest) => connect_gateway(shell, rest),
            Err(error) => show_login_error(&window, &sign_in_error_message(&error, &host)),
        }
    });
}

/// Cleared first so the same message set twice is announced again by screen readers.
fn show_login_error(window: &AppWindow, message: &str) {
    window.set_login_error_message(SharedString::default());
    window.set_login_error_message(message.into());
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
        ClientError::InvalidInstanceAddress(_) | ClientError::NotSignedIn => error.to_string(),
    }
}

/// Runs `request` on the network runtime, then `on_done` on the UI thread with its result.
fn run_request<T: Send + 'static>(
    shell: &SharedShell,
    request: impl Future<Output = Result<T, ClientError>> + Send + 'static,
    on_done: impl FnOnce(&SharedShell, Result<T, ClientError>) + 'static,
) {
    let task = shell.borrow().runtime.spawn(request);
    let shell = shell.clone();
    let scheduled = slint::spawn_local(async move {
        match task.await {
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

fn connect_gateway(shell: &SharedShell, rest: RestClient) {
    let Some(token) = rest.token().map(|token| Zeroizing::new(token.to_owned())) else {
        return;
    };
    let (sender, mut updates) = mpsc::channel(GATEWAY_UPDATE_QUEUE_CAPACITY);
    {
        let mut shell_mut = shell.borrow_mut();
        let gateway = shell_mut
            .runtime
            .spawn(run_gateway(rest.instance().clone(), token, sender))
            .abort_handle();
        shell_mut.connection = Some(Connection { rest, gateway });
        if let Some(window) = shell_mut.window.upgrade() {
            window.set_is_signed_in(true);
            window.set_connection_notice("Connecting…".into());
        }
    }
    let shell = shell.clone();
    let scheduled = slint::spawn_local(async move {
        while let Some(update) = updates.recv().await {
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

fn apply_gateway_update(shell: &SharedShell, update: GatewayUpdate) {
    let set_notice = |notice: &str| {
        if let Some(window) = shell.borrow().window.upgrade() {
            window.set_connection_notice(notice.into());
        }
    };
    match update {
        GatewayUpdate::Ready(ready) => {
            let previous_guild_id = {
                let mut shell = shell.borrow_mut();
                shell.state.apply_ready(*ready);
                shell.requested_history_channel_ids.clear();
                shell.requested_member_list_guild_ids.clear();
                if let (Some(window), Some(user)) =
                    (shell.window.upgrade(), shell.state.current_user())
                {
                    window.set_current_user_name(user.display_name.as_str().into());
                    window.set_current_user_initial(
                        shell_rows::first_letter(&user.display_name).into(),
                    );
                }
                shell.selected_guild_id
            };
            set_notice("");
            let guild_id = {
                let shell = shell.borrow();
                previous_guild_id
                    .filter(|id| shell.state.guild(*id).is_some())
                    .or_else(|| shell.state.guilds().first().map(|guild| guild.id))
            };
            select_guild(shell, guild_id);
        }
        GatewayUpdate::Event(event) => {
            let change = shell
                .borrow_mut()
                .state
                .apply_event(event, now_unix_milliseconds());
            apply_state_change(shell, change);
        }
        GatewayUpdate::Resumed => set_notice(""),
        GatewayUpdate::Disconnected => set_notice(RECONNECTING_NOTICE),
        GatewayUpdate::SignedOut => end_session(shell, SESSION_ENDED_MESSAGE),
    }
}

fn apply_state_change(shell: &SharedShell, change: StateChange) {
    let (selected_guild_id, selected_channel_id) = {
        let shell = shell.borrow();
        (shell.selected_guild_id, shell.selected_channel_id)
    };
    match change {
        StateChange::Nothing | StateChange::Typing { .. } => return,
        StateChange::Guilds => {
            // The selected guild or channel may be gone.
            let guild_still_exists =
                selected_guild_id.is_some_and(|id| shell.borrow().state.guild(id).is_some());
            if !guild_still_exists {
                let first_guild_id = shell.borrow().state.guilds().first().map(|guild| guild.id);
                select_guild(shell, first_guild_id);
                return;
            }
            let channel_still_exists = {
                let shell = shell.borrow();
                selected_guild_id
                    .and_then(|id| shell.state.guild(id))
                    .is_some_and(|guild| {
                        guild
                            .channels
                            .iter()
                            .any(|channel| Some(channel.id) == selected_channel_id)
                    })
            };
            if !channel_still_exists {
                let first_channel_id = selected_guild_id.and_then(|id| {
                    shell
                        .borrow()
                        .state
                        .guild(id)
                        .and_then(live_rows::first_text_channel_id)
                });
                open_channel(shell, first_channel_id);
            }
        }
        StateChange::Messages { channel_id } if Some(channel_id) == selected_channel_id => {
            refresh_messages(&shell.borrow());
            acknowledge_selected_channel(shell);
        }
        StateChange::Members { guild_id } if Some(guild_id) == selected_guild_id => {
            refresh_member_rows(&shell.borrow());
        }
        StateChange::Presence => refresh_member_rows(&shell.borrow()),
        StateChange::Messages { .. } | StateChange::Members { .. } | StateChange::ReadStates => {}
    }
    let shell = shell.borrow();
    refresh_server_items(&shell);
    refresh_channel_rows(&shell);
}

fn end_session(shell: &SharedShell, message: &str) {
    let mut shell = shell.borrow_mut();
    if let Some(connection) = shell.connection.take() {
        connection.gateway.abort();
    }
    shell.state = ClientState::default();
    shell.selected_guild_id = None;
    shell.selected_channel_id = None;
    shell.messages.set_vec(Vec::new());
    if let Some(window) = shell.window.upgrade() {
        window.set_is_signed_in(false);
        window.set_connection_notice(SharedString::default());
        window.set_login_is_busy(false);
        show_login_error(&window, message);
    }
}

fn select_guild(shell: &SharedShell, guild_id: Option<Snowflake>) {
    let first_channel_id = {
        let mut shell_mut = shell.borrow_mut();
        shell_mut.selected_guild_id = guild_id;
        let guild = guild_id.and_then(|id| shell_mut.state.guild(id));
        let first_channel_id = guild.and_then(live_rows::first_text_channel_id);
        let server_name = guild.map_or(HOME_SERVER_NAME.to_owned(), |guild| guild.name.clone());
        let server_index = guild_id
            .and_then(|id| {
                shell_mut
                    .state
                    .guilds()
                    .iter()
                    .position(|guild| guild.id == id)
            })
            .map_or(-1, shell_rows::saturating_int);
        if let Some(window) = shell_mut.window.upgrade() {
            window.set_server_name(server_name.into());
            window.set_selected_server_index(server_index);
        }
        first_channel_id
    };
    {
        let shell = shell.borrow();
        refresh_server_items(&shell);
        refresh_member_rows(&shell);
    }
    if let Some(guild_id) = guild_id {
        ensure_members_loaded(shell, guild_id);
    }
    // shortcut: home (direct messages) shows an empty channel list until DMs land in the client.
    open_channel(shell, first_channel_id);
}

fn open_channel(shell: &SharedShell, channel_id: Option<Snowflake>) {
    {
        let mut shell_mut = shell.borrow_mut();
        shell_mut.selected_channel_id = channel_id;
        let channel = shell_mut
            .selected_guild_id
            .and_then(|id| shell_mut.state.guild(id))
            .and_then(|guild| {
                guild
                    .channels
                    .iter()
                    .find(|channel| Some(channel.id) == channel_id)
            });
        if let Some(window) = shell_mut.window.upgrade() {
            window.set_selected_channel_id(
                channel_id
                    .map(|id| id.to_string())
                    .unwrap_or_default()
                    .into(),
            );
            window.set_selected_channel_name(
                channel
                    .map(|channel| channel.name.as_str())
                    .unwrap_or_default()
                    .into(),
            );
            window.set_selected_channel_topic(
                channel
                    .map(|channel| channel.topic.as_str())
                    .unwrap_or_default()
                    .into(),
            );
        }
        refresh_channel_rows(&shell_mut);
        refresh_messages(&shell_mut);
    }
    if let Some(channel_id) = channel_id {
        ensure_history_loaded(shell, channel_id);
        acknowledge_selected_channel(shell);
    }
}

fn ensure_history_loaded(shell: &SharedShell, channel_id: Snowflake) {
    if !shell
        .borrow_mut()
        .requested_history_channel_ids
        .insert(channel_id)
    {
        return;
    }
    let Some(rest) = authenticated_rest(shell) else {
        return;
    };
    // shortcut: only the latest page is loaded; older history (scrolling up) comes later.
    let request = async move {
        rest.list_messages(channel_id, &MessageHistoryQuery::default())
            .await
    };
    run_request(shell, request, move |shell, result| match result {
        Ok(messages) => {
            let change = shell
                .borrow_mut()
                .state
                .insert_messages(channel_id, messages);
            apply_state_change(shell, change);
        }
        Err(error) => {
            shell
                .borrow_mut()
                .requested_history_channel_ids
                .remove(&channel_id);
            report_request_error(shell, &error, "load message history");
        }
    });
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
    let content = text.trim().to_owned();
    let (Some(channel_id), Some(rest)) = (
        shell.borrow().selected_channel_id,
        authenticated_rest(shell),
    ) else {
        return;
    };
    if content.is_empty() {
        return;
    }
    let request = async move {
        let request = SendMessageRequest {
            content,
            reply_to_id: None,
            nonce: None,
        };
        rest.send_message(channel_id, &request).await
    };
    run_request(shell, request, move |shell, result| match result {
        Ok(message) => {
            let change = shell
                .borrow_mut()
                .state
                .insert_messages(channel_id, vec![message]);
            apply_state_change(shell, change);
            if let Some(window) = shell.borrow().window.upgrade() {
                window.invoke_scroll_to_latest_message();
            }
        }
        Err(error) => {
            // shortcut: the text goes back to the composer instead of a "failed, retry" message row.
            if let Some(window) = shell.borrow().window.upgrade()
                && window.get_composer_text().is_empty()
            {
                window.set_composer_text(text);
                window.set_latest_announcement(format!("Message not sent: {error}").into());
            }
            report_request_error(shell, &error, "send message");
        }
    });
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
}

fn refresh_member_rows(shell: &LiveShell) {
    let Some(window) = shell.window.upgrade() else {
        return;
    };
    let members = shell
        .selected_guild_id
        .map(|id| shell.state.members(id))
        .unwrap_or_default();
    window.set_member_rows(ModelRc::new(VecModel::from(live_rows::member_rows(
        members,
        &shell.state,
    ))));
}

fn refresh_messages(shell: &LiveShell) {
    let messages = shell
        .selected_channel_id
        .map(|id| shell.state.messages(id))
        .unwrap_or_default();
    shell.messages.set_vec(live_rows::message_rows(messages));
    // shortcut: always jumps to the latest message, even when the reader had scrolled up.
    if let Some(window) = shell.window.upgrade() {
        crate::scroll_to_latest_message_after_layout(&window);
    }
}
