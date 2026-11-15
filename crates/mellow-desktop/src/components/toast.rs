//! The toast (DESIGN.md §6.36): a card in the window's bottom trailing corner that stays until it
//! is acted on or closed. Its mark, a title, a sentence, an optional progress bar, its buttons
//! and a close button; it rises in once when it appears and never takes focus by itself.
//!
//! What it says is read out by the live regions its host keeps from the start (a region created
//! with its text is not announced), so the card itself is a labelled group.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, ElementId, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, RenderOnce, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _,
    relative,
};

use super::{
    FocusNextControl, FocusPreviousControl, IconButton, child_id, easing, icon, shadow, with_type,
};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_TOAST_BORDER_WIDTH, COMPONENT_TOAST_ENTER_DURATION, COMPONENT_TOAST_ENTER_OFFSET,
    COMPONENT_TOAST_ERROR_ICON_SIZE, COMPONENT_TOAST_GAP, COMPONENT_TOAST_LEADING_GAP,
    COMPONENT_TOAST_PADDING, COMPONENT_TOAST_PROGRESS_HEIGHT, COMPONENT_TOAST_RADIUS,
    COMPONENT_TOAST_WIDTH, DARK_THEME, MOTION_EASING_ENTER, SPACE_2XS, SPACE_XS, TYPE_BODY,
    TYPE_BODY_SM, TYPE_HEADING_MD,
};
use crate::theme::{animation_duration, color, length, theme};

/// How long one sweep of an indeterminate bar takes. Not a motion token: it is a working
/// indicator, not a transition, and stops with the work (or stands still under reduced motion).
const INDETERMINATE_SWEEP: Duration = Duration::from_millis(1400);
/// The share of the track an indeterminate bar's segment covers.
const INDETERMINATE_SEGMENT_SHARE: f32 = 0.32;

/// Whether the toast reports something that went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastTone {
    Neutral,
    /// The title and its icon in `text.danger`, with `warning-circle`.
    Danger,
}

/// A bar under the sentence: a share of the work done, or work under way of unknown length.
#[derive(Debug, Clone, PartialEq)]
pub struct ToastProgress {
    /// 0 to 1; `None` sweeps a segment along the track.
    pub share: Option<f32>,
    /// Shown at the bar's end ("42 %").
    pub figure: Option<SharedString>,
    /// What the bar measures, for screen readers ("Downloading Mellow 0.62.0…").
    pub label: SharedString,
}

/// The toast's close button: what it says, and what it does.
type DismissHandler = Box<dyn Fn(&mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct Toast {
    id: ElementId,
    tone: ToastTone,
    mark: Option<AnyElement>,
    title: SharedString,
    sentence: Option<SharedString>,
    progress: Option<ToastProgress>,
    actions: Vec<AnyElement>,
    dismiss: Option<(SharedString, DismissHandler)>,
    focus_handle: Option<FocusHandle>,
    /// Changes when the toast should rise in again (a newer version): part of the animation id.
    entrance: SharedString,
}

impl Toast {
    #[must_use]
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            tone: ToastTone::Neutral,
            mark: None,
            title: title.into(),
            sentence: None,
            progress: None,
            actions: Vec::new(),
            dismiss: None,
            focus_handle: None,
            entrance: SharedString::default(),
        }
    }

    #[must_use]
    pub fn tone(mut self, tone: ToastTone) -> Self {
        self.tone = tone;
        self
    }

    /// Drawn on the leading side (the app's still Melli).
    #[must_use]
    pub fn mark(mut self, mark: impl IntoElement) -> Self {
        self.mark = Some(mark.into_any_element());
        self
    }

    #[must_use]
    pub fn sentence(mut self, sentence: impl Into<SharedString>) -> Self {
        self.sentence = Some(sentence.into());
        self
    }

    #[must_use]
    pub fn progress(mut self, progress: ToastProgress) -> Self {
        self.progress = Some(progress);
        self
    }

    /// A button in the row under the text, in reading order.
    #[must_use]
    pub fn action(mut self, button: impl IntoElement) -> Self {
        self.actions.push(button.into_any_element());
        self
    }

    /// The close button, labelled `label`; `Escape` inside the toast closes it too.
    #[must_use]
    pub fn on_dismiss(
        mut self,
        label: impl Into<SharedString>,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.dismiss = Some((label.into(), Box::new(handler)));
        self
    }

    /// The host's handle, so it can move focus back into the toast when the focused button goes.
    #[must_use]
    pub fn track_focus(mut self, focus_handle: &FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle.clone());
        self
    }

    /// The toast rises in again whenever `entrance` changes.
    #[must_use]
    pub fn entrance(mut self, entrance: impl Into<SharedString>) -> Self {
        self.entrance = entrance.into();
        self
    }
}

