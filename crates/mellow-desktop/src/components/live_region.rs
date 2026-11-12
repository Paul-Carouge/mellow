//! A region screen readers read out politely, as the web's `aria-live="polite"`.

use gpui::{
    ElementId, InteractiveElement as _, IntoElement, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
};

/// Read out politely by screen readers each time `text` changes, as the web's `aria-live="polite"`.
/// GPUI has no live-region setter, so the node is marked live through its AccessKit node; AccessKit
/// announces a live node whose value changes (`NSAccessibilityAnnouncementRequestedNotification`
/// on macOS, a text change event on AT-SPI). The same text twice in a row is read once.
pub fn polite_live_region(id: impl Into<ElementId>, text: SharedString) -> impl IntoElement {
    div()
        .id(id)
        .role(Role::Status)
        .when(!text.is_empty(), |region| region.aria_value(text))
        .a11y_synthetic_children(|subtree| {
            subtree
                .parent_node()
                .set_live(gpui::accesskit::Live::Polite);
        })
        .size_0()
        .overflow_hidden()
}
