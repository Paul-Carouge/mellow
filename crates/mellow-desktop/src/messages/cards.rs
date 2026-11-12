//! The cards a message may carry instead of text (DESIGN.md §5.4): `PollCard.svelte` (ADR-0043,
//! docs/design/web-polls.md), `ChecklistCard.svelte` and `ProgressBarCard.svelte` (ADR-0047,
//! docs/design/web-checklists-and-progress.md). The three share the poll's well, rows, marks and
//! level colours, so they read as one family. They draw what they are given and say what the
//! reader did through callbacks; the conversation view owns the requests and the state that
//! changes a card's height (results shown, a checklist being edited).

use std::sync::Arc;

use gpui::{
    AnyElement, AnyView, App, ElementId, FocusHandle, FontWeight, Hsla, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement as _, RenderOnce, Role, SharedString,
    StatefulInteractiveElement, Styled as _, Toggled, Window, div, prelude::FluentBuilder as _,
    relative,
};
use gpui_base::motion::{Transition, transition};
use mellow_client_core::shared_logic::poll_view::{
    PollTimeLeft, PollTimeUnit, poll_end_unix_milliseconds, poll_results, poll_time_left,
};
use mellow_client_core::state::{checklist_progress, is_poll_ended, progress_percent};
use mellow_protocol::{Checklist, Poll, ProgressBar};

use super::time::{full_timestamp_label, local_date_time};
use crate::components::{
    ActivationHandler, ProgressMeter, Tooltip, child_id, colour_transition, easing, focus_ring,
    icon, inset_focus_ring, is_focus_visible, polite_live_region, remembered_focus_handle,
    remembered_hover, with_type,
};
use crate::i18n::catalogue::MinutesOrHoursOrDays;
use crate::i18n::{Strings, strings};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_POLL_ANSWER_BORDER_WIDTH, COMPONENT_POLL_ANSWER_GAP, COMPONENT_POLL_ANSWER_INNER_GAP,
    COMPONENT_POLL_ANSWER_MIN_HEIGHT, COMPONENT_POLL_ANSWER_PADDING_BLOCK,
    COMPONENT_POLL_ANSWER_PADDING_INLINE, COMPONENT_POLL_ANSWER_RADIUS,
    COMPONENT_POLL_BORDER_WIDTH, COMPONENT_POLL_COUNT_MIN_WIDTH, COMPONENT_POLL_FOOTER_GAP,
    COMPONENT_POLL_GAP, COMPONENT_POLL_HEADER_GAP, COMPONENT_POLL_LEADER_ICON_SIZE,
    COMPONENT_POLL_MARK_BORDER_WIDTH, COMPONENT_POLL_MARK_GLYPH_SIZE,
    COMPONENT_POLL_MARK_RADIUS_CHECKBOX, COMPONENT_POLL_MARK_SIZE, COMPONENT_POLL_MAX_WIDTH,
    COMPONENT_POLL_PADDING, COMPONENT_POLL_RADIUS, MOTION_DURATION_SLOW, MOTION_EASING_STANDARD,
    RADIUS_XS, SIZE_HIT_DESKTOP_MIN, SIZE_HIT_WEB_MIN, SPACE_2XS, SPACE_3XS, TYPE_BODY,
    TYPE_BODY_SM, TYPE_HEADING_MD, TYPE_MESSAGE,
};
use crate::theme::{animation_duration, color, length, theme};

/// Runs with an answer's or item's id.
pub type CardItemHandler = Arc<dyn Fn(u32, &mut Window, &mut App) + 'static>;

/// The well every card sits in, 432 px at most.
fn card_frame(id: ElementId, label: SharedString, gap: f32, cx: &App) -> gpui::Stateful<gpui::Div> {
    let tokens = theme(cx);
    div()
        .id(id)
        .role(Role::Group)
        .aria_label(label)
        .mt(length(SPACE_3XS))
        .flex()
        .flex_col()
        .gap(length(gap))
        .w_full()
        .max_w(length(COMPONENT_POLL_MAX_WIDTH))
        .p(length(COMPONENT_POLL_PADDING))
        .rounded(length(COMPONENT_POLL_RADIUS))
        .border(length(COMPONENT_POLL_BORDER_WIDTH))
        .border_color(color(tokens.component_poll_border))
        .bg(color(tokens.component_poll_background))
}

