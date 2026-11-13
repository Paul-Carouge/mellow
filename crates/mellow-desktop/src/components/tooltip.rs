//! The tooltip (DESIGN.md §6.34): a floating label on `surface.floating` with the high shadow,
//! shown after `motion.delay.tooltip`. The web relies on the browser's `title` for most buttons;
//! the desktop draws every one.
//!
//! GPUI keeps a tooltip while the pointer stays within its trigger's bounds, even once something
//! covers the trigger, and shows one whose delay ran out after a press. As the browser's `title`
//! does, a tooltip here hides on a press, on a keystroke and while the window is in the
//! background, and one due during a press stays hidden until the pointer leaves its trigger and
//! comes back: [`Tooltip::dismissal_layer`], drawn once at the window's root, sees those events.

use std::time::Duration;

use gpui::{
    AnyView, App, AppContext as _, Bounds, Context, Global, InteractiveElement as _, IntoElement,
    LongPressEvent, MouseDownEvent, MouseMoveEvent, ParentElement as _, Pixels, Point, Render,
    SharedString, Styled as _, TouchPhase, Window, canvas, div, point, px,
};

use super::{shadow, with_type};
use crate::theme::tokens::{
    COMPONENT_TOOLTIP_DELAY, COMPONENT_TOOLTIP_MAX_WIDTH, COMPONENT_TOOLTIP_PADDING_BLOCK,
    COMPONENT_TOOLTIP_PADDING_INLINE, COMPONENT_TOOLTIP_RADIUS, SPACE_2XS, TYPE_TOOLTIP,
};
use crate::theme::{color, delay, length, theme};

/// Counts the dismissals (presses, keys) so a tooltip shown before the latest one hides.
#[derive(Debug, Default)]
struct TooltipDismissals {
    count: u64,
    /// Set by a dismissal and cleared by the next pointer move without a button held (or a long
    /// press starting): a tooltip whose delay ends in between was due under the press and is
    /// never shown; GPUI builds the next one once the pointer leaves its trigger and comes back.
    is_holding_back_new_tooltips: bool,
    /// Whether a tooltip was drawn since the last dismissal: only then does a dismissal redraw
    /// the window, not on every keystroke (push to talk held repeats it).
    is_a_tooltip_drawn: bool,
    /// Keystrokes are intercepted once for the whole app.
    is_watching_keystrokes: bool,
}

impl Global for TooltipDismissals {}

fn dismiss_tooltips(window: &mut Window, cx: &mut App) {
    let dismissals = cx.default_global::<TooltipDismissals>();
    dismissals.count += 1;
    dismissals.is_holding_back_new_tooltips = true;
    if std::mem::take(&mut dismissals.is_a_tooltip_drawn) {
        window.refresh();
    }
}

fn stop_holding_back_new_tooltips(cx: &mut App) {
    if cx
        .try_global::<TooltipDismissals>()
        .is_some_and(|dismissals| dismissals.is_holding_back_new_tooltips)
    {
        cx.global_mut::<TooltipDismissals>()
            .is_holding_back_new_tooltips = false;
    }
}

/// Whether a tooltip shows, settled when GPUI first draws it (as it builds it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TooltipVisibility {
    NotDrawnYet,
    /// Shown until a dismissal comes after this count.
    ShownUntilDismissal(u64),
    /// Due during a press or a keystroke: never shown.
    HeldBack,
    /// The gallery's specimen, drawn in the page rather than floating: always shown.
    Specimen,
}

fn visibility_of_new_tooltip(cx: &App) -> TooltipVisibility {
    match cx.try_global::<TooltipDismissals>() {
        None => TooltipVisibility::ShownUntilDismissal(0),
        Some(dismissals) if dismissals.is_holding_back_new_tooltips => TooltipVisibility::HeldBack,
        Some(dismissals) => TooltipVisibility::ShownUntilDismissal(dismissals.count),
    }
}

#[derive(Debug, Clone)]
pub struct Tooltip {
    label: SharedString,
    visibility: TooltipVisibility,
    /// Moves the label from the pointer, where GPUI puts every tooltip, to an anchor's side.
    offset_from_pointer: Option<Point<Pixels>>,
}

