//! The web's `createPopoverToggle`: a popover closed by a click outside remembers where, because
//! that same click then lands on its toggle, which must not open it again.

use gpui::{Pixels, Point};

/// The click that last closed a popover, and which opener it was on (`()` when there is only one).
#[derive(Debug)]
pub struct OutsideClickDismissal<Opener = ()> {
    closing_click: Option<(Point<Pixels>, Opener)>,
}

// By hand: a derived `Default` would require `Opener: Default`.
impl<Opener> Default for OutsideClickDismissal<Opener> {
    fn default() -> Self {
        Self {
            closing_click: None,
        }
    }
}

impl<Opener: PartialEq> OutsideClickDismissal<Opener> {
    pub fn remember(&mut self, position: Point<Pixels>, opener: Opener) {
        self.closing_click = Some((position, opener));
    }

    pub fn forget(&mut self) {
        self.closing_click = None;
    }

    /// Whether the toggle pressed at `position` on `opener` is the click that closed the popover.
    /// Forgets that click either way: it answers only the toggle press that follows it.
    pub fn is_closing_click(&mut self, position: Point<Pixels>, opener: &Opener) -> bool {
        self.closing_click
            .take()
            .is_some_and(|(closed_at, closed_on)| closed_at == position && closed_on == *opener)
    }
}

#[cfg(test)]
mod tests {
    use gpui::{point, px};

    use super::OutsideClickDismissal;

    #[test]
    fn only_the_next_toggle_at_the_closing_click_on_its_opener_is_ignored() {
        let here = point(px(10.), px(20.));
        let elsewhere = point(px(11.), px(20.));
        let mut dismissal = OutsideClickDismissal::<u8>::default();
        assert!(!dismissal.is_closing_click(here, &1));

        dismissal.remember(here, 1);
        assert!(dismissal.is_closing_click(here, &1));
        assert!(!dismissal.is_closing_click(here, &1), "answered once");

        dismissal.remember(here, 1);
        assert!(!dismissal.is_closing_click(here, &2), "another opener");
        assert!(
            !dismissal.is_closing_click(here, &1),
            "forgotten by that press"
        );

        dismissal.remember(here, 1);
        assert!(!dismissal.is_closing_click(elsewhere, &1), "another place");

        dismissal.remember(here, 1);
        dismissal.forget();
        assert!(!dismissal.is_closing_click(here, &1));
    }
}