/// The card's heading: an optional glyph and the title, at heading size.
fn card_heading(glyph: Option<IconName>, title: SharedString, cx: &App) -> impl IntoElement {
    let tokens = theme(cx);
    with_type(div(), TYPE_HEADING_MD)
        .flex()
        .items_center()
        .gap(length(SPACE_2XS))
        .text_color(color(tokens.component_poll_question_text))
        .when_some(glyph, |heading, glyph| {
            heading.child(icon(
                glyph,
                COMPONENT_POLL_MARK_SIZE,
                color(tokens.component_poll_hint_text),
            ))
        })
        .child(div().min_w_0().child(title))
}

/// A word that acts, in the link colour ("Show results", "Edit list").
fn link_button(
    id: ElementId,
    glyph: Option<IconName>,
    label: SharedString,
    on_click: ActivationHandler,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let focus_handle = remembered_focus_handle(&id, window, cx);
    let hover = remembered_hover(&id, window, cx);
    let link = color(tokens.color_text_link);
    with_type(div(), TYPE_BODY_SM)
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .track_focus(&focus_handle)
        .relative()
        .flex()
        .items_center()
        .gap(length(SPACE_3XS))
        .min_h(length(SIZE_HIT_DESKTOP_MIN))
        .rounded(length(RADIUS_XS))
        .cursor_pointer()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(link)
        .when(*hover.read(cx), gpui::Styled::underline)
        .on_hover(move |is_hovered, _, cx| {
            hover.update(cx, |remembered, cx| {
                *remembered = *is_hovered;
                cx.notify();
            });
        })
        .on_click(move |_, window, cx| on_click(window, cx))
        .when_some(glyph, |button, glyph| {
            button.child(icon(glyph, COMPONENT_POLL_MARK_GLYPH_SIZE, link))
        })
        .child(label)
        .when(is_focus_visible(&focus_handle, window), |button| {
            button.child(focus_ring(RADIUS_XS, cx))
        })
        .into_any_element()
}

/// A radio dot or a checkbox square, filled with its glyph when chosen.
fn mark(is_checkbox: bool, is_checked: bool, cx: &App) -> impl IntoElement {
    let tokens = theme(cx);
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(length(COMPONENT_POLL_MARK_SIZE))
        .map(|mark| {
            if is_checkbox {
                mark.rounded(length(COMPONENT_POLL_MARK_RADIUS_CHECKBOX))
            } else {
                mark.rounded_full()
            }
        })
        .map(|mark| {
            if !is_checked {
                return mark
                    .border(length(COMPONENT_POLL_MARK_BORDER_WIDTH))
                    .border_color(color(tokens.component_poll_mark_border));
            }
            let glyph = color(tokens.component_poll_mark_glyph);
            mark.bg(color(tokens.component_poll_mark_checked))
                .map(|mark| {
                    if is_checkbox {
                        mark.child(icon(IconName::Check, COMPONENT_POLL_MARK_GLYPH_SIZE, glyph))
                    } else {
                        mark.child(
                            div()
                                .size(length(COMPONENT_POLL_MARK_GLYPH_SIZE / 2.0))
                                .rounded_full()
                                .bg(glyph),
                        )
                    }
                })
        })
}

/// Marks a control inert for screen readers (the web's `aria-disabled`).
fn inert_for_screen_readers<E: StatefulInteractiveElement>(element: E, is_inert: bool) -> E {
    if is_inert {
        element.a11y_synthetic_children(|subtree| subtree.parent_node().set_disabled())
    } else {
        element
    }
}

/// "Ends in 3 hours", "Ended": the time left as the footer says it.
#[must_use]
pub fn poll_time_left_label(time_left: PollTimeLeft, catalogue: Strings) -> String {
    let polls = catalogue.polls();
    match time_left {
        PollTimeLeft::Ended => polls.ended().to_owned(),
        PollTimeLeft::LessThanAMinute => polls.ends_in_less_than_a_minute().to_owned(),
        PollTimeLeft::Left { unit, amount } => {
            let unit = match unit {
                PollTimeUnit::Minutes => MinutesOrHoursOrDays::Minutes,
                PollTimeUnit::Hours => MinutesOrHoursOrDays::Hours,
                PollTimeUnit::Days => MinutesOrHoursOrDays::Days,
            };
            polls.ends_in(unit, i64::try_from(amount).unwrap_or(i64::MAX))
        }
    }
}

