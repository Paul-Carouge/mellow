//! A server's or channel's icon on its Overview page (`IconEditor.svelte` and
//! `PictureCropDialog.svelte`, DESIGN.md §6.54): the current icon, "Upload icon" opening the crop
//! dialog once a picture is chosen, and "Remove icon". Saved at once, apart from the page's
//! unsaved-changes bar, as avatars are. The instance crops and compresses; the gateway brings
//! the new icon to everyone, this page included.
//!
//! The crop dialog shows the picture on a stage, the kept square framed and the rest dimmed
//! around it. It opens on the largest centred square, which is what the instance keeps when
//! nothing moves; dragging, the wheel, the arrow keys, + and −, or the zoom slider choose another.
//! Under the stage, the result at its real size where it will live. The file is sent as it is
//! (an animated GIF stays animated), with the crop.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, ObjectFit,
    ParentElement as _, PathPromptOptions, Pixels, Point, Render, RenderImage, Role, ScrollEvent,
    SharedString, StatefulInteractiveElement as _, Styled as _, StyledImage as _, Subscription,
    Window, div, img, prelude::FluentBuilder as _, px,
};
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::format::server_initials;
use mellow_client_core::shared_logic::user_settings::MAXIMUM_PROFILE_IMAGE_BYTES;
use mellow_protocol::{IconCropQuery, ProfileImage, Snowflake};

use super::picture_crop::{
    CropSquare, PictureSize, contained_size, crop_moved_by, crop_to_send, crop_zoomed_to,
    is_picture_too_large, largest_centred_square, maximum_zoom, picture_size_of,
    placement_in_frame, zoom_of,
};
use super::picture_upload::{
    PictureReadProblem, describe_image_failure, describe_read_problem, read_picture,
};
use super::{error_alert, saved_notice};
use crate::components::{
    Button, ButtonVariant, ModalDialog, ModalDialogEvent, RangeBounds, RangeField, bounds_recorder,
    focus_ring, icon, is_focus_visible, with_type,
};
use crate::i18n::strings;
use crate::messages::{file_size_label, picture_animation, picture_for_box};
use crate::pictures::{Animation, DisplaySize, PictureDecodeError, decode_frames};
use crate::session::Session;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CHANNEL_ROW_GAP, COMPONENT_CHANNEL_ROW_HEIGHT, COMPONENT_CHANNEL_ROW_ICON_GAP,
    COMPONENT_CHANNEL_ROW_ICON_SIZE, COMPONENT_CHANNEL_ROW_IMAGE_RADIUS,
    COMPONENT_CHANNEL_ROW_PADDING_INLINE, COMPONENT_CHANNEL_ROW_RADIUS,
    COMPONENT_DIALOG_BUTTON_GAP, COMPONENT_DIALOG_SECTION_GAP, COMPONENT_DIALOG_WIDTH,
    COMPONENT_ICON_PICKER_FRAME_BORDER_WIDTH, COMPONENT_ICON_PICKER_FRAME_INSET,
    COMPONENT_ICON_PICKER_PREVIEW_RADIUS_CHANNEL, COMPONENT_ICON_PICKER_PREVIEW_SIZE,
    COMPONENT_ICON_PICKER_SAMPLE_GAP, COMPONENT_ICON_PICKER_STAGE_RADIUS,
    COMPONENT_RAIL_ICON_RADIUS_ACTIVE, COMPONENT_RAIL_ICON_RADIUS_REST, COMPONENT_RAIL_ITEM_SIZE,
    COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP, COMPONENT_SETTINGS_ACTION_GAP,
    COMPONENT_SETTINGS_SECTION_LABEL_GAP, SPACE_2XS, SPACE_XS, TYPE_BODY, TYPE_BODY_SM,
    TYPE_CATEGORY, TYPE_CHANNEL_NAME, TYPE_HEADING_LG,
};
use crate::theme::{color, length, theme};

/// Each arrow key moves the crop by this share of its side; with Shift, by the larger one.
const KEYBOARD_STEP_SHARE: f32 = 0.02;
const KEYBOARD_LARGE_STEP_SHARE: f32 = 0.1;
const KEYBOARD_ZOOM_FACTOR: f32 = 1.1;
/// How much one wheel pixel zooms: about 10 % per notch of a mouse wheel, as on the web.
const WHEEL_ZOOM_PER_PIXEL: f32 = 0.001;
/// A wheel notch reported in lines, as a mouse wheel does on Linux.
const WHEEL_PIXELS_PER_LINE: f32 = 100.0;
const ZOOM_SLIDER_STEP: f32 = 0.01;
/// The stage's picture is decoded to fit this many device pixels a side: the frame is never
/// wider than a dialog, and the samples are smaller.
const STAGE_DECODE_SIDE: f32 = 1024.0;

