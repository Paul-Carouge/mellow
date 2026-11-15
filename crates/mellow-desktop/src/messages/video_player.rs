//! A video's screen (6.76, `web-inline-media.md`): the picture on the media stage with its controls
//! laid over its bottom edge, as the web's browser controls are: play or pause, the time, a seek bar,
//! mute and volume, and in the viewer full screen, on a scrim that fades out upwards. They show while
//! the video is not playing, while the pointer moves over it and while one of them (or the screen)
//! holds keyboard focus, and go 2.5 s after the pointer last moved while it plays. Pressing the
//! picture plays or pauses it. Keys, as video sites have them: Space or K play and pause, J and L
//! go ten seconds back or forward, Left and Right five (on the focused screen), Up and Down move the
//! volume, M mutes and F goes full screen. The colours are the dark theme's in both themes: the
//! stage is dark in both (`color.media.*`). Shared by a message's video (`video_attachment`) and
//! the media viewer.

use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, ElementId, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    MouseMoveEvent, ParentElement as _, RenderOnce, Role, SharedString,
    StatefulInteractiveElement as _, StyleRefinement, Styled, Window, div, linear_color_stop,
    linear_gradient, prelude::FluentBuilder as _,
};
use mellow_protocol::Attachment;

use super::audio_attachment::playback_time_label;
use crate::attachment_audio::{
    AttachmentVolume, AudioPlayback, AudioView, attachment_audio_view, attachment_video_picture,
    attachment_volume, keep_player_while_shown, seek_attachment_audio, set_attachment_volume,
    stop_attachment_playback, toggle_attachment_audio,
};
use crate::components::{
    IconButton, RangeBounds, RangeField, child_id, icon, remembered_focus_handle, with_type,
};
use crate::i18n;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_VIDEO_PLAYER_CONTROL_ICON_SIZE, COMPONENT_VIDEO_PLAYER_CONTROL_ICON_SIZE_LARGE,
    COMPONENT_VIDEO_PLAYER_CONTROL_SIZE, COMPONENT_VIDEO_PLAYER_CONTROL_SIZE_LARGE,
    COMPONENT_VIDEO_PLAYER_CONTROLS_FADE_HEIGHT, COMPONENT_VIDEO_PLAYER_CONTROLS_GAP,
    COMPONENT_VIDEO_PLAYER_CONTROLS_HIDE_DELAY, COMPONENT_VIDEO_PLAYER_CONTROLS_PADDING,
    COMPONENT_VIDEO_PLAYER_PLAY_ICON_SIZE, COMPONENT_VIDEO_PLAYER_SEEK_JUMP,
    COMPONENT_VIDEO_PLAYER_SEEK_STEP, COMPONENT_VIDEO_PLAYER_VOLUME_WIDTH, DARK_THEME,
    SIZE_FOCUS_RING_WIDTH, TYPE_BODY_SM,
};
use crate::theme::{color, delay, length, theme};
use crate::voice::video_view::{VideoFit, video_view};

/// The seek bar moves by tenths of a second.
const SEEK_BAR_STEP_SECONDS: f32 = 0.1;
/// The volume slider moves by hundredths; Up and Down by twentieths, as video sites do.
const VOLUME_SLIDER_STEP: f32 = 0.01;
const VOLUME_KEY_STEP: f32 = 0.05;
const PERCENT: f32 = 100.0;

/// What a key does to a video.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum VideoKey {
    TogglePlay,
    Seek { is_forward: bool, by: Duration },
    ChangeVolume(f32),
    ToggleMute,
    Fullscreen,
}