// ---- Poll ----------------------------------------------------------------------------------

/// A poll in a message: the question, one row per answer and a footer. Results stay hidden until
/// the reader votes, asks to see them, or the poll ends; then each row fills from its leading side.
#[derive(IntoElement)]
pub struct PollCard {
    id: ElementId,
    poll: Poll,
    now_unix_milliseconds: u64,
    /// Still in the outbox: shown as it will arrive, nothing to press yet.
    is_pending: bool,
    /// Whoever reads a server channel, or writes in a conversation (ADR-0043).
    can_vote: bool,
    /// "Show results" was pressed.
    are_results_revealed: bool,
    on_choose: Option<CardItemHandler>,
    on_remove_vote: Option<ActivationHandler>,
    on_toggle_results: Option<ActivationHandler>,
    on_view_voters: Option<CardItemHandler>,
}

impl PollCard {
    #[must_use]
    pub fn new(id: impl Into<ElementId>, poll: Poll, now_unix_milliseconds: u64) -> Self {
        Self {
            id: id.into(),
            poll,
            now_unix_milliseconds,
            is_pending: false,
            can_vote: false,
            are_results_revealed: false,
            on_choose: None,
            on_remove_vote: None,
            on_toggle_results: None,
            on_view_voters: None,
        }
    }

    #[must_use]
    pub fn pending(mut self, is_pending: bool) -> Self {
        self.is_pending = is_pending;
        self
    }

    #[must_use]
    pub fn can_vote(mut self, can_vote: bool) -> Self {
        self.can_vote = can_vote;
        self
    }

    #[must_use]
    pub fn results_revealed(mut self, are_revealed: bool) -> Self {
        self.are_results_revealed = are_revealed;
        self
    }

    #[must_use]
    pub fn on_choose(mut self, handler: impl Fn(u32, &mut Window, &mut App) + 'static) -> Self {
        self.on_choose = Some(Arc::new(handler));
        self
    }

    #[must_use]
    pub fn on_remove_vote(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_remove_vote = Some(Arc::new(handler));
        self
    }

    #[must_use]
    pub fn on_toggle_results(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_toggle_results = Some(Arc::new(handler));
        self
    }

    #[must_use]
    pub fn on_view_voters(
        mut self,
        handler: impl Fn(u32, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_view_voters = Some(Arc::new(handler));
        self
    }
}

impl RenderOnce for PollCard {
    #[allow(clippy::too_many_lines)] // One card, in the order it is drawn, as the web's markup.
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let polls = catalogue.polls();
        let poll = &self.poll;
        let id = self.id;
        let results = poll_results(poll);
        let is_ended = is_poll_ended(poll, self.now_unix_milliseconds);
        let can_vote_now = self.can_vote && !self.is_pending && !is_ended;
        let are_results_shown =
            !self.is_pending && (is_ended || results.has_reader_voted || self.are_results_revealed);
        let is_multiple = poll.allows_multiple_answers;
        // A radio group is one Tab stop: the checked answer, else the first.
        let tab_stop_answer_id = poll
            .answers
            .iter()
            .find(|answer| answer.me_voted)
            .or(poll.answers.first())
            .map(|answer| answer.id);
        let answer_handles: Vec<FocusHandle> = poll
            .answers
            .iter()
            .map(|answer| {
                let focus_id = ElementId::NamedChild(
                    Arc::new(id.clone()),
                    SharedString::from(format!("answer-focus-{}", answer.id)),
                );
                window
                    .use_keyed_state(focus_id, cx, |_, cx| cx.focus_handle())
                    .read(cx)
                    .clone()
                    .tab_stop(is_multiple || Some(answer.id) == tab_stop_answer_id)
            })
            .collect();
        let bar_motion = Transition::new(animation_duration(MOTION_DURATION_SLOW, cx))
            .easing(easing(MOTION_EASING_STANDARD));

