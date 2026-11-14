//! A small signed-in world for the shell's tests: two servers, a category with text and voice
//! channels, two conversations and a few members, as `Ready` brings them.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyView, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, TestAppContext, VisualTestContext, Window, WindowAppearance, div, px, size,
};
use mellow_client_core::state::ClientState;
use mellow_protocol::{
    Channel, ChannelKind, ChannelUnread, ChosenStatus, CurrentUser, DirectMessageChannel, Guild,
    Member, Permissions, Presence, PresenceStatus, PublicUser, ReadState, Ready, Role, Snowflake,
};

pub const ME: Snowflake = Snowflake::from_raw(1000);
pub const ANA: Snowflake = Snowflake::from_raw(1001);
pub const ZOE: Snowflake = Snowflake::from_raw(1002);

pub const GUILD_RUST: Snowflake = Snowflake::from_raw(1);
pub const GUILD_BOOKS: Snowflake = Snowflake::from_raw(2);
pub const RUST_TALK: Snowflake = Snowflake::from_raw(20);
pub const RUST_ANNOUNCEMENTS: Snowflake = Snowflake::from_raw(21);
pub const RUST_GENERAL: Snowflake = Snowflake::from_raw(22);
pub const RUST_HELP: Snowflake = Snowflake::from_raw(23);
pub const RUST_VOICE: Snowflake = Snowflake::from_raw(24);
pub const BOOKS_READING: Snowflake = Snowflake::from_raw(31);
pub const MODERATORS: Snowflake = Snowflake::from_raw(90);

pub const CONVERSATION_ANA: Snowflake = Snowflake::from_raw(50);
pub const CONVERSATION_ZOE: Snowflake = Snowflake::from_raw(51);

/// The newest message everywhere; read positions sit on it unless a test makes a channel unread.
const NEWEST_MESSAGE: Snowflake = Snowflake::from_raw(9_000);

pub fn person(id: Snowflake, username: &str, display_name: &str) -> PublicUser {
    PublicUser {
        id,
        username: username.to_owned(),
        display_name: display_name.to_owned(),
        avatar: None,
        banner: None,
    }
}

/// A plain text message by `author` in `channel_id`.
pub fn text_message(
    id: u64,
    channel_id: Snowflake,
    author: &PublicUser,
    content: &str,
) -> mellow_protocol::Message {
    mellow_protocol::Message {
        id: Snowflake::from_raw(id),
        channel_id,
        author: author.clone(),
        content: content.to_owned(),
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

fn channel(
    id: Snowflake,
    guild_id: Snowflake,
    kind: ChannelKind,
    name: &str,
    parent_id: Option<Snowflake>,
    position: u32,
) -> Channel {
    Channel {
        id,
        guild_id,
        kind,
        name: name.to_owned(),
        topic: if id == RUST_GENERAL {
            "Everything Rust, in Lyon".to_owned()
        } else {
            String::new()
        },
        parent_id,
        position,
        icon: None,
        permission_overwrites: Vec::new(),
        user_limit: None,
    }
}

fn guild(id: Snowflake, name: &str, channels: Vec<Channel>) -> Guild {
    Guild {
        id,
        name: name.to_owned(),
        owner_id: ANA,
        icon: None,
        my_permissions: Permissions::VIEW_CHANNEL
            | Permissions::SEND_MESSAGES
            | Permissions::READ_MESSAGE_HISTORY,
        my_role_ids: Vec::new(),
        roles: vec![Role {
            id: MODERATORS,
            name: "Moderators".to_owned(),
            color: 0,
            position: 1,
            permissions: Permissions::empty(),
            is_displayed_separately: true,
            is_mentionable: false,
            icon: None,
            unicode_emoji: None,
        }],
        channels,
        emojis: Vec::new(),
        sounds: Vec::new(),
        my_join_sound_id: None,
    }
}

fn conversation(id: Snowflake, with: PublicUser, last_message: u64) -> DirectMessageChannel {
    DirectMessageChannel {
        id,
        recipients: vec![with],
        last_message_id: Some(Snowflake::from_raw(last_message)),
        owner_id: None,
        name: String::new(),
    }
}

/// `unreads`: channels with messages past the reader's position, and how many mention them.
pub fn sample_state_with_unreads(unreads: &[(Snowflake, u32)]) -> ClientState {
    use ChannelKind::{Category, Text, Voice};
    let rust = guild(
        GUILD_RUST,
        "Rust Lyon",
        vec![
            channel(RUST_TALK, GUILD_RUST, Category, "Talk", None, 1),
            channel(
                RUST_ANNOUNCEMENTS,
                GUILD_RUST,
                Text,
                "announcements",
                None,
                0,
            ),
            channel(
                RUST_GENERAL,
                GUILD_RUST,
                Text,
                "general",
                Some(RUST_TALK),
                0,
            ),
            channel(RUST_HELP, GUILD_RUST, Text, "help", Some(RUST_TALK), 1),
            channel(RUST_VOICE, GUILD_RUST, Voice, "Lounge", Some(RUST_TALK), 2),
        ],
    );
    let books = guild(
        GUILD_BOOKS,
        "Books",
        vec![channel(
            BOOKS_READING,
            GUILD_BOOKS,
            Text,
            "reading",
            None,
            0,
        )],
    );
    let conversations = vec![
        conversation(CONVERSATION_ZOE, person(ZOE, "zoe", "Zoé"), 8_000),
        conversation(CONVERSATION_ANA, person(ANA, "ana", "Ana"), 8_500),
    ];
    let read_states = [CONVERSATION_ANA, CONVERSATION_ZOE]
        .into_iter()
        .filter(|id| !unreads.iter().any(|(unread, _)| unread == id))
        .map(|channel_id| ReadState {
            channel_id,
            last_read_message_id: NEWEST_MESSAGE,
        })
        .collect();
    let mut state = ClientState::default();
    state.apply_ready(Ready {
        session_id: "session".to_owned(),
        user: CurrentUser {
            id: ME,
            username: "paul".to_owned(),
            display_name: "Paul".to_owned(),
            email: "paul@example.org".to_owned(),
            is_instance_administrator: false,
            avatar: None,
            banner: None,
            status: ChosenStatus::Online,
            custom_status: None,
            friend_code: None,
        },
        guilds: vec![rust, books],
        read_states,
        online_user_ids: Vec::new(),
        direct_message_channels: conversations,
        channel_unreads: unreads
            .iter()
            .map(|(channel_id, mention_count)| ChannelUnread {
                channel_id: *channel_id,
                last_message_id: Snowflake::from_raw(NEWEST_MESSAGE.raw() + 1),
                mention_count: *mention_count,
            })
            .collect(),
        presences: vec![Presence {
            user_id: ANA,
            status: PresenceStatus::Online,
            custom_status: Some("Reading".to_owned()),
        }],
        relationships: Vec::new(),
        may_create_guilds: true,
        voice_states: Vec::new(),
        voice_enabled: false,
        maximum_attachment_bytes: None,
    });
    state.set_members(
        GUILD_RUST,
        vec![
            member(person(ANA, "ana", "Ana"), vec![MODERATORS]),
            member(person(ZOE, "zoe", "Zoé"), Vec::new()),
            member(person(ME, "paul", "Paul"), Vec::new()),
        ],
    );
    state
}

pub fn sample_state() -> ClientState {
    sample_state_with_unreads(&[])
}

fn member(user: PublicUser, role_ids: Vec<Snowflake>) -> Member {
    Member {
        user,
        role_ids,
        joined_at_unix_milliseconds: 0,
    }
}

/// A window's root drawn once per frame around `child`: what an event costs, counted in frames.
pub struct FrameCountingRoot {
    pub child: AnyView,
    pub frames: Rc<Cell<u32>>,
}

impl Render for FrameCountingRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.frames.set(self.frames.get() + 1);
        div().size_full().flex().child(self.child.clone())
    }
}

