//! `Avatar.svelte` and `GroupAvatar.svelte`: a person's picture, or their initial on their own
//! tint while it loads, when they have none or when it fails. Decorative: the name always sits
//! next to it, so neither is in the accessibility tree.

use gpui::{
    App, FontWeight, Hsla, ImageSource, IntoElement, ObjectFit, ParentElement as _, RenderOnce,
    SharedString, Styled as _, StyledImage as _, Window, div, img, prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::format::{avatar_initial, avatar_tint_number};
use mellow_protocol::PublicUser;

use super::{ShownStatus, StatusIndicator, icon};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_GROUP_AVATAR_FACE_SCALE, COMPONENT_GROUP_AVATAR_RING_WIDTH,
    COMPONENT_MEMBER_ROW_OFFLINE_AVATAR_OPACITY, COMPONENT_STATUS_RING_WIDTH,
    COMPONENT_STATUS_SIZE_SMALL, ThemeTokens,
};
use crate::theme::{color, length, theme};

/// The initial is set at 40 % of the avatar, as on the web.
const INITIAL_SHARE_OF_AVATAR: f32 = 0.4;
/// `font-semibold` on the web's initial.
const INITIAL_FONT_WEIGHT: FontWeight = FontWeight::SEMIBOLD;

/// What an avatar needs to know about someone.
#[derive(Clone)]
pub struct AvatarPerson {
    pub initial: SharedString,
    /// 1 to 8: `color.avatar.tint-N`.
    pub tint_number: u32,
    /// The decoded picture; `None` shows the initial.
    pub picture: Option<ImageSource>,
}

impl AvatarPerson {
    #[must_use]
    pub fn from_user(user: &PublicUser, picture: Option<ImageSource>) -> Self {
        Self {
            initial: avatar_initial(user).into(),
            tint_number: avatar_tint_number(&user.id.to_string()),
            picture,
        }
    }
}

fn tint(tokens: &ThemeTokens, tint_number: u32) -> Hsla {
    color(match tint_number {
        1 => tokens.color_avatar_tint_1,
        2 => tokens.color_avatar_tint_2,
        3 => tokens.color_avatar_tint_3,
        4 => tokens.color_avatar_tint_4,
        5 => tokens.color_avatar_tint_5,
        6 => tokens.color_avatar_tint_6,
        7 => tokens.color_avatar_tint_7,
        _ => tokens.color_avatar_tint_8,
    })
}

#[derive(IntoElement)]
pub struct Avatar {
    person: AvatarPerson,
    size: f32,
    presence: Option<(ShownStatus, Hsla)>,
    is_dimmed: bool,
}

impl Avatar {
    /// `size` is the host's avatar token.
    #[must_use]
    pub fn new(person: AvatarPerson, size: f32) -> Self {
        Self {
            person,
            size,
            presence: None,
            is_dimmed: false,
        }
    }

    /// Shows the status shape, cut out of `ring`, the surface behind the avatar.
    #[must_use]
    pub fn presence(mut self, status: ShownStatus, ring: Hsla) -> Self {
        self.presence = Some((status, ring));
        self
    }

    /// Offline members are shown at half opacity.
    #[must_use]
    pub fn dimmed(mut self, is_dimmed: bool) -> Self {
        self.is_dimmed = is_dimmed;
        self
    }
}

fn initial_disc(person: &AvatarPerson, size: f32, tokens: &ThemeTokens) -> gpui::Div {
    div()
        .size_full()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(tint(tokens, person.tint_number))
        .text_color(color(tokens.color_avatar_initial))
        .text_size(length(size * INITIAL_SHARE_OF_AVATAR))
        .font_weight(INITIAL_FONT_WEIGHT)
        .child(person.initial.clone())
}

impl RenderOnce for Avatar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let ring_box = COMPONENT_STATUS_SIZE_SMALL + 2.0 * COMPONENT_STATUS_RING_WIDTH;
        let face = div()
            .relative()
            .size_full()
            .when(self.is_dimmed, |face| {
                face.opacity(COMPONENT_MEMBER_ROW_OFFLINE_AVATAR_OPACITY)
            })
            .child(initial_disc(&self.person, self.size, tokens))
            .when_some(self.person.picture.clone(), |face, picture| {
                // Over the initial, which stays visible while the picture loads or if it fails.
                face.child(
                    img(picture)
                        .absolute()
                        .inset_0()
                        .size_full()
                        .rounded_full()
                        .object_fit(ObjectFit::Cover),
                )
            });

        div()
            .relative()
            .flex_none()
            .size(length(self.size))
            .child(face)
            .when_some(self.presence, |avatar, (status, ring)| {
                avatar.child(
                    div()
                        .absolute()
                        .right(-length(COMPONENT_STATUS_RING_WIDTH))
                        .bottom(-length(COMPONENT_STATUS_RING_WIDTH))
                        .size(length(ring_box))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(ring)
                        .child(StatusIndicator::new(status, COMPONENT_STATUS_SIZE_SMALL)),
                )
            })
    }
}

/// A group's picture (DESIGN.md §5.6): the first two other people leaning on each other, the front
/// face cut out of the surface behind; one face when one is left; the users glyph when alone.
#[derive(IntoElement)]
pub struct GroupAvatar {
    people: Vec<AvatarPerson>,
    size: f32,
    ring: Hsla,
}

impl GroupAvatar {
    #[must_use]
    pub fn new(people: Vec<AvatarPerson>, size: f32, ring: Hsla) -> Self {
        Self { people, size, ring }
    }
}

impl RenderOnce for GroupAvatar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let face_size = self.size * COMPONENT_GROUP_AVATAR_FACE_SCALE;
        let mut people = self.people.into_iter();
        let container = div().relative().flex_none().size(length(self.size));
        match (people.next(), people.next()) {
            (Some(back), Some(front)) => container
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .child(Avatar::new(back, face_size)),
                )
                .child(
                    div()
                        .absolute()
                        .right(-length(COMPONENT_GROUP_AVATAR_RING_WIDTH))
                        .bottom(-length(COMPONENT_GROUP_AVATAR_RING_WIDTH))
                        .p(length(COMPONENT_GROUP_AVATAR_RING_WIDTH))
                        .rounded_full()
                        .bg(self.ring)
                        .child(Avatar::new(front, face_size)),
                ),
            (Some(only), None) => container.child(Avatar::new(only, self.size)),
            (None, _) => container.child(
                div()
                    .size_full()
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(color(tokens.component_group_avatar_empty_background))
                    .child(icon(
                        IconName::Users,
                        face_size,
                        color(tokens.component_group_avatar_empty_icon),
                    )),
            ),
        }
    }
}

debug_without_callbacks!(AvatarPerson, initial, tint_number);
debug_without_callbacks!(Avatar, size);
debug_without_callbacks!(GroupAvatar, size);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tokens::DARK_THEME;

    #[test]
    fn every_tint_number_maps_to_its_own_token() {
        let tints: Vec<Hsla> = (1..=8).map(|number| tint(&DARK_THEME, number)).collect();
        assert_eq!(tints[0], color(DARK_THEME.color_avatar_tint_1));
        assert_eq!(tints[7], color(DARK_THEME.color_avatar_tint_8));
        for (index, tint) in tints.iter().enumerate() {
            assert!(!tints[..index].contains(tint), "tint {} repeats", index + 1);
        }
    }
}