        let rows: Vec<AnyElement> =
            results
                .answers
                .iter()
                .zip(&answer_handles)
                .map(|(result, focus_handle)| {
                    let answer = result.answer;
                    let answer_id = answer.id;
                    let row_id = ElementId::NamedChild(
                        Arc::new(id.clone()),
                        SharedString::from(format!("answer-{answer_id}")),
                    );
                    let is_winner = is_ended && result.is_leading;
                    let hover = remembered_hover(&row_id, window, cx);
                    let border = if answer.me_voted {
                        tokens.component_poll_answer_border_mine
                    } else if can_vote_now && *hover.read(cx) {
                        tokens.component_poll_answer_border_hover
                    } else {
                        tokens.component_poll_answer_border
                    };
                    let border =
                        colour_transition(child_id(&row_id, "border"), color(border), window, cx);
                    // Grows from nothing when results appear, then follows each vote.
                    #[allow(clippy::cast_precision_loss)] // A percentage, at most 100.
                    let share = transition(
                        child_id(&row_id, "bar"),
                        if are_results_shown {
                            result.percent as f32 / 100.0
                        } else {
                            0.0
                        },
                        bar_motion.clone(),
                        window,
                        cx,
                    );
                    let results_sentence = polls
                        .answer_results(i64::from(answer.vote_count), i64::from(result.percent));
                    let mut spoken_name = answer.text.clone();
                    if is_winner {
                        spoken_name = format!("{spoken_name}, {}", polls.winner());
                    }
                    let on_choose = self.on_choose.clone().filter(|_| can_vote_now);
                    let is_ring_shown = is_focus_visible(focus_handle, window);
                    let choice = div()
                        .id(child_id(&row_id, "choice"))
                        .role(if is_multiple {
                            Role::CheckBox
                        } else {
                            Role::RadioButton
                        })
                        .aria_label(spoken_name)
                        .aria_toggled(if answer.me_voted {
                            Toggled::True
                        } else {
                            Toggled::False
                        })
                        .when(are_results_shown, |choice| {
                            choice.aria_description(results_sentence.clone())
                        })
                        .track_focus(focus_handle)
                        .relative()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap(length(COMPONENT_POLL_ANSWER_INNER_GAP))
                        .px(length(COMPONENT_POLL_ANSWER_PADDING_INLINE))
                        .py(length(COMPONENT_POLL_ANSWER_PADDING_BLOCK))
                        .when(can_vote_now, gpui::Styled::cursor_pointer)
                        .on_click(move |_, window, cx| {
                            if let Some(on_choose) = &on_choose {
                                on_choose(answer_id, window, cx);
                            }
                        })
                        .child(mark(is_multiple, answer.me_voted, cx))
                        .child(
                            with_type(div(), TYPE_MESSAGE)
                                .min_w_0()
                                .text_color(color(tokens.component_poll_answer_text))
                                .when(is_winner, |text| text.font_weight(FontWeight::SEMIBOLD))
                                .child(answer.text.clone()),
                        )
                        .when(is_winner, |choice| {
                            choice.child(icon(
                                IconName::Crown,
                                COMPONENT_POLL_LEADER_ICON_SIZE,
                                color(tokens.component_poll_leader_icon),
                            ))
                        })
                        .when(is_ring_shown, |choice| {
                            choice.child(inset_focus_ring(COMPONENT_POLL_ANSWER_RADIUS, cx))
                        });
                    let choice = inert_for_screen_readers(choice, !can_vote_now);
                    let figures = |column: gpui::Div| {
                        column
                            .flex()
                            .flex_none()
                            .flex_col()
                            .items_end()
                            .justify_center()
                            .min_w(length(COMPONENT_POLL_COUNT_MIN_WIDTH))
                            .px(length(COMPONENT_POLL_ANSWER_PADDING_INLINE))
                            .child(
                                with_type(div(), TYPE_BODY_SM)
                                    .text_color(color(tokens.component_poll_count_text))
                                    .child(polls.votes(i64::from(answer.vote_count))),
                            )
                            .child(
                                with_type(div(), TYPE_BODY)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(color(tokens.component_poll_percent_text))
                                    .child(polls.percent(i64::from(result.percent))),
                            )
                    };
                    let results_column: Option<AnyElement> = are_results_shown.then(|| match self
                        .on_view_voters
                        .clone()
                        .filter(|_| answer.vote_count > 0)
                    {
                        Some(on_view_voters) => {
                            let voters_id = child_id(&row_id, "voters");
                            let voters_focus = remembered_focus_handle(&voters_id, window, cx);
                            let voters_hover = remembered_hover(&voters_id, window, cx);
                            let is_voters_ring_shown = is_focus_visible(&voters_focus, window);
                            figures(div())
                                .id(voters_id)
                                .role(Role::Button)
                                .aria_label(format!(
                                    "{results_sentence}{}",
                                    polls.see_voters(&answer.text)
                                ))
                                .track_focus(&voters_focus)
                                .relative()
                                .rounded(length(COMPONENT_POLL_ANSWER_RADIUS))
                                .cursor_pointer()
                                .when(*voters_hover.read(cx), gpui::Styled::underline)
                                .on_hover(move |is_hovered, _, cx| {
                                    voters_hover.update(cx, |remembered, cx| {
                                        *remembered = *is_hovered;
                                        cx.notify();
                                    });
                                })
                                .on_click(move |_, window, cx| {
                                    on_view_voters(answer_id, window, cx);
                                })
                                .when(is_voters_ring_shown, |column| {
                                    column.child(inset_focus_ring(COMPONENT_POLL_ANSWER_RADIUS, cx))
                                })
                                .into_any_element()
                        }
                        None => figures(div()).relative().into_any_element(),
                    });
                    let bar_fill: Hsla = color(if answer.me_voted {
                        tokens.component_poll_bar_mine
                    } else {
                        tokens.component_poll_bar
                    });
                    div()
                        .id(row_id)
                        .relative()
                        .flex()
                        .items_stretch()
                        .min_h(length(COMPONENT_POLL_ANSWER_MIN_HEIGHT))
                        .overflow_hidden()
                        .rounded(length(COMPONENT_POLL_ANSWER_RADIUS))
                        .border(length(COMPONENT_POLL_ANSWER_BORDER_WIDTH))
                        .border_color(border)
                        .bg(color(tokens.component_poll_answer_background))
                        .on_hover(move |is_hovered, _, cx| {
                            hover.update(cx, |remembered, cx| {
                                *remembered = *is_hovered;
                                cx.notify();
                            });
                        })
                        .when(share > 0.0, |row| {
                            row.child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .bottom_0()
                                    .left_0()
                                    .w(relative(share))
                                    .bg(bar_fill),
                            )
                        })
                        .child(choice)
                        .children(results_column)
                        .into_any_element()
                })
                .collect();

