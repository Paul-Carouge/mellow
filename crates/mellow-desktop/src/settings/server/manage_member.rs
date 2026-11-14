//! What a moderator may do to one member (`ManageMemberDialog.svelte`, DESIGN.md §5.8, §8): the
//! body of the Members page's "Manage" dialog. Roles as switches, applied at once as on Discord
//! (locked roles say why), then Kick and Ban, which the page confirms in their own dialog. Only
//! client-core's `member_actions` and `can_manage_role` decide what is offered.

use gpui::{
    App, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Role as AccessRole, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::server_settings::{
    can_manage_role, member_actions, role_manager_of, roles_highest_first,
};
use mellow_protocol::{Member, Role, Snowflake};

use crate::components::{Button, ButtonVariant, Switch, icon, with_type};
use crate::i18n::strings;
use crate::session::Session;
use crate::settings::error_alert;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_DIALOG_BUTTON_GAP, COMPONENT_DIALOG_SECTION_GAP, COMPONENT_ROLES_ROW_DOT_SIZE,
    COMPONENT_ROLES_ROW_ICON_GAP, COMPONENT_ROLES_ROW_LOCK_ICON_SIZE,
    COMPONENT_SETTINGS_SECTION_LABEL_GAP, SIZE_HIT_WEB_MIN, TYPE_BODY_SM, TYPE_CATEGORY,
    TYPE_CHANNEL_NAME,
};
use crate::theme::{color, length, theme};

/// Kicking or banning, confirmed by the page in a dialog of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberRemoval {
    Kick,
    Ban,
}

/// The body asks the page to confirm a removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmMemberRemoval(pub MemberRemoval);

/// A role switch the moderator turned, shown until the gateway brings the member's new roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AskedRole {
    role_id: Snowflake,
    is_given: bool,
}

pub struct ManageMemberBody {
    session: Entity<Session>,
    guild_id: Snowflake,
    user_id: Snowflake,
    busy_role_id: Option<Snowflake>,
    asked_role: Option<AskedRole>,
    role_error: Option<SharedString>,
}

impl EventEmitter<ConfirmMemberRemoval> for ManageMemberBody {}

impl std::fmt::Debug for ManageMemberBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManageMemberBody")
            .field("user_id", &self.user_id)
            .field("busy_role_id", &self.busy_role_id)
            .finish_non_exhaustive()
    }
}

impl ManageMemberBody {
    pub fn new(
        session: Entity<Session>,
        guild_id: Snowflake,
        user_id: Snowflake,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&session, |body, _, cx| body.follow_member_roles(cx))
            .detach();
        Self {
            session,
            guild_id,
            user_id,
            busy_role_id: None,
            asked_role: None,
            role_error: None,
        }
    }

    fn member(&self, cx: &App) -> Option<Member> {
        self.session
            .read(cx)
            .state()
            .members(self.guild_id)
            .iter()
            .find(|member| member.user.id == self.user_id)
            .cloned()
    }

    /// Once the member's roles show the change, they are the truth again (another moderator may
    /// change them next).
    fn follow_member_roles(&mut self, cx: &mut Context<Self>) {
        if let (Some(asked), Some(member)) = (self.asked_role, self.member(cx))
            && member.role_ids.contains(&asked.role_id) == asked.is_given
        {
            self.asked_role = None;
        }
        cx.notify();
    }

    fn holds_role(&self, member: &Member, role: &Role) -> bool {
        match self.asked_role {
            Some(asked) if asked.role_id == role.id => asked.is_given,
            _ => member.role_ids.contains(&role.id),
        }
    }

    fn set_role(&mut self, role_id: Snowflake, is_given: bool, cx: &mut Context<Self>) {
        if self.busy_role_id.is_some() {
            return;
        }
        self.asked_role = None;
        self.busy_role_id = Some(role_id);
        self.role_error = None;
        cx.notify();
        let (guild_id, user_id) = (self.guild_id, self.user_id);
        let body = cx.entity().downgrade();
        let failure = strings(cx)
            .server_settings()
            .members()
            .role_failed()
            .to_owned();
        self.session.update(cx, |session, cx| {
            session.send_settings_change(
                if is_given {
                    "give member role"
                } else {
                    "take member role"
                },
                failure,
                move |rest| async move {
                    if is_given {
                        rest.add_member_role(guild_id, user_id, role_id).await
                    } else {
                        rest.remove_member_role(guild_id, user_id, role_id).await
                    }
                },
                cx,
                move |outcome, cx| {
                    let _ = body.update(cx, |body, cx| {
                        body.busy_role_id = None;
                        match outcome {
                            Ok(()) => {
                                body.asked_role = Some(AskedRole { role_id, is_given });
                                body.follow_member_roles(cx);
                            }
                            Err(message) => body.role_error = Some(message.into()),
                        }
                        cx.notify();
                    });
                },
            );
        });
    }
}

