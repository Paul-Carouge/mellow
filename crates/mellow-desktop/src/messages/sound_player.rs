//! A sound attachment's player (DESIGN.md 5.4, "Sound player", the web's `AudioPlayer.svelte`):
//! the file card holding a play key and the sound's **waveform as the seek bar**, then the time
//! and size, the speed, mute and volume. The shape shows once the sound played
//! (`attachment_audio::read_sound_shape`); until then the line is flat. Nothing loads before play
//! and one sound plays at a time (ADR-0038).

use std::time::Instant;

use gpui::{
    AnyElement, App, Bounds, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _,
    Pixels, Role, SharedString, StatefulInteractiveElement as _, Styled as _, Window, canvas, div,
    prelude::FluentBuilder as _,
};
use gpui_base::motion::{Transition, transition};
use mellow_client_core::shared_logic::audio_player::{
    WaveformKey, next_playback_speed, placeholder_levels, playback_fraction, playback_speed_number,
    played_bar_count, seek_fraction_after_key,
};
use mellow_client_core::shared_logic::link_previews::{clock_duration_label, duration_parts};
use mellow_protocol::Attachment;

use super::attachments::file_size_label;
use super::audio_attachment::media_card;
use super::time::locale;
use crate::attachment_audio::{
    AttachmentVolume, AudioPlayback, AudioView, attachment_sound_shape, attachment_speed,
    attachment_volume, seek_attachment_audio_to_fraction, set_attachment_speed,
    set_attachment_volume, toggle_attachment_audio,
};
use crate::attachment_download::{download_control, save_attachment};
use crate::components::{
    IconButton, RangeBounds, RangeField, Tooltip, child_id, colour_transition, easing, focus_ring,
    icon, is_focus_visible, remembered_focus_handle, remembered_hover, with_type,
};
use crate::i18n::{self, Strings};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_ATTACHMENT_DOWNLOAD_BUTTON_SIZE, COMPONENT_ATTACHMENT_DOWNLOAD_ICON_SIZE,
    COMPONENT_ATTACHMENT_FILE_GAP, COMPONENT_ATTACHMENT_ROW_GAP,
    COMPONENT_ATTACHMENT_ROW_ICON_SIZE, COMPONENT_AUDIO_PLAYER_CONTROL_SIZE,
    COMPONENT_AUDIO_PLAYER_PLAY_BUTTON_ICON_SIZE, COMPONENT_AUDIO_PLAYER_PLAY_BUTTON_SIZE,
    COMPONENT_AUDIO_PLAYER_VOLUME_WIDTH, COMPONENT_AUDIO_PLAYER_WAVE_BAR_COUNT,
    COMPONENT_AUDIO_PLAYER_WAVE_BAR_GAP, COMPONENT_AUDIO_PLAYER_WAVE_BAR_MIN_HEIGHT,
    COMPONENT_AUDIO_PLAYER_WAVE_HEIGHT, COMPONENT_AUDIO_PLAYER_WAVE_PLACEHOLDER_LEVEL,
    FONT_WEIGHT_SEMIBOLD, MOTION_DELAY_LOADING_NOTICE, MOTION_DURATION_BASE, MOTION_EASING_ENTER,
    RADIUS_FULL, RADIUS_SM, RADIUS_XS, SPACE_2XS, TYPE_BODY_LG, TYPE_BODY_SM,
};
use crate::theme::{animation_duration, color, delay, length, theme};

/// The volume slider moves by hundredths.
const VOLUME_STEP: f32 = 0.01;

/// Bars across the waveform, from its token.
fn bar_count() -> usize {
    // A token number, a few dozen.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = COMPONENT_AUDIO_PLAYER_WAVE_BAR_COUNT as usize;
    count
}

/// Whether "Loading…" shows: only once loading takes a while, so a quick start does not flash it.
fn is_loading_notice_due(loading_since: Option<Instant>, now: Instant) -> bool {
    loading_since
        .is_some_and(|since| now.duration_since(since) >= delay(MOTION_DELAY_LOADING_NOTICE))
}

/// The bar's height in the waveform's box: its level of the box, never under the minimum.
fn bar_height(level: f32) -> f32 {
    (level.clamp(0.0, 1.0) * COMPONENT_AUDIO_PLAYER_WAVE_HEIGHT)
        .max(COMPONENT_AUDIO_PLAYER_WAVE_BAR_MIN_HEIGHT)
}

