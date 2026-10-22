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
use crate::generated_ui::{
    AppWindow, AttachmentRow, MellowComponent, MessageDelivery, MessageRow, QuickSwitcherKind,
    RoleColor,
};
use crate::mentions::{MentionablePeople, find_mentions};
use crate::message_markup::{styled_text_from_message_body, styled_text_with_mentions};
use crate::quick_switcher::{self, Destination};
use crate::{idle_benchmark, person_style, scroll_benchmark, shell_rows};

/// The busiest channel gets the 10 000-message history used by the scroll benchmark.
const BUSIEST_CHANNEL_NAME: &str = "general";
const BUSIEST_CHANNEL_MESSAGE_COUNT: usize = 10_000;
const QUIET_CHANNEL_MESSAGE_COUNT: usize = 40;
const DEMO_UNREAD_MESSAGE_COUNT: usize = 3;
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
    /// What each result of the open quick switcher leads to, by option index.
    quick_switcher_destinations: Vec<Destination>,
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

/// Role colours, the names a mention can use and the mention colour, for the fake messages.
struct DemoPeople {
    role_colors: HashMap<&'static str, u32>,
    mentionable: MentionablePeople,
    mention_color: String,
}

fn demo_people(window: &AppWindow) -> DemoPeople {
    let color = window.global::<MellowComponent>().get_mention_chip_text();
    let member_names: Vec<&str> = fake_role_groups()
        .iter()
        .flat_map(|group| group.members.iter().map(|member| member.display_name))
        .collect();
    DemoPeople {
        role_colors: role_colors_by_name(),
        mentionable: MentionablePeople::from_names(
            std::iter::once(CURRENT_USERNAME).chain(member_names),
        ),
        mention_color: format!(
            "#{:02X}{:02X}{:02X}",
            color.red(),
            color.green(),
            color.blue()
        ),
    }
}

