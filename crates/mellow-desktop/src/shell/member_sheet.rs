//! The member side sheet (`MemberList.svelte`, DESIGN.md §5.7, ADR-0022 §1): roles displayed
//! separately first, then Online and Offline, by name; offline avatars dimmed while names keep
//! full contrast. Docked beside the conversation where it fits, over it otherwise. Choosing a
//! member shows their profile in the sheet (`member_profile`).

use gpui::{
    AnyElement, Context, ElementId, InteractiveElement as _, IntoElement, KeyDownEvent,
    MouseButton, ParentElement as _, Role, StatefulInteractiveElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::member_order::{
    MemberListGroupKind, member_list_groups, members_sorted_by_display_name,
};
use mellow_protocol::PresenceStatus;

use super::{DuskShell, profile_picture};
use crate::components::{
    Avatar, AvatarPerson, Button, IconButton, LiveRegion as _, bounds_recorder, inset_focus_ring,
    is_context_menu_keystroke, is_focus_visible, menu_origin_at_pointer,
    menu_origin_beside_remembered, remembered_bounds, remembered_focus_handle, remembered_hover,
    with_type,
};
use crate::home::group_sheet::group_sheet_rows;
use crate::home::{PersonMenuRequest, conversation_display_name, presence_name, shown_presence};
use crate::i18n::strings;
use crate::session::MembersLoad;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CHAT_HEADER_BUTTON_SIZE, COMPONENT_CHAT_HEADER_HEIGHT,
    COMPONENT_CHAT_HEADER_ICON_SIZE, COMPONENT_MEMBER_LIST_GROUP_HEIGHT,
    COMPONENT_MEMBER_ROW_AVATAR_SIZE, COMPONENT_MEMBER_ROW_GAP, COMPONENT_MEMBER_ROW_HEIGHT,
    COMPONENT_MEMBER_ROW_MARGIN_INLINE, COMPONENT_MEMBER_ROW_PADDING_INLINE,
    COMPONENT_MEMBER_ROW_RADIUS, COMPONENT_SIDE_SHEET_RADIUS, SPACE_2XS, SPACE_MD, SPACE_SM,
    SPACE_XS, TYPE_BODY, TYPE_BODY_SM, TYPE_CATEGORY, TYPE_HEADING_MD, TYPE_MEMBER_NAME,
};
use crate::theme::{color, length, theme, window_width_at_default_zoom};

impl DuskShell {
    /// "Members of Rust", or of the open group.
    pub(super) fn member_sheet_label(&self, cx: &gpui::App) -> String {
        let strings = strings(cx);
        let state = self.session.read(cx).state();
        let Some(place_id) = self.member_sheet_place_id(cx) else {
            return String::new();
        };
        if let Some(guild) = state.guild(place_id) {
            return strings.shell().members_of(&guild.name);
        }
        state
            .direct_message_channel(place_id)
            .map(|group| {
                let current_user_id = state.current_user().map(|user| user.id);
                strings.shell().members_of(&conversation_display_name(
                    group,
                    current_user_id,
                    strings,
                ))
            })
            .unwrap_or_default()
    }

