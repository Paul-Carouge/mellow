//! Checklists and progress bars (ADR-0047) as the client keeps them (the web's `checklists.ts`).

use mellow_protocol::{Checklist, ChecklistTick, ProgressBar};

/// A checklist or progress bar: copies are ordered by their revision.
pub trait Revisioned {
    fn revision(&self) -> u64;
}

impl Revisioned for Checklist {
    fn revision(&self) -> u64 {
        self.revision
    }
}

impl Revisioned for ProgressBar {
    fn revision(&self) -> u64 {
        self.revision
    }
}

/// Keeps the newer of two copies: `incoming` only when its revision is higher, so a pin's
/// `MESSAGE_UPDATE` or a history page read before a tick never undoes it. True when it changed.
pub fn keep_newest<Tracker: Revisioned>(
    cached: &mut Option<Tracker>,
    incoming: Option<Tracker>,
) -> bool {
    let Some(incoming) = incoming else {
        return false;
    };
    if cached
        .as_ref()
        .is_some_and(|cached| incoming.revision() <= cached.revision())
    {
        return false;
    }
    *cached = Some(incoming);
    true
}

/// One item ticked (`Some`) or unticked (`None`). True when it changed.
pub fn set_item_tick(checklist: &mut Checklist, item_id: u32, tick: Option<ChecklistTick>) -> bool {
    let Some(item) = checklist.items.iter_mut().find(|item| item.id == item_id) else {
        return false;
    };
    if item.checked.is_some() == tick.is_some() {
        return false;
    }
    item.checked = tick;
    true
}

/// How many items are ticked, out of all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChecklistProgress {
    pub done: usize,
    pub total: usize,
}

#[must_use]
pub fn checklist_progress(checklist: &Checklist) -> ChecklistProgress {
    ChecklistProgress {
        done: checklist
            .items
            .iter()
            .filter(|item| item.checked.is_some())
            .count(),
        total: checklist.items.len(),
    }
}

/// A whole percentage, rounded down so 99.6 % never reads as done.
#[must_use]
pub fn progress_percent(value: u32, maximum: u32) -> u32 {
    if maximum == 0 {
        return 0;
    }
    let percent = u64::from(value.min(maximum)) * 100 / u64::from(maximum);
    u32::try_from(percent).unwrap_or(100)
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{ChecklistItem, Snowflake};

    use super::*;

    fn tick(user_id: u64, at_unix_milliseconds: u64) -> ChecklistTick {
        ChecklistTick {
            user_id: Snowflake::from_raw(user_id),
            at_unix_milliseconds,
        }
    }

    fn release() -> Checklist {
        Checklist {
            title: "Release".into(),
            items: vec![
                ChecklistItem {
                    id: 1,
                    text: "Tag".into(),
                    checked: Some(tick(2, 5)),
                },
                ChecklistItem {
                    id: 3,
                    text: "Deploy".into(),
                    checked: None,
                },
            ],
            revision: 4,
        }
    }

    fn with_revision(revision: u64) -> Checklist {
        Checklist {
            revision,
            ..release()
        }
    }

    #[test]
    fn keeps_the_copy_of_the_highest_revision_and_the_cached_one_on_a_tie() {
        let mut cached = Some(release());
        assert!(!keep_newest(&mut cached, Some(with_revision(3))));
        assert!(!keep_newest(&mut cached, Some(with_revision(4))));
        assert!(!keep_newest(&mut cached, None));
        assert_eq!(cached, Some(release()));
        assert!(keep_newest(&mut cached, Some(with_revision(5))));
        assert_eq!(cached, Some(with_revision(5)));
        let mut absent = None;
        assert!(keep_newest(&mut absent, Some(with_revision(5))));
    }

    #[test]
    fn ticks_and_unticks_one_item_and_changes_nothing_otherwise() {
        let mut checklist = release();
        assert!(set_item_tick(&mut checklist, 3, Some(tick(1, 9))));
        assert_eq!(checklist.items[1].checked, Some(tick(1, 9)));
        assert_eq!(checklist.items[0], release().items[0]);
        let mut checklist = release();
        assert!(!set_item_tick(&mut checklist, 1, Some(tick(1, 9))));
        assert!(!set_item_tick(&mut checklist, 9, Some(tick(1, 9))));
        assert!(set_item_tick(&mut checklist, 1, None));
        assert_eq!(checklist.items[0].checked, None);
    }

    #[test]
    fn counts_ticked_items_and_rounds_percentages_down() {
        assert_eq!(
            checklist_progress(&release()),
            ChecklistProgress { done: 1, total: 2 }
        );
        assert_eq!(progress_percent(3, 5), 60);
        assert_eq!(progress_percent(995, 1_000), 99);
        assert_eq!(progress_percent(7, 5), 100);
        assert_eq!(progress_percent(1, 0), 0);
    }
}