        // Arrows move between a single choice's answers without voting: a vote is a request, so
        // Space or Enter casts it.
        let arrow_handles = answer_handles;
        let answers = div()
            .id(child_id(&id, "answers"))
            .role(if is_multiple {
                Role::Group
            } else {
                Role::RadioGroup
            })
            .aria_label(poll.question.clone())
            .when(!is_ended, |group| {
                group.aria_description(if is_multiple {
                    polls.choose_several()
                } else {
                    polls.choose_one()
                })
            })
            .flex()
            .flex_col()
            .gap(length(COMPONENT_POLL_ANSWER_GAP))
            .when(!is_multiple, |group| {
                group.on_key_down(move |event: &KeyDownEvent, window, cx| {
                    if event.keystroke.modifiers.modified() {
                        return;
                    }
                    let step: isize = match event.keystroke.key.as_str() {
                        "down" | "right" => 1,
                        "up" | "left" => -1,
                        _ => return,
                    };
                    // The voters buttons sit in the group too: arrows there keep their meaning.
                    let Some(current) = arrow_handles
                        .iter()
                        .position(|handle| handle.is_focused(window))
                    else {
                        return;
                    };
                    cx.stop_propagation();
                    let count = arrow_handles.len().cast_signed();
                    let next = (current.cast_signed() + step)
                        .rem_euclid(count)
                        .cast_unsigned();
                    if let Some(handle) = arrow_handles.get(next) {
                        window.focus(handle, cx);
                    }
                })
            })
            .children(rows);

