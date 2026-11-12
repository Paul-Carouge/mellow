//! `ModalDialog.svelte` (DESIGN.md §6.35): title, optional sentence, the dialog's own controls, an
//! inline error, and a footer with Cancel and one action, over a scrim that keeps the window
//! behind inert. Tab and Shift+Tab stay inside; Esc, Cancel and the scrim close it except while
//! busy; focus goes back where it came from.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyView, App, Context, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, anchored, deferred, div,
    point, prelude::FluentBuilder as _, px,
};

use super::{
    Button, ButtonVariant, FocusNextControl, FocusPreviousControl, LiveRegion as _, easing,
    focus_next_inside, focus_previous_inside, icon, shadow, with_type,
};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CONTEXT_MENU_BORDER_WIDTH, COMPONENT_DIALOG_BUTTON_GAP,
    COMPONENT_DIALOG_ENTER_DURATION, COMPONENT_DIALOG_ERROR_GAP, COMPONENT_DIALOG_ERROR_ICON_SIZE,
    COMPONENT_DIALOG_MARGIN, COMPONENT_DIALOG_PADDING, COMPONENT_DIALOG_RADIUS,
    COMPONENT_DIALOG_SECTION_GAP, COMPONENT_DIALOG_SUBTITLE_GAP, COMPONENT_DIALOG_WIDTH,
    COMPONENT_DIALOG_WIDTH_WIDE, DARK_THEME, MOTION_DURATION_BASE, MOTION_EASING_SETTLE,
    MOTION_EASING_STANDARD, TYPE_BODY, TYPE_HEADING_LG,
};
use crate::theme::{color, length, theme, window_width_at_default_zoom};

/// What the dialog asks its host to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalDialogEvent {
    /// Closed by Esc, Cancel or the scrim; focus is already back on the opener.
    Closed,
    /// The action button: the host runs it, marking the dialog busy meanwhile.
    Submitted,
}

/// A dialog's words and options; the host owns the controls in `body`.
#[allow(clippy::struct_excessive_bools)] // Independent options of one dialog, as the web's props.
pub struct ModalDialog {
    title: SharedString,
    subtitle: Option<SharedString>,
    body: Option<AnyView>,
    is_wide: bool,
    cancel_label: SharedString,
    /// `None` leaves Cancel alone in the footer.
    submit_label: Option<SharedString>,
    busy_submit_label: Option<SharedString>,
    is_submit_danger: bool,
    is_submit_enabled: bool,
    is_busy: bool,
    error_message: Option<SharedString>,
    container_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    opener: Option<FocusHandle>,
}

impl EventEmitter<ModalDialogEvent> for ModalDialog {}

impl Focusable for ModalDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.container_focus.clone()
    }
}

impl ModalDialog {
    /// Opens over the window; the control that had focus gets it back when the dialog closes.
    /// Focus always starts on Cancel for now (DESIGN.md §8 wants that for destructive dialogs and
    /// Cancel-only ones); starting on a body control lands with the first dialog that needs it.
    pub fn new(
        title: impl Into<SharedString>,
        cancel_label: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let opener = window.focused(cx);
        let cancel_focus = cx.focus_handle().tab_stop(true);
        window.focus(&cancel_focus, cx);
        Self {
            title: title.into(),
            subtitle: None,
            body: None,
            is_wide: false,
            cancel_label: cancel_label.into(),
            submit_label: None,
            busy_submit_label: None,
            is_submit_danger: false,
            is_submit_enabled: true,
            is_busy: false,
            error_message: None,
            container_focus: cx.focus_handle(),
            cancel_focus,
            submit_focus: cx.focus_handle().tab_stop(true),
            opener,
        }
    }

    #[must_use]
    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    #[must_use]
    pub fn body(mut self, body: AnyView) -> Self {
        self.body = Some(body);
        self
    }

    /// Two columns side by side, at the wide width.
    #[must_use]
    pub fn wide(mut self) -> Self {
        self.is_wide = true;
        self
    }

