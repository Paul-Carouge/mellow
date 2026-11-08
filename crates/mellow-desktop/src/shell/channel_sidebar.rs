//! The navigator card (`Navigator.svelte`, ADR-0022 §1) and its channel list
//! (`ChannelSidebar.svelte`, `ConversationList.svelte`, DESIGN.md §5.3, §5.6): the server's name,
//! then its channels by category, or the conversations at Home. Collapsed categories still show
//! the open and unread channels and the voice channels with people in them, as on Discord.

use gpui::{
    AnyElement, Context, ElementId, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Role, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::channel_order::channel_sections;
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::group_conversations::participant_count;
use mellow_client_core::state::ClientState;
use mellow_protocol::{Channel, ChannelKind, DirectMessageChannel, Permissions, Snowflake};

use super::place_menus::{MenuRow, PlaceMenuTarget, is_target_muted, with_place_menu};
use super::regions::{Region, RovingTabStop};
use super::server_rail::name_with_unread_state;
use super::{DuskShell, profile_picture};
use crate::components::{
    Avatar, AvatarPerson, Button, ButtonVariant, GroupAvatar, IconButton, MentionBadge, icon,
    inset_focus_ring, is_focus_visible, remembered_focus_handle, remembered_hover, with_type,
};
use crate::home::{conversation_display_name, shown_presence};
use crate::i18n::strings;
use crate::session::now_unix_milliseconds;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CATEGORY_ADD_ICON_SIZE, COMPONENT_CATEGORY_CHEVRON_SIZE, COMPONENT_CATEGORY_HEIGHT,
    COMPONENT_CATEGORY_MARGIN_TOP, COMPONENT_CATEGORY_PADDING_INLINE, COMPONENT_CHANNEL_ROW_GAP,
    COMPONENT_CHANNEL_ROW_HEIGHT, COMPONENT_CHANNEL_ROW_ICON_GAP, COMPONENT_CHANNEL_ROW_ICON_SIZE,
    COMPONENT_CHANNEL_ROW_MARGIN_INLINE, COMPONENT_CHANNEL_ROW_MUTED_ICON_SIZE,
    COMPONENT_CHANNEL_ROW_PADDING_INLINE, COMPONENT_CHANNEL_ROW_RADIUS,
    COMPONENT_CHANNEL_ROW_UNREAD_PILL_HEIGHT, COMPONENT_CHANNEL_ROW_UNREAD_PILL_OFFSET,
    COMPONENT_CHANNEL_ROW_UNREAD_PILL_WIDTH, COMPONENT_MEMBER_ROW_AVATAR_SIZE,
    COMPONENT_MEMBER_ROW_HEIGHT, COMPONENT_NAVIGATOR_RADIUS, COMPONENT_NAVIGATOR_STRIP_WIDTH,
    COMPONENT_SIDEBAR_HEADER_HEIGHT, COMPONENT_VOICE_PARTICIPANT_AVATAR_SIZE,
    COMPONENT_VOICE_PARTICIPANT_GAP, COMPONENT_VOICE_PARTICIPANT_INDENT,
    COMPONENT_VOICE_PARTICIPANT_ROW_HEIGHT, SPACE_2XS, SPACE_MD, SPACE_XS, TYPE_BODY, TYPE_BODY_SM,
    TYPE_CATEGORY, TYPE_CHANNEL_NAME, TYPE_HEADING_MD,
};
use crate::theme::{color, length, theme};

/// Whether the reader may send in a channel; a read-only one shows a lock (DESIGN.md §5.3).
pub(super) fn can_send_in(state: &ClientState, channel_id: Snowflake) -> bool {
    state
        .my_channel_permissions(channel_id)
        .is_none_or(|permissions| permissions.contains(Permissions::SEND_MESSAGES))
}

