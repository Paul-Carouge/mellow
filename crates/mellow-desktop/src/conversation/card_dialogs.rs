//! "Create a poll" (`CreatePollDialog.svelte`, ADR-0043), "Create a checklist"
//! (`CreateChecklistDialog.svelte`) and the progress bar's dialog for creating and updating one
//! (`ProgressBarDialog.svelte`, ADR-0047), opened from the composer's "/" menu or a bar's
//! "Update". A too long or repeated text is said as it is typed; a missing one only once the
//! action is pressed, which then focuses the first field to fix. Enter in a row goes to the next
//! one and adds one after the last; it never sends. Sending goes through the outbox, so the dialog
//! closes at once.
//!
//! shortcut: the poll's duration is a group of radio rows rather than the web's drop-down, and the
//! character counters sit under their field rather than beside its label; a select primitive
//! would bring both closer to the web.

use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, Entity, Focusable as _, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, WeakEntity, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::input::{InputEvent, InputState};
use mellow_client_core::shared_logic::poll_form::{
    AnswerProblem, POLL_ANSWER_COUNTER_THRESHOLD, PollDraft, TextProblem, can_add_answer,
    can_remove_answer, character_count, new_poll_of, poll_draft_problems,
};
use mellow_client_core::shared_logic::tracker_form::{
    ChecklistDraft, ProgressBarDraft, can_add_checklist_row, checklist_draft_problems,
    new_checklist_of, new_progress_bar_of, progress_bar_draft_of, progress_bar_draft_problems,
};
use mellow_client_core::state::MessageCard;
use mellow_protocol::{
    MAXIMUM_CHECKLIST_ITEM_LENGTH, MAXIMUM_CHECKLIST_ITEMS, MAXIMUM_CHECKLIST_TITLE_LENGTH,
    MAXIMUM_POLL_ANSWER_LENGTH, MAXIMUM_POLL_ANSWERS, MAXIMUM_POLL_QUESTION_LENGTH,
    MAXIMUM_PROGRESS_BAR_LABEL_LENGTH, MAXIMUM_PROGRESS_BAR_MAXIMUM, MINIMUM_POLL_ANSWERS,
    POLL_DURATIONS_IN_HOURS, ProgressBar, Snowflake,
};

use super::ConversationView;
use super::cards::CardDialogDone;
use crate::components::{
    Button, ButtonVariant, ChoiceGroup, ChoiceLegend, ChoiceOption, IconButton, ModalDialog,
    ModalDialogEvent, Switch, TextField, with_type,
};
use crate::i18n::{Strings, strings};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_BUTTON_PADDING_INLINE, COMPONENT_DIALOG_SECTION_GAP, COMPONENT_TEXT_FIELD_HELPER_GAP,
    COMPONENT_TEXT_FIELD_ICON_SIZE, COMPONENT_TEXT_FIELD_LABEL_GAP, SIZE_HIT_WEB_MIN, SPACE_2XS,
    SPACE_SM, TYPE_BODY_SM, TYPE_CATEGORY,
};
use crate::theme::{color, length, theme};

/// Past this many characters of a checklist item, its counter shows (the title's always does).
const CHECKLIST_ITEM_COUNTER_THRESHOLD: usize = MAXIMUM_CHECKLIST_ITEM_LENGTH - 20;

/// Which dialog to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CardDialogKind {
    Poll,
    Checklist,
    /// `Some` updates that message's bar; `None` creates one.
    ProgressBar(Option<Snowflake>),
}

type DoneHandler = Rc<dyn Fn(CardDialogDone, &mut App)>;

/// One of the three forms, drawn inside the `ModalDialog` that owns the footer.
pub(super) struct CardForm {
    kind: CardDialogKind,
    /// The question, the checklist's title or the bar's label.
    title: Entity<InputState>,
    /// The poll's answers or the checklist's items, each with its Enter subscription.
    rows: Vec<(Entity<InputState>, Subscription)>,
    /// The bar's value and maximum.
    numbers: Option<(Entity<InputState>, Entity<InputState>)>,
    allows_multiple_answers: bool,
    duration_hours: u32,
    has_tried_sending: bool,
    dialog: Option<WeakEntity<ModalDialog>>,
    on_done: DoneHandler,
    subscriptions: Vec<Subscription>,
}

