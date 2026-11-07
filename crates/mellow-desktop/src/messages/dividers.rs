//! The dividers between rows (`MessageList.svelte`, DESIGN.md §5.4): a day's date between two
//! rules, and the apricot "New" line above the first message sent since the reader's last visit.
//! Where new messages start a day, one line carries both, as on Discord.

use gpui::{
    App, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};

use crate::components::with_type;
use crate::i18n;
use crate::theme::tokens::{
    COMPONENT_MESSAGE_GROUP_GAP, COMPONENT_UNREAD_DIVIDER_LINE_WIDTH,
    COMPONENT_UNREAD_DIVIDER_PILL_HEIGHT, COMPONENT_UNREAD_DIVIDER_PILL_RADIUS, SPACE_MD, SPACE_XS,
    TYPE_BADGE, TYPE_MESSAGE_TIMESTAMP, ThemeTokens,
};
use crate::theme::{color, length, theme};

fn rule(colour: Hsla) -> gpui::Div {
    div()
        .flex_1()
        .h(length(COMPONENT_UNREAD_DIVIDER_LINE_WIDTH))
        .bg(colour)
}

fn new_pill(tokens: &ThemeTokens, label: &'static str) -> gpui::Div {
    with_type(div(), TYPE_BADGE)
        .flex_none()
        .flex()
        .items_center()
        .h(length(COMPONENT_UNREAD_DIVIDER_PILL_HEIGHT))
        .px(length(SPACE_XS))
        .rounded(length(COMPONENT_UNREAD_DIVIDER_PILL_RADIUS))
        .bg(color(tokens.component_unread_divider_pill_background))
        .text_color(color(tokens.component_unread_divider_pill_text))
        .child(label)
}

/// A day's date between two rules; apricot with the "New" pill where new messages start that day.
#[derive(Debug, IntoElement)]
pub struct DayDivider {
    /// "November 5, 2026".
    day: SharedString,
    /// The unread line too: the time the first new message was sent ("14:32").
    new_since: Option<SharedString>,
}

impl DayDivider {
    #[must_use]
    pub fn new(day: impl Into<SharedString>) -> Self {
        Self {
            day: day.into(),
            new_since: None,
        }
    }

    #[must_use]
    pub fn new_since(mut self, time_of_day: Option<SharedString>) -> Self {
        self.new_since = time_of_day;
        self
    }
}

impl RenderOnce for DayDivider {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let strings = i18n::strings(cx).messages();
        let line = color(if self.new_since.is_some() {
            tokens.component_unread_divider_line
        } else {
            tokens.color_border_divider
        });
        let label: SharedString = match &self.new_since {
            Some(time) => strings.day_and_new_since(&self.day, time).into(),
            None => self.day.clone(),
        };
        div()
            .id(SharedString::from(format!("day-divider-{}", self.day)))
            .role(Role::Splitter)
            .aria_label(label)
            .mx(length(SPACE_MD))
            .mt(length(COMPONENT_MESSAGE_GROUP_GAP))
            .flex()
            .items_center()
            .gap(length(SPACE_XS))
            .child(rule(line))
            .child(
                with_type(div(), TYPE_MESSAGE_TIMESTAMP)
                    .flex_none()
                    .text_color(color(tokens.color_text_muted))
                    .child(self.day),
            )
            .child(rule(line))
            .when(self.new_since.is_some(), |divider| {
                divider.child(new_pill(tokens, strings.new_pill()))
            })
    }
}

/// The apricot line with its "New" pill: new messages are "now" (DESIGN.md §2.3).
#[derive(Debug, IntoElement)]
pub struct UnreadDivider {
    /// The time the first new message was sent ("14:32").
    since: SharedString,
}

impl UnreadDivider {
    #[must_use]
    pub fn new(time_of_day: impl Into<SharedString>) -> Self {
        Self {
            since: time_of_day.into(),
        }
    }
}

impl RenderOnce for UnreadDivider {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let strings = i18n::strings(cx).messages();
        div()
            .id("unread-divider")
            .role(Role::Splitter)
            .aria_label(strings.new_since(&self.since))
            .mx(length(SPACE_MD))
            .mt(length(COMPONENT_MESSAGE_GROUP_GAP))
            .flex()
            .items_center()
            .child(rule(color(tokens.component_unread_divider_line)))
            .child(new_pill(tokens, strings.new_pill()))
    }
}
