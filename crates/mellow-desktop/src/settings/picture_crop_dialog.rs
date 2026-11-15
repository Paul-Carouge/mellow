//! The crop dialog of icons, avatars and banners (`PictureCropDialog.svelte`, DESIGN.md §6.54):
//! the chosen picture on a stage, the kept part framed and the rest dimmed around it: a square
//! for icons, a circle for avatars (they only ever show round), the profile card's band for
//! banners. It opens on the largest centred part, which is what the instance keeps when nothing
//! moves; dragging, the wheel, the arrow keys, + and −, or the zoom slider choose another, Enter
//! saves and Escape cancels. Under the stage, the result at its real size where it will live.
//!
//! The file is sent as it is, with the crop: an animated GIF or WebP stays animated, the
//! instance cutting every frame. The stage shows its first frame only, as channel icons and
//! reduced motion do on the web: the crop is the same for every frame, and one decoded frame
//! keeps a large GIF from filling the memory while the person frames it.

use std::sync::Arc;

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, EventEmitter, FocusHandle,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement as _, Pixels, Point, Render, RenderImage, Role, ScrollEvent,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, img,
    prelude::FluentBuilder as _, px,
};

use super::picture_crop::{
    BANNER_ASPECT, CropArea, PictureSize, SQUARE_ASPECT, SentCrop, contained_size, crop_in_still,
    crop_moved_by, crop_to_send, crop_zoomed_to, is_picture_too_large, largest_centred_crop,
    maximum_zoom, picture_size_of, placement_in_frame, zoom_of,
};
use super::picture_upload::PictureReadProblem;
use crate::components::{
    Avatar, AvatarPerson, Button, ButtonVariant, ModalDialog, RangeBounds, RangeField,
    bounds_recorder, focus_ring, is_focus_visible, with_type,
};
use crate::i18n::strings;
use crate::pictures::{Animation, DisplaySize, PictureDecodeError, decode_frames};
use crate::theme::tokens::{
    COMPONENT_CHANNEL_ROW_GAP, COMPONENT_CHANNEL_ROW_HEIGHT, COMPONENT_CHANNEL_ROW_ICON_GAP,
    COMPONENT_CHANNEL_ROW_ICON_SIZE, COMPONENT_CHANNEL_ROW_IMAGE_RADIUS,
    COMPONENT_CHANNEL_ROW_PADDING_INLINE, COMPONENT_CHANNEL_ROW_RADIUS,
    COMPONENT_DIALOG_BUTTON_GAP, COMPONENT_DIALOG_SECTION_GAP,
    COMPONENT_ICON_PICKER_FRAME_BORDER_WIDTH, COMPONENT_ICON_PICKER_FRAME_INSET,
    COMPONENT_ICON_PICKER_SAMPLE_GAP, COMPONENT_ICON_PICKER_STAGE_RADIUS,
    COMPONENT_MESSAGE_AVATAR_SIZE, COMPONENT_PROFILE_POPOVER_AVATAR_RING_WIDTH,
    COMPONENT_PROFILE_POPOVER_AVATAR_SIZE, COMPONENT_PROFILE_POPOVER_BANNER_IMAGE_HEIGHT,
    COMPONENT_PROFILE_POPOVER_BORDER_WIDTH, COMPONENT_PROFILE_POPOVER_PADDING,
    COMPONENT_PROFILE_POPOVER_RADIUS, COMPONENT_PROFILE_POPOVER_TEXT_GAP,
    COMPONENT_PROFILE_POPOVER_WIDTH, COMPONENT_RAIL_ICON_RADIUS_ACTIVE,
    COMPONENT_RAIL_ICON_RADIUS_REST, COMPONENT_RAIL_ITEM_SIZE,
    COMPONENT_SETTINGS_SECTION_LABEL_GAP, SPACE_2XS, SPACE_XS, TYPE_BODY_SM, TYPE_CATEGORY,
    TYPE_CHANNEL_NAME,
};
use crate::theme::{color, length, theme};

