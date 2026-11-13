//! The hidden component gallery (`MELLOW_COMPONENT_GALLERY=1`, or `=dark` / `=light` to force a
//! theme): every primitive in its states, and sample messages, to compare with the web's
//! components side by side.
//! Developer-facing, so its own words are English and not translated.

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, MouseButton, MouseDownEvent, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, WindowAppearance, div,
};
use gpui_component::input::InputState;

use super::avatar::AvatarPerson;
use super::badge::RoleChip;
use super::menu::{MenuAlignment, MenuAnchor};
use super::range_field::RangeBounds;
use super::settings_list_status::SettingsListStatusKind;
use super::text_field::PasswordToggle;
use super::unsaved_bar::UnsavedChangesLabels;
use super::{
    Avatar, Button, ButtonSize, ButtonVariant, ChoiceGroup, ChoiceOption, ContextMenu, GroupAvatar,
    IconButton, IconButtonTone, MentionBadge, MenuEntry, MenuItem, MenuItemTone, MenuOrigin,
    ModalDialog, ModalDialogEvent, ProgressMeter, RangeField, RoleChips, SettingsListStatus,
    SettingsNavEntry, SettingsNavItem, SettingsSheet, SettingsSheetLabels, ShownStatus,
    StatusIndicator, Switch, Tabs, TextField, Tooltip, UnsavedChangesBar, with_tab_navigation,
    with_type,
};
use crate::composer::{Composer, ReplyTarget};
use crate::messages::samples::{MessageSamples, SampleActionHandler};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_MEMBER_ROW_AVATAR_SIZE, COMPONENT_MESSAGE_AVATAR_SIZE,
    COMPONENT_MESSAGE_REPLY_AVATAR_SIZE, COMPONENT_PROFILE_POPOVER_AVATAR_SIZE,
    COMPONENT_STATUS_SIZE_LARGE, COMPONENT_STATUS_SIZE_MENU, COMPONENT_STATUS_SIZE_SMALL, SPACE_LG,
    SPACE_MD, SPACE_SM, SPACE_XL, TYPE_CATEGORY, TYPE_HEADING_LG,
};
use crate::theme::{ActiveTheme, SANS_FONT_FAMILY, apply_theme, color, length, theme};
use crate::window_chrome::{TITLE_BAR_BAND_HEIGHT, main_window_options};

/// The environment variable that opens the gallery instead of the app.
pub const COMPONENT_GALLERY_VARIABLE: &str = "MELLOW_COMPONENT_GALLERY";

/// How long the sample dialog stays busy before it shows its error.
const SAMPLE_DIALOG_WORK: Duration = Duration::from_millis(1500);

/// Which theme the gallery shows: the OS's, or the one the variable forces.
#[must_use]
pub fn forced_appearance(variable_value: &str) -> Option<WindowAppearance> {
    match variable_value {
        "dark" => Some(WindowAppearance::Dark),
        "light" => Some(WindowAppearance::Light),
        _ => None,
    }
}

pub struct ComponentGallery {
    switch_on: bool,
    range_value: f32,
    choice_index: usize,
    tab_index: usize,
    unsaved_attention: u32,
    plain_field: Entity<InputState>,
    helper_field: Entity<InputState>,
    error_field: Entity<InputState>,
    password_field: Entity<InputState>,
    composer: Entity<Composer>,
    menu: Option<(Entity<ContextMenu>, Subscription)>,
    dialog: Option<(Entity<ModalDialog>, Subscription)>,
    /// The sample settings sheet's chosen section, while it is open.
    settings_section: Option<usize>,
    settings_nav_focus: [FocusHandle; SAMPLE_SETTINGS_SECTIONS.len()],
    settings_container_focus: FocusHandle,
    settings_page_focus: FocusHandle,
    menu_button_focus: FocusHandle,
    root_focus: FocusHandle,
    messages: MessageSamples,
}