/// The video key `key` stands for. Left, Right, Up and Down only act on the focused screen: in the
/// viewer they otherwise move between pictures. Space is the focused screen's own click.
#[must_use]
pub(crate) fn video_key(key: &str, is_screen_focused: bool) -> Option<VideoKey> {
    let jump = Duration::from_millis(COMPONENT_VIDEO_PLAYER_SEEK_JUMP.0);
    let step = Duration::from_millis(COMPONENT_VIDEO_PLAYER_SEEK_STEP.0);
    Some(match key {
        "k" => VideoKey::TogglePlay,
        "j" => VideoKey::Seek {
            is_forward: false,
            by: jump,
        },
        "l" => VideoKey::Seek {
            is_forward: true,
            by: jump,
        },
        "m" => VideoKey::ToggleMute,
        "f" => VideoKey::Fullscreen,
        "left" if is_screen_focused => VideoKey::Seek {
            is_forward: false,
            by: step,
        },
        "right" if is_screen_focused => VideoKey::Seek {
            is_forward: true,
            by: step,
        },
        "up" if is_screen_focused => VideoKey::ChangeVolume(VOLUME_KEY_STEP),
        "down" if is_screen_focused => VideoKey::ChangeVolume(-VOLUME_KEY_STEP),
        _ => return None,
    })
}

/// Where a seek lands: inside the video, its end when the length is known.
#[must_use]
pub(crate) fn seek_target(
    position: Duration,
    duration: Option<Duration>,
    is_forward: bool,
    by: Duration,
) -> Duration {
    if is_forward {
        let target = position.saturating_add(by);
        duration.map_or(target, |duration| target.min(duration))
    } else {
        position.saturating_sub(by)
    }
}

/// Runs `key` on `attachment`'s player; full screen is left to the host. True when it acted.
pub(crate) fn run_video_key(attachment: &Attachment, key: VideoKey, cx: &mut App) -> bool {
    let view = attachment_audio_view(attachment.id, cx);
    let volume = attachment_volume(cx);
    match key {
        VideoKey::TogglePlay => toggle_attachment_audio(attachment, cx),
        VideoKey::Seek { is_forward, by } => seek_attachment_audio(
            attachment.id,
            seek_target(view.position, view.duration, is_forward, by),
            cx,
        ),
        VideoKey::ChangeVolume(change) => set_attachment_volume(
            AttachmentVolume {
                volume: volume.volume + change,
                is_muted: false,
            },
            cx,
        ),
        VideoKey::ToggleMute => set_attachment_volume(
            AttachmentVolume {
                is_muted: !volume.is_muted,
                ..volume
            },
            cx,
        ),
        VideoKey::Fullscreen => return false,
    }
    true
}

/// Pauses `attachment` if it plays, and stops its download if it loads, or it would start once
/// fetched (the viewer moving to another picture, or closing).
pub(crate) fn pause_video(attachment: &Attachment, cx: &mut App) {
    match attachment_audio_view(attachment.id, cx).playback {
        AudioPlayback::Playing => toggle_attachment_audio(attachment, cx),
        AudioPlayback::Loading => stop_attachment_playback(attachment.id, cx),
        _ => {}
    }
}

/// Plays `attachment` unless it plays or loads already ("Enlarge" goes on playing, as the web).
pub(crate) fn play_video(attachment: &Attachment, cx: &mut App) {
    if matches!(
        attachment_audio_view(attachment.id, cx).playback,
        AudioPlayback::Idle | AudioPlayback::Paused | AudioPlayback::Ended
    ) {
        toggle_attachment_audio(attachment, cx);
    }
}

/// Where the pointer is over a screen, remembered between frames.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ScreenPointer {
    is_over_screen: bool,
    is_over_controls: bool,
    last_move: Option<Instant>,
}

/// Whether the controls show: always while the video does not play or one of them holds keyboard
/// focus; while it plays, until the pointer has rested (or left) for the hide delay.
#[must_use]
pub(crate) fn are_controls_shown(
    is_playing: bool,
    has_focus: bool,
    is_pointer_over: bool,
    last_pointer_move: Option<Instant>,
    now: Instant,
) -> bool {
    let hide_delay = delay(COMPONENT_VIDEO_PLAYER_CONTROLS_HIDE_DELAY);
    !is_playing
        || has_focus
        || (is_pointer_over
            && last_pointer_move
                .is_some_and(|moved| now.saturating_duration_since(moved) < hide_delay))
}

