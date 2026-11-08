//! A group's members in the side sheet (`GroupMembersSheet.svelte`, DESIGN.md §5.6, §5.7,
//! ADR-0026): the group's actions as the server menu words them (Add friends in apricot, Leave
//! group in red, last), then everyone in it, you included, the owner said in words. The owner
//! removes others from their row. The shell draws the sheet's frame around these rows.

use gpui::{
    AnyElement, App, ElementId, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Role, StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::search_text::fold_for_search;
use mellow_protocol::{DirectMessageChannel, PresenceStatus, PublicUser};

use super::friends_pane::shown_presence;
use super::{Home, presence_name};
use crate::components::{
    Avatar, AvatarPerson, IconButton, IconButtonTone, icon, inset_focus_ring, is_focus_visible,
    remembered_focus_handle, remembered_hover, with_type,
};
use crate::i18n::strings;
use crate::session::Session;
use crate::shell::profile_picture;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CONTEXT_MENU_ITEM_HEIGHT, COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE,
    COMPONENT_FRIENDS_ACTION_ICON_SIZE, COMPONENT_FRIENDS_ACTION_SIZE,
    COMPONENT_MEMBER_LIST_GROUP_HEIGHT, COMPONENT_MEMBER_ROW_AVATAR_SIZE, COMPONENT_MEMBER_ROW_GAP,
    COMPONENT_MEMBER_ROW_HEIGHT, COMPONENT_MEMBER_ROW_MARGIN_INLINE,
    COMPONENT_MEMBER_ROW_PADDING_INLINE, COMPONENT_MEMBER_ROW_RADIUS, SPACE_2XS, SPACE_MD,
    SPACE_SM, SPACE_XS, TYPE_BODY_SM, TYPE_CATEGORY, TYPE_MEMBER_NAME, TYPE_MENU_ITEM,
};
use crate::theme::{color, length, theme};

/// How a group action reads (`groupAction`'s tone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Accent,
    Normal,
    Danger,
}

/// Everyone in the group, you included, by name.
fn people_of(group: &DirectMessageChannel, current_user: Option<PublicUser>) -> Vec<PublicUser> {
    let current_user_id = current_user.as_ref().map(|user| user.id);
    let mut everyone: Vec<PublicUser> = current_user
        .into_iter()
        .chain(
            group
                .recipients
                .iter()
                .filter(|recipient| Some(recipient.id) != current_user_id)
                .cloned(),
        )
        .collect();
    everyone.sort_by_cached_key(|person| fold_for_search(display_name(person)));
    everyone
}