/// Opens the dialog over the window, focus in its first field; `on_done` hears how it ended.
pub(super) fn open_card_dialog(
    kind: CardDialogKind,
    is_conversation: bool,
    current_bar: Option<&ProgressBar>,
    window: &mut Window,
    cx: &mut Context<ConversationView>,
    on_done: impl Fn(CardDialogDone, &mut App) + 'static,
) -> (Entity<ModalDialog>, Subscription) {
    let catalogue = strings(cx);
    let on_done: DoneHandler = Rc::new(on_done);
    let bar_draft = progress_bar_draft_of(current_bar);
    let form = cx.new(|cx| CardForm::new(kind, &bar_draft, on_done.clone(), window, cx));
    let (title, subtitle, submit) = match kind {
        CardDialogKind::Poll => {
            let polls = catalogue.polls();
            let subtitle = if is_conversation {
                polls.create_subtitle_conversation()
            } else {
                polls.create_subtitle()
            };
            (polls.create(), Some(subtitle), polls.send())
        }
        CardDialogKind::Checklist => {
            let words = catalogue.checklists();
            (words.create(), Some(words.create_subtitle()), words.send())
        }
        CardDialogKind::ProgressBar(None) => {
            let words = catalogue.progress_bars();
            (words.create(), Some(words.create_subtitle()), words.send())
        }
        CardDialogKind::ProgressBar(Some(_)) => {
            let words = catalogue.progress_bars();
            (words.edit(), None, words.save())
        }
    };
    let dialog = cx.new(|cx| {
        let dialog = ModalDialog::new(title, catalogue.common().cancel(), window, cx)
            .submit(submit, submit, false)
            .body(form.clone().into());
        match subtitle {
            Some(subtitle) => dialog.subtitle(subtitle),
            None => dialog,
        }
    });
    form.update(cx, |form, cx| {
        let submitted = cx.subscribe_in(
            &dialog,
            window,
            |form: &mut CardForm, dialog, event: &ModalDialogEvent, window, cx| {
                if *event == ModalDialogEvent::Submitted {
                    form.send(dialog, window, cx);
                }
            },
        );
        form.subscriptions.push(submitted);
        form.dialog = Some(dialog.downgrade());
    });
    let title_focus = form.read(cx).title.read(cx).focus_handle(cx);
    window.focus(&title_focus, cx);
    let subscription = cx.subscribe(&dialog, move |_, _, event: &ModalDialogEvent, cx| {
        if *event == ModalDialogEvent::Closed {
            on_done(CardDialogDone::Closed, cx);
        }
    });
    (dialog, subscription)
}

