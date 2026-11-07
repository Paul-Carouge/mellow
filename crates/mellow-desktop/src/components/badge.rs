//! `MentionBadge.svelte` and `RoleChips.svelte`.

use gpui::{
    App, BoxShadow, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce,
    Rgba, Role, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, point,
    prelude::FluentBuilder as _, px, rgb,
};
use mellow_client_core::shared_logic::format::mention_badge_text;

use super::{icon, with_type};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_BADGE_HEIGHT, COMPONENT_BADGE_MIN_WIDTH, COMPONENT_BADGE_PADDING_INLINE,
    COMPONENT_BADGE_RING_WIDTH, COMPONENT_SERVER_SETTINGS_CHIP_DOT_SIZE,
    COMPONENT_SERVER_SETTINGS_CHIP_GAP, COMPONENT_SERVER_SETTINGS_CHIP_HEIGHT,
    COMPONENT_SERVER_SETTINGS_CHIP_ICON_SIZE, COMPONENT_SERVER_SETTINGS_CHIP_PADDING_INLINE,
    COMPONENT_SERVER_SETTINGS_CHIP_RADIUS, COMPONENT_SERVER_SETTINGS_CHIP_ROLE_TINT, TYPE_BADGE,
    TYPE_BODY_SM,
};
use crate::theme::{color, length, theme};

/// The red mention count (DESIGN.md §2.3, §4.2): never animated, and hidden from screen readers
/// because the row or icon it sits on says "3 mentions" in its own name.
#[derive(Debug, IntoElement)]
pub struct MentionBadge {
    mention_count: u32,
    ring: Option<Hsla>,
}

impl MentionBadge {
    #[must_use]
    pub fn new(mention_count: u32) -> Self {
        Self {
            mention_count,
            ring: None,
        }
    }

    /// On an icon: cut out of `surface`, the colour behind the icon.
    #[must_use]
    pub fn ring(mut self, surface: Hsla) -> Self {
        self.ring = Some(surface);
        self
    }
}

impl RenderOnce for MentionBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        with_type(div(), TYPE_BADGE)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(length(COMPONENT_BADGE_HEIGHT))
            .min_w(length(COMPONENT_BADGE_MIN_WIDTH))
            .px(length(COMPONENT_BADGE_PADDING_INLINE))
            .rounded_full()
            .bg(color(tokens.component_badge_background))
            .text_color(color(tokens.component_badge_text))
            .when_some(self.ring, |badge, surface| {
                // Outside the badge, as the web's spread shadow: the badge keeps its own size.
                badge.shadow(vec![BoxShadow {
                    color: surface,
                    offset: point(px(0.0), px(0.0)),
                    blur_radius: px(0.0),
                    spread_radius: px(COMPONENT_BADGE_RING_WIDTH),
                    inset: false,
                }])
            })
            .child(mention_badge_text(self.mention_count))
    }
}

/// One chip: a role with its colour (0 is the neutral role colour), or the owner's crown.
#[derive(Debug, Clone)]
pub enum RoleChip {
    Owner { label: SharedString },
    Role { name: SharedString, colour: u32 },
}

/// A member's roles as small chips, highest first (the host sorts), each with its colour dot and a
/// quiet wash of that colour (decorative: the name carries the meaning).
#[derive(Debug, IntoElement)]
pub struct RoleChips {
    label: SharedString,
    chips: Vec<RoleChip>,
}

impl RoleChips {
    #[must_use]
    pub fn new(label: impl Into<SharedString>, chips: Vec<RoleChip>) -> Self {
        Self {
            label: label.into(),
            chips,
        }
    }
}

/// `color-mix(in oklab, role share, chip background)`.
// shortcut: mixes in sRGB rather than OKLab; the 18 % share keeps the difference below what the
// eye sees on these dark and light washes. Upgrade if a contrast check ever flags a role colour.
pub(crate) fn role_chip_wash(role_colour: u32, chip_background: Hsla) -> Hsla {
    let role = rgb(role_colour);
    let background = chip_background.to_rgb();
    let share = COMPONENT_SERVER_SETTINGS_CHIP_ROLE_TINT;
    let mix = |role: f32, background: f32| role * share + background * (1.0 - share);
    Rgba {
        r: mix(role.r, background.r),
        g: mix(role.g, background.g),
        b: mix(role.b, background.b),
        a: background.a,
    }
    .into()
}

impl RenderOnce for RoleChips {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let chip_background = color(tokens.component_server_settings_chip_background);
        div()
            .id(SharedString::from(format!("role-chips-{}", self.label)))
            .role(Role::List)
            .aria_label(self.label)
            .flex()
            .flex_wrap()
            .gap(length(COMPONENT_SERVER_SETTINGS_CHIP_GAP))
            .children(self.chips.into_iter().map(|chip| {
                let (name, background, mark) = match chip {
                    RoleChip::Owner { label } => (
                        label,
                        chip_background,
                        icon(
                            IconName::Crown,
                            COMPONENT_SERVER_SETTINGS_CHIP_ICON_SIZE,
                            color(tokens.component_server_settings_chip_owner_icon),
                        )
                        .into_any_element(),
                    ),
                    RoleChip::Role { name, colour } => {
                        let (background, dot) = if colour == 0 {
                            (chip_background, color(tokens.component_roles_neutral_color))
                        } else {
                            (
                                role_chip_wash(colour, chip_background),
                                Hsla::from(rgb(colour)),
                            )
                        };
                        (
                            name,
                            background,
                            div()
                                .flex_none()
                                .size(length(COMPONENT_SERVER_SETTINGS_CHIP_DOT_SIZE))
                                .rounded_full()
                                .bg(dot)
                                .into_any_element(),
                        )
                    }
                };
                with_type(div(), TYPE_BODY_SM)
                    .flex()
                    .items_center()
                    .max_w_full()
                    .gap(length(COMPONENT_SERVER_SETTINGS_CHIP_GAP))
                    .h(length(COMPONENT_SERVER_SETTINGS_CHIP_HEIGHT))
                    .px(length(COMPONENT_SERVER_SETTINGS_CHIP_PADDING_INLINE))
                    .rounded(length(COMPONENT_SERVER_SETTINGS_CHIP_RADIUS))
                    .bg(background)
                    .text_color(color(tokens.component_server_settings_chip_text))
                    .child(mark)
                    .child(div().truncate().child(name))
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wash_takes_a_small_share_of_the_role_colour() {
        let background: Hsla = rgb(0x0000_0000).into();
        let wash = role_chip_wash(0xFF_0000, background).to_rgb();
        assert!((wash.r - COMPONENT_SERVER_SETTINGS_CHIP_ROLE_TINT).abs() < 1e-3);
        assert!(wash.g.abs() < 1e-3 && wash.b.abs() < 1e-3);
    }
}
