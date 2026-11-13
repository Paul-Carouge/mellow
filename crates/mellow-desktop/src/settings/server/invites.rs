//! Invites (`InviteSettings.svelte`, DESIGN.md §5.8): the invites that still work, newest first,
//! with their code, who created them, their uses and when they expire. "Create invite" asks for
//! the limits first (`InviteLimitsFields.svelte`); revoking asks first (§8). No gateway event
//! tells about invites, so the list is read when the page opens and after each change.
//!
//! shortcut: the web's native selects are radio groups here, the desktop having no select yet.

use std::time::Duration;

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Task, Window, div,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::invites::{
    DEFAULT_INVITE_LIMITS, INVITE_LIFETIME_PRESETS_IN_SECONDS, INVITE_MAXIMUM_USES_PRESETS,
    InviteExpiry, duration_label, invite_expiry, relative_future_label,
};
use mellow_protocol::{CreateInviteRequest, GuildInvite, Snowflake};

use super::list_rows::{
    empty_sentence, list_row, list_status, person_avatar, secondary_line, status_notice,
    text_column,
};
use crate::components::{
    Button, ButtonVariant, ChoiceGroup, ChoiceOption, ModalDialog, ModalDialogEvent, with_type,
};
use crate::i18n::{Strings, strings};
use crate::messages::locale;
use crate::session::{Session, now_unix_milliseconds};
use crate::settings::{SettingsPage, SettingsPageEvent};
use crate::sign_in_form::describe_error;
use crate::theme::tokens::{
    COMPONENT_INVITE_FIELD_BASIS, COMPONENT_SERVER_SETTINGS_TOOLBAR_GAP,
    COMPONENT_TEXT_FIELD_FIELD_GAP, TYPE_CODE,
};
use crate::theme::{color, length, theme};

/// "Expires in 3 days" counts from now, refreshed every minute while the page is open.
const EXPIRY_REFRESH: Duration = Duration::from_secs(60);

/// The dialog's two choices; choosing only changes the limits, the dialog creates the invite.
pub(super) struct InviteLimitsFields {
    limits: CreateInviteRequest,
    /// The dialog opens on the checked lifetime; later draws leave focus where the reader put it.
    should_focus_lifetime: bool,
}

impl InviteLimitsFields {
    fn lifetime_label(seconds: Option<u32>, catalogue: Strings) -> String {
        seconds.map_or_else(
            || catalogue.invite_limits().never().to_owned(),
            |seconds| duration_label(locale(catalogue), seconds),
        )
    }

    fn uses_label(uses: Option<u32>, catalogue: Strings) -> String {
        uses.map_or_else(
            || catalogue.invite_limits().no_limit().to_owned(),
            |uses| catalogue.invite_limits().uses(i64::from(uses)),
        )
    }
}

impl Render for InviteLimitsFields {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let should_focus_lifetime = std::mem::take(&mut self.should_focus_lifetime);
        let catalogue = strings(cx);
        let words = catalogue.invite_limits();
        let lifetime_index = INVITE_LIFETIME_PRESETS_IN_SECONDS
            .iter()
            .position(|preset| *preset == self.limits.expires_in_seconds)
            .unwrap_or_default();
        let uses_index = INVITE_MAXIMUM_USES_PRESETS
            .iter()
            .position(|preset| *preset == self.limits.max_uses)
            .unwrap_or_default();
        let lifetime_fields = cx.entity().downgrade();
        let uses_fields = lifetime_fields.clone();
        let field = || div().flex_1().min_w(length(COMPONENT_INVITE_FIELD_BASIS));
        div()
            .flex()
            .flex_wrap()
            .gap(length(COMPONENT_TEXT_FIELD_FIELD_GAP))
            .child(
                field().child(
                    ChoiceGroup::new(
                        "invite-lifetime",
                        words.expire_after(),
                        INVITE_LIFETIME_PRESETS_IN_SECONDS
                            .iter()
                            .map(|seconds| {
                                ChoiceOption::new(Self::lifetime_label(*seconds, catalogue))
                            })
                            .collect(),
                        lifetime_index,
                    )
                    .focus_checked(should_focus_lifetime)
                    .on_select(move |index, _, cx| {
                        let _ = lifetime_fields.update(cx, |fields, cx| {
                            if let Some(preset) = INVITE_LIFETIME_PRESETS_IN_SECONDS.get(index) {
                                fields.limits.expires_in_seconds = *preset;
                            }
                            cx.notify();
                        });
                    }),
                ),
            )
            .child(
                field().child(
                    ChoiceGroup::new(
                        "invite-uses",
                        words.maximum_uses(),
                        INVITE_MAXIMUM_USES_PRESETS
                            .iter()
                            .map(|uses| ChoiceOption::new(Self::uses_label(*uses, catalogue)))
                            .collect(),
                        uses_index,
                    )
                    .on_select(move |index, _, cx| {
                        let _ = uses_fields.update(cx, |fields, cx| {
                            if let Some(preset) = INVITE_MAXIMUM_USES_PRESETS.get(index) {
                                fields.limits.max_uses = *preset;
                            }
                            cx.notify();
                        });
                    }),
                ),
            )
    }
}

