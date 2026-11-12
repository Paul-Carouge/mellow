//! The user card across the navigator's foot (`UserCard.svelte`, ADR-0022 §4): as tall as the
//! composer's base row and as far from the bottom of its card, so both line up across the window
//! (`size.dock.*`). The avatar and name open the status menu (DESIGN.md §5.3), which also sets
//! the custom status and copies the user ID; the gear opens the user settings, which hold
//! "Sign out" and the notification choices, as on the web.

use gpui::{
    App, AppContext as _, ClipboardItem, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, Role, StatefulInteractiveElement as _, Styled as _, Window, canvas, div,
    prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_protocol::{ChosenStatus, PublicUser};

use super::custom_status_dialog::CustomStatusForm;
use super::{DuskShell, profile_picture};
use crate::components::{
    Avatar, AvatarPerson, ContextMenu, IconButton, LiveRegion as _, MenuAlignment, MenuAnchor,
    MenuEntry, MenuItem, MenuOrigin, ShownStatus, focus_ring, icon, is_focus_visible,
    remembered_hover, with_type,
};
use crate::i18n::{Strings, strings};
use crate::session::Session;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_DIALOG_COPIED_LABEL_DURATION, COMPONENT_STATUS_SIZE_MENU,
    COMPONENT_USER_PANEL_AVATAR_SIZE, COMPONENT_USER_PANEL_BUTTON_ICON_SIZE,
    COMPONENT_USER_PANEL_BUTTON_SIZE, COMPONENT_USER_PANEL_RADIUS, RADIUS_SM, SIZE_DOCK_HEIGHT,
    SIZE_DOCK_MARGIN, SPACE_2XS, SPACE_XS, TYPE_BODY_SM, TYPE_USER_PANEL_NAME,
};
use crate::theme::{color, delay, length, theme};

const CHOSEN_STATUSES: [ChosenStatus; 4] = [
    ChosenStatus::Online,
    ChosenStatus::Idle,
    ChosenStatus::DoNotDisturb,
    ChosenStatus::Invisible,
];

pub(super) fn shown_status(status: ChosenStatus) -> ShownStatus {
    match status {
        ChosenStatus::Online => ShownStatus::Online,
        ChosenStatus::Idle => ShownStatus::Idle,
        ChosenStatus::DoNotDisturb => ShownStatus::DoNotDisturb,
        ChosenStatus::Invisible => ShownStatus::Invisible,
    }
}

fn status_name(strings: Strings, status: ChosenStatus) -> &'static str {
    let presence = strings.presence();
    match status {
        ChosenStatus::Online => presence.online(),
        ChosenStatus::Idle => presence.idle(),
        ChosenStatus::DoNotDisturb => presence.do_not_disturb(),
        ChosenStatus::Invisible => presence.invisible(),
    }
}

