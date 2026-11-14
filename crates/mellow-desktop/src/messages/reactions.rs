//! `MessageReactions.svelte` (DESIGN.md §5.4): reaction pills under a message, emoji and count,
//! pressed when the reader is among them; activating one asks the host to add or take back the
//! reader's reaction. After them, Add reaction opens the host's emoji picker.

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::sync::Arc;

use gpui::{
    AnyElement, AnyView, App, Bounds, ElementId, InteractiveElement as _, IntoElement, ObjectFit,
    ParentElement as _, Pixels, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, StyledImage as _, Toggled, Window, div, img, prelude::FluentBuilder as _,
};
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::custom_emoji::{ReactionAppearance, reaction_appearance};
use mellow_protocol::{CustomEmoji, ReactionSummary, Snowflake};

use super::{picture_animation, picture_for_box};
use crate::components::{
    Tooltip, bounds_recorder, child_id, colour_transition, focus_ring, icon, is_focus_visible,
    remembered_bounds, remembered_focus_handle, remembered_hover, with_type,
};
use crate::i18n::{self, Strings};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CUSTOM_EMOJI_MISSING_BORDER_WIDTH, COMPONENT_CUSTOM_EMOJI_MISSING_RADIUS,
    COMPONENT_REACTION_BORDER_WIDTH, COMPONENT_REACTION_EMOJI_SIZE, COMPONENT_REACTION_GAP,
    COMPONENT_REACTION_HEIGHT, COMPONENT_REACTION_MARGIN_TOP, COMPONENT_REACTION_PADDING_INLINE,
    COMPONENT_REACTION_RADIUS, COMPONENT_REACTION_ROW_GAP, FONT_WEIGHT_SEMIBOLD, TYPE_BODY_SM,
};
use crate::theme::{color, length, theme};

/// What a pill reports when it is activated: the reaction's stored key.
pub type ReactionToggleHandler = Arc<dyn Fn(&str, &mut Window, &mut App) + 'static>;
/// The tooltip a pill shows: who reacted, which the host may have to ask the instance for.
pub type ReactionTooltipBuilder = Arc<dyn Fn(&ReactionPill, &mut App) -> AnyView + 'static>;
/// Add reaction: the host opens its emoji picker beside the button, drawn at these bounds.
pub type AddReactionHandler = Arc<dyn Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static>;

/// The add button, and whether it shows (the row is hovered or holds focus, or its picker is
/// open); hidden, it keeps its room so the pills do not move.
#[derive(Clone)]
pub struct AddReactionButton {
    pub on_add: AddReactionHandler,
    pub is_shown: bool,
}

/// One reaction as a pill shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionPill {
    pub key: String,
    pub appearance: ReactionAppearance,
    pub count: u32,
    pub is_mine: bool,
}

/// The message's reactions, each with how its emoji shows.
#[must_use]
pub fn reaction_pills<S: BuildHasher>(
    reactions: &[ReactionSummary],
    custom_emojis: &HashMap<Snowflake, CustomEmoji, S>,
) -> Vec<ReactionPill> {
    reactions
        .iter()
        .map(|reaction| ReactionPill {
            key: reaction.emoji.clone(),
            appearance: reaction_appearance(&reaction.emoji, custom_emojis),
            count: reaction.count,
            is_mine: reaction.me,
        })
        .collect()
}

/// The emoji's name for people: itself, `:name:`, or "a deleted emoji".
fn appearance_label(appearance: &ReactionAppearance, strings: Strings) -> String {
    match appearance {
        ReactionAppearance::Unicode(emoji) => emoji.clone(),
        ReactionAppearance::Custom(emoji) => format!(":{}:", emoji.name),
        ReactionAppearance::Deleted => strings.reactions().deleted_emoji().to_owned(),
    }
}

