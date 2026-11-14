//! Members (`MemberSettings.svelte`, DESIGN.md §5.8): everyone in the server, searchable, with
//! their roles. "Manage" opens what the reader may do to that member (give or take roles, kick,
//! ban: `manage_member`); kicks and bans are confirmed in their own dialog, a ban with an optional
//! reason. The list is the one the member sheet uses, kept current by the gateway.

use gpui::{
    App, AppContext as _, Context, ElementId, Entity, EventEmitter, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Role as AccessRole, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::input::{InputEvent, InputState};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::member_order::members_sorted_by_display_name;
use mellow_client_core::shared_logic::server_settings::{
    MAXIMUM_BAN_REASON_LENGTH, matches_member_search, member_actions, role_manager_of,
};
use mellow_protocol::{CreateBanRequest, Member, Snowflake};

use super::manage_member::{ConfirmMemberRemoval, ManageMemberBody, MemberRemoval};
use crate::components::{
    Avatar, AvatarPerson, Button, LiveRegion as _, ModalDialog, ModalDialogEvent, RoleChip,
    RoleChips, SettingsListStatus, SettingsListStatusKind, TextField, is_picture_row_pointed_at,
    track_picture_row_pointer, with_type,
};
use crate::i18n::strings;
use crate::session::{MembersLoad, Session};
use crate::settings::{SettingsPage, SettingsPageEvent};
use crate::shell::profile_picture;
use crate::theme::tokens::{
    COMPONENT_DIALOG_WIDTH, COMPONENT_SERVER_SETTINGS_AVATAR_SIZE,
    COMPONENT_SERVER_SETTINGS_ROW_GAP, COMPONENT_SERVER_SETTINGS_ROW_MIN_HEIGHT,
    COMPONENT_SERVER_SETTINGS_ROW_PADDING_BLOCK, COMPONENT_SERVER_SETTINGS_TEXT_GAP,
    COMPONENT_SERVER_SETTINGS_TOOLBAR_GAP, TYPE_BODY, TYPE_MEMBER_NAME,
};
use crate::theme::{color, length, theme};

struct ManagedMember {
    user_id: Snowflake,
    dialog: Entity<ModalDialog>,
    _subscriptions: Vec<Subscription>,
}

struct RemovalConfirmation {
    removal: MemberRemoval,
    dialog: Entity<ModalDialog>,
    _subscriptions: Vec<Subscription>,
}

pub struct MembersPage {
    session: Entity<Session>,
    guild_id: Snowflake,
    search_input: Entity<InputState>,
    notice: Option<SharedString>,
    managed: Option<ManagedMember>,
    confirmation: Option<RemovalConfirmation>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsPageEvent> for MembersPage {}

/// No form: nothing waits to be saved, so there is no bar to shake.
impl SettingsPage for MembersPage {
    fn set_attention(&mut self, _attention: u32, _cx: &mut Context<Self>) {}
}

impl std::fmt::Debug for MembersPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MembersPage")
            .field("guild_id", &self.guild_id)
            .field(
                "managed",
                &self.managed.as_ref().map(|managed| managed.user_id),
            )
            .finish_non_exhaustive()
    }
}