/// "5 seconds", or "0 seconds": a time as a screen reader says it.
fn spoken_time(seconds: f64, strings: Strings) -> String {
    // Whole seconds of a sound under a day.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let parts = duration_parts(seconds.max(0.0).floor() as u32);
    let spoken = strings.link_previews().spoken_duration(
        i64::from(parts.hours),
        i64::from(parts.minutes),
        i64::from(parts.seconds),
    );
    if spoken.is_empty() {
        strings.messages().zero_seconds().to_owned()
    } else {
        spoken
    }
}

fn fraction_at(position_x: Pixels, track: Bounds<Pixels>) -> f32 {
    let width = f32::from(track.size.width);
    if width <= 0.0 {
        0.0
    } else {
        (f32::from(position_x - track.origin.x) / width).clamp(0.0, 1.0)
    }
}

/// The card of a sound the app plays: name row, player row, time and controls, problems.
pub(super) fn sound_player(
    id: &ElementId,
    attachment: &Attachment,
    view: AudioView,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Stateful<gpui::Div> {
    let tokens = theme(cx);
    let strings = i18n::strings(cx);
    let messages = strings.messages();
    let (_, saved) = download_control(id, attachment, cx);
    let duration_seconds = view.duration.map(|duration| duration.as_secs_f64());
    let fraction = view.pending_start.map_or_else(
        || playback_fraction(view.position.as_secs_f64(), duration_seconds),
        f64::from,
    );
    let elapsed_seconds =
        duration_seconds.map_or(view.position.as_secs_f64(), |duration| fraction * duration);
    let is_loading_notice_shown = view.playback == AudioPlayback::Loading
        && is_loading_notice_due(view.loading_since, Instant::now());
    let time_text = if is_loading_notice_shown {
        Some(messages.sound_loading().to_owned())
    } else {
        duration_seconds.map(|duration| {
            // Whole seconds of a sound under a day.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let (elapsed, total) = (elapsed_seconds.floor() as u32, duration.round() as u32);
            format!(
                "{} / {}",
                clock_duration_label(elapsed),
                clock_duration_label(total)
            )
        })
    };
    let problem = (view.playback == AudioPlayback::Stopped).then(|| {
        with_type(div(), TYPE_BODY_SM)
            .flex()
            .items_center()
            .gap(length(SPACE_2XS))
            .pl(length(
                COMPONENT_AUDIO_PLAYER_PLAY_BUTTON_SIZE + COMPONENT_ATTACHMENT_FILE_GAP,
            ))
            .text_color(color(tokens.component_audio_player_error_text))
            .child(icon(
                IconName::WarningCircle,
                COMPONENT_ATTACHMENT_ROW_ICON_SIZE,
                color(tokens.component_audio_player_error_text),
            ))
            .child(messages.sound_stopped())
            .child(crate::components::assertive_live_region(
                child_id(id, "problem"),
                messages.sound_stopped().into(),
            ))
    });
    media_card(id.clone(), messages.audio_player(&attachment.file_name), cx)
        .gap(length(COMPONENT_ATTACHMENT_ROW_GAP))
        .child(name_row(id, attachment, window, cx))
        .child(
            div()
                .debug_selector(|| "audio-player".to_owned())
                .flex()
                .items_center()
                .gap(length(COMPONENT_ATTACHMENT_FILE_GAP))
                .child(play_key(id, attachment, view.playback, window, cx))
                .child(waveform(
                    id,
                    attachment,
                    view,
                    fraction,
                    elapsed_seconds,
                    window,
                    cx,
                )),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(length(COMPONENT_ATTACHMENT_FILE_GAP))
                .pl(length(
                    COMPONENT_AUDIO_PLAYER_PLAY_BUTTON_SIZE + COMPONENT_ATTACHMENT_FILE_GAP,
                ))
                .child(
                    with_type(div(), TYPE_BODY_SM)
                        .flex_1()
                        .flex()
                        .flex_wrap()
                        .gap_x(length(COMPONENT_ATTACHMENT_FILE_GAP))
                        .text_color(color(tokens.component_audio_player_time_text))
                        .children(time_text)
                        .child(file_size_label(attachment.size_bytes, strings)),
                )
                .child(controls(id, attachment, window, cx)),
        )
        .children(problem)
        .when(view.playback == AudioPlayback::NoOutput, |card| {
            card.child(super::audio_attachment::playback_note(
                messages.no_audio_output(),
                cx,
            ))
        })
        .children(saved)
}

/// The file's name as the download link (the card's first Tab stop), and the download key, which
/// repeats it for the pointer only.
fn name_row(
    id: &ElementId,
    attachment: &Attachment,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let strings = i18n::strings(cx);
    let link_id = child_id(id, "name");
    let focus = remembered_focus_handle(&link_id, window, cx);
    let hover = remembered_hover(&link_id, window, cx);
    let is_hovered = *hover.read(cx);
    let key_id = child_id(id, "download");
    let key_hover = remembered_hover(&key_id, window, cx);
    let is_key_hovered = *key_hover.read(cx);
    let name = SharedString::from(attachment.file_name.clone());
    let label = SharedString::from(strings.messages().download(&attachment.file_name));
    let (saved_by_link, saved_by_key) = (attachment.clone(), attachment.clone());
    let key_tooltip = label.clone();
    let name_tooltip = name.clone();
    div()
        .flex()
        .items_center()
        .gap(length(COMPONENT_ATTACHMENT_FILE_GAP))
        .child(
            with_type(div(), TYPE_BODY_LG)
                .id(link_id)
                .role(Role::Link)
                .aria_label(label)
                .track_focus(&focus)
                .relative()
                .flex_1()
                .min_w_0()
                .truncate()
                .rounded(length(RADIUS_XS))
                .cursor_pointer()
                .text_color(color(tokens.component_attachment_file_name_text))
                .when(is_hovered, gpui::Styled::underline)
                .tooltip(move |_, cx| Tooltip::view(name_tooltip.clone(), cx))
                .tooltip_show_delay(Tooltip::show_delay())
                .on_hover(move |is_hovered, _, cx| {
                    hover.update(cx, |remembered, cx| {
                        *remembered = *is_hovered;
                        cx.notify();
                    });
                })
                .on_click(move |_, _, cx| save_attachment(&saved_by_link, cx))
                .child(name)
                .when(is_focus_visible(&focus, window), |link| {
                    link.child(focus_ring(RADIUS_XS, cx))
                }),
        )
        .child(
            div()
                .id(key_id)
                .flex_none()
                .size(length(COMPONENT_ATTACHMENT_DOWNLOAD_BUTTON_SIZE))
                .flex()
                .items_center()
                .justify_center()
                .rounded(length(RADIUS_SM))
                .cursor_pointer()
                .when(is_key_hovered, |key| {
                    key.bg(color(tokens.color_state_hover))
                })
                .tooltip(move |_, cx| Tooltip::view(key_tooltip.clone(), cx))
                .tooltip_show_delay(Tooltip::show_delay())
                .on_hover(move |is_hovered, _, cx| {
                    key_hover.update(cx, |remembered, cx| {
                        *remembered = *is_hovered;
                        cx.notify();
                    });
                })
                .on_click(move |_, _, cx| save_attachment(&saved_by_key, cx))
                .child(icon(
                    IconName::DownloadSimple,
                    COMPONENT_ATTACHMENT_DOWNLOAD_ICON_SIZE,
                    color(if is_key_hovered {
                        tokens.component_attachment_download_icon_hover
                    } else {
                        tokens.component_attachment_download_icon
                    }),
                )),
        )
        .into_any_element()
}

/// The 44 px round key: apricot while the sound plays or loads after a press (one key at a time).
fn play_key(
    id: &ElementId,
    attachment: &Attachment,
    playback: AudioPlayback,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let messages = i18n::strings(cx).messages();
    let key_id = child_id(id, "toggle");
    let focus = remembered_focus_handle(&key_id, window, cx);
    let hover = remembered_hover(&key_id, window, cx);
    let is_hovered = *hover.read(cx);
    let is_lit = matches!(playback, AudioPlayback::Playing | AudioPlayback::Loading);
    let fill = colour_transition(
        child_id(&key_id, "fill"),
        color(match (is_lit, is_hovered) {
            (true, true) => tokens.component_audio_player_play_button_background_playing_hover,
            (true, false) => tokens.component_audio_player_play_button_background_playing,
            (false, true) => tokens.component_audio_player_play_button_background_hover,
            (false, false) => tokens.component_audio_player_play_button_background,
        }),
        window,
        cx,
    );
    let glyph = colour_transition(
        child_id(&key_id, "glyph"),
        color(if is_lit {
            tokens.component_audio_player_play_button_icon_playing
        } else {
            tokens.component_audio_player_play_button_icon
        }),
        window,
        cx,
    );
    let label = if is_lit {
        messages.pause_sound(&attachment.file_name)
    } else {
        messages.play_sound(&attachment.file_name)
    };
    let toggled = attachment.clone();
    div()
        .id(key_id)
        .role(Role::Button)
        .aria_label(label)
        .track_focus(&focus)
        .relative()
        .flex_none()
        .size(length(COMPONENT_AUDIO_PLAYER_PLAY_BUTTON_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .rounded(length(RADIUS_FULL))
        .bg(fill)
        .cursor_pointer()
        .on_hover(move |is_hovered, _, cx| {
            hover.update(cx, |remembered, cx| {
                *remembered = *is_hovered;
                cx.notify();
            });
        })
        .on_click(move |_, _, cx| toggle_attachment_audio(&toggled, cx))
        .child(icon(
            if is_lit {
                IconName::Pause
            } else {
                IconName::Play
            },
            COMPONENT_AUDIO_PLAYER_PLAY_BUTTON_ICON_SIZE,
            glyph,
        ))
        .when(is_focus_visible(&focus, window), |key| {
            key.child(focus_ring(RADIUS_FULL, cx))
        })
        .into_any_element()
}

/// The waveform, a slider over the whole sound: a press or a drag puts the playhead there; arrows
/// move 5 s, Page keys a tenth, Home and End to the ends, Space plays or pauses.
#[allow(clippy::too_many_lines)] // One slider's element tree, in the order it is drawn.
fn waveform(
    id: &ElementId,
    attachment: &Attachment,
    view: AudioView,
    fraction: f64,
    elapsed_seconds: f64,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let strings = i18n::strings(cx);
    let messages = strings.messages();
    let wave_id = child_id(id, "wave");
    let focus = remembered_focus_handle(&wave_id, window, cx);
    let track_bounds = window.use_keyed_state(child_id(&wave_id, "track"), cx, |_, _| {
        Bounds::<Pixels>::default()
    });
    let is_dragging = window.use_keyed_state(child_id(&wave_id, "drag"), cx, |_, _| false);
    let count = bar_count();
    let levels = attachment_sound_shape(attachment.id, cx)
        .and_then(|shape| shape.levels.clone())
        .filter(|levels| levels.len() == count);
    // The bars grow from the flat line to the shape once, when it arrives (at once under reduced
    // motion; a shape already known when the card appears does not animate).
    let growth = transition(
        child_id(&wave_id, "growth"),
        if levels.is_some() { 1.0_f32 } else { 0.0 },
        Transition::new(animation_duration(MOTION_DURATION_BASE, cx))
            .easing(easing(MOTION_EASING_ENTER)),
        window,
        cx,
    );
    let placeholder = placeholder_levels(count, COMPONENT_AUDIO_PLAYER_WAVE_PLACEHOLDER_LEVEL);
    let shown_levels: Vec<f32> = match levels {
        Some(levels) => levels
            .iter()
            .map(|level| {
                COMPONENT_AUDIO_PLAYER_WAVE_PLACEHOLDER_LEVEL
                    + (level - COMPONENT_AUDIO_PLAYER_WAVE_PLACEHOLDER_LEVEL) * growth
            })
            .collect(),
        None => placeholder,
    };
    let played_bars = played_bar_count(fraction, count);
    let is_playing = view.playback == AudioPlayback::Playing;
    let played_colour = color(if is_playing {
        tokens.component_audio_player_wave_played
    } else {
        tokens.component_audio_player_wave_played_paused
    });
    let unplayed_colour = color(tokens.component_audio_player_wave_unplayed);
    let duration_seconds = view.duration.map(|duration| duration.as_secs_f64());
    let value_text: SharedString = match duration_seconds {
        Some(duration) => messages
            .sound_position_value(
                &spoken_time(elapsed_seconds, strings),
                &spoken_time(duration, strings),
            )
            .into(),
        None => spoken_time(elapsed_seconds, strings).into(),
    };
    let attachment_id = attachment.id;
    let toggled = attachment.clone();
    let (down_track, paint_track) = (track_bounds.clone(), track_bounds);
    let (down_drag, paint_drag) = (is_dragging.clone(), is_dragging);
    let bars = shown_levels.into_iter().enumerate().map(|(index, level)| {
        div()
            .flex_1()
            .min_w_0()
            .h(length(bar_height(level)))
            .rounded(length(RADIUS_FULL))
            .bg(if index < played_bars {
                played_colour
            } else {
                unplayed_colour
            })
    });
    div()
        .id(wave_id)
        .role(Role::Slider)
        .aria_label(messages.sound_position(&attachment.file_name))
        .aria_value(value_text)
        .aria_numeric_value(fraction)
        .aria_min_numeric_value(0.0)
        .aria_max_numeric_value(1.0)
        .track_focus(&focus)
        .relative()
        .flex_1()
        .min_w_0()
        .h(length(COMPONENT_AUDIO_PLAYER_WAVE_HEIGHT))
        .rounded(length(RADIUS_XS))
        .cursor_pointer()
        .on_key_down(move |event: &KeyDownEvent, _, cx| {
            if event.keystroke.modifiers.modified() {
                return;
            }
            if event.keystroke.key == "space" {
                cx.stop_propagation();
                toggle_attachment_audio(&toggled, cx);
                return;
            }
            if let Some(key) = WaveformKey::from_key_name(&event.keystroke.key) {
                cx.stop_propagation();
                let next = seek_fraction_after_key(key, fraction, duration_seconds);
                // A share of a sound, 0 to 1.
                #[allow(clippy::cast_possible_truncation)]
                seek_attachment_audio_to_fraction(attachment_id, next as f32, cx);
            }
        })
        .on_mouse_down(
            MouseButton::Left,
            move |event: &MouseDownEvent, window, cx| {
                down_drag.update(cx, |dragging, _| *dragging = true);
                let fraction = fraction_at(event.position.x, *down_track.read(cx));
                seek_attachment_audio_to_fraction(attachment_id, fraction, cx);
                // The next paint registers the window-level drag listeners.
                window.refresh();
            },
        )
        .child(
            // Measures the waveform for the pointer and, while dragging, follows the pointer
            // across the whole window, as `RangeField` does.
            canvas(
                move |bounds, _, cx| paint_track.update(cx, |stored, _| *stored = bounds),
                move |track, (), window, cx| {
                    if !*paint_drag.read(cx) {
                        return;
                    }
                    let (move_drag, up_drag) = (paint_drag.clone(), paint_drag);
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                        if !phase.bubble() || !*move_drag.read(cx) {
                            return;
                        }
                        if event.pressed_button != Some(MouseButton::Left) {
                            move_drag.update(cx, |dragging, _| *dragging = false);
                            return;
                        }
                        let fraction = fraction_at(event.position.x, track);
                        seek_attachment_audio_to_fraction(attachment_id, fraction, cx);
                    });
                    window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                        if phase.bubble() {
                            up_drag.update(cx, |dragging, _| *dragging = false);
                        }
                    });
                },
            )
            .absolute()
            .inset_0(),
        )
        .child(
            div()
                .size_full()
                .flex()
                .items_center()
                .gap(length(COMPONENT_AUDIO_PLAYER_WAVE_BAR_GAP))
                .children(bars),
        )
        .when(is_focus_visible(&focus, window), |wave| {
            wave.child(focus_ring(RADIUS_XS, cx))
        })
        .into_any_element()
}