#[derive(IntoElement)]
pub struct MessageReactions {
    id: ElementId,
    pills: Vec<ReactionPill>,
    /// Joining an existing reaction: the reader sees the message's history.
    can_join: bool,
    on_toggle: Option<ReactionToggleHandler>,
    tooltip: Option<ReactionTooltipBuilder>,
    add_button: Option<AddReactionButton>,
}

impl MessageReactions {
    #[must_use]
    pub fn new(id: impl Into<ElementId>, pills: Vec<ReactionPill>) -> Self {
        Self {
            id: id.into(),
            pills,
            can_join: true,
            on_toggle: None,
            tooltip: None,
            add_button: None,
        }
    }

    /// Add reaction after the pills, where the reader may start a new one.
    #[must_use]
    pub fn add_button(mut self, add_button: Option<AddReactionButton>) -> Self {
        self.add_button = add_button;
        self
    }

    #[must_use]
    pub fn can_join(mut self, can_join: bool) -> Self {
        self.can_join = can_join;
        self
    }

    #[must_use]
    pub fn on_toggle(mut self, handler: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Arc::new(handler));
        self
    }

    /// Names who reacted on hover and focus instead of the pill's label (`MessageReactions`'
    /// tooltip).
    #[must_use]
    pub fn tooltip(mut self, builder: Option<ReactionTooltipBuilder>) -> Self {
        self.tooltip = builder;
        self
    }
}

impl RenderOnce for MessageReactions {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let strings = i18n::strings(cx);
        let pills: Vec<_> = self
            .pills
            .into_iter()
            .map(|pill| {
                let id = ElementId::NamedChild(Arc::new(self.id.clone()), pill.key.clone().into());
                // A deleted emoji's reaction can still be joined and taken back (ADR-0028).
                let is_usable = pill.is_mine || self.can_join;
                render_pill(
                    id,
                    pill,
                    is_usable,
                    PillHandlers {
                        on_toggle: self.on_toggle.clone(),
                        tooltip: self.tooltip.clone(),
                    },
                    strings,
                    window,
                    cx,
                )
            })
            .collect();
        let add_button = self
            .add_button
            .map(|add_button| render_add_button(&self.id, add_button, strings, window, cx));
        div()
            .id(self.id)
            .role(Role::List)
            .aria_label(strings.reactions().reactions())
            .when(!pills.is_empty(), |list| {
                list.mt(length(COMPONENT_REACTION_MARGIN_TOP))
            })
            .flex()
            .flex_wrap()
            .items_center()
            .gap(length(COMPONENT_REACTION_ROW_GAP))
            .children(pills)
            .children(add_button)
    }
}

/// `data-add-reaction`: a pill-shaped smiley that opens the picker.
fn render_add_button(
    list_id: &ElementId,
    add_button: AddReactionButton,
    strings: Strings,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let id = child_id(list_id, "add");
    let bounds = remembered_bounds(&id, window, cx);
    let AddReactionButton { on_add, is_shown } = add_button;
    // Hidden, it leaves the Tab order too, as the web's `invisible` does.
    let focus_handle = remembered_focus_handle(&id, window, cx).tab_stop(is_shown);
    let hover = remembered_hover(&id, window, cx);
    let is_hovered = *hover.read(cx);
    let is_ring_shown = is_focus_visible(&focus_handle, window);
    let label: SharedString = strings.reactions().add_reaction().into();
    let tint = color(if is_hovered {
        tokens.component_reaction_add_icon_hover
    } else {
        tokens.component_reaction_add_icon
    });
    let border = color(if is_hovered {
        tokens.component_reaction_border_hover
    } else {
        tokens.component_reaction_border
    });
    let tooltip = label.clone();
    div()
        .id(id)
        // Hidden, it keeps its room but leaves the accessibility tree (the web's `invisible`).
        .when(is_shown, |button| {
            button.role(Role::Button).aria_label(label)
        })
        .track_focus(&focus_handle)
        .relative()
        .flex()
        .items_center()
        .h(length(COMPONENT_REACTION_HEIGHT))
        .px(length(COMPONENT_REACTION_PADDING_INLINE))
        .rounded(length(COMPONENT_REACTION_RADIUS))
        .border(length(COMPONENT_REACTION_BORDER_WIDTH))
        .border_color(border)
        .bg(color(tokens.component_reaction_background))
        .when(!is_shown, |button| button.opacity(0.0))
        .cursor_pointer()
        .tooltip(move |_, cx| Tooltip::view(tooltip.clone(), cx))
        .tooltip_show_delay(Tooltip::show_delay())
        .on_hover(move |is_hovered, _, cx| {
            hover.update(cx, |remembered, cx| {
                *remembered = *is_hovered;
                cx.notify();
            });
        })
        .on_click({
            let bounds = bounds.clone();
            move |_, window, cx| {
                if let Some(drawn) = bounds.read(cx).filter(|_| is_shown) {
                    on_add(drawn, window, cx);
                }
            }
        })
        .child(icon(IconName::Smiley, COMPONENT_REACTION_EMOJI_SIZE, tint))
        .child(bounds_recorder(bounds))
        .when(is_ring_shown, |button| {
            button.child(focus_ring(COMPONENT_REACTION_RADIUS, cx))
        })
        .into_any_element()
}

