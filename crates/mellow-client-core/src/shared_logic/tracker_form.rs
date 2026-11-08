//! The "Create a checklist" and progress bar forms' rules and their outbox look (the web's
//! `tracker-form.ts` and `tracker-view.ts`, ADR-0047): what the instance would refuse, said
//! before sending, counted as it counts (Unicode scalar values, trimmed). Items may repeat.

use mellow_protocol::{
    Checklist, ChecklistItem, MAXIMUM_CHECKLIST_ITEM_LENGTH, MAXIMUM_CHECKLIST_ITEMS,
    MAXIMUM_CHECKLIST_TITLE_LENGTH, MAXIMUM_PROGRESS_BAR_LABEL_LENGTH,
    MAXIMUM_PROGRESS_BAR_MAXIMUM, NewChecklist, NewProgressBar, ProgressBar,
};

use super::poll_form::{TextProblem, character_count, length_problem};
use super::unicode_classes::js_trim;

/// Rows shown in a new checklist: enough to start, more come with Enter.
const NEW_CHECKLIST_ROWS: usize = 3;
/// A new progress bar counts in percent.
const PERCENT: u32 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistDraft {
    pub title: String,
    /// As typed, blank rows included: blank items are left out when sent.
    pub items: Vec<String>,
}

impl Default for ChecklistDraft {
    fn default() -> Self {
        Self {
            title: String::new(),
            items: vec![String::new(); NEW_CHECKLIST_ROWS],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistDraftProblems {
    pub title: Option<TextProblem>,
    /// One per row.
    pub items_too_long: Vec<bool>,
    pub has_no_item: bool,
}

impl ChecklistDraftProblems {
    #[must_use]
    pub fn has_any(&self) -> bool {
        self.title.is_some() || self.has_no_item || self.items_too_long.contains(&true)
    }
}

#[must_use]
pub fn checklist_draft_problems(draft: &ChecklistDraft) -> ChecklistDraftProblems {
    ChecklistDraftProblems {
        title: length_problem(&draft.title, MAXIMUM_CHECKLIST_TITLE_LENGTH),
        items_too_long: draft
            .items
            .iter()
            .map(|text| character_count(text) > MAXIMUM_CHECKLIST_ITEM_LENGTH)
            .collect(),
        has_no_item: draft.items.iter().all(|text| js_trim(text).is_empty()),
    }
}

/// What is sent, once the draft has no problem.
#[must_use]
pub fn new_checklist_of(draft: &ChecklistDraft) -> NewChecklist {
    NewChecklist {
        title: js_trim(&draft.title).to_owned(),
        items: draft
            .items
            .iter()
            .map(|text| js_trim(text))
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
            .collect(),
    }
}

#[must_use]
pub fn can_add_checklist_row(draft: &ChecklistDraft) -> bool {
    draft.items.len() < MAXIMUM_CHECKLIST_ITEMS
}

/// One item typed on a card (added or renamed): `None` when the instance would refuse it.
#[must_use]
pub fn checklist_item_text(text: &str) -> Option<String> {
    length_problem(text, MAXIMUM_CHECKLIST_ITEM_LENGTH)
        .is_none()
        .then(|| js_trim(text).to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressBarDraft {
    pub label: String,
    /// As typed in the number fields.
    pub value: String,
    pub maximum: String,
}

/// A new bar opens at 0 out of 100; an update starts from the bar as it is.
#[must_use]
pub fn progress_bar_draft_of(bar: Option<&ProgressBar>) -> ProgressBarDraft {
    match bar {
        Some(bar) => ProgressBarDraft {
            label: bar.label.clone(),
            value: bar.value.to_string(),
            maximum: bar.maximum.to_string(),
        },
        None => ProgressBarDraft {
            label: String::new(),
            value: "0".into(),
            maximum: PERCENT.to_string(),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_field_names)] // The web's names, one per field of the form.
pub struct ProgressBarDraftProblems {
    pub label: Option<TextProblem>,
    pub is_value_invalid: bool,
    pub is_maximum_invalid: bool,
}

impl ProgressBarDraftProblems {
    #[must_use]
    pub fn has_any(self) -> bool {
        self.label.is_some() || self.is_value_invalid || self.is_maximum_invalid
    }
}

/// A whole number written in decimal digits only, within the range; `None` otherwise.
fn whole_number_in(text: &str, minimum: u32, maximum: u32) -> Option<u32> {
    let trimmed = js_trim(text);
    if trimmed.is_empty() || !trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    // Too many digits for a u32 is past every maximum.
    let value = trimmed.parse::<u32>().ok()?;
    (minimum..=maximum).contains(&value).then_some(value)
}

#[must_use]
pub fn progress_bar_draft_problems(draft: &ProgressBarDraft) -> ProgressBarDraftProblems {
    let maximum = whole_number_in(&draft.maximum, 1, MAXIMUM_PROGRESS_BAR_MAXIMUM);
    ProgressBarDraftProblems {
        label: length_problem(&draft.label, MAXIMUM_PROGRESS_BAR_LABEL_LENGTH),
        is_maximum_invalid: maximum.is_none(),
        // Judged against a valid maximum only: a wrong maximum is said once, on its own field.
        is_value_invalid: maximum
            .is_some_and(|maximum| whole_number_in(&draft.value, 0, maximum).is_none()),
    }
}

/// What is sent, once the draft has no problem (`None` while it has one).
#[must_use]
pub fn new_progress_bar_of(draft: &ProgressBarDraft) -> Option<NewProgressBar> {
    let maximum = whole_number_in(&draft.maximum, 1, MAXIMUM_PROGRESS_BAR_MAXIMUM)?;
    Some(NewProgressBar {
        label: js_trim(&draft.label).to_owned(),
        value: whole_number_in(&draft.value, 0, maximum)?,
        maximum,
    })
}

/// A checklist still in the outbox, shown as it will arrive: nothing ticked.
#[must_use]
pub fn pending_checklist_preview(checklist: &NewChecklist) -> Checklist {
    Checklist {
        title: checklist.title.clone(),
        items: (1..)
            .zip(&checklist.items)
            .map(|(id, text)| ChecklistItem {
                id,
                text: text.clone(),
                checked: None,
            })
            .collect(),
        revision: 0,
    }
}

#[must_use]
pub fn pending_progress_bar_preview(bar: &NewProgressBar) -> ProgressBar {
    ProgressBar {
        label: bar.label.clone(),
        value: bar.value,
        maximum: bar.maximum,
        revision: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(title: &str, items: &[&str]) -> ChecklistDraft {
        ChecklistDraft {
            title: title.to_owned(),
            items: items.iter().map(|&item| item.to_owned()).collect(),
        }
    }

    fn bar_draft(label: &str, value: &str, maximum: &str) -> ProgressBarDraft {
        ProgressBarDraft {
            label: label.to_owned(),
            value: value.to_owned(),
            maximum: maximum.to_owned(),
        }
    }

    #[test]
    fn needs_a_title_and_one_item_and_sends_the_filled_rows_trimmed() {
        assert_eq!(
            checklist_draft_problems(&ChecklistDraft::default()),
            ChecklistDraftProblems {
                title: Some(TextProblem::Empty),
                items_too_long: vec![false, false, false],
                has_no_item: true,
            }
        );
        let filled = draft(" Release ", &[" Tag", "", "Tag "]);
        assert!(!checklist_draft_problems(&filled).has_any());
        assert_eq!(
            new_checklist_of(&filled),
            NewChecklist {
                title: "Release".into(),
                items: vec!["Tag".into(), "Tag".into()],
            }
        );
    }

    #[test]
    fn says_which_item_is_too_long_and_stops_adding_rows_at_the_maximum() {
        let long = "x".repeat(MAXIMUM_CHECKLIST_ITEM_LENGTH + 1);
        assert_eq!(
            checklist_draft_problems(&draft("t", &[&long, "ok"])).items_too_long,
            [true, false]
        );
        let full = ChecklistDraft {
            title: "t".into(),
            items: vec![String::new(); MAXIMUM_CHECKLIST_ITEMS],
        };
        assert!(!can_add_checklist_row(&full));
        assert_eq!(checklist_item_text("  Deploy "), Some("Deploy".into()));
        assert_eq!(checklist_item_text("   "), None);
        assert_eq!(checklist_item_text(&long), None);
    }

    #[test]
    fn starts_in_percent_or_from_the_bar_being_updated() {
        assert_eq!(progress_bar_draft_of(None), bar_draft("", "0", "100"));
        let bar = ProgressBar {
            label: "Sync".into(),
            value: 3,
            maximum: 5,
            revision: 2,
        };
        assert_eq!(
            progress_bar_draft_of(Some(&bar)),
            bar_draft("Sync", "3", "5")
        );
    }

    #[test]
    fn takes_whole_numbers_from_0_to_the_maximum_only() {
        let problems =
            |value, maximum| progress_bar_draft_problems(&bar_draft("Sync", value, maximum));
        assert!(!problems("5", "5").has_any());
        assert!(problems("6", "5").is_value_invalid);
        assert!(problems("2.5", "5").is_value_invalid);
        assert!(problems("-1", "5").is_value_invalid);
        assert!(problems("", "5").is_value_invalid);
        assert!(problems("1", "99999999999").is_maximum_invalid);
        assert_eq!(
            problems("1", "0"),
            ProgressBarDraftProblems {
                label: None,
                is_value_invalid: false,
                is_maximum_invalid: true,
            }
        );
        assert_eq!(
            new_progress_bar_of(&bar_draft(" Sync ", " 3", "5 ")),
            Some(NewProgressBar {
                label: "Sync".into(),
                value: 3,
                maximum: 5,
            })
        );
    }

    #[test]
    fn shows_trackers_in_the_outbox_as_they_will_arrive() {
        let checklist = pending_checklist_preview(&NewChecklist {
            title: "Release".into(),
            items: vec!["Tag".into(), "Build".into()],
        });
        let items: Vec<(u32, &str, bool)> = checklist
            .items
            .iter()
            .map(|item| (item.id, item.text.as_str(), item.checked.is_some()))
            .collect();
        assert_eq!(items, [(1, "Tag", false), (2, "Build", false)]);
        let bar = pending_progress_bar_preview(&NewProgressBar {
            label: "Sync".into(),
            value: 2,
            maximum: 4,
        });
        assert_eq!((bar.value, bar.maximum, bar.revision), (2, 4, 0));
    }
}
