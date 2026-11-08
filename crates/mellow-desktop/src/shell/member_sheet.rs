//! The member side sheet (`MemberList.svelte`, DESIGN.md §5.7, ADR-0022 §1): roles displayed
//! separately first, then Online and Offline, by name; offline avatars dimmed while names keep
//! full contrast. Docked beside the conversation where it fits, over it otherwise.
//! shortcut: choosing a member shows their profile in the sheet on the web; profiles come with
//! M3 step 4, so rows are read, not pressed, until then.

use gpui::{
    AnyElement, Context, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::member_order::{
    MemberListGroupKind, member_list_groups, members_sorted_by_display_name,
};
use mellow_protocol::PresenceStatus;

use super::{DuskShell, profile_picture};
use crate::components::{Avatar, AvatarPerson, Button, IconButton, with_type};
use crate::home::group_sheet::group_sheet_rows;
use crate::home::{conversation_display_name, presence_name, shown_presence};
use crate::i18n::strings;
use crate::session::MembersLoad;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CHAT_HEADER_BUTTON_SIZE, COMPONENT_CHAT_HEADER_HEIGHT,
    COMPONENT_CHAT_HEADER_ICON_SIZE, COMPONENT_MEMBER_LIST_GROUP_HEIGHT,
    COMPONENT_MEMBER_ROW_AVATAR_SIZE, COMPONENT_MEMBER_ROW_GAP, COMPONENT_MEMBER_ROW_HEIGHT,
    COMPONENT_MEMBER_ROW_MARGIN_INLINE, COMPONENT_MEMBER_ROW_PADDING_INLINE,
    COMPONENT_SIDE_SHEET_RADIUS, SPACE_2XS, SPACE_MD, SPACE_SM, SPACE_XS, TYPE_BODY, TYPE_BODY_SM,
    TYPE_CATEGORY, TYPE_HEADING_MD, TYPE_MEMBER_NAME,
};
use crate::theme::{color, length, theme};

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
        let sheet_label = self.member_sheet_label(cx);
        let group = self
            .member_sheet_place_id(cx)
            .and_then(|group_id| state.direct_message_channel(group_id))
            .filter(|_| guild.is_none())
            .cloned();
        let heading = if group.is_some() {
            strings.groups().members()
        } else {
            strings.members().members()
        };
        let entity = cx.entity();
        let body: Vec<AnyElement> = match &guild {
            None => group
                .map(|group| group_sheet_rows(&self.home, &self.session, &group, window, cx))
                .unwrap_or_default(),
            Some(guild) => {
                let members = state.members(guild.id);
                let load = self.session.read(cx).members_load(guild.id);
                if members.is_empty() && load == Some(MembersLoad::Failed) {
                    let guild_id = guild.id;
                    let session = self.session.clone();
                    vec![
                        div()
                            .id("members-failed")
                            .role(Role::Alert)
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
                            .role(Role::Status)
                            .px(length(SPACE_MD))
                            .pt(length(SPACE_XS))
                            .text_color(color(tokens.color_text_muted))
                            .child(strings.members().loading())
                            .into_any_element(),
                    ]
                } else {
                    let sorted = members_sorted_by_display_name(members);
                    let is_online = |user_id| state.presence_of(user_id) != PresenceStatus::Offline;
                    let groups = member_list_groups(&sorted, &guild.roles, is_online);
                    let mut rows = Vec::new();
                    for group in groups {
                        // The id follows the group, not its heading: two roles may share a name.
                        let (heading, id) = match group.kind {
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
                        let count = group.members.len();
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
                        for member in group.members {
                            let user = &member.user;
                            let presence = state.presence_of(user.id);
                            let custom_status = state.custom_status_of(user.id).map(str::to_owned);
                            let name = display_name(user).to_owned();
                            let label = strings.members().member_label(
                                &name,
                                presence_name(strings, presence),
                                custom_status.as_deref(),
                            );
                            let picture = user.avatar.map(|image| {
                                profile_picture(image, COMPONENT_MEMBER_ROW_AVATAR_SIZE, window)
                            });
                            rows.push(
                                div()
                                    .id(("member-row", user.id.raw()))
                                    .role(Role::ListItem)
                                    .aria_label(label)
                                    .mx(length(COMPONENT_MEMBER_ROW_MARGIN_INLINE))
                                    .mt(length(COMPONENT_MEMBER_ROW_GAP))
                                    .h(length(COMPONENT_MEMBER_ROW_HEIGHT))
                                    .px(length(COMPONENT_MEMBER_ROW_PADDING_INLINE))
                                    .flex()
                                    .items_center()
                                    .gap(length(SPACE_SM))
                                    .child(
                                        Avatar::new(
                                            AvatarPerson::from_user(user, picture),
                                            COMPONENT_MEMBER_ROW_AVATAR_SIZE,
                                        )
                                        .presence(
                                            shown_presence(presence),
                                            color(tokens.component_side_sheet_background),
                                        )
                                        .dimmed(presence == PresenceStatus::Offline),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .child(
                                                with_type(div(), TYPE_MEMBER_NAME)
                                                    .truncate()
                                                    .text_color(color(
                                                        tokens.component_member_row_name_text,
                                                    ))
                                                    .child(name),
                                            )
                                            .when_some(custom_status, |lines, status| {
                                                lines.child(
                                                    with_type(div(), TYPE_BODY_SM)
                                                        .truncate()
                                                        .text_color(color(
                                                            tokens
                                                                .component_member_row_activity_text,
                                                        ))
                                                        .child(status),
                                                )
                                            }),
                                    )
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
                    .role(Role::List)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb(length(SPACE_MD))
                    .children(body),
            )
    }
}