    /// `width` is the docked width; `None` is the overlay, at the token width or (`is_full_window`)
    /// across the window.
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    pub(super) fn render_member_sheet(
        &mut self,
        width: Option<f32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tokens = theme(cx);
        let strings = strings(cx);
        let state = self.session.read(cx).state();
        let guild = self
            .member_sheet_place_id(cx)
            .and_then(|guild_id| state.guild(guild_id))
            .cloned();
        let group = self
            .member_sheet_place_id(cx)
            .and_then(|group_id| state.direct_message_channel(group_id))
            .filter(|_| guild.is_none())
            .cloned();
        // From the list, so a new avatar or name shows in an open profile too.
        let profile_user = guild.as_ref().and_then(|guild| {
            let user_id = self.member_profile.shown_user_id(guild.id)?;
            state
                .members(guild.id)
                .iter()
                .find(|member| member.user.id == user_id)
                .map(|member| member.user.clone())
        });
        let sheet_label = match &profile_user {
            Some(user) => strings.members().profile_of(display_name(user)),
            None => self.member_sheet_label(cx),
        };
        let heading = if profile_user.is_some() {
            strings.members().profile()
        } else if group.is_some() {
            strings.groups().members()
        } else {
            strings.members().members()
        };
        let entity = cx.entity();
        let back_entity = entity.clone();
        let sheet_width = width.unwrap_or_else(|| window_width_at_default_zoom(window));
        let body: Vec<AnyElement> = match &guild {
            None => group
                .map(|group| group_sheet_rows(&self.home, &self.session, &group, window, cx))
                .unwrap_or_default(),
            Some(_) if profile_user.is_some() => profile_user
                .iter()
                .map(|user| self.render_member_profile(user, sheet_width, window, cx))
                .collect(),
            Some(guild) => {
                let members = state.members(guild.id);
                let load = self.session.read(cx).members_load(guild.id);
                if members.is_empty() && load == Some(MembersLoad::Failed) {
                    let guild_id = guild.id;
                    let session = self.session.clone();
                    vec![
                        div()
                            .id("members-failed")
                            .live_assertive(strings.members().load_failed())
                            .px(length(SPACE_MD))
                            .pt(length(SPACE_XS))
                            .flex()
                            .flex_col()
                            .items_start()
                            .gap(length(SPACE_SM))
                            .child(
                                with_type(div(), TYPE_BODY)
                                    .text_color(color(tokens.color_text_muted))
                                    .child(strings.members().load_failed()),
                            )
                            .child(
                                Button::new("members-try-again", strings.common().try_again())
                                    .on_click(move |_, cx| {
                                        session.update(cx, |session, cx| {
                                            session.load_members(guild_id, true, cx);
                                        });
                                    }),
                            )
                            .into_any_element(),
                    ]
                } else if members.is_empty() {
                    vec![
                        with_type(div(), TYPE_BODY)
                            .id("members-loading")
                            .live_polite(strings.members().loading())
                            .px(length(SPACE_MD))
                            .pt(length(SPACE_XS))
                            .text_color(color(tokens.color_text_muted))
                            .child(strings.members().loading())
                            .into_any_element(),
                    ]
                } else {
                    let guild_id = guild.id;
                    let sorted = members_sorted_by_display_name(members);
                    let is_online = |user_id| state.presence_of(user_id) != PresenceStatus::Offline;
                    // Read out of the state first: drawing a row needs the context mutably.
                    let groups: Vec<_> = member_list_groups(&sorted, &guild.roles, is_online)
                        .into_iter()
                        .map(|group| {
                            let people: Vec<_> = group
                                .members
                                .iter()
                                .map(|member| {
                                    let user_id = member.user.id;
                                    (
                                        member.user.clone(),
                                        state.presence_of(user_id),
                                        state.custom_status_of(user_id).map(str::to_owned),
                                    )
                                })
                                .collect();
                            (group.kind, people)
                        })
                        .collect();
                    let mut rows = Vec::new();
                    for (kind, people) in groups {
                        // The id follows the group, not its heading: two roles may share a name.
                        let (heading, id) = match kind {
                            MemberListGroupKind::Role(role) => (
                                role.name.clone(),
                                ElementId::NamedInteger("member-group-role".into(), role.id.raw()),
                            ),
                            MemberListGroupKind::Online => (
                                strings.common().online().to_owned(),
                                ElementId::from("member-group-online"),
                            ),
                            MemberListGroupKind::Offline => (
                                strings.common().offline().to_owned(),
                                ElementId::from("member-group-offline"),
                            ),
                        };
                        let count = people.len();
                        // Sentence case, the count as its own number (ADR-0022 §3).
                        rows.push(
                            with_type(div(), TYPE_CATEGORY)
                                .id(id)
                                .role(Role::Heading)
                                .aria_level(3)
                                .aria_label(strings.members().group_label(
                                    &heading,
                                    i64::try_from(count).unwrap_or(i64::MAX),
                                ))
                                .h(length(COMPONENT_MEMBER_LIST_GROUP_HEIGHT))
                                .px(length(SPACE_MD))
                                .pb(length(SPACE_2XS))
                                .flex()
                                .items_end()
                                .justify_between()
                                .text_color(color(tokens.component_member_list_group_text))
                                .child(heading)
                                .child(count.to_string())
                                .into_any_element(),
                        );
                        for (user, presence, custom_status) in people {
                            let user = &user;
                            let name = display_name(user).to_owned();
                            let label = strings.members().member_label(
                                &name,
                                presence_name(strings, presence),
                                custom_status.as_deref(),
                            );
                            let row_id = ElementId::from(("member-row", user.id.raw()));
                            let focus_handle = remembered_focus_handle(&row_id, window, cx);
                            if self.member_profile.take_row_to_focus(user.id) {
                                let focus_handle = focus_handle.clone();
                                window.defer(cx, move |window, cx| window.focus(&focus_handle, cx));
                            }
                            let hover = remembered_hover(&row_id, window, cx);
                            let is_hovered = *hover.read(cx);
                            let is_ring_shown = is_focus_visible(&focus_handle, window);
                            let picture = user.avatar.map(|image| {
                                profile_picture(
                                    image,
                                    COMPONENT_MEMBER_ROW_AVATAR_SIZE,
                                    is_hovered || focus_handle.contains_focused(window, cx),
                                    window,
                                    cx,
                                )
                            });
                            let user_id = user.id;
                            let row_entity = entity.clone();
                            let row_bounds = remembered_bounds(&row_id, window, cx);
                            let open_menu = {
                                let entity = entity.clone();
                                let user = user.clone();
                                let opener = focus_handle.clone();
                                let row_bounds = row_bounds.clone();
                                // At the pointer, else beside the row (Shift+F10, the Menu key).
                                move |at: Option<gpui::Point<gpui::Pixels>>,
                                      window: &mut Window,
                                      cx: &mut gpui::App| {
                                    let origin = match at {
                                        Some(position) => Some(menu_origin_at_pointer(position)),
                                        None => menu_origin_beside_remembered(&row_bounds, cx),
                                    };
                                    let Some(origin) = origin else {
                                        return false;
                                    };
                                    let request = PersonMenuRequest {
                                        opener: Some(opener.clone()),
                                        profile_guild_id: Some(guild_id),
                                        ..PersonMenuRequest::new(user.clone(), origin)
                                    };
                                    entity.update(cx, |shell, cx| {
                                        shell.home.update(cx, |home, cx| {
                                            home.open_person_menu(request, window, cx);
                                        });
                                    });
                                    true
                                }
                            };
                            let open_menu_by_key = open_menu.clone();
                            let row = div()
                                .id(row_id)
                                .role(Role::Button)
                                .aria_label(label)
                                .track_focus(&focus_handle)
                                .relative()
                                .mx(length(COMPONENT_MEMBER_ROW_MARGIN_INLINE))
                                .mt(length(COMPONENT_MEMBER_ROW_GAP))
                                .h(length(COMPONENT_MEMBER_ROW_HEIGHT))
                                .px(length(COMPONENT_MEMBER_ROW_PADDING_INLINE))
                                .flex()
                                .items_center()
                                .gap(length(SPACE_SM))
                                .rounded(length(COMPONENT_MEMBER_ROW_RADIUS))
                                .when(is_hovered, |row| {
                                    row.bg(color(tokens.component_member_row_background_hover))
                                })
                                .cursor_pointer()
                                .on_hover(move |is_hovered, _, cx| {
                                    hover.update(cx, |remembered, cx| {
                                        *remembered = *is_hovered;
                                        cx.notify();
                                    });
                                })
                                .on_click(move |_, window, cx| {
                                    row_entity.update(cx, |shell, cx| {
                                        shell.show_member_profile(guild_id, user_id, window, cx);
                                    });
                                })
                                .on_mouse_down(MouseButton::Right, move |event, window, cx| {
                                    cx.stop_propagation();
                                    open_menu(Some(event.position), window, cx);
                                })
                                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                    if is_context_menu_keystroke(&event.keystroke)
                                        && open_menu_by_key(None, window, cx)
                                    {
                                        cx.stop_propagation();
                                    }
                                })
                                .child(bounds_recorder(row_bounds))
                                .child({
                                    let is_offline = presence == PresenceStatus::Offline;
                                    let avatar = Avatar::new(
                                        AvatarPerson::from_user(user, picture),
                                        COMPONENT_MEMBER_ROW_AVATAR_SIZE,
                                    )
                                    .dimmed(is_offline);
                                    // Offline rows show no shape (DESIGN.md §5.7): the dimmed
                                    // avatar and the Offline group say it.
                                    if is_offline {
                                        avatar
                                    } else {
                                        avatar.presence(
                                            shown_presence(presence),
                                            color(tokens.component_side_sheet_background),
                                        )
                                    }
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .child(
                                            with_type(div(), TYPE_MEMBER_NAME)
                                                .truncate()
                                                .text_color(color(if is_hovered {
                                                    tokens.component_member_row_name_text_hover
                                                } else {
                                                    tokens.component_member_row_name_text
                                                }))
                                                .child(name),
                                        )
                                        .when_some(custom_status, |lines, status| {
                                            lines.child(
                                                with_type(div(), TYPE_BODY_SM)
                                                    .truncate()
                                                    .text_color(color(
                                                        tokens.component_member_row_activity_text,
                                                    ))
                                                    .child(status),
                                            )
                                        }),
                                )
                                .when(is_ring_shown, |row| {
                                    row.child(inset_focus_ring(COMPONENT_MEMBER_ROW_RADIUS, cx))
                                });
                            // A list item holding its button, as the web's `<li><button>`.
                            rows.push(
                                div()
                                    .id(("member-item", user_id.raw()))
                                    .role(Role::ListItem)
                                    .child(row)
                                    .into_any_element(),
                            );
                        }
                    }
                    rows
                }
            }
        };