impl ComponentGallery {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = |placeholder: &'static str,
                     value: &'static str,
                     window: &mut Window,
                     cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            })
        };
        let plain_field = field("Search members", "", window, cx);
        let helper_field = field("", "Rust Lyon", window, cx);
        let error_field = field("", "rust lyon!", window, cx);
        let password_field = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .default_value("correct horse")
        });
        let composer = cx.new(|cx| {
            let mut composer = Composer::new(window, cx);
            composer.set_channel(
                Some(mellow_protocol::Snowflake::from_raw(1)),
                "Message #general".into(),
                window,
                cx,
            );
            composer.set_mention_candidates(
                ["ana", "anouk", "léo"]
                    .into_iter()
                    .zip(2..)
                    .map(|(name, id)| mellow_protocol::PublicUser {
                        id: mellow_protocol::Snowflake::from_raw(id),
                        username: name.to_owned(),
                        display_name: String::new(),
                        avatar: None,
                        banner: None,
                    })
                    .collect(),
                Some(mellow_protocol::Snowflake::from_raw(1)),
                cx,
            );
            composer.set_reply_target(
                Some(ReplyTarget {
                    message_id: mellow_protocol::Snowflake::from_raw(9),
                    author_name: "Ana".into(),
                }),
                window,
                cx,
            );
            composer
        });
        Self {
            switch_on: true,
            range_value: 100.0,
            choice_index: 0,
            tab_index: 0,
            unsaved_attention: 0,
            plain_field,
            helper_field,
            error_field,
            password_field,
            composer,
            menu: None,
            dialog: None,
            settings_section: None,
            settings_nav_focus: std::array::from_fn(|_| cx.focus_handle()),
            settings_container_focus: cx.focus_handle(),
            settings_page_focus: cx.focus_handle(),
            menu_button_focus: cx.focus_handle().tab_stop(true),
            root_focus: cx.focus_handle(),
            messages: MessageSamples::new(),
        }
    }

    fn open_menu(&mut self, origin: MenuOrigin, window: &mut Window, cx: &mut Context<Self>) {
        let opener = Some(self.menu_button_focus.clone());
        let entries = vec![
            MenuEntry::Item(
                MenuItem::new("Invite people", IconName::UserPlus, |_, _| {})
                    .tone(MenuItemTone::Accent),
            ),
            MenuEntry::Item(MenuItem::new("Copy link", IconName::Copy, |_, _| {})),
            MenuEntry::Item(MenuItem::new("Épingler", IconName::PushPin, |_, _| {})),
            MenuEntry::Item(MenuItem::new("Edit", IconName::PencilSimple, |_, _| {})),
            MenuEntry::Item(MenuItem::new("Mute", IconName::BellSlash, |_, _| {}).disabled(true)),
            MenuEntry::Separator,
            MenuEntry::Item(
                MenuItem::new("Delete message", IconName::Trash, |_, _| {})
                    .tone(MenuItemTone::Danger),
            ),
        ];
        let menu =
            cx.new(|cx| ContextMenu::open("Message actions", entries, origin, opener, window, cx));
        let subscription = cx.subscribe(&menu, |gallery, _, _: &gpui::DismissEvent, cx| {
            gallery.menu = None;
            cx.notify();
        });
        self.menu = Some((menu, subscription));
        cx.notify();
    }

    fn open_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| {
            ModalDialog::new("Delete #general?", "Cancel", window, cx)
                .subtitle("Its messages are deleted for everyone. This cannot be undone.")
                .submit("Delete channel", "Deleting…", true)
        });
        let subscription = cx.subscribe_in(
            &dialog,
            window,
            |gallery, dialog, event: &ModalDialogEvent, _, cx| match event {
                ModalDialogEvent::Closed => {
                    gallery.dialog = None;
                    cx.notify();
                }
                ModalDialogEvent::Submitted => {
                    dialog.update(cx, |dialog, cx| {
                        dialog.set_error(None, cx);
                        dialog.set_busy(true, cx);
                    });
                    let dialog = dialog.downgrade();
                    cx.spawn(async move |_, cx| {
                        cx.background_executor().timer(SAMPLE_DIALOG_WORK).await;
                        // The dialog may have closed meanwhile: nothing to update then.
                        let _ = dialog.update(cx, |dialog, cx| {
                            dialog.set_busy(false, cx);
                            dialog.set_error(
                                Some(
                                    "Couldn't delete the channel: the instance is offline.".into(),
                                ),
                                cx,
                            );
                        });
                    })
                    .detach();
                }
            },
        );
        self.dialog = Some((dialog, subscription));
        cx.notify();
    }

    fn open_settings_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_section = Some(0);
        window.focus(&self.settings_nav_focus[0], cx);
        cx.notify();
    }

    /// The sample sheet: three sections and a danger item that only closes it.
    fn settings_sheet(&self, window: &Window, cx: &Context<Self>) -> Option<SettingsSheet> {
        let section = self.settings_section?;
        let entity = cx.entity();
        let mut entries = vec![SettingsNavEntry::Heading("Rust Lyon".into())];
        for (index, (label, focus_handle)) in SAMPLE_SETTINGS_SECTIONS
            .iter()
            .zip(&self.settings_nav_focus)
            .enumerate()
        {
            if index == SAMPLE_SETTINGS_SECTIONS.len() - 1 {
                entries.push(SettingsNavEntry::Separator);
            }
            entries.push(SettingsNavEntry::Item(SettingsNavItem {
                label: (*label).into(),
                focus_handle: focus_handle.clone(),
                is_selected: index == section,
                is_danger: index == SAMPLE_SETTINGS_SECTIONS.len() - 1,
            }));
        }
        let close = {
            let entity = entity.clone();
            move |window: &mut Window, cx: &mut App| {
                entity.update(cx, |gallery, cx| {
                    gallery.settings_section = None;
                    window.focus(&gallery.root_focus, cx);
                    cx.notify();
                });
            }
        };
        let choose_close = close.clone();
        Some(
            SettingsSheet::new(
                "gallery-settings",
                SettingsSheetLabels {
                    label: "Server settings, Rust Lyon".into(),
                    title: "Server settings".into(),
                    back: "Back to server settings".into(),
                    close: "Close settings".into(),
                    escape_key: "Esc".into(),
                },
                self.settings_container_focus.clone(),
                self.settings_page_focus.clone(),
            )
            .wide(
                crate::theme::window_width_at_default_zoom(window)
                    >= crate::theme::tokens::BREAKPOINT_NAVIGATOR,
                true,
            )
            .entries(entries)
            .page(
                SAMPLE_SETTINGS_SECTIONS[section],
                div()
                    .child("A section's page: its fields, then the unsaved changes bar.")
                    .into_any_element(),
            )
            .on_choose(move |index, window, cx| {
                if index == SAMPLE_SETTINGS_SECTIONS.len() - 1 {
                    choose_close(window, cx);
                    return;
                }
                entity.update(cx, |gallery, cx| {
                    gallery.settings_section = Some(index);
                    cx.notify();
                });
            })
            .on_request_close(close.clone())
            .on_back(close),
        )
    }

    fn switch_theme(theme_index: usize, window: &mut Window, cx: &mut App) {
        let appearance = if theme_index == 0 {
            WindowAppearance::Dark
        } else {
            WindowAppearance::Light
        };
        apply_theme(ActiveTheme::for_appearance(appearance), cx);
        window.refresh();
    }
}

