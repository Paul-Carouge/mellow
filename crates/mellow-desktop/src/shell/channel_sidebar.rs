//! The navigator card (`Navigator.svelte`, ADR-0022 §1) and its channel list
//! (`ChannelSidebar.svelte`, `ConversationList.svelte`, DESIGN.md §5.3, §5.6): the server's name,
//! then its channels by category, or the conversations at Home. Collapsed categories still show
//! the open and unread channels and the voice channels with people in them, as on Discord.

use gpui::{
    AnyElement, Context, ElementId, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Role, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::call_view::{LiveBadge, Watcher, live_badge_of};
use mellow_client_core::shared_logic::channel_order::channel_sections;
use mellow_client_core::shared_logic::context_menus_places::{
    ServerMenuPlace, server_menu_actions, server_menu_facts_of,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::group_conversations::participant_count;
use mellow_client_core::state::ClientState;
use mellow_protocol::{Channel, ChannelKind, DirectMessageChannel, Permissions, Snowflake};

use super::place_menus::{MenuRow, PlaceMenuTarget, is_target_muted, with_place_menu};
use super::regions::{Region, RovingTabStop};
use super::server_rail::name_with_unread_state;
use super::{DuskShell, profile_picture};
use crate::components::{
    Avatar, AvatarPerson, Button, ButtonVariant, GroupAvatar, IconButton, MentionBadge, child_id,
    focus_ring, icon, inset_focus_ring, is_focus_visible, remembered_focus_handle,
    remembered_hover, with_type,
};
use crate::home::{conversation_display_name, shown_presence};
use crate::i18n::strings;
use crate::session::now_unix_milliseconds;
use crate::settings::channel::has_channel_settings_page_for_reader;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CATEGORY_ADD_ICON_SIZE, COMPONENT_CATEGORY_CHEVRON_SIZE, COMPONENT_CATEGORY_HEIGHT,
    COMPONENT_CATEGORY_MARGIN_TOP, COMPONENT_CHANNEL_ROW_GAP, COMPONENT_CHANNEL_ROW_HEIGHT,
    COMPONENT_CHANNEL_ROW_ICON_GAP, COMPONENT_CHANNEL_ROW_ICON_SIZE,
    COMPONENT_CHANNEL_ROW_MARGIN_INLINE, COMPONENT_CHANNEL_ROW_MUTED_ICON_SIZE,
    COMPONENT_CHANNEL_ROW_PADDING_INLINE, COMPONENT_CHANNEL_ROW_RADIUS,
    COMPONENT_CHANNEL_ROW_SETTINGS_BUTTON_SIZE, COMPONENT_CHANNEL_ROW_SETTINGS_ICON_SIZE,
    COMPONENT_CHANNEL_ROW_UNREAD_PILL_HEIGHT, COMPONENT_CHANNEL_ROW_UNREAD_PILL_OFFSET,
    COMPONENT_CHANNEL_ROW_UNREAD_PILL_WIDTH, COMPONENT_CONVERSATION_ROW_AVATAR_GAP,
    COMPONENT_MEMBER_ROW_AVATAR_SIZE, COMPONENT_MEMBER_ROW_GAP, COMPONENT_MEMBER_ROW_HEIGHT,
    COMPONENT_NAVIGATOR_RADIUS, COMPONENT_NAVIGATOR_STRIP_WIDTH,
    COMPONENT_SIDEBAR_HEADER_CHEVRON_SIZE, COMPONENT_SIDEBAR_HEADER_HEIGHT,
    COMPONENT_VOICE_PARTICIPANT_AVATAR_SIZE, COMPONENT_VOICE_PARTICIPANT_GAP,
    COMPONENT_VOICE_PARTICIPANT_INDENT, COMPONENT_VOICE_PARTICIPANT_ROW_HEIGHT,
    COMPONENT_VOICE_PARTICIPANT_SPEAKING_RING_GAP, COMPONENT_VOICE_PARTICIPANT_SPEAKING_RING_WIDTH,
    COMPONENT_VOICE_PARTICIPANT_STATE_ICON_SIZE, SPACE_2XS, SPACE_MD, SPACE_XS, TYPE_BODY,
    TYPE_BODY_SM, TYPE_CATEGORY, TYPE_CHANNEL_NAME, TYPE_HEADING_MD,
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
    /// A voice channel's row: pressing it joins, never opens.
    voice: Option<VoiceRowLook>,
    /// Space above the row: the rows' gap, more before a list's first row.
    top_margin: f32,
    /// From the leading icon or avatar to the name.
    icon_gap: f32,
    /// The channel whose settings the trailing gear opens ("Edit channel"): shown on the
    /// selected row and on hover or focus.
    gear: Option<Snowflake>,
}

/// What a voice channel's row adds (`voiceChannelRow` in `ChannelSidebar.svelte`).
struct VoiceRowLook {
    /// Where the reader is reads bright, without the selected wash.
    is_here: bool,
    /// "2/5" when the channel has a limit.
    count_and_limit: Option<String>,
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
        let voice_panel = self.render_voice_panel(window, cx);
        let voice_announcements = self.render_voice_announcements(cx);
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
            .children(voice_panel)
            .children(voice_announcements)
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
        let guild_id = guild.as_ref().map(|guild| guild.id);
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
        let server_menu_guild_id =
            guild_id.filter(|guild_id| self.has_server_header_menu(*guild_id, cx));
        let header: AnyElement = match server_menu_guild_id {
            Some(guild_id) => self
                .server_header_button(guild_id, heading.clone(), window, cx)
                .into_any_element(),
            None => with_type(div(), TYPE_HEADING_MD)
                .id("sidebar-heading")
                .role(Role::Heading)
                .aria_level(2)
                .size_full()
                .pl(length(SPACE_XS))
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
                })
                .into_any_element(),
        };
        let reorder_error = self.render_reorder_error(cx);

        div()
            .debug_selector(|| "channel-column".to_owned())
            .flex_none()
            .w(length(width))
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .h(length(COMPONENT_SIDEBAR_HEADER_HEIGHT))
                    .mt(length(SPACE_XS))
                    .mr(length(SPACE_XS))
                    .child(header),
            )
            .children(reorder_error)
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

    /// Whether the open server's header opens a menu: it offers the reader something.
    fn has_server_header_menu(&self, guild_id: Snowflake, cx: &gpui::App) -> bool {
        let state = self.state(cx);
        state.guild(guild_id).is_some_and(|guild| {
            !server_menu_actions(
                server_menu_facts_of(guild, state.current_user().map(|user| user.id), false),
                ServerMenuPlace::Header,
            )
            .is_empty()
        })
    }

    /// The server's name as the button of its menu (`ChannelSidebar.svelte`'s header): a caret
    /// that becomes a close while the menu is open, and the hover wash.
    fn server_header_button(
        &mut self,
        guild_id: Snowflake,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tokens = theme(cx);
        let strings = strings(cx);
        let id = ElementId::from("server-header");
        let focus_handle = remembered_focus_handle(&id, window, cx);
        let hover = remembered_hover(&id, window, cx);
        let is_open = self.is_server_header_menu_open();
        let is_washed = is_open || *hover.read(cx);
        let is_ring_shown = is_focus_visible(&focus_handle, window);
        let bounds = self.place_menus.server_header_bounds.clone();
        let presser = cx.entity().downgrade();
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(SharedString::from(format!(
                "{name}, {}",
                strings.sidebar().server_menu()
            )))
            .aria_expanded(is_open)
            .track_focus(&focus_handle)
            .relative()
            .size_full()
            .px(length(SPACE_XS))
            .flex()
            .items_center()
            .gap(length(SPACE_XS))
            .rounded(length(crate::theme::tokens::RADIUS_SM))
            .when(is_washed, |button| {
                button.bg(color(tokens.component_sidebar_header_background_hover))
            })
            .text_color(color(tokens.component_sidebar_header_text))
            .cursor_pointer()
            .on_hover(move |is_hovered, _, cx| {
                hover.update(cx, |remembered, cx| {
                    *remembered = *is_hovered;
                    cx.notify();
                });
            })
            .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                let _ = presser.update(cx, |shell, _| shell.note_server_header_press());
            })
            .on_click(cx.listener(move |shell, _, window, cx| {
                shell.toggle_server_header_menu(guild_id, window, cx);
            }))
            .on_key_down(cx.listener(move |shell, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space" | "down") {
                    cx.stop_propagation();
                    shell.toggle_server_header_menu(guild_id, window, cx);
                }
            }))
            .child(
                with_type(div(), TYPE_HEADING_MD)
                    .id("sidebar-heading")
                    .role(Role::Heading)
                    .aria_level(2)
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(name),
            )
            .child(icon(
                if is_open {
                    IconName::X
                } else {
                    IconName::CaretDown
                },
                COMPONENT_SIDEBAR_HEADER_CHEVRON_SIZE,
                color(tokens.color_text_secondary),
            ))
            .when(is_ring_shown, |button| {
                button.child(focus_ring(crate::theme::tokens::RADIUS_SM, cx))
            })
            // Records where the header is, for its menu to open under it.
            .child(
                gpui::canvas(
                    move |header_bounds, _, _| bounds.set(Some(header_bounds)),
                    |_, (), _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
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
            if !can_create_channels {
                return vec![
                    empty_note(strings.sidebar().no_visible_channels_yet(), cx).into_any_element(),
                ];
            }
            let entity = cx.entity().downgrade();
            return vec![
                div()
                    .mt(length(SPACE_XS))
                    .mr(length(COMPONENT_CHANNEL_ROW_MARGIN_INLINE))
                    .px(length(COMPONENT_CHANNEL_ROW_PADDING_INLINE))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(length(SPACE_XS))
                    .child(
                        with_type(div(), TYPE_BODY)
                            .text_color(color(theme(cx).color_text_muted))
                            .child(strings.sidebar().no_channels_yet()),
                    )
                    .child(
                        Button::new("empty-create-channel", strings.sidebar().create_a_channel())
                            .variant(ButtonVariant::Primary)
                            .on_click(move |window, cx| {
                                let _ = entity.update(cx, |shell, cx| {
                                    shell.open_create_channel(
                                        guild_id,
                                        None,
                                        ChannelKind::Text,
                                        window,
                                        cx,
                                    );
                                });
                            }),
                    )
                    .into_any_element(),
            ];
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
                let can_create_in = self.can_manage_server_channels(guild_id, cx);
                rows.push(Self::category_row(
                    category,
                    is_collapsed,
                    can_create_in.then_some(guild_id),
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
                    let is_here = self
                        .voice
                        .as_ref()
                        .is_some_and(|voice| voice.read(cx).channel_id() == Some(channel.id));
                    if !is_collapsed || !participant_ids.is_empty() || is_here {
                        let child_index = rows.len();
                        rows.push(self.voice_channel_row(
                            channel,
                            guild_id,
                            &participant_ids,
                            (tab_stop, child_index),
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
                        voice: None,
                        top_margin: COMPONENT_CHANNEL_ROW_GAP,
                        icon_gap: COMPONENT_CHANNEL_ROW_ICON_GAP,
                        gear: has_channel_settings_page_for_reader(state, channel.id)
                            .then_some(channel.id),
                    },
                    (tab_stop, rows.len()),
                    window,
                    cx,
                ));
            }
        }
        rows
    }

    /// "Create a channel in Talk", at the category header's trailing end.
    fn category_add_button(
        guild_id: Snowflake,
        category: &Channel,
        cx: &mut Context<Self>,
    ) -> IconButton {
        let category_id = category.id;
        let entity = cx.entity().downgrade();
        IconButton::new(
            ElementId::from(("category-add", category_id.raw())),
            IconName::Plus,
            strings(cx).sidebar().create_channel_in(&category.name),
        )
        .sizes(COMPONENT_CATEGORY_HEIGHT, COMPONENT_CATEGORY_ADD_ICON_SIZE)
        .on_click(move |window, cx| {
            let _ = entity.update(cx, |shell, cx| {
                shell.open_create_channel(
                    guild_id,
                    Some(category_id),
                    ChannelKind::Text,
                    window,
                    cx,
                );
            });
        })
    }

    /// A category's header: its button folds it; `create_in` (the server, for a manager) adds
    /// the "+" that creates a channel inside it.
    fn category_row(
        category: &Channel,
        is_collapsed: bool,
        create_in: Option<Snowflake>,
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
        let add_button =
            create_in.map(|guild_id| Self::category_add_button(guild_id, category, cx));
        // The caret starts at the list's edge, as the web's category button has no padding.
        div()
            .mt(length(COMPONENT_CATEGORY_MARGIN_TOP))
            .mr(length(COMPONENT_CHANNEL_ROW_MARGIN_INLINE))
            .flex()
            .items_center()
            .gap(length(SPACE_2XS))
            .child(
                header
                    .flex_1()
                    .min_w_0()
                    .role(Role::Button)
                    .aria_label(SharedString::from(category.name.clone()))
                    .aria_expanded(!is_collapsed)
                    .track_focus(&focus_handle)
                    .relative()
                    .flex()
                    .items_center()
                    .gap(length(SPACE_2XS))
                    .h(length(COMPONENT_CATEGORY_HEIGHT))
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
            .children(add_button)
            .into_any_element()
    }

    /// A voice channel and the people in it. Voice channels are joined, never opened: pressing
    /// the row joins, and each participant opens their volume and "Mute for me".
    fn voice_channel_row(
        &mut self,
        channel: &Channel,
        guild_id: Snowflake,
        participant_ids: &[Snowflake],
        tab_stop: (&mut RovingTabStop, usize),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let strings = strings(cx);
        let state = self.session.read(cx).state();
        let can_connect = state
            .my_channel_permissions(channel.id)
            .is_none_or(|permissions| permissions.contains(Permissions::CONNECT));
        let is_here = self
            .voice
            .as_ref()
            .is_some_and(|voice| voice.read(cx).channel_id() == Some(channel.id));
        let people = i64::try_from(participant_ids.len()).unwrap_or(i64::MAX);
        let limit = channel.user_limit.map(i64::from);
        let label = strings
            .voice()
            .row_label(&channel.name, people, limit, is_here, can_connect);
        let leading_ink = row_ink(
            RowLook {
                is_selected: false,
                has_unread: is_here,
                is_muted: false,
            },
            false,
            cx,
        );
        let row = Self::sidebar_row(
            SidebarRow {
                id: ElementId::from(("voice-row", channel.id.raw())),
                label,
                name: channel.name.clone().into(),
                second_line: None,
                leading: icon(
                    if can_connect {
                        IconName::SpeakerHigh
                    } else {
                        IconName::Lock
                    },
                    COMPONENT_CHANNEL_ROW_ICON_SIZE,
                    leading_ink,
                )
                .into_any_element(),
                height: COMPONENT_CHANNEL_ROW_HEIGHT,
                is_selected: false,
                has_unread: false,
                mention_count: 0,
                channel_id: Some(channel.id),
                is_muted: false,
                menu: Some(PlaceMenuTarget::Channel(channel.id)),
                voice: Some(VoiceRowLook {
                    is_here,
                    count_and_limit: limit.map(|limit| format!("{people}/{limit}")),
                }),
                top_margin: COMPONENT_CHANNEL_ROW_GAP,
                icon_gap: COMPONENT_CHANNEL_ROW_ICON_GAP,
                gear: has_channel_settings_page_for_reader(state, channel.id).then_some(channel.id),
            },
            tab_stop,
            window,
            cx,
        );
        let participants: Vec<AnyElement> = participant_ids
            .iter()
            .map(|user_id| self.voice_participant_row(guild_id, *user_id, window, cx))
            .collect();
        div()
            .child(row)
            .when(!participants.is_empty(), |item| {
                item.child(
                    div()
                        .id(ElementId::from(("voice-participants", channel.id.raw())))
                        .role(Role::List)
                        .aria_label(strings.voice().participants_in(&channel.name))
                        .children(participants),
                )
            })
            .into_any_element()
    }

    /// One person in a voice channel: the speaking ring, their name, and their marks (microphone
    /// off, sound off, muted for the reader). Speaking is shown, never announced.
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn voice_participant_row(
        &mut self,
        guild_id: Snowflake,
        user_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let strings = strings(cx);
        let state = self.session.read(cx).state();
        let is_me = state.current_user().is_some_and(|user| user.id == user_id);
        let voice_state = state.voice_state_of(user_id).cloned();
        let person = state
            .members(guild_id)
            .iter()
            .map(|member| member.user.clone())
            .find(|user| user.id == user_id)
            .or_else(|| {
                state
                    .current_user_as_public_user()
                    .filter(|user| user.id == user_id)
            });
        let is_speaking_by_server = state.is_speaking(user_id);
        // The sound they just played, for its length: shown, never announced, like speaking.
        let played_sound = state
            .sound_indicator(user_id, crate::session::now_unix_milliseconds())
            .map(|indicator| {
                let sound = state
                    .sounds_of(indicator.guild_id)
                    .iter()
                    .find(|sound| sound.id == indicator.sound_id);
                (
                    sound.and_then(|sound| sound.emoji.clone()),
                    sound.map(|sound| sound.name.clone()).unwrap_or_default(),
                )
            });
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
                id: user_id,
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
        let is_self_speaking = self
            .voice
            .as_ref()
            .is_some_and(|voice| voice.read(cx).is_self_speaking());
        let is_speaking = is_speaking_by_server || (is_me && is_self_speaking);
        let is_muted_for_me =
            !is_me && crate::voice::Voice::person_setting(user_id, cx).is_locally_muted;
        let (is_deafened, is_self_muted, is_suppressed) =
            voice_state
                .as_ref()
                .map_or((false, false, false), |voice_state| {
                    (
                        voice_state.self_deaf,
                        voice_state.self_mute,
                        voice_state.suppressed,
                    )
                });
        let is_microphone_off = is_self_muted || is_suppressed;
        let is_streaming = voice_state.as_ref().is_some_and(|state| state.self_stream);
        let is_camera_on = voice_state.as_ref().is_some_and(|state| state.self_video);
        let live_badge = voice_state.as_ref().and_then(|voice_state| {
            let watcher = self.voice.as_ref().map_or(
                Watcher {
                    connected_channel_id: None,
                    can_watch_streams: false,
                },
                |voice| {
                    let voice = voice.read(cx);
                    Watcher {
                        connected_channel_id: voice
                            .phase()
                            .is_some()
                            .then(|| voice.channel_id())
                            .flatten(),
                        can_watch_streams: crate::voice::Voice::can_show_video_in(
                            voice_state.video_codec,
                        ),
                    }
                },
            );
            live_badge_of(
                voice_state,
                state.current_user().map(|user| user.id),
                watcher,
            )
        });
        let states: Vec<&str> = [
            (is_deafened, strings.voice().sound_off()),
            (
                is_self_muted && !is_deafened,
                strings.voice().microphone_off(),
            ),
            (is_suppressed, strings.voice().cannot_speak()),
            (is_muted_for_me, strings.voice().muted_for_you()),
            (is_streaming, strings.screen_share().shares_screen()),
            (is_camera_on, strings.camera().camera_on()),
        ]
        .into_iter()
        .filter_map(|(is_shown, word)| is_shown.then_some(word))
        .collect();
        let label = strings.voice().participant_label(&name, &states);
        let id = ElementId::from(("voice-participant", user_id.raw()));
        let focus_handle = remembered_focus_handle(&id, window, cx);
        let hover = remembered_hover(&id, window, cx);
        let is_hovered = *hover.read(cx);
        let is_ring_shown = is_focus_visible(&focus_handle, window);
        let state_ink = color(tokens.component_voice_participant_state_icon);
        let ring = length(
            COMPONENT_VOICE_PARTICIPANT_SPEAKING_RING_GAP
                + COMPONENT_VOICE_PARTICIPANT_SPEAKING_RING_WIDTH,
        );
        let popover_name: SharedString = name.clone().into();
        // Your own row opens nothing: the user card holds your toggles.
        let can_open = !is_me && self.voice.is_some();
        let watch_label = strings.screen_share().watch(&name);
        let row = div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .track_focus(&focus_handle)
            .relative()
            .mr(length(COMPONENT_CHANNEL_ROW_MARGIN_INLINE))
            .h(length(COMPONENT_VOICE_PARTICIPANT_ROW_HEIGHT))
            .pl(length(COMPONENT_VOICE_PARTICIPANT_INDENT))
            .pr(length(COMPONENT_CHANNEL_ROW_PADDING_INLINE))
            .flex()
            .items_center()
            .gap(length(COMPONENT_VOICE_PARTICIPANT_GAP))
            .rounded(length(COMPONENT_CHANNEL_ROW_RADIUS))
            .when(is_hovered, |row| {
                row.bg(color(tokens.component_voice_participant_background_hover))
            })
            .when(can_open, |row| {
                row.cursor_pointer().on_click(cx.listener(
                    move |shell, event: &gpui::ClickEvent, window, cx| {
                        shell.open_voice_person_popover(
                            user_id,
                            popover_name.clone(),
                            event.position(),
                            window,
                            cx,
                        );
                    },
                ))
            })
            .on_hover(move |is_hovered, _, cx| {
                hover.update(cx, |remembered, cx| {
                    *remembered = *is_hovered;
                    cx.notify();
                });
            })
            .child(
                div()
                    .relative()
                    .flex_none()
                    .child(Avatar::new(avatar, COMPONENT_VOICE_PARTICIPANT_AVATAR_SIZE))
                    .when(is_speaking, |avatar| {
                        // The ring sits outside the picture, a gap of the card between them.
                        avatar.child(
                            div()
                                .absolute()
                                .top(-ring)
                                .left(-ring)
                                .right(-ring)
                                .bottom(-ring)
                                .rounded_full()
                                .border(length(COMPONENT_VOICE_PARTICIPANT_SPEAKING_RING_WIDTH))
                                .border_color(color(
                                    tokens.component_voice_participant_speaking_ring,
                                )),
                        )
                    }),
            )
            .child(
                with_type(div(), TYPE_BODY_SM)
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(color(if is_speaking {
                        tokens.component_voice_participant_name_text_speaking
                    } else {
                        tokens.component_voice_participant_name_text
                    }))
                    .child(name),
            )
            .when_some(played_sound, |row, (emoji, sound_name)| {
                let tooltip: SharedString = strings.soundboard().plays(&sound_name).into();
                row.child(
                    div()
                        .id("voice-participant-sound")
                        .flex_none()
                        .tooltip(move |_, cx| crate::components::Tooltip::view(tooltip.clone(), cx))
                        .tooltip_show_delay(crate::components::Tooltip::show_delay())
                        .child(crate::soundboard::sound_emoji(
                            emoji.as_deref(),
                            crate::theme::tokens::COMPONENT_REACTION_EMOJI_SIZE,
                            color(tokens.component_voice_participant_name_text_speaking),
                            self.session.read(cx).state(),
                            window,
                            cx,
                        )),
                )
            })
            .when(is_microphone_off && !is_deafened, |row| {
                row.child(icon(
                    IconName::MicrophoneSlash,
                    COMPONENT_VOICE_PARTICIPANT_STATE_ICON_SIZE,
                    state_ink,
                ))
            })
            .when(is_deafened, |row| {
                row.child(icon(
                    IconName::SpeakerSlash,
                    COMPONENT_VOICE_PARTICIPANT_STATE_ICON_SIZE,
                    state_ink,
                ))
            })
            .when(is_muted_for_me, |row| {
                row.child(icon(
                    IconName::SpeakerX,
                    COMPONENT_VOICE_PARTICIPANT_STATE_ICON_SIZE,
                    color(tokens.component_voice_participant_local_mute_icon),
                ))
            })
            .when(is_camera_on, |row| {
                row.child(icon(
                    IconName::VideoCamera,
                    COMPONENT_VOICE_PARTICIPANT_STATE_ICON_SIZE,
                    state_ink,
                ))
            })
            // Shown only: your own share never loops back, and elsewhere you join first.
            .when(
                live_badge.is_some_and(|badge| badge != LiveBadge::Watch),
                |row| {
                    row.child(super::call_view::live_chip(
                        strings.screen_share().live(),
                        cx,
                    ))
                },
            )
            .when(is_ring_shown, |row| {
                row.child(inset_focus_ring(COMPONENT_CHANNEL_ROW_RADIUS, cx))
            });
        if live_badge != Some(LiveBadge::Watch) {
            return row.into_any_element();
        }
        // Their screen, watched large: a button of its own beside the row.
        let watch_id = ElementId::from(("voice-participant-watch", user_id.raw()));
        let watch_focus = remembered_focus_handle(&watch_id, window, cx);
        let is_watch_ring_shown = is_focus_visible(&watch_focus, window);
        let voice = self.voice.clone();
        div()
            .flex()
            .items_center()
            .child(div().flex_1().min_w_0().child(row))
            .child(
                div()
                    .id(watch_id)
                    .role(Role::Button)
                    .aria_label(watch_label)
                    .track_focus(&watch_focus)
                    .relative()
                    .flex_none()
                    .h(length(COMPONENT_VOICE_PARTICIPANT_ROW_HEIGHT))
                    .pr(length(COMPONENT_CHANNEL_ROW_PADDING_INLINE))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        if let Some(voice) = voice.as_ref() {
                            voice.update(cx, |voice, cx| voice.watch_stream(Some(user_id), cx));
                        }
                    })
                    .child(super::call_view::live_chip(
                        strings.screen_share().live(),
                        cx,
                    ))
                    .when(is_watch_ring_shown, |button| {
                        button.child(inset_focus_ring(COMPONENT_CHANNEL_ROW_RADIUS, cx))
                    }),
            )
            .into_any_element()
    }

    /// Joins a voice channel from its row; in the channel you are in, it shows the call
    /// (ADR-0035), as on Discord.
    fn join_voice(&mut self, channel_id: Snowflake, cx: &mut Context<Self>) {
        if let Some(voice) = self.voice.clone() {
            voice.update(cx, |voice, cx| {
                if voice.channel_id() == Some(channel_id) {
                    voice.open_call_view(cx);
                } else {
                    voice.join(channel_id, cx);
                }
            });
        }
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
                voice: None,
                // The Friends row stands apart from the header, as on the web.
                top_margin: SPACE_XS,
                icon_gap: COMPONENT_CONVERSATION_ROW_AVATAR_GAP,
                gear: None,
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
            let row = Self::sidebar_row(
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
                    voice: None,
                    // The list opens a step below the Friends row (`ul.mt-xs`, which the first
                    // row's own gap collapses into).
                    top_margin: if child_index == 1 {
                        SPACE_XS
                    } else {
                        COMPONENT_MEMBER_ROW_GAP
                    },
                    icon_gap: COMPONENT_CONVERSATION_ROW_AVATAR_GAP,
                    gear: None,
                },
                (tab_stop, child_index),
                window,
                cx,
            );
            rows.push(Self::with_conversation_menu(
                row,
                conversation.id,
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
        let is_here = row.voice.as_ref().is_some_and(|voice| voice.is_here);
        let ink = row_ink(
            RowLook {
                is_selected: row.is_selected,
                has_unread: row.has_unread || is_here,
                is_muted: row.is_muted,
            },
            is_hovered,
            cx,
        );
        let joins_voice = row.voice.is_some();
        let count_and_limit = row.voice.and_then(|voice| voice.count_and_limit);
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
        // Shown on hover, on focus within the row and on the selected text row; the gear takes
        // the badge's (or the limit's) place while it shows.
        let item_id = child_id(&row.id, "item");
        let gear_id = child_id(&item_id, "gear");
        let gear_focus = remembered_focus_handle(&gear_id, window, cx);
        let is_gear_shown = row.gear.is_some()
            && (is_hovered
                || focus_handle.is_focused(window)
                || gear_focus.is_focused(window)
                || (row.is_selected && !joins_voice));
        let has_trailing_count = row.mention_count > 0 || count_and_limit.is_some();
        let gear = row.gear.map(|gear_channel_id| {
            let entity = cx.entity().downgrade();
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .right(length(SPACE_2XS))
                .flex()
                .items_center()
                .when(!is_gear_shown, gpui::Styled::invisible)
                .child(
                    IconButton::new(
                        gear_id,
                        IconName::GearSix,
                        strings(cx).sidebar().edit_channel(&row.name),
                    )
                    .track_focus(&gear_focus)
                    .sizes(
                        COMPONENT_CHANNEL_ROW_SETTINGS_BUTTON_SIZE,
                        COMPONENT_CHANNEL_ROW_SETTINGS_ICON_SIZE,
                    )
                    .on_click(move |window, cx| {
                        let _ = entity.update(cx, |shell, cx| {
                            shell.open_channel_settings(gear_channel_id, window, cx);
                        });
                    }),
                )
        });
        // Room for the gear, so a long name never runs under it; a badge already keeps it.
        let gear_room = (row.gear.is_some() && !has_trailing_count).then_some(
            COMPONENT_CHANNEL_ROW_SETTINGS_BUTTON_SIZE + COMPONENT_CHANNEL_ROW_PADDING_INLINE,
        );
        let button = div()
            .id(row.id)
            .role(if joins_voice {
                Role::Button
            } else {
                Role::Link
            })
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
            .id(item_id)
            .relative()
            .mt(length(row.top_margin))
            .mr(length(COMPONENT_CHANNEL_ROW_MARGIN_INLINE))
            .on_hover(move |is_hovered, _, cx| {
                hover.update(cx, |remembered, cx| {
                    *remembered = *is_hovered;
                    cx.notify();
                });
            })
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
                    .gap(length(row.icon_gap))
                    .h(length(row.height))
                    .px(length(COMPONENT_CHANNEL_ROW_PADDING_INLINE))
                    .when_some(gear_room, |button, room| button.pr(length(room)))
                    .rounded(length(COMPONENT_CHANNEL_ROW_RADIUS))
                    .when_some(fill, |button, fill| button.bg(color(fill)))
                    .text_color(ink)
                    .cursor_pointer()
                    .on_click(cx.listener(move |shell, _, window, cx| match channel_id {
                        Some(channel_id) if joins_voice => shell.join_voice(channel_id, cx),
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
                        button.child(
                            div()
                                .when(is_gear_shown, gpui::Styled::invisible)
                                .child(MentionBadge::new(row.mention_count)),
                        )
                    })
                    .when_some(count_and_limit, |button, count_and_limit| {
                        button.child(
                            with_type(div(), TYPE_BODY_SM)
                                .flex_none()
                                .when(is_gear_shown, gpui::Styled::invisible)
                                .text_color(color(tokens.component_voice_participant_limit_text))
                                .child(count_and_limit),
                        )
                    })
                    .when(is_ring_shown, |button| {
                        button.child(inset_focus_ring(COMPONENT_CHANNEL_ROW_RADIUS, cx))
                    }),
            )
            .children(gear)
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