/// The speed key: each press goes to the next speed and back to 1×.
fn speed_key(
    id: &ElementId,
    attachment: &Attachment,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let strings = i18n::strings(cx);
    let speed = attachment_speed(attachment.id, cx);
    let speed_number = playback_speed_number(speed, locale(strings));
    let speed_label = SharedString::from(strings.messages().playback_speed(&speed_number));
    let speed_id = child_id(id, "speed");
    let speed_focus = remembered_focus_handle(&speed_id, window, cx);
    let speed_hover = remembered_hover(&speed_id, window, cx);
    let is_speed_hovered = *speed_hover.read(cx);
    let attachment_id = attachment.id;
    let speed_tooltip = speed_label.clone();
    with_type(div(), TYPE_BODY_SM)
        .id(speed_id)
        .role(Role::Button)
        .aria_label(speed_label)
        .track_focus(&speed_focus)
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .h(length(COMPONENT_AUDIO_PLAYER_CONTROL_SIZE))
        .min_w(length(COMPONENT_AUDIO_PLAYER_CONTROL_SIZE))
        .px(length(SPACE_2XS))
        .rounded(length(RADIUS_SM))
        .font_weight(FontWeight(f32::from(FONT_WEIGHT_SEMIBOLD)))
        .cursor_pointer()
        .text_color(color(if is_speed_hovered {
            tokens.component_audio_player_control_text_hover
        } else {
            tokens.component_audio_player_control_text
        }))
        .when(is_speed_hovered, |key| {
            key.bg(color(
                tokens.component_audio_player_control_background_hover,
            ))
        })
        .tooltip(move |_, cx| Tooltip::view(speed_tooltip.clone(), cx))
        .tooltip_show_delay(Tooltip::show_delay())
        .on_hover(move |is_hovered, _, cx| {
            speed_hover.update(cx, |remembered, cx| {
                *remembered = *is_hovered;
                cx.notify();
            });
        })
        .on_click(move |_, _, cx| {
            set_attachment_speed(attachment_id, next_playback_speed(speed), cx);
        })
        .child(format!("{speed_number}×"))
        .when(is_focus_visible(&speed_focus, window), |key| {
            key.child(focus_ring(RADIUS_SM, cx))
        })
        .into_any_element()
}