/// Whether focus holds the controls on screen: on the screen from the keyboard, or on a control
/// however it got there.
#[must_use]
pub(crate) fn has_controls_focus(
    is_focus_in_controls: bool,
    is_screen_focused: bool,
    was_last_input_keyboard: bool,
) -> bool {
    is_focus_in_controls || (was_last_input_keyboard && is_screen_focused)
}

type FullscreenHandler = Rc<dyn Fn(&mut Window, &mut App) + 'static>;

/// A video's picture with its controls over it.
#[derive(IntoElement)]
pub(crate) struct VideoScreen {
    id: ElementId,
    attachment: Attachment,
    is_large: bool,
    focus_handle: Option<FocusHandle>,
    on_fullscreen: Option<FullscreenHandler>,
    /// The full screen button, in the viewer: whether the window is full screen now.
    fullscreen_button: Option<bool>,
    /// Drawn over the screen while the video loads, in place of the resting play glyph and the
    /// controls.
    loading_cover: Option<AnyElement>,
    /// The host's size and shape.
    style: StyleRefinement,
}

impl Styled for VideoScreen {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl VideoScreen {
    pub(crate) fn new(id: ElementId, attachment: Attachment) -> Self {
        Self {
            id,
            attachment,
            is_large: false,
            focus_handle: None,
            on_fullscreen: None,
            fullscreen_button: None,
            loading_cover: None,
            style: StyleRefinement::default(),
        }
    }

    /// The viewer's size: 44 px controls.
    #[must_use]
    pub(crate) fn large(mut self) -> Self {
        self.is_large = true;
        self
    }

    /// The host's handle for the screen, to focus it.
    #[must_use]
    pub(crate) fn track_focus(mut self, focus_handle: &FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle.clone());
        self
    }

    /// What covers the screen while the video loads (a card's poster with its loading note).
    #[must_use]
    pub(crate) fn loading_cover(mut self, cover: impl IntoElement) -> Self {
        self.loading_cover = Some(cover.into_any_element());
        self
    }

