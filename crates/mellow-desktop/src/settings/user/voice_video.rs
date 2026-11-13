//! The Voice page's Video part (`VoiceSettings.svelte`): the camera choice (radio rows as for the
//! audio devices, applied live to a camera on), and the preview, mirrored as people expect to see
//! themselves, on the call view's stage. Outside a call's camera, the preview runs only after
//! "Test the camera", as the microphone test does; with the camera on in the call it shows that
//! camera. Leaving the page stops the test and lets the call's camera stop converting pictures.
//!
//! Pictures are read every [`PREVIEW_FRAME_PERIOD`] while a camera is shown; each one (at most
//! 640 × 360, converted by the camera's thread) becomes an image off the interface's thread, and
//! the previous image's atlas slot is freed at the next draw.

use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AppContext as _, Context, InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _,
    Render, RenderImage, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledImage as _, Task, Window, div, img, prelude::FluentBuilder as _,
};
use image::{Delay, Frame, RgbaImage};
use mellow_client_core::shared_logic::voice_settings::{DeviceChoice, DeviceChoices};
use mellow_media::DecodedVideoFrame;

use super::{DEVICE_REFRESH_PERIOD, device_group, helper_text, warning_line};
use crate::components::{Button, ButtonVariant, icon, polite_live_region, with_type};
use crate::i18n::strings;
use crate::settings::user::parts::section_label;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CALL_VIEW_STAGE_RADIUS, COMPONENT_SETTINGS_SECTION_GAP,
    COMPONENT_SETTINGS_SECTION_LABEL_GAP, COMPONENT_SETTINGS_VOICE_COLUMN_MIN_WIDTH,
    COMPONENT_VOICE_PANEL_SIGNAL_ICON_SIZE, SPACE_2XS, SPACE_LG, SPACE_MD, SPACE_XS, TYPE_BODY,
};
use crate::theme::{color, length, theme};
use crate::voice::{CameraPhase, Voice, current_settings, voice};

/// The preview's redraw rate at most: the camera's 30 frames per second.
const PREVIEW_FRAME_PERIOD: Duration = Duration::from_millis(33);
/// The stage's shape, as the call view's tiles.
const PREVIEW_ASPECT_RATIO: f32 = 16.0 / 9.0;

pub(super) struct VoiceVideoSection {
    pub(super) picture: Option<Arc<RenderImage>>,
    /// Images replaced since the last draw, whose atlas slots that draw frees.
    released: Vec<Arc<RenderImage>>,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl std::fmt::Debug for VoiceVideoSection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VoiceVideoSection")
            .field("has_picture", &self.picture.is_some())
            .finish_non_exhaustive()
    }
}

/// A BGRA picture as an image GPUI draws (its images are BGRA too).
fn render_image_of(picture: &DecodedVideoFrame) -> Option<Arc<RenderImage>> {
    let pixels = RgbaImage::from_raw(picture.width, picture.height, picture.pixels.to_vec())?;
    let frame = Frame::from_parts(pixels, 0, 0, Delay::from_numer_denom_ms(0, 1));
    Some(Arc::new(RenderImage::new(vec![frame])))
}

impl VoiceVideoSection {
    pub(super) fn new(cx: &mut Context<Self>) -> Self {
        let mut subscriptions = vec![cx.on_release(|_, cx| {
            if let Some(voice) = voice(cx) {
                voice.update(cx, |voice, cx| voice.set_camera_preview_shown(false, cx));
            }
        })];
        let mut tasks = Vec::new();
        if let Some(voice) = voice(cx) {
            voice.update(cx, |voice, cx| voice.set_camera_preview_shown(true, cx));
            subscriptions.push(cx.observe(&voice, |_, _, cx| cx.notify()));
            tasks.push(Self::refresh_cameras(cx));
            tasks.push(Self::follow_pictures(cx));
        }
        Self {
            picture: None,
            released: Vec::new(),
            _tasks: tasks,
            _subscriptions: subscriptions,
        }
    }

    /// Lists the cameras when the page opens, then every few seconds, so one plugged in shows.
    fn refresh_cameras(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |section, cx| {
            loop {
                let is_gone = section
                    .update(cx, |_, cx| {
                        if let Some(voice) = voice(cx) {
                            voice.update(cx, Voice::refresh_cameras);
                        }
                    })
                    .is_err();
                if is_gone {
                    return;
                }
                cx.background_executor().timer(DEVICE_REFRESH_PERIOD).await;
            }
        })
    }

    /// Takes the newest picture, makes it an image off the interface's thread, shows it.
    fn follow_pictures(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |section, cx| {
            loop {
                cx.background_executor().timer(PREVIEW_FRAME_PERIOD).await;
                let Ok(picture) = section.update(cx, |section, cx| {
                    let voice = voice(cx)?;
                    let (is_shown, picture) = {
                        let voice = voice.read(cx);
                        (
                            voice.camera_phase() != CameraPhase::Off
                                || voice.is_camera_preview_running(),
                            voice.take_camera_preview_picture(),
                        )
                    };
                    if !is_shown && section.picture.is_some() {
                        section.released.extend(section.picture.take());
                        cx.notify();
                    }
                    picture
                }) else {
                    return;
                };
                let Some(picture) = picture else {
                    continue;
                };
                let image = cx
                    .background_spawn(async move { render_image_of(&picture) })
                    .await;
                let is_gone = section
                    .update(cx, |section, cx| {
                        section.released.extend(section.picture.take());
                        section.picture = image;
                        cx.notify();
                    })
                    .is_err();
                if is_gone {
                    return;
                }
            }
        })
    }
}