/// Whose icon the editor changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconTarget {
    Server(Snowflake),
    Channel(Snowflake),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IconBusy {
    /// The chosen file is being read and measured, before the dialog opens.
    Opening,
    Upload,
    Remove,
}

/// The picture under the crop, decoded once for the stage and the samples.
#[derive(Clone)]
struct ChosenPicture {
    bytes: Arc<Vec<u8>>,
    size: PictureSize,
    still: Arc<RenderImage>,
}

pub struct IconEditor {
    session: Entity<Session>,
    target: IconTarget,
    busy: Option<IconBusy>,
    error_message: Option<SharedString>,
    notice: Option<SharedString>,
    upload_focus: FocusHandle,
    crop_dialog: Option<(Entity<ModalDialog>, Entity<CropStage>, Subscription)>,
    _session_observation: Subscription,
}

impl std::fmt::Debug for IconEditor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IconEditor")
            .field("target", &self.target)
            .field("busy", &self.busy)
            .finish_non_exhaustive()
    }
}

/// The stored icon and the name the editor shows (initials, samples).
fn icon_and_name(
    session: &Entity<Session>,
    target: IconTarget,
    cx: &App,
) -> (Option<ProfileImage>, String) {
    let state = session.read(cx).state();
    match target {
        IconTarget::Server(guild_id) => state
            .guild(guild_id)
            .map(|guild| (guild.icon, guild.name.clone()))
            .unwrap_or_default(),
        IconTarget::Channel(channel_id) => state
            .guild_channel(channel_id)
            .map(|channel| (channel.icon, channel.name.clone()))
            .unwrap_or_default(),
    }
}

/// Measures and decodes a picked picture for the stage, fitted in its square so a panorama
/// stays within the texture atlas's limits; refuses what the instance would refuse for its size.
fn prepare_picture(bytes: Vec<u8>) -> Result<ChosenPicture, PictureReadProblem> {
    let size = picture_size_of(&bytes).ok_or(PictureReadProblem::Unreadable)?;
    if is_picture_too_large(size) {
        return Err(PictureReadProblem::TooManyPixels);
    }
    let (width, height) = contained_size(size, STAGE_DECODE_SIDE);
    let frames = decode_frames(&bytes, Animation::StillFrame, DisplaySize { width, height })
        .map_err(|error| match error {
            PictureDecodeError::TooLarge => PictureReadProblem::TooManyPixels,
            PictureDecodeError::Undecodable | PictureDecodeError::ColourConversion => {
                PictureReadProblem::Unreadable
            }
        })?;
    Ok(ChosenPicture {
        bytes: Arc::new(bytes),
        size,
        still: Arc::new(RenderImage::new(frames)),
    })
}

/// The crop of the stage's still picture, `sample_side` device pixels square, for the samples.
/// Reads the decoded frame in place (no copy of the whole picture) and scales only the crop.
fn cropped_still(
    picture: &ChosenPicture,
    crop: CropSquare,
    sample_side: f32,
) -> Option<RenderImage> {
    let size = picture.still.size(0);
    let (still_width, still_height) = (
        u32::try_from(size.width.0).ok()?,
        u32::try_from(size.height.0).ok()?,
    );
    // The decoded frame's own channel order (BGRA) is kept: cutting and scaling treat channels
    // alike.
    let still = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(
        still_width,
        still_height,
        picture.still.as_bytes(0)?,
    )?;
    #[allow(clippy::cast_precision_loss)]
    let scale = still_width as f32 / picture.size.width;
    // Inside the decoded frame by construction; rounding is kept within it.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let to_pixels = |value: f32| (value * scale).round().max(0.0) as u32;
    let x = to_pixels(crop.x).min(still_width.saturating_sub(1));
    let y = to_pixels(crop.y).min(still_height.saturating_sub(1));
    let cut_side = to_pixels(crop.side)
        .max(1)
        .min(still_width - x)
        .min(still_height - y);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let sample_side = sample_side.round().max(1.0) as u32;
    let cut = image::imageops::crop_imm(&still, x, y, cut_side, cut_side);
    let sample = image::imageops::thumbnail(&*cut, sample_side, sample_side);
    Some(RenderImage::new(vec![image::Frame::new(sample)]))
}

/// Frees `image`'s tiles in every window's atlas. Deferred, as `voice/video_view.rs` does: the
/// window whose handler is running is out of the app's window list until it returns.
fn free_atlas_tiles(image: Arc<RenderImage>, cx: &mut App) {
    cx.defer(move |cx| cx.drop_image(image, None));
}

