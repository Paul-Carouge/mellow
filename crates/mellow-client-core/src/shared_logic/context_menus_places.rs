//! Which items the menus of places show (`web/src/components/context-menus.ts`, DESIGN.md §6.33,
//! docs/design/web-context-menus.md): a server, a channel row, a category header and the phone
//! chat header's "More options", with the notification items they share. Only actions the reader
//! may do appear; items come in sections separated by a line, danger items always last. The
//! client decides which actions it can carry out and says so through the facts.

use mellow_protocol::{Guild, Permissions, Snowflake};

/// One line of a menu: an action, or the line between two sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuLine<Action> {
    Action(Action),
    Separator,
}

/// The non-empty sections, with a separator between each (`joinSections`).
fn join_sections<Action: Copy>(sections: &[&[Option<Action>]]) -> Vec<MenuLine<Action>> {
    let mut lines = Vec::new();
    for section in sections {
        let actions: Vec<Action> = section.iter().flatten().copied().collect();
        if actions.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(MenuLine::Separator);
        }
        lines.extend(actions.into_iter().map(MenuLine::Action));
    }
    lines
}

// ---- Notifications (shared by servers, channels and conversations) ----

/// "Mute…" or "Unmute", then "Notification settings…" (docs/design/web-notifications.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationMenuAction {
    Mute,
    Unmute,
    NotificationSettings,
}

fn notification_section(is_muted: bool) -> [Option<NotificationMenuAction>; 2] {
    [
        Some(if is_muted {
            NotificationMenuAction::Unmute
        } else {
            NotificationMenuAction::Mute
        }),
        Some(NotificationMenuAction::NotificationSettings),
    ]
}

// ---- A channel row ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMenuAction {
    JoinVoice,
    LeaveVoice,
    EditChannel,
    MoveUp,
    MoveDown,
    MoveToCategory,
    CopyChannelId,
    DeleteChannel,
    Notification(NotificationMenuAction),
}

/// A voice channel's first item: Join, or Leave when the reader is in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceMenuAction {
    Join,
    Leave,
}

// Independent facts of one row, as the web's interface: no state machine hides here.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelMenuFacts {
    /// The reader sees at least one page of its settings.
    pub can_edit_channel: bool,
    /// Muted itself (a muted server's channels are quiet too, but unmute from the server).
    pub is_muted: bool,
    /// `MANAGE_CHANNELS` on the server, the row not first or last of its section, no move in flight.
    pub can_move_up: bool,
    pub can_move_down: bool,
    /// `MANAGE_CHANNELS` in the channel too, and somewhere else to go.
    pub can_move_to_category: bool,
    /// `MANAGE_CHANNELS` in the channel.
    pub can_delete: bool,
    /// A voice channel. Voice channels hold no messages to mute.
    pub voice: Option<VoiceMenuAction>,
}

/// Move up and Move down are the single-pointer and keyboard path for dragging (WCAG 2.2 2.5.7).
#[must_use]
pub fn channel_menu_actions(facts: ChannelMenuFacts) -> Vec<MenuLine<ChannelMenuAction>> {
    use ChannelMenuAction as Action;
    let notifications: Vec<Option<Action>> = if facts.voice.is_none() {
        notification_section(facts.is_muted)
            .into_iter()
            .map(|action| action.map(Action::Notification))
            .collect()
    } else {
        Vec::new()
    };
    join_sections(&[
        &[
            (facts.voice == Some(VoiceMenuAction::Join)).then_some(Action::JoinVoice),
            (facts.voice == Some(VoiceMenuAction::Leave)).then_some(Action::LeaveVoice),
        ],
        &[facts.can_edit_channel.then_some(Action::EditChannel)],
        &notifications,
        &[
            facts.can_move_up.then_some(Action::MoveUp),
            facts.can_move_down.then_some(Action::MoveDown),
            facts.can_move_to_category.then_some(Action::MoveToCategory),
        ],
        &[Some(Action::CopyChannelId)],
        &[facts.can_delete.then_some(Action::DeleteChannel)],
    ])
}

// ---- A category header ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryMenuAction {
    RenameCategory,
    MoveUp,
    MoveDown,
    CopyCategoryId,
    DeleteCategory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategoryMenuFacts {
    /// Rename and delete: `MANAGE_CHANNELS` in the category.
    pub can_manage: bool,
    pub can_move_up: bool,
    pub can_move_down: bool,
}

