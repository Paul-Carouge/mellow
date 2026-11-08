//! The server strip of the navigator card (`ServerRail.svelte`, DESIGN.md §5.2, ADR-0022 §1):
//! Home, a separator, then the servers. Icons morph from circle to squircle on hover, focus and
//! selection; the pill at the card's edge says unread (short), hover (medium) or selected (tall);
//! the mention badge sits on the icon's corner. Tooltips name each icon at once.

use gpui::{
    AnyElement, Context, ElementId, InteractiveElement as _, IntoElement, KeyDownEvent, ObjectFit,
    ParentElement as _, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledImage as _, Window, div, img, prelude::FluentBuilder as _,
};
use gpui_base::motion::{Transition, transition};
use mellow_client_core::shared_logic::format::server_initials;
use mellow_client_core::shared_logic::notification_settings::NotificationSettings;
use mellow_client_core::shared_logic::notifications::{NotificationPlace, is_place_muted};
use mellow_client_core::state::ClientState;
use mellow_protocol::{Guild, Snowflake};

use super::place_menus::{MenuRow, PlaceMenuTarget, is_target_muted, with_place_menu};
use super::regions::{Region, RovingTabStop};
use super::{DuskShell, icon_picture};
use crate::components::{
    MentionBadge, Tooltip, child_id, easing, focus_ring, icon, is_focus_visible,
    remembered_focus_handle, remembered_hover, with_type,
};
use crate::i18n::{Strings, strings};
use crate::session::now_unix_milliseconds;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_BADGE_RING_WIDTH, COMPONENT_NAVIGATOR_STRIP_WIDTH, COMPONENT_RAIL_ICON_RADIUS_ACTIVE,
    COMPONENT_RAIL_ICON_RADIUS_REST, COMPONENT_RAIL_ITEM_GAP, COMPONENT_RAIL_ITEM_SIZE,
    COMPONENT_RAIL_MORPH_DURATION, COMPONENT_RAIL_PADDING_TOP, COMPONENT_RAIL_PILL_HEIGHT_HOVER,
    COMPONENT_RAIL_PILL_HEIGHT_SELECTED, COMPONENT_RAIL_PILL_HEIGHT_UNREAD,
    COMPONENT_RAIL_PILL_WIDTH, COMPONENT_RAIL_TOOLTIP_DELAY, MOTION_DURATION_BASE,
    MOTION_EASING_SETTLE, MOTION_EASING_STANDARD, SIZE_RAIL_SEPARATOR_HEIGHT,
    SIZE_RAIL_SEPARATOR_WIDTH, TYPE_BODY,
};
use crate::theme::{animation_duration, color, delay, length, theme};

/// What a rail entry shows.
enum RailMark<'a> {
    Home,
    Server { guild: &'a Guild, is_muted: bool },
}

/// "Rust Lyon, 3 mentions, unread" (DESIGN.md §7.3): the count first, the state last
/// (`nameWithUnreadState`).
pub(super) fn name_with_unread_state(
    strings: Strings,
    name: &str,
    has_unread: bool,
    mention_count: u32,
) -> String {
    let with_mentions = if mention_count > 0 {
        strings
            .common()
            .with_mentions(name, i64::from(mention_count))
    } else {
        name.to_owned()
    };
    if has_unread {
        strings.common().with_unread(&with_mentions)
    } else {
        with_mentions
    }
}

/// Whether any of the server's channels not muted has unread messages; a muted server shows none
/// (DESIGN.md §5.3).
pub(super) fn has_unread_server(
    state: &ClientState,
    guild: &Guild,
    settings: &NotificationSettings,
    now: u64,
) -> bool {
    !is_target_muted(settings, guild.id, now)
        && guild.channels.iter().any(|channel| {
            let place = NotificationPlace {
                guild_id: Some(guild.id),
                channel_id: channel.id,
            };
            !is_place_muted(settings, place, now) && state.has_unread(channel.id)
        })
}