fn message_row(message: &FakeMessage, people: &DemoPeople) -> MessageRow {
    let mentions = find_mentions(message.body, &people.mentionable);
    MessageRow {
        author: message.author.into(),
        author_initial: shell_rows::first_letter(message.author).into(),
        author_tint: person_style::avatar_tint_index(message.author),
        author_role_color: person_style::role_color(
            people.role_colors.get(message.author).copied(),
        ),
        timestamp: format!("Today at {}", message.time_of_day).into(),
        short_time: message.time_of_day.clone().into(),
        body: styled_text_with_mentions(message.body, &mentions, &people.mention_color),
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
        starts_unread: false,
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
    let people = demo_people(window);
    let mut rows: Vec<MessageRow> = fake_conversation(message_count_for_channel(channel_name))
        .iter()
        .map(|message| message_row(message, &people))
        .collect();
    // The last few messages are new, so the "New" divider and its bar show (DESIGN.md §5.4).
    let first_unread_index = rows.len().saturating_sub(DEMO_UNREAD_MESSAGE_COUNT);
    // Dated from the last message read, as in the live shell.
    let last_read_time = first_unread_index
        .checked_sub(1)
        .and_then(|index| rows.get(index))
        .map(|row| row.short_time.clone())
        .unwrap_or_default();
    let unread_bar_text = rows
        .get_mut(first_unread_index)
        .map_or_else(String::new, |row| {
            row.starts_unread = true;
            row.starts_author_group = true;
            format!("{DEMO_UNREAD_MESSAGE_COUNT} new messages since today at {last_read_time}")
        });
    state.messages.set_vec(rows);
    window.set_unread_bar_text(unread_bar_text.into());
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
        quick_switcher_destinations: Vec::new(),
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

/// The quick switcher on fake data: the shared fake channels and the fake servers; no
/// conversations, since the demo has none.
fn connect_quick_switcher(
    window: &AppWindow,
    state: &Rc<RefCell<ShellState>>,
    servers: &[FakeServer],
) {
    let server_destinations: Vec<Destination> = servers
        .iter()
        .enumerate()
        .map(|(index, server)| Destination {
            kind: QuickSwitcherKind::Server,
            id: index.to_string(),
            server_id: index.to_string(),
            name: server.name.to_owned(),
            place: String::new(),
            has_unread: server.has_unread_messages,
        })
        .collect();
    let place = servers
        .first()
        .map(|server| server.name.to_owned())
        .unwrap_or_default();
    let weak_window = window.as_weak();
    let shared_state = state.clone();
    window.on_quick_switcher_query_changed(move |query| {
        let Some(window) = weak_window.upgrade() else {
            return;
        };
        let mut state = shared_state.borrow_mut();
        let channels = state
            .categories
            .iter()
            .flat_map(|category| &category.channels)
            .map(|channel| Destination {
                kind: QuickSwitcherKind::Channel,
                id: channel.name.to_owned(),
                server_id: String::new(),
                name: channel.name.to_owned(),
                // The demo's servers all share these channels; name the first one.
                place: place.clone(),
                has_unread: channel.has_unread_messages,
            });
        let destinations: Vec<Destination> = channels.chain(server_destinations.clone()).collect();
        let (rows, chosen) = quick_switcher::results(&destinations, &query);
        state.quick_switcher_destinations = chosen;
        window.set_quick_switcher_rows(ModelRc::new(VecModel::from(rows)));
    });
    let weak_window = window.as_weak();
    let shared_state = state.clone();
    window.on_quick_switcher_chosen(move |index| {
        let Some(window) = weak_window.upgrade() else {
            return;
        };
        let destination = usize::try_from(index).ok().and_then(|index| {
            shared_state
                .borrow()
                .quick_switcher_destinations
                .get(index)
                .cloned()
        });
        match destination {
            Some(destination) if destination.kind == QuickSwitcherKind::Channel => {
                open_channel(&window, &mut shared_state.borrow_mut(), &destination.id);
            }
            Some(destination) => {
                if let Ok(server_index) = destination.id.parse::<i32>() {
                    window.invoke_select_server(server_index);
                }
            }
            None => {}
        }
    });
}

fn connect_navigation_callbacks(
    window: &AppWindow,
    state: &Rc<RefCell<ShellState>>,
    servers: Vec<FakeServer>,
) {
    // Mentions are coloured in Rust, so the rows follow the theme.
    let weak_window = window.as_weak();
    let shared_state = state.clone();
    window.on_theme_changed(move || {
        if let Some(window) = weak_window.upgrade() {
            let channel_name = shared_state.borrow().selected_channel_name.clone();
            open_channel(&window, &mut shared_state.borrow_mut(), &channel_name);
        }
    });

    connect_quick_switcher(window, state, &servers);

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

/// "Mark as read": the divider and its bar go.
fn forget_unread_divider(window: &AppWindow, state: &ShellState) {
    window.set_unread_bar_text(SharedString::default());
    let messages = &state.messages;
    if let Some(index) = (0..messages.row_count()).find(|index| {
        messages
            .row_data(*index)
            .is_some_and(|row| row.starts_unread)
    }) && let Some(mut row) = messages.row_data(index)
    {
        row.starts_unread = false;
        messages.set_row_data(index, row);
    }
}

fn connect_message_callbacks(window: &AppWindow, state: &Rc<RefCell<ShellState>>) {
    let weak_window = window.as_weak();
    let shared_state = state.clone();
    window.on_mark_channel_read(move || {
        if let Some(window) = weak_window.upgrade() {
            forget_unread_divider(&window, &shared_state.borrow());
        }
    });
    let weak_window = window.as_weak();
    let shared_state = state.clone();
    window.on_unread_divider_read(move || {
        if let Some(window) = weak_window.upgrade() {
            forget_unread_divider(&window, &shared_state.borrow());
        }
    });

    let weak_window = window.as_weak();
    let shared_state = state.clone();
    window.on_send_message(move |text: SharedString| {
        let Some(window) = weak_window.upgrade() else {
            return;
        };
        if text.trim().is_empty() {
            return;
        }
        forget_unread_divider(&window, &shared_state.borrow());
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
            starts_unread: false,
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