    /// `busy_label` replaces it while the action runs ("Renaming…").
    #[must_use]
    pub fn submit(
        mut self,
        label: impl Into<SharedString>,
        busy_label: impl Into<SharedString>,
        is_danger: bool,
    ) -> Self {
        self.submit_label = Some(label.into());
        self.busy_submit_label = Some(busy_label.into());
        self.is_submit_danger = is_danger;
        self
    }

    pub fn set_busy(&mut self, is_busy: bool, cx: &mut Context<Self>) {
        self.is_busy = is_busy;
        cx.notify();
    }

    /// Renames the action as the dialog's controls change ("Message", "Create group"); `None`
    /// leaves Cancel alone.
    pub fn set_submit_labels(
        &mut self,
        labels: Option<(SharedString, SharedString)>,
        cx: &mut Context<Self>,
    ) {
        let (label, busy_label) = labels.unzip();
        self.submit_label = label;
        self.busy_submit_label = busy_label;
        cx.notify();
    }

    pub fn set_submit_enabled(&mut self, is_enabled: bool, cx: &mut Context<Self>) {
        self.is_submit_enabled = is_enabled;
        cx.notify();
    }

    /// What failed, shown above the footer and announced at once.
    pub fn set_error(&mut self, error_message: Option<SharedString>, cx: &mut Context<Self>) {
        self.error_message = error_message;
        cx.notify();
    }

    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.is_busy
    }

    #[must_use]
    pub fn is_submit_enabled(&self) -> bool {
        self.is_submit_enabled
    }

    /// Closes unless busy, giving focus back to the opener first.
    pub fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy {
            return;
        }
        if let Some(opener) = self.opener.as_ref() {
            window.focus(opener, cx);
        }
        cx.emit(ModalDialogEvent::Closed);
    }

    /// The action, as its button runs it: Enter in a field of the dialog's form calls this.
    pub fn submit_now(&mut self, cx: &mut Context<Self>) {
        if !self.is_busy && self.is_submit_enabled && self.submit_label.is_some() {
            cx.emit(ModalDialogEvent::Submitted);
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified() {
            cx.stop_propagation();
            self.request_close(window, cx);
        }
    }
}

impl Render for ModalDialog {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let viewport = window.viewport_size();
        let width_token = if self.is_wide {
            COMPONENT_DIALOG_WIDTH_WIDE
        } else {
            COMPONENT_DIALOG_WIDTH
        };
        let width =
            width_token.min(window_width_at_default_zoom(window) - 2.0 * COMPONENT_DIALOG_MARGIN);
        let is_busy = self.is_busy;
        let submit_label = if is_busy {
            self.busy_submit_label.clone().or(self.submit_label.clone())
        } else {
            self.submit_label.clone()
        };
        let has_submit = submit_label.is_some();
        let entity = cx.entity();
        let cancel_entity = entity.clone();
        let submit_entity = entity.clone();

        let footer = div()
            .debug_selector(|| "dialog-footer".to_owned())
            .flex_none()
            .flex()
            .flex_wrap()
            .justify_end()
            .gap(length(COMPONENT_DIALOG_BUTTON_GAP))
            .px(length(COMPONENT_DIALOG_PADDING))
            .pb(length(COMPONENT_DIALOG_PADDING))
            .bg(color(tokens.component_dialog_footer_background))
            .child(
                // Beside an action, Cancel stays quiet; alone, it is the dialog's button.
                Button::new("dialog-cancel", self.cancel_label.clone())
                    .variant(if has_submit {
                        ButtonVariant::Ghost
                    } else {
                        ButtonVariant::Secondary
                    })
                    .disabled(is_busy)
                    .track_focus(&self.cancel_focus)
                    .on_click(move |window, cx| {
                        cancel_entity.update(cx, |dialog, cx| dialog.request_close(window, cx));
                    }),
            )
            .when_some(submit_label, |footer, label| {
                footer.child(
                    Button::new("dialog-submit", label)
                        .variant(if self.is_submit_danger {
                            ButtonVariant::Danger
                        } else {
                            ButtonVariant::Primary
                        })
                        .disabled(is_busy || !self.is_submit_enabled)
                        .track_focus(&self.submit_focus)
                        .on_click(move |_, cx| {
                            submit_entity.update(cx, ModalDialog::submit_now);
                        }),
                )
            });