impl Render for ManageMemberBody {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.session.read(cx).state();
        let me = state.current_user().map(|user| user.id);
        let (Some(guild), Some(member)) = (state.guild(self.guild_id).cloned(), self.member(cx))
        else {
            return div().into_any_element();
        };
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.server_settings().members();
        let name = display_name(&member.user).to_owned();
        let manager = role_manager_of(&guild, me);
        let actions = member_actions(&guild, &manager, &member, me);
        let named_roles: Vec<&Role> = roles_highest_first(&guild)
            .into_iter()
            .filter(|role| role.id != guild.id)
            .collect();
        let has_locked_role = named_roles
            .iter()
            .any(|role| !can_manage_role(&manager, role, guild.id));
        let locked_note = catalogue.server_settings().roles().locked();
        let body = cx.entity().downgrade();
        let role_rows: Vec<_> = named_roles
            .iter()
            .map(|role| {
                let is_locked = !can_manage_role(&manager, role, guild.id);
                let is_on = self.holds_role(&member, role);
                let role_id = role.id;
                let body = body.clone();
                div()
                    .flex()
                    .items_center()
                    .min_h(length(SIZE_HIT_WEB_MIN))
                    .gap(length(COMPONENT_ROLES_ROW_ICON_GAP))
                    .border_b_1()
                    .border_color(color(tokens.component_server_settings_row_divider))
                    .child(super::roles::role_dot(
                        role.color,
                        COMPONENT_ROLES_ROW_DOT_SIZE,
                        cx,
                    ))
                    .child(
                        with_type(div(), TYPE_CHANNEL_NAME)
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_color(color(tokens.color_text_normal))
                            .child(role.name.clone()),
                    )
                    .when(is_locked, |row| {
                        row.child(icon(
                            IconName::Lock,
                            COMPONENT_ROLES_ROW_LOCK_ICON_SIZE,
                            color(tokens.component_roles_row_lock_icon),
                        ))
                    })
                    .child({
                        let switch =
                            Switch::new(("member-role", role.id.raw()), is_on, role.name.clone())
                                .control_only()
                                .locked(is_locked || self.busy_role_id.is_some())
                                .on_change(move |is_given, _, cx| {
                                    let _ = body.update(cx, |body, cx| {
                                        body.set_role(role_id, is_given, cx);
                                    });
                                });
                        if is_locked {
                            switch.description(locked_note)
                        } else {
                            switch
                        }
                    })
                    .into_any_element()
            })
            .collect();
        let kick_body = body.clone();
        let ban_body = body;
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_DIALOG_SECTION_GAP))
            .when(actions.can_give_roles, |column| {
                column.child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            with_type(div(), TYPE_CATEGORY)
                                .id("manage-member-roles")
                                .role(AccessRole::Heading)
                                .text_color(color(tokens.component_settings_section_label_text))
                                .child(catalogue.server_settings().sections().roles()),
                        )
                        .children(role_rows)
                        .when(has_locked_role, |section| {
                            section.child(
                                with_type(div(), TYPE_BODY_SM)
                                    .mt(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                                    .text_color(color(tokens.component_roles_note_text))
                                    .child(locked_note),
                            )
                        })
                        .child(
                            div()
                                .when(self.role_error.is_some(), |alert| {
                                    alert.mt(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                                })
                                .child(error_alert(
                                    "manage-member-role-error",
                                    self.role_error.clone(),
                                    cx,
                                )),
                        ),
                )
            })
            .when(actions.can_kick || actions.can_ban, |column| {
                column.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(length(COMPONENT_DIALOG_BUTTON_GAP))
                        .when(actions.can_kick, |buttons| {
                            buttons.child(
                                Button::new("manage-member-kick", words.kick_label(&name))
                                    .variant(ButtonVariant::Danger)
                                    .on_click(move |_, cx| {
                                        let _ = kick_body.update(cx, |_, cx| {
                                            cx.emit(ConfirmMemberRemoval(MemberRemoval::Kick));
                                        });
                                    }),
                            )
                        })
                        .when(actions.can_ban, |buttons| {
                            buttons.child(
                                Button::new("manage-member-ban", words.ban_label(&name))
                                    .variant(ButtonVariant::Danger)
                                    .on_click(move |_, cx| {
                                        let _ = ban_body.update(cx, |_, cx| {
                                            cx.emit(ConfirmMemberRemoval(MemberRemoval::Ban));
                                        });
                                    }),
                            )
                        }),
                )
            })
            .into_any_element()
    }
}
