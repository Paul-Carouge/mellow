//! The soundboard on screen (ADR-0032, `docs/design/web-soundboard.md`): the voice panel's popover
//! of the call's server sounds ([`SoundboardPopover`]) and a sound's picture ([`sound_emoji`]).
//! What plays and when is `crate::voice`'s; adding and editing sounds is Server settings →
//! Soundboard (`crate::settings::server`).

mod popover;

use gpui::{AnyElement, App, Hsla, IntoElement, ParentElement as _, Styled as _, Window, div};
use mellow_client_core::shared_logic::custom_emoji::{custom_emoji_of_reaction_key, display_emoji};
use mellow_client_core::state::ClientState;

pub use self::popover::{SoundboardPopover, SoundboardPopoverEvent};
use crate::components::icon;
use crate::emoji_picker::custom_emoji_picture;
use crate::theme::icons::IconName;
use crate::theme::length;

/// A sound's picture (`SoundEmoji.svelte`): its emoji (a reaction key), else the speaker glyph in
/// `glyph_ink`, so every sound has a shape to find it by; a deleted server emoji shows the
/// speaker too. Always decorative: the control around it says the sound's name.
pub fn sound_emoji(
    emoji_key: Option<&str>,
    size: f32,
    glyph_ink: Hsla,
    state: &ClientState,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let speaker = || icon(IconName::SpeakerHigh, size, glyph_ink).into_any_element();
    let Some(key) = emoji_key else {
        return speaker();
    };
    match custom_emoji_of_reaction_key(key) {
        Some(emoji_id) => {
            let is_known = state
                .guilds()
                .iter()
                .any(|guild| guild.emojis.iter().any(|emoji| emoji.id == emoji_id));
            if is_known {
                custom_emoji_picture(emoji_id, size, window, cx)
            } else {
                speaker()
            }
        }
        None => div()
            .flex_none()
            .text_size(length(size))
            .line_height(length(size))
            .child(display_emoji(key))
            .into_any_element(),
    }
}
