//! Discord's "Add members or roles" (`AddOverwriteTargetDialog.svelte`, DESIGN.md §5.8): one
//! search over the roles the reader may target and the members without an overwrite here.
//! Choosing one adds it at once, all Inherit.

use gpui::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, EventEmitter,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::input::{InputEvent, InputState};
use mellow_client_core::shared_logic::channel_settings::{
    OverwriteTarget, addable_members, addable_roles,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::server_settings::role_manager_of;
use mellow_protocol::{OverwriteKind, Permissions, SetPermissionOverwriteRequest, Snowflake};

use super::permissions::{TargetMark, role_colour, target_mark};
use crate::components::{
    Button, ModalDialog, ModalDialogEvent, TextField, child_id, focus_ring, is_focus_visible,
    remembered_focus_handle, remembered_hover, with_type,
};
use crate::i18n::strings;
use crate::session::{MembersLoad, Session};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_PINS_MAX_HEIGHT, COMPONENT_ROLES_ROW_ICON_GAP, COMPONENT_ROLES_ROW_PADDING_INLINE,
    COMPONENT_ROLES_ROW_RADIUS, COMPONENT_SETTINGS_SECTION_LABEL_GAP, SIZE_HIT_WEB_MIN, TYPE_BODY,
    TYPE_CATEGORY, TYPE_CHANNEL_NAME,
};
use crate::theme::{color, length, theme};

/// What the dialog tells the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddOverwriteTargetEvent {
    /// Closed without adding; focus is back on the opener.
    Closed,
    /// Added: the page shows it once the gateway brings it.
    Added(OverwriteTarget),
}

/// One row the dialog offers.
#[derive(Debug, Clone)]
pub(super) struct Candidate {
    pub(super) target: OverwriteTarget,
    name: String,
    mark: TargetMark,
}

/// The members the search finds, once the member list is known.
#[derive(Debug, Clone)]
pub(super) enum MemberCandidates {
    Loading,
    /// The member list could not be loaded: unknown, not empty.
    Failed,
    Ready(Vec<Candidate>),
}

/// What the search finds.
#[derive(Debug, Clone)]
pub(super) struct Candidates {
    pub(super) roles: Vec<Candidate>,
    pub(super) members: MemberCandidates,
}

pub struct AddOverwriteTarget {
    session: Entity<Session>,
    channel_id: Snowflake,
    dialog: Option<Entity<ModalDialog>>,
    search: Entity<InputState>,
    is_busy: bool,
    subscriptions: Vec<Subscription>,
}

impl EventEmitter<AddOverwriteTargetEvent> for AddOverwriteTarget {}

impl std::fmt::Debug for AddOverwriteTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AddOverwriteTarget")
            .field("channel_id", &self.channel_id)
            .field("is_busy", &self.is_busy)
            .finish_non_exhaustive()
    }
}