        div()
            .debug_selector(|| "side-sheet".to_owned())
            .id("side-sheet")
            .role(Role::Complementary)
            .aria_label(sheet_label)
            .track_focus(&self.member_sheet_focus)
            .h_full()
            .flex_none()
            .map(|sheet| match width {
                Some(width) => sheet.w(length(width)),
                None => sheet.w_full(),
            })
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(color(tokens.component_side_sheet_background))
            .rounded(length(COMPONENT_SIDE_SHEET_RADIUS))
            .child(
                div()
                    .flex_none()
                    .h(length(COMPONENT_CHAT_HEADER_HEIGHT))
                    .pl(length(SPACE_MD))
                    .pr(length(SPACE_2XS))
                    .flex()
                    .items_center()
                    .gap(length(SPACE_2XS))
                    .text_color(color(tokens.color_text_heading))
                    // The caret points back, towards the list.
                    .when(profile_user.is_some(), |head| {
                        head.child(
                            IconButton::new(
                                "back-to-members",
                                IconName::CaretLeft,
                                strings.members().back_to_members(),
                            )
                            .sizes(
                                COMPONENT_CHAT_HEADER_BUTTON_SIZE,
                                COMPONENT_CHAT_HEADER_ICON_SIZE,
                            )
                            .track_focus(self.member_profile.back_focus())
                            .on_click(move |_, cx| {
                                back_entity.update(cx, DuskShell::show_member_list);
                            }),
                        )
                    })
                    .child(
                        with_type(div(), TYPE_HEADING_MD)
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(heading),
                    )
                    .child(
                        IconButton::new(
                            "close-member-sheet",
                            IconName::X,
                            strings.common().close(),
                        )
                        .sizes(
                            COMPONENT_CHAT_HEADER_BUTTON_SIZE,
                            COMPONENT_CHAT_HEADER_ICON_SIZE,
                        )
                        .on_click(move |window, cx| {
                            entity.update(cx, |shell, cx| shell.close_member_sheet(window, cx));
                        }),
                    ),
            )
            .child(
                div()
                    .id("member-list")
                    // The profile replaces the list in the same scroller; it is no list.
                    .when(profile_user.is_none(), |list| list.role(Role::List))
                    .track_scroll(self.member_profile.list_scroll())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb(length(SPACE_MD))
                    .children(body),
            )
    }
}
