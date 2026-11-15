//! A person's hover card (`ProfileHoverCard.svelte`, `hover-card-placement.ts`, DESIGN.md §5.7.2):
//! a glance at who this is, after the pointer or keyboard focus rests on their name. It repeats
//! what the trigger's name says and what the modal gives in full, so it takes no focus and has no
//! accessible role; "View profile" is out of the Tab order (Enter on the trigger opens the modal).

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, Bounds, Context, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Point, Size, StatefulInteractiveElement as _,
    Styled as _, Window, anchored, canvas, deferred, div, point, prelude::FluentBuilder as _, px,
};
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::format::{display_name, format_medium_date};
use mellow_client_core::shared_logic::markdown::{BlockNode, InlineNode};
use mellow_client_core::shared_logic::profiles::UserProfileLoad;

use super::ProfileSurfaces;
use super::facts::profile_facts;
use crate::components::{Avatar, AvatarPerson, easing, role_icon, shadow, with_type};
use crate::home::{presence_name, shown_presence};
use crate::i18n::strings;
use crate::messages::{
    MessageBody, MessageMarkdown, local_day_of, locale, picture_animation, picture_for_box,
};
use crate::theme::tokens::{
    COMPONENT_PROFILE_HOVER_CARD_ABOUT_LINES, COMPONENT_PROFILE_HOVER_CARD_AVATAR_SIZE,
    COMPONENT_PROFILE_HOVER_CARD_BORDER_WIDTH, COMPONENT_PROFILE_HOVER_CARD_ENTER_OFFSET,
    COMPONENT_PROFILE_HOVER_CARD_GAP, COMPONENT_PROFILE_HOVER_CARD_MARGIN,
    COMPONENT_PROFILE_HOVER_CARD_OFFSET, COMPONENT_PROFILE_HOVER_CARD_PADDING,
    COMPONENT_PROFILE_HOVER_CARD_RADIUS, COMPONENT_PROFILE_HOVER_CARD_ROLE_LIMIT,
    COMPONENT_PROFILE_HOVER_CARD_STATUS_SIZE, COMPONENT_PROFILE_HOVER_CARD_WIDTH,
    COMPONENT_ROLE_ICON_GAP, COMPONENT_ROLE_ICON_MEMBER_SIZE, FONT_WEIGHT_SEMIBOLD,
    MOTION_DURATION_FAST, MOTION_EASING_ENTER, SPACE_2XS, SPACE_XS, TYPE_BODY_SM, TYPE_HEADING_MD,
};
use crate::theme::{animation_duration, color, length, theme};

/// Where a card goes beside its trigger: by a column, or under a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HoverCardPlacement {
    /// Rows in a column (the member list): on the side of the column with room, top-aligned.
    Beside,
    /// Names in the conversation (authors, mentions): under the name, or above it.
    Below,
}

/// The side of the trigger the card ended on: its entrance slides from there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HoverCardSide {
    Left,
    Right,
    Below,
    Above,
}

/// `hoverCardPosition`: the card's top left corner and side. `column` is the box a "beside" card
/// keeps clear of; without one the trigger itself is. A card that fits on neither side goes under
/// the trigger instead. Always inside `viewport` by `margin`.
#[must_use]
pub(crate) fn hover_card_position(
    placement: HoverCardPlacement,
    trigger: Bounds<Pixels>,
    column: Option<Bounds<Pixels>>,
    card: Size<Pixels>,
    viewport: Size<Pixels>,
    offset: Pixels,
    margin: Pixels,
) -> (Point<Pixels>, HoverCardSide) {
    let clamp = |value: Pixels, least: Pixels, most: Pixels| value.min(most).max(least);
    let below_or_above = || {
        let left = clamp(trigger.left(), margin, viewport.width - margin - card.width);
        let below = trigger.bottom() + offset;
        if below + card.height <= viewport.height - margin {
            return (point(left, below), HoverCardSide::Below);
        }
        let above = trigger.top() - offset - card.height;
        if above >= margin {
            return (point(left, above), HoverCardSide::Above);
        }
        let top = clamp(below, margin, viewport.height - margin - card.height);
        (point(left, top), HoverCardSide::Below)
    };
    if placement == HoverCardPlacement::Below {
        return below_or_above();
    }
    let edges = column.unwrap_or(trigger);
    let room_on_left = edges.left() - offset - margin;
    let room_on_right = viewport.width - edges.right() - offset - margin;
    let top = clamp(
        trigger.top(),
        margin,
        viewport.height - margin - card.height,
    );
    let fits_left = room_on_left >= card.width;
    let fits_right = room_on_right >= card.width;
    if fits_left && (!fits_right || room_on_left >= room_on_right) {
        return (
            point(edges.left() - offset - card.width, top),
            HoverCardSide::Left,
        );
    }
    if fits_right {
        return (point(edges.right() + offset, top), HoverCardSide::Right);
    }
    below_or_above()
}

