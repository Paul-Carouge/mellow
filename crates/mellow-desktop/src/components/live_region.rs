//! Regions screen readers read out, as the web's `aria-live="polite"` (`role="status"`) and
//! `aria-live="assertive"` (`role="alert"`).
//!
//! AccessKit takes `live` from the node or its ancestors, never from the role, and its macOS
//! adapter announces a live node only when its value (`aria_value`) changes, not its label. So a
//! bare `.role(Role::Status)` or `.role(Role::Alert)` is silent: go through this module instead.

use gpui::{
    ElementId, InteractiveElement as _, IntoElement, Role, SharedString,
    StatefulInteractiveElement, Styled as _,
    accesskit::{Live, Node},
    div,
};

/// Makes an element with an id a region screen readers read out each time its text changes.
/// GPUI has no live-region setter, so the node is marked live through its AccessKit node; AccessKit
/// announces a live node whose value changes (`NSAccessibilityAnnouncementRequestedNotification`
/// on macOS, a text change event on AT-SPI). The same text twice in a row is read once.
pub trait LiveRegion: StatefulInteractiveElement {
    /// Read out when the reader is idle, as the web's `role="status"`: progress, counts, results.
    #[must_use]
    fn live_polite(self, text: impl Into<SharedString>) -> Self {
        mark_live(self, Role::Status, Live::Polite, text.into())
    }

    /// Read out at once, interrupting, as the web's `role="alert"`: errors that need attention now.
    #[must_use]
    fn live_assertive(self, text: impl Into<SharedString>) -> Self {
        mark_live(self, Role::Alert, Live::Assertive, text.into())
    }
}

impl<E: StatefulInteractiveElement> LiveRegion for E {}

fn mark_live<E: StatefulInteractiveElement>(
    element: E,
    role: Role,
    live: Live,
    text: SharedString,
) -> E {
    let element = element.role(role);
    let element = if text.is_empty() {
        element
    } else {
        element.aria_value(text)
    };
    element.a11y_synthetic_children(move |subtree| set_node_live(subtree.parent_node(), live))
}

/// The one AccessKit property GPUI has no setter for.
fn set_node_live(node: &mut Node, live: Live) {
    node.set_live(live);
}

/// An unseen region read out politely each time `text` changes (see [`LiveRegion::live_polite`]).
pub fn polite_live_region(id: impl Into<ElementId>, text: SharedString) -> impl IntoElement {
    div().id(id).live_polite(text).size_0().overflow_hidden()
}

/// An unseen region read out at once each time `text` changes (see
/// [`LiveRegion::live_assertive`]).
pub fn assertive_live_region(id: impl Into<ElementId>, text: SharedString) -> impl IntoElement {
    div().id(id).live_assertive(text).size_0().overflow_hidden()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // GPUI's test windows never activate accessibility (`TestWindow` keeps the default no-op
    // `a11y_init`, so no AccessKit tree is ever built), so the node is checked at the lowest level
    // reachable: the AccessKit node GPUI hands to the `a11y_synthetic_children` closure, with the
    // role and value GPUI sets from `.role(...)` and `.aria_value(...)`.
    #[test]
    fn status_and_alert_nodes_are_live_and_carry_their_text_as_value() {
        for (role, live) in [(Role::Status, Live::Polite), (Role::Alert, Live::Assertive)] {
            let mut node = Node::new(role);
            node.set_value("Saved");
            set_node_live(&mut node, live);
            assert_eq!(node.role(), role);
            assert_eq!(node.live(), Some(live));
            assert_eq!(node.value(), Some("Saved"));
        }
    }

    fn rust_files_under(directory: &Path, files: &mut Vec<std::path::PathBuf>) {
        let entries = std::fs::read_dir(directory).expect("the source directory is readable");
        for entry in entries {
            let path = entry.expect("a directory entry is readable").path();
            if path.is_dir() {
                rust_files_under(&path, files);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }

    /// A bare status or alert role is silent; every one goes through [`LiveRegion`].
    #[test]
    fn no_status_or_alert_role_is_set_outside_this_module() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_files_under(&source, &mut files);
        let offenders: Vec<String> = files
            .iter()
            .filter(|path| !path.ends_with("components/live_region.rs"))
            .flat_map(|path| {
                let text = std::fs::read_to_string(path).expect("a source file is readable");
                text.lines()
                    .enumerate()
                    .filter(|(_, line)| {
                        line.contains("Role::Status") || line.contains("Role::Alert")
                    })
                    .map(|(index, _)| format!("{}:{}", path.display(), index + 1))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(
            offenders.is_empty(),
            "silent status or alert roles, use LiveRegion: {offenders:#?}"
        );
    }
}