        let end = poll_end_unix_milliseconds(poll);
        let end_label = full_timestamp_label(local_date_time(end), catalogue);
        let end_tooltip: SharedString = if is_ended {
            polls.ended_at(&end_label)
        } else {
            polls.ends_at(&end_label)
        }
        .into();
        let footer_text = format!(
            "{} · {}",
            polls.votes(i64::from(results.total_votes)),
            poll_time_left_label(poll_time_left(poll, self.now_unix_milliseconds), catalogue)
        );
        let footer_action: Option<AnyElement> = if self.is_pending || is_ended {
            None
        } else if results.has_reader_voted {
            self.on_remove_vote
                .filter(|_| self.can_vote)
                .map(|on_remove| {
                    link_button(
                        child_id(&id, "remove-vote"),
                        None,
                        polls.remove_vote().into(),
                        on_remove,
                        window,
                        cx,
                    )
                })
        } else {
            self.on_toggle_results.map(|on_toggle| {
                let label = if self.are_results_revealed {
                    polls.hide_results()
                } else {
                    polls.show_results()
                };
                link_button(
                    child_id(&id, "results"),
                    None,
                    label.into(),
                    on_toggle,
                    window,
                    cx,
                )
            })
        };

        card_frame(
            id.clone(),
            poll.question.clone().into(),
            COMPONENT_POLL_GAP,
            cx,
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(length(COMPONENT_POLL_HEADER_GAP))
                .child(card_heading(None, poll.question.clone().into(), cx))
                .when(!is_ended, |header| {
                    header.child(
                        with_type(div(), TYPE_BODY_SM)
                            .text_color(color(tokens.component_poll_hint_text))
                            .child(if is_multiple {
                                polls.choose_several()
                            } else {
                                polls.choose_one()
                            }),
                    )
                }),
        )
        .child(answers)
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_x(length(COMPONENT_POLL_FOOTER_GAP))
                .child(
                    with_type(div(), TYPE_BODY_SM)
                        .id(child_id(&id, "time-left"))
                        .text_color(color(tokens.component_poll_footer_text))
                        .tooltip(move |_, cx| Tooltip::view(end_tooltip.clone(), cx))
                        .tooltip_show_delay(Tooltip::show_delay())
                        .child(footer_text),
                )
                .children(footer_action),
        )
    }
}

impl std::fmt::Debug for PollCard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PollCard")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

// ---- Checklist -----------------------------------------------------------------------------

/// A checklist in a message: the title and its "3/5" level, then one checkbox row per item with
/// who ticked it and when. Its author and moderators switch it to editing, where `editor` (the
/// conversation's field rows) replaces the rows.
#[derive(IntoElement)]
pub struct ChecklistCard {
    id: ElementId,
    checklist: Checklist,
    /// "Ticked by Ada, November 5, 2026 at 14:32", one per item, `None` while not ticked.
    ticked_by: Vec<Option<String>>,
    is_pending: bool,
    can_tick: bool,
    can_edit: bool,
    editor: Option<AnyView>,
    on_toggle: Option<CardItemHandler>,
    on_edit: Option<ActivationHandler>,
}

impl ChecklistCard {
    #[must_use]
    pub fn new(
        id: impl Into<ElementId>,
        checklist: Checklist,
        ticked_by: Vec<Option<String>>,
    ) -> Self {
        Self {
            id: id.into(),
            checklist,
            ticked_by,
            is_pending: false,
            can_tick: false,
            can_edit: false,
            editor: None,
            on_toggle: None,
            on_edit: None,
        }
    }

    #[must_use]
    pub fn pending(mut self, is_pending: bool) -> Self {
        self.is_pending = is_pending;
        self
    }

    #[must_use]
    pub fn can_tick(mut self, can_tick: bool) -> Self {
        self.can_tick = can_tick;
        self
    }

    #[must_use]
    pub fn can_edit(mut self, can_edit: bool) -> Self {
        self.can_edit = can_edit;
        self
    }

    /// Shown instead of the rows while the list is edited.
    #[must_use]
    pub fn editor(mut self, editor: Option<AnyView>) -> Self {
        self.editor = editor;
        self
    }

    #[must_use]
    pub fn on_toggle(mut self, handler: impl Fn(u32, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Arc::new(handler));
        self
    }

    /// "Edit list", or "Done" while editing.
    #[must_use]
    pub fn on_edit(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_edit = Some(Arc::new(handler));
        self
    }
}