/// Categories have no settings page: their menu renames them (DESIGN.md §5.3).
#[must_use]
pub fn category_menu_actions(facts: CategoryMenuFacts) -> Vec<MenuLine<CategoryMenuAction>> {
    use CategoryMenuAction as Action;
    join_sections(&[
        &[facts.can_manage.then_some(Action::RenameCategory)],
        &[
            facts.can_move_up.then_some(Action::MoveUp),
            facts.can_move_down.then_some(Action::MoveDown),
        ],
        &[Some(Action::CopyCategoryId)],
        &[facts.can_manage.then_some(Action::DeleteCategory)],
    ])
}

// ---- A server (its icon in the strip, or the header's menu) ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerMenuAction {
    InvitePeople,
    ServerSettings,
    CreateChannel,
    CreateCategory,
    CopyServerId,
    LeaveServer,
    Notification(NotificationMenuAction),
}

#[allow(clippy::struct_excessive_bools)] // Independent facts, as the web's interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerMenuFacts {
    pub can_invite_people: bool,
    pub can_open_settings: bool,
    pub can_create_channels: bool,
    /// Everyone but the owner.
    pub can_leave: bool,
    pub is_muted: bool,
}

/// Holds any of `wanted` (the owner and administrators hold them all), as `hasPermission` and
/// `holds` read a server's permission snapshot.
fn holds_any(held: Permissions, wanted: Permissions) -> bool {
    held.contains(Permissions::ADMINISTRATOR) || held.intersects(wanted)
}

/// What the reader may do with `guild`, from its permission snapshot (`serverMenuFactsOf`).
#[must_use]
pub fn server_menu_facts_of(
    guild: &Guild,
    user_id: Option<Snowflake>,
    is_muted: bool,
) -> ServerMenuFacts {
    ServerMenuFacts {
        can_invite_people: holds_any(guild.my_permissions, Permissions::CREATE_INVITE),
        can_open_settings: super::server_settings::can_open_server_settings(guild, user_id),
        can_create_channels: holds_any(guild.my_permissions, Permissions::MANAGE_CHANNELS),
        can_leave: user_id != Some(guild.owner_id),
        is_muted,
    }
}

/// Where a server's menu opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerMenuPlace {
    /// The channel list's header, on the open server.
    Header,
    /// Its icon in the server strip, on any server, without opening it.
    Strip,
}

/// The header's menu works on the open server; the strip's icon menu works on any server and
/// copies its ID, as Discord's does.
#[must_use]
pub fn server_menu_actions(
    facts: ServerMenuFacts,
    place: ServerMenuPlace,
) -> Vec<MenuLine<ServerMenuAction>> {
    use ServerMenuAction as Action;
    let notifications: Vec<Option<Action>> = notification_section(facts.is_muted)
        .into_iter()
        .map(|action| action.map(Action::Notification))
        .collect();
    let leave = [facts.can_leave.then_some(Action::LeaveServer)];
    match place {
        ServerMenuPlace::Header => join_sections(&[
            &[
                facts.can_invite_people.then_some(Action::InvitePeople),
                facts.can_open_settings.then_some(Action::ServerSettings),
                facts.can_create_channels.then_some(Action::CreateChannel),
                facts.can_create_channels.then_some(Action::CreateCategory),
            ],
            &notifications,
            &leave,
        ]),
        ServerMenuPlace::Strip => join_sections(&[
            &[
                facts.can_invite_people.then_some(Action::InvitePeople),
                facts.can_open_settings.then_some(Action::ServerSettings),
            ],
            &notifications,
            &[Some(Action::CopyServerId)],
            &leave,
        ]),
    }
}

// ---- The phone chat header's "More options" ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatHeaderMenuAction {
    GoTo,
    PinnedMessages,
    AddFriendsToGroup,
    ChannelSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatHeaderMenuFacts {
    pub is_group: bool,
    /// A channel is open and the reader may change at least one of its settings.
    pub can_edit_channel: bool,
}

/// The header keys a narrow window folds away (DESIGN.md 4.5), each only where the wide header
/// shows it.
#[must_use]
pub fn chat_header_menu_actions(facts: ChatHeaderMenuFacts) -> Vec<MenuLine<ChatHeaderMenuAction>> {
    use ChatHeaderMenuAction as Action;
    join_sections(&[&[
        Some(Action::GoTo),
        Some(Action::PinnedMessages),
        facts.is_group.then_some(Action::AddFriendsToGroup),
        facts.can_edit_channel.then_some(Action::ChannelSettings),
    ]])
}

#[cfg(test)]
mod tests {
    use super::*;

    use MenuLine::Separator;

    fn line<Action>(action: Action) -> MenuLine<Action> {
        MenuLine::Action(action)
    }