/// About me in a glance: its links wait in the modal, so they are plain text here.
fn without_links(nodes: &[InlineNode]) -> Vec<InlineNode> {
    nodes
        .iter()
        .flat_map(|node| match node {
            InlineNode::Link { children, .. } => without_links(children),
            InlineNode::Strong(children) => vec![InlineNode::Strong(without_links(children))],
            InlineNode::Emphasis(children) => vec![InlineNode::Emphasis(without_links(children))],
            InlineNode::Underline(children) => vec![InlineNode::Underline(without_links(children))],
            InlineNode::Strikethrough(children) => {
                vec![InlineNode::Strikethrough(without_links(children))]
            }
            other => vec![other.clone()],
        })
        .collect()
}

fn glance_body(body: &MessageBody) -> MessageBody {
    MessageBody {
        blocks: body
            .blocks
            .iter()
            .map(|block| match block {
                BlockNode::Paragraph(children) => BlockNode::Paragraph(without_links(children)),
                other => other.clone(),
            })
            .collect(),
        is_jumbo: false,
        names_reader: false,
    }
}

impl ProfileSurfaces {
    #[allow(clippy::too_many_lines)] // One card, in the order it is drawn.
    pub(super) fn render_hover_card(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let trigger = self.hover.shown.clone()?;
        // Gone with its row (a member regrouped, a message deleted): the card goes too.
        let Some(anchor) = trigger.anchor.upgrade() else {
            cx.defer_in(window, |surfaces, _, cx| surfaces.close_hover_card(cx));
            return None;
        };
        let viewport = window.viewport_size();
        // Anything moving under it closes it; a resize does.
        match self.hover.window_size {
            Some(size) if size != viewport => {
                cx.defer_in(window, |surfaces, _, cx| surfaces.close_hover_card(cx));
                return None;
            }
            _ => self.hover.window_size = Some(viewport),
        }
        let trigger_bounds = (*anchor.read(cx))?;
        let column = trigger
            .column
            .as_ref()
            .and_then(gpui::WeakEntity::upgrade)
            .and_then(|column| *column.read(cx));
        let rem = window.rem_size();
        let width = length(COMPONENT_PROFILE_HOVER_CARD_WIDTH).to_pixels(rem);
        let measured = self.hover.measured_height.get();
        #[cfg(test)]
        {
            self.hover.is_drawn_measured = measured.is_some();
        }
        let (origin, side) = hover_card_position(
            trigger.placement,
            trigger_bounds,
            column,
            Size {
                width,
                height: measured.unwrap_or(px(0.0)),
            },
            viewport,
            length(COMPONENT_PROFILE_HOVER_CARD_OFFSET).to_pixels(rem),
            length(COMPONENT_PROFILE_HOVER_CARD_MARGIN).to_pixels(rem),
        );

        let tokens = theme(cx);
        let catalogue = strings(cx);
        let state = self.session.read(cx).state();
        let facts = profile_facts(&trigger.user, self.guild_id, state);
        let name = display_name(&facts.user).to_owned();
        let picture = facts.user.avatar.map(|image| {
            picture_for_box(
                DownloadableFile::ProfileImage(image.id),
                COMPONENT_PROFILE_HOVER_CARD_AVATAR_SIZE,
                COMPONENT_PROFILE_HOVER_CARD_AVATAR_SIZE,
                picture_animation(cx),
                window,
            )
        });
        let surface = color(tokens.component_profile_hover_card_background);
        let since = facts
            .server
            .as_ref()
            .map_or(facts.joined_mellow_at_unix_milliseconds, |server| {
                server.joined_at_unix_milliseconds
            });
        let since_label = catalogue
            .profile_card()
            .member_since_date(&format_medium_date(locale(catalogue), local_day_of(since)));
        let about = match self.hover.details.current().clone() {
            UserProfileLoad::Loaded { user_id, profile } if user_id == facts.user.id => self
                .about_body(&profile.about_me)
                .map(|body| std::rc::Rc::new(glance_body(&body))),
            _ => None,
        };
        let chips = facts.server.as_ref().and_then(|server| {
            server.role_chips(
                &name,
                // A count token: a small whole number.
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Some(COMPONENT_PROFILE_HOVER_CARD_ROLE_LIMIT as usize),
                catalogue,
            )
        });
        let role_icon_element = facts
            .role_icon
            .as_ref()
            .and_then(|role| role_icon(role, COMPONENT_ROLE_ICON_MEMBER_SIZE, window, cx));
        let entity = cx.entity();
        let view_entity = entity.clone();
        let user = trigger.user.clone();
        let opener = trigger.opener.clone();
        let measured_height = self.hover.measured_height.clone();
        let line_height = TYPE_BODY_SM.font_size * TYPE_BODY_SM.line_height;

        let card = div()
            .id("profile-hover-card")
            .debug_selector(|| "profile-hover-card".to_owned())
            .occlude()
            .relative()
            .w(length(COMPONENT_PROFILE_HOVER_CARD_WIDTH))
            .p(length(COMPONENT_PROFILE_HOVER_CARD_PADDING))
            .flex()
            .flex_col()
            .gap(length(COMPONENT_PROFILE_HOVER_CARD_GAP))
            .rounded(length(COMPONENT_PROFILE_HOVER_CARD_RADIUS))
            .border(length(COMPONENT_PROFILE_HOVER_CARD_BORDER_WIDTH))
            .border_color(color(tokens.component_profile_hover_card_border))
            .bg(surface)
            .shadow(shadow(tokens.component_profile_hover_card_shadow))
            // Not until it is measured: placed above its name, it must not flash below first.
            .when(measured.is_none(), |card| card.opacity(0.0))
            .on_hover(move |is_hovered, _, cx| {
                entity.update(cx, |surfaces, cx| {
                    if *is_hovered {
                        surfaces.enter_card();
                    } else {
                        surfaces.leave_card(cx);
                    }
                });
            })
            .child(
                canvas(
                    move |bounds, window, _| {
                        if measured_height.get() != Some(bounds.size.height) {
                            measured_height.set(Some(bounds.size.height));
                            // Placed again with its height, and About me may have grown it: a
                            // refresh during prepaint is dropped, a next frame is not.
                            window.request_animation_frame();
                        }
                    },
                    |_, (), _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(length(COMPONENT_PROFILE_HOVER_CARD_GAP))
                    .child(
                        Avatar::new(
                            AvatarPerson::from_user(&facts.user, picture),
                            COMPONENT_PROFILE_HOVER_CARD_AVATAR_SIZE,
                        )
                        .presence(shown_presence(facts.presence), surface)
                        .status_size(COMPONENT_PROFILE_HOVER_CARD_STATUS_SIZE),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                with_type(div(), TYPE_HEADING_MD)
                                    .min_w_0()
                                    .flex()
                                    .items_center()
                                    .gap(length(COMPONENT_ROLE_ICON_GAP))
                                    .text_color(color(
                                        tokens.component_profile_hover_card_name_text,
                                    ))
                                    .child(div().min_w_0().truncate().child(name))
                                    .children(role_icon_element),
                            )
                            .child(
                                with_type(div(), TYPE_BODY_SM)
                                    .text_color(color(
                                        tokens.component_profile_hover_card_presence_text,
                                    ))
                                    .child(presence_name(catalogue, facts.presence)),
                            )
                            .children(facts.custom_status.clone().map(|status| {
                                with_type(div(), TYPE_BODY_SM)
                                    .truncate()
                                    .text_color(color(
                                        tokens.component_profile_hover_card_status_text,
                                    ))
                                    .child(status)
                            })),
                    ),
            )
            .children(about.map(|body| {
                // Cut after its lines: GPUI clamps each text, the height clamps the whole.
                with_type(div(), TYPE_BODY_SM)
                    .max_h(length(
                        line_height * COMPONENT_PROFILE_HOVER_CARD_ABOUT_LINES,
                    ))
                    .overflow_hidden()
                    .text_color(color(tokens.component_profile_hover_card_about_text))
                    .child(MessageMarkdown::new(
                        ("profile-hover-card-about", trigger.user.id.raw()),
                        body,
                    ))
            }))
            .children(chips)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_x(length(SPACE_XS))
                    .gap_y(length(SPACE_2XS))
                    .pt(length(COMPONENT_PROFILE_HOVER_CARD_PADDING))
                    .border_t(length(COMPONENT_PROFILE_HOVER_CARD_BORDER_WIDTH))
                    .border_color(color(tokens.component_profile_hover_card_divider))
                    .child(
                        with_type(div(), TYPE_BODY_SM)
                            .text_color(color(tokens.component_profile_hover_card_footer_text))
                            .child(since_label),
                    )
                    .child(
                        with_type(div(), TYPE_BODY_SM)
                            .id("profile-hover-card-view")
                            .debug_selector(|| "profile-hover-card-view".to_owned())
                            .font_weight(gpui::FontWeight(f32::from(FONT_WEIGHT_SEMIBOLD)))
                            .cursor_pointer()
                            .text_color(color(tokens.component_profile_hover_card_action_text))
                            .hover(gpui::Styled::underline)
                            .on_click(move |_, window, cx| {
                                let user = user.clone();
                                let opener = opener.clone();
                                view_entity.update(cx, |surfaces, cx| {
                                    surfaces.open_modal(user, opener, window, cx);
                                });
                            })
                            .child(catalogue.profile_card().view_profile()),
                    ),
            );