debug_without_callbacks!(Toast, id, title);

impl RenderOnce for Toast {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let is_dark = *tokens == DARK_THEME;
        let progress_id = child_id(&self.id, "progress");
        let enter_id = ElementId::NamedChild(
            std::sync::Arc::new(child_id(&self.id, "enter")),
            self.entrance.clone(),
        );
        let text = text_block(self.tone, self.title.clone(), self.sentence, cx);
        let progress = self
            .progress
            .map(|progress| progress_bar(&progress_id, progress, cx));
        let actions = (!self.actions.is_empty()).then(|| {
            div()
                .flex()
                .flex_wrap()
                .gap(length(SPACE_XS))
                .children(self.actions)
        });

        let body = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(length(COMPONENT_TOAST_GAP))
            .child(text)
            .children(progress)
            .children(actions);

        let has_close_button = self.dismiss.is_some();
        let (close_button, escape_handler) = match self.dismiss {
            Some((label, handler)) => {
                let handler = std::rc::Rc::new(handler);
                let on_escape = std::rc::Rc::clone(&handler);
                (
                    Some(
                        IconButton::new(child_id(&self.id, "close"), IconName::X, label)
                            .on_click(move |window, cx| handler(window, cx)),
                    ),
                    Some(on_escape),
                )
            }
            None => (None, None),
        };

        let enter_duration = animation_duration(COMPONENT_TOAST_ENTER_DURATION, cx);
        div()
            .id(self.id)
            .role(Role::Group)
            .aria_label(self.title)
            .when_some(self.focus_handle, |card, focus_handle| {
                card.track_focus(&focus_handle)
            })
            .when_some(escape_handler, |card, on_escape| {
                card.on_key_down(move |event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        on_escape(window, cx);
                    }
                })
            })
            // Tab moves on from the toast's controls: it lies outside the screen, whose own
            // handler would otherwise never see the keystroke.
            .on_action(|_: &FocusNextControl, window, cx| window.focus_next(cx))
            .on_action(|_: &FocusPreviousControl, window, cx| window.focus_prev(cx))
            // Clicks on the card stay on it: the screen under it must not take them.
            .occlude()
            .relative()
            .w(length(COMPONENT_TOAST_WIDTH))
            .max_w_full()
            .flex()
            .items_start()
            .gap(length(COMPONENT_TOAST_LEADING_GAP))
            .p(length(COMPONENT_TOAST_PADDING))
            .when(has_close_button, |card| {
                // The close button sits in the corner; the text keeps clear of it.
                card.pr(length(COMPONENT_TOAST_PADDING / 2.0))
            })
            .rounded(length(COMPONENT_TOAST_RADIUS))
            .bg(color(tokens.component_toast_background))
            .shadow(shadow(tokens.component_toast_shadow))
            .when(is_dark, |card| {
                card.border(length(COMPONENT_TOAST_BORDER_WIDTH))
                    .border_color(color(tokens.component_toast_border))
            })
            .children(self.mark)
            .child(body)
            .children(close_button)
            .with_animation(
                enter_id,
                Animation::new(enter_duration)
                    .with_easing(|progress| easing(MOTION_EASING_ENTER).sample(progress)),
                |card, progress| {
                    card.opacity(progress.clamp(0.0, 1.0))
                        .top(length(COMPONENT_TOAST_ENTER_OFFSET * (1.0 - progress)))
                },
            )
    }
}

/// The title, after `warning-circle` when something went wrong, and the sentence under it.
fn text_block(
    tone: ToastTone,
    title: SharedString,
    sentence: Option<SharedString>,
    cx: &App,
) -> gpui::Div {
    let tokens = theme(cx);
    let title_colour = match tone {
        ToastTone::Neutral => color(tokens.component_toast_title_text),
        ToastTone::Danger => color(tokens.component_toast_error_text),
    };
    let heading = div()
        .flex()
        .items_start()
        .gap(length(SPACE_XS))
        .when(tone == ToastTone::Danger, |heading| {
            heading.child(icon(
                IconName::WarningCircle,
                COMPONENT_TOAST_ERROR_ICON_SIZE,
                title_colour,
            ))
        })
        .child(
            with_type(div(), TYPE_HEADING_MD)
                .flex_1()
                .min_w_0()
                .text_color(title_colour)
                .child(title),
        );
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(length(SPACE_2XS))
        .child(heading)
        .when_some(sentence, |text, sentence| {
            text.child(
                with_type(div(), TYPE_BODY)
                    .text_color(color(tokens.component_toast_body_text))
                    .child(sentence),
            )
        })
}

