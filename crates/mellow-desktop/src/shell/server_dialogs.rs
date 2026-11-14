//! The dialogs behind the rail's "+" and a server's menus (`AddServerDialog.svelte`,
//! `InvitePeopleDialog.svelte`, `CreateChannelDialog.svelte`, `DeleteChannelDialog.svelte`,
//! `MoveToCategoryDialog.svelte`, `RenameCategoryDialog.svelte`, DESIGN.md §5.2, §5.3, §8), and
//! the menus' moves of channels and categories (`channel-reorder.svelte.ts`). One dialog shows at
//! a time; the shell draws it over everything.
//!
//! `ModalDialog` keeps its title, so a dialog whose title follows its controls ("Create a server"
//! and "Join a server", "Create a channel" and "Create a category") is drawn again with the same
//! body, focus first given back to the dialog's opener so it still returns there.
//!
//! shortcut: the web's server icon and live preview in "Create a server", the voice channel's user
//! limit in "Create a channel" (set it in the channel's settings), and the pending order a move
//! shows until the gateway echoes it are not here yet; the instance's echo comes within a frame
//! or two on a healthy connection.

use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, AppContext as _, Context, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::input::{InputEvent, InputState};
use mellow_client_core::shared_logic::channel_order::{
    ChannelDestination, ChannelSection, Step, category_step_index, channel_sections,
    channel_step_destination, placements_of, with_category_moved, with_channel_moved,
};
use mellow_client_core::shared_logic::channel_settings::{
    MAXIMUM_CHANNEL_NAME_LENGTH, MAXIMUM_CHANNEL_TOPIC_LENGTH, channel_name_as_saved,
};
use mellow_client_core::shared_logic::invites::{
    DEFAULT_INVITE_LIMITS, INVITE_LIFETIME_PRESETS_IN_SECONDS, INVITE_MAXIMUM_USES_PRESETS,
    duration_label, invite_code_groups, spoken_invite_code,
};
use mellow_client_core::shared_logic::server_settings::MAXIMUM_SERVER_NAME_LENGTH;
use mellow_protocol::{
    Channel, ChannelKind, CreateChannelRequest, CreateInviteRequest, InitialCategory,
    InitialTextChannel, Invite, Permissions, Snowflake, UpdateChannelRequest,
};

use super::DuskShell;
use crate::components::{
    Button, ButtonVariant, ChoiceGroup, ChoiceLegend, ChoiceOption, ContextMenu, IconButton,
    LiveRegion as _, MenuAlignment, MenuAnchor, MenuEntry, MenuItem, MenuOrigin, ModalDialog,
    ModalDialogEvent, TextField, focus_ring, icon, is_focus_visible, with_type,
};
use crate::i18n::{Language, Strings, strings};
use crate::messages::locale;
use crate::session::Session;
use crate::sign_in_form::describe_error;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CATEGORY_CHEVRON_SIZE, COMPONENT_CHANNEL_ROW_MARGIN_INLINE,
    COMPONENT_CHANNEL_ROW_PADDING_INLINE, COMPONENT_CHANNEL_ROW_SETTINGS_BUTTON_SIZE,
    COMPONENT_CHANNEL_ROW_SETTINGS_ICON_SIZE, COMPONENT_CHANNEL_ROW_TRAILING_ICON_SIZE,
    COMPONENT_DIALOG_BUTTON_GAP, COMPONENT_DIALOG_COPIED_LABEL_DURATION,
    COMPONENT_DIALOG_SECTION_GAP, COMPONENT_INVITE_CODE_BORDER_WIDTH,
    COMPONENT_INVITE_CODE_PADDING_BLOCK, COMPONENT_INVITE_CODE_PADDING_INLINE,
    COMPONENT_INVITE_CODE_RADIUS, COMPONENT_INVITE_FIELD_BASIS, COMPONENT_INVITE_STACK_GAP,
    COMPONENT_TEXT_FIELD_FIELD_GAP, RADIUS_XS, SPACE_2XS, SPACE_XS, TYPE_BODY, TYPE_BODY_SM,
    TYPE_INVITE_CODE,
};
use crate::theme::{color, delay, length, theme};

/// The words the web does not have: the desktop's way to another instance (DESIGN.md §5.1).
/// French in the catalogue's voice (tutoiement, a narrow no-break space before `?`).
struct InstanceWords {
    connect_another_instance: &'static str,
    title: &'static str,
    body: fn(&str) -> String,
    submit: &'static str,
    signing_out: &'static str,
}

const ENGLISH_INSTANCE_WORDS: InstanceWords = InstanceWords {
    connect_another_instance: "Connect to another instance",
    title: "Connect to another instance?",
    body: |instance| {
        format!(
            "You'll be signed out of {instance} on this computer, then you can enter another \
             instance's address. Your account and servers stay on {instance}: sign back in \
             whenever you like."
        )
    },
    submit: "Sign out and continue",
    signing_out: "Signing out…",
};

const FRENCH_INSTANCE_WORDS: InstanceWords = InstanceWords {
    connect_another_instance: "Se connecter à une autre instance",
    title: "Se connecter à une autre instance\u{202f}?",
    body: |instance| {
        format!(
            "Tu seras déconnecté de {instance} sur cet ordinateur, puis tu pourras saisir \
             l’adresse d’une autre instance. Ton compte et tes serveurs restent sur {instance}\u{a0}: \
             reconnecte-toi quand tu veux."
        )
    },
    submit: "Me déconnecter et continuer",
    signing_out: "Déconnexion…",
};

fn instance_words(words: Strings) -> &'static InstanceWords {
    match words.language() {
        Language::English => &ENGLISH_INSTANCE_WORDS,
        Language::French => &FRENCH_INSTANCE_WORDS,
    }
}

/// "Connect to another instance", for the rail's "+" menu.
pub(super) fn connect_another_instance_label(words: Strings) -> &'static str {
    instance_words(words).connect_another_instance
}

/// The starter templates of "Create a server" (`server-templates.ts`), in the order listed; the
/// first is chosen when the dialog opens.
const TEMPLATE_COUNT: usize = 5;

fn template_of(
    words: Strings,
    index: usize,
) -> (
    &'static str,
    &'static [crate::i18n::catalogue::CategoriesEntry],
) {
    let templates = words.add_server().templates();
    match index {
        0 => (templates.friends().name(), templates.friends().categories()),
        1 => (
            templates.community().name(),
            templates.community().categories(),
        ),
        2 => (
            templates.study_group().name(),
            templates.study_group().categories(),
        ),
        3 => (templates.gaming().name(), templates.gaming().categories()),
        _ => (templates.blank().name(), templates.blank().categories()),
    }
}

/// What a template sends as `initial_categories`.
fn initial_categories_of(words: Strings, index: usize) -> Vec<InitialCategory> {
    template_of(words, index)
        .1
        .iter()
        .map(|category| InitialCategory {
            name: category.name.to_owned(),
            channels: category
                .channels
                .iter()
                .map(|channel| InitialTextChannel {
                    name: channel.name.to_owned(),
                    topic: channel.topic.unwrap_or_default().to_owned(),
                })
                .collect(),
        })
        .collect()
}