    /// What F does; with `button`, a full screen button too, showing whether the window is.
    #[must_use]
    pub(crate) fn on_fullscreen(
        mut self,
        button: Option<bool>,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.fullscreen_button = button;
        self.on_fullscreen = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for VideoScreen {
    #[allow(clippy::too_many_lines)] // The screen, its listeners and what it shows.
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let attachment = self.attachment;
        // The viewer's screen keeps the player too: its row may leave the list meanwhile.
        keep_player_while_shown(&id, attachment.id, window, cx);
        let view = attachment_audio_view(attachment.id, cx);
        let picture = attachment_video_picture(attachment.id, cx);
        let is_playing = view.playback == AudioPlayback::Playing;
        let loading_cover = self
            .loading_cover
            .filter(|_| view.playback == AudioPlayback::Loading);
        let focus_handle = self
            .focus_handle
            .unwrap_or_else(|| remembered_focus_handle(&id, window, cx));
        let controls_focus = window
            .use_keyed_state(child_id(&id, "controls-focus"), cx, |_, cx| {
                cx.focus_handle()
            })
            .read(cx)
            .clone();
        let pointer = window.use_keyed_state(child_id(&id, "pointer"), cx, |_, _| {
            ScreenPointer::default()
        });
        let pointer_now = *pointer.read(cx);
        let now = cx.background_executor().now();
        let is_screen_ring_shown =
            focus_handle.is_focused(window) && window.last_input_was_keyboard();
        let has_keyboard_focus = has_controls_focus(
            controls_focus.contains_focused(window, cx),
            focus_handle.is_focused(window),
            window.last_input_was_keyboard(),
        );
        let is_shown = are_controls_shown(
            is_playing,
            has_keyboard_focus,
            pointer_now.is_over_screen || pointer_now.is_over_controls,
            pointer_now.last_move,
            now,
        );
        let name = attachment.file_name.clone();
        // Under a loading cover the controls would only crowd its words (a narrow card puts
        // them on top of each other) and play is disabled anyway: a click on the screen cancels.
        let controls = (is_shown && loading_cover.is_none()).then(|| {
            controls_bar(
                &id,
                &attachment,
                view,
                self.is_large,
                self.fullscreen_button.zip(self.on_fullscreen.clone()),
                &controls_focus,
                &pointer,
                cx,
            )
        });

        let (clicked, keyed, keyed_focus) =
            (attachment.clone(), attachment.clone(), focus_handle.clone());
        let on_fullscreen = self.on_fullscreen;
        let (hover_pointer, move_pointer, key_pointer) =
            (pointer.clone(), pointer.clone(), pointer);
        let mut screen = div();
        *screen.style() = self.style;
        screen
            .id(id.clone())
            .debug_selector(|| "video-screen".to_owned())
            .role(Role::Group)
            .aria_label(SharedString::from(
                i18n::strings(cx).messages().video_player(&name),
            ))
            .track_focus(&focus_handle)
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .bg(color(DARK_THEME.color_media_stage))
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                toggle_attachment_audio(&clicked, cx);
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.modified() {
                    return;
                }
                let Some(key) = video_key(&event.keystroke.key, keyed_focus.is_focused(window))
                else {
                    return;
                };
                cx.stop_propagation();
                remember_activity(&key_pointer, cx);
                if !run_video_key(&keyed, key, cx)
                    && let Some(on_fullscreen) = on_fullscreen.as_ref()
                {
                    on_fullscreen(window, cx);
                }
            })
            .on_hover(move |is_over, window, cx| {
                hover_pointer.update(cx, |pointer, _| pointer.is_over_screen = *is_over);
                window.refresh();
            })
            .on_mouse_move(move |_: &MouseMoveEvent, window, cx| {
                if remember_activity(&move_pointer, cx) {
                    window.refresh();
                }
            })
            .children(picture.as_ref().map(|picture| {
                div()
                    .absolute()
                    .size_full()
                    .child(video_view(picture, VideoFit::Contain))
            }))
            // Decorative: the play button in the controls is the one screen readers and Tab reach.
            .when(!is_playing && loading_cover.is_none(), |screen| {
                screen.child(icon(
                    IconName::Play,
                    COMPONENT_VIDEO_PLAYER_PLAY_ICON_SIZE,
                    color(DARK_THEME.color_media_text),
                ))
            })
            .children(loading_cover)
            .children(controls)
            .when(is_screen_ring_shown, |screen| {
                // Inside the edge: the screen clips what passes its corners.
                screen.child(
                    div()
                        .absolute()
                        .inset_0()
                        .border(length(SIZE_FOCUS_RING_WIDTH))
                        .border_color(color(DARK_THEME.color_media_focus)),
                )
            })
    }
}

/// Notes the pointer or a key moving now; true when the controls were hidden by the delay, so
/// the window must be drawn again to bring them back.
fn remember_activity(pointer: &gpui::Entity<ScreenPointer>, cx: &mut App) -> bool {
    let now = cx.background_executor().now();
    let hide_delay = delay(COMPONENT_VIDEO_PLAYER_CONTROLS_HIDE_DELAY);
    pointer.update(cx, |pointer, _| {
        let was_hidden = pointer
            .last_move
            .is_none_or(|moved| now.saturating_duration_since(moved) >= hide_delay);
        pointer.last_move = Some(now);
        was_hidden
    })
}