/// Speed, mute and volume, at the trailing end of the time row.
fn controls(
    id: &ElementId,
    attachment: &Attachment,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let strings = i18n::strings(cx);
    let messages = strings.messages();
    let volume = attachment_volume(cx);
    let shown_volume = if volume.is_muted { 0.0 } else { volume.volume };
    div()
        .flex()
        .items_center()
        .gap(length(SPACE_2XS))
        .child(speed_key(id, attachment, window, cx))
        .child(
            IconButton::new(
                child_id(id, "mute"),
                if volume.is_muted || volume.volume == 0.0 {
                    IconName::SpeakerX
                } else {
                    IconName::SpeakerHigh
                },
                if volume.is_muted {
                    messages.unmute_sound()
                } else {
                    messages.mute_sound()
                },
            )
            .sizes(
                COMPONENT_AUDIO_PLAYER_CONTROL_SIZE,
                COMPONENT_ATTACHMENT_ROW_ICON_SIZE,
            )
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
            // shortcut: the slider takes `component.slider.accent`, which `RangeField` draws for
            // every slider, not `audio-player.volume-accent` (the same apricot today).
            div()
                .flex_none()
                .w(length(COMPONENT_AUDIO_PLAYER_VOLUME_WIDTH))
                .child(
                    RangeField::new(
                        child_id(id, "volume"),
                        messages.sound_volume(),
                        shown_volume,
                        RangeBounds {
                            minimum: 0.0,
                            maximum: 1.0,
                            step: VOLUME_STEP,
                        },
                        move |volume| {
                            // 0 to 100 after rounding.
                            #[allow(clippy::cast_possible_truncation)]
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
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn a_bar_is_its_level_of_the_box_and_never_thinner_than_the_minimum() {
        assert!((bar_height(1.0) - COMPONENT_AUDIO_PLAYER_WAVE_HEIGHT).abs() < f32::EPSILON);
        assert!((bar_height(0.5) - COMPONENT_AUDIO_PLAYER_WAVE_HEIGHT / 2.0).abs() < f32::EPSILON);
        assert!(
            (bar_height(0.0) - COMPONENT_AUDIO_PLAYER_WAVE_BAR_MIN_HEIGHT).abs() < f32::EPSILON
        );
        assert!((bar_height(3.0) - COMPONENT_AUDIO_PLAYER_WAVE_HEIGHT).abs() < f32::EPSILON);
        assert_eq!(bar_count(), 48);
    }

    #[test]
    fn loading_shows_only_once_it_takes_a_while() {
        let start = Instant::now();
        assert!(!is_loading_notice_due(None, start));
        assert!(!is_loading_notice_due(
            Some(start),
            start + Duration::from_millis(339)
        ));
        assert!(is_loading_notice_due(
            Some(start),
            start + Duration::from_millis(340)
        ));
    }

    #[test]
    fn a_position_is_said_in_words_and_zero_has_its_own() {
        let english = Strings::new(crate::i18n::Language::English);
        assert_eq!(spoken_time(0.4, english), english.messages().zero_seconds());
        assert_eq!(spoken_time(65.9, english), "1 minute 5 seconds");
    }
}