    const MUTE: ChannelMenuAction = ChannelMenuAction::Notification(NotificationMenuAction::Mute);
    const UNMUTE: ChannelMenuAction =
        ChannelMenuAction::Notification(NotificationMenuAction::Unmute);
    const SETTINGS: ChannelMenuAction =
        ChannelMenuAction::Notification(NotificationMenuAction::NotificationSettings);

    const READER: ChannelMenuFacts = ChannelMenuFacts {
        can_edit_channel: false,
        is_muted: false,
        can_move_up: false,
        can_move_down: false,
        can_move_to_category: false,
        can_delete: false,
        voice: None,
    };

    #[test]
    fn a_channel_offers_edit_only_with_a_settings_page_to_show() {
        use ChannelMenuAction::{CopyChannelId, EditChannel};
        assert_eq!(
            channel_menu_actions(ChannelMenuFacts {
                can_edit_channel: true,
                ..READER
            }),
            vec![
                line(EditChannel),
                Separator,
                line(MUTE),
                line(SETTINGS),
                Separator,
                line(CopyChannelId),
            ]
        );
        assert_eq!(
            channel_menu_actions(READER),
            vec![line(MUTE), line(SETTINGS), Separator, line(CopyChannelId)]
        );
    }

    #[test]
    fn a_voice_channel_puts_join_or_leave_first_and_has_nothing_to_mute() {
        use ChannelMenuAction::{CopyChannelId, DeleteChannel, EditChannel, JoinVoice, LeaveVoice};
        assert_eq!(
            channel_menu_actions(ChannelMenuFacts {
                voice: Some(VoiceMenuAction::Join),
                ..READER
            }),
            vec![line(JoinVoice), Separator, line(CopyChannelId)]
        );
        assert_eq!(
            channel_menu_actions(ChannelMenuFacts {
                can_edit_channel: true,
                can_delete: true,
                voice: Some(VoiceMenuAction::Leave),
                ..READER
            }),
            vec![
                line(LeaveVoice),
                Separator,
                line(EditChannel),
                Separator,
                line(CopyChannelId),
                Separator,
                line(DeleteChannel),
            ]
        );
    }

    #[test]
    fn a_muted_channel_offers_unmute() {
        assert_eq!(
            channel_menu_actions(ChannelMenuFacts {
                is_muted: true,
                ..READER
            }),
            vec![
                line(UNMUTE),
                line(SETTINGS),
                Separator,
                line(ChannelMenuAction::CopyChannelId),
            ]
        );
    }

    #[test]
    fn the_moves_a_manager_may_make_sit_between_edit_and_the_id() {
        use ChannelMenuAction::{
            CopyChannelId, DeleteChannel, EditChannel, MoveDown, MoveToCategory,
        };
        assert_eq!(
            channel_menu_actions(ChannelMenuFacts {
                can_edit_channel: true,
                can_move_down: true,
                can_move_to_category: true,
                can_delete: true,
                ..READER
            }),
            vec![
                line(EditChannel),
                Separator,
                line(MUTE),
                line(SETTINGS),
                Separator,
                line(MoveDown),
                line(MoveToCategory),
                Separator,
                line(CopyChannelId),
                Separator,
                line(DeleteChannel),
            ]
        );
    }

    #[test]
    fn a_category_renames_moves_copies_its_id_and_deletes_last() {
        use CategoryMenuAction::{
            CopyCategoryId, DeleteCategory, MoveDown, MoveUp, RenameCategory,
        };
        assert_eq!(
            category_menu_actions(CategoryMenuFacts {
                can_manage: true,
                can_move_up: true,
                can_move_down: true,
            }),
            vec![
                line(RenameCategory),
                Separator,
                line(MoveUp),
                line(MoveDown),
                Separator,
                line(CopyCategoryId),
                Separator,
                line(DeleteCategory),
            ]
        );
        assert_eq!(
            category_menu_actions(CategoryMenuFacts {
                can_manage: false,
                can_move_up: false,
                can_move_down: false,
            }),
            vec![line(CopyCategoryId)]
        );
    }

    const EVERYTHING: ServerMenuFacts = ServerMenuFacts {
        can_invite_people: true,
        can_open_settings: true,
        can_create_channels: true,
        can_leave: true,
        is_muted: false,
    };
    const SERVER_MUTE: ServerMenuAction =
        ServerMenuAction::Notification(NotificationMenuAction::Mute);
    const SERVER_UNMUTE: ServerMenuAction =
        ServerMenuAction::Notification(NotificationMenuAction::Unmute);
    const SERVER_SETTINGS: ServerMenuAction =
        ServerMenuAction::Notification(NotificationMenuAction::NotificationSettings);