impl MembersPage {
    pub fn new(
        session: Entity<Session>,
        guild_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![
            cx.subscribe(&search_input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.observe_in(&session, window, |page, _, window, cx| {
                page.close_dialogs_of_a_member_gone(window, cx);
                cx.notify();
            }),
        ];
        session.update(cx, |session, cx| session.load_members(guild_id, false, cx));
        Self {
            session,
            guild_id,
            search_input,
            notice: None,
            managed: None,
            confirmation: None,
            _subscriptions: subscriptions,
        }
    }

    fn member(&self, user_id: Snowflake, cx: &App) -> Option<Member> {
        self.session
            .read(cx)
            .state()
            .members(self.guild_id)
            .iter()
            .find(|member| member.user.id == user_id)
            .cloned()
    }

    /// A member who left, was removed by another moderator, or the list reloading without them:
    /// their dialogs close and focus goes to the search, the buttons that opened them being gone.
    fn close_dialogs_of_a_member_gone(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(user_id) = self.managed.as_ref().map(|managed| managed.user_id) else {
            return;
        };
        let is_removing = self
            .confirmation
            .as_ref()
            .is_some_and(|confirmation| confirmation.dialog.read(cx).is_busy());
        if is_removing {
            return;
        }
        let Some(member) = self.member(user_id, cx) else {
            self.close_member_dialogs(window, cx);
            return;
        };
        // The reader lost the permission, or the member now outranks them: the confirmation no
        // longer offers what the instance would refuse.
        let removal = self
            .confirmation
            .as_ref()
            .map(|confirmation| confirmation.removal);
        if let Some(removal) = removal
            && !self.may_remove(&member, removal, cx)
        {
            self.confirmation = None;
            cx.notify();
        }
    }

    fn may_remove(&self, member: &Member, removal: MemberRemoval, cx: &App) -> bool {
        let state = self.session.read(cx).state();
        let me = state.current_user().map(|user| user.id);
        state.guild(self.guild_id).is_some_and(|guild| {
            let actions = member_actions(guild, &role_manager_of(guild, me), member, me);
            match removal {
                MemberRemoval::Kick => actions.can_kick,
                MemberRemoval::Ban => actions.can_ban,
            }
        })
    }

    fn close_member_dialogs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirmation = None;
        self.managed = None;
        self.search_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn open_manage_dialog(
        &mut self,
        user_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(member) = self.member(user_id, cx) else {
            return;
        };
        let catalogue = strings(cx);
        let session = self.session.clone();
        let guild_id = self.guild_id;
        let body = cx.new(|cx| ManageMemberBody::new(session, guild_id, user_id, cx));
        let dialog = cx.new(|cx| {
            ModalDialog::new(
                display_name(&member.user).to_owned(),
                catalogue.common().close(),
                window,
                cx,
            )
            .body(body.clone().into())
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &dialog,
                window,
                |page, _, event: &ModalDialogEvent, _, cx| {
                    if *event == ModalDialogEvent::Closed {
                        page.managed = None;
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(
                &body,
                window,
                move |page, _, event: &ConfirmMemberRemoval, window, cx| {
                    page.open_removal_confirmation(user_id, event.0, window, cx);
                },
            ),
        ];
        self.managed = Some(ManagedMember {
            user_id,
            dialog,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    fn open_removal_confirmation(
        &mut self,
        user_id: Snowflake,
        removal: MemberRemoval,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(member), Some(guild_name)) = (
            self.member(user_id, cx),
            self.session
                .read(cx)
                .state()
                .guild(self.guild_id)
                .map(|guild| guild.name.clone()),
        ) else {
            return;
        };
        let catalogue = strings(cx);
        let words = catalogue.server_settings().members();
        let name = display_name(&member.user).to_owned();
        let cancel = catalogue.common().cancel();
        let reason_input = (removal == MemberRemoval::Ban).then(|| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .validate(|text, _| text.chars().count() <= MAXIMUM_BAN_REASON_LENGTH)
            })
        });
        let dialog = cx.new(|cx| {
            let dialog = match removal {
                MemberRemoval::Kick => {
                    ModalDialog::new(words.kick_title(&name, &guild_name), cancel, window, cx)
                        .subtitle(words.kick_body(&name))
                        .submit(words.kick_label(&name), words.kicking(), true)
                }
                MemberRemoval::Ban => {
                    ModalDialog::new(words.ban_title(&name, &guild_name), cancel, window, cx)
                        .subtitle(words.ban_body(&name))
                        .submit(words.ban_label(&name), words.banning(), true)
                }
            };
            match &reason_input {
                Some(input) => dialog.body(
                    cx.new(|_| BanReasonBody {
                        input: input.clone(),
                        label: words.ban_reason().into(),
                    })
                    .into(),
                ),
                None => dialog,
            }
        });
        let enter_input = reason_input.clone();
        let mut subscriptions = vec![cx.subscribe_in(
            &dialog,
            window,
            move |page, dialog, event: &ModalDialogEvent, window, cx| match event {
                ModalDialogEvent::Closed => {
                    page.confirmation = None;
                    cx.notify();
                }
                ModalDialogEvent::Submitted => {
                    let reason = reason_input
                        .as_ref()
                        .map(|input| input.read(cx).value().trim().to_owned());
                    page.remove(user_id, removal, reason, dialog, window, cx);
                }
            },
        )];
        if let Some(input) = enter_input {
            let dialog = dialog.downgrade();
            subscriptions.push(cx.subscribe(&input, move |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let _ = dialog.update(cx, ModalDialog::submit_now);
                }
            }));
        }
        self.confirmation = Some(RemovalConfirmation {
            removal,
            dialog,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    fn remove(
        &mut self,
        user_id: Snowflake,
        removal: MemberRemoval,
        reason: Option<String>,
        dialog: &Entity<ModalDialog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(member) = self.member(user_id, cx) else {
            return;
        };
        // Read before the member leaves the list, and the dialogs with them.
        let name = display_name(&member.user).to_owned();
        let words = strings(cx).server_settings().members();
        let (done, failure) = match removal {
            MemberRemoval::Kick => (words.kicked(&name), words.kick_failed(&name)),
            MemberRemoval::Ban => (words.banned(&name), words.ban_failed(&name)),
        };
        dialog.update(cx, |dialog, cx| {
            dialog.set_error(None, cx);
            dialog.set_busy(true, cx);
        });
        let guild_id = self.guild_id;
        let page = cx.entity().downgrade();
        let dialog = dialog.downgrade();
        let window_handle = window.window_handle();
        self.session.update(cx, |session, cx| {
            session.send_settings_change(
                match removal {
                    MemberRemoval::Kick => "kick member",
                    MemberRemoval::Ban => "ban member",
                },
                failure,
                move |rest| async move {
                    match removal {
                        MemberRemoval::Kick => rest.kick_member(guild_id, user_id).await,
                        MemberRemoval::Ban => {
                            let request = CreateBanRequest {
                                reason: reason.unwrap_or_default(),
                            };
                            rest.ban_member(guild_id, user_id, &request).await
                        }
                    }
                },
                cx,
                move |outcome, cx| {
                    let _ = window_handle.update(cx, |_, window, cx| match outcome {
                        Ok(()) => {
                            let _ = page.update(cx, |page, cx| {
                                page.notice = Some(done.into());
                                page.close_member_dialogs(window, cx);
                            });
                        }
                        Err(message) => {
                            let _ = dialog.update(cx, |dialog, cx| {
                                dialog.set_busy(false, cx);
                                dialog.set_error(Some(message.into()), cx);
                            });
                        }
                    });
                },
            );
        });
    }

    fn member_row(
        &self,
        member: &Member,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tokens = theme(cx);
        let state = self.session.read(cx).state();
        let me = state.current_user().map(|user| user.id);
        let catalogue = strings(cx);
        let words = catalogue.server_settings().members();
        let name = display_name(&member.user).to_owned();
        let (can_manage, chips) = state
            .guild(self.guild_id)
            .map_or((false, Vec::new()), |guild| {
                let manager = role_manager_of(guild, me);
                let mut roles: Vec<_> = guild
                    .roles
                    .iter()
                    .filter(|role| member.role_ids.contains(&role.id))
                    .collect();
                roles.sort_by_key(|role| std::cmp::Reverse(role.position));
                let owner = (member.user.id == guild.owner_id).then(|| RoleChip::Owner {
                    label: words.owner().into(),
                });
                let chips = owner
                    .into_iter()
                    .chain(roles.into_iter().map(|role| RoleChip::Role {
                        name: role.name.clone().into(),
                        colour: role.color,
                    }))
                    .collect();
                (member_actions(guild, &manager, member, me).has_any(), chips)
            });
        let row_id = ElementId::from(("server-member-row", member.user.id.raw()));
        let is_row_pointed_at = is_picture_row_pointed_at(&row_id, cx);
        let picture = member.user.avatar.map(|image| {
            profile_picture(
                image,
                COMPONENT_SERVER_SETTINGS_AVATAR_SIZE,
                is_row_pointed_at,
                window,
                cx,
            )
        });
        let user_id = member.user.id;
        let page = cx.entity().downgrade();
        track_picture_row_pointer(div().id(row_id.clone()), row_id, window)
            .flex()
            .items_center()
            .min_h(length(COMPONENT_SERVER_SETTINGS_ROW_MIN_HEIGHT))
            .gap(length(COMPONENT_SERVER_SETTINGS_ROW_GAP))
            .py(length(COMPONENT_SERVER_SETTINGS_ROW_PADDING_BLOCK))
            .border_b_1()
            .border_color(color(tokens.component_server_settings_row_divider))
            .child(Avatar::new(
                AvatarPerson::from_user(&member.user, picture),
                COMPONENT_SERVER_SETTINGS_AVATAR_SIZE,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .flex_1()
                    .gap(length(COMPONENT_SERVER_SETTINGS_TEXT_GAP))
                    .child(
                        with_type(div(), TYPE_MEMBER_NAME)
                            .truncate()
                            .text_color(color(tokens.color_text_heading))
                            .child(name.clone()),
                    )
                    .when(!chips.is_empty(), |text| {
                        text.child(RoleChips::new(words.member_roles(&name), chips))
                    }),
            )
            .when(can_manage, |row| {
                row.child(
                    Button::new(("manage-member", user_id.raw()), words.manage())
                        .accessible_label(words.manage_label(&name))
                        .on_click(move |window, cx| {
                            let _ = page.update(cx, |page, cx| {
                                page.open_manage_dialog(user_id, window, cx);
                            });
                        }),
                )
            })
    }
}

/// The ban dialog's reason field (`ManageMemberDialog.svelte`).
struct BanReasonBody {
    input: Entity<InputState>,
    label: SharedString,
}

impl Render for BanReasonBody {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        TextField::new("ban-reason", self.label.clone(), &self.input)
    }
}

impl Render for MembersPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.server_settings().members();
        let session = self.session.read(cx);
        let members = session.state().members(self.guild_id);
        let load = session.members_load(self.guild_id);
        let query = self.search_input.read(cx).value().to_string();
        let shown: Vec<Member> = members_sorted_by_display_name(members)
            .into_iter()
            .filter(|member| matches_member_search(member, &query))
            .collect();
        let is_loaded = !members.is_empty();
        let list = if !is_loaded && load == Some(MembersLoad::Failed) {
            let session = self.session.clone();
            let guild_id = self.guild_id;
            SettingsListStatus::new(
                "members-status",
                SettingsListStatusKind::Failed {
                    failure_text: words.load_failed().into(),
                    retry_label: catalogue.common().try_again().into(),
                    on_retry: std::sync::Arc::new(move |_, cx| {
                        session.update(cx, |session, cx| session.load_members(guild_id, true, cx));
                    }),
                },
            )
            .into_any_element()
        } else if !is_loaded {
            SettingsListStatus::new(
                "members-status",
                SettingsListStatusKind::Loading {
                    label: catalogue.server_settings().loading().into(),
                },
            )
            .into_any_element()
        } else if shown.is_empty() {
            with_type(div(), TYPE_BODY)
                .text_color(color(tokens.component_settings_body_text))
                .child(words.no_match(query.trim()))
                .into_any_element()
        } else {
            let rows: Vec<_> = shown
                .iter()
                .map(|member| self.member_row(member, window, cx).into_any_element())
                .collect();
            div()
                .id("members-list")
                .role(AccessRole::List)
                .aria_label(SharedString::from(
                    catalogue.server_settings().sections().members(),
                ))
                .children(rows)
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_SERVER_SETTINGS_TOOLBAR_GAP))
            .child(
                div().max_w(length(COMPONENT_DIALOG_WIDTH)).child(
                    TextField::new("members-search", words.search(), &self.search_input)
                        .helper(words.search_helper()),
                ),
            )
            .child(list)
            .child(
                div()
                    .id("members-notice")
                    .live_polite(self.notice.clone().unwrap_or_default())
                    .h_0()
                    .overflow_hidden(),
            )
            .when_some(self.managed.as_ref(), |column, managed| {
                column.child(managed.dialog.clone())
            })
            .when_some(self.confirmation.as_ref(), |column, confirmation| {
                column.child(confirmation.dialog.clone())
            })
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Entity, TestAppContext, VisualTestContext};
    use mellow_client_core::shared_logic::server_settings::ServerSettingsSection;
    use mellow_protocol::GatewayEvent;

    use super::*;
    use crate::settings::server::test_support::ADMINS;
    use crate::settings::server::test_support::{open_server_settings, receive, shown_page};
    use crate::shell_fixtures::{GUILD_RUST, ZOE};

    fn confirm_kick(page: &Entity<MembersPage>, cx: &mut VisualTestContext) {
        page.update_in(cx, |page, window, cx| {
            page.open_manage_dialog(ZOE, window, cx);
            page.open_removal_confirmation(ZOE, MemberRemoval::Kick, window, cx);
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn a_failed_kick_keeps_its_confirmation_open(cx: &mut TestAppContext) {
        let (_, settings, cx) = open_server_settings(cx, ServerSettingsSection::Members);
        let page = shown_page::<MembersPage>(&settings, cx);
        confirm_kick(&page, cx);
        let dialog = page.read_with(cx, |page, _| {
            page.confirmation
                .as_ref()
                .map(|confirmation| confirmation.dialog.clone())
                .expect("the kick asks for confirmation")
        });
        // The test session has no connection: the instance never answers yes.
        dialog.update(cx, ModalDialog::submit_now);
        cx.run_until_parked();
        assert!(!dialog.read_with(cx, |dialog, _| dialog.is_busy()));
        page.read_with(cx, |page, _| {
            assert!(page.confirmation.is_some() && page.managed.is_some());
            assert!(page.notice.is_none());
        });
    }

    #[gpui::test]
    fn the_dialogs_of_a_member_who_leaves_close(cx: &mut TestAppContext) {
        let (session, settings, cx) = open_server_settings(cx, ServerSettingsSection::Members);
        let page = shown_page::<MembersPage>(&settings, cx);
        confirm_kick(&page, cx);
        receive(
            &session,
            GatewayEvent::GuildMemberRemove {
                guild_id: GUILD_RUST,
                user_id: ZOE,
            },
            cx,
        );
        page.read_with(cx, |page, _| {
            assert!(page.confirmation.is_none() && page.managed.is_none());
        });
    }

    #[gpui::test]
    fn a_confirmation_closes_when_the_member_comes_to_outrank_the_reader(cx: &mut TestAppContext) {
        let (session, settings, cx) = open_server_settings(cx, ServerSettingsSection::Members);
        let page = shown_page::<MembersPage>(&settings, cx);
        confirm_kick(&page, cx);
        receive(
            &session,
            GatewayEvent::GuildMemberUpdate {
                guild_id: GUILD_RUST,
                user_id: ZOE,
                role_ids: vec![ADMINS],
            },
            cx,
        );
        page.read_with(cx, |page, _| {
            assert!(page.confirmation.is_none() && page.managed.is_some());
        });
    }
}