impl Render for VoiceVideoSection {
    #[allow(clippy::too_many_lines)] // the Video part, in the web's order
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for released in self.released.drain(..) {
            // Only fails for a window without an atlas, which holds nothing to free.
            let _ = window.drop_image(released);
        }
        let catalogue = strings(cx);
        let words = catalogue.voice_settings();
        let tokens = theme(cx);
        let Some(voice_entity) = voice(cx) else {
            return div();
        };
        let voice = voice_entity.read(cx);
        let settings = current_settings(cx);
        let listed = voice.listed_cameras();
        let choices = DeviceChoices {
            default_device_name: None,
            devices: listed
                .unwrap_or_default()
                .iter()
                .map(|camera| DeviceChoice {
                    device_id: camera.id.clone(),
                    label: camera.name.clone(),
                })
                .collect(),
        };
        let is_camera_on = voice.camera_phase() != CameraPhase::Off;
        let is_preview_running = voice.is_camera_preview_running();
        let is_shown = is_camera_on || is_preview_running;
        let problem = voice.camera_preview_problem();
        let has_no_camera = listed.is_some_and(<[_]>::is_empty);
        let toggle_voice = voice_entity.downgrade();
        let problem_sentence: SharedString = problem
            .map_or_else(SharedString::default, |problem| {
                problem.sentence(catalogue).into()
            });

        let controls = div()
            .flex()
            .flex_col()
            .gap(length(SPACE_XS))
            .flex_1()
            .min_w(length(COMPONENT_SETTINGS_VOICE_COLUMN_MIN_WIDTH))
            .child(device_group(
                "voice-camera-device",
                words.camera(),
                &choices,
                listed.is_some(),
                settings.camera_device_id.as_deref(),
                words.camera_device_missing(),
                |settings, device_id| settings.camera_device_id = device_id,
                cx,
            ))
            .when(has_no_camera, |controls| {
                controls.child(helper_text(catalogue.camera().problems().not_found(), cx))
            })
            .child(helper_text(
                if is_camera_on {
                    words.camera_live_description()
                } else {
                    words.camera_preview_description()
                },
                cx,
            ))
            .when(!is_camera_on, |controls| {
                controls.child(
                    div().flex().child(
                        Button::new(
                            "voice-camera-test",
                            if is_preview_running {
                                words.stop_camera_test()
                            } else {
                                words.test_camera()
                            },
                        )
                        .variant(ButtonVariant::Secondary)
                        .on_click(move |_, cx| {
                            let _ = toggle_voice.update(cx, |voice, cx| {
                                if voice.is_camera_preview_running() {
                                    voice.stop_camera_preview(cx);
                                } else {
                                    voice.start_camera_preview(cx);
                                }
                            });
                        }),
                    ),
                )
            })
            .when_some(problem, |controls, problem| {
                controls.child(warning_line(
                    IconName::WarningCircle,
                    problem.sentence(catalogue),
                    tokens.component_settings_error_text,
                ))
            })
            .child(polite_live_region(
                "voice-camera-preview-problem",
                problem_sentence,
            ));

        let picture = self.picture.clone().filter(|_| is_shown);
        let caption = if is_shown {
            catalogue.camera().starting()
        } else {
            words.camera_preview_idle()
        };
        let stage = div()
            .id("voice-camera-preview")
            .role(Role::Figure)
            .aria_label(words.camera_preview())
            .relative()
            .flex_1()
            .min_w(length(COMPONENT_SETTINGS_VOICE_COLUMN_MIN_WIDTH))
            .aspect_ratio(PREVIEW_ASPECT_RATIO)
            .overflow_hidden()
            .rounded(length(COMPONENT_CALL_VIEW_STAGE_RADIUS))
            .bg(color(tokens.component_call_view_stage))
            .map(|stage| match picture {
                Some(picture) => stage.child(
                    img(picture)
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Cover),
                ),
                None => stage.child(
                    with_type(div(), TYPE_BODY)
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(length(SPACE_2XS))
                        .p(length(SPACE_MD))
                        .text_color(color(tokens.component_call_view_tile_text))
                        .when(!is_shown, |caption| {
                            caption.child(icon(
                                IconName::VideoCameraSlash,
                                COMPONENT_VOICE_PANEL_SIGNAL_ICON_SIZE,
                                color(tokens.component_call_view_tile_text),
                            ))
                        })
                        .child(caption),
                ),
            });

        div()
            .mt(length(COMPONENT_SETTINGS_SECTION_GAP))
            .flex()
            .flex_col()
            .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
            .child(section_label("voice-video", words.video(), cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x(length(SPACE_LG))
                    .gap_y(length(SPACE_MD))
                    .child(controls)
                    .child(stage),
            )
    }
}