fn section(title: &'static str, cx: &App) -> gpui::Div {
    div().flex().flex_col().gap(length(SPACE_SM)).child(
        with_type(div(), TYPE_CATEGORY)
            .text_color(color(theme(cx).color_text_muted))
            .child(title),
    )
}

fn column() -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(length(SPACE_LG))
}

fn row() -> gpui::Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(length(SPACE_MD))
}

fn person(initial: &'static str, tint_number: u32) -> AvatarPerson {
    AvatarPerson {
        initial: initial.into(),
        tint_number,
        picture: None,
    }
}

impl Render for ComponentGallery {
    #[allow(clippy::too_many_lines)] // One page listing every primitive, as the eye scans it.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let entity = cx.entity();
        let is_dark = *tokens == crate::theme::tokens::DARK_THEME;
        let sheet = color(tokens.color_surface_chat);

        let buttons = section("Buttons", cx)
            .child(
                row().children(
                    [
                        ButtonVariant::Primary,
                        ButtonVariant::Secondary,
                        ButtonVariant::Danger,
                        ButtonVariant::Ghost,
                    ]
                    .into_iter()
                    .enumerate()
                    .flat_map(|(index, variant)| {
                        [
                            Button::new(("button", index), format!("{variant:?}"))
                                .variant(variant)
                                .into_any_element(),
                            Button::new(
                                ("button-disabled", index),
                                format!("{variant:?} disabled"),
                            )
                            .variant(variant)
                            .disabled(true)
                            .into_any_element(),
                        ]
                    }),
                ),
            )
            .child(
                row()
                    .child(Button::new("button-small", "Small").size(ButtonSize::Small))
                    .child(Button::new("button-default", "Default"))
                    .child(
                        Button::new("button-large", "Large")
                            .variant(ButtonVariant::Primary)
                            .size(ButtonSize::Large),
                    ),
            )
            .child(
                row()
                    .child(IconButton::new(
                        "icon-settings",
                        IconName::GearSix,
                        "User settings",
                    ))
                    .child(
                        IconButton::new("icon-delete", IconName::Trash, "Delete")
                            .tone(IconButtonTone::Danger),
                    )
                    .child(
                        IconButton::new("icon-mute", IconName::MicrophoneSlash, "Mute")
                            .toggled(true),
                    )
                    .child(
                        IconButton::new("icon-disabled", IconName::PushPin, "Pin").disabled(true),
                    )
                    .child(Tooltip::specimen_view("A tooltip, as it floats".into(), cx)),
            );