/// `open_shell` inside a [`FrameCountingRoot`]; the count starts once the shell is drawn.
pub fn open_shell_counting_frames(
    cx: &mut TestAppContext,
    state: ClientState,
) -> (
    Entity<crate::session::Session>,
    Rc<Cell<u32>>,
    &mut VisualTestContext,
) {
    let session = session_for_shell(cx, state);
    let frames = Rc::new(Cell::new(0));
    let (shell_session, counted) = (session.clone(), Rc::clone(&frames));
    let (_, cx) = cx.add_window_view(|window, cx| FrameCountingRoot {
        child: cx
            .new(|cx| crate::shell::DuskShell::new(shell_session, window, cx))
            .into(),
        frames: counted,
    });
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    frames.set(0);
    (session, frames, cx)
}

/// A shell over `state`, in an active window of `width` × 800, with the app's keys bound.
pub fn open_shell(
    cx: &mut TestAppContext,
    state: ClientState,
    width: f32,
) -> (Entity<crate::shell::DuskShell>, &mut VisualTestContext) {
    let session = session_for_shell(cx, state);
    let (shell, cx) =
        cx.add_window_view(|window, cx| crate::shell::DuskShell::new(session, window, cx));
    cx.simulate_resize(size(px(width), px(800.0)));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (shell, cx)
}

/// The app's globals and keys, and a session holding `state`.
fn session_for_shell(
    cx: &mut TestAppContext,
    state: ClientState,
) -> Entity<crate::session::Session> {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(crate::strings_pending_i18n::ActiveStrings(
            &crate::strings_pending_i18n::ENGLISH,
        ));
        cx.set_global(crate::theme::ActiveTheme::for_appearance(
            WindowAppearance::Dark,
        ));
        crate::components::bind_tab_navigation(cx);
        crate::shell::bind_shell_keys(cx);
    });
    let session = cx.new(|_| crate::session::Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| *session.state_mut() = state);
    session
}

/// The sample world where the reader also administers Rust Lyon.
pub fn sample_state_administering_rust() -> ClientState {
    let mut state = sample_state();
    let mut guild = state
        .guild(GUILD_RUST)
        .cloned()
        .expect("the sample world has Rust Lyon");
    guild.my_permissions |= Permissions::ADMINISTRATOR;
    state.apply_event(
        mellow_protocol::GatewayEvent::GuildCreate(Box::new(guild)),
        0,
    );
    state
}