impl DuskShell {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    pub(super) fn render_user_card(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tokens = theme(cx);
        let strings = strings(cx);
        let session = self.session.read(cx);
        let status_error = session.status_error().map(str::to_owned);
        let user = session.state().current_user().cloned();
        let bounds = self.user_card_bounds.clone();
        let card = div()
            .debug_selector(|| "user-card".to_owned())
            .relative()
            .flex_none()
            .h(length(SIZE_DOCK_HEIGHT))
            .mx(length(SIZE_DOCK_MARGIN))
            .mb(length(SIZE_DOCK_MARGIN))
            .px(length(SPACE_2XS))
            .flex()
            .items_center()
            .rounded(length(COMPONENT_USER_PANEL_RADIUS))
            .bg(color(tokens.component_user_panel_background))
            // Records where the card is, for the status menu to open across it.
            .child(
                canvas(
                    move |card_bounds, _, _| bounds.set(Some(card_bounds)),
                    |_, (), _, _| {},
                )
                .absolute()
                .size_full(),
            );
        let Some(user) = user else {
            return card;
        };
        let person = PublicUser {
            id: user.id,
            username: user.username.clone(),
            display_name: user.display_name.clone(),
            avatar: user.avatar,
            banner: None,
        };
        let name = display_name(&person).to_owned();
        let status = user.status;
        let custom_status = user.custom_status.clone();
        let is_menu_open = self.status_menu.is_some();
        let hover = remembered_hover(&"user-status-button".into(), window, cx);
        let is_active = is_menu_open || *hover.read(cx);
        // The status shape is cut out of whatever the button shows behind it.
        let surface = color(if is_active {
            tokens.component_user_panel_button_background_hover
        } else {
            tokens.component_user_panel_background
        });
        let picture = user
            .avatar
            .map(|image| profile_picture(image, COMPONENT_USER_PANEL_AVATAR_SIZE, window));
        let second_line = if self.is_user_id_copied {
            Some((strings.user_card().user_id_copied().to_owned(), true))
        } else {
            custom_status.clone().map(|text| (text, false))
        };
        let status_line_ink = color(if is_active {
            tokens.component_user_panel_status_text_hover
        } else {
            tokens.component_user_panel_status_text
        });
        let label = strings.user_card().status_button_label(
            &name,
            status_name(strings, status),
            custom_status.as_deref(),
        );
        let is_ring_shown = is_focus_visible(&self.status_button_focus, window);

        card.child(
            div()
                .id("user-status-button")
                .role(Role::Button)
                .aria_label(label)
                .aria_expanded(is_menu_open)
                .track_focus(&self.status_button_focus)
                .relative()
                .flex_1()
                .min_w_0()
                .self_stretch()
                .my(length(SPACE_2XS))
                .px(length(SPACE_2XS))
                .flex()
                .items_center()
                .gap(length(SPACE_XS))
                .rounded(length(RADIUS_SM))
                .bg(surface)
                .cursor_pointer()
                .on_hover(move |is_hovered, _, cx| {
                    hover.update(cx, |remembered, cx| {
                        *remembered = *is_hovered;
                        cx.notify();
                    });
                })
                .on_click(cx.listener(|shell, _, window, cx| shell.toggle_status_menu(window, cx)))
                .child(
                    Avatar::new(
                        AvatarPerson::from_user(&person, picture),
                        COMPONENT_USER_PANEL_AVATAR_SIZE,
                    )
                    .presence(shown_status(status), surface),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            with_type(div(), TYPE_USER_PANEL_NAME)
                                .truncate()
                                .text_color(color(tokens.component_user_panel_name_text))
                                .child(name),
                        )
                        .when_some(second_line, |lines, (text, is_copied)| {
                            lines.child(
                                with_type(div(), TYPE_BODY_SM)
                                    .flex()
                                    .items_center()
                                    .gap(length(SPACE_2XS))
                                    .min_w_0()
                                    .text_color(status_line_ink)
                                    .when(is_copied, |line| {
                                        line.child(icon(
                                            IconName::Check,
                                            COMPONENT_STATUS_SIZE_MENU,
                                            status_line_ink,
                                        ))
                                    })
                                    .child(div().min_w_0().truncate().child(text)),
                            )
                        })
                        .when_some(status_error, |lines, error| {
                            lines.child(
                                with_type(div(), TYPE_BODY_SM)
                                    .id("status-error")
                                    .live_assertive(error.clone())
                                    .truncate()
                                    .text_color(color(tokens.color_text_danger))
                                    .child(error),
                            )
                        }),
                )
                .when(is_ring_shown, |button| {
                    button.child(focus_ring(RADIUS_SM, cx))
                }),
        )
        // shortcut: the microphone and headset toggles come with voice (M3 step 7).
        .child(
            IconButton::new(
                "user-settings-button",
                IconName::GearSix,
                strings.user_card().user_settings(),
            )
            .sizes(
                COMPONENT_USER_PANEL_BUTTON_SIZE,
                COMPONENT_USER_PANEL_BUTTON_ICON_SIZE,
            )
            .key_shortcuts(if cfg!(target_os = "macos") {
                "Meta+,"
            } else {
                "Control+,"
            })
            .on_click({
                let shell = cx.entity().downgrade();
                move |window, cx| {
                    let _ = shell.update(cx, |shell, cx| shell.open_user_settings(window, cx));
                }
            }),
        )
    }

    /// The status menu across the card, above it: the card sits at the bottom of the window.
    fn toggle_status_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.status_menu.take().is_some() {
            cx.notify();
            return;
        }
        let Some(card) = self.user_card_bounds.get() else {
            return;
        };
        let strings = strings(cx);
        let Some(user) = self.session.read(cx).state().current_user().cloned() else {
            return;
        };
        let session = self.session.clone();
        let mut entries = status_choice_entries(&session, strings, user.status);
        let shell = cx.entity().downgrade();
        let user_id = user.id;
        entries.push(MenuEntry::Separator);
        let dialog_shell = shell.clone();
        entries.push(MenuEntry::Item(MenuItem::new(
            if user.custom_status.is_some() {
                strings.user_card().edit_custom_status()
            } else {
                strings.user_card().set_custom_status()
            },
            IconName::Smiley,
            move |window, cx| {
                let _ = dialog_shell.update(cx, |shell, cx| {
                    shell.open_custom_status_dialog(window, cx);
                });
            },
        )));
        entries.push(MenuEntry::Separator);
        entries.push(MenuEntry::Item(MenuItem::new(
            strings.user_card().copy_user_id(),
            IconName::Copy,
            move |_, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(user_id.to_string()));
                let _ = shell.update(cx, DuskShell::show_user_id_copied);
            },
        )));
        let origin = MenuOrigin::Anchor {
            bounds: MenuAnchor {
                top: f32::from(card.top()),
                bottom: f32::from(card.bottom()),
                left: f32::from(card.left()),
                right: f32::from(card.right()),
            },
            alignment: MenuAlignment::Stretch,
        };
        let opener = Some(self.status_button_focus.clone());
        let label = strings.user_card().status_menu();
        let menu = cx.new(|cx| ContextMenu::open(label, entries, origin, opener, window, cx));
        let subscription = cx.subscribe(&menu, |shell, _, _: &gpui::DismissEvent, cx| {
            shell.status_menu = None;
            cx.notify();
        });
        self.status_menu = Some((menu, subscription));
        cx.notify();
    }

    /// The custom status dialog; focus goes back to the status button when it closes.
    pub(super) fn open_custom_status_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.status_button_focus, cx);
        self.custom_status_dialog = Some(CustomStatusForm::open(
            self.session.clone(),
            window,
            cx,
            |shell: &mut Self, cx| {
                shell.custom_status_dialog = None;
                cx.notify();
            },
        ));
        cx.notify();
    }

    /// "User ID copied" on the card's second line for a moment, where focus lands.
    fn show_user_id_copied(&mut self, cx: &mut Context<Self>) {
        let timer = cx
            .background_executor()
            .timer(delay(COMPONENT_DIALOG_COPIED_LABEL_DURATION));
        self.is_user_id_copied = true;
        // Replacing the timer cancels the previous one.
        self.copied_label_timer = Some(cx.spawn(async move |shell, cx| {
            timer.await;
            let _ = shell.update(cx, |shell, cx| {
                shell.is_user_id_copied = false;
                cx.notify();
            });
        }));
        cx.notify();
    }
}

/// One menu choice per status the user can pick, the current one checked.
fn status_choice_entries(
    session: &gpui::Entity<Session>,
    strings: Strings,
    current_status: ChosenStatus,
) -> Vec<MenuEntry> {
    CHOSEN_STATUSES
        .into_iter()
        .map(|status| {
            let session = session.clone();
            let mut item = MenuItem::choice(
                status_name(strings, status),
                shown_status(status),
                status == current_status,
                move |_, cx: &mut App| {
                    let failure = strings.user_card().status_failed();
                    session.update(cx, |session, cx| {
                        if session.state().current_user().map(|user| user.status) != Some(status) {
                            session.choose_status(status, failure, cx);
                        }
                    });
                },
            );
            let hint = match status {
                ChosenStatus::DoNotDisturb => Some(strings.user_card().do_not_disturb_hint()),
                ChosenStatus::Invisible => Some(strings.user_card().invisible_hint()),
                ChosenStatus::Online | ChosenStatus::Idle => None,
            };
            if let Some(hint) = hint {
                item = item.description(hint);
            }
            MenuEntry::Item(item)
        })
        .collect()
}