impl AddOverwriteTarget {
    /// Opens over the window, the search focused.
    pub fn open(
        session: Entity<Session>,
        channel_id: Snowflake,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let catalogue = strings(cx);
        let words = catalogue.channel_settings().permissions();
        let channel_name = session
            .read(cx)
            .state()
            .guild_channel(channel_id)
            .map(|channel| channel.name.clone())
            .unwrap_or_default();
        let search = cx.new(|cx| InputState::new(window, cx));
        let picker = cx.new(|cx| {
            let subscriptions = vec![
                cx.observe(&session, |_, _, cx| cx.notify()),
                cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                }),
            ];
            Self {
                session,
                channel_id,
                dialog: None,
                search: search.clone(),
                is_busy: false,
                subscriptions,
            }
        });
        let body = picker.clone().into();
        let dialog = cx.new(|cx| {
            ModalDialog::new(
                words.add_title(&channel_name),
                catalogue.common().cancel(),
                window,
                cx,
            )
            .subtitle(words.add_subtitle())
            .body(body)
        });
        picker.update(cx, |picker, cx| {
            let subscription = cx.subscribe(&dialog, |picker, _, event: &ModalDialogEvent, cx| {
                if *event == ModalDialogEvent::Closed {
                    picker.dialog = None;
                    cx.emit(AddOverwriteTargetEvent::Closed);
                }
            });
            picker.subscriptions.push(subscription);
            picker.dialog = Some(dialog);
        });
        // The search takes focus, as the web's `data-initial-focus`.
        search.update(cx, |search, cx| search.focus(window, cx));
        picker
    }

    /// The dialog holding the search, drawn by the page.
    #[must_use]
    pub fn dialog(&self) -> Option<&Entity<ModalDialog>> {
        self.dialog.as_ref()
    }

    pub(super) fn candidates(&self, cx: &App) -> Candidates {
        let session = self.session.read(cx);
        let state = session.state();
        let (Some(guild), Some(channel)) = (
            state.guild_of_channel(self.channel_id),
            state.guild_channel(self.channel_id),
        ) else {
            return Candidates {
                roles: Vec::new(),
                members: MemberCandidates::Ready(Vec::new()),
            };
        };
        let query = self.search.read(cx).value().to_string();
        let manager = role_manager_of(guild, state.current_user().map(|user| user.id));
        let roles = addable_roles(guild, channel, &manager, &query)
            .into_iter()
            .map(|role| Candidate {
                target: OverwriteTarget {
                    kind: OverwriteKind::Role,
                    id: role.id,
                },
                name: role.name.clone(),
                mark: TargetMark::Role(role_colour(role.color)),
            })
            .collect();
        let members_list = state.members(guild.id);
        let members = match session.members_load(guild.id) {
            Some(MembersLoad::Loading) => MemberCandidates::Loading,
            Some(MembersLoad::Failed) if members_list.is_empty() => MemberCandidates::Failed,
            // Not asked yet: the page asked on opening, so it is on its way.
            None if members_list.is_empty() => MemberCandidates::Loading,
            _ => MemberCandidates::Ready(
                addable_members(channel, members_list, &query)
                    .into_iter()
                    .map(|member| Candidate {
                        target: OverwriteTarget {
                            kind: OverwriteKind::Member,
                            id: member.user.id,
                        },
                        name: display_name(&member.user).to_owned(),
                        mark: TargetMark::Member(Some(member.user.clone())),
                    })
                    .collect(),
            ),
        };
        Candidates { roles, members }
    }

    fn set_busy(&mut self, is_busy: bool, cx: &mut Context<Self>) {
        self.is_busy = is_busy;
        if let Some(dialog) = self.dialog.clone() {
            dialog.update(cx, |dialog, cx| dialog.set_busy(is_busy, cx));
        }
        cx.notify();
    }

    fn add(&mut self, target: OverwriteTarget, cx: &mut Context<Self>) {
        if self.is_busy {
            return;
        }
        self.set_busy(true, cx);
        if let Some(dialog) = self.dialog.clone() {
            dialog.update(cx, |dialog, cx| dialog.set_error(None, cx));
        }
        let channel_id = self.channel_id;
        let request = SetPermissionOverwriteRequest {
            kind: target.kind,
            allow: Permissions::empty(),
            deny: Permissions::empty(),
        };
        let failure = strings(cx)
            .channel_settings()
            .permissions()
            .add_failed()
            .to_owned();
        let picker = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_settings_change(
                "add permission overwrite",
                failure,
                move |rest| async move {
                    rest.set_permission_overwrite(channel_id, target.id, &request)
                        .await
                },
                cx,
                move |outcome, cx| {
                    let _ = picker.update(cx, |picker, cx| {
                        picker.set_busy(false, cx);
                        match outcome {
                            Ok(()) => {
                                picker.dialog = None;
                                cx.emit(AddOverwriteTargetEvent::Added(target));
                            }
                            Err(message) => {
                                if let Some(dialog) = picker.dialog.clone() {
                                    dialog.update(cx, |dialog, cx| {
                                        dialog.set_error(Some(message.into()), cx);
                                    });
                                }
                            }
                        }
                    });
                },
            );
        });
    }

    fn render_rows(
        heading_id: &'static str,
        heading: &'static str,
        rows: &[Candidate],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let entity = cx.entity();
        let items: Vec<AnyElement> = rows
            .iter()
            .map(|candidate| {
                let id = ElementId::from(("overwrite-candidate", candidate.target.id.raw()));
                let focus = remembered_focus_handle(&id, window, cx);
                let hover = remembered_hover(&id, window, cx);
                let is_ring_shown = is_focus_visible(&focus, window);
                let target = candidate.target;
                let entity = entity.clone();
                let (fill, text) = if *hover.read(cx) {
                    (
                        Some(tokens.component_roles_row_background_hover),
                        tokens.component_roles_row_text_hover,
                    )
                } else {
                    (None, tokens.component_roles_row_text)
                };
                with_type(div(), TYPE_CHANNEL_NAME)
                    .id(id)
                    .role(Role::Button)
                    .aria_label(SharedString::from(candidate.name.clone()))
                    .track_focus(&focus)
                    .relative()
                    .flex()
                    .items_center()
                    .gap(length(COMPONENT_ROLES_ROW_ICON_GAP))
                    .w_full()
                    .min_h(length(SIZE_HIT_WEB_MIN))
                    .px(length(COMPONENT_ROLES_ROW_PADDING_INLINE))
                    .rounded(length(COMPONENT_ROLES_ROW_RADIUS))
                    .when_some(fill, |row, fill| row.bg(color(fill)))
                    .text_color(color(text))
                    .cursor_pointer()
                    .on_hover(move |is_hovered, _, cx| {
                        hover.update(cx, |remembered, cx| {
                            *remembered = *is_hovered;
                            cx.notify();
                        });
                    })
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |picker, cx| picker.add(target, cx));
                    })
                    .child(target_mark(&candidate.mark, window, cx))
                    .child(div().min_w_0().truncate().child(candidate.name.clone()))
                    .when(is_ring_shown, |row| {
                        row.child(focus_ring(COMPONENT_ROLES_ROW_RADIUS, cx))
                    })
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .child(
                with_type(div(), TYPE_CATEGORY)
                    .id(child_id(&ElementId::from(heading_id), "title"))
                    .role(Role::Heading)
                    .aria_level(3)
                    .text_color(color(tokens.component_settings_section_label_text))
                    .child(heading),
            )
            .child(
                div()
                    .id(heading_id)
                    .role(Role::List)
                    .aria_label(heading)
                    .flex()
                    .flex_col()
                    .children(items),
            )
            .into_any_element()
    }
}

