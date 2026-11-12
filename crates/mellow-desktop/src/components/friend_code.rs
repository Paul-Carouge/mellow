//! `FriendCode.svelte`: the user's own friend code (ADR-0048) with a copy button, shown in My
//! account and under "Add friend". The button says "Copied" for the dialog's copied-label duration
//! and the copy is announced, wherever the row is shown.

use gpui::{
    App, ClipboardItem, Context, ElementId, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Task, Window, div,
};

use super::{Button, with_type};
use crate::i18n::strings;
use crate::theme::tokens::{
    COMPONENT_DIALOG_COPIED_LABEL_DURATION, COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP,
    COMPONENT_SETTINGS_ACCOUNT_VALUE_GAP, COMPONENT_SETTINGS_SECTION_LABEL_GAP, TYPE_BODY_SM,
    TYPE_CATEGORY, TYPE_CODE,
};
use crate::theme::{color, delay, length, theme};

/// What the row remembers between frames: whether the code was just copied, and the announcement.
#[derive(Debug, Default)]
pub struct FriendCodeCopyState {
    is_copied: bool,
    announcement: SharedString,
    copied_label_timer: Option<Task<()>>,
}

impl FriendCodeCopyState {
    fn copy(&mut self, code: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(code.to_owned()));
        self.is_copied = true;
        self.announcement = strings(cx).friend_code().copied().into();
        let timer = cx
            .background_executor()
            .timer(delay(COMPONENT_DIALOG_COPIED_LABEL_DURATION));
        // Replacing the timer cancels the previous one.
        self.copied_label_timer = Some(cx.spawn(async move |state, cx| {
            timer.await;
            let _ = state.update(cx, |state, cx| {
                state.is_copied = false;
                cx.notify();
            });
        }));
        cx.notify();
    }
}

/// The friend code row; its parent keeps a `FriendCodeCopyState` for as long as it is shown.
#[derive(IntoElement)]
pub struct FriendCode {
    id: ElementId,
    code: SharedString,
    state: Entity<FriendCodeCopyState>,
}

impl FriendCode {
    pub fn new(
        id: impl Into<ElementId>,
        code: impl Into<SharedString>,
        state: Entity<FriendCodeCopyState>,
    ) -> Self {
        Self {
            id: id.into(),
            code: code.into(),
            state,
        }
    }
}

debug_without_callbacks!(FriendCode, id, code);

impl RenderOnce for FriendCode {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let words = strings(cx).friend_code();
        let copy_state = self.state.read(cx);
        let button_label = if copy_state.is_copied {
            words.copied_short()
        } else {
            words.copy()
        };
        let announcement = copy_state.announcement.clone();
        let code = self.code.clone();
        let state = self.state;
        div()
            .id(self.id)
            .flex()
            .flex_col()
            .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(length(COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP))
                    .child(
                        // A term and its value, so screen readers read the code with its label.
                        div()
                            .id("friend-code-value")
                            .aria_label(SharedString::from(format!(
                                "{}, {}",
                                words.label(),
                                self.code
                            )))
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(length(COMPONENT_SETTINGS_ACCOUNT_VALUE_GAP))
                            .child(
                                with_type(div(), TYPE_CATEGORY)
                                    .text_color(color(tokens.component_settings_section_label_text))
                                    .child(words.label()),
                            )
                            .child(
                                with_type(div(), TYPE_CODE)
                                    .text_color(color(tokens.component_settings_account_value_text))
                                    .child(self.code),
                            ),
                    )
                    .child(
                        Button::new("friend-code-copy", button_label).on_click(move |_, cx| {
                            state.update(cx, |state, cx| state.copy(&code, cx));
                        }),
                    ),
            )
            .child(
                with_type(div(), TYPE_BODY_SM)
                    .text_color(color(tokens.color_text_secondary))
                    .child(words.hint()),
            )
            .child(
                div()
                    .id("friend-code-announcement")
                    .role(Role::Status)
                    .aria_label(announcement)
                    .h_0()
                    .overflow_hidden(),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext as _, TestAppContext};

    use super::*;

    #[gpui::test]
    fn a_copy_is_announced_and_the_copied_label_goes_back_after_its_duration(
        cx: &mut TestAppContext,
    ) {
        let state = cx.new(|_| FriendCodeCopyState::default());
        state.update(cx, |state, cx| state.copy("ABCD-EFGH", cx));
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("ABCD-EFGH".to_owned())
        );
        cx.read(|cx| {
            let state = state.read(cx);
            assert!(state.is_copied);
            assert_eq!(state.announcement, strings(cx).friend_code().copied());
        });
        cx.executor()
            .advance_clock(delay(COMPONENT_DIALOG_COPIED_LABEL_DURATION));
        cx.run_until_parked();
        cx.read(|cx| assert!(!state.read(cx).is_copied));
    }
}