impl Tooltip {
    #[must_use]
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            visibility: TooltipVisibility::NotDrawnYet,
            offset_from_pointer: None,
        }
    }

    fn build(
        label: SharedString,
        offset_from_pointer: Option<Point<Pixels>>,
        cx: &mut App,
    ) -> gpui::Entity<Self> {
        cx.new(|_| Self {
            offset_from_pointer,
            ..Self::new(label)
        })
    }

    /// The component gallery's specimen: always drawn, whatever the dismissals and the focus.
    pub fn specimen_view(label: SharedString, cx: &mut App) -> AnyView {
        cx.new(|_| Self {
            visibility: TooltipVisibility::Specimen,
            ..Self::new(label)
        })
        .into()
    }

    /// For `StatefulInteractiveElement::tooltip`.
    pub fn view(label: SharedString, cx: &mut App) -> AnyView {
        Self::build(label, None, cx).into()
    }

    /// For `StatefulInteractiveElement::tooltip`: the label on the trailing side of `anchor`,
    /// centred on it, as the rail's (DESIGN.md §5.2, `ServerRail.svelte`). `anchor` is the item's
    /// slot, whose trailing edge the label overlaps by `space.2xs`, as on the web; without it
    /// (not drawn yet), the label follows the pointer like every other tooltip.
    pub fn view_on_trailing_side(
        label: SharedString,
        anchor: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyView {
        let offset = anchor.map(|anchor| {
            offset_to_trailing_side(anchor, window.mouse_position(), window.rem_size())
        });
        Self::build(label, offset, cx).into()
    }

    /// A label that arrives after the tooltip shows (who reacted, once the instance answers).
    pub fn set_label(&mut self, label: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.label = label.into();
        cx.notify();
    }

    /// The delay token, kept under reduced motion.
    #[must_use]
    pub fn show_delay() -> Duration {
        delay(COMPONENT_TOOLTIP_DELAY)
    }

    /// Hides tooltips on a press or a key anywhere in the window: drawn once, at its root.
    #[must_use]
    pub fn dismissal_layer() -> impl IntoElement {
        canvas(
            |_, _, _| {},
            |_, (), window, cx| {
                // The capture phase, before any element can stop the event.
                window.on_mouse_event(|_: &MouseDownEvent, phase, window, cx| {
                    if phase.capture() {
                        dismiss_tooltips(window, cx);
                    }
                });
                // Keys go down the focus path, which a layer beside it never sees: every
                // keystroke is intercepted instead, before any handler can consume it.
                let dismissals = cx.default_global::<TooltipDismissals>();
                if !dismissals.is_watching_keystrokes {
                    dismissals.is_watching_keystrokes = true;
                    cx.intercept_keystrokes(|_, window, cx| dismiss_tooltips(window, cx))
                        .detach();
                }
                window.on_mouse_event(|event: &MouseMoveEvent, phase, _, cx| {
                    if phase.capture() && event.pressed_button.is_none() {
                        stop_holding_back_new_tooltips(cx);
                    }
                });
                // A long press shows its tooltip at once, though its touch began with a press.
                window.on_mouse_event(|event: &LongPressEvent, phase, _, cx| {
                    if phase.capture() && event.phase == TouchPhase::Started {
                        stop_holding_back_new_tooltips(cx);
                    }
                });
            },
        )
    }

    fn is_shown(&mut self, window: &Window, cx: &mut App) -> bool {
        if self.visibility == TooltipVisibility::Specimen {
            return true;
        }
        if self.visibility == TooltipVisibility::NotDrawnYet {
            self.visibility = visibility_of_new_tooltip(cx);
        }
        let dismissal_count = cx
            .try_global::<TooltipDismissals>()
            .map_or(0, |dismissals| dismissals.count);
        let is_shown = window.is_window_active()
            && self.visibility == TooltipVisibility::ShownUntilDismissal(dismissal_count);
        if is_shown {
            cx.default_global::<TooltipDismissals>().is_a_tooltip_drawn = true;
        }
        is_shown
    }
}

/// Finds the drawn label in tests.
const TOOLTIP_LABEL_SELECTOR: &str = "tooltip-label";

/// From where GPUI draws a tooltip (one pixel past the pointer) to the trailing side of `anchor`.
fn offset_to_trailing_side(
    anchor: Bounds<Pixels>,
    pointer: Point<Pixels>,
    rem_size: Pixels,
) -> Point<Pixels> {
    let overlap = length(SPACE_2XS).to_pixels(rem_size);
    let gpui_origin = pointer + point(px(1.0), px(1.0));
    point(
        anchor.right() - overlap - gpui_origin.x,
        anchor.center().y - gpui_origin.y,
    )
}