/// "general, photos, plans": the channels a template brings, as its row describes it.
fn template_channel_names(words: Strings, index: usize) -> String {
    template_of(words, index)
        .1
        .iter()
        .flat_map(|category| category.channels.iter().map(|channel| channel.name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// What a link-like button does, from a click or a key.
type Activation = Rc<dyn Fn(&mut Window, &mut App)>;

/// A sentence's link-like button ("Join a server" under "Already have an invite?").
fn mode_link(
    id: &'static str,
    label: SharedString,
    focus: &FocusHandle,
    window: &Window,
    cx: &App,
    on_activate: Activation,
) -> impl IntoElement {
    let tokens = theme(cx);
    let is_ring_shown = is_focus_visible(focus, window);
    let on_key = on_activate.clone();
    with_type(div(), TYPE_BODY)
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .track_focus(focus)
        .relative()
        .px(length(SPACE_2XS))
        .rounded(length(RADIUS_XS))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(color(tokens.component_dialog_mode_link_text))
        .cursor_pointer()
        .hover(gpui::Styled::underline)
        .on_click(move |_, window, cx| on_activate(window, cx))
        .on_key_down(move |event, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                on_key(window, cx);
            }
        })
        .child(label)
        .when(is_ring_shown, |link| link.child(focus_ring(RADIUS_XS, cx)))
}

// ---- "Create a server" and "Join a server" ----

pub(super) struct AddServerBody {
    is_joining: bool,
    /// Join only when the instance keeps server creation to its administrators.
    may_create: bool,
    name: Entity<InputState>,
    code: Entity<InputState>,
    template_index: usize,
    is_busy: bool,
    link_focus: FocusHandle,
    shell: WeakEntity<DuskShell>,
}

impl AddServerBody {
    fn can_submit(&self, cx: &App) -> bool {
        let field = if self.is_joining {
            &self.code
        } else {
            &self.name
        };
        !self.is_busy && !field.read(cx).value().trim().is_empty()
    }

    fn field(&self) -> Entity<InputState> {
        if self.is_joining {
            self.code.clone()
        } else {
            self.name.clone()
        }
    }
}

impl Render for AddServerBody {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.add_server();
        let body = cx.entity().downgrade();
        let fields: AnyElement = if self.is_joining {
            TextField::new("join-server-invite-code", words.invite_code(), &self.code)
                .helper(words.invite_code_helper())
                .monospace()
                .read_only(self.is_busy)
                .into_any_element()
        } else {
            let options = (0..TEMPLATE_COUNT)
                .map(|index| {
                    ChoiceOption::new(template_of(catalogue, index).0)
                        .description(template_channel_names(catalogue, index))
                })
                .collect();
            div()
                .flex()
                .flex_col()
                .gap(length(COMPONENT_DIALOG_SECTION_GAP))
                .child(
                    TextField::new("create-server-name", words.server_name(), &self.name)
                        .read_only(self.is_busy),
                )
                .child(
                    ChoiceGroup::new(
                        "create-server-template",
                        words.start_with(),
                        options,
                        self.template_index,
                    )
                    .legend(ChoiceLegend::Field)
                    .on_select(move |index, _, cx| {
                        let _ = body.update(cx, |body, cx| {
                            if !body.is_busy {
                                body.template_index = index;
                                cx.notify();
                            }
                        });
                    }),
                )
                .into_any_element()
        };
        let note: AnyElement = if self.may_create {
            let shell = self.shell.clone();
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(length(SPACE_2XS))
                .child(
                    with_type(div(), TYPE_BODY)
                        .text_color(color(tokens.component_dialog_hint_text))
                        .child(if self.is_joining {
                            words.has_no_invite()
                        } else {
                            words.has_invite()
                        }),
                )
                .child(mode_link(
                    "add-server-mode",
                    if self.is_joining {
                        words.create_title()
                    } else {
                        words.join_title()
                    }
                    .into(),
                    &self.link_focus,
                    window,
                    cx,
                    Rc::new(move |window, cx| {
                        let _ = shell.update(cx, |shell, cx| {
                            shell.switch_add_server_page(window, cx);
                        });
                    }),
                ))
                .into_any_element()
        } else {
            with_type(div(), TYPE_BODY)
                .text_color(color(tokens.component_dialog_hint_text))
                .child(words.creation_reserved_to_administrators())
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_DIALOG_SECTION_GAP))
            .child(fields)
            .child(note)
    }
}

// ---- "Invite people" ----

#[allow(clippy::struct_excessive_bools)] // Independent states of one dialog, as the web's.
pub(super) struct InviteBody {
    session: Entity<Session>,
    guild_id: Snowflake,
    dialog: WeakEntity<ModalDialog>,
    invite: Option<Invite>,
    /// The limits the shown code was created with, for the sentence under it.
    limits_of_code: CreateInviteRequest,
    /// What the choices say; applied by "Create a new code".
    chosen_limits: CreateInviteRequest,
    is_limits_panel_open: bool,
    is_creating: bool,
    has_failed: bool,
    is_copied: bool,
    copied_timer: Option<Task<()>>,
    announcement: SharedString,
    copy_focus: FocusHandle,
    disclosure_focus: FocusHandle,
}