/// The sheet's rows for `group`.
#[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
pub fn group_sheet_rows(
    home: &Entity<Home>,
    session: &Entity<Session>,
    group: &DirectMessageChannel,
    window: &mut Window,
    cx: &mut App,
) -> Vec<AnyElement> {
    let tokens = theme(cx);
    let catalogue = strings(cx);
    let words = catalogue.groups();
    let group_id = group.id;
    let mut rows = Vec::new();
    for (id, label, icon_name, tone) in [
        (
            "group-add-friends",
            words.add_friends(),
            IconName::UserPlus,
            Tone::Accent,
        ),
        (
            "group-rename",
            words.rename(),
            IconName::PencilSimple,
            Tone::Normal,
        ),
        (
            "group-leave",
            words.leave(),
            IconName::SignOut,
            Tone::Danger,
        ),
    ] {
        let id = ElementId::from(id);
        let focus = remembered_focus_handle(&id, window, cx);
        let hover = remembered_hover(&id, window, cx);
        let is_hovered = *hover.read(cx);
        let ink = match tone {
            Tone::Accent => tokens.component_context_menu_accent_text,
            Tone::Danger => tokens.component_context_menu_danger_text,
            Tone::Normal if is_hovered => tokens.component_member_row_name_text_hover,
            Tone::Normal => tokens.component_member_row_name_text,
        };
        let is_ring_shown = is_focus_visible(&focus, window);
        let home = home.clone();
        rows.push(
            div()
                .mx(length(COMPONENT_MEMBER_ROW_MARGIN_INLINE))
                .mt(length(COMPONENT_MEMBER_ROW_GAP))
                .child(
                    with_type(div(), TYPE_MENU_ITEM)
                        .id(id)
                        .role(Role::Button)
                        .aria_label(label)
                        .track_focus(&focus)
                        .relative()
                        .h(length(COMPONENT_CONTEXT_MENU_ITEM_HEIGHT))
                        .px(length(COMPONENT_MEMBER_ROW_PADDING_INLINE))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(length(SPACE_XS))
                        .rounded(length(COMPONENT_MEMBER_ROW_RADIUS))
                        .when(is_hovered, |action| {
                            action.bg(color(tokens.component_member_row_background_hover))
                        })
                        .text_color(color(ink))
                        .cursor_pointer()
                        .on_hover(move |is_hovered, _, cx| {
                            hover.update(cx, |remembered, cx| {
                                *remembered = *is_hovered;
                                cx.notify();
                            });
                        })
                        .on_click(move |_, window, cx| {
                            home.update(cx, |home, cx| match tone {
                                Tone::Accent => home.open_people_picker(Some(group_id), window, cx),
                                Tone::Normal => home.ask_to_rename_group(group_id, window, cx),
                                Tone::Danger => home.ask_to_leave_group(group_id, window, cx),
                            });
                        })
                        .child(label)
                        .child(icon(
                            icon_name,
                            COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE,
                            color(ink),
                        ))
                        .when(is_ring_shown, |action| {
                            action.child(inset_focus_ring(COMPONENT_MEMBER_ROW_RADIUS, cx))
                        }),
                )
                .into_any_element(),
        );
    }

    let state = session.read(cx).state();
    let current_user = state.current_user().map(|user| PublicUser {
        id: user.id,
        username: user.username.clone(),
        display_name: user.display_name.clone(),
        avatar: user.avatar,
        banner: user.banner,
    });
    let current_user_id = current_user.as_ref().map(|user| user.id);
    let is_owner = current_user_id.is_some() && group.owner_id == current_user_id;
    let people = people_of(group, current_user);
    let count = people.len();
    // Sentence case, the count as its own number, as in the member list (ADR-0022 §3).
    rows.push(
        with_type(div(), TYPE_CATEGORY)
            .id("group-members-heading")
            .role(Role::Heading)
            .aria_level(3)
            .aria_label(
                catalogue
                    .members()
                    .group_label(words.members(), i64::try_from(count).unwrap_or(i64::MAX)),
            )
            .h(length(COMPONENT_MEMBER_LIST_GROUP_HEIGHT))
            .px(length(SPACE_MD))
            .pb(length(SPACE_2XS))
            .flex()
            .items_end()
            .justify_between()
            .text_color(color(tokens.component_member_list_group_text))
            .child(words.members())
            .child(count.to_string())
            .into_any_element(),
    );
    for person in people {
        let presence = state.presence_of(person.id);
        let is_online = presence != PresenceStatus::Offline;
        let is_person_owner = Some(person.id) == group.owner_id;
        let custom_status = state.custom_status_of(person.id).map(str::to_owned);
        let name = display_name(&person).to_owned();
        let label = words.member_label(
            &name,
            presence_name(catalogue, presence),
            is_person_owner,
            custom_status.as_deref(),
        );
        let second_line = if is_person_owner {
            Some(words.owner().to_owned())
        } else {
            custom_status
        };
        let picture = person
            .avatar
            .map(|image| profile_picture(image, COMPONENT_MEMBER_ROW_AVATAR_SIZE, window));
        let mut avatar = Avatar::new(
            AvatarPerson::from_user(&person, picture),
            COMPONENT_MEMBER_ROW_AVATAR_SIZE,
        );
        if is_online {
            avatar = avatar.presence(
                shown_presence(presence),
                color(tokens.component_side_sheet_background),
            );
        }
        let can_remove = is_owner && Some(person.id) != current_user_id;
        let remove = can_remove.then(|| {
            let home = home.clone();
            let person = person.clone();
            IconButton::new(
                ElementId::from(("group-remove", person.id.raw())),
                IconName::UserMinus,
                words.remove(&name),
            )
            .sizes(
                COMPONENT_FRIENDS_ACTION_SIZE,
                COMPONENT_FRIENDS_ACTION_ICON_SIZE,
            )
            .tone(IconButtonTone::Danger)
            .on_click(move |window, cx| {
                home.update(cx, |home, cx| {
                    home.ask_to_remove_from_group(group_id, person.clone(), window, cx);
                });
            })
        });
        rows.push(
            div()
                .id(ElementId::from(("group-member", person.id.raw())))
                .role(Role::ListItem)
                // One sentence for screen readers; the owner is said in words, never by a mark.
                .aria_label(label)
                .mx(length(COMPONENT_MEMBER_ROW_MARGIN_INLINE))
                .mt(length(COMPONENT_MEMBER_ROW_GAP))
                .min_h(length(COMPONENT_MEMBER_ROW_HEIGHT))
                .px(length(COMPONENT_MEMBER_ROW_PADDING_INLINE))
                .flex()
                .items_center()
                .gap(length(SPACE_SM))
                .child(avatar.dimmed(!is_online))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            with_type(div(), TYPE_MEMBER_NAME)
                                .truncate()
                                .text_color(color(tokens.component_member_row_name_text))
                                .child(name),
                        )
                        .when_some(second_line, |lines, text| {
                            lines.child(
                                with_type(div(), TYPE_BODY_SM)
                                    .truncate()
                                    .text_color(color(tokens.component_member_row_activity_text))
                                    .child(text),
                            )
                        }),
                )
                .children(remove)
                .into_any_element(),
        );
    }
    rows
}