/// The row of controls over the picture's bottom edge, on its scrim.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)] // One row of six controls.
fn controls_bar(
    id: &ElementId,
    attachment: &Attachment,
    view: AudioView,
    is_large: bool,
    fullscreen: Option<(bool, FullscreenHandler)>,
    controls_focus: &FocusHandle,
    pointer: &gpui::Entity<ScreenPointer>,
    cx: &App,
) -> AnyElement {
    let tokens = theme(cx);
    let strings = i18n::strings(cx);
    let messages = strings.messages();
    let volume = attachment_volume(cx);
    let name = &attachment.file_name;
    let (button_size, icon_size) = if is_large {
        (
            COMPONENT_VIDEO_PLAYER_CONTROL_SIZE_LARGE,
            COMPONENT_VIDEO_PLAYER_CONTROL_ICON_SIZE_LARGE,
        )
    } else {
        (
            COMPONENT_VIDEO_PLAYER_CONTROL_SIZE,
            COMPONENT_VIDEO_PLAYER_CONTROL_ICON_SIZE,
        )
    };
    let button = |name: &'static str, glyph: IconName, label: String| {
        IconButton::new(child_id(id, name), glyph, label)
            .sizes(button_size, icon_size)
            .on_media_stage()
    };
    let is_playing = view.playback == AudioPlayback::Playing;
    let is_loading = view.playback == AudioPlayback::Loading;
    let (toggle_icon, toggle_label) = if is_playing {
        (IconName::Pause, messages.pause_audio(name))
    } else if is_loading {
        (IconName::Play, messages.loading_audio(name))
    } else {
        (IconName::Play, messages.play_audio(name))
    };
    let toggled = attachment.clone();
    let position_text = playback_time_label(view.position);
    let time_text = view.duration.map_or_else(
        || position_text.clone(),
        |duration| format!("{position_text} / {}", playback_time_label(duration)),
    );
    let seek_bar = view
        .duration
        .filter(|duration| !duration.is_zero())
        .map(|duration| {
            let attachment_id = attachment.id;
            let duration_text = playback_time_label(duration);
            RangeField::new(
                child_id(id, "seek"),
                messages.audio_position(name),
                view.position.as_secs_f32(),
                RangeBounds {
                    minimum: 0.0,
                    maximum: duration.as_secs_f32(),
                    step: SEEK_BAR_STEP_SECONDS,
                },
                move |seconds| {
                    messages
                        .audio_position_value(
                            &playback_time_label(Duration::from_secs_f32(seconds)),
                            &duration_text,
                        )
                        .into()
                },
            )
            .label_hidden()
            .on_media_stage()
            .on_change(move |seconds, _, cx| {
                seek_attachment_audio(attachment_id, Duration::from_secs_f32(seconds), cx);
            })
        });
    let shown_volume = if volume.is_muted { 0.0 } else { volume.volume };
    let fullscreen_button = fullscreen.map(|(is_fullscreen, handler)| {
        let screen_share = strings.screen_share();
        let (glyph, label) = if is_fullscreen {
            (IconName::CornersIn, screen_share.exit_fullscreen())
        } else {
            (IconName::CornersOut, screen_share.fullscreen())
        };
        button("fullscreen", glyph, label.to_owned())
            .on_click(move |window, cx| handler(window, cx))
    });
    let (hover_pointer, move_pointer) = (pointer.clone(), pointer.clone());
    div()
        .id(child_id(id, "controls"))
        .debug_selector(|| "video-controls".to_owned())
        .track_focus(controls_focus)
        // Its clicks stay on the controls: the picture under them plays and pauses.
        .occlude()
        .on_hover(move |is_over, _, cx| {
            hover_pointer.update(cx, |pointer, _| pointer.is_over_controls = *is_over);
        })
        .on_mouse_move(move |_: &MouseMoveEvent, window, cx| {
            if remember_activity(&move_pointer, cx) {
                window.refresh();
            }
        })
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .flex()
        .items_center()
        .gap(length(COMPONENT_VIDEO_PLAYER_CONTROLS_GAP))
        .px(length(COMPONENT_VIDEO_PLAYER_CONTROLS_PADDING))
        .pb(length(COMPONENT_VIDEO_PLAYER_CONTROLS_PADDING))
        .pt(length(COMPONENT_VIDEO_PLAYER_CONTROLS_FADE_HEIGHT))
        .bg(linear_gradient(
            0.0,
            linear_color_stop(
                color(tokens.component_video_player_controls_scrim_start),
                0.0,
            ),
            linear_color_stop(color(tokens.component_video_player_controls_scrim_end), 1.0),
        ))
        .cursor_default()
        .child(
            button("toggle", toggle_icon, toggle_label)
                .disabled(is_loading)
                .on_click(move |_, cx| toggle_attachment_audio(&toggled, cx)),
        )
        .child(
            with_type(div(), TYPE_BODY_SM)
                .flex_none()
                .text_color(color(tokens.component_video_player_time_text))
                .child(time_text),
        )
        .child(div().flex_1().min_w_0().children(seek_bar))
        .child(
            button(
                "mute",
                if volume.is_muted || volume.volume == 0.0 {
                    IconName::SpeakerX
                } else {
                    IconName::SpeakerHigh
                },
                if volume.is_muted {
                    messages.unmute_audio().to_owned()
                } else {
                    messages.mute_audio().to_owned()
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
                .w(length(COMPONENT_VIDEO_PLAYER_VOLUME_WIDTH))
                .child(
                    RangeField::new(
                        child_id(id, "volume"),
                        messages.audio_volume(),
                        shown_volume,
                        RangeBounds {
                            minimum: 0.0,
                            maximum: 1.0,
                            step: VOLUME_SLIDER_STEP,
                        },
                        move |volume| {
                            #[allow(clippy::cast_possible_truncation)]
                            // 0 to 100 after rounding.
                            let percent = (volume * PERCENT).round() as i64;
                            strings.voice_settings().percent(percent).into()
                        },
                    )
                    .label_hidden()
                    .on_media_stage()
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
        .children(fullscreen_button)
        .into_any_element()
}

impl std::fmt::Debug for VideoScreen {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VideoScreen")
            .field("id", &self.id)
            .field("attachment", &self.attachment.id)
            .field("is_large", &self.is_large)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_follow_video_sites_and_arrows_only_act_on_the_focused_screen() {
        assert_eq!(video_key("k", false), Some(VideoKey::TogglePlay));
        assert_eq!(video_key("m", false), Some(VideoKey::ToggleMute));
        assert_eq!(video_key("f", false), Some(VideoKey::Fullscreen));
        assert_eq!(
            video_key("j", false),
            Some(VideoKey::Seek {
                is_forward: false,
                by: Duration::from_secs(10)
            })
        );
        assert_eq!(
            video_key("l", true),
            Some(VideoKey::Seek {
                is_forward: true,
                by: Duration::from_secs(10)
            })
        );
        assert_eq!(video_key("right", false), None);
        assert_eq!(
            video_key("right", true),
            Some(VideoKey::Seek {
                is_forward: true,
                by: Duration::from_secs(5)
            })
        );
        assert_eq!(
            video_key("up", true),
            Some(VideoKey::ChangeVolume(VOLUME_KEY_STEP))
        );
        assert_eq!(video_key("down", false), None);
        // Space is the focused screen's click, never a second toggle.
        assert_eq!(video_key("space", true), None);
    }

    #[test]
    fn a_control_clicked_keeps_the_controls_until_focus_leaves_them() {
        // Hiding a focused control would drop focus to the window's root.
        assert!(has_controls_focus(true, false, false));
        assert!(has_controls_focus(false, true, true));
        assert!(!has_controls_focus(false, true, false));
        assert!(!has_controls_focus(false, false, true));
    }

    #[test]
    fn a_seek_stays_inside_the_video() {
        let second = Duration::from_secs(1);
        let ten = Duration::from_secs(10);
        assert_eq!(seek_target(3 * second, None, false, ten), Duration::ZERO);
        assert_eq!(
            seek_target(3 * second, Some(8 * second), true, ten),
            8 * second
        );
        assert_eq!(seek_target(3 * second, None, true, ten), 13 * second);
    }

    #[test]
    fn controls_hide_only_while_playing_untouched() {
        let start = Instant::now();
        let hide_delay = delay(COMPONENT_VIDEO_PLAYER_CONTROLS_HIDE_DELAY);
        let just_before = start + hide_delay.saturating_sub(Duration::from_millis(1));
        assert!(are_controls_shown(false, false, false, None, start));
        assert!(are_controls_shown(true, true, false, None, start));
        assert!(are_controls_shown(
            true,
            false,
            true,
            Some(start),
            just_before
        ));
        assert!(!are_controls_shown(
            true,
            false,
            true,
            Some(start),
            start + hide_delay
        ));
        // The pointer left: nothing to keep them for.
        assert!(!are_controls_shown(true, false, false, Some(start), start));
    }
}