impl DuskShell {
    pub(super) fn render_server_rail(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let strings = strings(cx);
        let state = self.session.read(cx).state();
        let selected_guild_id = self.navigation.selected_guild_id();
        let is_home_selected = self.navigation.place().is_some() && selected_guild_id.is_none();
        let settings = self.notification_settings(cx);
        let now = now_unix_milliseconds();
        // A muted conversation raises nothing at Home: every message of a conversation counts.
        let audible_conversations: Vec<Snowflake> = state
            .direct_message_channels()
            .iter()
            .map(|conversation| conversation.id)
            .filter(|channel_id| {
                let place = NotificationPlace {
                    guild_id: None,
                    channel_id: *channel_id,
                };
                !is_place_muted(&settings, place, now)
            })
            .collect();
        let has_unread_conversation = audible_conversations
            .iter()
            .any(|channel_id| state.has_unread(*channel_id));
        let conversations_mention_count = audible_conversations
            .iter()
            .map(|channel_id| state.mention_count(*channel_id))
            .fold(0, u32::saturating_add);
        let servers: Vec<(Guild, bool, u32)> = state
            .guilds()
            .iter()
            .map(|guild| {
                (
                    guild.clone(),
                    has_unread_server(state, guild, &settings, now),
                    state.guild_mention_count(guild.id),
                )
            })
            .collect();

        let has_selected_row = self.navigation.place().is_some();
        let mut tab_stop = self.regions.rail.tab_stop_chooser(has_selected_row, window);
        let mut items: Vec<AnyElement> = Vec::with_capacity(servers.len() + 2);
        items.push(Self::rail_item(
            &RailMark::Home,
            &strings.rail().direct_messages().into(),
            is_home_selected,
            has_unread_conversation,
            conversations_mention_count,
            (&mut tab_stop, items.len()),
            window,
            cx,
        ));
        items.push(
            div()
                .flex_none()
                .h(length(SIZE_RAIL_SEPARATOR_HEIGHT))
                .w(length(SIZE_RAIL_SEPARATOR_WIDTH))
                .rounded_full()
                .bg(color(theme(cx).component_rail_separator_color))
                .into_any_element(),
        );
        for (guild, has_unread, mention_count) in servers {
            let is_selected = selected_guild_id == Some(guild.id);
            let name = SharedString::from(guild.name.clone());
            let child_index = items.len();
            let is_muted = is_target_muted(&settings, guild.id, now);
            items.push(Self::rail_item(
                &RailMark::Server {
                    guild: &guild,
                    is_muted,
                },
                &name,
                is_selected,
                has_unread,
                mention_count,
                (&mut tab_stop, child_index),
                window,
                cx,
            ));
        }
        self.regions.rail.keep_rows(tab_stop, cx);

        div()
            .debug_selector(|| "server-rail".to_owned())
            .id("server-rail")
            .role(Role::Navigation)
            .aria_label(strings.rail().servers())
            .track_focus(&self.regions.rail.focus)
            .track_scroll(&self.regions.rail.scroll)
            .on_key_down(cx.listener(|shell, event: &KeyDownEvent, window, cx| {
                shell.move_in_list(Region::ServerRail, event, window, cx);
            }))
            .flex_none()
            .w(length(COMPONENT_NAVIGATOR_STRIP_WIDTH))
            .h_full()
            .overflow_y_scroll()
            .pt(length(COMPONENT_RAIL_PADDING_TOP))
            .flex()
            .flex_col()
            .items_center()
            .gap(length(COMPONENT_RAIL_ITEM_GAP))
            .children(items)
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // One rail entry, as the web's snippet.
    fn rail_item(
        mark: &RailMark<'_>,
        name: &SharedString,
        is_selected: bool,
        has_unread: bool,
        mention_count: u32,
        (tab_stop, child_index): (&mut RovingTabStop, usize),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let strings = strings(cx);
        let (guild_id, is_muted) = match mark {
            RailMark::Home => (None, false),
            RailMark::Server { guild, is_muted } => (Some(guild.id), *is_muted),
        };
        let id = match guild_id {
            Some(guild_id) => ElementId::from(("rail-server", guild_id.raw())),
            None => ElementId::from("rail-home"),
        };
        let focus_handle = tab_stop.row(
            &id,
            remembered_focus_handle(&id, window, cx),
            is_selected,
            child_index,
        );
        let hover = remembered_hover(&id, window, cx);
        let is_ring_shown = is_focus_visible(&focus_handle, window);
        let is_active = is_selected || *hover.read(cx) || is_ring_shown;
        // Three curves (ADR-0022 §3): the squircle settles, the pill moves on `standard`.
        let radius = transition(
            child_id(&id, "radius"),
            if is_active {
                COMPONENT_RAIL_ICON_RADIUS_ACTIVE
            } else {
                COMPONENT_RAIL_ICON_RADIUS_REST
            },
            Transition::new(animation_duration(COMPONENT_RAIL_MORPH_DURATION, cx))
                .easing(easing(MOTION_EASING_SETTLE)),
            window,
            cx,
        );
        let pill_height = transition(
            child_id(&id, "pill"),
            if is_selected {
                COMPONENT_RAIL_PILL_HEIGHT_SELECTED
            } else if is_active {
                COMPONENT_RAIL_PILL_HEIGHT_HOVER
            } else if has_unread {
                COMPONENT_RAIL_PILL_HEIGHT_UNREAD
            } else {
                0.0
            },
            Transition::new(animation_duration(MOTION_DURATION_BASE, cx))
                .easing(easing(MOTION_EASING_STANDARD)),
            window,
            cx,
        );
        let (fill, ink) = match (mark, is_active) {
            (RailMark::Home, true) => (tokens.color_accent_fill, tokens.color_text_on_accent),
            (RailMark::Home | RailMark::Server { .. }, false) => (
                tokens.component_rail_icon_background_rest,
                tokens.component_rail_icon_text_rest,
            ),
            (RailMark::Server { .. }, true) => (
                tokens.component_rail_icon_background_active,
                tokens.component_rail_icon_text_active,
            ),
        };
        let content: AnyElement = match mark {
            RailMark::Home => icon(
                IconName::ChatCircle,
                COMPONENT_RAIL_ITEM_SIZE / 2.0,
                color(ink),
            )
            .into_any_element(),
            RailMark::Server { guild, .. } => match guild.icon {
                Some(picture) => img(icon_picture(picture, COMPONENT_RAIL_ITEM_SIZE, window))
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .with_fallback({
                        let initials = server_initials(&guild.name);
                        move || {
                            with_type(div(), TYPE_BODY)
                                .child(initials.clone())
                                .into_any_element()
                        }
                    })
                    .into_any_element(),
                None => with_type(div(), TYPE_BODY)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(server_initials(&guild.name))
                    .into_any_element(),
            },
        };
        let label = name_with_unread_state(
            strings,
            &if is_muted {
                strings.notifications().with_muted(name)
            } else {
                name.to_string()
            },
            has_unread,
            mention_count,
        );
        let tooltip = name.clone();
        let pill_colour = if is_selected {
            tokens.component_rail_pill_color_selected
        } else {
            tokens.component_rail_pill_color
        };

        let menu_focus = focus_handle.clone();
        let button = div()
            .id(id.clone())
            .role(Role::Button)
            .aria_label(label)
            .when(is_selected, |button| button.aria_selected(true))
            .track_focus(&focus_handle)
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size(length(COMPONENT_RAIL_ITEM_SIZE))
            .rounded(length(radius))
            .bg(color(fill))
            .text_color(color(ink))
            .cursor_pointer()
            .tooltip(move |_, cx| Tooltip::view(tooltip.clone(), cx))
            .tooltip_show_delay(delay(COMPONENT_RAIL_TOOLTIP_DELAY))
            .on_hover(move |is_hovered, _, cx| {
                hover.update(cx, |remembered, cx| {
                    *remembered = *is_hovered;
                    cx.notify();
                });
            })
            .on_click(cx.listener(move |shell, _, window, cx| {
                shell.open_place(guild_id, None, window, cx);
            }))
            // The button clips its picture; the ring sits around it, outside the clip.
            .child(
                div()
                    .size_full()
                    .rounded(length(radius))
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(content),
            )
            .when(is_ring_shown, |button| button.child(focus_ring(radius, cx)))
            .when(mention_count > 0, |button| {
                button.child(
                    div()
                        .absolute()
                        .right(-length(COMPONENT_BADGE_RING_WIDTH))
                        .bottom(-length(COMPONENT_BADGE_RING_WIDTH))
                        .child(
                            MentionBadge::new(mention_count)
                                .ring(color(tokens.component_navigator_background)),
                        ),
                )
            });
        let button = match guild_id {
            Some(guild_id) => with_place_menu(
                button,
                MenuRow {
                    target: PlaceMenuTarget::Server(guild_id),
                    region: Region::ServerRail,
                    child_index,
                },
                menu_focus,
                cx,
            ),
            None => button,
        };

        div()
            .relative()
            .w_full()
            .flex()
            .flex_none()
            .justify_center()
            // At the card's inner edge, half its width showing (ADR-0022 §1).
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(length((COMPONENT_RAIL_ITEM_SIZE - pill_height) / 2.0))
                    .w(length(COMPONENT_RAIL_PILL_WIDTH / 2.0))
                    .h(length(pill_height))
                    .rounded_r(length(COMPONENT_RAIL_PILL_WIDTH))
                    .bg(color(pill_colour)),
            )
            .child(button)
            .into_any_element()
    }
}