        let switches = section("Switch, slider, choice", cx)
            .child(
                row()
                    .child(
                        Switch::new("switch", self.switch_on, "Notifications").on_change({
                            let entity = entity.clone();
                            move |is_on, _, cx| {
                                entity.update(cx, |gallery, cx| {
                                    gallery.switch_on = is_on;
                                    cx.notify();
                                });
                            }
                        }),
                    )
                    .child(Switch::new("switch-off", false, "Off"))
                    .child(Switch::new("switch-locked-on", true, "Locked on").locked(true))
                    .child(Switch::new("switch-locked-off", false, "Locked off").locked(true)),
            )
            .child(
                div().w(length(SPACE_XL * 10.0)).child(
                    RangeField::new(
                        "range",
                        "Output volume",
                        self.range_value,
                        RangeBounds {
                            minimum: 0.0,
                            maximum: 200.0,
                            step: 1.0,
                        },
                        |value| SharedString::from(format!("{value:.0} %")),
                    )
                    .on_change({
                        let entity = entity.clone();
                        move |value, _, cx| {
                            entity.update(cx, |gallery, cx| {
                                gallery.range_value = value;
                                cx.notify();
                            });
                        }
                    }),
                ),
            )
            .child(
                div().w(length(SPACE_XL * 12.0)).child(
                    ChoiceGroup::new(
                        "choice",
                        "Channel type",
                        vec![
                            ChoiceOption::new("Text")
                                .icon(IconName::Hash)
                                .description("Messages, pictures, GIFs, opinions and puns"),
                            ChoiceOption::new("Voice")
                                .icon(IconName::SpeakerHigh)
                                .description("Talk, share a screen, hang out"),
                            ChoiceOption::new("Announcements"),
                        ],
                        self.choice_index,
                    )
                    .on_select({
                        let entity = entity.clone();
                        move |index, _, cx| {
                            entity.update(cx, |gallery, cx| {
                                gallery.choice_index = index;
                                cx.notify();
                            });
                        }
                    }),
                ),
            );