    #[test]
    fn the_header_menu_creates_channels_and_categories() {
        use ServerMenuAction::{
            CreateCategory, CreateChannel, InvitePeople, LeaveServer, ServerSettings,
        };
        assert_eq!(
            server_menu_actions(EVERYTHING, ServerMenuPlace::Header),
            vec![
                line(InvitePeople),
                line(ServerSettings),
                line(CreateChannel),
                line(CreateCategory),
                Separator,
                line(SERVER_MUTE),
                line(SERVER_SETTINGS),
                Separator,
                line(LeaveServer),
            ]
        );
    }

    #[test]
    fn the_strip_menu_copies_the_id_and_keeps_leave_last() {
        use ServerMenuAction::{CopyServerId, InvitePeople, LeaveServer, ServerSettings};
        assert_eq!(
            server_menu_actions(EVERYTHING, ServerMenuPlace::Strip),
            vec![
                line(InvitePeople),
                line(ServerSettings),
                Separator,
                line(SERVER_MUTE),
                line(SERVER_SETTINGS),
                Separator,
                line(CopyServerId),
                Separator,
                line(LeaveServer),
            ]
        );
    }

    #[test]
    fn a_plain_owner_gets_only_notifications_and_the_id() {
        let owner = ServerMenuFacts {
            can_invite_people: false,
            can_open_settings: false,
            can_create_channels: false,
            can_leave: false,
            is_muted: true,
        };
        assert_eq!(
            server_menu_actions(owner, ServerMenuPlace::Strip),
            vec![
                line(SERVER_UNMUTE),
                line(SERVER_SETTINGS),
                Separator,
                line(ServerMenuAction::CopyServerId),
            ]
        );
        assert_eq!(
            server_menu_actions(owner, ServerMenuPlace::Header),
            vec![line(SERVER_UNMUTE), line(SERVER_SETTINGS)]
        );
    }

    fn server(owner: u64, permissions: Permissions) -> Guild {
        Guild {
            id: Snowflake::from_raw(10),
            name: "Rust Lyon".to_owned(),
            owner_id: Snowflake::from_raw(owner),
            icon: None,
            my_permissions: permissions,
            my_role_ids: Vec::new(),
            roles: Vec::new(),
            channels: Vec::new(),
            emojis: Vec::new(),
            sounds: Vec::new(),
            my_join_sound_id: None,
        }
    }

    #[test]
    fn server_facts_come_from_the_permission_snapshot_and_the_owner() {
        let me = Some(Snowflake::from_raw(1));
        let plain = server_menu_facts_of(&server(2, Permissions::VIEW_CHANNEL), me, false);
        assert_eq!(
            plain,
            ServerMenuFacts {
                can_invite_people: false,
                can_open_settings: false,
                can_create_channels: false,
                can_leave: true,
                is_muted: false,
            }
        );
        let owner = server_menu_facts_of(&server(1, Permissions::VIEW_CHANNEL), me, true);
        assert!(owner.can_open_settings && !owner.can_leave && owner.is_muted);
        let administrator = server_menu_facts_of(&server(2, Permissions::ADMINISTRATOR), me, false);
        assert!(
            administrator.can_invite_people
                && administrator.can_open_settings
                && administrator.can_create_channels
        );
        let moderator = server_menu_facts_of(&server(2, Permissions::BAN_MEMBERS), me, false);
        assert!(moderator.can_open_settings && !moderator.can_create_channels);
    }

    #[test]
    fn the_chat_header_folds_go_to_pins_and_what_the_place_allows() {
        use ChatHeaderMenuAction::{AddFriendsToGroup, ChannelSettings, GoTo, PinnedMessages};
        let plain = ChatHeaderMenuFacts {
            is_group: false,
            can_edit_channel: false,
        };
        assert_eq!(
            chat_header_menu_actions(plain),
            vec![line(GoTo), line(PinnedMessages)]
        );
        assert_eq!(
            chat_header_menu_actions(ChatHeaderMenuFacts {
                can_edit_channel: true,
                ..plain
            }),
            vec![line(GoTo), line(PinnedMessages), line(ChannelSettings)]
        );
        assert_eq!(
            chat_header_menu_actions(ChatHeaderMenuFacts {
                is_group: true,
                ..plain
            }),
            vec![line(GoTo), line(PinnedMessages), line(AddFriendsToGroup)]
        );
    }
}