        // Slides from the trigger's side as it fades in; at once under reduced motion.
        let enter_offset = COMPONENT_PROFILE_HOVER_CARD_ENTER_OFFSET;
        let (from_x, from_y) = match side {
            HoverCardSide::Left => (enter_offset, 0.0),
            HoverCardSide::Right => (-enter_offset, 0.0),
            HoverCardSide::Below => (0.0, -enter_offset),
            HoverCardSide::Above => (0.0, enter_offset),
        };
        let duration = animation_duration(MOTION_DURATION_FAST, cx);
        let card = if duration == Duration::ZERO || measured.is_none() {
            card.into_any_element()
        } else {
            card.with_animation(
                ("profile-hover-card-enter", trigger.user.id.raw()),
                Animation::new(duration)
                    .with_easing(|progress| easing(MOTION_EASING_ENTER).sample(progress)),
                move |card, progress| {
                    let rest = 1.0 - progress.min(1.0);
                    card.opacity(progress.min(1.0))
                        .left(length(from_x * rest))
                        .top(length(from_y * rest))
                },
            )
            .into_any_element()
        };
        Some(
            deferred(anchored().position(origin).child(card))
                .with_priority(1)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use gpui::{px, size};

    use super::*;

    fn bounds(left: f32, top: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(left), px(top)), size(px(width), px(height)))
    }

    const CARD: Size<Pixels> = Size {
        width: px(300.0),
        height: px(200.0),
    };
    const VIEWPORT: Size<Pixels> = Size {
        width: px(1280.0),
        height: px(800.0),
    };

    fn place(
        placement: HoverCardPlacement,
        trigger: Bounds<Pixels>,
        column: Option<Bounds<Pixels>>,
    ) -> (Point<Pixels>, HoverCardSide) {
        hover_card_position(placement, trigger, column, CARD, VIEWPORT, px(8.0), px(8.0))
    }

    #[test]
    fn a_member_row_s_card_sits_beside_the_sheet_on_the_side_with_room() {
        let sheet = bounds(1000.0, 60.0, 260.0, 700.0);
        let row = bounds(1008.0, 120.0, 244.0, 42.0);
        assert_eq!(
            place(HoverCardPlacement::Beside, row, Some(sheet)),
            (point(px(692.0), px(120.0)), HoverCardSide::Left)
        );
        // Near the bottom, it stays inside the window.
        let low_row = bounds(1008.0, 760.0, 244.0, 42.0);
        assert_eq!(
            place(HoverCardPlacement::Beside, low_row, Some(sheet)).0.y,
            px(592.0)
        );
    }

    #[test]
    fn a_name_s_card_goes_below_it_or_above_when_there_is_no_room() {
        let name = bounds(400.0, 100.0, 60.0, 24.0);
        assert_eq!(
            place(HoverCardPlacement::Below, name, None),
            (point(px(400.0), px(132.0)), HoverCardSide::Below)
        );
        let low_name = bounds(1200.0, 700.0, 60.0, 24.0);
        assert_eq!(
            place(HoverCardPlacement::Below, low_name, None),
            (point(px(972.0), px(492.0)), HoverCardSide::Above)
        );
    }

    #[test]
    fn about_me_links_are_plain_text_in_the_glance() {
        let nodes = vec![InlineNode::Strong(vec![InlineNode::Link {
            url: "https://example.com".to_owned(),
            children: vec![InlineNode::Text("site".to_owned())],
            is_masked: true,
        }])];
        assert_eq!(
            without_links(&nodes),
            vec![InlineNode::Strong(vec![InlineNode::Text(
                "site".to_owned()
            )])]
        );
    }
}