impl Render for AddOverwriteTarget {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.channel_settings().permissions();
        let candidates = self.candidates(cx);
        let query = self.search.read(cx).value().trim().to_owned();
        let is_empty = candidates.roles.is_empty()
            && matches!(&candidates.members, MemberCandidates::Ready(members) if members.is_empty());
        let status = |text: String| {
            with_type(div(), TYPE_BODY)
                .id("overwrite-candidates-status")
                .role(Role::Status)
                .aria_label(SharedString::from(text.clone()))
                .text_color(color(tokens.component_settings_body_text))
                .child(text)
        };
        let roles = (!candidates.roles.is_empty()).then(|| {
            Self::render_rows(
                "overwrite-roles",
                words.roles(),
                &candidates.roles,
                window,
                cx,
            )
        });
        let members = match &candidates.members {
            MemberCandidates::Loading => {
                Some(status(words.loading_members().to_owned()).into_any_element())
            }
            MemberCandidates::Failed => {
                let session = self.session.clone();
                let guild_id = session
                    .read(cx)
                    .state()
                    .guild_of_channel(self.channel_id)
                    .map(|guild| guild.id);
                Some(
                    div()
                        .id("overwrite-members-failed")
                        .role(Role::Alert)
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                        .child(
                            with_type(div(), TYPE_BODY)
                                .text_color(color(tokens.component_settings_body_text))
                                .child(catalogue.members().load_failed()),
                        )
                        .child(
                            Button::new(
                                "overwrite-members-try-again",
                                catalogue.common().try_again(),
                            )
                            .on_click(move |_, cx| {
                                if let Some(guild_id) = guild_id {
                                    session.update(cx, |session, cx| {
                                        session.load_members(guild_id, true, cx);
                                    });
                                }
                            }),
                        )
                        .into_any_element(),
                )
            }
            MemberCandidates::Ready(members) if !members.is_empty() => Some(Self::render_rows(
                "overwrite-members",
                words.members(),
                members,
                window,
                cx,
            )),
            MemberCandidates::Ready(_) => None,
        };
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
            .child(
                TextField::new("overwrite-search", words.search(), &self.search)
                    .leading_icon(IconName::MagnifyingGlass),
            )
            .child(
                div()
                    .id("overwrite-candidates")
                    .max_h(length(COMPONENT_PINS_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                    .children(roles)
                    .children(members)
                    .when(is_empty, |list| {
                        list.child(status(if query.is_empty() {
                            words.nothing_to_add().to_owned()
                        } else {
                            words.no_match(&query)
                        }))
                    }),
            )
    }
}