/// The dialog open over the page.
enum OpenDialog {
    Create {
        dialog: Entity<ModalDialog>,
        fields: Entity<InviteLimitsFields>,
        _subscription: Subscription,
    },
    Revoke {
        dialog: Entity<ModalDialog>,
        _subscription: Subscription,
    },
}

impl OpenDialog {
    fn dialog(&self) -> &Entity<ModalDialog> {
        match self {
            Self::Create { dialog, .. } | Self::Revoke { dialog, .. } => dialog,
        }
    }
}

pub struct ServerInvitesPage {
    session: Entity<Session>,
    guild_id: Snowflake,
    /// `None` while loading or after a failure.
    invites: Option<Vec<GuildInvite>>,
    has_failed: bool,
    /// Answers to an earlier load are dropped.
    load_generation: u64,
    now_unix_milliseconds: u64,
    notice: SharedString,
    create_focus: FocusHandle,
    open_dialog: Option<OpenDialog>,
    /// Where focus goes once the next frame is drawn: the dialog and its opener went away.
    pending_focus: Option<FocusHandle>,
    _clock: Task<()>,
}

impl EventEmitter<SettingsPageEvent> for ServerInvitesPage {}

impl SettingsPage for ServerInvitesPage {
    fn set_attention(&mut self, _attention: u32, _cx: &mut Context<Self>) {}
}

impl std::fmt::Debug for ServerInvitesPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerInvitesPage")
            .field("guild_id", &self.guild_id)
            .field("has_failed", &self.has_failed)
            .finish_non_exhaustive()
    }
}

impl ServerInvitesPage {
    pub fn new(session: Entity<Session>, guild_id: Snowflake, cx: &mut Context<Self>) -> Self {
        let clock = cx.spawn(async move |page, cx| {
            loop {
                cx.background_executor().timer(EXPIRY_REFRESH).await;
                let is_open = page
                    .update(cx, |page, cx| {
                        page.now_unix_milliseconds = now_unix_milliseconds();
                        cx.notify();
                    })
                    .is_ok();
                if !is_open {
                    break;
                }
            }
        });
        let mut page = Self {
            session,
            guild_id,
            invites: None,
            has_failed: false,
            load_generation: 0,
            now_unix_milliseconds: now_unix_milliseconds(),
            notice: SharedString::default(),
            create_focus: cx.focus_handle(),
            open_dialog: None,
            pending_focus: None,
            _clock: clock,
        };
        page.load(cx);
        page
    }