impl Render for Tooltip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.is_shown(window, cx) {
            return div().into_any_element();
        }
        let tokens = theme(cx);
        let label = with_type(div(), TYPE_TOOLTIP)
            .max_w(length(COMPONENT_TOOLTIP_MAX_WIDTH))
            .px(length(COMPONENT_TOOLTIP_PADDING_INLINE))
            .py(length(COMPONENT_TOOLTIP_PADDING_BLOCK))
            .rounded(length(COMPONENT_TOOLTIP_RADIUS))
            .bg(color(tokens.component_tooltip_background))
            .border(length(1.0))
            .border_color(color(tokens.color_border_divider))
            .shadow(shadow(tokens.component_tooltip_shadow))
            .text_color(color(tokens.component_tooltip_text))
            .debug_selector(|| TOOLTIP_LABEL_SELECTOR.to_owned())
            .child(self.label.clone());
        match self.offset_from_pointer {
            None => label.into_any_element(),
            // A row of no height centres the label on the anchor's middle whatever its height;
            // the root keeps no size, so GPUI never flips it away from the window's edges.
            Some(offset) => div()
                .child(
                    div()
                        .absolute()
                        .left(offset.x)
                        .top(offset.y)
                        .w(length(COMPONENT_TOOLTIP_MAX_WIDTH))
                        .h(px(0.0))
                        .flex()
                        .items_center()
                        .child(label),
                )
                .into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{
        Bounds, Context, IntoElement, Modifiers, MouseButton, Pixels, Render,
        StatefulInteractiveElement as _, TestAppContext, VisualTestContext, Window,
        WindowAppearance, bounds, div, point, px, size,
    };

    use super::*;
    use crate::theme::ActiveTheme;

    /// A 100 × 40 trigger at the window's origin, its tooltip at the pointer or beside `anchor`.
    struct Harness {
        anchor: Option<Bounds<Pixels>>,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let anchor = self.anchor;
            div().size_full().child(Tooltip::dismissal_layer()).child(
                div()
                    .id("trigger")
                    .w(px(100.0))
                    .h(px(40.0))
                    .tooltip(move |window, cx| match anchor {
                        None => Tooltip::view("Paramètres utilisateur".into(), cx),
                        Some(anchor) => Tooltip::view_on_trailing_side(
                            "Atelier".into(),
                            Some(anchor),
                            window,
                            cx,
                        ),
                    })
                    .tooltip_show_delay(Tooltip::show_delay())
                    .on_click(|_, _, _| {}),
            )
        }
    }

    const ON_TRIGGER: (f32, f32) = (50.0, 20.0);

    fn open_with(
        anchor: Option<Bounds<Pixels>>,
        cx: &mut TestAppContext,
    ) -> &mut VisualTestContext {
        cx.update(|cx| cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark)));
        let (_, cx) = cx.add_window_view(|_, _| Harness { anchor });
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        cx
    }

    fn open(cx: &mut TestAppContext) -> &mut VisualTestContext {
        open_with(None, cx)
    }

    fn hover_trigger(cx: &mut VisualTestContext) {
        cx.simulate_mouse_move(
            point(px(ON_TRIGGER.0), px(ON_TRIGGER.1)),
            None,
            Modifiers::none(),
        );
    }

    fn wait_past_the_delay(cx: &mut VisualTestContext) {
        cx.executor()
            .advance_clock(Tooltip::show_delay() + Duration::from_millis(50));
        cx.run_until_parked();
    }

    /// Where the last drawn frame painted a tooltip's label, if it did.
    fn drawn_label(cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
        cx.run_until_parked();
        cx.debug_bounds(TOOLTIP_LABEL_SELECTOR)
    }

    #[gpui::test]
    fn a_hovered_trigger_shows_its_tooltip_after_the_delay(cx: &mut TestAppContext) {
        let cx = open(cx);
        hover_trigger(cx);
        wait_past_the_delay(cx);
        assert!(drawn_label(cx).is_some());
    }

    #[gpui::test]
    fn a_click_before_the_delay_ends_keeps_the_tooltip_hidden(cx: &mut TestAppContext) {
        let cx = open(cx);
        hover_trigger(cx);
        cx.simulate_click(point(px(ON_TRIGGER.0), px(ON_TRIGGER.1)), Modifiers::none());
        wait_past_the_delay(cx);
        assert_eq!(drawn_label(cx), None);
    }

    #[gpui::test]
    fn a_press_hides_a_shown_tooltip(cx: &mut TestAppContext) {
        let cx = open(cx);
        hover_trigger(cx);
        wait_past_the_delay(cx);
        cx.simulate_mouse_down(
            point(px(ON_TRIGGER.0), px(ON_TRIGGER.1)),
            MouseButton::Left,
            Modifiers::none(),
        );
        assert_eq!(drawn_label(cx), None);
    }

    #[gpui::test]
    fn a_key_hides_a_shown_tooltip(cx: &mut TestAppContext) {
        let cx = open(cx);
        hover_trigger(cx);
        wait_past_the_delay(cx);
        assert!(drawn_label(cx).is_some());
        // A plain key: nothing else in the window asks for a new frame.
        cx.simulate_keystrokes("a");
        assert_eq!(drawn_label(cx), None);
    }

    /// GPUI happens to redraw on each path above (input modality, its own press handler); a
    /// dismissal redraws by itself all the same.
    #[gpui::test]
    fn a_dismissal_alone_redraws_the_window_without_the_tooltip(cx: &mut TestAppContext) {
        let cx = open(cx);
        hover_trigger(cx);
        wait_past_the_delay(cx);
        assert!(drawn_label(cx).is_some());
        cx.update(dismiss_tooltips);
        assert_eq!(drawn_label(cx), None);
    }

    #[gpui::test]
    fn the_window_losing_focus_hides_a_shown_tooltip(cx: &mut TestAppContext) {
        let cx = open(cx);
        hover_trigger(cx);
        wait_past_the_delay(cx);
        cx.deactivate_window();
        assert_eq!(drawn_label(cx), None);
    }

    #[gpui::test]
    fn a_tooltip_shows_again_once_the_pointer_leaves_and_comes_back(cx: &mut TestAppContext) {
        let cx = open(cx);
        hover_trigger(cx);
        cx.simulate_click(point(px(ON_TRIGGER.0), px(ON_TRIGGER.1)), Modifiers::none());
        wait_past_the_delay(cx);
        cx.simulate_mouse_move(point(px(300.0), px(300.0)), None, Modifiers::none());
        hover_trigger(cx);
        wait_past_the_delay(cx);
        assert!(drawn_label(cx).is_some());
    }

    #[gpui::test]
    fn a_trailing_tooltip_is_drawn_beside_its_anchor_centred_on_it(cx: &mut TestAppContext) {
        let slot = bounds(point(px(8.0), px(100.0)), size(px(72.0), px(48.0)));
        let cx = open_with(Some(slot), cx);
        hover_trigger(cx);
        wait_past_the_delay(cx);
        let label = drawn_label(cx).expect("the tooltip is drawn");
        // 4 px (`space.2xs`) inside the slot's trailing edge, centred on its middle.
        assert_eq!(label.left(), px(76.0));
        assert!((label.center().y - slot.center().y).abs() < px(0.5));
    }

    #[gpui::test]
    fn the_gallery_specimen_stays_drawn_without_focus(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark)));
        let (_, cx) = cx.add_window_view(|_, cx| {
            struct Gallery(gpui::AnyView);
            impl Render for Gallery {
                fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                    div().size_full().child(self.0.clone())
                }
            }
            Gallery(Tooltip::specimen_view("A tooltip, as it floats".into(), cx))
        });
        cx.simulate_keystrokes("a");
        cx.deactivate_window();
        assert!(drawn_label(cx).is_some());
    }

    #[test]
    fn the_trailing_side_is_centred_on_the_anchor_and_overlaps_its_edge() {
        let slot = bounds(point(px(8.0), px(100.0)), size(px(72.0), px(48.0)));
        let pointer = point(px(40.0), px(110.0));
        let offset = offset_to_trailing_side(slot, pointer, px(16.0));
        // GPUI draws at the pointer plus one pixel; the label starts 4 px inside the slot's edge
        // (80 − 4 = 76) and is centred on its middle (124).
        assert_eq!(
            pointer + point(px(1.0), px(1.0)) + offset,
            point(px(76.0), px(124.0))
        );
    }
}