        // The body scrolls and the footer stays, as `overflow-y-auto` does in ModalDialog.svelte.
        let content = div()
            .id("dialog-content")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_DIALOG_SECTION_GAP))
            .p(length(COMPONENT_DIALOG_PADDING))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_DIALOG_SUBTITLE_GAP))
                    .child(
                        with_type(div(), TYPE_HEADING_LG)
                            .text_color(color(tokens.component_dialog_title_text))
                            .child(self.title.clone()),
                    )
                    .when_some(self.subtitle.clone(), |heading, subtitle| {
                        heading.child(
                            with_type(div(), TYPE_BODY)
                                .text_color(color(tokens.component_dialog_subtitle_text))
                                .child(subtitle),
                        )
                    }),
            )
            .when_some(self.body.clone(), gpui::ParentElement::child)
            .when_some(self.error_message.clone(), |content, error| {
                content.child(
                    with_type(div(), TYPE_BODY)
                        .id("dialog-error")
                        .live_assertive(error.clone())
                        .flex()
                        .gap(length(COMPONENT_DIALOG_ERROR_GAP))
                        .text_color(color(tokens.component_dialog_error_text))
                        .child(icon(
                            IconName::WarningCircle,
                            COMPONENT_DIALOG_ERROR_ICON_SIZE,
                            color(tokens.component_dialog_error_text),
                        ))
                        .child(error),
                )
            });

        // Dark floating layers get a 1 px divider outline (`dark:[outline…]` on the web).
        let is_dark = *tokens == DARK_THEME;
        let card = div()
            .id("modal-dialog")
            .role(Role::Dialog)
            .aria_label(self.title.clone())
            .when_some(self.subtitle.clone(), |card, subtitle| {
                card.aria_description(subtitle)
            })
            .track_focus(&self.container_focus)
            .tab_group()
            .key_context("ModalDialog")
            .on_key_down(cx.listener(Self::on_key_down))
            .on_action(cx.listener(|dialog, _: &FocusNextControl, window, cx| {
                focus_next_inside(&dialog.container_focus, window, cx);
            }))
            .on_action(cx.listener(|dialog, _: &FocusPreviousControl, window, cx| {
                focus_previous_inside(&dialog.container_focus, window, cx);
            }))
            // Clicks on the card stay on the card; only the scrim closes.
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .flex()
            .flex_col()
            .w(length(width))
            .max_h(length(
                f32::from(viewport.height) - 2.0 * COMPONENT_DIALOG_MARGIN,
            ))
            .overflow_hidden()
            .rounded(length(COMPONENT_DIALOG_RADIUS))
            .bg(color(tokens.component_dialog_background))
            .text_color(color(tokens.component_dialog_body_text))
            .shadow(shadow(tokens.elevation_high))
            .when(is_dark, |card| {
                card.border(length(COMPONENT_CONTEXT_MENU_BORDER_WIDTH))
                    .border_color(color(tokens.color_border_divider))
            })
            .child(content)
            .child(footer)
            // GPUI cannot scale a box yet, so the card settles in by fading only (documented gap).
            .with_animation(
                "modal-dialog-enter",
                Animation::new(Duration::from_millis(COMPONENT_DIALOG_ENTER_DURATION.0))
                    .with_easing(|progress| easing(MOTION_EASING_SETTLE).sample(progress)),
                |card, progress| card.opacity(progress.min(1.0)),
            );

        let scrim = div()
            .id("modal-dialog-scrim")
            .occlude()
            .w(px(f32::from(viewport.width)))
            .h(px(f32::from(viewport.height)))
            .flex()
            .items_center()
            .justify_center()
            .bg(color(tokens.component_dialog_scrim))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|dialog, _, window, cx| dialog.request_close(window, cx)),
            )
            .child(card)
            .with_animation(
                "modal-dialog-scrim",
                Animation::new(Duration::from_millis(MOTION_DURATION_BASE.0))
                    .with_easing(|progress| easing(MOTION_EASING_STANDARD).sample(progress)),
                gpui::Styled::opacity,
            );

        deferred(anchored().position(point(px(0.0), px(0.0))).child(scrim)).with_priority(2)
    }
}

debug_without_callbacks!(ModalDialog, title);