/// Each arrow key moves the crop by this share of its size; with Shift, by the larger one.
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

/// What the picture becomes: its frame's shape and the samples under the stage.
pub enum CropKind {
    ServerIcon,
    /// The channel's name, for the sample rows.
    ChannelIcon {
        name: String,
    },
    Avatar,
    /// Whose avatar covers part of the band in the profile card's sample.
    Banner {
        person: AvatarPerson,
    },
}

impl CropKind {
    fn aspect(&self) -> f32 {
        match self {
            Self::Banner { .. } => BANNER_ASPECT,
            _ => SQUARE_ASPECT,
        }
    }

    fn is_round(&self) -> bool {
        matches!(self, Self::Avatar)
    }

    /// The sample picture's width in window pixels: the largest place it shows under the stage.
    /// Its height follows from the aspect.
    fn sample_width(&self) -> f32 {
        match self {
            Self::ServerIcon | Self::ChannelIcon { .. } => COMPONENT_RAIL_ITEM_SIZE,
            Self::Avatar => COMPONENT_PROFILE_POPOVER_AVATAR_SIZE,
            Self::Banner { .. } => COMPONENT_PROFILE_POPOVER_WIDTH,
        }
    }
}

/// The picture under the crop, decoded once for the stage and the samples.
pub struct ChosenPicture {
    pub bytes: Arc<Vec<u8>>,
    size: PictureSize,
    still: Arc<RenderImage>,
}

/// Measures and decodes a picked picture for the stage, fitted in its square so a panorama
/// stays within the texture atlas's limits; refuses what the instance would refuse for its size.
/// Blocking: call it off the UI thread.
pub fn prepare_picture(bytes: Vec<u8>) -> Result<ChosenPicture, PictureReadProblem> {
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

/// The crop of the stage's still picture, `sample_width` device pixels wide and of the crop's
/// shape, for the samples. Reads the decoded frame in place (no copy of the whole picture) and
/// scales only the crop.
fn cropped_still(
    picture: &ChosenPicture,
    crop: CropArea,
    aspect: f32,
    sample_width: f32,
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
    let (x, y, cut_width, cut_height) =
        crop_in_still(crop, aspect, picture.size, (still_width, still_height));
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (sample_width, sample_height) = (
        sample_width.round().max(1.0) as u32,
        (sample_width / aspect).round().max(1.0) as u32,
    );
    let cut = image::imageops::crop_imm(&still, x, y, cut_width, cut_height);
    let sample = image::imageops::thumbnail(&*cut, sample_width, sample_height);
    Some(RenderImage::new(vec![image::Frame::new(sample)]))
}

/// Frees `image`'s tiles in every window's atlas. Deferred, as `voice/video_view.rs` does: the
/// window whose handler is running is out of the app's window list until it returns.
fn free_atlas_tiles(image: Arc<RenderImage>, cx: &mut App) {
    cx.defer(move |cx| cx.drop_image(image, None));
}

/// Opens the crop dialog on `picture`: the dialog to draw and listen to (Submitted, Closed), its
/// stage to read the crop from, and the subscription making Enter on the stage save.
pub fn open_crop_dialog<Owner: 'static>(
    picture: ChosenPicture,
    kind: CropKind,
    window: &mut Window,
    cx: &mut Context<Owner>,
) -> (Entity<ModalDialog>, Entity<CropStage>, Subscription) {
    let catalogue = strings(cx);
    let icon_words = catalogue.icon_editor().crop();
    let (title, subtitle, save) = match kind {
        CropKind::ServerIcon | CropKind::ChannelIcon { .. } => {
            (icon_words.title(), icon_words.subtitle(), icon_words.save())
        }
        CropKind::Avatar => {
            let words = catalogue.profile().avatar().crop();
            (words.title(), words.subtitle(), words.save())
        }
        CropKind::Banner { .. } => {
            let words = catalogue.profile().banner().crop();
            (words.title(), words.subtitle(), words.save())
        }
    };
    let stage = cx.new(|cx| CropStage::new(picture, kind, cx));
    let dialog = cx.new(|cx| {
        ModalDialog::new(title, catalogue.common().cancel(), window, cx)
            .subtitle(subtitle)
            .body(stage.clone().into())
            .submit(save, icon_words.saving(), false)
    });
    // The stage takes focus, as the web's `data-initial-focus`: its keys move the picture.
    let stage_focus = stage.read(cx).focus.clone();
    window.focus(&stage_focus, cx);
    let submitted_dialog = dialog.downgrade();
    let enter = cx.subscribe(&stage, move |_, _, _: &CropStageEvent, cx| {
        let _ = submitted_dialog.update(cx, ModalDialog::submit_now);
    });
    (dialog, stage, enter)
}