    // Only the tests against a real server read it; the server refuses Windows (ADR-0006).
    #[cfg(all(test, not(windows)))]
    pub(crate) fn invites(&self) -> Option<&[GuildInvite]> {
        self.invites.as_deref()
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.has_failed = false;
        self.load_generation += 1;
        let generation = self.load_generation;
        let guild_id = self.guild_id;
        let page = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_settings_request(
                "list invites",
                move |rest| async move { rest.list_invites(guild_id).await },
                cx,
                move |answer, cx| {
                    let _ = page.update(cx, |page, cx| {
                        if page.load_generation != generation {
                            return;
                        }
                        if let Ok(invites) = answer {
                            page.invites = Some(invites);
                            page.now_unix_milliseconds = now_unix_milliseconds();
                        } else {
                            page.invites = None;
                            page.has_failed = true;
                        }
                        cx.notify();
                    });
                },
            );
        });
    }

    fn close_dialog(&mut self, cx: &mut Context<Self>) {
        self.open_dialog = None;
        cx.notify();
    }

    /// After a create or a revoke, as the web's fallback focus.
    fn close_dialog_to_create_button(&mut self, cx: &mut Context<Self>) {
        self.pending_focus = Some(self.create_focus.clone());
        self.close_dialog(cx);
    }

    pub(crate) fn open_create_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let catalogue = strings(cx);
        let words = catalogue.server_settings().invites();
        let fields = cx.new(|_| InviteLimitsFields {
            limits: DEFAULT_INVITE_LIMITS,
            should_focus_lifetime: true,
        });
        let body = fields.clone();
        let dialog = cx.new(|cx| {
            ModalDialog::new(
                words.create_title(),
                catalogue.common().cancel(),
                window,
                cx,
            )
            .body(body.into())
            .submit(words.create_submit(), words.creating(), false)
        });
        let subscription = cx.subscribe(
            &dialog,
            |page, _, event: &ModalDialogEvent, cx| match event {
                ModalDialogEvent::Submitted => page.create(cx),
                ModalDialogEvent::Closed => page.close_dialog(cx),
            },
        );
        self.open_dialog = Some(OpenDialog::Create {
            dialog,
            fields,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(crate) fn create(&mut self, cx: &mut Context<Self>) {
        let Some(OpenDialog::Create { dialog, fields, .. }) = self.open_dialog.as_ref() else {
            return;
        };
        let (dialog, limits) = (dialog.clone(), fields.read(cx).limits.clone());
        dialog.update(cx, |dialog, cx| {
            dialog.set_busy(true, cx);
            dialog.set_error(None, cx);
        });
        let guild_id = self.guild_id;
        let page = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_settings_request(
                "create an invite",
                move |rest| async move { rest.create_invite(guild_id, &limits).await },
                cx,
                move |answer, cx| {
                    dialog.update(cx, |dialog, cx| dialog.set_busy(false, cx));
                    match answer {
                        Ok(invite) => {
                            let _ = page.update(cx, |page, cx| {
                                page.notice = strings(cx)
                                    .server_settings()
                                    .invites()
                                    .created(&invite.code)
                                    .into();
                                page.close_dialog_to_create_button(cx);
                                page.load(cx);
                            });
                        }
                        Err(error) => {
                            let summary = strings(cx).server_settings().invites().create_failed();
                            let sentence = describe_error(
                                &error,
                                summary,
                                crate::strings_pending_i18n::strings(cx),
                            );
                            dialog.update(cx, |dialog, cx| {
                                dialog.set_error(Some(sentence.into()), cx);
                            });
                        }
                    }
                },
            );
        });
    }

    fn open_revoke_dialog(&mut self, code: String, window: &mut Window, cx: &mut Context<Self>) {
        let catalogue = strings(cx);
        let words = catalogue.server_settings().invites();
        let dialog = cx.new(|cx| {
            ModalDialog::new(
                words.revoke_title(),
                catalogue.common().cancel(),
                window,
                cx,
            )
            .subtitle(words.revoke_body())
            .submit(words.revoke_submit(), words.revoking(), true)
        });
        let subscription =
            cx.subscribe(
                &dialog,
                move |page, _, event: &ModalDialogEvent, cx| match event {
                    ModalDialogEvent::Submitted => page.revoke(code.clone(), cx),
                    ModalDialogEvent::Closed => page.close_dialog(cx),
                },
            );
        self.open_dialog = Some(OpenDialog::Revoke {
            dialog,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn revoke(&mut self, code: String, cx: &mut Context<Self>) {
        let Some(dialog) = self.open_dialog.as_ref().map(|open| open.dialog().clone()) else {
            return;
        };
        dialog.update(cx, |dialog, cx| {
            dialog.set_busy(true, cx);
            dialog.set_error(None, cx);
        });
        let page = cx.entity().downgrade();
        let sent_code = code.clone();
        self.session.update(cx, |session, cx| {
            session.send_settings_request(
                "revoke an invite",
                move |rest| async move { rest.revoke_invite(&sent_code).await },
                cx,
                move |answer, cx| {
                    dialog.update(cx, |dialog, cx| dialog.set_busy(false, cx));
                    match answer {
                        Ok(()) => {
                            let _ = page.update(cx, |page, cx| {
                                if let Some(invites) = page.invites.as_mut() {
                                    invites.retain(|invite| invite.code != code);
                                }
                                // The row and its button are gone.
                                page.close_dialog_to_create_button(cx);
                            });
                        }
                        Err(error) => {
                            let summary = strings(cx).server_settings().invites().revoke_failed();
                            let sentence = describe_error(
                                &error,
                                summary,
                                crate::strings_pending_i18n::strings(cx),
                            );
                            dialog.update(cx, |dialog, cx| {
                                dialog.set_error(Some(sentence.into()), cx);
                            });
                        }
                    }
                },
            );
        });
    }

    fn expiry_label(&self, invite: &GuildInvite, catalogue: Strings) -> String {
        let words = catalogue.server_settings().invites();
        match invite_expiry(
            invite.expires_at_unix_milliseconds,
            self.now_unix_milliseconds,
        ) {
            InviteExpiry::Never => words.never_expires().to_owned(),
            InviteExpiry::Expired => words.expired().to_owned(),
            InviteExpiry::In { count, unit } => {
                words.expires_in(&relative_future_label(locale(catalogue), count, unit))
            }
        }
    }

    fn render_list(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let catalogue = strings(cx);
        let words = catalogue.server_settings().invites();
        let Some(invites) = self.invites.as_ref() else {
            let page = cx.entity().downgrade();
            return list_status(
                "invites-status",
                self.has_failed,
                words.load_failed(),
                move |_, cx| {
                    let _ = page.update(cx, ServerInvitesPage::load);
                },
                cx,
            )
            .into_any_element();
        };
        if invites.is_empty() {
            return empty_sentence(words.empty(), cx).into_any_element();
        }
        let code_color = color(theme(cx).component_server_settings_code_text);
        let rows = invites.iter().enumerate().map(|(index, invite)| {
            let page = cx.entity().downgrade();
            let code = invite.code.clone();
            let details = format!(
                "{} · {}",
                words.uses(i64::from(invite.uses), invite.max_uses.map(i64::from)),
                self.expiry_label(invite, catalogue)
            );
            list_row(cx)
                .id(("invite", index))
                .role(Role::ListItem)
                .aria_position_in_set(index + 1)
                .aria_size_of_set(invites.len())
                .child(person_avatar(&invite.creator, window, cx))
                .child(
                    text_column()
                        .child(
                            with_type(div(), TYPE_CODE)
                                .truncate()
                                .text_color(code_color)
                                .child(invite.code.clone()),
                        )
                        .child(secondary_line(
                            words.created_by(display_name(&invite.creator)),
                            cx,
                        ))
                        .child(secondary_line(details, cx)),
                )
                .child(
                    Button::new(("invite-revoke", index), words.revoke())
                        .accessible_label(words.revoke_label(&invite.code))
                        .on_click(move |window, cx| {
                            let code = code.clone();
                            let _ = page.update(cx, |page, cx| {
                                page.open_revoke_dialog(code, window, cx);
                            });
                        }),
                )
        });
        div()
            .id("invites-list")
            .role(Role::List)
            .aria_label(catalogue.server_settings().sections().invites())
            .children(rows.collect::<Vec<_>>())
            .into_any_element()
    }
}

impl Render for ServerInvitesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(focus) = self.pending_focus.take() {
            window.focus(&focus, cx);
        }
        let words = strings(cx).server_settings().invites();
        let page = cx.entity().downgrade();
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_SERVER_SETTINGS_TOOLBAR_GAP))
            .child(
                div().child(
                    Button::new("invites-create", words.create_invite())
                        .variant(ButtonVariant::Primary)
                        .track_focus(&self.create_focus)
                        .on_click(move |window, cx| {
                            let _ = page.update(cx, |page, cx| page.open_create_dialog(window, cx));
                        }),
                ),
            )
            .child(self.render_list(window, cx))
            .child(status_notice("invites-notice", self.notice.clone()))
            .children(self.open_dialog.as_ref().map(|open| open.dialog().clone()))
    }
}