impl InviteBody {
    /// Creates an invite with `limits`; the first one opens with the dialog (Discord's default
    /// limits), later ones only with "Create a new code".
    fn create_invite(&mut self, limits: CreateInviteRequest, cx: &mut Context<Self>) {
        if self.is_creating {
            return;
        }
        let is_replacing_code = self.invite.is_some();
        self.is_creating = true;
        self.has_failed = false;
        if let Some(dialog) = self.dialog.upgrade() {
            dialog.update(cx, |dialog, cx| dialog.set_error(None, cx));
        }
        cx.notify();
        let guild_id = self.guild_id;
        let body = cx.entity().downgrade();
        let sent_limits = limits.clone();
        self.session.update(cx, |session, cx| {
            session.send_settings_request(
                "create an invite",
                move |rest| async move { rest.create_invite(guild_id, &sent_limits).await },
                cx,
                move |answer, cx| {
                    let _ = body.update(cx, |body, cx| {
                        body.is_creating = false;
                        match answer {
                            Ok(invite) => {
                                if is_replacing_code {
                                    body.announcement = strings(cx)
                                        .invite_people()
                                        .new_code_created(&spoken_invite_code(&invite.code))
                                        .into();
                                }
                                body.invite = Some(invite);
                                body.limits_of_code = limits;
                                body.is_copied = false;
                            }
                            Err(error) => {
                                body.has_failed = true;
                                let sentence = describe_error(
                                    &error,
                                    strings(cx).invite_people().failed(),
                                    crate::strings_pending_i18n::strings(cx),
                                );
                                if let Some(dialog) = body.dialog.upgrade() {
                                    dialog.update(cx, |dialog, cx| {
                                        dialog.set_error(Some(sentence.into()), cx);
                                    });
                                }
                            }
                        }
                        cx.notify();
                    });
                },
            );
        });
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let Some(invite) = &self.invite else {
            return;
        };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(invite.code.clone()));
        self.is_copied = true;
        // Said again on a second copy: the region hears a changed value only.
        let copied = strings(cx).invite_people().code_copied();
        self.announcement = if self.announcement.as_ref() == copied {
            format!("{copied}\u{a0}").into()
        } else {
            copied.into()
        };
        self.copied_timer = Some(cx.spawn(async move |body, cx| {
            cx.background_executor()
                .timer(delay(COMPONENT_DIALOG_COPIED_LABEL_DURATION))
                .await;
            let _ = body.update(cx, |body, cx| {
                body.is_copied = false;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn limits_sentence(limits: &CreateInviteRequest, words: Strings) -> String {
        let lifetime = limits
            .expires_in_seconds
            .map(|seconds| duration_label(locale(words), seconds));
        words
            .invite_people()
            .limits(lifetime.as_deref(), limits.max_uses.map(i64::from))
    }

    fn limit_fields(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let catalogue = strings(cx);
        let words = catalogue.invite_limits();
        let lifetime_index = INVITE_LIFETIME_PRESETS_IN_SECONDS
            .iter()
            .position(|preset| *preset == self.chosen_limits.expires_in_seconds)
            .unwrap_or_default();
        let uses_index = INVITE_MAXIMUM_USES_PRESETS
            .iter()
            .position(|preset| *preset == self.chosen_limits.max_uses)
            .unwrap_or_default();
        let lifetime_body = cx.entity().downgrade();
        let uses_body = lifetime_body.clone();
        let field = || div().flex_1().min_w(length(COMPONENT_INVITE_FIELD_BASIS));
        div()
            .flex()
            .flex_wrap()
            .gap(length(COMPONENT_TEXT_FIELD_FIELD_GAP))
            .child(
                field().child(
                    ChoiceGroup::new(
                        "invite-people-lifetime",
                        words.expire_after(),
                        INVITE_LIFETIME_PRESETS_IN_SECONDS
                            .iter()
                            .map(|seconds| {
                                ChoiceOption::new(seconds.map_or_else(
                                    || words.never().to_owned(),
                                    |seconds| duration_label(locale(catalogue), seconds),
                                ))
                            })
                            .collect(),
                        lifetime_index,
                    )
                    .legend(ChoiceLegend::Field)
                    .on_select(move |index, _, cx| {
                        let _ = lifetime_body.update(cx, |body, cx| {
                            if let Some(preset) = INVITE_LIFETIME_PRESETS_IN_SECONDS.get(index) {
                                body.chosen_limits.expires_in_seconds = *preset;
                            }
                            cx.notify();
                        });
                    }),
                ),
            )
            .child(
                field().child(
                    ChoiceGroup::new(
                        "invite-people-uses",
                        words.maximum_uses(),
                        INVITE_MAXIMUM_USES_PRESETS
                            .iter()
                            .map(|uses| {
                                ChoiceOption::new(uses.map_or_else(
                                    || words.no_limit().to_owned(),
                                    |uses| words.uses(i64::from(uses)),
                                ))
                            })
                            .collect(),
                        uses_index,
                    )
                    .legend(ChoiceLegend::Field)
                    .on_select(move |index, _, cx| {
                        let _ = uses_body.update(cx, |body, cx| {
                            if let Some(preset) = INVITE_MAXIMUM_USES_PRESETS.get(index) {
                                body.chosen_limits.max_uses = *preset;
                            }
                            cx.notify();
                        });
                    }),
                ),
            )
    }
}

impl Render for InviteBody {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.invite_people();
        let body = cx.entity().downgrade();
        let well_content: AnyElement = match &self.invite {
            Some(invite) => {
                let mut code = with_type(div(), TYPE_INVITE_CODE)
                    .flex()
                    .text_color(color(tokens.component_invite_code_text));
                for (index, group) in invite_code_groups(&invite.code).into_iter().enumerate() {
                    if index > 0 {
                        code = code.child(
                            div()
                                .text_color(color(tokens.component_invite_code_dash_text))
                                .child("-"),
                        );
                    }
                    code = code.child(group.to_owned());
                }
                div()
                    .id("invite-people-code")
                    .aria_label(SharedString::from(
                        words.code_label(&spoken_invite_code(&invite.code)),
                    ))
                    .child(code)
                    .into_any_element()
            }
            None if self.has_failed && !self.is_creating => {
                Button::new("invite-people-retry", catalogue.common().try_again())
                    .variant(ButtonVariant::Secondary)
                    .on_click(move |_, cx| {
                        let _ = body.update(cx, |body, cx| {
                            let limits = body.limits_of_code.clone();
                            body.create_invite(limits, cx);
                        });
                    })
                    .into_any_element()
            }
            // Inside the code's own line box, so the well keeps its height when the code comes.
            None => with_type(div(), TYPE_INVITE_CODE)
                .child(
                    with_type(div(), TYPE_BODY)
                        .text_color(color(tokens.component_dialog_hint_text))
                        .child(words.creating()),
                )
                .into_any_element(),
        };
        let copy_body = cx.entity().downgrade();
        let toggle_body = cx.entity().downgrade();
        let create_body = cx.entity().downgrade();
        let is_disclosure_ring_shown = is_focus_visible(&self.disclosure_focus, window);
        let limits_panel = self.is_limits_panel_open.then(|| {
            div()
                .flex()
                .flex_col()
                .gap(length(COMPONENT_DIALOG_SECTION_GAP))
                .child(self.limit_fields(cx))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap(length(COMPONENT_DIALOG_BUTTON_GAP))
                        .child(
                            with_type(div(), TYPE_BODY_SM)
                                .text_color(color(tokens.component_dialog_hint_text))
                                .child(words.previous_code_stays()),
                        )
                        .child(
                            Button::new(
                                "invite-people-new-code",
                                if self.is_creating && self.invite.is_some() {
                                    words.creating_new_code()
                                } else {
                                    words.create_new_code()
                                },
                            )
                            .variant(ButtonVariant::Secondary)
                            .disabled(self.is_creating)
                            .on_click(move |_, cx| {
                                let _ = create_body.update(cx, |body, cx| {
                                    let limits = body.chosen_limits.clone();
                                    body.create_invite(limits, cx);
                                });
                            }),
                        ),
                )
        });
        div()
            .relative()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_DIALOG_SECTION_GAP))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_INVITE_STACK_GAP))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(length(COMPONENT_INVITE_CODE_RADIUS))
                            .border(length(COMPONENT_INVITE_CODE_BORDER_WIDTH))
                            .border_color(color(tokens.component_invite_code_border))
                            .bg(color(tokens.component_invite_code_background))
                            .px(length(COMPONENT_INVITE_CODE_PADDING_INLINE))
                            .py(length(COMPONENT_INVITE_CODE_PADDING_BLOCK))
                            .child(well_content),
                    )
                    .child(
                        Button::new(
                            "invite-people-copy",
                            if self.is_copied {
                                words.copied()
                            } else {
                                words.copy()
                            },
                        )
                        .variant(ButtonVariant::Primary)
                        .full_width()
                        .track_focus(&self.copy_focus)
                        .disabled(self.invite.is_none())
                        .on_click(move |_, cx| {
                            let _ = copy_body.update(cx, InviteBody::copy);
                        }),
                    )
                    .when_some(self.invite.as_ref(), |stack, _| {
                        stack.child(
                            with_type(div(), TYPE_BODY_SM)
                                .text_color(color(tokens.component_invite_limits_text))
                                .child(Self::limits_sentence(&self.limits_of_code, catalogue)),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_INVITE_STACK_GAP))
                    .child(
                        with_type(div(), TYPE_BODY)
                            .id("invite-people-change-limits")
                            .role(Role::Button)
                            .aria_label(SharedString::from(words.change_limits()))
                            .aria_expanded(self.is_limits_panel_open)
                            .track_focus(&self.disclosure_focus)
                            .relative()
                            .flex()
                            .items_center()
                            .gap(length(SPACE_2XS))
                            .self_start()
                            .px(length(SPACE_2XS))
                            .rounded(length(RADIUS_XS))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(color(tokens.component_dialog_mode_link_text))
                            .cursor_pointer()
                            .hover(gpui::Styled::underline)
                            .on_click(move |_, _, cx| {
                                let _ = toggle_body.update(cx, |body, cx| {
                                    body.is_limits_panel_open = !body.is_limits_panel_open;
                                    cx.notify();
                                });
                            })
                            .on_key_down(cx.listener(|body, event: &gpui::KeyDownEvent, _, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    cx.stop_propagation();
                                    body.is_limits_panel_open = !body.is_limits_panel_open;
                                    cx.notify();
                                }
                            }))
                            .child(icon(
                                if self.is_limits_panel_open {
                                    IconName::CaretDown
                                } else {
                                    IconName::CaretRight
                                },
                                COMPONENT_CATEGORY_CHEVRON_SIZE,
                                color(tokens.component_dialog_mode_link_text),
                            ))
                            .child(words.change_limits())
                            .when(is_disclosure_ring_shown, |link| {
                                link.child(focus_ring(RADIUS_XS, cx))
                            }),
                    )
                    .children(limits_panel),
            )
            // Out of the flow, so it adds no gap under the limits.
            .child(
                div()
                    .id("invite-people-announcement")
                    .absolute()
                    .size_0()
                    .overflow_hidden()
                    .live_polite(self.announcement.clone()),
            )
    }
}

// ---- "Create a channel" and "Create a category" ----

/// The kinds offered, in order; a category's "+" leaves Category out.
const CHANNEL_KINDS: [ChannelKind; 3] =
    [ChannelKind::Text, ChannelKind::Voice, ChannelKind::Category];

pub(super) struct CreateChannelBody {
    /// From a category's "+": the channel goes in it.
    category: Option<Channel>,
    kind: ChannelKind,
    name: Entity<InputState>,
    topic: Entity<InputState>,
    is_busy: bool,
    /// Drawn again after the title changed: focus goes back to the kind chosen.
    should_focus_kind: bool,
    shell: WeakEntity<DuskShell>,
}

impl CreateChannelBody {
    fn can_submit(&self, cx: &App) -> bool {
        !self.is_busy && !self.name.read(cx).value().trim().is_empty()
    }
}

impl Render for CreateChannelBody {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalogue = strings(cx);
        let words = catalogue.create_channel();
        let voice = catalogue.voice();
        let kinds: Vec<ChannelKind> = CHANNEL_KINDS
            .into_iter()
            .filter(|kind| self.category.is_none() || *kind != ChannelKind::Category)
            .collect();
        let options = kinds
            .iter()
            .map(|kind| match kind {
                ChannelKind::Voice => ChoiceOption::new(voice.voice_channel())
                    .description(voice.voice_channel_description())
                    .icon(IconName::SpeakerHigh),
                ChannelKind::Category => ChoiceOption::new(words.category())
                    .description(words.category_description())
                    .icon(IconName::List),
                ChannelKind::Text => ChoiceOption::new(words.text_channel())
                    .description(words.text_channel_description())
                    .icon(IconName::Hash),
            })
            .collect();
        let checked = kinds
            .iter()
            .position(|kind| *kind == self.kind)
            .unwrap_or_default();
        let should_focus_kind = std::mem::take(&mut self.should_focus_kind);
        let shell = self.shell.clone();
        let is_category = self.kind == ChannelKind::Category;
        let mut name_field = TextField::new(
            "create-channel-name",
            if is_category {
                words.category_name()
            } else {
                words.channel_name()
            },
            &self.name,
        )
        .read_only(self.is_busy);
        if !is_category {
            name_field = name_field.leading_icon(if self.kind == ChannelKind::Voice {
                IconName::SpeakerHigh
            } else {
                IconName::Hash
            });
        }
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_DIALOG_SECTION_GAP))
            .child(
                ChoiceGroup::new(
                    "create-channel-kind",
                    words.channel_type(),
                    options,
                    checked,
                )
                .legend(ChoiceLegend::Field)
                .focus_checked(should_focus_kind)
                .on_select(move |index, window, cx| {
                    let Some(kind) = kinds.get(index).copied() else {
                        return;
                    };
                    let _ = shell.update(cx, |shell, cx| {
                        shell.choose_channel_kind(kind, window, cx);
                    });
                }),
            )
            .child(name_field)
            .when(self.kind == ChannelKind::Text, |fields| {
                fields.child(
                    TextField::new("create-channel-topic", words.topic(), &self.topic)
                        .helper(words.topic_helper())
                        .read_only(self.is_busy),
                )
            })
    }
}

