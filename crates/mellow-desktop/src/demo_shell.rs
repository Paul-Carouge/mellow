//! The shell on fake data (ADR-0012), started with `MELLOW_DEMO_WORKSPACE=1`.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use slint::{ComponentHandle as _, Model as _, ModelRc, SharedString, VecModel};

use crate::clipboard;
use crate::fake_messages::{FakeMessage, fake_conversation};
use crate::fake_workspace::{
    FakeCategory, FakeServer, fake_categories, fake_role_groups, fake_servers,
};
use crate::generated_ui::{AppWindow, AttachmentRow, MessageDelivery, MessageRow, RoleColor};
use crate::message_markup::styled_text_from_message_body;
use crate::{idle_benchmark, person_style, scroll_benchmark, shell_rows};

/// The busiest channel gets the 10 000-message history used by the scroll benchmark.
const BUSIEST_CHANNEL_NAME: &str = "general";
const BUSIEST_CHANNEL_MESSAGE_COUNT: usize = 10_000;
const QUIET_CHANNEL_MESSAGE_COUNT: usize = 40;
const CURRENT_USER_NAME: &str = "You";
const CURRENT_USERNAME: &str = "you";
const FAKE_ATTACHMENT_NAME: &str = "release-checklist.ods";
const FAKE_ATTACHMENT_SIZE: &str = "18.0 KB";
/// Shown when the rail's home entry (index -1 in `server_rail.slint`) is selected.
const HOME_SERVER_NAME: &str = "Direct messages";

struct ShellState {
    categories: Vec<FakeCategory>,
    collapsed_category_indexes: BTreeSet<usize>,
    selected_channel_name: String,
    messages: Rc<VecModel<MessageRow>>,
    /// See `clipboard::copy_text`.
    clipboard: Option<arboard::Clipboard>,
}

/// Each fake member's role colour, by display name (the demo's stand-in for an id).
fn role_colors_by_name() -> HashMap<&'static str, u32> {
    fake_role_groups()
        .iter()
        .filter_map(|group| group.role_color.map(|color| (group, color)))
        .flat_map(|(group, color)| {
            group
                .members
                .iter()
                .map(move |member| (member.display_name, color))
        })
        .collect()
}

fn message_row(message: &FakeMessage, role_colors: &HashMap<&'static str, u32>) -> MessageRow {
    MessageRow {
        author: message.author.into(),
        author_initial: shell_rows::first_letter(message.author).into(),
        author_tint: person_style::avatar_tint_index(message.author),
        author_role_color: person_style::role_color(role_colors.get(message.author).copied()),
        timestamp: format!("Today at {}", message.time_of_day).into(),
        short_time: message.time_of_day.clone().into(),
        body: styled_text_from_message_body(message.body),
        body_plain: message.body.into(),
        attachments: if message.has_attachment {
            ModelRc::new(VecModel::from(vec![AttachmentRow {
                name: FAKE_ATTACHMENT_NAME.into(),
                size: FAKE_ATTACHMENT_SIZE.into(),
            }]))
        } else {
            ModelRc::default()
        },
        starts_author_group: message.starts_author_group,
        reply_author: message.reply_author.unwrap_or_default().into(),
        reply_author_initial: shell_rows::first_letter(message.reply_author.unwrap_or_default())
            .into(),
        reply_author_tint: person_style::avatar_tint_index(
            message.reply_author.unwrap_or_default(),
        ),
        reply_preview: message.reply_preview.unwrap_or_default().into(),
        mentions_current_user: message.mentions_current_user,
        is_own: message.author == CURRENT_USER_NAME,
        can_delete: message.author == CURRENT_USER_NAME,
        is_edited: false,
        delivery: MessageDelivery::Sent,
    }
}

fn message_count_for_channel(channel_name: &str) -> usize {
    if channel_name == BUSIEST_CHANNEL_NAME {
        BUSIEST_CHANNEL_MESSAGE_COUNT
    } else {
        QUIET_CHANNEL_MESSAGE_COUNT
    }
}

fn refresh_channel_rows(window: &AppWindow, state: &ShellState) {
    let rows = shell_rows::channel_rows(
        &state.categories,
        &state.collapsed_category_indexes,
        &state.selected_channel_name,
    );
    window.set_channel_rows(ModelRc::new(VecModel::from(rows)));
}

fn open_channel(window: &AppWindow, state: &mut ShellState, channel_name: &str) {
    channel_name.clone_into(&mut state.selected_channel_name);
    let topic = state
        .categories
        .iter()
        .flat_map(|category| &category.channels)
        .find(|channel| channel.name == channel_name)
        .map(|channel| channel.topic)
        .unwrap_or_default();
    let role_colors = role_colors_by_name();
    state.messages.set_vec(
        fake_conversation(message_count_for_channel(channel_name))
            .iter()
            .map(|message| message_row(message, &role_colors))
            .collect::<Vec<_>>(),
    );
    window.set_selected_channel_id(channel_name.into());
    window.set_selected_channel_name(channel_name.into());
    window.set_selected_channel_topic(topic.into());
    refresh_channel_rows(window, state);
    crate::scroll_to_latest_message_after_layout(window);
}