/// The track and its fill, with the figure at its end; a progress indicator for screen readers.
fn progress_bar(id: &ElementId, progress: ToastProgress, cx: &App) -> impl IntoElement {
    let tokens = theme(cx);
    let fill = color(tokens.component_toast_progress_fill);
    let track = div()
        .id(id.clone())
        .role(Role::ProgressIndicator)
        .aria_label(progress.label)
        .when_some(progress.figure.clone(), |track, figure| {
            track.aria_value(figure)
        })
        .when_some(progress.share, |track, share| {
            track
                .aria_numeric_value(f64::from(share) * 100.0)
                .aria_min_numeric_value(0.0)
                .aria_max_numeric_value(100.0)
        })
        .relative()
        .flex_1()
        .min_w_0()
        .h(length(COMPONENT_TOAST_PROGRESS_HEIGHT))
        .rounded_full()
        .overflow_hidden()
        .bg(color(tokens.component_toast_progress_track));
    let track = match progress.share {
        Some(share) => track
            .child(
                div()
                    .h_full()
                    .w(relative(share.clamp(0.0, 1.0)))
                    .rounded_full()
                    .bg(fill),
            )
            .into_any_element(),
        // Under reduced motion the segment stands still at the start of the track.
        None if cx.reduce_motion() => track
            .child(
                div()
                    .h_full()
                    .w(relative(INDETERMINATE_SEGMENT_SHARE))
                    .rounded_full()
                    .bg(fill),
            )
            .into_any_element(),
        None => track
            .child(
                div()
                    .absolute()
                    .top_0()
                    .h_full()
                    .w(relative(INDETERMINATE_SEGMENT_SHARE))
                    .rounded_full()
                    .bg(fill)
                    .with_animation(
                        child_id(id, "sweep"),
                        Animation::new(INDETERMINATE_SWEEP).repeat(),
                        |segment, progress| {
                            // From just outside the leading edge to just outside the trailing.
                            let start = progress * (1.0 + INDETERMINATE_SEGMENT_SHARE)
                                - INDETERMINATE_SEGMENT_SHARE;
                            segment.left(relative(start))
                        },
                    ),
            )
            .into_any_element(),
    };
    div()
        .flex()
        .items_center()
        .gap(length(SPACE_XS))
        .child(track)
        .when_some(progress.figure, |row, figure| {
            row.child(
                with_type(div(), TYPE_BODY_SM)
                    .flex_none()
                    .text_color(color(tokens.component_toast_body_text))
                    .child(figure),
            )
        })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{
        Context, FocusHandle, InputEvent as _, KeyUpEvent, Keystroke, Render, TestAppContext,
        VisualTestContext, WindowAppearance,
    };

    use super::*;
    use crate::components::{Button, bind_tab_navigation, with_tab_navigation};
    use crate::theme::ActiveTheme;

    struct ToastHarness {
        root: FocusHandle,
        toast: FocusHandle,
        action: FocusHandle,
        dismissals: Rc<Cell<u32>>,
    }

    impl Render for ToastHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let dismissals = Rc::clone(&self.dismissals);
            with_tab_navigation(div().size_full(), &self.root).child(
                Toast::new("toast", "Mellow 0.62.0 is available")
                    .sentence("Mellow restarts once the update is installed.")
                    .action(Button::new("update", "Update and restart").track_focus(&self.action))
                    .on_dismiss("Hide the update notice", move |_, _| {
                        dismissals.set(dismissals.get() + 1);
                    })
                    .track_focus(&self.toast),
            )
        }
    }

    fn press(cx: &mut VisualTestContext, key: &'static str) {
        cx.simulate_keystrokes(key);
        let keystroke = Keystroke::parse(key).expect("a valid keystroke in a test");
        cx.update(|window, cx| {
            window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn tab_reaches_the_action_then_the_close_button_and_escape_closes(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
            bind_tab_navigation(cx);
        });
        let dismissals = Rc::new(Cell::new(0));
        let counted = Rc::clone(&dismissals);
        let (harness, cx) = cx.add_window_view(move |_, cx| ToastHarness {
            root: cx.focus_handle(),
            toast: cx.focus_handle(),
            action: cx.focus_handle(),
            dismissals: counted,
        });
        let (root, toast, action) = harness.read_with(cx, |harness, _| {
            (
                harness.root.clone(),
                harness.toast.clone(),
                harness.action.clone(),
            )
        });
        cx.update(|window, cx| {
            window.activate_window();
            window.focus(&root, cx);
        });
        cx.run_until_parked();
        assert!(
            !cx.update(|window, cx| toast.contains_focused(window, cx)),
            "the toast never takes focus by itself"
        );
        press(cx, "tab");
        assert!(cx.update(|window, _| action.is_focused(window)));
        press(cx, "tab");
        assert!(
            cx.update(|window, cx| !action.is_focused(window) && toast.contains_focused(window, cx)),
            "the close button comes after the action"
        );
        press(cx, "escape");
        assert_eq!(dismissals.get(), 1);
    }
}