        let identity = section("Status, avatars, badges, roles", cx)
            .child(
                row().children(
                    [
                        COMPONENT_STATUS_SIZE_SMALL,
                        COMPONENT_STATUS_SIZE_MENU,
                        COMPONENT_STATUS_SIZE_LARGE,
                    ]
                    .into_iter()
                    .flat_map(|size| {
                        ShownStatus::ALL
                            .into_iter()
                            .map(move |status| StatusIndicator::new(status, size))
                    }),
                ),
            )
            .child(
                row()
                    .child(Avatar::new(
                        person("A", 1),
                        COMPONENT_MESSAGE_REPLY_AVATAR_SIZE,
                    ))
                    .child(
                        Avatar::new(person("B", 2), COMPONENT_MEMBER_ROW_AVATAR_SIZE)
                            .presence(ShownStatus::Online, sheet),
                    )
                    .child(
                        Avatar::new(person("C", 3), COMPONENT_MESSAGE_AVATAR_SIZE)
                            .presence(ShownStatus::Idle, sheet),
                    )
                    .child(
                        Avatar::new(person("D", 4), COMPONENT_MESSAGE_AVATAR_SIZE)
                            .presence(ShownStatus::DoNotDisturb, sheet),
                    )
                    .child(
                        Avatar::new(person("E", 5), COMPONENT_MEMBER_ROW_AVATAR_SIZE)
                            .presence(ShownStatus::Offline, sheet)
                            .dimmed(true),
                    )
                    .child(Avatar::new(
                        person("F", 6),
                        COMPONENT_PROFILE_POPOVER_AVATAR_SIZE,
                    ))
                    .child(GroupAvatar::new(
                        vec![person("G", 7), person("H", 8)],
                        COMPONENT_MESSAGE_AVATAR_SIZE,
                        sheet,
                    ))
                    .child(GroupAvatar::new(
                        vec![person("I", 1)],
                        COMPONENT_MESSAGE_AVATAR_SIZE,
                        sheet,
                    ))
                    .child(GroupAvatar::new(
                        Vec::new(),
                        COMPONENT_MESSAGE_AVATAR_SIZE,
                        sheet,
                    )),
            )
            .child(
                row()
                    .child(MentionBadge::new(3))
                    .child(MentionBadge::new(42))
                    .child(MentionBadge::new(1000))
                    .child(MentionBadge::new(7).ring(sheet))
                    .child(RoleChips::new(
                        "Roles",
                        vec![
                            RoleChip::Owner {
                                label: "Owner".into(),
                            },
                            RoleChip::Role {
                                name: "Moderators".into(),
                                colour: 0x003B_A55C,
                            },
                            RoleChip::Role {
                                name: "Artists".into(),
                                colour: 0x00E9_1E63,
                            },
                            RoleChip::Role {
                                name: "Members".into(),
                                colour: 0,
                            },
                        ],
                    )),
            );

        let fields = section("Text fields", cx).child(
            div()
                .w(length(SPACE_XL * 12.0))
                .flex()
                .flex_col()
                .gap(length(SPACE_MD))
                .child(
                    TextField::new("field-plain", "Search", &self.plain_field)
                        .leading_icon(IconName::MagnifyingGlass),
                )
                .child(
                    TextField::new("field-helper", "Server name", &self.helper_field)
                        .helper("Shown to everyone you invite."),
                )
                .child(
                    TextField::new("field-error", "Invite code", &self.error_field)
                        .monospace()
                        .error("Only letters and digits."),
                )
                .child(
                    TextField::new("field-password", "Password", &self.password_field).password(
                        PasswordToggle {
                            label: "Show password".into(),
                        },
                    ),
                ),
        );

