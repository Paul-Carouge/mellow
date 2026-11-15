//! `Avatar.svelte` and `GroupAvatar.svelte`: a person's picture, or their initial on their own
//! tint while it loads, when they have none or when it fails. Decorative: the name always sits
//! next to it, so neither is in the accessibility tree.

use gpui::{
    App, FontWeight, Hsla, ImageSource, InteractiveElement as _, IntoElement, ObjectFit,
    ParentElement as _, RenderOnce, SharedString, Styled as _, StyledImage as _, Window, div, img,
    prelude::FluentBuilder as _,
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

/// `color.avatar.tint-N`: the person's own colour, behind their initial or across a profile's
/// banner strip.
#[must_use]
pub fn avatar_tint(tokens: &ThemeTokens, tint_number: u32) -> Hsla {
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
    status_size: f32,
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
            status_size: COMPONENT_STATUS_SIZE_SMALL,
            is_dimmed: false,
        }
    }

    /// Shows the status shape, cut out of `ring`, the surface behind the avatar.
    #[must_use]
    pub fn presence(mut self, status: ShownStatus, ring: Hsla) -> Self {
        self.presence = Some((status, ring));
        self
    }

    /// The status shape's size token, for avatars larger than a row's (the profile's).
    #[must_use]
    pub fn status_size(mut self, size: f32) -> Self {
        self.status_size = size;
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
        .debug_selector(|| "avatar-initial".to_owned())
        .size_full()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(avatar_tint(tokens, person.tint_number))
        .text_color(color(tokens.color_avatar_initial))
        .text_size(length(size * INITIAL_SHARE_OF_AVATAR))
        .font_weight(INITIAL_FONT_WEIGHT)
        .child(person.initial.clone())
}

/// Whether `picture` has something to draw this frame. Asking a store-backed picture also starts
/// its download and wakes this view once it lands, exactly as the `img` drawing it would.
fn is_picture_ready(picture: &ImageSource, window: &mut Window, cx: &mut App) -> bool {
    match picture {
        ImageSource::Custom(load) => matches!(load(window, cx), Some(Ok(_))),
        ImageSource::Render(_) => true,
        // Not used for avatars: GPUI's own asset loaders, drawn over the initial as before.
        ImageSource::Resource(_) | ImageSource::Image(_) => false,
    }
}

impl RenderOnce for Avatar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_picture_shown = self
            .person
            .picture
            .as_ref()
            .is_some_and(|picture| is_picture_ready(picture, window, cx));
        let tokens = theme(cx);
        let ring_box = self.status_size + 2.0 * COMPONENT_STATUS_RING_WIDTH;
        let face = div()
            .relative()
            .size_full()
            .when(self.is_dimmed, |face| {
                face.opacity(COMPONENT_MEMBER_ROW_OFFLINE_AVATAR_OPACITY)
            })
            // As on the web, the initial goes once the picture is drawn: a transparent picture
            // must not show it through.
            .when(!is_picture_shown, |face| {
                face.child(initial_disc(&self.person, self.size, tokens))
            })
            .when_some(self.person.picture.clone(), |face, picture| {
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
                        .child(StatusIndicator::new(status, self.status_size)),
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
    use std::sync::Arc;

    use gpui::{Context, ImageCacheError, Render, RenderImage, TestAppContext, WindowAppearance};

    use super::*;
    use crate::theme::ActiveTheme;
    use crate::theme::tokens::DARK_THEME;

    const AVATAR_SIZE: f32 = 40.0;

    struct AvatarHarness {
        picture: Option<ImageSource>,
    }

    impl Render for AvatarHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(Avatar::new(
                AvatarPerson {
                    initial: "D".into(),
                    tint_number: 1,
                    picture: self.picture.clone(),
                },
                AVATAR_SIZE,
            ))
        }
    }

    /// A fully transparent picture: anything drawn under it would show through.
    fn transparent_picture() -> Arc<RenderImage> {
        let pixels = image::RgbaImage::new(4, 4);
        Arc::new(RenderImage::new(vec![image::Frame::new(pixels)]))
    }

    fn is_initial_drawn(picture: Option<ImageSource>, cx: &mut TestAppContext) -> bool {
        cx.update(|cx| cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark)));
        let (_, cx) = cx.add_window_view(|_, _| AvatarHarness { picture });
        cx.run_until_parked();
        cx.debug_bounds("avatar-initial").is_some()
    }

    #[gpui::test]
    fn a_ready_picture_hides_the_initial_even_when_transparent(cx: &mut TestAppContext) {
        let picture = ImageSource::Render(transparent_picture());
        assert!(!is_initial_drawn(Some(picture), cx));
    }

    #[gpui::test]
    fn a_ready_picture_from_the_store_hides_the_initial(cx: &mut TestAppContext) {
        let ready = transparent_picture();
        let picture = ImageSource::Custom(Arc::new(move |_, _| Some(Ok(Arc::clone(&ready)))));
        assert!(!is_initial_drawn(Some(picture), cx));
    }

    #[gpui::test]
    fn the_initial_shows_while_the_picture_loads(cx: &mut TestAppContext) {
        let picture = ImageSource::Custom(Arc::new(|_, _| None));
        assert!(is_initial_drawn(Some(picture), cx));
    }

    #[gpui::test]
    fn the_initial_shows_when_the_picture_failed(cx: &mut TestAppContext) {
        let picture = ImageSource::Custom(Arc::new(|_, _| {
            Some(Err(ImageCacheError::Asset("picture unavailable".into())))
        }));
        assert!(is_initial_drawn(Some(picture), cx));
    }

    #[gpui::test]
    fn the_initial_shows_without_a_picture(cx: &mut TestAppContext) {
        assert!(is_initial_drawn(None, cx));
    }

    #[test]
    fn every_tint_number_maps_to_its_own_token() {
        let tints: Vec<Hsla> = (1..=8)
            .map(|number| avatar_tint(&DARK_THEME, number))
            .collect();
        assert_eq!(tints[0], color(DARK_THEME.color_avatar_tint_1));
        assert_eq!(tints[7], color(DARK_THEME.color_avatar_tint_8));
        for (index, tint) in tints.iter().enumerate() {
            assert!(!tints[..index].contains(tint), "tint {} repeats", index + 1);
        }
    }
}