/// Enter on the stage: the dialog saves, as its button would.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropStageEvent;

/// The crop dialog's body: the stage, the zoom, and the samples at their real size.
pub struct CropStage {
    picture: ChosenPicture,
    kind: CropKind,
    crop: CropArea,
    focus: FocusHandle,
    /// The frame as last drawn: how many window pixels a picture pixel takes.
    frame_bounds: Entity<Option<Bounds<Pixels>>>,
    /// Where the pointer was at the last drag step.
    drag_from: Option<Point<Pixels>>,
    /// The crop alone, small, for the samples: GPUI rounds a picture's own corners but clips
    /// its children to rectangles, so a rounded sample needs the cropped picture itself.
    sample: Option<(CropArea, Arc<RenderImage>)>,
}

impl EventEmitter<CropStageEvent> for CropStage {}

impl CropStage {
    fn new(picture: ChosenPicture, kind: CropKind, cx: &mut Context<Self>) -> Self {
        let crop = largest_centred_crop(picture.size, kind.aspect());
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
            kind,
            crop,
            focus: cx.focus_handle().tab_stop(true),
            frame_bounds: cx.new(|_| None),
            drag_from: None,
            sample: None,
        }
    }

    fn aspect(&self) -> f32 {
        self.kind.aspect()
    }

    /// The sample picture for the current crop, cut again when the crop moved.
    fn sample_picture(&mut self, window: &mut Window) -> Option<Arc<RenderImage>> {
        if let Some((crop, sample)) = &self.sample
            && *crop == self.crop
        {
            return Some(Arc::clone(sample));
        }
        let width = self.kind.sample_width();
        let sample = Arc::new(cropped_still(
            &self.picture,
            self.crop,
            self.aspect(),
            width * window.scale_factor(),
        )?);
        if let Some((_, previous)) = self.sample.replace((self.crop, Arc::clone(&sample))) {
            let _ = window.drop_image(previous);
        }
        Some(sample)
    }

    /// Whole pixels for the query, or `None` for the centred part the instance keeps itself.
    pub fn crop_to_send(&self) -> Option<SentCrop> {
        crop_to_send(self.crop, self.picture.size, self.aspect())
    }

    fn zoom(&self) -> f32 {
        zoom_of(self.crop, self.picture.size, self.aspect())
    }

    /// Window pixels per picture pixel inside the frame.
    fn scale(&self, cx: &App) -> Option<(Bounds<Pixels>, f32)> {
        let bounds = (*self.frame_bounds.read(cx))?;
        let width = f32::from(bounds.size.width);
        (width > 0.0).then(|| (bounds, width / self.crop.width))
    }

    fn point_in_picture(&self, position: Point<Pixels>, cx: &App) -> Option<(f32, f32)> {
        let (bounds, scale) = self.scale(cx)?;
        Some((
            self.crop.x + f32::from(position.x - bounds.origin.x) / scale,
            self.crop.y + f32::from(position.y - bounds.origin.y) / scale,
        ))
    }

    fn set_crop(&mut self, crop: CropArea, cx: &mut Context<Self>) {
        if crop != self.crop {
            self.crop = crop;
            cx.notify();
        }
    }

    fn move_by(&mut self, delta_x: f32, delta_y: f32, cx: &mut Context<Self>) {
        let crop = crop_moved_by(
            self.crop,
            self.picture.size,
            self.aspect(),
            delta_x,
            delta_y,
        );
        self.set_crop(crop, cx);
    }

    fn zoom_to(&mut self, zoom: f32, anchor: Option<(f32, f32)>, cx: &mut Context<Self>) {
        let crop = crop_zoomed_to(self.crop, self.picture.size, self.aspect(), zoom, anchor);
        self.set_crop(crop, cx);
    }

    fn centre(&mut self, cx: &mut Context<Self>) {
        let centred = largest_centred_crop(self.picture.size, self.aspect());
        self.set_crop(centred, cx);
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
        self.drag_from = Some(event.position);
        // Dragging the picture right shows more of its left: the crop moves the other way.
        self.move_by(
            -f32::from(event.position.x - from.x) / scale,
            -f32::from(event.position.y - from.y) / scale,
            cx,
        );
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
        if keystroke.key == "enter" && !keystroke.modifiers.shift {
            cx.emit(CropStageEvent);
            cx.stop_propagation();
            return;
        }
        let share = if keystroke.modifiers.shift {
            KEYBOARD_LARGE_STEP_SHARE
        } else {
            KEYBOARD_STEP_SHARE
        };
        let step_x = self.crop.width * share;
        let step_y = self.crop.height(self.aspect()) * share;
        // The keys move the picture, as dragging does.
        let movement = match keystroke.key.as_str() {
            "left" => Some((step_x, 0.0)),
            "right" => Some((-step_x, 0.0)),
            "up" => Some((0.0, step_y)),
            "down" => Some((0.0, -step_y)),
            _ => None,
        };
        if let Some((delta_x, delta_y)) = movement {
            self.move_by(delta_x, delta_y, cx);
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

    /// The whole picture in a box `frame_width` wide, placed so the crop fills it.
    fn cropped_picture(&self, frame_width: f32) -> impl IntoElement {
        let (left, top, width, height) =
            placement_in_frame(self.crop, self.picture.size, frame_width);
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

    /// The top of the profile card (`ProfileCard.svelte`): the band, then the avatar in its
    /// ring covering part of it, as others will see it.
    fn banner_in_card(
        picture: Option<&Arc<RenderImage>>,
        person: &AvatarPerson,
        cx: &App,
    ) -> gpui::Div {
        let tokens = theme(cx);
        let surface = color(tokens.component_profile_popover_background);
        let radius = length(COMPONENT_PROFILE_POPOVER_RADIUS);
        let ring = COMPONENT_PROFILE_POPOVER_AVATAR_RING_WIDTH;
        div()
            .w_full()
            .max_w(length(COMPONENT_PROFILE_POPOVER_WIDTH))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(radius)
            .border(length(COMPONENT_PROFILE_POPOVER_BORDER_WIDTH))
            .border_color(color(tokens.component_profile_popover_border))
            .bg(surface)
            .child(
                div()
                    .w_full()
                    .h(length(COMPONENT_PROFILE_POPOVER_BANNER_IMAGE_HEIGHT))
                    .bg(color(tokens.component_profile_popover_banner))
                    .when_some(picture, |band, picture| {
                        // GPUI clips children to rectangles: the picture rounds its own corners.
                        band.child(
                            img(gpui::ImageSource::Render(Arc::clone(picture)))
                                .size_full()
                                .rounded_t(radius),
                        )
                    }),
            )
            .child(
                div()
                    .px(length(COMPONENT_PROFILE_POPOVER_PADDING))
                    .pb(length(COMPONENT_PROFILE_POPOVER_PADDING))
                    .child(
                        div()
                            .size(length(COMPONENT_PROFILE_POPOVER_AVATAR_SIZE + 2.0 * ring))
                            .mt(length(
                                COMPONENT_PROFILE_POPOVER_TEXT_GAP
                                    - COMPONENT_PROFILE_POPOVER_AVATAR_SIZE / 2.0
                                    - ring,
                            ))
                            .ml(-length(ring))
                            .p(length(ring))
                            .rounded_full()
                            .bg(surface)
                            .child(Avatar::new(
                                person.clone(),
                                COMPONENT_PROFILE_POPOVER_AVATAR_SIZE,
                            )),
                    ),
            )
    }

    fn channel_rows(picture: Option<&Arc<RenderImage>>, name: &str, cx: &App) -> gpui::Div {
        let tokens = theme(cx);
        div()
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
                            .child(name.to_owned()),
                    )
            }))
    }

    fn render_samples(&mut self, window: &mut Window, cx: &App) -> gpui::Div {
        let tokens = theme(cx);
        let picture = self.sample_picture(window);
        let picture = picture.as_ref();
        let catalogue = strings(cx);
        let words = catalogue.icon_editor().crop();
        let samples = match &self.kind {
            // The strip's two shapes: a circle at rest, the squircle when hovered or selected.
            CropKind::ServerIcon => div()
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
            CropKind::ChannelIcon { name } => Self::channel_rows(picture, name, cx),
            // The largest and the most frequent places an avatar shows.
            CropKind::Avatar => {
                let avatar_words = catalogue.profile().avatar().crop();
                div()
                    .flex()
                    .items_end()
                    .gap(length(COMPONENT_ICON_PICKER_SAMPLE_GAP))
                    .child(Self::sample(
                        avatar_words.sample_profile(),
                        picture,
                        COMPONENT_PROFILE_POPOVER_AVATAR_SIZE,
                        COMPONENT_PROFILE_POPOVER_AVATAR_SIZE / 2.0,
                        cx,
                    ))
                    .child(Self::sample(
                        avatar_words.sample_messages(),
                        picture,
                        COMPONENT_MESSAGE_AVATAR_SIZE,
                        COMPONENT_MESSAGE_AVATAR_SIZE / 2.0,
                        cx,
                    ))
            }
            // The profile card itself, its avatar covering part of the band.
            CropKind::Banner { person } => Self::banner_in_card(picture, person, cx),
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
        let (frame_width, frame_height) = self.frame_bounds.read(cx).map_or((0.0, 0.0), |bounds| {
            (f32::from(bounds.size.width), f32::from(bounds.size.height))
        });
        let aspect = self.aspect();
        let is_round = self.kind.is_round();
        let zoom = self.zoom();
        let zoom_limit = maximum_zoom(self.picture.size, aspect);
        let is_ring_shown = is_focus_visible(&self.focus, window);
        let scrim = color(tokens.component_icon_picker_outside_scrim);
        let inset = length(COMPONENT_ICON_PICKER_FRAME_INSET);
        let stage_radius = length(COMPONENT_ICON_PICKER_STAGE_RADIUS);
        let stage = cx.entity().downgrade();
        let centre_stage = stage.clone();

        // The outside of the frame, dimmed. A circle: a ring around the frame, its inner edge the
        // frame and its outer edge past the stage's corners (GPUI rounds a border's inner edge by
        // the radius less its width). Other shapes: four bands around the frame.
        let ring = frame_width;
        let round_outside = is_round.then(|| {
            div()
                .absolute()
                .top(px(-ring))
                .left(px(-ring))
                .size(px(frame_width + 2.0 * ring))
                .border(px(ring))
                .rounded(px(frame_width / 2.0 + ring))
                .border_color(scrim)
        });
        let outside_bands: Vec<gpui::Div> = if is_round {
            Vec::new()
        } else {
            vec![
                div().absolute().top_0().left_0().right_0().h(inset),
                div().absolute().bottom_0().left_0().right_0().h(inset),
                div().absolute().top(inset).bottom(inset).left_0().w(inset),
                div().absolute().top(inset).bottom(inset).right_0().w(inset),
            ]
            .into_iter()
            .map(|band| band.bg(scrim))
            .collect()
        };

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
            .rounded(stage_radius)
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
                    .rounded(stage_radius)
                    .child(
                        div()
                            .absolute()
                            .top(inset)
                            .left(inset)
                            .w(px(frame_width))
                            .h(px(frame_height))
                            .when(frame_width > 0.0, |frame| {
                                frame.child(self.cropped_picture(frame_width))
                            })
                            .children(round_outside),
                    )
                    .children(outside_bands)
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
                // The frame, of the kept part's shape and as wide as the stage allows; its
                // bounds size the picture.
                div()
                    .relative()
                    .w_full()
                    .aspect_ratio(aspect)
                    .border(length(COMPONENT_ICON_PICKER_FRAME_BORDER_WIDTH))
                    .border_color(color(tokens.component_icon_picker_frame_border))
                    .when(is_round, gpui::Styled::rounded_full)
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
                                let _ = centre_stage.update(cx, Self::centre);
                            }),
                    ),
            )
            .child(self.render_samples(window, cx))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{TestAppContext, VisualTestContext, WindowAppearance};

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

    fn focused_stage(
        picture: ChosenPicture,
        kind: CropKind,
        cx: &mut TestAppContext,
    ) -> (Entity<CropStage>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            cx.set_global(crate::theme::ActiveTheme::for_appearance(
                WindowAppearance::Dark,
            ));
        });
        let (stage, cx) = cx.add_window_view(|_, cx| CropStage::new(picture, kind, cx));
        cx.update(|window, cx| {
            window.activate_window();
            let focus = stage.read(cx).focus.clone();
            window.focus(&focus, cx);
        });
        cx.run_until_parked();
        (stage, cx)
    }

    #[gpui::test]
    fn the_keys_move_and_zoom_the_crop_as_dragging_does(cx: &mut TestAppContext) {
        let (stage, cx) = focused_stage(picture_of(400, 200), CropKind::ServerIcon, cx);
        let crop = |cx: &mut VisualTestContext| stage.read_with(cx, |stage, _| stage.crop);
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
        stage.update(cx, CropStage::centre);
        assert_eq!(stage.read_with(cx, |stage, _| stage.crop_to_send()), None);
    }

    #[gpui::test]
    fn a_banner_moves_within_its_band_and_enter_asks_to_save(cx: &mut TestAppContext) {
        let person = AvatarPerson {
            initial: "P".into(),
            tint_number: 1,
            picture: None,
        };
        let (stage, cx) = focused_stage(picture_of(1000, 1000), CropKind::Banner { person }, cx);
        let saved = Rc::new(Cell::new(0));
        let counted = Rc::clone(&saved);
        let _subscription = cx.update(|_, cx| {
            cx.subscribe(&stage, move |_, _: &CropStageEvent, _| {
                counted.set(counted.get() + 1);
            })
        });
        // The 1000 × 400 band, centred; Down shows more of the top, 2 % of its 400 px height.
        assert_eq!(stage.read_with(cx, |stage, _| stage.crop.y), 300.0);
        cx.simulate_keystrokes("down");
        assert!((stage.read_with(cx, |stage, _| stage.crop.y) - 292.0).abs() < 0.001);
        assert_eq!(
            stage.read_with(cx, |stage, _| stage.crop_to_send()),
            Some(SentCrop {
                x: 0,
                y: 292,
                width: 1000
            })
        );
        cx.simulate_keystrokes("enter");
        assert_eq!(saved.get(), 1);
    }
}