// ---- "Move to category…" ----

pub(super) struct MoveToCategoryBody {
    /// `None` is "No category".
    destinations: Vec<Option<Channel>>,
    current: Option<Snowflake>,
    chosen: Option<Snowflake>,
    is_busy: bool,
    should_focus_choice: bool,
    dialog: WeakEntity<ModalDialog>,
}

impl Render for MoveToCategoryBody {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let words = strings(cx).channel_order();
        let options = self
            .destinations
            .iter()
            .map(|destination| {
                let id = destination.as_ref().map(|category| category.id);
                let option = ChoiceOption::new(destination.as_ref().map_or_else(
                    || words.no_category().to_owned(),
                    |category| category.name.clone(),
                ));
                if id == self.current {
                    option.description(words.current_place())
                } else {
                    option
                }
            })
            .collect();
        let checked = self
            .destinations
            .iter()
            .position(|destination| destination.as_ref().map(|category| category.id) == self.chosen)
            .unwrap_or_default();
        let ids: Vec<Option<Snowflake>> = self
            .destinations
            .iter()
            .map(|destination| destination.as_ref().map(|category| category.id))
            .collect();
        let body = cx.entity().downgrade();
        let should_focus = std::mem::take(&mut self.should_focus_choice);
        ChoiceGroup::new("move-to-category", words.destination(), options, checked)
            .legend(ChoiceLegend::Field)
            .focus_checked(should_focus)
            .on_select(move |index, _, cx| {
                let Some(chosen) = ids.get(index).copied() else {
                    return;
                };
                let _ = body.update(cx, |body, cx| {
                    if body.is_busy {
                        return;
                    }
                    body.chosen = chosen;
                    let is_changed = chosen != body.current;
                    if let Some(dialog) = body.dialog.upgrade() {
                        dialog.update(cx, |dialog, cx| dialog.set_submit_enabled(is_changed, cx));
                    }
                    cx.notify();
                });
            })
    }
}

/// One text field in a dialog (the delete confirmation, a category's new name).
pub(super) struct FieldBody {
    id: &'static str,
    label: SharedString,
    input: Entity<InputState>,
}

impl Render for FieldBody {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        TextField::new(self.id, self.label.clone(), &self.input)
    }
}

// ---- The open dialog ----

#[derive(Clone)]
enum OpenDialog {
    AddServer(Entity<AddServerBody>),
    AnotherInstance,
    InvitePeople,
    CreateChannel {
        guild_id: Snowflake,
        body: Entity<CreateChannelBody>,
    },
    DeleteChannel(Channel),
    MoveToCategory {
        channel: Channel,
        body: Entity<MoveToCategoryBody>,
    },
    RenameCategory {
        category: Channel,
        input: Entity<InputState>,
    },
}

struct ShownDialog {
    dialog: Entity<ModalDialog>,
    kind: OpenDialog,
    subscriptions: Vec<Subscription>,
}

/// What a dialog says and offers.
struct DialogWords {
    title: String,
    subtitle: Option<String>,
    cancel: &'static str,
    /// The action, its busy label and whether it destroys something.
    submit: Option<(String, &'static str, bool)>,
}

/// The rail's "+" menu, the open dialog and a refused move.
#[derive(Default)]
pub(super) struct ServerDialogs {
    shown: Option<ShownDialog>,
    add_menu: Option<(Entity<ContextMenu>, Subscription)>,
    /// Whether the "+" menu was open when the button was pressed: the press closes it before the
    /// click, which must then not open it again.
    was_add_menu_open_at_press: Option<bool>,
    /// What had focus when the dialog first opened, given back to a dialog drawn again.
    opener: Option<FocusHandle>,
    /// Why the last move was refused: the old order is back, and this says why until closed or
    /// the next move (`reorder.error`).
    reorder_error: Option<SharedString>,
    /// A move is on its way: no other starts until it is answered.
    is_reordering: bool,
}

fn input(
    placeholder: Option<&str>,
    maximum_length: Option<usize>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    let placeholder = placeholder.map(str::to_owned);
    cx.new(|cx| {
        let state = InputState::new(window, cx);
        let state = match placeholder {
            Some(placeholder) => state.placeholder(placeholder),
            None => state,
        };
        match maximum_length {
            Some(maximum) => state.validate(move |text, _| text.chars().count() <= maximum),
            None => state,
        }
    })
}

impl DuskShell {
    pub(super) fn is_add_server_menu_open(&self) -> bool {
        self.server_dialogs.add_menu.is_some()
    }

    pub(super) fn note_add_server_press(&mut self) {
        self.server_dialogs.was_add_menu_open_at_press = Some(self.is_add_server_menu_open());
    }

    /// The rail's "+": create a server (when the instance lets the reader), join one with an
    /// invite, or connect to another instance. Beside the button, as a server icon's menu.
    pub(super) fn toggle_add_server_menu(
        &mut self,
        child_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_open = self
            .server_dialogs
            .was_add_menu_open_at_press
            .take()
            .unwrap_or_else(|| self.is_add_server_menu_open());
        if was_open {
            self.server_dialogs.add_menu = None;
            cx.notify();
            return;
        }
        let words = strings(cx);
        let shell = cx.entity().downgrade();
        let may_create = self.state(cx).may_create_guilds();
        let mut entries = Vec::with_capacity(4);
        if may_create {
            let shell = shell.clone();
            entries.push(MenuEntry::Item(MenuItem::new(
                words.add_server().create_title(),
                IconName::PlusCircle,
                move |window, cx| {
                    let _ = shell.update(cx, |shell, cx| shell.open_add_server(false, window, cx));
                },
            )));
        }
        {
            let shell = shell.clone();
            entries.push(MenuEntry::Item(MenuItem::new(
                words.add_server().join_title(),
                IconName::LinkSimple,
                move |window, cx| {
                    let _ = shell.update(cx, |shell, cx| shell.open_add_server(true, window, cx));
                },
            )));
        }
        entries.push(MenuEntry::Separator);
        entries.push(MenuEntry::Item(MenuItem::new(
            connect_another_instance_label(words),
            IconName::ArrowSquareOut,
            move |window, cx| {
                let _ = shell.update(cx, |shell, cx| shell.confirm_another_instance(window, cx));
            },
        )));
        // Beside the button, as a server icon's menu; at the pointer when it is not laid out.
        let origin = if let Some(bounds) = self.regions.rail.scroll.bounds_for_item(child_index) {
            MenuOrigin::Anchor {
                bounds: MenuAnchor {
                    top: f32::from(bounds.top()),
                    bottom: f32::from(bounds.top()),
                    left: f32::from(bounds.right()),
                    right: f32::from(bounds.right()),
                },
                alignment: MenuAlignment::Start,
            }
        } else {
            let pointer = window.mouse_position();
            MenuOrigin::Point {
                x: f32::from(pointer.x),
                y: f32::from(pointer.y),
            }
        };
        let opener = window.focused(cx);
        let menu = cx.new(|cx| {
            ContextMenu::open(
                words.rail().add_server(),
                entries,
                origin,
                opener,
                window,
                cx,
            )
        });
        let subscription = cx.subscribe(&menu, |shell, _, _: &gpui::DismissEvent, cx| {
            shell.server_dialogs.add_menu = None;
            cx.notify();
        });
        self.server_dialogs.add_menu = Some((menu, subscription));
        cx.notify();
    }

