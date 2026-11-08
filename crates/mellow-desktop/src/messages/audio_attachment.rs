//! A sound attached to a message (`MessageAttachments.svelte`, `web-inline-media.md`): the file
//! card it was, with a player under its name row. The web draws the browser's own controls; here
//! they are Dusk's: play or pause, the time, a seek bar, mute and volume, all reachable with Tab
//! and named for screen readers. Nothing is fetched before play, nothing plays by itself, and a
//! file the app cannot decode becomes the card with the web's "download it" note.

use std::time::Duration;

use gpui::{
    App, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use mellow_protocol::Attachment;

use super::attachments::file_size_label;
use crate::attachment_audio::{
    AttachmentVolume, AudioPlayback, attachment_audio_view, attachment_volume,
    seek_attachment_audio, set_attachment_volume, toggle_attachment_audio,
};
use crate::components::{IconButton, RangeBounds, RangeField, child_id, icon, with_type};
use crate::i18n;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_ATTACHMENT_AUDIO_VOLUME_WIDTH, COMPONENT_ATTACHMENT_FILE_BORDER_WIDTH,
    COMPONENT_ATTACHMENT_FILE_GAP, COMPONENT_ATTACHMENT_FILE_ICON_SIZE,
    COMPONENT_ATTACHMENT_FILE_MAX_WIDTH, COMPONENT_ATTACHMENT_FILE_PADDING,
    COMPONENT_ATTACHMENT_FILE_RADIUS, SPACE_2XS, SPACE_XS, TYPE_BODY_LG, TYPE_BODY_SM,
};
use crate::theme::{color, length, theme};

/// The seek bar moves by tenths of a second; arrows by one step, Page Up and Down by a tenth.
const SEEK_STEP_SECONDS: f32 = 0.1;
/// The volume slider moves by hundredths.
const VOLUME_STEP: f32 = 0.01;
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
    #[allow(clippy::too_many_lines)] // One card: its name row, then the player or the note.
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let strings = i18n::strings(cx);
        let messages = strings.messages();
        let view = attachment_audio_view(self.attachment.id, cx);
        let volume = attachment_volume(cx);
        let name = self.attachment.file_name.clone();
        let id = self.id;

        let name_row = div()
            .flex()
            .items_center()
            .gap(length(COMPONENT_ATTACHMENT_FILE_GAP))
            .child(icon(
                IconName::FileAudio,
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
                            .child(name.clone()),
                    )
                    .child(
                        with_type(div(), TYPE_BODY_SM)
                            .text_color(color(tokens.component_attachment_file_size_text))
                            .child(file_size_label(self.attachment.size_bytes, strings)),
                    ),
            );
        let note = |text: &'static str| {
            with_type(div(), TYPE_BODY_SM)
                .debug_selector(|| "audio-note".to_owned())
                .text_color(color(tokens.component_attachment_playback_note_text))
                .child(text)
        };

        let player = (view.playback != AudioPlayback::CannotPlay).then(|| {
            let is_playing = view.playback == AudioPlayback::Playing;
            let is_loading = view.playback == AudioPlayback::Loading;
            let (toggle_icon, toggle_label) = if is_playing {
                (IconName::Pause, messages.pause_audio(&name))
            } else if is_loading {
                (IconName::Play, messages.loading_audio(&name))
            } else {
                (IconName::Play, messages.play_audio(&name))
            };
            let toggled = self.attachment.clone();
            let position_text = playback_time_label(view.position);
            let time_text = view.duration.map_or_else(
                || position_text.clone(),
                |duration| format!("{position_text} / {}", playback_time_label(duration)),
            );
            let seek_bar = view
                .duration
                .filter(|duration| !duration.is_zero())
                .map(|duration| {
                    let attachment_id = self.attachment.id;
                    let duration_text = playback_time_label(duration);
                    let value_messages = messages;
                    RangeField::new(
                        child_id(&id, "seek"),
                        messages.audio_position(&name),
                        view.position.as_secs_f32(),
                        RangeBounds {
                            minimum: 0.0,
                            maximum: duration.as_secs_f32(),
                            step: SEEK_STEP_SECONDS,
                        },
                        move |seconds| {
                            value_messages
                                .audio_position_value(
                                    &playback_time_label(Duration::from_secs_f32(seconds)),
                                    &duration_text,
                                )
                                .into()
                        },
                    )
                    .label_hidden()
                    .on_change(move |seconds, _, cx| {
                        seek_attachment_audio(attachment_id, Duration::from_secs_f32(seconds), cx);
                    })
                });
            let shown_volume = if volume.is_muted { 0.0 } else { volume.volume };
            div()
                .debug_selector(|| "audio-player".to_owned())
                .flex()
                .items_center()
                .gap(length(SPACE_XS))
                .child(
                    IconButton::new(child_id(&id, "toggle"), toggle_icon, toggle_label)
                        .disabled(is_loading)
                        .on_click(move |_, cx| toggle_attachment_audio(&toggled, cx)),
                )
                .child(
                    with_type(div(), TYPE_BODY_SM)
                        .flex_none()
                        .text_color(color(tokens.component_attachment_file_size_text))
                        .child(time_text),
                )
                .child(div().flex_1().min_w_0().children(seek_bar))
                .child(
                    IconButton::new(
                        child_id(&id, "mute"),
                        if volume.is_muted || volume.volume == 0.0 {
                            IconName::SpeakerX
                        } else {
                            IconName::SpeakerHigh
                        },
                        if volume.is_muted {
                            messages.unmute_audio()
                        } else {
                            messages.mute_audio()
                        },
                    )
                    .toggled(volume.is_muted)
                    .on_click(move |_, cx| {
                        set_attachment_volume(
                            AttachmentVolume {
                                is_muted: !volume.is_muted,
                                ..volume
                            },
                            cx,
                        );
                    }),
                )
                .child(
                    div()
                        .flex_none()
                        .w(length(COMPONENT_ATTACHMENT_AUDIO_VOLUME_WIDTH))
                        .child(
                            RangeField::new(
                                child_id(&id, "volume"),
                                messages.audio_volume(),
                                shown_volume,
                                RangeBounds {
                                    minimum: 0.0,
                                    maximum: 1.0,
                                    step: VOLUME_STEP,
                                },
                                move |volume| {
                                    #[allow(clippy::cast_possible_truncation)]
                                    // 0 to 100 after rounding.
                                    let percent = (volume * 100.0).round() as i64;
                                    strings.voice_settings().percent(percent).into()
                                },
                            )
                            .label_hidden()
                            .on_change(|volume, _, cx| {
                                set_attachment_volume(
                                    AttachmentVolume {
                                        volume,
                                        is_muted: false,
                                    },
                                    cx,
                                );
                            }),
                        ),
                )
        });

        div()
            .id(id.clone())
            .role(Role::Group)
            .aria_label(SharedString::from(messages.audio_player(&name)))
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
            .child(name_row)
            .children(player)
            .when(view.playback == AudioPlayback::CannotPlay, |card| {
                card.child(note(messages.cannot_play()))
            })
            .when(view.playback == AudioPlayback::NoOutput, |card| {
                card.child(note(messages.no_audio_output()))
            })
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