impl IconEditor {
    pub fn new(session: Entity<Session>, target: IconTarget, cx: &mut Context<Self>) -> Self {
        let observation = cx.observe(&session, |_, _, cx| cx.notify());
        Self {
            session,
            target,
            busy: None,
            error_message: None,
            notice: None,
            upload_focus: cx.focus_handle().tab_stop(true),
            crop_dialog: None,
            _session_observation: observation,
        }
    }

    fn change_failed(cx: &App) -> &'static str {
        strings(cx).icon_editor().change_failed()
    }

    fn choose_picture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() || self.crop_dialog.is_some() {
            return;
        }
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        let window_handle = window.window_handle();
        cx.spawn(async move |editor, cx| {
            let path: PathBuf = match picked.await {
                Ok(Ok(Some(paths))) => match paths.into_iter().next() {
                    Some(path) => path,
                    None => return,
                },
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(error)) => {
                    tracing::warn!(%error, "the file picker failed");
                    return;
                }
            };
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = editor.update(cx, |editor, cx| {
                    editor.open_picked_file(path, window, cx);
                });
            });
        })
        .detach();
    }

    /// Reads and measures the picked file off the UI thread, then opens the crop dialog on it.
    fn open_picked_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() || self.crop_dialog.is_some() {
            return;
        }
        self.busy = Some(IconBusy::Opening);
        self.error_message = None;
        self.notice = None;
        cx.notify();
        let window_handle = window.window_handle();
        cx.spawn(async move |editor, cx| {
            let prepared = cx
                .background_spawn(async move { read_picture(&path).and_then(prepare_picture) })
                .await;
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = editor.update(cx, |editor, cx| {
                    editor.busy = None;
                    match prepared {
                        Ok(picture) => editor.open_crop_dialog(picture, window, cx),
                        Err(problem) => {
                            editor.error_message = Some(
                                describe_read_problem(&problem, Self::change_failed(cx), cx).into(),
                            );
                        }
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn open_crop_dialog(
        &mut self,
        picture: ChosenPicture,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let catalogue = strings(cx);
        let words = catalogue.icon_editor().crop();
        let (_, name) = icon_and_name(&self.session, self.target, cx);
        let target = self.target;
        let stage_picture = picture.clone();
        let stage = cx.new(|cx| CropStage::new(stage_picture, target, name, cx));
        let dialog = cx.new(|cx| {
            ModalDialog::new(words.title(), catalogue.common().cancel(), window, cx)
                .subtitle(words.subtitle())
                .body(stage.clone().into())
                .submit(words.save(), words.saving(), false)
        });
        // The stage takes focus, as the web's `data-initial-focus`: its keys move the picture.
        let stage_focus = stage.read(cx).focus.clone();
        window.focus(&stage_focus, cx);
        let subscribed_stage = stage.clone();
        let subscription = cx.subscribe_in(
            &dialog,
            window,
            move |editor, dialog, event: &ModalDialogEvent, window, cx| match event {
                ModalDialogEvent::Closed => {
                    editor.crop_dialog = None;
                    cx.notify();
                }
                ModalDialogEvent::Submitted => {
                    let crop = subscribed_stage.read(cx).crop_to_send();
                    editor.upload(picture.bytes.as_ref().clone(), crop, dialog, window, cx);
                }
            },
        );
        self.crop_dialog = Some((dialog, stage, subscription));
    }

    fn upload(
        &mut self,
        bytes: Vec<u8>,
        crop: IconCropQuery,
        dialog: &Entity<ModalDialog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(IconBusy::Upload);
        self.error_message = None;
        self.notice = None;
        dialog.update(cx, |dialog, cx| {
            dialog.set_error(None, cx);
            dialog.set_busy(true, cx);
        });
        cx.notify();
        let editor = cx.entity().downgrade();
        let dialog = dialog.downgrade();
        let window_handle = window.window_handle();
        let target = self.target;
        self.session.update(cx, |session, cx| {
            session.send_settings_request(
                "change icon",
                move |rest| async move {
                    match target {
                        IconTarget::Server(guild_id) => {
                            rest.set_guild_icon(guild_id, bytes, &crop).await.map(drop)
                        }
                        IconTarget::Channel(channel_id) => rest
                            .set_channel_icon(channel_id, bytes, &crop)
                            .await
                            .map(drop),
                    }
                },
                cx,
                move |answer, cx| {
                    let _ = window_handle.update(cx, |_, window, cx| {
                        let failure = answer.as_ref().err().map(|error| {
                            SharedString::from(describe_image_failure(
                                error,
                                Self::change_failed(cx),
                                cx,
                            ))
                        });
                        let _ = dialog.update(cx, |dialog, cx| {
                            dialog.set_busy(false, cx);
                            match &failure {
                                // The dialog stays open with its square, so trying again needs
                                // no new pick.
                                Some(message) => dialog.set_error(Some(message.clone()), cx),
                                None => dialog.request_close(window, cx),
                            }
                        });
                        let _ = editor.update(cx, |editor, cx| {
                            editor.busy = None;
                            if failure.is_none() {
                                editor.notice = Some(strings(cx).icon_editor().saved().into());
                            }
                            cx.notify();
                        });
                    });
                },
            );
        });
    }

    fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(IconBusy::Remove);
        self.error_message = None;
        self.notice = None;
        cx.notify();
        let editor = cx.entity().downgrade();
        let window_handle = window.window_handle();
        let target = self.target;
        let failure = strings(cx).icon_editor().remove_failed().to_owned();
        self.session.update(cx, |session, cx| {
            session.send_settings_change(
                "remove icon",
                failure,
                move |rest| async move {
                    match target {
                        IconTarget::Server(guild_id) => {
                            rest.remove_guild_icon(guild_id).await.map(drop)
                        }
                        IconTarget::Channel(channel_id) => {
                            rest.remove_channel_icon(channel_id).await.map(drop)
                        }
                    }
                },
                cx,
                move |outcome, cx| {
                    let _ = window_handle.update(cx, |_, window, cx| {
                        let _ = editor.update(cx, |editor, cx| {
                            editor.busy = None;
                            match outcome {
                                Ok(()) => {
                                    editor.notice =
                                        Some(strings(cx).icon_editor().removed().into());
                                    // "Remove icon" goes with the icon: focus stays in the editor.
                                    window.focus(&editor.upload_focus, cx);
                                }
                                Err(message) => editor.error_message = Some(message.into()),
                            }
                            cx.notify();
                        });
                    });
                },
            );
        });
    }

    fn render_preview(
        &self,
        icon_image: Option<ProfileImage>,
        name: &str,
        window: &Window,
        cx: &App,
    ) -> gpui::Div {
        let tokens = theme(cx);
        let is_server = matches!(self.target, IconTarget::Server(_));
        let preview = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .size(length(COMPONENT_ICON_PICKER_PREVIEW_SIZE))
            .bg(color(tokens.component_icon_picker_preview_background))
            .text_color(color(tokens.component_icon_picker_preview_text))
            .map(|preview| {
                if is_server {
                    preview.rounded_full()
                } else {
                    preview.rounded(length(COMPONENT_ICON_PICKER_PREVIEW_RADIUS_CHANNEL))
                }
            });
        match icon_image {
            Some(image) => {
                // The preview plays an animated server icon unless reduced motion keeps
                // pictures still; channel icons are always stored still.
                let animation = if is_server {
                    picture_animation(cx)
                } else {
                    Animation::StillFrame
                };
                preview.child(
                    img(picture_for_box(
                        DownloadableFile::Icon(image.id),
                        COMPONENT_ICON_PICKER_PREVIEW_SIZE,
                        COMPONENT_ICON_PICKER_PREVIEW_SIZE,
                        animation,
                        window,
                    ))
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    // GPUI clips children to rectangles: the picture rounds its own corners.
                    .map(|picture| {
                        if is_server {
                            picture.rounded_full()
                        } else {
                            picture.rounded(length(COMPONENT_ICON_PICKER_PREVIEW_RADIUS_CHANNEL))
                        }
                    }),
                )
            }
            None if is_server => {
                preview.child(with_type(div(), TYPE_HEADING_LG).child(server_initials(name)))
            }
            None => preview.child(icon(
                IconName::Hash,
                COMPONENT_ICON_PICKER_PREVIEW_SIZE / 2.0,
                color(tokens.component_icon_picker_preview_text),
            )),
        }
    }
}

impl Render for IconEditor {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let editor_words = catalogue.icon_editor();
        let (icon_image, name) = icon_and_name(&self.session, self.target, cx);
        let has_icon = icon_image.is_some();
        let (title, empty_text, note) = match self.target {
            IconTarget::Server(_) => {
                let words = editor_words.server();
                (words.title(), words.empty_text(), words.note())
            }
            IconTarget::Channel(_) => {
                let words = editor_words.channel();
                (words.title(), words.empty_text(), words.note())
            }
        };
        let formats =
            editor_words.formats(&file_size_label(MAXIMUM_PROFILE_IMAGE_BYTES, catalogue));
        let description = format!("{} {formats}", if has_icon { note } else { empty_text });
        let is_busy = self.busy.is_some();
        let upload_label = match self.busy {
            Some(IconBusy::Opening) => editor_words.crop().opening(),
            Some(IconBusy::Upload) => editor_words.uploading(),
            _ => editor_words.upload(),
        };
        let remove_label = if self.busy == Some(IconBusy::Remove) {
            editor_words.removing()
        } else {
            editor_words.remove()
        };
        let editor = cx.entity().downgrade();
        let remove_editor = editor.clone();
        div()
            .id("icon-editor")
            .role(Role::Group)
            .aria_label(title)
            .flex()
            .flex_wrap()
            .items_start()
            .gap(length(COMPONENT_SETTINGS_ACTION_GAP))
            .child(self.render_preview(icon_image, &name, window, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .max_w(length(COMPONENT_DIALOG_WIDTH))
                    .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                    .child(
                        with_type(div().id("icon-editor-title"), TYPE_CATEGORY)
                            .role(Role::Heading)
                            .text_color(color(tokens.component_settings_section_label_text))
                            .child(title),
                    )
                    .child(
                        with_type(div(), TYPE_BODY)
                            .text_color(color(tokens.component_settings_body_text))
                            .child(description),
                    )
                    .child(
                        div()
                            .mt(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                            .flex()
                            .flex_wrap()
                            .gap(length(COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP))
                            .child(
                                Button::new("icon-editor-upload", upload_label)
                                    .variant(ButtonVariant::Secondary)
                                    .disabled(is_busy)
                                    .track_focus(&self.upload_focus)
                                    .on_click(move |window, cx| {
                                        let _ = editor.update(cx, |editor, cx| {
                                            editor.choose_picture(window, cx);
                                        });
                                    }),
                            )
                            .when(has_icon, |buttons| {
                                buttons.child(
                                    Button::new("icon-editor-remove", remove_label)
                                        .variant(ButtonVariant::Secondary)
                                        .disabled(is_busy)
                                        .on_click(move |window, cx| {
                                            let _ = remove_editor.update(cx, |editor, cx| {
                                                editor.remove(window, cx);
                                            });
                                        }),
                                )
                            }),
                    )
                    // Outside the column's gap, so the empty live regions take no room.
                    .child(
                        div()
                            .when(self.error_message.is_none(), |alert| {
                                alert.mt(-length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                            })
                            .child(error_alert(
                                "icon-editor-error",
                                self.error_message.clone(),
                                cx,
                            ))
                            .child(saved_notice("icon-editor-notice", self.notice.clone(), cx)),
                    ),
            )
            .when_some(self.crop_dialog.as_ref(), |editor, (dialog, _, _)| {
                editor.child(dialog.clone())
            })
    }
}

/// The crop dialog's body: the stage, the zoom, and the samples at their real size.
struct CropStage {
    picture: ChosenPicture,
    target: IconTarget,
    name: String,
    crop: CropSquare,
    focus: FocusHandle,
    /// The frame as last drawn: how many window pixels a picture pixel takes.
    frame_bounds: Entity<Option<Bounds<Pixels>>>,
    /// Where the pointer was at the last drag step.
    drag_from: Option<Point<Pixels>>,
    /// The crop alone, small, for the samples: GPUI rounds a picture's own corners but clips
    /// its children to rectangles, so a rounded sample needs the cropped picture itself.
    sample: Option<(CropSquare, Arc<RenderImage>)>,
}

impl CropStage {
    fn new(
        picture: ChosenPicture,
        target: IconTarget,
        name: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let crop = largest_centred_square(picture.size);
        // The textures go with the stage, however the dialog ends (Cancel, saved, the settings
        // closed under it).
        cx.on_release(|stage: &mut Self, cx| {
            let still = Arc::clone(&stage.picture.still);
            let sample = stage.sample.take().map(|(_, sample)| sample);
            free_atlas_tiles(still, cx);
            if let Some(sample) = sample {
                free_atlas_tiles(sample, cx);
            }
        })
        .detach();
        Self {
            picture,
            target,
            name,
            crop,
            focus: cx.focus_handle().tab_stop(true),
            frame_bounds: cx.new(|_| None),
            drag_from: None,
            sample: None,
        }
    }

    /// The sample picture for the current crop, cut again when the crop moved.
    fn sample_picture(&mut self, window: &mut Window) -> Option<Arc<RenderImage>> {
        if let Some((crop, sample)) = &self.sample
            && *crop == self.crop
        {
            return Some(Arc::clone(sample));
        }
        let side = COMPONENT_RAIL_ITEM_SIZE * window.scale_factor();
        let sample = Arc::new(cropped_still(&self.picture, self.crop, side)?);
        if let Some((_, previous)) = self.sample.replace((self.crop, Arc::clone(&sample))) {
            let _ = window.drop_image(previous);
        }
        Some(sample)
    }

    fn crop_to_send(&self) -> IconCropQuery {
        crop_to_send(self.crop, self.picture.size)
    }

    fn zoom(&self) -> f32 {
        zoom_of(self.crop, self.picture.size)
    }

    /// Window pixels per picture pixel inside the frame.
    fn scale(&self, cx: &App) -> Option<(Bounds<Pixels>, f32)> {
        let bounds = (*self.frame_bounds.read(cx))?;
        let width = f32::from(bounds.size.width);
        (width > 0.0).then(|| (bounds, width / self.crop.side))
    }

    fn point_in_picture(&self, position: Point<Pixels>, cx: &App) -> Option<(f32, f32)> {
        let (bounds, scale) = self.scale(cx)?;
        Some((
            self.crop.x + f32::from(position.x - bounds.origin.x) / scale,
            self.crop.y + f32::from(position.y - bounds.origin.y) / scale,
        ))
    }

    fn set_crop(&mut self, crop: CropSquare, cx: &mut Context<Self>) {
        if crop != self.crop {
            self.crop = crop;
            cx.notify();
        }
    }

    fn zoom_to(&mut self, zoom: f32, anchor: Option<(f32, f32)>, cx: &mut Context<Self>) {
        let crop = crop_zoomed_to(self.crop, self.picture.size, zoom, anchor);
        self.set_crop(crop, cx);
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        self.drag_from = Some(event.position);
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(from) = self.drag_from else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag_from = None;
            return;
        }
        let Some((_, scale)) = self.scale(cx) else {
            return;
        };
        // Dragging the picture right shows more of its left: the crop moves the other way.
        let delta_x = -f32::from(event.position.x - from.x) / scale;
        let delta_y = -f32::from(event.position.y - from.y) / scale;
        self.drag_from = Some(event.position);
        let crop = crop_moved_by(self.crop, self.picture.size, delta_x, delta_y);
        self.set_crop(crop, cx);
    }

    fn on_scroll_wheel(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let pixels_up = f32::from(event.delta.pixel_delta(px(WHEEL_PIXELS_PER_LINE)).y);
        let zoom = self.zoom() * (pixels_up * WHEEL_ZOOM_PER_PIXEL).exp();
        let anchor = self.point_in_picture(event.position, cx);
        self.zoom_to(zoom, anchor, cx);
        cx.stop_propagation();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.platform {
            return;
        }
        let share = if keystroke.modifiers.shift {
            KEYBOARD_LARGE_STEP_SHARE
        } else {
            KEYBOARD_STEP_SHARE
        };
        let step = self.crop.side * share;
        // The keys move the picture, as dragging does.
        let movement = match keystroke.key.as_str() {
            "left" => Some((step, 0.0)),
            "right" => Some((-step, 0.0)),
            "up" => Some((0.0, step)),
            "down" => Some((0.0, -step)),
            _ => None,
        };
        if let Some((delta_x, delta_y)) = movement {
            let crop = crop_moved_by(self.crop, self.picture.size, delta_x, delta_y);
            self.set_crop(crop, cx);
            cx.stop_propagation();
            return;
        }
        let typed = keystroke
            .key_char
            .as_deref()
            .unwrap_or(keystroke.key.as_str());
        let zoom = match typed {
            "+" | "=" => self.zoom() * KEYBOARD_ZOOM_FACTOR,
            "-" | "_" | "−" => self.zoom() / KEYBOARD_ZOOM_FACTOR,
            _ => return,
        };
        self.zoom_to(zoom, None, cx);
        cx.stop_propagation();
    }

    /// The whole picture in a `side`-wide box, placed so the crop fills it.
    fn cropped_picture(&self, side: f32) -> impl IntoElement {
        let (left, top, width, height) = placement_in_frame(self.crop, self.picture.size, side);
        img(gpui::ImageSource::Render(Arc::clone(&self.picture.still)))
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .h(px(height))
    }

    fn sample(
        label: &'static str,
        picture: Option<&Arc<RenderImage>>,
        side: f32,
        radius: f32,
        cx: &App,
    ) -> gpui::Div {
        let tokens = theme(cx);
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(length(SPACE_2XS))
            .child(
                div()
                    .size(length(side))
                    .rounded(length(radius))
                    .bg(color(tokens.component_icon_picker_stage_background))
                    .when_some(picture, |sample, picture| {
                        sample.child(
                            img(gpui::ImageSource::Render(Arc::clone(picture)))
                                .size_full()
                                .rounded(length(radius)),
                        )
                    }),
            )
            .child(
                with_type(div(), TYPE_BODY_SM)
                    .text_color(color(tokens.component_icon_picker_sample_label_text))
                    .child(label),
            )
    }

    fn render_samples(&mut self, window: &mut Window, cx: &App) -> gpui::Div {
        let tokens = theme(cx);
        let picture = self.sample_picture(window);
        let picture = picture.as_ref();
        let words = strings(cx).icon_editor().crop();
        let samples = match self.target {
            // The strip's two shapes: a circle at rest, the squircle when hovered or selected.
            IconTarget::Server(_) => div()
                .flex()
                .gap(length(COMPONENT_ICON_PICKER_SAMPLE_GAP))
                .child(Self::sample(
                    words.sample_at_rest(),
                    picture,
                    COMPONENT_RAIL_ITEM_SIZE,
                    COMPONENT_RAIL_ICON_RADIUS_REST,
                    cx,
                ))
                .child(Self::sample(
                    words.sample_selected(),
                    picture,
                    COMPONENT_RAIL_ITEM_SIZE,
                    COMPONENT_RAIL_ICON_RADIUS_ACTIVE,
                    cx,
                )),
            // A channel row on the navigator, read and selected, at the list's own size.
            IconTarget::Channel(_) => div()
                .flex()
                .flex_col()
                .gap(length(COMPONENT_CHANNEL_ROW_GAP))
                .p(length(SPACE_XS))
                .rounded(length(COMPONENT_ICON_PICKER_STAGE_RADIUS))
                .bg(color(tokens.component_navigator_background))
                .children([false, true].map(|is_selected| {
                    div()
                        .flex()
                        .items_center()
                        .h(length(COMPONENT_CHANNEL_ROW_HEIGHT))
                        .gap(length(COMPONENT_CHANNEL_ROW_ICON_GAP))
                        .px(length(COMPONENT_CHANNEL_ROW_PADDING_INLINE))
                        .rounded(length(COMPONENT_CHANNEL_ROW_RADIUS))
                        .map(|row| {
                            if is_selected {
                                row.bg(color(tokens.component_channel_row_background_selected))
                                    .text_color(color(tokens.component_channel_row_text_selected))
                            } else {
                                row.text_color(color(tokens.component_channel_row_text_read))
                            }
                        })
                        .child(
                            div()
                                .flex_none()
                                .size(length(COMPONENT_CHANNEL_ROW_ICON_SIZE))
                                .when_some(picture, |sample, picture| {
                                    sample.child(
                                        img(gpui::ImageSource::Render(Arc::clone(picture)))
                                            .size_full()
                                            .rounded(length(COMPONENT_CHANNEL_ROW_IMAGE_RADIUS)),
                                    )
                                }),
                        )
                        .child(
                            with_type(div(), TYPE_CHANNEL_NAME)
                                .min_w_0()
                                .truncate()
                                .child(self.name.clone()),
                        )
                })),
        };
        // What sighted people check by eye; the stage already holds the picture for everyone.
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
            .child(
                with_type(div(), TYPE_CATEGORY)
                    .text_color(color(tokens.component_settings_section_label_text))
                    .child(words.samples()),
            )
            .child(samples)
    }
}

impl Render for CropStage {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let words = strings(cx).icon_editor().crop();
        let frame_side = self
            .frame_bounds
            .read(cx)
            .map_or(0.0, |bounds| f32::from(bounds.size.width));
        let zoom = self.zoom();
        let zoom_limit = maximum_zoom(self.picture.size);
        let is_ring_shown = is_focus_visible(&self.focus, window);
        let scrim = color(tokens.component_icon_picker_outside_scrim);
        let inset = length(COMPONENT_ICON_PICKER_FRAME_INSET);
        let stage_radius = length(COMPONENT_ICON_PICKER_STAGE_RADIUS);
        let stage = cx.entity().downgrade();
        let centre_stage = stage.clone();

        let stage_element = div()
            .id("icon-crop-stage")
            .role(Role::Application)
            .aria_label(SharedString::from(format!(
                "{}. {}",
                words.stage(),
                words.stage_hint()
            )))
            .track_focus(&self.focus)
            .relative()
            .w_full()
            .rounded(length(COMPONENT_ICON_PICKER_STAGE_RADIUS))
            .bg(color(tokens.component_icon_picker_stage_background))
            .p(inset)
            .cursor_grab()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|stage, _, _, _| stage.drag_from = None),
            )
            .on_scroll(cx.listener(Self::on_scroll_wheel))
            .on_key_down(cx.listener(Self::on_key_down))
            .child(
                // Clips the picture to the stage, whose corners are rounded.
                div()
                    .absolute()
                    .inset_0()
                    .overflow_hidden()
                    .rounded(length(COMPONENT_ICON_PICKER_STAGE_RADIUS))
                    .child(
                        div()
                            .absolute()
                            .top(inset)
                            .left(inset)
                            .size(px(frame_side))
                            .when(frame_side > 0.0, |frame| {
                                frame.child(self.cropped_picture(frame_side))
                            }),
                    )
                    // The outside of the frame, dimmed: four bands around it.
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .h(inset)
                            .bg(scrim),
                    )
                    .child(
                        div()
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .right_0()
                            .h(inset)
                            .bg(scrim),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(inset)
                            .bottom(inset)
                            .left_0()
                            .w(inset)
                            .bg(scrim),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(inset)
                            .bottom(inset)
                            .right_0()
                            .w(inset)
                            .bg(scrim),
                    )
                    // GPUI clips children to rectangles: a ring in the dialog's colour, its inner
                    // edge the stage's rounded edge, hides the picture's square corners.
                    .child(
                        div()
                            .absolute()
                            .inset(-stage_radius)
                            .border(stage_radius)
                            .rounded(stage_radius * 2.0)
                            .border_color(color(tokens.component_dialog_background)),
                    ),
            )
            .child(
                // The frame: square, as wide as the stage allows; its bounds size the picture.
                div()
                    .relative()
                    .w_full()
                    .aspect_ratio(1.0)
                    .border(length(COMPONENT_ICON_PICKER_FRAME_BORDER_WIDTH))
                    .border_color(color(tokens.component_icon_picker_frame_border))
                    .child(bounds_recorder(self.frame_bounds.clone())),
            )
            .when(is_ring_shown, |stage| {
                stage.child(focus_ring(COMPONENT_ICON_PICKER_STAGE_RADIUS, cx))
            });

        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_DIALOG_SECTION_GAP))
            .child(stage_element)
            .child(
                with_type(div(), TYPE_BODY_SM)
                    .mt(-length(SPACE_XS))
                    .text_color(color(tokens.component_icon_picker_sample_label_text))
                    .child(words.stage_hint()),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(length(COMPONENT_DIALOG_BUTTON_GAP))
                    .child(
                        div().flex_1().min_w_0().child(
                            RangeField::new(
                                "icon-crop-zoom",
                                words.zoom(),
                                zoom,
                                RangeBounds {
                                    minimum: 1.0,
                                    maximum: zoom_limit,
                                    step: ZOOM_SLIDER_STEP,
                                },
                                |zoom| format!("{}\u{202f}%", (zoom * 100.0).round()).into(),
                            )
                            // Nothing to zoom into: a tiny picture is shown whole.
                            .disabled(zoom_limit <= 1.0)
                            .on_change(move |zoom, _, cx| {
                                let _ = stage.update(cx, |stage, cx| stage.zoom_to(zoom, None, cx));
                            }),
                        ),
                    )
                    .child(
                        Button::new("icon-crop-centre", words.centre())
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, cx| {
                                let _ = centre_stage.update(cx, |stage, cx| {
                                    let centred = largest_centred_square(stage.picture.size);
                                    stage.set_crop(centred, cx);
                                });
                            }),
                    ),
            )
            .child(self.render_samples(window, cx))
    }
}

