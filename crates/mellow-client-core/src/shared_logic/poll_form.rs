//! The "Create a poll" form's rules (the web's `poll-form.ts`, ADR-0043): what the instance would
//! refuse, said before sending, counted as the instance counts (Unicode scalar values, trimmed).

use mellow_protocol::{
    MAXIMUM_POLL_ANSWER_LENGTH, MAXIMUM_POLL_ANSWERS, MAXIMUM_POLL_QUESTION_LENGTH,
    MINIMUM_POLL_ANSWERS, NewPoll,
};

use super::unicode_classes::js_trim;

/// Discord's default: a day.
pub const DEFAULT_POLL_DURATION_HOURS: u32 = 24;

/// Past this many characters of an answer, its counter shows (the question's always does).
pub const POLL_ANSWER_COUNTER_THRESHOLD: usize = MAXIMUM_POLL_ANSWER_LENGTH - 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollDraft {
    pub question: String,
    /// As typed, blank rows included: blank answers are left out when sent.
    pub answers: Vec<String>,
    pub allows_multiple_answers: bool,
    pub duration_hours: u32,
}

impl Default for PollDraft {
    fn default() -> Self {
        Self {
            question: String::new(),
            answers: vec![String::new(); MINIMUM_POLL_ANSWERS],
            allows_multiple_answers: false,
            duration_hours: DEFAULT_POLL_DURATION_HOURS,
        }
    }
}

/// Characters as the instance counts them: trimmed, one per Unicode scalar value.
#[must_use]
pub fn character_count(text: &str) -> usize {
    js_trim(text).chars().count()
}

/// What is wrong with a text that must not be empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextProblem {
    Empty,
    TooLong,
}

/// What is wrong with one answer row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerProblem {
    TooLong,
    /// The same as an earlier row, case and outer spaces ignored.
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollDraftProblems {
    pub question: Option<TextProblem>,
    /// One per row; a repeat is marked on its later rows, the first stays clean.
    pub answers: Vec<Option<AnswerProblem>>,
    pub has_too_few_answers: bool,
}

impl PollDraftProblems {
    #[must_use]
    pub fn has_any(&self) -> bool {
        self.question.is_some()
            || self.has_too_few_answers
            || self.answers.iter().any(Option::is_some)
    }
}

/// Empty, or past `maximum` characters.
#[must_use]
pub fn length_problem(text: &str, maximum: usize) -> Option<TextProblem> {
    match character_count(text) {
        0 => Some(TextProblem::Empty),
        length if length > maximum => Some(TextProblem::TooLong),
        _ => None,
    }
}

#[must_use]
pub fn poll_draft_problems(draft: &PollDraft) -> PollDraftProblems {
    let mut seen = Vec::new();
    let answers = draft
        .answers
        .iter()
        .map(|text| {
            // Case ignored, as the instance compares them.
            let folded = js_trim(text).to_lowercase();
            if folded.is_empty() {
                return None;
            }
            if character_count(text) > MAXIMUM_POLL_ANSWER_LENGTH {
                return Some(AnswerProblem::TooLong);
            }
            if seen.contains(&folded) {
                return Some(AnswerProblem::Duplicate);
            }
            seen.push(folded);
            None
        })
        .collect();
    PollDraftProblems {
        question: length_problem(&draft.question, MAXIMUM_POLL_QUESTION_LENGTH),
        answers,
        has_too_few_answers: filled_answers(draft).len() < MINIMUM_POLL_ANSWERS,
    }
}

fn filled_answers(draft: &PollDraft) -> Vec<String> {
    draft
        .answers
        .iter()
        .map(|text| js_trim(text))
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .collect()
}

/// What is sent, once the draft has no problem.
#[must_use]
pub fn new_poll_of(draft: &PollDraft) -> NewPoll {
    NewPoll {
        question: js_trim(&draft.question).to_owned(),
        answers: filled_answers(draft),
        allows_multiple_answers: draft.allows_multiple_answers,
        duration_hours: draft.duration_hours,
    }
}

#[must_use]
pub fn can_add_answer(draft: &PollDraft) -> bool {
    draft.answers.len() < MAXIMUM_POLL_ANSWERS
}

#[must_use]
pub fn can_remove_answer(draft: &PollDraft) -> bool {
    draft.answers.len() > MINIMUM_POLL_ANSWERS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(question: &str, answers: &[&str]) -> PollDraft {
        PollDraft {
            question: question.to_owned(),
            answers: answers.iter().map(|&answer| answer.to_owned()).collect(),
            ..PollDraft::default()
        }
    }

    #[test]
    fn starts_with_two_empty_answers_for_a_day_one_choice() {
        assert_eq!(
            PollDraft::default(),
            PollDraft {
                question: String::new(),
                answers: vec![String::new(), String::new()],
                allows_multiple_answers: false,
                duration_hours: 24,
            }
        );
    }

    #[test]
    fn counts_characters_as_the_instance_does_trimmed_an_emoji_is_one() {
        assert_eq!(character_count("  Pizza "), 5);
        assert_eq!(character_count("🍕🍕"), 2);
    }

    #[test]
    fn asks_for_a_question_and_two_answers() {
        let problems = poll_draft_problems(&draft("  ", &["Pizza", " "]));
        assert_eq!(problems.question, Some(TextProblem::Empty));
        assert!(problems.has_too_few_answers);
        assert!(problems.has_any());
    }

    #[test]
    fn refuses_texts_past_their_limits_emoji_counted_once() {
        let long_question = "🍕".repeat(MAXIMUM_POLL_QUESTION_LENGTH + 1);
        let long_answer = "a".repeat(MAXIMUM_POLL_ANSWER_LENGTH + 1);
        let problems = poll_draft_problems(&draft(&long_question, &["Pizza", &long_answer]));
        assert_eq!(problems.question, Some(TextProblem::TooLong));
        assert_eq!(problems.answers, [None, Some(AnswerProblem::TooLong)]);
        let at_limit = "🍕".repeat(MAXIMUM_POLL_QUESTION_LENGTH);
        assert_eq!(
            poll_draft_problems(&draft(&at_limit, &["a", "b"])).question,
            None
        );
    }

    #[test]
    fn marks_a_repeated_answer_on_its_later_rows_case_and_spaces_ignored() {
        let problems =
            poll_draft_problems(&draft("Lunch?", &["Pizza", "Soup", " pizza ", "", "PIZZA"]));
        let duplicate = Some(AnswerProblem::Duplicate);
        assert_eq!(problems.answers, [None, None, duplicate, None, duplicate]);
        assert!(!problems.has_too_few_answers);
    }

    #[test]
    fn sends_trimmed_texts_without_blank_rows() {
        let filled = PollDraft {
            allows_multiple_answers: true,
            duration_hours: 72,
            ..draft(" Lunch? ", &[" Pizza", "", "Soup "])
        };
        assert!(!poll_draft_problems(&filled).has_any());
        assert_eq!(
            new_poll_of(&filled),
            NewPoll {
                question: "Lunch?".into(),
                answers: vec!["Pizza".into(), "Soup".into()],
                allows_multiple_answers: true,
                duration_hours: 72,
            }
        );
    }

    #[test]
    fn keeps_between_two_and_ten_rows() {
        assert!(!can_remove_answer(&PollDraft::default()));
        assert!(can_add_answer(&PollDraft::default()));
        let answers: Vec<String> = (0..MAXIMUM_POLL_ANSWERS)
            .map(|index| format!("A{index}"))
            .collect();
        let full = PollDraft {
            answers,
            ..draft("Q", &[])
        };
        assert!(!can_add_answer(&full));
        assert!(can_remove_answer(&full));
    }
}