impl RenderOnce for ChecklistCard {
    #[allow(clippy::too_many_lines)] // One card, in the order it is drawn, as the web's markup.
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.checklists();
        let id = self.id;
        let progress = checklist_progress(&self.checklist);
        let (done, total) = (
            u32::try_from(progress.done).unwrap_or(u32::MAX),
            u32::try_from(progress.total).unwrap_or(u32::MAX),
        );
        let can_tick_now = self.can_tick && !self.is_pending;
        let title: SharedString = self.checklist.title.clone().into();
        let is_editing = self.editor.is_some();

        let rows: Vec<AnyElement> = self
            .checklist
            .items
            .iter()
            .zip(self.ticked_by)
            .map(|(item, ticked_by)| {
                let item_id = item.id;
                let row_id = ElementId::NamedChild(
                    Arc::new(id.clone()),
                    SharedString::from(format!("item-{item_id}")),
                );
                let is_checked = item.checked.is_some();
                let focus_handle = remembered_focus_handle(&row_id, window, cx);
                let hover = remembered_hover(&row_id, window, cx);
                let is_hovered = can_tick_now && *hover.read(cx);
                let on_toggle = self.on_toggle.clone().filter(|_| can_tick_now);
                let border = color(if is_checked {
                    tokens.component_poll_answer_border_mine
                } else {
                    tokens.component_poll_answer_border
                });
                let row = div()
                    .id(row_id.clone())
                    .role(Role::CheckBox)
                    .aria_label(item.text.clone())
                    .aria_toggled(if is_checked {
                        Toggled::True
                    } else {
                        Toggled::False
                    })
                    .when_some(ticked_by.clone(), |row, ticked_by| {
                        row.aria_description(ticked_by)
                    })
                    .track_focus(&focus_handle)
                    .relative()
                    .flex()
                    .items_center()
                    .gap(length(COMPONENT_POLL_ANSWER_INNER_GAP))
                    .min_h(length(SIZE_HIT_WEB_MIN))
                    .px(length(COMPONENT_POLL_ANSWER_PADDING_INLINE))
                    .py(length(COMPONENT_POLL_ANSWER_PADDING_BLOCK))
                    .rounded(length(COMPONENT_POLL_ANSWER_RADIUS))
                    .border(length(COMPONENT_POLL_ANSWER_BORDER_WIDTH))
                    .border_color(colour_transition(
                        child_id(&row_id, "border"),
                        border,
                        window,
                        cx,
                    ))
                    .bg(color(if is_hovered {
                        tokens.color_state_hover
                    } else {
                        tokens.component_poll_answer_background
                    }))
                    .when(can_tick_now, gpui::Styled::cursor_pointer)
                    .on_hover(move |is_hovered, _, cx| {
                        hover.update(cx, |remembered, cx| {
                            *remembered = *is_hovered;
                            cx.notify();
                        });
                    })
                    .on_click(move |_, window, cx| {
                        if let Some(on_toggle) = &on_toggle {
                            on_toggle(item_id, window, cx);
                        }
                    })
                    .child(mark(true, is_checked, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                with_type(div(), TYPE_MESSAGE)
                                    .text_color(color(tokens.component_poll_answer_text))
                                    .when(is_checked, gpui::Styled::line_through)
                                    .child(item.text.clone()),
                            )
                            .when_some(ticked_by, |text, ticked_by| {
                                text.child(
                                    with_type(div(), TYPE_BODY_SM)
                                        .text_color(color(tokens.component_poll_count_text))
                                        .child(ticked_by),
                                )
                            }),
                    )
                    .when(is_focus_visible(&focus_handle, window), |row| {
                        row.child(inset_focus_ring(COMPONENT_POLL_ANSWER_RADIUS, cx))
                    });
                inert_for_screen_readers(row, !can_tick_now).into_any_element()
            })
            .collect();

        let edit_button = (self.can_edit && !self.is_pending)
            .then_some(self.on_edit)
            .flatten()
            .map(|on_edit| {
                let (glyph, label) = if is_editing {
                    (None, words.done_editing())
                } else {
                    (Some(IconName::PencilSimple), words.edit())
                };
                link_button(
                    child_id(&id, "edit"),
                    glyph,
                    label.into(),
                    on_edit,
                    window,
                    cx,
                )
            });

