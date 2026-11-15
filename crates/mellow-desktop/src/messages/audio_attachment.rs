//! A sound attached to a message (`MessageAttachments.svelte`, DESIGN.md 5.4 "Sound player"): the
//! file card holding Mellow's own player (`sound_player`). Nothing is fetched before play, nothing
//! plays by itself, and a file the app cannot decode becomes the card with the web's "download
//! it" note and its download button. Videos reuse this card's frame and name row
//! (`video_attachment`).

use std::time::Duration;

use gpui::{
    AnyElement, App, Div, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, Role, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
};
use mellow_protocol::Attachment;

use super::attachments::file_size_label;
use crate::attachment_audio::{AudioPlayback, attachment_audio_view, keep_player_while_shown};
use crate::attachment_download::download_control;
use crate::components::{icon, with_type};
use crate::i18n;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_ATTACHMENT_FILE_BORDER_WIDTH, COMPONENT_ATTACHMENT_FILE_GAP,
    COMPONENT_ATTACHMENT_FILE_ICON_SIZE, COMPONENT_ATTACHMENT_FILE_MAX_WIDTH,
    COMPONENT_ATTACHMENT_FILE_PADDING, COMPONENT_ATTACHMENT_FILE_RADIUS, SPACE_2XS, TYPE_BODY_LG,
    TYPE_BODY_SM,
};
use crate::theme::{color, length, theme};

const SECONDS_PER_MINUTE: u64 = 60;
const SECONDS_PER_HOUR: u64 = 3_600;

/// "0:07", "12:34", "1:02:03": a player's clock.
#[must_use]
pub(crate) fn playback_time_label(time: Duration) -> String {
    let seconds = time.as_secs();
    let (hours, minutes, seconds) = (
        seconds / SECONDS_PER_HOUR,
        seconds % SECONDS_PER_HOUR / SECONDS_PER_MINUTE,
        seconds % SECONDS_PER_MINUTE,
    );
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// A media card's frame: the file card's border, ground and padding.
pub(crate) fn media_card(id: ElementId, label: String, cx: &App) -> gpui::Stateful<Div> {
    let tokens = theme(cx);
    div()
        .id(id)
        .role(Role::Group)
        .aria_label(SharedString::from(label))
        .flex()
        .flex_col()
        .gap(length(SPACE_2XS))
        .w_full()
        .max_w(length(COMPONENT_ATTACHMENT_FILE_MAX_WIDTH))
        .p(length(COMPONENT_ATTACHMENT_FILE_PADDING))
        .rounded(length(COMPONENT_ATTACHMENT_FILE_RADIUS))
        .border(length(COMPONENT_ATTACHMENT_FILE_BORDER_WIDTH))
        .border_color(color(tokens.component_attachment_file_border))
        .bg(color(tokens.component_attachment_file_background))
}

/// The card's first row: the file's glyph, name and size, then its trailing buttons.
pub(crate) fn name_row(
    attachment: &Attachment,
    glyph: IconName,
    trailing: Vec<AnyElement>,
    cx: &App,
) -> Div {
    let tokens = theme(cx);
    let strings = i18n::strings(cx);
    div()
        .flex()
        .items_center()
        .gap(length(COMPONENT_ATTACHMENT_FILE_GAP))
        .child(icon(
            glyph,
            COMPONENT_ATTACHMENT_FILE_ICON_SIZE,
            color(tokens.component_attachment_file_icon),
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    with_type(div(), TYPE_BODY_LG)
                        .truncate()
                        .text_color(color(tokens.component_attachment_file_name_text))
                        .child(attachment.file_name.clone()),
                )
                .child(
                    with_type(div(), TYPE_BODY_SM)
                        .text_color(color(tokens.component_attachment_file_size_text))
                        .child(file_size_label(attachment.size_bytes, strings)),
                ),
        )
        .children(trailing)
}

/// A sentence under the card ("download it instead", "no sound output").
pub(crate) fn playback_note(text: &'static str, cx: &App) -> Div {
    with_type(div(), TYPE_BODY_SM)
        .debug_selector(|| "audio-note".to_owned())
        .text_color(color(theme(cx).component_attachment_playback_note_text))
        .child(text)
}

#[derive(IntoElement)]
pub(crate) struct AudioAttachment {
    id: ElementId,
    attachment: Attachment,
}

impl AudioAttachment {
    pub(crate) fn new(id: ElementId, attachment: Attachment) -> Self {
        Self { id, attachment }
    }
}

impl RenderOnce for AudioAttachment {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        keep_player_while_shown(&self.id, self.attachment.id, window, cx);
        let view = attachment_audio_view(self.attachment.id, cx);
        if view.playback != AudioPlayback::CannotPlay {
            return super::sound_player::sound_player(&self.id, &self.attachment, view, window, cx);
        }
        // What the app cannot play becomes the download card, with the web's note.
        let messages = i18n::strings(cx).messages();
        let (download, saved) = download_control(&self.id, &self.attachment, cx);
        media_card(
            self.id.clone(),
            messages.audio_player(&self.attachment.file_name),
            cx,
        )
        .child(name_row(
            &self.attachment,
            IconName::FileAudio,
            vec![download.into_any_element()],
            cx,
        ))
        .child(playback_note(messages.cannot_play(), cx))
        .children(saved)
    }
}

impl std::fmt::Debug for AudioAttachment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AudioAttachment")
            .field("id", &self.id)
            .field("attachment", &self.attachment.id)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Context, Render, TestAppContext, WindowAppearance, px};

    use super::*;
    use crate::attachment_audio::{MAXIMUM_PLAYED_ATTACHMENT_BYTES, toggle_attachment_audio};
    use crate::theme::ActiveTheme;
    use crate::theme::tokens::SIZE_HIT_DESKTOP_MIN;

    struct Card(Attachment);

    impl Render for Card {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(COMPONENT_ATTACHMENT_FILE_MAX_WIDTH))
                .child(AudioAttachment::new("sound".into(), self.0.clone()))
        }
    }

    fn sound(size_bytes: u64) -> Attachment {
        Attachment {
            id: mellow_protocol::Snowflake::from_raw(43),
            file_name: "mémo-vocal.opus".to_owned(),
            content_type: "audio/ogg".to_owned(),
            size_bytes,
            width: None,
            height: None,
        }
    }

    #[gpui::test]
    fn a_sound_shows_its_player_until_it_cannot_play_here(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
            gpui_component::init(cx);
        });
        let too_large = sound(MAXIMUM_PLAYED_ATTACHMENT_BYTES + 1);
        let (_card, cx) = cx.add_window_view(|_, _| Card(too_large.clone()));
        cx.run_until_parked();
        let player = cx
            .debug_bounds("audio-player")
            .expect("a sound not played yet shows its player");
        assert!(player.size.height >= px(SIZE_HIT_DESKTOP_MIN));
        assert!(cx.debug_bounds("audio-note").is_none());

        // Refused before anything is fetched: the card says to download it instead.
        cx.update(|window, cx| {
            toggle_attachment_audio(&too_large, cx);
            window.refresh();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("audio-player").is_none());
        assert!(cx.debug_bounds("audio-note").is_some());
    }

    #[test]
    fn the_clock_reads_minutes_and_seconds_then_hours() {
        assert_eq!(playback_time_label(Duration::ZERO), "0:00");
        assert_eq!(playback_time_label(Duration::from_millis(7_900)), "0:07");
        assert_eq!(playback_time_label(Duration::from_secs(754)), "12:34");
        assert_eq!(playback_time_label(Duration::from_secs(3_723)), "1:02:03");
    }
}