impl CardForm {
    fn new(
        kind: CardDialogKind,
        bar_draft: &ProgressBarDraft,
        on_done: DoneHandler,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let catalogue = strings(cx);
        let (placeholder, initial_rows, initial_title) = match kind {
            CardDialogKind::Poll => (
                catalogue.polls().question_placeholder(),
                PollDraft::default().answers.len(),
                String::new(),
            ),
            CardDialogKind::Checklist => (
                catalogue.checklists().title_placeholder(),
                ChecklistDraft::default().items.len(),
                String::new(),
            ),
            CardDialogKind::ProgressBar(_) => (
                catalogue.progress_bars().label_placeholder(),
                0,
                bar_draft.label.clone(),
            ),
        };
        let title = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(initial_title)
        });
        let numbers = matches!(kind, CardDialogKind::ProgressBar(_)).then(|| {
            let value =
                cx.new(|cx| InputState::new(window, cx).default_value(bar_draft.value.clone()));
            let maximum =
                cx.new(|cx| InputState::new(window, cx).default_value(bar_draft.maximum.clone()));
            (value, maximum)
        });
        let mut subscriptions = vec![cx.subscribe_in(
            &title,
            window,
            |form: &mut Self, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => form.leave_title(window, cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        )];
        if let Some((value, maximum)) = &numbers {
            for field in [value, maximum] {
                subscriptions.push(cx.subscribe_in(
                    field,
                    window,
                    |form: &mut Self, _, event: &InputEvent, window, cx| match event {
                        InputEvent::PressEnter { .. } => form.submit_from_field(window, cx),
                        InputEvent::Change => cx.notify(),
                        _ => {}
                    },
                ));
            }
        }
        let mut form = Self {
            kind,
            title,
            rows: Vec::new(),
            numbers,
            allows_multiple_answers: false,
            duration_hours: PollDraft::default().duration_hours,
            has_tried_sending: false,
            dialog: None,
            on_done,
            subscriptions,
        };
        for _ in 0..initial_rows {
            form.push_row(window, cx);
        }
        form
    }

    fn row_placeholder(&self, position: usize, catalogue: Strings) -> String {
        let position = i64::try_from(position).unwrap_or(i64::MAX);
        match self.kind {
            CardDialogKind::Poll => catalogue.polls().answer(position),
            _ => catalogue.checklists().item(position),
        }
    }

    fn push_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = self.row_placeholder(self.rows.len() + 1, strings(cx));
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |form: &mut Self, input, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => form.leave_row(input, window, cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        );
        self.rows.push((input, subscription));
    }

    fn can_add_row(&self, cx: &App) -> bool {
        match self.kind {
            CardDialogKind::Poll => can_add_answer(&self.poll_draft(cx)),
            _ => can_add_checklist_row(&self.checklist_draft(cx)),
        }
    }

    /// A poll keeps two answers; a checklist one item.
    fn can_remove_row(&self, cx: &App) -> bool {
        match self.kind {
            CardDialogKind::Poll => can_remove_answer(&self.poll_draft(cx)),
            _ => self.rows.len() > 1,
        }
    }

    fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_add_row(cx) {
            return;
        }
        self.push_row(window, cx);
        if let Some((input, _)) = self.rows.last() {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    /// The row that took its place gets focus, else the one before; placeholders renumber.
    fn remove_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_remove_row(cx) || index >= self.rows.len() {
            return;
        }
        drop(self.rows.remove(index));
        let catalogue = strings(cx);
        for (position, (input, _)) in self.rows.iter().enumerate() {
            let placeholder = self.row_placeholder(position + 1, catalogue);
            input.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx);
            });
        }
        if let Some((input, _)) = self
            .rows
            .get(index)
            .or_else(|| self.rows.get(index.wrapping_sub(1)))
        {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    /// Enter in the title: the first row, or the action for a progress bar.
    fn leave_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.rows.first() {
            Some((input, _)) => input.update(cx, |input, cx| input.focus(window, cx)),
            None => self.submit_from_field(window, cx),
        }
    }

    /// Enter in a row: the next one, adding one after the last.
    fn leave_row(
        &mut self,
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.rows.iter().position(|(row, _)| row == input) else {
            return;
        };
        match self.rows.get(index + 1) {
            Some((next, _)) => next.update(cx, |next, cx| next.focus(window, cx)),
            None => self.add_row(window, cx),
        }
    }

    fn submit_from_field(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dialog) = self.dialog.as_ref().and_then(WeakEntity::upgrade) {
            dialog.update(cx, ModalDialog::submit_now);
        }
    }

    fn text_of(input: &Entity<InputState>, cx: &App) -> String {
        input.read(cx).value().to_string()
    }

    fn row_texts(&self, cx: &App) -> Vec<String> {
        self.rows
            .iter()
            .map(|(input, _)| Self::text_of(input, cx))
            .collect()
    }

    fn poll_draft(&self, cx: &App) -> PollDraft {
        PollDraft {
            question: Self::text_of(&self.title, cx),
            answers: self.row_texts(cx),
            allows_multiple_answers: self.allows_multiple_answers,
            duration_hours: self.duration_hours,
        }
    }

    fn checklist_draft(&self, cx: &App) -> ChecklistDraft {
        ChecklistDraft {
            title: Self::text_of(&self.title, cx),
            items: self.row_texts(cx),
        }
    }

    fn bar_draft(&self, cx: &App) -> ProgressBarDraft {
        let (value, maximum) = self
            .numbers
            .as_ref()
            .map(|(value, maximum)| (Self::text_of(value, cx), Self::text_of(maximum, cx)))
            .unwrap_or_default();
        ProgressBarDraft {
            label: Self::text_of(&self.title, cx),
            value,
            maximum,
        }
    }

    /// What the action sends, or the field to fix first.
    fn card_or_first_problem(&self, cx: &App) -> Result<CardDialogDone, Entity<InputState>> {
        let first_row_where = |is_wrong: &dyn Fn(usize) -> bool| {
            (0..self.rows.len())
                .find(|index| is_wrong(*index))
                .or((!self.rows.is_empty()).then_some(0))
                .and_then(|index| self.rows.get(index))
                .map_or_else(|| self.title.clone(), |(input, _)| input.clone())
        };
        match self.kind {
            CardDialogKind::Poll => {
                let draft = self.poll_draft(cx);
                let problems = poll_draft_problems(&draft);
                if !problems.has_any() {
                    return Ok(CardDialogDone::Send(MessageCard::Poll(new_poll_of(&draft))));
                }
                if problems.question.is_some() {
                    return Err(self.title.clone());
                }
                let first_problem = problems.answers.iter().position(Option::is_some);
                let first_empty = draft.answers.iter().position(|text| text.trim().is_empty());
                Err(first_row_where(&|index| {
                    Some(index) == first_problem.or(first_empty)
                }))
            }
            CardDialogKind::Checklist => {
                let draft = self.checklist_draft(cx);
                let problems = checklist_draft_problems(&draft);
                if !problems.has_any() {
                    return Ok(CardDialogDone::Send(MessageCard::Checklist(
                        new_checklist_of(&draft),
                    )));
                }
                if problems.title.is_some() {
                    return Err(self.title.clone());
                }
                Err(first_row_where(&|index| problems.items_too_long[index]))
            }
            CardDialogKind::ProgressBar(message_id) => {
                let draft = self.bar_draft(cx);
                let problems = progress_bar_draft_problems(&draft);
                if let Some(bar) = new_progress_bar_of(&draft).filter(|_| !problems.has_any()) {
                    return Ok(match message_id {
                        Some(message_id) => CardDialogDone::UpdateProgressBar(message_id, bar),
                        None => CardDialogDone::Send(MessageCard::ProgressBar(bar)),
                    });
                }
                let (value, maximum) = self.numbers.clone().unzip();
                Err(if problems.label.is_some() {
                    self.title.clone()
                } else if problems.is_maximum_invalid {
                    maximum.unwrap_or_else(|| self.title.clone())
                } else {
                    value.unwrap_or_else(|| self.title.clone())
                })
            }
        }
    }

    fn send(&mut self, dialog: &Entity<ModalDialog>, window: &mut Window, cx: &mut Context<Self>) {
        match self.card_or_first_problem(cx) {
            Ok(done) => {
                dialog.update(cx, |dialog, cx| dialog.request_close(window, cx));
                (self.on_done)(done, cx);
            }
            Err(field) => {
                self.has_tried_sending = true;
                field.update(cx, |field, cx| field.focus(window, cx));
                cx.notify();
            }
        }
    }

    // ---- Drawing ----

    fn text_problem_sentence(
        &self,
        problem: Option<TextProblem>,
        maximum: usize,
        empty_sentence: &str,
        catalogue: Strings,
    ) -> Option<String> {
        match problem {
            Some(TextProblem::TooLong) => Some(
                catalogue
                    .polls()
                    .text_too_long(i64::try_from(maximum).unwrap_or(i64::MAX)),
            ),
            Some(TextProblem::Empty) if self.has_tried_sending => Some(empty_sentence.to_owned()),
            _ => None,
        }
    }

    fn counter(used: usize, maximum: usize, catalogue: Strings) -> String {
        let (used, maximum) = (
            i64::try_from(used).unwrap_or(i64::MAX),
            i64::try_from(maximum).unwrap_or(i64::MAX),
        );
        catalogue.polls().characters_used(used, maximum)
    }

    fn title_field(&self, cx: &App) -> TextField {
        let catalogue = strings(cx);
        let text = Self::text_of(&self.title, cx);
        let (label, maximum, problem, empty_sentence) = match self.kind {
            CardDialogKind::Poll => (
                catalogue.polls().question(),
                MAXIMUM_POLL_QUESTION_LENGTH,
                poll_draft_problems(&self.poll_draft(cx)).question,
                catalogue.polls().question_empty(),
            ),
            CardDialogKind::Checklist => (
                catalogue.checklists().title(),
                MAXIMUM_CHECKLIST_TITLE_LENGTH,
                checklist_draft_problems(&self.checklist_draft(cx)).title,
                catalogue.checklists().title_empty(),
            ),
            CardDialogKind::ProgressBar(_) => (
                catalogue.progress_bars().label(),
                MAXIMUM_PROGRESS_BAR_LABEL_LENGTH,
                progress_bar_draft_problems(&self.bar_draft(cx)).label,
                catalogue.progress_bars().label_empty(),
            ),
        };
        let field = TextField::new("card-dialog-title", label, &self.title).helper(Self::counter(
            character_count(&text),
            maximum,
            catalogue,
        ));
        match self.text_problem_sentence(problem, maximum, empty_sentence, catalogue) {
            Some(error) => field.error(error),
            None => field,
        }
    }

    /// The rows' sentences, one per row, and the one said under all of them.
    fn row_messages(&self, cx: &App) -> (Vec<Option<String>>, Option<String>) {
        let catalogue = strings(cx);
        let too_long = |maximum: usize| {
            catalogue
                .polls()
                .text_too_long(i64::try_from(maximum).unwrap_or(i64::MAX))
        };
        if self.kind == CardDialogKind::Poll {
            let draft = self.poll_draft(cx);
            let problems = poll_draft_problems(&draft);
            let rows = problems
                .answers
                .iter()
                .zip(&draft.answers)
                .map(|(problem, text)| match problem {
                    Some(AnswerProblem::TooLong) => Some(too_long(MAXIMUM_POLL_ANSWER_LENGTH)),
                    Some(AnswerProblem::Duplicate) => {
                        Some(catalogue.polls().duplicate_answer(text.trim()))
                    }
                    None => None,
                })
                .collect();
            let minimum = i64::try_from(MINIMUM_POLL_ANSWERS).unwrap_or(i64::MAX);
            let below = (problems.has_too_few_answers && self.has_tried_sending)
                .then(|| catalogue.polls().too_few_answers(minimum));
            return (rows, below);
        }
        let problems = checklist_draft_problems(&self.checklist_draft(cx));
        let rows = problems
            .items_too_long
            .iter()
            .map(|is_too_long| is_too_long.then(|| too_long(MAXIMUM_CHECKLIST_ITEM_LENGTH)))
            .collect();
        let below = (problems.has_no_item && self.has_tried_sending)
            .then(|| catalogue.checklists().too_few_items().to_owned());
        (rows, below)
    }

    #[allow(clippy::too_many_lines)] // One section, in the order it is drawn.
    fn render_rows(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let is_poll = self.kind == CardDialogKind::Poll;
        let (maximum, threshold) = if is_poll {
            (MAXIMUM_POLL_ANSWER_LENGTH, POLL_ANSWER_COUNTER_THRESHOLD)
        } else {
            (
                MAXIMUM_CHECKLIST_ITEM_LENGTH,
                CHECKLIST_ITEM_COUNTER_THRESHOLD,
            )
        };
        let (messages, below) = self.row_messages(cx);
        let can_remove = self.can_remove_row(cx);
        let entity = cx.entity();
        let rows =
            self.rows
                .iter()
                .zip(messages)
                .enumerate()
                .map(|(index, ((input, _), message))| {
                    let position = i64::try_from(index + 1).unwrap_or(i64::MAX);
                    let label = self.row_placeholder(index + 1, catalogue);
                    let used = character_count(&Self::text_of(input, cx));
                    let field =
                        TextField::new(("card-dialog-row", index), label, input).label_hidden();
                    let field = if used > threshold {
                        field.helper(Self::counter(used, maximum, catalogue))
                    } else {
                        field
                    };
                    let field = match message {
                        Some(message) => field.error(message),
                        None => field,
                    };
                    let remove_label = if is_poll {
                        catalogue.polls().remove_answer(position)
                    } else {
                        catalogue.checklists().remove_item(position)
                    };
                    let entity = entity.clone();
                    div()
                        .flex()
                        .items_start()
                        .gap(length(SPACE_2XS))
                        .child(div().flex_1().min_w_0().child(field))
                        .when(can_remove, |row| {
                            row.child(
                                IconButton::new(
                                    ("card-dialog-remove", index),
                                    IconName::X,
                                    remove_label,
                                )
                                .sizes(SIZE_HIT_WEB_MIN, COMPONENT_TEXT_FIELD_ICON_SIZE)
                                .on_click(move |window, cx| {
                                    entity
                                        .update(cx, |form, cx| form.remove_row(index, window, cx));
                                }),
                            )
                        })
                });
        let rows_left = i64::try_from(
            if is_poll {
                MAXIMUM_POLL_ANSWERS
            } else {
                MAXIMUM_CHECKLIST_ITEMS
            }
            .saturating_sub(self.rows.len()),
        )
        .unwrap_or(0);
        let (heading, add_label, left_sentence) = if is_poll {
            (
                catalogue.polls().answers(),
                catalogue.polls().add_answer(),
                catalogue.polls().answers_left(rows_left),
            )
        } else {
            (
                catalogue.checklists().items(),
                catalogue.checklists().add_item(),
                catalogue.checklists().items_left(rows_left),
            )
        };
        let add_entity = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_TEXT_FIELD_LABEL_GAP))
            .child(
                with_type(div(), TYPE_CATEGORY)
                    .text_color(color(tokens.component_text_field_label_text))
                    .child(heading),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_TEXT_FIELD_HELPER_GAP))
                    .children(rows),
            )
            .when_some(below, |section, below| {
                section.child(
                    with_type(div(), TYPE_BODY_SM)
                        .text_color(color(tokens.component_dialog_error_text))
                        .child(below),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x(length(SPACE_SM))
                    .when(self.can_add_row(cx), |footer| {
                        // Its label lines up with the fields' edge; the hover wash reaches past it.
                        footer.child(
                            div().ml(-length(COMPONENT_BUTTON_PADDING_INLINE)).child(
                                Button::new("card-dialog-add", add_label)
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(move |window, cx| {
                                        add_entity.update(cx, |form, cx| form.add_row(window, cx));
                                    }),
                            ),
                        )
                    })
                    .child(
                        with_type(div(), TYPE_BODY_SM)
                            .text_color(color(tokens.component_text_field_helper_text))
                            .child(left_sentence),
                    ),
            )
            .into_any_element()
    }

    fn render_poll_options(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let polls = strings(cx).polls();
        let switch_entity = cx.entity();
        let duration_entity = cx.entity();
        let options = POLL_DURATIONS_IN_HOURS
            .iter()
            .map(|hours| ChoiceOption::new(polls.duration_option(i64::from(*hours))))
            .collect();
        let checked = POLL_DURATIONS_IN_HOURS
            .iter()
            .position(|hours| *hours == self.duration_hours)
            .unwrap_or(0);
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_DIALOG_SECTION_GAP))
            .child(
                Switch::new(
                    "card-dialog-multiple",
                    self.allows_multiple_answers,
                    polls.allow_multiple(),
                )
                .description(polls.allow_multiple_hint())
                .without_divider()
                .on_change(move |is_on, _, cx| {
                    switch_entity.update(cx, |form, cx| {
                        form.allows_multiple_answers = is_on;
                        cx.notify();
                    });
                }),
            )
            .child(
                ChoiceGroup::new("card-dialog-duration", polls.duration(), options, checked)
                    .legend(ChoiceLegend::Field)
                    .on_select(move |index, _, cx| {
                        duration_entity.update(cx, |form, cx| {
                            if let Some(hours) = POLL_DURATIONS_IN_HOURS.get(index) {
                                form.duration_hours = *hours;
                            }
                            cx.notify();
                        });
                    }),
            )
            .into_any_element()
    }

    fn render_numbers(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let (value, maximum) = self.numbers.as_ref()?;
        let tokens = theme(cx);
        let words = strings(cx).progress_bars();
        let draft = self.bar_draft(cx);
        let problems = progress_bar_draft_problems(&draft);
        let maximum_number = i64::from(MAXIMUM_PROGRESS_BAR_MAXIMUM);
        let typed_maximum = draft.maximum.trim().parse::<i64>().unwrap_or(0);
        let value_field = TextField::new("card-dialog-value", words.value(), value);
        let value_field = if problems.is_value_invalid && self.has_tried_sending {
            value_field.error(words.whole_number_up_to(0, typed_maximum))
        } else {
            value_field
        };
        let maximum_field = TextField::new("card-dialog-maximum", words.maximum(), maximum);
        let maximum_field = if problems.is_maximum_invalid && self.has_tried_sending {
            maximum_field.error(words.whole_number_up_to(1, maximum_number))
        } else {
            maximum_field
        };
        Some(
            div()
                .flex()
                .flex_col()
                .gap(length(COMPONENT_TEXT_FIELD_HELPER_GAP))
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(length(SPACE_SM))
                        .child(div().flex_1().min_w_0().child(value_field))
                        .child(div().flex_1().min_w_0().child(maximum_field)),
                )
                .child(
                    with_type(div(), TYPE_BODY_SM)
                        .text_color(color(tokens.component_text_field_helper_text))
                        .child(words.maximum_hint()),
                )
                .into_any_element(),
        )
    }
}

impl Render for CardForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.title_field(cx);
        let rows = if self.rows.is_empty() {
            None
        } else {
            Some(self.render_rows(cx))
        };
        let options = if self.kind == CardDialogKind::Poll {
            Some(self.render_poll_options(cx))
        } else {
            None
        };
        let numbers = self.render_numbers(cx);
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_DIALOG_SECTION_GAP))
            .child(title)
            .children(rows)
            .children(options)
            .children(numbers)
    }
}

impl std::fmt::Debug for CardForm {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CardForm")
            .field("kind", &self.kind)
            .field("rows", &self.rows.len())
            .finish_non_exhaustive()
    }
}