    /// Shows a dialog with `words` and `body`; `kind` says what its action does.
    fn show_server_dialog(
        &mut self,
        words: DialogWords,
        body: Option<AnyView>,
        kind: OpenDialog,
        mut subscriptions: Vec<Subscription>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ModalDialog> {
        // A dialog drawn again gives focus back to the first one's opener first, so closing it
        // still lands there.
        match (&self.server_dialogs.shown, &self.server_dialogs.opener) {
            (Some(_), Some(opener)) => window.focus(opener, cx),
            _ => self.server_dialogs.opener = window.focused(cx),
        }
        let dialog = cx.new(|cx| {
            let mut dialog = ModalDialog::new(words.title, words.cancel, window, cx);
            if let Some(subtitle) = words.subtitle {
                dialog = dialog.subtitle(subtitle);
            }
            if let Some((label, busy_label, is_danger)) = words.submit {
                dialog = dialog.submit(label, busy_label, is_danger);
            }
            match body {
                Some(body) => dialog.body(body),
                None => dialog,
            }
        });
        subscriptions.push(cx.subscribe_in(
            &dialog,
            window,
            |shell, dialog, event: &ModalDialogEvent, window, cx| {
                let is_current = shell
                    .server_dialogs
                    .shown
                    .as_ref()
                    .is_some_and(|shown| shown.dialog == *dialog);
                if !is_current {
                    return;
                }
                match event {
                    ModalDialogEvent::Closed => {
                        shell.server_dialogs.shown = None;
                        shell.server_dialogs.opener = None;
                        cx.notify();
                    }
                    ModalDialogEvent::Submitted => shell.submit_server_dialog(window, cx),
                }
            },
        ));
        self.server_dialogs.shown = Some(ShownDialog {
            dialog: dialog.clone(),
            kind,
            subscriptions,
        });
        cx.notify();
        dialog
    }

    /// Enables the action as `can_submit` says, after each keystroke in `input`; Enter submits.
    fn follow_field(
        input: &Entity<InputState>,
        can_submit: impl Fn(&DuskShell, &App) -> bool + 'static,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe(input, move |shell, _, event: &InputEvent, cx| {
            let Some(dialog) = shell
                .server_dialogs
                .shown
                .as_ref()
                .map(|shown| shown.dialog.clone())
            else {
                return;
            };
            match event {
                InputEvent::Change => {
                    let is_enabled = can_submit(shell, cx);
                    dialog.update(cx, |dialog, cx| dialog.set_submit_enabled(is_enabled, cx));
                }
                InputEvent::PressEnter { .. } => dialog.update(cx, ModalDialog::submit_now),
                _ => {}
            }
        })
    }

    /// "Create a server" (`is_joining` false) or "Join a server" (`AddServerDialog.svelte`).
    pub(super) fn open_add_server(
        &mut self,
        is_joining: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let words = strings(cx).add_server();
        let may_create = self.state(cx).may_create_guilds();
        let name = input(None, Some(MAXIMUM_SERVER_NAME_LENGTH), window, cx);
        let code = input(Some(words.invite_code_placeholder()), None, window, cx);
        let shell = cx.entity().downgrade();
        let body = cx.new(|cx| AddServerBody {
            is_joining: is_joining || !may_create,
            may_create,
            name,
            code,
            template_index: 0,
            is_busy: false,
            link_focus: cx.focus_handle().tab_stop(true),
            shell,
        });
        self.server_dialogs.shown = None;
        self.draw_add_server(&body, window, cx);
    }

    fn draw_add_server(
        &mut self,
        body: &Entity<AddServerBody>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let catalogue = strings(cx);
        let words = catalogue.add_server();
        let is_joining = body.read(cx).is_joining;
        let (name, code) = {
            let body = body.read(cx);
            (body.name.clone(), body.code.clone())
        };
        let can_submit = {
            let body = body.downgrade();
            move |_: &DuskShell, cx: &App| {
                body.upgrade()
                    .is_some_and(|body| body.read(cx).can_submit(cx))
            }
        };
        let subscriptions = vec![
            Self::follow_field(&name, can_submit.clone(), cx),
            Self::follow_field(&code, can_submit.clone(), cx),
        ];
        let dialog_words = DialogWords {
            title: if is_joining {
                words.join_title()
            } else {
                words.create_title()
            }
            .to_owned(),
            subtitle: Some(
                if is_joining {
                    words.join_subtitle()
                } else {
                    words.create_subtitle()
                }
                .to_owned(),
            ),
            cancel: catalogue.common().cancel(),
            submit: Some(if is_joining {
                (words.join().to_owned(), words.joining(), false)
            } else {
                (words.create().to_owned(), words.creating(), false)
            }),
        };
        let dialog = self.show_server_dialog(
            dialog_words,
            Some(body.clone().into()),
            OpenDialog::AddServer(body.clone()),
            subscriptions,
            window,
            cx,
        );
        let is_enabled = can_submit(self, cx);
        dialog.update(cx, |dialog, cx| dialog.set_submit_enabled(is_enabled, cx));
        let field = body.read(cx).field();
        field.update(cx, |input, cx| input.focus(window, cx));
    }