/// A reaction's emoji at the pill's size: the character, the server emoji's picture, or a dashed
/// outline for a deleted one.
fn reaction_emoji(
    appearance: &ReactionAppearance,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    match appearance {
        ReactionAppearance::Custom(emoji) => img(picture_for_box(
            DownloadableFile::Icon(emoji.id),
            COMPONENT_REACTION_EMOJI_SIZE,
            COMPONENT_REACTION_EMOJI_SIZE,
            picture_animation(cx),
            window,
        ))
        .flex_none()
        .size(length(COMPONENT_REACTION_EMOJI_SIZE))
        .object_fit(ObjectFit::Contain)
        .into_any_element(),
        ReactionAppearance::Unicode(emoji) => div()
            .text_size(length(COMPONENT_REACTION_EMOJI_SIZE))
            .line_height(length(COMPONENT_REACTION_EMOJI_SIZE))
            .font_weight(crate::emoji_picker::emoji_glyph_weight())
            .child(emoji.clone())
            .into_any_element(),
        // A dashed outline where the deleted emoji's picture was, as the web's `ReactionEmoji`.
        ReactionAppearance::Deleted => div()
            .flex_none()
            .size(length(COMPONENT_REACTION_EMOJI_SIZE))
            .rounded(length(COMPONENT_CUSTOM_EMOJI_MISSING_RADIUS))
            .border(length(COMPONENT_CUSTOM_EMOJI_MISSING_BORDER_WIDTH))
            .border_dashed()
            .border_color(color(tokens.component_custom_emoji_missing_border))
            .into_any_element(),
    }
}

/// What a pill's activation and tooltip ask of the host.
#[derive(Clone)]
struct PillHandlers {
    on_toggle: Option<ReactionToggleHandler>,
    tooltip: Option<ReactionTooltipBuilder>,
}

