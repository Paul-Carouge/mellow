//! The quick switcher's results (DESIGN.md §5.10): channels, people (conversations) and servers
//! matching what the person types, grouped by kind in the reference mockup's order. `@` keeps people, `#` channels and `*` servers.

use slint::SharedString;

use crate::generated_ui::{QuickSwitcherKind, QuickSwitcherRow};

/// shortcut: a fixed number of results, so the list never needs to scroll to the selection;
/// scroll the list with the arrows if people ask for more.
const MAXIMUM_RESULTS: usize = 8;

/// Somewhere the switcher can go.
#[derive(Debug, Clone, PartialEq)]
pub struct Destination {
    pub kind: QuickSwitcherKind,
    /// The channel, conversation or server id, as the shell's callbacks take it.
    pub id: String,
    /// The server a channel belongs to, or the server itself; empty for conversations.
    pub server_id: String,
    pub name: String,
    /// The server a channel belongs to; empty otherwise.
    pub place: String,
    pub has_unread: bool,
}

/// The rows to show for `query`, and the destination behind each option row, in option order.
pub fn results(
    destinations: &[Destination],
    query: &str,
) -> (Vec<QuickSwitcherRow>, Vec<Destination>) {
    let query = query.trim();
    let (only_kind, text) = match query.chars().next() {
        Some('@') => (Some(QuickSwitcherKind::Conversation), &query[1..]),
        Some('#') => (Some(QuickSwitcherKind::Channel), &query[1..]),
        Some('*') => (Some(QuickSwitcherKind::Server), &query[1..]),
        _ => (None, query),
    };
    let text = text.trim().to_lowercase();
    let mut matches: Vec<(bool, &Destination)> = destinations
        .iter()
        .filter(|destination| only_kind.is_none_or(|kind| destination.kind == kind))
        .filter_map(|destination| {
            let name = destination.name.to_lowercase();
            name.contains(&text)
                .then(|| (name.starts_with(&text), destination))
        })
        .collect();
    // Names starting with the text come first; otherwise the order given (unread, then recent).
    matches.sort_by_key(|(starts_with_text, _)| !starts_with_text);
    matches.truncate(MAXIMUM_RESULTS);

    let mut rows = Vec::new();
    let mut chosen = Vec::new();
    for (kind, group_label) in [
        (QuickSwitcherKind::Channel, "Channels"),
        (QuickSwitcherKind::Conversation, "People"),
        (QuickSwitcherKind::Server, "Servers"),
    ] {
        let in_group: Vec<&Destination> = matches
            .iter()
            .map(|(_, destination)| *destination)
            .filter(|destination| destination.kind == kind)
            .collect();
        if in_group.is_empty() {
            continue;
        }
        rows.push(QuickSwitcherRow {
            is_group: true,
            label: group_label.into(),
            kind,
            place: SharedString::default(),
            has_unread: false,
            option_index: -1,
        });
        for destination in in_group {
            rows.push(QuickSwitcherRow {
                is_group: false,
                label: destination.name.as_str().into(),
                kind,
                place: destination.place.as_str().into(),
                has_unread: destination.has_unread,
                option_index: i32::try_from(chosen.len()).unwrap_or(i32::MAX),
            });
            chosen.push(destination.clone());
        }
    }
    (rows, chosen)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn destination(kind: QuickSwitcherKind, name: &str) -> Destination {
        Destination {
            kind,
            id: name.into(),
            server_id: String::new(),
            name: name.into(),
            place: String::new(),
            has_unread: false,
        }
    }

    fn sample() -> Vec<Destination> {
        vec![
            destination(QuickSwitcherKind::Conversation, "Ana"),
            destination(QuickSwitcherKind::Channel, "general"),
            destination(QuickSwitcherKind::Channel, "off-topic"),
            destination(QuickSwitcherKind::Server, "Rust Lyon"),
        ]
    }

    #[test]
    fn results_are_grouped_by_kind_with_option_indexes_in_order() {
        let (rows, chosen) = results(&sample(), "");
        let labels: Vec<_> = rows.iter().map(|row| row.label.to_string()).collect();
        assert_eq!(
            labels,
            [
                "Channels",
                "general",
                "off-topic",
                "People",
                "Ana",
                "Servers",
                "Rust Lyon"
            ]
        );
        let indexes: Vec<_> = rows.iter().map(|row| row.option_index).collect();
        assert_eq!(indexes, [-1, 0, 1, -1, 2, -1, 3]);
        assert_eq!(chosen[1].name, "off-topic");
    }

    #[test]
    fn prefixes_keep_one_kind_and_names_starting_with_the_text_come_first() {
        let (_, channels) = results(&sample(), "#  ");
        assert!(
            channels
                .iter()
                .all(|destination| destination.kind == QuickSwitcherKind::Channel)
        );
        let (_, found) = results(&sample(), "o");
        let names: Vec<_> = found
            .iter()
            .map(|destination| destination.name.as_str())
            .collect();
        assert_eq!(names, ["off-topic", "Rust Lyon"]);
        let (rows, found) = results(&sample(), "zzz");
        assert!(rows.is_empty() && found.is_empty());
    }
}