        let navigation = section("Tabs, menus, dialogs", cx)
            .child(
                Tabs::new(
                    "tabs",
                    "Friends",
                    vec![
                        "Online".into(),
                        "All".into(),
                        "Pending".into(),
                        "Blocked".into(),
                    ],
                    self.tab_index,
                )
                .on_select({
                    let entity = entity.clone();
                    move |index, _, cx| {
                        entity.update(cx, |gallery, cx| {
                            gallery.tab_index = index;
                            cx.notify();
                        });
                    }
                }),
            )
            .child(
                Tabs::new(
                    "theme-tabs",
                    "Theme",
                    vec!["Dark".into(), "Light".into()],
                    usize::from(!is_dark),
                )
                .on_select(Self::switch_theme),
            )
            .child(
                row()
                    .child(
                        Button::new("open-menu", "Open a menu")
                            .track_focus(&self.menu_button_focus)
                            .on_click({
                                let entity = entity.clone();
                                move |window, cx| {
                                    let anchor = MenuAnchor {
                                        top: 0.0,
                                        bottom: 0.0,
                                        left: 0.0,
                                        right: 0.0,
                                    };
                                    let origin = window.mouse_position();
                                    let origin = MenuOrigin::Anchor {
                                        bounds: MenuAnchor {
                                            top: f32::from(origin.y),
                                            bottom: f32::from(origin.y),
                                            left: f32::from(origin.x),
                                            ..anchor
                                        },
                                        alignment: MenuAlignment::Start,
                                    };
                                    entity.update(cx, |gallery, cx| {
                                        gallery.open_menu(origin, window, cx);
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new("open-dialog", "Open a dialog")
                            .variant(ButtonVariant::Danger)
                            .on_click({
                                let entity = entity.clone();
                                move |window, cx| {
                                    entity
                                        .update(cx, |gallery, cx| gallery.open_dialog(window, cx));
                                }
                            }),
                    )
                    .child(
                        Button::new("open-settings-sheet", "Open a settings sheet").on_click({
                            let entity = entity.clone();
                            move |window, cx| {
                                entity.update(cx, |gallery, cx| {
                                    gallery.open_settings_sheet(window, cx);
                                });
                            }
                        }),
                    )
                    .child(
                        div()
                            .id("right-click-area")
                            .p(length(SPACE_MD))
                            .rounded(length(SPACE_SM))
                            .border_1()
                            .border_color(color(tokens.color_border_strong))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(|gallery, event: &MouseDownEvent, window, cx| {
                                    let origin = MenuOrigin::Point {
                                        x: f32::from(event.position.x),
                                        y: f32::from(event.position.y),
                                    };
                                    gallery.open_menu(origin, window, cx);
                                }),
                            )
                            .child("Right-click here"),
                    ),
            );

        let feedback = section("Progress, loading, unsaved changes", cx)
            .child(div().w(length(SPACE_XL * 10.0)).child(ProgressMeter::new(
                "progress",
                "Checklist",
                3,
                5,
                "3/5",
                "3 of 5 done",
            )))
            .child(
                row()
                    .items_start()
                    .child(
                        div()
                            .w(length(SPACE_XL * 10.0))
                            .child(SettingsListStatus::new(
                                "list-loading",
                                SettingsListStatusKind::Loading {
                                    label: "Loading the bans".into(),
                                },
                            )),
                    )
                    .child(SettingsListStatus::new(
                        "list-failed",
                        SettingsListStatusKind::Failed {
                            failure_text: "Couldn't load the bans.".into(),
                            retry_label: "Try again".into(),
                            on_retry: std::sync::Arc::new(|_, _| {}),
                        },
                    )),
            )
            .child(
                div().w_full().child(
                    UnsavedChangesBar::new(
                        "unsaved",
                        UnsavedChangesLabels {
                            careful: "Careful, you have unsaved changes!".into(),
                            save_or_reset: "Save or reset your changes first.".into(),
                            reset: "Reset".into(),
                            save_changes: "Save changes".into(),
                            saving: "Saving…".into(),
                        },
                        std::sync::Arc::new(|_, _| {}),
                        std::sync::Arc::new({
                            let entity = entity.clone();
                            move |_, cx| {
                                entity.update(cx, |gallery, cx| {
                                    gallery.unsaved_attention += 1;
                                    cx.notify();
                                });
                            }
                        }),
                    )
                    .attention(self.unsaved_attention),
                ),
            );

        let on_message_action: SampleActionHandler = Rc::new({
            let entity = entity.clone();
            move |action, _, cx| {
                entity.update(cx, |gallery, cx| {
                    gallery.messages.apply(&action);
                    cx.notify();
                });
            }
        });
        // The chat sheet at a laptop's message column width, as rows sit in the list.
        let messages = section("Messages", cx).child(
            div()
                .w_full()
                .max_w(length(MESSAGE_SAMPLE_COLUMN_WIDTH))
                .py(length(SPACE_SM))
                .rounded(length(SPACE_SM))
                .border_1()
                .border_color(color(tokens.color_border_divider))
                .bg(color(tokens.color_surface_chat))
                .flex()
                .flex_col()
                .children(self.messages.rows(&on_message_action, cx)),
        );

        let settings_sheet = self.settings_sheet(window, cx);
        let overlays: Vec<AnyElement> = self
            .menu
            .iter()
            .map(|(menu, _)| menu.clone().into_any_element())
            .chain(
                self.dialog
                    .iter()
                    .map(|(dialog, _)| dialog.clone().into_any_element()),
            )
            .collect();

        let gallery = with_tab_navigation(
            div()
                .id("component-gallery")
                .size_full()
                .overflow_y_scroll()
                .bg(color(tokens.color_surface_chat))
                .font_family(SANS_FONT_FAMILY)
                .text_color(color(tokens.color_text_normal))
                .p(length(SPACE_LG))
                .pt(length(TITLE_BAR_BAND_HEIGHT))
                .flex()
                .flex_col()
                .gap(length(SPACE_LG))
                .child(
                    with_type(div(), TYPE_HEADING_LG)
                        .text_color(color(tokens.color_text_heading))
                        .child("Mellow component gallery"),
                )
                // Three columns, so one screenshot shows every primitive.
                .child(
                    div()
                        .flex()
                        .gap(length(SPACE_XL))
                        .child(column().child(buttons).child(switches))
                        .child(column().child(identity).child(fields))
                        .child(column().child(navigation).child(feedback)),
                )
                .child(messages)
                // Type ":" or "@" and two letters for suggestions; files dropped on it fail to
                // upload, as the gallery has no instance.
                .child(section("Composer", cx).child(self.composer.clone()))
                .children(overlays),
            &self.root_focus,
        );
        div()
            .relative()
            .size_full()
            .child(gallery)
            .children(settings_sheet)
    }
}

/// Opens the gallery in place of the app when the variable is set.
#[must_use]
pub fn gallery_requested() -> Option<Option<WindowAppearance>> {
    std::env::var(COMPONENT_GALLERY_VARIABLE)
        .ok()
        .filter(|value| !value.is_empty() && value != "0")
        .map(|value| forced_appearance(&value))
}

/// Almost the whole screen, so one capture shows every primitive.
#[must_use]
pub fn gallery_window_options(cx: &App) -> gpui::WindowOptions {
    let mut options = main_window_options(cx);
    if let Some(screen) = cx.primary_display().map(|display| display.bounds().size) {
        let size = gpui::size(
            screen.width * GALLERY_SHARE_OF_SCREEN,
            screen.height * GALLERY_SHARE_OF_SCREEN,
        );
        options.window_bounds = Some(gpui::WindowBounds::Windowed(gpui::Bounds::centered(
            None, size, cx,
        )));
    }
    options
}

const GALLERY_SHARE_OF_SCREEN: f32 = 0.95;
/// The sample settings sheet's sections; the last is its danger item.
const SAMPLE_SETTINGS_SECTIONS: [&str; 4] = ["Overview", "Roles", "Emoji", "Delete server"];
/// The message column of a 1280 px window with the navigator and the side sheet docked.
const MESSAGE_SAMPLE_COLUMN_WIDTH: f32 = 720.0;

/// Builds the gallery view, following the OS theme or forcing one. A forced theme is for
/// screenshots: motion is reduced too, because macOS gives a window opened behind others no
/// frames, which would freeze every transition half-way in the capture.
pub fn open_gallery(
    forced: Option<WindowAppearance>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ComponentGallery> {
    match forced {
        Some(appearance) => {
            apply_theme(ActiveTheme::for_appearance(appearance), cx);
            cx.set_reduce_motion(true);
        }
        None => crate::theme::follow_window_appearance(window, cx),
    }
    let gallery = cx.new(|cx| ComponentGallery::new(window, cx));
    // The root holds focus first, so the first Tab reaches the gallery's controls.
    let root_focus = gallery.read(cx).root_focus.clone();
    window.focus(&root_focus, cx);
    gallery
}

debug_without_callbacks!(ComponentGallery);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_variable_can_force_a_theme() {
        assert_eq!(forced_appearance("dark"), Some(WindowAppearance::Dark));
        assert_eq!(forced_appearance("light"), Some(WindowAppearance::Light));
        assert_eq!(forced_appearance("1"), None);
    }
}
