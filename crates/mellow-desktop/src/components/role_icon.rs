//! A role's icon (`RoleIcon.svelte`, ADR-0050, DESIGN.md §5.4 and §5.7): its picture, or its
//! Unicode emoji, in a square of `size`. Role icons play by themselves like custom emoji, so the
//! reader's "Images animées" choice and reduced motion decide (`chosen_profile_image_animation`).
//! Decorative: the text beside it, or its control's accessible name, names the role.

use gpui::{
    AnyElement, App, IntoElement as _, ObjectFit, ParentElement as _, Styled as _,
    StyledImage as _, Window, div, img,
};
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::custom_emoji::display_emoji;
use mellow_protocol::Role;

use crate::messages::picture_for_box;
use crate::pictures::chosen_profile_image_animation;
use crate::theme::length;

/// The role's picture, else its emoji, at `size`; `None` for a role without an icon, so nothing
/// takes its place.
pub fn role_icon(role: &Role, size: f32, window: &Window, cx: &App) -> Option<AnyElement> {
    if let Some(picture) = role.icon {
        return Some(
            img(picture_for_box(
                DownloadableFile::Icon(picture.id),
                size,
                size,
                chosen_profile_image_animation(&picture, true, cx),
                window,
            ))
            .flex_none()
            .size(length(size))
            .object_fit(ObjectFit::Contain)
            .into_any_element(),
        );
    }
    let emoji = role
        .unicode_emoji
        .as_deref()
        .filter(|emoji| !emoji.is_empty())?;
    Some(
        div()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .size(length(size))
            .text_size(length(size))
            .line_height(length(size))
            .child(display_emoji(emoji))
            .into_any_element(),
    )
}