/// What a sidebar row shows and does.
struct SidebarRow {
    id: ElementId,
    /// The accessible name, with its unread and mention state.
    label: String,
    name: SharedString,
    /// A group's "3 members" under its name.
    second_line: Option<SharedString>,
    leading: AnyElement,
    height: f32,
    is_selected: bool,
    has_unread: bool,
    mention_count: u32,
    channel_id: Option<Snowflake>,
    /// Greyed with a bell-slash, never unread (DESIGN.md §5.3).
    is_muted: bool,
    /// The place whose menu a right-click, the Menu key or Shift+F10 opens.
    menu: Option<PlaceMenuTarget>,
}

impl DuskShell {
    /// The navigator card: strip and channel list side by side, the user card across its foot.
    /// In the drawer the card meets the window's leading edge: only its trailing corners round.
    pub(super) fn render_navigator(
        &mut self,
        channels_width: f32,
        is_in_drawer: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tokens = theme(cx);
        let strings = strings(cx);
        let rail = self.render_server_rail(window, cx);
        let sidebar = self.render_channel_sidebar(channels_width, window, cx);
        let user_card = self.render_user_card(window, cx);
        div()
            .debug_selector(|| "navigator".to_owned())
            .id("navigator")
            .role(Role::Navigation)
            .aria_label(strings.shell().servers_and_channels())
            .flex_none()
            .w(length(COMPONENT_NAVIGATOR_STRIP_WIDTH + channels_width))
            .h_full()
            .flex()
            .flex_col()
            .bg(color(tokens.component_navigator_background))
            .map(|card| {
                if is_in_drawer {
                    card.rounded_r(length(COMPONENT_NAVIGATOR_RADIUS))
                } else {
                    card.rounded(length(COMPONENT_NAVIGATOR_RADIUS))
                }
            })
            .overflow_hidden()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .child(rail)
                    .child(sidebar),
            )
            .child(user_card)
    }

    fn render_channel_sidebar(
        &mut self,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tokens = theme(cx);
        let strings = strings(cx);
        let state = self.session.read(cx).state();
        let guild = self
            .navigation
            .selected_guild_id()
            .and_then(|guild_id| state.guild(guild_id))
            .cloned();
        let (heading, list_label) = match &guild {
            Some(guild) => (
                guild.name.clone(),
                strings.sidebar().channels_in(&guild.name),
            ),
            None => (
                strings.sidebar().direct_messages().to_owned(),
                strings.sidebar().direct_messages().to_owned(),
            ),
        };
        let is_home = guild.is_none();
        let has_selected_row = self.navigation.selected_channel_id().is_some()
            || (guild.is_none() && self.navigation.place().is_some());
        let mut tab_stop = self
            .regions
            .channel_list
            .tab_stop_chooser(has_selected_row, window);
        let rows = match guild {
            Some(guild) => self.guild_rows(&guild.channels, guild.id, &mut tab_stop, window, cx),
            None => self.conversation_rows(&mut tab_stop, window, cx),
        };
        let entity = cx.entity();
        self.regions.channel_list.keep_rows(tab_stop, cx);

        div()
            .debug_selector(|| "channel-column".to_owned())
            .flex_none()
            .w(length(width))
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                with_type(div(), TYPE_HEADING_MD)
                    .id("sidebar-heading")
                    .role(Role::Heading)
                    .aria_level(2)
                    .flex_none()
                    .h(length(COMPONENT_SIDEBAR_HEADER_HEIGHT))
                    .mt(length(SPACE_XS))
                    .mr(length(SPACE_XS))
                    .px(length(SPACE_XS))
                    .flex()
                    .items_center()
                    .gap(length(SPACE_2XS))
                    .text_color(color(tokens.component_sidebar_header_text))
                    .child(div().flex_1().min_w_0().truncate().child(heading))
                    // Home: Discord's "+" beside "Direct messages" starts a conversation or a
                    // group (ADR-0026).
                    .when(is_home, |header| {
                        header.child(
                            IconButton::new(
                                "new-message",
                                IconName::Plus,
                                strings.groups().new_message(),
                            )
                            .sizes(COMPONENT_CATEGORY_HEIGHT, COMPONENT_CATEGORY_ADD_ICON_SIZE)
                            .on_click(move |window, cx| {
                                entity.update(cx, |shell, cx| shell.open_new_message(window, cx));
                            }),
                        )
                    }),
            )
            .child(
                // The negative margin lets the unread pills sit in the strip's margin, unclipped.
                div()
                    .id("channel-list")
                    .role(Role::Navigation)
                    .aria_label(list_label)
                    .track_focus(&self.regions.channel_list.focus)
                    .track_scroll(&self.regions.channel_list.scroll)
                    .on_key_down(cx.listener(|shell, event: &KeyDownEvent, window, cx| {
                        shell.move_in_list(Region::ChannelList, event, window, cx);
                    }))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .ml(-length(COMPONENT_CHANNEL_ROW_UNREAD_PILL_OFFSET))
                    .pl(length(COMPONENT_CHANNEL_ROW_UNREAD_PILL_OFFSET))
                    .pb(length(SPACE_XS))
                    .children(rows),
            )
    }

    #[allow(clippy::too_many_lines)] // One list, in the order it is drawn.
    fn guild_rows(
        &mut self,
        channels: &[Channel],
        guild_id: Snowflake,
        tab_stop: &mut RovingTabStop,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let strings = strings(cx);
        let sections = channel_sections(channels);
        if sections.iter().all(|section| section.channels.is_empty()) {
            let can_create_channels = self
                .session
                .read(cx)
                .state()
                .guild(guild_id)
                .is_some_and(|guild| guild.my_permissions.contains(Permissions::MANAGE_CHANNELS));
            // shortcut: the web's "Create a channel" button opens its dialog, which comes with
            // server management (M3 step 6).
            let note = if can_create_channels {
                strings.sidebar().no_channels_yet()
            } else {
                strings.sidebar().no_visible_channels_yet()
            };
            return vec![empty_note(note, cx).into_any_element()];
        }
        let selected_channel_id = self.navigation.selected_channel_id();
        let settings = self.notification_settings(cx);
        let now = now_unix_milliseconds();
        let mut rows = Vec::new();
        for section in sections {
            let is_collapsed = section
                .category
                .as_ref()
                .is_some_and(|category| self.collapsed_category_ids.contains(&category.id));
            if let Some(category) = &section.category {
                rows.push(Self::category_row(
                    category,
                    is_collapsed,
                    (tab_stop, rows.len()),
                    window,
                    cx,
                ));
            }
            for channel in &section.channels {
                let state = self.session.read(cx).state();
                if channel.kind == ChannelKind::Voice {
                    let participant_ids: Vec<Snowflake> = state
                        .voice_participants(channel.id)
                        .map(|voice_state| voice_state.user_id)
                        .collect();
                    if !is_collapsed || !participant_ids.is_empty() {
                        rows.push(self.voice_channel_row(
                            channel,
                            guild_id,
                            &participant_ids,
                            window,
                            cx,
                        ));
                    }
                    continue;
                }
                let is_selected = selected_channel_id == Some(channel.id);
                let is_muted = is_target_muted(&settings, channel.id, now);
                let has_unread = !is_selected && !is_muted && state.has_unread(channel.id);
                if is_collapsed && !is_selected && !has_unread {
                    continue;
                }
                let mention_count = if is_selected {
                    0
                } else {
                    state.mention_count(channel.id)
                };
                let can_send = can_send_in(state, channel.id);
                let name_with_mute = if is_muted {
                    strings.notifications().with_muted(&channel.name)
                } else {
                    channel.name.clone()
                };
                let name_for_label = if can_send {
                    name_with_mute
                } else {
                    format!("{name_with_mute}, {}", strings.common().read_only())
                };
                rows.push(Self::sidebar_row(
                    SidebarRow {
                        id: ElementId::from(("channel-row", channel.id.raw())),
                        label: name_with_unread_state(
                            strings,
                            &name_for_label,
                            has_unread,
                            mention_count,
                        ),
                        name: channel.name.clone().into(),
                        second_line: None,
                        leading: icon(
                            if can_send {
                                IconName::Hash
                            } else {
                                IconName::Lock
                            },
                            COMPONENT_CHANNEL_ROW_ICON_SIZE,
                            row_ink(
                                RowLook {
                                    is_selected,
                                    has_unread,
                                    is_muted,
                                },
                                false,
                                cx,
                            ),
                        )
                        .into_any_element(),
                        height: COMPONENT_CHANNEL_ROW_HEIGHT,
                        is_selected,
                        has_unread,
                        mention_count,
                        channel_id: Some(channel.id),
                        is_muted,
                        menu: Some(PlaceMenuTarget::Channel(channel.id)),
                    },
                    (tab_stop, rows.len()),
                    window,
                    cx,
                ));
            }
        }
        rows
    }

    fn category_row(
        category: &Channel,
        is_collapsed: bool,
        (tab_stop, child_index): (&mut RovingTabStop, usize),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let id = ElementId::from(("category-row", category.id.raw()));
        let focus_handle = tab_stop.row(
            &id,
            remembered_focus_handle(&id, window, cx),
            false,
            child_index,
        );
        let hover = remembered_hover(&id, window, cx);
        let ink = if *hover.read(cx) {
            tokens.component_category_text_hover
        } else {
            tokens.component_category_text
        };
        let category_id = category.id;
        let is_ring_shown = is_focus_visible(&focus_handle, window);
        let header = with_place_menu(
            with_type(div(), TYPE_CATEGORY).id(id),
            MenuRow {
                target: PlaceMenuTarget::Category(category_id),
                region: Region::ChannelList,
                child_index,
            },
            focus_handle.clone(),
            cx,
        );
        div()
            .mt(length(COMPONENT_CATEGORY_MARGIN_TOP))
            .mr(length(COMPONENT_CHANNEL_ROW_MARGIN_INLINE))
            .child(
                header
                    .role(Role::Button)
                    .aria_label(SharedString::from(category.name.clone()))
                    .aria_expanded(!is_collapsed)
                    .track_focus(&focus_handle)
                    .relative()
                    .flex()
                    .items_center()
                    .gap(length(SPACE_2XS))
                    .h(length(COMPONENT_CATEGORY_HEIGHT))
                    .px(length(COMPONENT_CATEGORY_PADDING_INLINE))
                    .rounded(length(crate::theme::tokens::RADIUS_XS))
                    .text_color(color(ink))
                    .cursor_pointer()
                    .on_hover(move |is_hovered, _, cx| {
                        hover.update(cx, |remembered, cx| {
                            *remembered = *is_hovered;
                            cx.notify();
                        });
                    })
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.toggle_category(category_id, cx);
                    }))
                    .on_key_down(cx.listener(move |shell, event: &KeyDownEvent, _, cx| {
                        let key = event.keystroke.key.as_str();
                        let is_collapsed = shell.collapsed_category_ids.contains(&category_id);
                        if (key == "left" && !is_collapsed) || (key == "right" && is_collapsed) {
                            cx.stop_propagation();
                            shell.toggle_category(category_id, cx);
                        }
                    }))
                    .child(icon(
                        if is_collapsed {
                            IconName::CaretRight
                        } else {
                            IconName::CaretDown
                        },
                        COMPONENT_CATEGORY_CHEVRON_SIZE,
                        color(ink),
                    ))
                    .child(div().min_w_0().truncate().child(category.name.clone()))
                    .when(is_ring_shown, |row| {
                        row.child(inset_focus_ring(crate::theme::tokens::RADIUS_XS, cx))
                    }),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    /// A voice channel and the people in it. Voice channels are joined, never opened.
    /// shortcut: joining from the row comes with voice (M3 step 7); the row is read, not pressed.
    fn voice_channel_row(
        &mut self,
        channel: &Channel,
        guild_id: Snowflake,
        participant_ids: &[Snowflake],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let strings = strings(cx);
        let state = self.session.read(cx).state();
        let can_connect = state
            .my_channel_permissions(channel.id)
            .is_none_or(|permissions| permissions.contains(Permissions::CONNECT));
        let people = i64::try_from(participant_ids.len()).unwrap_or(i64::MAX);
        let limit = channel.user_limit.map(i64::from);
        let label = strings
            .voice()
            .row_label(&channel.name, people, limit, false, can_connect);
        let ink = color(tokens.component_channel_row_text_read);
        let participants: Vec<AnyElement> = participant_ids
            .iter()
            .map(|user_id| {
                let person = state
                    .members(guild_id)
                    .iter()
                    .map(|member| &member.user)
                    .find(|user| user.id == *user_id)
                    .cloned();
                let (name, avatar) = if let Some(user) = person {
                    let picture = user.avatar.map(|image| {
                        profile_picture(image, COMPONENT_VOICE_PARTICIPANT_AVATAR_SIZE, window)
                    });
                    (
                        display_name(&user).to_owned(),
                        AvatarPerson::from_user(&user, picture),
                    )
                } else {
                    // The member list is still on its way.
                    let someone = strings.voice().someone();
                    let placeholder = mellow_protocol::PublicUser {
                        id: *user_id,
                        username: someone.to_owned(),
                        display_name: someone.to_owned(),
                        avatar: None,
                        banner: None,
                    };
                    (
                        someone.to_owned(),
                        AvatarPerson::from_user(&placeholder, None),
                    )
                };
                div()
                    .mr(length(COMPONENT_CHANNEL_ROW_MARGIN_INLINE))
                    .h(length(COMPONENT_VOICE_PARTICIPANT_ROW_HEIGHT))
                    .pl(length(COMPONENT_VOICE_PARTICIPANT_INDENT))
                    .flex()
                    .items_center()
                    .gap(length(COMPONENT_VOICE_PARTICIPANT_GAP))
                    .child(Avatar::new(avatar, COMPONENT_VOICE_PARTICIPANT_AVATAR_SIZE))
                    .child(
                        with_type(div(), TYPE_BODY_SM)
                            .min_w_0()
                            .truncate()
                            .text_color(color(tokens.component_voice_participant_name_text))
                            .child(name),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .mt(length(COMPONENT_CHANNEL_ROW_GAP))
            .child(
                div()
                    .id(ElementId::from(("voice-row", channel.id.raw())))
                    .role(Role::Group)
                    .aria_label(label)
                    .mr(length(COMPONENT_CHANNEL_ROW_MARGIN_INLINE))
                    .h(length(COMPONENT_CHANNEL_ROW_HEIGHT))
                    .px(length(COMPONENT_CHANNEL_ROW_PADDING_INLINE))
                    .flex()
                    .items_center()
                    .gap(length(COMPONENT_CHANNEL_ROW_ICON_GAP))
                    .text_color(ink)
                    .child(icon(
                        if can_connect {
                            IconName::SpeakerHigh
                        } else {
                            IconName::Lock
                        },
                        COMPONENT_CHANNEL_ROW_ICON_SIZE,
                        ink,
                    ))
                    .child(
                        with_type(div(), TYPE_CHANNEL_NAME)
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(channel.name.clone()),
                    )
                    .when_some(limit, |row, limit| {
                        row.child(
                            with_type(div(), TYPE_BODY_SM)
                                .flex_none()
                                .text_color(color(tokens.component_voice_participant_limit_text))
                                .child(format!("{people}/{limit}")),
                        )
                    }),
            )
            .children(participants)
            .into_any_element()
    }

    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    /// Home: Friends, then the conversations, most recent first.
    fn conversation_rows(
        &mut self,
        tab_stop: &mut RovingTabStop,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let tokens = theme(cx);
        let strings = strings(cx);
        let selected = self.navigation.selected_channel_id();
        let is_friends_open = self.navigation.place().is_some() && selected.is_none();
        let incoming = self.session.read(cx).state().incoming_request_count();
        let mut rows = vec![Self::sidebar_row(
            SidebarRow {
                id: ElementId::from("friends-row"),
                label: strings
                    .sidebar()
                    .friends_with_requests(i64::try_from(incoming).unwrap_or(i64::MAX)),
                name: strings.sidebar().friends().into(),
                second_line: None,
                leading: div()
                    .flex_none()
                    .size(length(COMPONENT_MEMBER_ROW_AVATAR_SIZE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(
                        IconName::Users,
                        COMPONENT_CHANNEL_ROW_ICON_SIZE,
                        row_ink(
                            RowLook {
                                is_selected: is_friends_open,
                                has_unread: false,
                                is_muted: false,
                            },
                            false,
                            cx,
                        ),
                    ))
                    .into_any_element(),
                height: COMPONENT_MEMBER_ROW_HEIGHT,
                is_selected: is_friends_open,
                has_unread: false,
                mention_count: u32::try_from(incoming).unwrap_or(u32::MAX),
                channel_id: None,
                is_muted: false,
                menu: None,
            },
            (tab_stop, 0),
            window,
            cx,
        )];
        let state = self.session.read(cx).state();
        let current_user_id = state.current_user().map(|user| user.id);
        let conversations: Vec<DirectMessageChannel> = state.direct_message_channels().to_vec();
        let settings = self.notification_settings(cx);
        let now = now_unix_milliseconds();
        if conversations.is_empty() {
            rows.push(
                div()
                    .mt(length(SPACE_MD))
                    .mr(length(COMPONENT_CHANNEL_ROW_MARGIN_INLINE))
                    .px(length(COMPONENT_CHANNEL_ROW_PADDING_INLINE))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(length(SPACE_XS))
                    .child(
                        with_type(div(), TYPE_CHANNEL_NAME)
                            .text_color(color(tokens.color_text_heading))
                            .child(strings.sidebar().no_conversations_title()),
                    )
                    .child(
                        with_type(div(), TYPE_BODY_SM)
                            .text_color(color(tokens.color_text_secondary))
                            .child(strings.sidebar().no_conversations()),
                    )
                    .child({
                        let entity = cx.entity();
                        Button::new("empty-new-message", strings.groups().new_message())
                            .variant(ButtonVariant::Secondary)
                            .on_click(move |window, cx| {
                                entity.update(cx, |shell, cx| shell.open_new_message(window, cx));
                            })
                    })
                    .into_any_element(),
            );
            return rows;
        }
        for conversation in conversations {
            let state = self.session.read(cx).state();
            let is_selected = selected == Some(conversation.id);
            let is_muted = is_target_muted(&settings, conversation.id, now);
            let has_unread = !is_selected && !is_muted && state.has_unread(conversation.id);
            let mention_count = if is_selected || is_muted {
                0
            } else {
                state.mention_count(conversation.id)
            };
            let name = conversation_display_name(&conversation, current_user_id, strings);
            let others: Vec<AvatarPerson> = conversation
                .recipients
                .iter()
                .filter(|recipient| Some(recipient.id) != current_user_id)
                .map(|recipient| {
                    let picture = recipient.avatar.map(|image| {
                        profile_picture(image, COMPONENT_MEMBER_ROW_AVATAR_SIZE, window)
                    });
                    AvatarPerson::from_user(recipient, picture)
                })
                .collect();
            let is_group = conversation.owner_id.is_some();
            let leading = if is_group {
                GroupAvatar::new(
                    others,
                    COMPONENT_MEMBER_ROW_AVATAR_SIZE,
                    color(tokens.component_navigator_background),
                )
                .into_any_element()
            } else {
                let recipient_id = conversation
                    .recipients
                    .iter()
                    .find(|recipient| Some(recipient.id) != current_user_id)
                    .map(|recipient| recipient.id);
                let presence = recipient_id.map(|user_id| state.presence_of(user_id));
                match others.into_iter().next().zip(presence) {
                    Some((person, presence)) => {
                        Avatar::new(person, COMPONENT_MEMBER_ROW_AVATAR_SIZE)
                            .presence(
                                shown_presence(presence),
                                color(tokens.component_navigator_background),
                            )
                            .into_any_element()
                    }
                    None => div()
                        .size(length(COMPONENT_MEMBER_ROW_AVATAR_SIZE))
                        .into_any_element(),
                }
            };
            // Everyone in a group, you included (`participantCount`).
            let member_count = i64::try_from(participant_count(&conversation, current_user_id))
                .unwrap_or(i64::MAX);
            let name_with_mute = if is_muted {
                strings.notifications().with_muted(&name)
            } else {
                name.clone()
            };
            let label = if is_group {
                strings.groups().row_label(&name_with_mute, member_count)
            } else {
                name_with_mute
            };
            let child_index = rows.len();
            rows.push(Self::sidebar_row(
                SidebarRow {
                    id: ElementId::from(("conversation-row", conversation.id.raw())),
                    label: name_with_unread_state(strings, &label, has_unread, mention_count),
                    name: name.into(),
                    // No presence on a group: its second line counts its people, as on Discord.
                    second_line: is_group
                        .then(|| strings.groups().member_count(member_count).into()),
                    leading,
                    height: COMPONENT_MEMBER_ROW_HEIGHT,
                    is_selected,
                    has_unread,
                    mention_count,
                    channel_id: Some(conversation.id),
                    is_muted,
                    menu: None,
                },
                (tab_stop, child_index),
                window,
                cx,
            ));
        }
        rows
    }

    /// A channel or conversation row (`sidebarRowClasses`): the unread pill at the strip's edge,
    /// the selected wash, unread names brighter, the mention badge last. `channel_id` `None` is
    /// the Friends row, which opens Home.
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn sidebar_row(
        row: SidebarRow,
        (tab_stop, child_index): (&mut RovingTabStop, usize),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let focus_handle = tab_stop.row(
            &row.id,
            remembered_focus_handle(&row.id, window, cx),
            row.is_selected,
            child_index,
        );
        let hover = remembered_hover(&row.id, window, cx);
        let is_hovered = *hover.read(cx);
        let ink = row_ink(
            RowLook {
                is_selected: row.is_selected,
                has_unread: row.has_unread,
                is_muted: row.is_muted,
            },
            is_hovered,
            cx,
        );
        let fill = if row.is_selected {
            Some(tokens.component_channel_row_background_selected)
        } else if is_hovered {
            Some(tokens.component_channel_row_background_hover)
        } else {
            None
        };
        let is_ring_shown = is_focus_visible(&focus_handle, window);
        let channel_id = row.channel_id;
        let menu_focus = focus_handle.clone();
        let button = div()
            .id(row.id)
            .role(Role::Link)
            .aria_label(row.label)
            .when(row.is_selected, |button| button.aria_selected(true))
            .track_focus(&focus_handle);
        let button = match row.menu {
            Some(target) => with_place_menu(
                button,
                MenuRow {
                    target,
                    region: Region::ChannelList,
                    child_index,
                },
                menu_focus,
                cx,
            ),
            None => button,
        };
        div()
            .relative()
            .mt(length(COMPONENT_CHANNEL_ROW_GAP))
            .mr(length(COMPONENT_CHANNEL_ROW_MARGIN_INLINE))
            .when(row.has_unread, |item| {
                item.child(
                    div()
                        .absolute()
                        .left(-length(COMPONENT_CHANNEL_ROW_UNREAD_PILL_OFFSET))
                        .top(length(
                            (row.height - COMPONENT_CHANNEL_ROW_UNREAD_PILL_HEIGHT) / 2.0,
                        ))
                        .w(length(COMPONENT_CHANNEL_ROW_UNREAD_PILL_WIDTH))
                        .h(length(COMPONENT_CHANNEL_ROW_UNREAD_PILL_HEIGHT))
                        .rounded_full()
                        .bg(color(tokens.component_channel_row_unread_pill_color)),
                )
            })
            .child(
                button
                    .relative()
                    .flex()
                    .items_center()
                    .gap(length(COMPONENT_CHANNEL_ROW_ICON_GAP))
                    .h(length(row.height))
                    .px(length(COMPONENT_CHANNEL_ROW_PADDING_INLINE))
                    .rounded(length(COMPONENT_CHANNEL_ROW_RADIUS))
                    .when_some(fill, |button, fill| button.bg(color(fill)))
                    .text_color(ink)
                    .cursor_pointer()
                    .on_hover(move |is_hovered, _, cx| {
                        hover.update(cx, |remembered, cx| {
                            *remembered = *is_hovered;
                            cx.notify();
                        });
                    })
                    .on_click(cx.listener(move |shell, _, window, cx| match channel_id {
                        Some(channel_id) => shell.open_channel(channel_id, window, cx),
                        None => shell.open_place(None, None, window, cx),
                    }))
                    .child(row.leading)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                with_type(div(), TYPE_CHANNEL_NAME)
                                    .truncate()
                                    .child(row.name),
                            )
                            .when_some(row.second_line, |names, second_line| {
                                names.child(
                                    with_type(div(), TYPE_BODY_SM)
                                        .truncate()
                                        .text_color(color(
                                            tokens.component_group_avatar_member_count_text,
                                        ))
                                        .child(second_line),
                                )
                            }),
                    )
                    .when(row.is_muted, |button| {
                        button.child(icon(
                            IconName::BellSlash,
                            COMPONENT_CHANNEL_ROW_MUTED_ICON_SIZE,
                            color(tokens.component_channel_row_muted_icon),
                        ))
                    })
                    .when(row.mention_count > 0, |button| {
                        button.child(MentionBadge::new(row.mention_count))
                    })
                    .when(is_ring_shown, |button| {
                        button.child(inset_focus_ring(COMPONENT_CHANNEL_ROW_RADIUS, cx))
                    }),
            )
            .into_any_element()
    }
}

/// What a row's colour says.
#[derive(Debug, Clone, Copy)]
struct RowLook {
    is_selected: bool,
    has_unread: bool,
    is_muted: bool,
}

/// The row's text and icon colour (`sidebarRowClasses`): never by colour alone, the pill, the
/// bell-slash and the accessible name say unread and muted too.
fn row_ink(look: RowLook, is_hovered: bool, cx: &gpui::App) -> gpui::Hsla {
    let tokens = theme(cx);
    color(if look.is_selected {
        tokens.component_channel_row_text_selected
    } else if is_hovered {
        tokens.component_channel_row_text_hover
    } else if look.is_muted {
        tokens.component_channel_row_text_muted_channel
    } else if look.has_unread {
        tokens.component_channel_row_text_unread
    } else {
        tokens.component_channel_row_text_read
    })
}

/// A sentence where the list has nothing to show.
fn empty_note(text: &'static str, cx: &gpui::App) -> impl IntoElement + use<> {
    let tokens = theme(cx);
    with_type(div(), TYPE_BODY)
        .mt(length(SPACE_XS))
        .mr(length(COMPONENT_CHANNEL_ROW_MARGIN_INLINE))
        .px(length(COMPONENT_CHANNEL_ROW_PADDING_INLINE))
        .text_color(color(tokens.color_text_muted))
        .child(text)
}