/// Fills the shell with deterministic fake data: used for design reviews, window snapshots and
/// the scroll benchmark, which must not depend on a running instance.
pub fn start(window: &AppWindow) -> Result<(), slint::PlatformError> {
    let servers = fake_servers();

    let state = Rc::new(RefCell::new(ShellState {
        categories: fake_categories(),
        collapsed_category_indexes: BTreeSet::new(),
        selected_channel_name: String::new(),
        messages: Rc::new(VecModel::default()),
        clipboard: None,
    }));

    window.set_is_signed_in(true);
    window.set_servers(ModelRc::new(VecModel::from(shell_rows::server_items(
        &servers,
    ))));
    window.set_selected_server_index(0);
    window.set_server_name(
        servers
            .first()
            .map(|server| server.name)
            .unwrap_or_default()
            .into(),
    );
    window.set_member_rows(ModelRc::new(VecModel::from(shell_rows::member_rows(
        &fake_role_groups(),
    ))));
    window.set_current_user_name(CURRENT_USER_NAME.into());
    window.set_current_user_initial(shell_rows::first_letter(CURRENT_USER_NAME).into());
    window.set_current_user_tint(person_style::avatar_tint_index(CURRENT_USER_NAME));
    window.set_account_display_name(CURRENT_USER_NAME.into());
    window.set_account_username(CURRENT_USERNAME.into());
    window.set_messages(state.borrow().messages.clone().into());
    open_channel(window, &mut state.borrow_mut(), BUSIEST_CHANNEL_NAME);

    connect_navigation_callbacks(window, &state, servers);
    connect_message_callbacks(window, &state);

    if std::env::var_os(scroll_benchmark::ENABLE_SCROLL_BENCHMARK_VARIABLE).is_some() {
        scroll_benchmark::start(window)?;
    }
    if std::env::var_os(idle_benchmark::ENABLE_IDLE_BENCHMARK_VARIABLE).is_some() {
        idle_benchmark::start(window)?;
    }
    Ok(())
}

fn connect_navigation_callbacks(
    window: &AppWindow,
    state: &Rc<RefCell<ShellState>>,
    servers: Vec<FakeServer>,
) {
    let weak_window = window.as_weak();
    window.on_select_server(move |server_index| {
        let Some(window) = weak_window.upgrade() else {
            return;
        };
        window.set_selected_server_index(server_index);
        let server_name = usize::try_from(server_index)
            .ok()
            .and_then(|index| servers.get(index))
            // A negative index is the home entry.
            .map_or(HOME_SERVER_NAME, |server| server.name);
        window.set_server_name(server_name.into());
        // shortcut: every fake server, and home, shares the same channels until the client core exists.
    });

    let weak_window = window.as_weak();
    let shared_state = state.clone();
    window.on_select_channel(move |channel_name: SharedString| {
        if let Some(window) = weak_window.upgrade() {
            open_channel(&window, &mut shared_state.borrow_mut(), &channel_name);
        }
    });

    let weak_window = window.as_weak();
    let shared_state = state.clone();
    window.on_toggle_category(move |category_index| {
        let Some(window) = weak_window.upgrade() else {
            return;
        };
        let Ok(category_index) = usize::try_from(category_index) else {
            return;
        };
        let mut state = shared_state.borrow_mut();
        if !state.collapsed_category_indexes.remove(&category_index) {
            state.collapsed_category_indexes.insert(category_index);
        }
        refresh_channel_rows(&window, &state);
    });
}

fn connect_message_callbacks(window: &AppWindow, state: &Rc<RefCell<ShellState>>) {
    let weak_window = window.as_weak();
    let shared_state = state.clone();
    window.on_send_message(move |text: SharedString| {
        let Some(window) = weak_window.upgrade() else {
            return;
        };
        if text.trim().is_empty() {
            return;
        }
        let messages = shared_state.borrow().messages.clone();
        let last_author = messages
            .row_data(messages.row_count().saturating_sub(1))
            .map(|row| row.author);
        messages.push(MessageRow {
            author: CURRENT_USER_NAME.into(),
            author_initial: shell_rows::first_letter(CURRENT_USER_NAME).into(),
            author_tint: person_style::avatar_tint_index(CURRENT_USER_NAME),
            author_role_color: RoleColor::default(),
            // shortcut: fake messages have no real clock; the client core will provide timestamps.
            timestamp: "Today".into(),
            short_time: SharedString::default(),
            body: styled_text_from_message_body(&text),
            body_plain: text.clone(),
            attachments: slint::ModelRc::default(),
            starts_author_group: last_author.as_deref() != Some(CURRENT_USER_NAME),
            reply_author: SharedString::default(),
            reply_author_initial: SharedString::default(),
            reply_author_tint: 0,
            reply_preview: SharedString::default(),
            mentions_current_user: false,
            // shortcut: the demo shows Edit and Delete on its own messages but does not apply them.
            is_own: true,
            can_delete: true,
            is_edited: false,
            delivery: MessageDelivery::Sent,
        });
        window.set_latest_announcement(format!("{CURRENT_USER_NAME}: {text}").into());
        window.invoke_scroll_to_latest_message();
    });

    let shared_state = state.clone();
    window.on_copy_message_text(move |message_index| {
        let mut state = shared_state.borrow_mut();
        let Some(message) = usize::try_from(message_index)
            .ok()
            .and_then(|index| state.messages.row_data(index))
        else {
            return;
        };
        clipboard::copy_text(&mut state.clipboard, &message.body_plain);
    });

    window.on_link_clicked(|_url| {
        // shortcut: links are not opened yet; opening needs the confirmation dialog of DESIGN.md §5.4.
    });
}