        card_frame(id.clone(), title.clone(), COMPONENT_POLL_GAP, cx)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_POLL_HEADER_GAP))
                    .child(card_heading(Some(IconName::ListChecks), title.clone(), cx))
                    .child(level(
                        &child_id(&id, "level"),
                        title,
                        done,
                        total,
                        words.done(i64::from(done), i64::from(total)),
                        words.done_spoken(i64::from(done), i64::from(total)),
                        window,
                        cx,
                    )),
            )
            .map(|card| match self.editor {
                Some(editor) => card.child(editor),
                None => card.child(
                    div()
                        .id(child_id(&id, "items"))
                        .role(Role::List)
                        .flex()
                        .flex_col()
                        .gap(length(COMPONENT_POLL_ANSWER_GAP))
                        .children(rows),
                ),
            })
            .children(edit_button.map(|button| div().flex().child(button)))
    }
}

impl std::fmt::Debug for ChecklistCard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChecklistCard")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// A level with its figure, and a polite live region saying each new value once it changes:
/// screen readers do not announce a progress bar's new value on their own, and a page of history
/// stays quiet (the web's `liveAnnouncer`).
#[allow(clippy::too_many_arguments)] // The meter's own arguments, plus the window.
fn level(
    id: &ElementId,
    label: SharedString,
    value: u32,
    maximum: u32,
    figure: String,
    spoken: String,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let first_spoken = window.use_keyed_state(child_id(id, "first-spoken"), cx, {
        let spoken = spoken.clone();
        move |_, _| (spoken, false)
    });
    let has_changed = {
        let (first, has_changed) = first_spoken.read(cx);
        *has_changed || *first != spoken
    };
    if has_changed && !first_spoken.read(cx).1 {
        first_spoken.update(cx, |remembered, _| remembered.1 = true);
    }
    let announced: SharedString = if has_changed {
        spoken.clone().into()
    } else {
        SharedString::default()
    };
    div()
        .flex()
        .flex_col()
        .child(ProgressMeter::new(
            id.clone(),
            label,
            value,
            maximum,
            figure,
            spoken,
        ))
        .child(polite_live_region(child_id(id, "announcement"), announced))
}

// ---- Progress bar --------------------------------------------------------------------------

/// A progress bar in a message: its label and level; its author and moderators update it.
#[derive(IntoElement)]
pub struct ProgressBarCard {
    id: ElementId,
    bar: ProgressBar,
    is_pending: bool,
    on_update: Option<ActivationHandler>,
}

impl ProgressBarCard {
    #[must_use]
    pub fn new(id: impl Into<ElementId>, bar: ProgressBar) -> Self {
        Self {
            id: id.into(),
            bar,
            is_pending: false,
            on_update: None,
        }
    }

    #[must_use]
    pub fn pending(mut self, is_pending: bool) -> Self {
        self.is_pending = is_pending;
        self
    }

    /// "Update", for its author and moderators.
    #[must_use]
    pub fn on_update(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_update = Some(Arc::new(handler));
        self
    }
}

impl RenderOnce for ProgressBarCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let words = strings(cx).progress_bars();
        let id = self.id;
        let bar = self.bar;
        let percent = i64::from(progress_percent(bar.value, bar.maximum));
        let (value, maximum) = (i64::from(bar.value), i64::from(bar.maximum));
        let label: SharedString = bar.label.clone().into();
        let update_button = self
            .on_update
            .filter(|_| !self.is_pending)
            .map(|on_update| {
                link_button(
                    child_id(&id, "update"),
                    Some(IconName::PencilSimple),
                    words.update().into(),
                    on_update,
                    window,
                    cx,
                )
            });
        card_frame(id.clone(), label.clone(), COMPONENT_POLL_HEADER_GAP, cx)
            .child(card_heading(Some(IconName::Gauge), label.clone(), cx))
            .child(level(
                &child_id(&id, "level"),
                label,
                bar.value,
                bar.maximum,
                words.figure(value, maximum, percent),
                words.spoken(value, maximum, percent),
                window,
                cx,
            ))
            .children(update_button.map(|button| div().flex().child(button)))
    }
}

impl std::fmt::Debug for ProgressBarCard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProgressBarCard")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