#[cfg(test)]
mod crop_stage_tests {
    use gpui::{TestAppContext, WindowAppearance};

    use super::*;

    fn picture_of(width: u32, height: u32) -> ChosenPicture {
        let mut bytes = Vec::new();
        image::RgbaImage::new(width, height)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        prepare_picture(bytes).expect("a readable picture")
    }

    #[gpui::test]
    fn the_keys_move_and_zoom_the_crop_as_dragging_does(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            cx.set_global(crate::theme::ActiveTheme::for_appearance(
                WindowAppearance::Dark,
            ));
        });
        let picture = picture_of(400, 200);
        let (stage, cx) = cx.add_window_view(|_, cx| {
            CropStage::new(
                picture,
                IconTarget::Server(Snowflake::from_raw(1)),
                "Kitchen".into(),
                cx,
            )
        });
        cx.update(|window, cx| {
            window.activate_window();
            let focus = stage.read(cx).focus.clone();
            window.focus(&focus, cx);
        });
        cx.run_until_parked();
        let crop = |cx: &mut gpui::VisualTestContext| stage.read_with(cx, |stage, _| stage.crop);
        assert_eq!(crop(cx).x, 100.0);

        // Right moves the picture right, so the square shows more of its left.
        cx.simulate_keystrokes("right");
        assert!((crop(cx).x - 96.0).abs() < 0.001);
        cx.simulate_keystrokes("shift-left");
        assert!((crop(cx).x - 116.0).abs() < 0.001);

        cx.simulate_keystrokes("=");
        assert!(
            (stage.read_with(cx, |stage, _| stage.zoom()) - KEYBOARD_ZOOM_FACTOR).abs() < 0.001
        );
        cx.simulate_keystrokes("-");
        assert!((stage.read_with(cx, |stage, _| stage.zoom()) - 1.0).abs() < 0.001);
        // Untouched size and back to the centre: nothing is sent, the instance centres.
        stage.update(cx, |stage, cx| {
            let centred = largest_centred_square(stage.picture.size);
            stage.set_crop(centred, cx);
        });
        assert_eq!(
            stage.read_with(cx, |stage, _| stage.crop_to_send()),
            IconCropQuery::default()
        );
    }
}