fn render_pill(
    id: ElementId,
    pill: ReactionPill,
    is_usable: bool,
    handlers: PillHandlers,
    strings: Strings,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let PillHandlers { on_toggle, tooltip } = handlers;
    let tokens = theme(cx);
    let focus_handle = remembered_focus_handle(&id, window, cx);
    let hover = remembered_hover(&id, window, cx);
    let is_hovered = *hover.read(cx) && is_usable && !pill.is_mine;
    let (background, border, text) = if pill.is_mine {
        (
            tokens.component_reaction_background_me,
            tokens.component_reaction_border_me,
            tokens.component_reaction_count_text_me,
        )
    } else if is_hovered {
        (
            tokens.component_reaction_background_hover,
            tokens.component_reaction_border_hover,
            tokens.component_reaction_count_text,
        )
    } else {
        (
            tokens.component_reaction_background,
            tokens.component_reaction_border,
            tokens.component_reaction_count_text,
        )
    };
    let background = colour_transition(child_id(&id, "background"), color(background), window, cx);
    let border = colour_transition(child_id(&id, "border"), color(border), window, cx);
    let label = appearance_label(&pill.appearance, strings);
    let count = i64::from(pill.count);
    let accessible_label: SharedString = strings.reactions().chip(&label, count).into();
    let emoji = reaction_emoji(&pill.appearance, window, cx);
    let tooltip_pill = pill.clone();
    let key = pill.key;
    let is_ring_shown = is_focus_visible(&focus_handle, window);
    with_type(div(), TYPE_BODY_SM)
        .id(id)
        .role(Role::Button)
        .aria_label(accessible_label.clone())
        .aria_toggled(if pill.is_mine {
            Toggled::True
        } else {
            Toggled::False
        })
        .track_focus(&focus_handle)
        .relative()
        .flex()
        .items_center()
        .gap(length(COMPONENT_REACTION_GAP))
        .h(length(COMPONENT_REACTION_HEIGHT))
        .px(length(COMPONENT_REACTION_PADDING_INLINE))
        .rounded(length(COMPONENT_REACTION_RADIUS))
        .border(length(COMPONENT_REACTION_BORDER_WIDTH))
        .border_color(border)
        .bg(background)
        .text_color(color(text))
        .font_weight(gpui::FontWeight(f32::from(FONT_WEIGHT_SEMIBOLD)))
        .when(is_usable, gpui::Styled::cursor_pointer)
        .tooltip(move |_, cx| match &tooltip {
            Some(tooltip) => tooltip(&tooltip_pill, cx),
            None => Tooltip::view(accessible_label.clone(), cx),
        })
        .tooltip_show_delay(Tooltip::show_delay())
        .on_hover(move |is_hovered, _, cx| {
            hover.update(cx, |remembered, cx| {
                *remembered = *is_hovered;
                cx.notify();
            });
        })
        .on_click(move |_, window, cx| {
            if let Some(on_toggle) = on_toggle.as_ref().filter(|_| is_usable) {
                on_toggle(&key, window, cx);
            }
        })
        .child(emoji)
        .child(pill.count.to_string())
        .when(is_ring_shown, |pill| {
            pill.child(focus_ring(COMPONENT_REACTION_RADIUS, cx))
        })
        .into_any_element()
}

impl std::fmt::Debug for AddReactionButton {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AddReactionButton")
            .field("is_shown", &self.is_shown)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for MessageReactions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MessageReactions")
            .field("id", &self.id)
            .field("pills", &self.pills.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn pills_keep_the_order_counts_and_the_readers_own_reactions() {
        let party = CustomEmoji {
            id: Snowflake::from_raw(500),
            name: "party".into(),
            is_animated: false,
        };
        let custom = HashMap::from([(party.id, party.clone())]);
        let reactions = [
            ReactionSummary {
                emoji: "👍".into(),
                count: 3,
                me: true,
            },
            ReactionSummary {
                emoji: ":500".into(),
                count: 1,
                me: false,
            },
            ReactionSummary {
                emoji: ":999".into(),
                count: 2,
                me: false,
            },
        ];
        let pills = reaction_pills(&reactions, &custom);
        assert_eq!(pills[0].count, 3);
        assert!(pills[0].is_mine);
        assert_eq!(pills[1].appearance, ReactionAppearance::Custom(party));
        assert_eq!(pills[2].appearance, ReactionAppearance::Deleted);
        let english = Strings::new(Language::English);
        assert_eq!(appearance_label(&pills[1].appearance, english), ":party:");
        assert_eq!(
            appearance_label(&pills[2].appearance, english),
            english.reactions().deleted_emoji()
        );
    }
}