    /// "Already have an invite? Join a server", and back.
    fn switch_add_server_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ShownDialog {
            kind: OpenDialog::AddServer(body),
            ..
        }) = &self.server_dialogs.shown
        else {
            return;
        };
        let body = body.clone();
        if body.read(cx).is_busy {
            return;
        }
        body.update(cx, |body, cx| {
            body.is_joining = !body.is_joining;
            cx.notify();
        });
        self.draw_add_server(&body, window, cx);
    }

    /// "Connect to another instance": one session is kept at a time, so this signs out here
    /// (asking first) and the sign-in card opens on the instance address.
    fn confirm_another_instance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let catalogue = strings(cx);
        let words = instance_words(catalogue);
        let instance = self
            .session
            .read(cx)
            .last_instance()
            .map(|instance| {
                let address = instance.as_str();
                address
                    .strip_prefix("https://")
                    .or_else(|| address.strip_prefix("http://"))
                    .unwrap_or(address)
                    .trim_end_matches('/')
                    .to_owned()
            })
            .unwrap_or_default();
        self.server_dialogs.shown = None;
        self.show_server_dialog(
            DialogWords {
                title: words.title.to_owned(),
                subtitle: Some((words.body)(&instance)),
                cancel: catalogue.common().cancel(),
                submit: Some((words.submit.to_owned(), words.signing_out, false)),
            },
            None,
            OpenDialog::AnotherInstance,
            Vec::new(),
            window,
            cx,
        );
    }

    /// "Invite people": a code is created as the dialog opens, with Discord's default limits.
    pub(super) fn open_invite_people(
        &mut self,
        guild_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(server_name) = self
            .state(cx)
            .guild(guild_id)
            .map(|guild| guild.name.clone())
        else {
            return;
        };
        let catalogue = strings(cx);
        let words = catalogue.invite_people();
        let session = self.session.clone();
        let body = cx.new(|cx| InviteBody {
            session,
            guild_id,
            dialog: WeakEntity::new_invalid(),
            invite: None,
            limits_of_code: DEFAULT_INVITE_LIMITS,
            chosen_limits: DEFAULT_INVITE_LIMITS,
            is_limits_panel_open: false,
            is_creating: false,
            has_failed: false,
            is_copied: false,
            copied_timer: None,
            announcement: SharedString::default(),
            copy_focus: cx.focus_handle().tab_stop(true),
            disclosure_focus: cx.focus_handle().tab_stop(true),
        });
        self.server_dialogs.shown = None;
        let dialog = self.show_server_dialog(
            DialogWords {
                title: words.title(&server_name),
                subtitle: Some(words.subtitle().to_owned()),
                cancel: catalogue.common().close(),
                submit: None,
            },
            Some(body.clone().into()),
            OpenDialog::InvitePeople,
            Vec::new(),
            window,
            cx,
        );
        body.update(cx, |body, cx| {
            body.dialog = dialog.downgrade();
            window.focus(&body.copy_focus, cx);
            body.create_invite(DEFAULT_INVITE_LIMITS, cx);
        });
    }

    /// "Create a channel", in `category` from its "+"; "Create a category" opens on Category.
    pub(super) fn open_create_channel(
        &mut self,
        guild_id: Snowflake,
        category: Option<Snowflake>,
        initial_kind: ChannelKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let category = category.and_then(|id| self.state(cx).guild_channel(id).cloned());
        let kind = if category.is_some() {
            ChannelKind::Text
        } else {
            initial_kind
        };
        let name = input(None, Some(MAXIMUM_CHANNEL_NAME_LENGTH), window, cx);
        let topic = input(None, Some(MAXIMUM_CHANNEL_TOPIC_LENGTH), window, cx);
        let shell = cx.entity().downgrade();
        let body = cx.new(|_| CreateChannelBody {
            category,
            kind,
            name,
            topic,
            is_busy: false,
            should_focus_kind: false,
            shell,
        });
        self.server_dialogs.shown = None;
        self.draw_create_channel(guild_id, &body, window, cx);
        let name = body.read(cx).name.clone();
        name.update(cx, |input, cx| input.focus(window, cx));
    }

    fn draw_create_channel(
        &mut self,
        guild_id: Snowflake,
        body: &Entity<CreateChannelBody>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let catalogue = strings(cx);
        let words = catalogue.create_channel();
        let (is_category, subtitle, name) = {
            let body = body.read(cx);
            (
                body.kind == ChannelKind::Category,
                body.category
                    .as_ref()
                    .map(|category| words.in_category(&category.name)),
                body.name.clone(),
            )
        };
        let can_submit = {
            let body = body.downgrade();
            move |_: &DuskShell, cx: &App| {
                body.upgrade()
                    .is_some_and(|body| body.read(cx).can_submit(cx))
            }
        };
        let topic = body.read(cx).topic.clone();
        let subscriptions = vec![
            Self::follow_field(&name, can_submit.clone(), cx),
            Self::follow_field(&topic, can_submit.clone(), cx),
        ];
        let dialog = self.show_server_dialog(
            DialogWords {
                title: if is_category {
                    words.title_category()
                } else {
                    words.title()
                }
                .to_owned(),
                subtitle,
                cancel: catalogue.common().cancel(),
                submit: Some((
                    if is_category {
                        words.submit_category()
                    } else {
                        words.submit()
                    }
                    .to_owned(),
                    words.creating(),
                    false,
                )),
            },
            Some(body.clone().into()),
            OpenDialog::CreateChannel {
                guild_id,
                body: body.clone(),
            },
            subscriptions,
            window,
            cx,
        );
        let is_enabled = can_submit(self, cx);
        dialog.update(cx, |dialog, cx| dialog.set_submit_enabled(is_enabled, cx));
    }

    /// A kind chosen in "Create a channel": the title follows when Category comes or goes.
    fn choose_channel_kind(
        &mut self,
        kind: ChannelKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ShownDialog {
            kind: OpenDialog::CreateChannel { guild_id, body },
            ..
        }) = &self.server_dialogs.shown
        else {
            return;
        };
        let (guild_id, body) = (*guild_id, body.clone());
        let was_category = body.read(cx).kind == ChannelKind::Category;
        if body.read(cx).is_busy {
            return;
        }
        body.update(cx, |body, cx| {
            body.kind = kind;
            cx.notify();
        });
        if was_category != (kind == ChannelKind::Category) {
            body.update(cx, |body, _| body.should_focus_kind = true);
            self.draw_create_channel(guild_id, &body, window, cx);
        }
    }

    /// "Delete channel" asks for the name first; "Delete category" only to confirm.
    pub(super) fn confirm_delete_channel(
        &mut self,
        channel_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(channel) = self.state(cx).guild_channel(channel_id).cloned() else {
            return;
        };
        let catalogue = strings(cx);
        let words = catalogue.channel_removal();
        let is_category = channel.kind == ChannelKind::Category;
        let confirmation = (!is_category).then(|| input(None, None, window, cx));
        let mut subscriptions = Vec::new();
        let body: Option<AnyView> = confirmation.as_ref().map(|confirmation| {
            let name = channel.name.clone();
            let field = confirmation.downgrade();
            subscriptions.push(Self::follow_field(
                confirmation,
                move |_, cx| {
                    field
                        .upgrade()
                        .is_some_and(|field| field.read(cx).value().trim() == name)
                },
                cx,
            ));
            cx.new(|_| FieldBody {
                id: "delete-channel-name",
                label: words.confirm_label(&channel.name).into(),
                input: confirmation.clone(),
            })
            .into()
        });
        self.server_dialogs.shown = None;
        let dialog = self.show_server_dialog(
            DialogWords {
                title: if is_category {
                    words.category_title(&channel.name)
                } else {
                    words.channel_title(&channel.name)
                },
                subtitle: Some(
                    if is_category {
                        words.category_body()
                    } else {
                        words.channel_body()
                    }
                    .to_owned(),
                ),
                cancel: catalogue.common().cancel(),
                submit: Some((
                    if is_category {
                        words.delete_category()
                    } else {
                        words.delete_channel()
                    }
                    .to_owned(),
                    words.deleting(),
                    true,
                )),
            },
            body,
            OpenDialog::DeleteChannel(channel),
            subscriptions,
            window,
            cx,
        );
        if confirmation.is_some() {
            dialog.update(cx, |dialog, cx| dialog.set_submit_enabled(false, cx));
        }
    }

    /// "Move to category…": to the end of the one chosen.
    pub(super) fn open_move_to_category(
        &mut self,
        channel_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = self.state(cx);
        let Some(channel) = state.guild_channel(channel_id).cloned() else {
            return;
        };
        let Some(guild) = state.guild_of_channel(channel_id) else {
            return;
        };
        let sections = channel_sections(&guild.channels);
        let current = sections
            .iter()
            .find(|section| {
                section
                    .channels
                    .iter()
                    .any(|listed| listed.id == channel_id)
            })
            .and_then(|section| section.category.as_ref().map(|category| category.id));
        let destinations: Vec<Option<Channel>> = std::iter::once(None)
            .chain(
                sections
                    .iter()
                    .filter_map(|section| section.category.clone().map(Some)),
            )
            .collect();
        let catalogue = strings(cx);
        let words = catalogue.channel_order();
        let body = cx.new(|_| MoveToCategoryBody {
            destinations,
            current,
            chosen: current,
            is_busy: false,
            should_focus_choice: true,
            dialog: WeakEntity::new_invalid(),
        });
        self.server_dialogs.shown = None;
        let dialog = self.show_server_dialog(
            DialogWords {
                title: words.move_title(&channel.name),
                subtitle: Some(words.move_subtitle().to_owned()),
                cancel: catalogue.common().cancel(),
                submit: Some((words.r#move().to_owned(), words.moving(), false)),
            },
            Some(body.clone().into()),
            OpenDialog::MoveToCategory {
                channel,
                body: body.clone(),
            },
            Vec::new(),
            window,
            cx,
        );
        dialog.update(cx, |dialog, cx| dialog.set_submit_enabled(false, cx));
        body.update(cx, |body, _| body.dialog = dialog.downgrade());
    }

    /// "Rename category".
    pub(super) fn open_rename_category(
        &mut self,
        category_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(category) = self.state(cx).guild_channel(category_id).cloned() else {
            return;
        };
        let catalogue = strings(cx);
        let field = input(None, Some(MAXIMUM_CHANNEL_NAME_LENGTH), window, cx);
        field.update(cx, |field, cx| {
            field.set_value(category.name.clone(), window, cx);
        });
        let current_name = category.name.clone();
        let watched = field.downgrade();
        let subscriptions = vec![Self::follow_field(
            &field,
            move |_, cx| {
                watched.upgrade().is_some_and(|field| {
                    let typed = field.read(cx).value();
                    let typed = typed.trim();
                    !typed.is_empty() && typed != current_name
                })
            },
            cx,
        )];
        let body = cx.new(|_| FieldBody {
            id: "rename-category-name",
            label: catalogue.create_channel().category_name().into(),
            input: field.clone(),
        });
        self.server_dialogs.shown = None;
        let dialog = self.show_server_dialog(
            DialogWords {
                title: catalogue.channel_removal().rename_category().to_owned(),
                subtitle: None,
                cancel: catalogue.common().cancel(),
                submit: Some((
                    catalogue.common().save().to_owned(),
                    catalogue.channel_removal().renaming(),
                    false,
                )),
            },
            Some(body.into()),
            OpenDialog::RenameCategory {
                category,
                input: field.clone(),
            },
            subscriptions,
            window,
            cx,
        );
        dialog.update(cx, |dialog, cx| dialog.set_submit_enabled(false, cx));
        field.update(cx, |field, cx| field.focus(window, cx));
    }

    /// The open dialog's action.
    fn submit_server_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(shown) = &self.server_dialogs.shown else {
            return;
        };
        let dialog = shown.dialog.clone();
        let catalogue = strings(cx);
        dialog.update(cx, |dialog, cx| {
            dialog.set_error(None, cx);
            dialog.set_busy(true, cx);
        });
        let kind = shown.kind.clone();
        let finish = Self::dialog_finisher(&dialog, window, cx);
        match kind {
            OpenDialog::AddServer(body) => self.submit_add_server(&body, finish, cx),
            OpenDialog::AnotherInstance => self.sign_out_for_another_instance(&dialog, cx),
            OpenDialog::InvitePeople => {}
            OpenDialog::CreateChannel { guild_id, body } => {
                self.submit_create_channel(guild_id, &body, finish, cx);
            }
            OpenDialog::DeleteChannel(channel) => {
                let channel_id = channel.id;
                let words = catalogue.channel_removal();
                let (failure, done) = if channel.kind == ChannelKind::Category {
                    (
                        words.category_delete_failed(),
                        words.category_deleted(&channel.name),
                    )
                } else {
                    (
                        words.channel_delete_failed(),
                        words.channel_deleted(&channel.name),
                    )
                };
                self.session.update(cx, |session, cx| {
                    session.send_settings_change(
                        "delete a channel",
                        failure.to_owned(),
                        move |rest| async move { rest.delete_channel(channel_id).await },
                        cx,
                        move |answer, cx| {
                            finish(answer.map(|()| ServerDialogOutcome::Done(done)), cx);
                        },
                    );
                });
            }
            OpenDialog::MoveToCategory { channel, body } => {
                let chosen = body.read(cx).chosen;
                body.update(cx, |body, cx| {
                    body.is_busy = true;
                    cx.notify();
                });
                let watched = body.downgrade();
                self.move_channel_to(
                    &channel,
                    chosen,
                    Box::new(move |answer, cx| {
                        let _ = watched.update(cx, |body, cx| {
                            body.is_busy = false;
                            cx.notify();
                        });
                        finish(answer.map(ServerDialogOutcome::Done), cx);
                    }),
                    cx,
                );
            }
            OpenDialog::RenameCategory { category, input } => {
                let category_id = category.id;
                let name = channel_name_as_saved(input.read(cx).value().trim()).to_owned();
                let words = catalogue.channel_removal();
                let done = words.category_renamed(&name);
                self.session.update(cx, |session, cx| {
                    session.send_settings_change(
                        "rename a category",
                        words.rename_failed().to_owned(),
                        move |rest| async move {
                            rest.update_channel(
                                category_id,
                                &UpdateChannelRequest {
                                    name: Some(name),
                                    ..UpdateChannelRequest::default()
                                },
                            )
                            .await
                        },
                        cx,
                        move |answer, cx| {
                            finish(answer.map(|()| ServerDialogOutcome::Done(done)), cx);
                        },
                    );
                });
            }
        }
    }

    /// Creates or joins the server the dialog describes; the new server opens.
    fn submit_add_server(
        &mut self,
        body: &Entity<AddServerBody>,
        finish: Finisher,
        cx: &mut Context<Self>,
    ) {
        let catalogue = strings(cx);
        let (is_joining, typed, template_index) = {
            let body = body.read(cx);
            let field = if body.is_joining {
                &body.code
            } else {
                &body.name
            };
            (
                body.is_joining,
                field.read(cx).value().trim().to_owned(),
                body.template_index,
            )
        };
        body.update(cx, |body, cx| {
            body.is_busy = true;
            cx.notify();
        });
        let on_done = {
            let body = body.downgrade();
            move |answer: Result<Snowflake, String>, cx: &mut App| {
                let _ = body.update(cx, |body, cx| {
                    body.is_busy = false;
                    cx.notify();
                });
                finish(answer.map(ServerDialogOutcome::OpenServer), cx);
            }
        };
        let words = catalogue.add_server();
        self.session.update(cx, |session, cx| {
            if is_joining {
                session.join_server(typed, words.join_failed().to_owned(), cx, on_done);
            } else {
                let categories = initial_categories_of(catalogue, template_index);
                session.create_server(
                    typed,
                    Some(categories),
                    words.create_failed().to_owned(),
                    cx,
                    on_done,
                );
            }
        });
    }

    /// Creates the channel or category the dialog describes; a text channel opens.
    fn submit_create_channel(
        &mut self,
        guild_id: Snowflake,
        body: &Entity<CreateChannelBody>,
        finish: Finisher,
        cx: &mut Context<Self>,
    ) {
        let catalogue = strings(cx);
        let request = {
            let body = body.read(cx);
            CreateChannelRequest {
                kind: body.kind,
                name: body.name.read(cx).value().trim().to_owned(),
                topic: if body.kind == ChannelKind::Text {
                    body.topic.read(cx).value().trim().to_owned()
                } else {
                    String::new()
                },
                parent_id: if body.kind == ChannelKind::Category {
                    None
                } else {
                    body.category.as_ref().map(|category| category.id)
                },
                user_limit: None,
            }
        };
        let failure = if request.kind == ChannelKind::Category {
            catalogue.create_channel().failed_category()
        } else {
            catalogue.create_channel().failed()
        };
        body.update(cx, |body, cx| {
            body.is_busy = true;
            cx.notify();
        });
        let watched = body.downgrade();
        self.session.update(cx, |session, cx| {
            session.create_channel(
                guild_id,
                request,
                failure.to_owned(),
                cx,
                move |answer, cx| {
                    let _ = watched.update(cx, |body, cx| {
                        body.is_busy = false;
                        cx.notify();
                    });
                    // A text channel opens once created; a category or a voice channel
                    // does not.
                    finish(
                        answer.map(|channel| {
                            if channel.kind == ChannelKind::Text {
                                ServerDialogOutcome::OpenChannel(guild_id, channel.id)
                            } else {
                                ServerDialogOutcome::Done(String::new())
                            }
                        }),
                        cx,
                    );
                },
            );
        });
    }

    /// "Sign out and continue": the shell goes with the session; a refusal stays in the dialog.
    fn sign_out_for_another_instance(
        &mut self,
        dialog: &Entity<ModalDialog>,
        cx: &mut Context<Self>,
    ) {
        self.session
            .update(cx, Session::sign_out_for_another_instance);
        let observed = dialog.downgrade();
        let subscription = cx.observe(&self.session, move |_, session, cx| {
            let session = session.read(cx);
            if session.is_signing_out() {
                return;
            }
            let error = session.sign_out_error().map(str::to_owned);
            let _ = observed.update(cx, |dialog, cx| {
                dialog.set_busy(false, cx);
                dialog.set_error(error.map(SharedString::from), cx);
            });
        });
        if let Some(shown) = &mut self.server_dialogs.shown {
            shown.subscriptions.push(subscription);
        }
    }

    /// What closes `dialog` once its action answered: a failure stays in it; a success closes
    /// it and goes where the action leads.
    fn dialog_finisher(
        dialog: &Entity<ModalDialog>,
        window: &Window,
        cx: &Context<Self>,
    ) -> Finisher {
        let dialog = dialog.downgrade();
        let shell = cx.entity().downgrade();
        let window_handle = window.window_handle();
        Rc::new(move |answer, cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = dialog.update(cx, |dialog, cx| {
                    dialog.set_busy(false, cx);
                    match &answer {
                        Ok(_) => dialog.request_close(window, cx),
                        Err(message) => dialog.set_error(Some(message.clone().into()), cx),
                    }
                });
                let Ok(outcome) = answer else {
                    return;
                };
                let _ = shell.update(cx, |shell, cx| match outcome {
                    ServerDialogOutcome::OpenServer(guild_id) => {
                        shell.open_place(Some(guild_id), None, window, cx);
                        shell.focus_composer(window, cx);
                    }
                    ServerDialogOutcome::OpenChannel(guild_id, channel_id) => {
                        shell.open_place(Some(guild_id), Some(channel_id), window, cx);
                        shell.focus_composer(window, cx);
                    }
                    ServerDialogOutcome::Done(sentence) => {
                        if !sentence.is_empty() {
                            shell.announce(sentence, cx);
                        }
                    }
                });
            });
        })
    }

    fn focus_composer(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.conversation.update(cx, |conversation, cx| {
            conversation.focus_composer(window, cx);
        });
    }

    // ---- Moves (`channel-reorder.svelte.ts`) ----

    /// Whether the open server's order may change now: the reader manages channels and no move
    /// is on its way.
    pub(super) fn can_reorder_now(&self, guild_id: Snowflake, cx: &App) -> bool {
        !self.server_dialogs.is_reordering && self.can_manage_server_channels(guild_id, cx)
    }

    /// Whether the reader creates and orders the server's channels (`canCreateChannels`: the
    /// owner and administrators hold every permission).
    pub(super) fn can_manage_server_channels(&self, guild_id: Snowflake, cx: &App) -> bool {
        self.state(cx).guild(guild_id).is_some_and(|guild| {
            guild.my_permissions.contains(Permissions::ADMINISTRATOR)
                || guild.my_permissions.contains(Permissions::MANAGE_CHANNELS)
        })
    }

    fn sections_of(&self, guild_id: Snowflake, cx: &App) -> Vec<ChannelSection> {
        self.state(cx)
            .guild(guild_id)
            .map(|guild| channel_sections(&guild.channels))
            .unwrap_or_default()
    }

    /// Whether `channel_id` can go one step `step` (the menu's "Move up" and "Move down").
    pub(super) fn can_move_channel(
        &self,
        guild_id: Snowflake,
        channel_id: Snowflake,
        step: Step,
        cx: &App,
    ) -> bool {
        self.can_reorder_now(guild_id, cx)
            && channel_step_destination(&self.sections_of(guild_id, cx), channel_id, step).is_some()
    }

    pub(super) fn can_move_category(
        &self,
        guild_id: Snowflake,
        category_id: Snowflake,
        step: Step,
        cx: &App,
    ) -> bool {
        self.can_reorder_now(guild_id, cx)
            && category_step_index(&self.sections_of(guild_id, cx), category_id, step).is_some()
    }

    /// Whether the server has a category to move a channel into.
    pub(super) fn has_categories(&self, guild_id: Snowflake, cx: &App) -> bool {
        self.sections_of(guild_id, cx)
            .iter()
            .any(|section| section.category.is_some())
    }

    pub(super) fn move_channel_step(
        &mut self,
        channel_id: Snowflake,
        step: Step,
        cx: &mut Context<Self>,
    ) {
        let state = self.state(cx);
        let (Some(guild_id), Some(name)) = (
            state.guild_of_channel(channel_id).map(|guild| guild.id),
            state
                .guild_channel(channel_id)
                .map(|channel| channel.name.clone()),
        ) else {
            return;
        };
        let sections = self.sections_of(guild_id, cx);
        let Some(next) = channel_step_destination(&sections, channel_id, step)
            .and_then(|destination| with_channel_moved(&sections, channel_id, destination))
        else {
            return;
        };
        let words = strings(cx).channel_order();
        let sentence = if step == Step::Up {
            words.channel_moved_up(&name)
        } else {
            words.channel_moved_down(&name)
        };
        self.send_order(
            guild_id,
            &next,
            words.channel_move_failed(),
            sentence,
            None,
            cx,
        );
    }

    pub(super) fn move_category_step(
        &mut self,
        category_id: Snowflake,
        step: Step,
        cx: &mut Context<Self>,
    ) {
        let state = self.state(cx);
        let (Some(guild_id), Some(name)) = (
            state.guild_of_channel(category_id).map(|guild| guild.id),
            state
                .guild_channel(category_id)
                .map(|category| category.name.clone()),
        ) else {
            return;
        };
        let sections = self.sections_of(guild_id, cx);
        let Some(next) = category_step_index(&sections, category_id, step)
            .and_then(|index| with_category_moved(&sections, category_id, index))
        else {
            return;
        };
        let words = strings(cx).channel_order();
        let sentence = if step == Step::Up {
            words.category_moved_up(&name)
        } else {
            words.category_moved_down(&name)
        };
        self.send_order(
            guild_id,
            &next,
            words.category_move_failed(),
            sentence,
            None,
            cx,
        );
    }

    /// "Move to category…": to the end of `category_id` (`None`: out of any).
    fn move_channel_to(
        &mut self,
        channel: &Channel,
        category_id: Option<Snowflake>,
        on_done: DialogAnswer,
        cx: &mut Context<Self>,
    ) {
        let Some(guild_id) = self
            .state(cx)
            .guild_of_channel(channel.id)
            .map(|guild| guild.id)
        else {
            return;
        };
        let sections = self.sections_of(guild_id, cx);
        let index = sections
            .iter()
            .find(|section| section.category.as_ref().map(|category| category.id) == category_id)
            .map_or(0, |section| section.channels.len());
        let Some(next) = with_channel_moved(
            &sections,
            channel.id,
            ChannelDestination {
                category_id,
                index,
                leaves_hidden_category: true,
            },
        ) else {
            on_done(Ok(String::new()), cx);
            return;
        };
        let words = strings(cx).channel_order();
        let sentence = match sections
            .iter()
            .filter_map(|section| section.category.as_ref())
            .find(|category| Some(category.id) == category_id)
        {
            Some(category) => words.channel_moved_to(&channel.name, &category.name),
            None => words.channel_moved_out(&channel.name),
        };
        self.send_order(
            guild_id,
            &next,
            words.channel_move_failed(),
            sentence,
            Some(on_done),
            cx,
        );
    }

    /// Sends the whole new order. From a menu (`dialog_answer` `None`), success reads
    /// `sentence` out and a refusal shows under the header; a dialog gets both itself.
    fn send_order(
        &mut self,
        guild_id: Snowflake,
        next: &[ChannelSection],
        failure_summary: &'static str,
        sentence: String,
        dialog_answer: Option<DialogAnswer>,
        cx: &mut Context<Self>,
    ) {
        let placements = placements_of(next);
        self.server_dialogs.is_reordering = true;
        self.server_dialogs.reorder_error = None;
        cx.notify();
        let shell = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_settings_change(
                "reorder channels",
                failure_summary.to_owned(),
                move |rest| async move { rest.reorder_channels(guild_id, &placements).await },
                cx,
                move |answer, cx| {
                    let answer = answer.map(|()| sentence);
                    let _ = shell.update(cx, |shell, cx| {
                        shell.server_dialogs.is_reordering = false;
                        if dialog_answer.is_none() {
                            match &answer {
                                Ok(sentence) => shell.announce(sentence.clone(), cx),
                                Err(error) => {
                                    shell.server_dialogs.reorder_error = Some(error.clone().into());
                                }
                            }
                        }
                        cx.notify();
                    });
                    if let Some(dialog_answer) = dialog_answer {
                        dialog_answer(answer, cx);
                    }
                },
            );
        });
    }

    /// Why the last move was refused, under the server header, with a close button.
    pub(super) fn render_reorder_error(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let error = self.server_dialogs.reorder_error.clone()?;
        let tokens = theme(cx);
        let entity = cx.entity().downgrade();
        Some(
            with_type(div(), TYPE_BODY_SM)
                .id("reorder-error")
                .live_assertive(error.clone())
                .mt(length(SPACE_XS))
                .mr(length(COMPONENT_CHANNEL_ROW_MARGIN_INLINE))
                .pl(length(COMPONENT_CHANNEL_ROW_PADDING_INLINE))
                .flex()
                .items_start()
                .gap(length(SPACE_XS))
                .text_color(color(tokens.color_text_danger))
                .child(icon(
                    IconName::WarningCircle,
                    COMPONENT_CHANNEL_ROW_TRAILING_ICON_SIZE,
                    color(tokens.color_text_danger),
                ))
                .child(div().flex_1().min_w_0().child(error))
                .child(
                    IconButton::new(
                        "reorder-error-close",
                        IconName::X,
                        strings(cx).common().close(),
                    )
                    .sizes(
                        COMPONENT_CHANNEL_ROW_SETTINGS_BUTTON_SIZE,
                        COMPONENT_CHANNEL_ROW_SETTINGS_ICON_SIZE,
                    )
                    .on_click(move |_, cx| {
                        let _ = entity.update(cx, |shell, cx| {
                            shell.server_dialogs.reorder_error = None;
                            cx.notify();
                        });
                    }),
                )
                .into_any_element(),
        )
    }

    /// The "+" menu and the open dialog, drawn over the shell.
    pub(super) fn render_server_dialogs(&self) -> Vec<AnyElement> {
        self.server_dialogs
            .add_menu
            .as_ref()
            .map(|(menu, _)| menu.clone().into_any_element())
            .into_iter()
            .chain(
                self.server_dialogs
                    .shown
                    .as_ref()
                    .map(|shown| shown.dialog.clone().into_any_element()),
            )
            .collect()
    }
}

/// What closes a dialog once its action answered, and goes where it leads.
type Finisher = Rc<dyn Fn(Result<ServerDialogOutcome, String>, &mut App)>;

/// What a dialog waiting on a move hears: the sentence to read out, or why it failed.
type DialogAnswer = Box<dyn FnOnce(Result<String, String>, &mut App)>;

/// Where a dialog's success leads.
enum ServerDialogOutcome {
    OpenServer(Snowflake),
    OpenChannel(Snowflake, Snowflake),
    /// Stays here; the sentence (if any) is read out.
    Done(String),
}

#[cfg(test)]
#[path = "server_dialogs_tests.rs"]
mod tests;
