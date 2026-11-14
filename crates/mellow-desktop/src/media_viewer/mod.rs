//! The media viewer (6.76, `PictureViewer.svelte`, docs/design/web-picture-viewer.md): a message's
//! pictures and videos large over the app, in the opaque dark room of the media stage, under the
//! title band (`client_content_bounds`), with a band of controls under it: the file name and
//! "Ana · Today at 14:32 · 3.4 MB", "2 / 4" between previous and next (Left and Right, wrapping),
//! then download, copy link, open the original in the browser and close.
//!
//! A picture is fitted and never grown past its own pixels; the list's preview shows at once
//! while the full file loads, "Loading the picture…" only after 340 ms, and a failure says so with
//! "Try again". A picture larger than the room toggles to its actual size (a click keeps the
//! clicked point under the pointer, the keyboard centres it); there the wheel, a drag and the
//! arrows move it, Ctrl+wheel down goes back to fit, and the wheel up at fit zooms in. A video
//! fills the room with its controls over it (`messages::video_player`); its full screen is the
//! system's, the window filling the screen with the room in it.
//!
//! Rendered once by the shell (`media_viewer`), outside any message: inside a row, its clicks and
//! keys would reach the row's handlers, and a deleted message would take it along. Esc, "Close"
//! or a click on the room close it, focus going back to the picture or button that opened it.
//! Tab stays inside. The room fades in and the picture grows from 96 %, at once under reduced
//! motion.

pub(crate) mod geometry;

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, Bounds, ClickEvent,
    ClipboardItem, Context, CursorStyle, Entity, FocusHandle, Global, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ObjectFit, ParentElement as _, Pixels, Point, Render, Role, ScrollEvent, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Task, Toggled, Window,
    anchored, canvas, deferred, div, img, prelude::FluentBuilder as _,
};
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::format::file_name_start_and_end;
use mellow_media::PLAYABLE_ATTACHMENT_VIDEO_TYPES;
use mellow_protocol::{Attachment, Snowflake};

use self::geometry::{
    FALLBACK_VIDEO_SIZE, PixelSize, actual_size_scroll_start, fitted_picture_size,
    fitted_video_size, is_larger_than, maximum_scroll, picture_start, wrapped_index,
};
use crate::attachment_audio::{
    AudioPlayback, attachment_audio_view, attachment_rest_client, is_video_played_in_app,
};
use crate::attachment_download::{
    OpenOutcome, SaveOutcome, open_attachment_with_default_app, open_outcome, opened_extension,
    save_attachment, save_outcome,
};
use crate::components::{
    FocusNextControl, FocusPreviousControl, IconButton, Tooltip, easing, focus_next_inside,
    focus_previous_inside, focus_ring_in, icon, is_focus_visible, polite_live_region, with_type,
};
use crate::i18n;
use crate::messages::video_player::{VideoScreen, pause_video, run_video_key, video_key};
use crate::messages::{
    PREVIEWABLE_PICTURE_TYPES, TOKEN_PIXELS_PER_REM, file_size_label, local_date_time,
    message_timestamp_label, picture_animation, picture_box, picture_request_for_box,
};
use crate::pictures::{
    PictureRequest, PictureStatus, picture_source, picture_status, retry_picture,
};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_PICTURE_VIEWER_BAND_GAP, COMPONENT_PICTURE_VIEWER_BAND_MIN_HEIGHT,
    COMPONENT_PICTURE_VIEWER_BAND_PADDING_BLOCK, COMPONENT_PICTURE_VIEWER_BAND_PADDING_INLINE,
    COMPONENT_PICTURE_VIEWER_BAND_ROW_GAP, COMPONENT_PICTURE_VIEWER_CAPTION_MIN_WIDTH,
    COMPONENT_PICTURE_VIEWER_CONTROL_GAP, COMPONENT_PICTURE_VIEWER_CONTROL_ICON_SIZE,
    COMPONENT_PICTURE_VIEWER_CONTROL_RADIUS, COMPONENT_PICTURE_VIEWER_CONTROL_SIZE,
    COMPONENT_PICTURE_VIEWER_COPIED_LABEL_DURATION, COMPONENT_PICTURE_VIEWER_ENTER_SCALE,
    COMPONENT_PICTURE_VIEWER_PICTURE_RADIUS, COMPONENT_PICTURE_VIEWER_RETRY_BORDER_WIDTH,
    COMPONENT_PICTURE_VIEWER_STAGE_PADDING, COMPONENT_PICTURE_VIEWER_STATUS_DELAY,
    COMPONENT_PICTURE_VIEWER_STATUS_GAP, COMPONENT_PICTURE_VIEWER_STATUS_ICON_SIZE,
    COMPONENT_PICTURE_VIEWER_STATUS_PADDING, COMPONENT_PICTURE_VIEWER_STATUS_RADIUS,
    MOTION_DURATION_BASE, MOTION_EASING_ENTER, TYPE_BODY, TYPE_BODY_LG, TYPE_BODY_SM, TYPE_BUTTON,
};

use crate::theme::{animation_duration, color, delay, length, theme};

/// What the arrow keys move a picture shown at its own size by, as a browser scrolls a page.
const ARROW_KEY_PAN: f32 = 40.0;
/// How far a press may travel and still be a click rather than a drag, in token pixels.
const CLICK_SLOP: f32 = 4.0;
/// The layer above dialogs and the enlarged overlays of the shell.
const VIEWER_LAYER_PRIORITY: usize = 3;

/// The pictures and videos of one message, with who sent them and when.
#[derive(Debug, Clone)]
pub(crate) struct ViewerSet {
    pub(crate) media: Rc<[Attachment]>,
    pub(crate) author_name: SharedString,
    pub(crate) sent_at_unix_milliseconds: u64,
}

impl ViewerSet {
    /// The viewer's set of a message: its pictures and the videos it plays, in message order.
    pub(crate) fn of_message(
        attachments: &[Attachment],
        author_name: impl Into<SharedString>,
        sent_at_unix_milliseconds: u64,
    ) -> Self {
        Self {
            media: attachments
                .iter()
                .filter(|attachment| is_viewer_picture(attachment) || is_viewer_video(attachment))
                .cloned()
                .collect(),
            author_name: author_name.into(),
            sent_at_unix_milliseconds,
        }
    }
}

/// A picture the instance serves for showing (ADR-0020).
#[must_use]
pub(crate) fn is_viewer_picture(attachment: &Attachment) -> bool {
    PREVIEWABLE_PICTURE_TYPES.contains(&attachment.content_type.as_str())
}

/// A video the list plays, so the viewer can too.
#[must_use]
pub(crate) fn is_viewer_video(attachment: &Attachment) -> bool {
    PLAYABLE_ATTACHMENT_VIDEO_TYPES.contains(&attachment.content_type.as_str())
        && is_video_played_in_app(attachment)
}

/// The viewer open now, drawn by the shell.
#[derive(Default)]
struct MediaViewerHost(Option<Entity<MediaViewer>>);

impl Global for MediaViewerHost {}

/// The open viewer, for the shell to draw over everything.
#[must_use]
pub(crate) fn open_media_viewer_view(cx: &App) -> Option<Entity<MediaViewer>> {
    cx.try_global::<MediaViewerHost>()?.0.clone()
}

/// Whether a layer covering the whole shell is open (the media viewer): the shell's own
/// shortcuts (channels, servers, search, the switcher, settings) wait until it closes, since
/// GPUI runs key bindings before the layer's key handler.
#[must_use]
pub(crate) fn is_modal_layer_open(cx: &App) -> bool {
    open_media_viewer_view(cx).is_some()
}

/// Forgets the open viewer: its window or its session is gone.
pub(crate) fn forget_media_viewer(cx: &mut App) {
    if cx.has_global::<MediaViewerHost>() {
        cx.set_global(MediaViewerHost(None));
    }
    if cx.has_global::<ViewerFullscreen>() {
        cx.set_global(ViewerFullscreen::default());
    }
}

/// The system's full screen the viewer asked for. Platforms grant it later (Wayland, macOS'
/// animation), so the viewer follows its intent rather than `is_fullscreen`: leaving before the
/// window got there is done once it does.
#[derive(Debug, Default)]
struct ViewerFullscreen {
    is_made_by_viewer: bool,
    should_leave: bool,
}

impl Global for ViewerFullscreen {}

/// Whether the window is full screen (or getting there) because the viewer asked: its bounds are
/// not remembered for the next window.
#[must_use]
pub(crate) fn is_window_fullscreen_by_viewer(cx: &App) -> bool {
    cx.try_global::<ViewerFullscreen>()
        .is_some_and(|fullscreen| fullscreen.is_made_by_viewer)
}

fn enter_viewer_fullscreen(window: &mut Window, cx: &mut App) {
    if window.is_fullscreen() {
        return;
    }
    let fullscreen = cx.default_global::<ViewerFullscreen>();
    fullscreen.is_made_by_viewer = true;
    fullscreen.should_leave = false;
    window.toggle_fullscreen();
}

fn leave_viewer_fullscreen(window: &mut Window, cx: &mut App) {
    let fullscreen = cx.default_global::<ViewerFullscreen>();
    if !fullscreen.is_made_by_viewer {
        return;
    }
    if window.is_fullscreen() {
        *fullscreen = ViewerFullscreen::default();
        window.toggle_fullscreen();
    } else {
        // Still on its way: left as soon as it arrives (`follow_window_fullscreen`).
        fullscreen.should_leave = true;
    }
}

/// The window's bounds changed: a full screen the viewer no longer wants is left now.
pub(crate) fn follow_window_fullscreen(window: &mut Window, cx: &mut App) {
    let Some(fullscreen) = cx.try_global::<ViewerFullscreen>() else {
        return;
    };
    if fullscreen.should_leave && window.is_fullscreen() {
        cx.set_global(ViewerFullscreen::default());
        window.toggle_fullscreen();
    }
}

/// Opens the viewer on `attachment_id` of `set`; focus goes back to `opener` when it closes.
/// `is_fullscreen_wanted` (F on a video) fills the screen with the window first.
pub(crate) fn open_media_viewer(
    set: &ViewerSet,
    attachment_id: Snowflake,
    opener: Option<FocusHandle>,
    is_fullscreen_wanted: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(index) = set
        .media
        .iter()
        .position(|attachment| attachment.id == attachment_id)
    else {
        return;
    };
    let set = set.clone();
    let viewer =
        cx.new(|cx| MediaViewer::new(set, index, opener, is_fullscreen_wanted, window, cx));
    cx.set_global(MediaViewerHost(Some(viewer)));
    window.refresh();
}

/// A press on the picture that may become a drag.
#[derive(Debug, Clone, Copy)]
struct PictureDrag {
    pointer_start: Point<Pixels>,
    scroll_start: Point<f32>,
    has_moved: bool,
}

/// Where the shown picture sits and how large, in token pixels.
#[derive(Debug, Clone, Copy)]
struct PictureLayout {
    /// The room under the band.
    stage: PixelSize,
    /// The room inside its padding.
    area: PixelSize,
    /// Its own size, when the instance said.
    natural: Option<PixelSize>,
    is_zoomable: bool,
}

#[allow(clippy::struct_excessive_bools)] // Independent states of one viewer, as the web's.
pub(crate) struct MediaViewer {
    set: ViewerSet,
    index: usize,
    opener: Option<FocusHandle>,
    container_focus: FocusHandle,
    close_focus: FocusHandle,
    picture_focus: FocusHandle,
    video_focus: FocusHandle,
    retry_focus: FocusHandle,
    /// The shown picture's last full-size frame ready, kept under a new size while that loads
    /// (actual size, a resized window), so the picture stays sharp instead of the preview's.
    last_ready_full: Option<PictureRequest>,
    is_actual_size: bool,
    /// How far the room is scrolled at actual size, in token pixels.
    scroll: Point<f32>,
    drag: Option<PictureDrag>,
    /// Whether the last press began on the picture, so its release beside it does not close.
    is_press_on_picture: bool,
    /// The room as last drawn, in window pixels.
    stage_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    loading_since: Option<Instant>,
    loading_notice_timer: Option<Task<()>>,
    is_copied: bool,
    copied_timer: Option<Task<()>>,
    announcement: SharedString,
}

impl MediaViewer {
    fn new(
        set: ViewerSet,
        index: usize,
        opener: Option<FocusHandle>,
        is_fullscreen_wanted: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let close_focus = cx.focus_handle().tab_stop(true);
        let video_focus = cx.focus_handle().tab_stop(true);
        // Close first (the dialog's name is read, Enter gets out); a video's own screen, so its
        // keys work at once.
        let is_video = set.media.get(index).is_some_and(is_viewer_video);
        window.focus(if is_video { &video_focus } else { &close_focus }, cx);
        if is_fullscreen_wanted {
            enter_viewer_fullscreen(window, cx);
        }
        Self {
            set,
            index,
            opener,
            container_focus: cx.focus_handle(),
            close_focus,
            picture_focus: cx.focus_handle().tab_stop(true),
            video_focus,
            retry_focus: cx.focus_handle().tab_stop(true),
            last_ready_full: None,
            is_actual_size: false,
            scroll: Point::default(),
            drag: None,
            is_press_on_picture: false,
            stage_bounds: Rc::new(Cell::new(None)),
            loading_since: None,
            loading_notice_timer: None,
            is_copied: false,
            copied_timer: None,
            announcement: SharedString::default(),
        }
    }

    /// Whether focus is inside the viewer.
    #[cfg(test)]
    pub(crate) fn has_focus_within(&self, window: &Window, cx: &App) -> bool {
        self.container_focus.contains_focused(window, cx)
    }

    fn shown(&self) -> Option<&Attachment> {
        self.set.media.get(self.index)
    }

    /// Token pixels per window pixel's worth of zoom.
    fn zoom(window: &Window) -> f32 {
        f32::from(window.rem_size()) / TOKEN_PIXELS_PER_REM
    }

    fn layout(&self, window: &Window) -> PictureLayout {
        let zoom = Self::zoom(window);
        let stage = self.stage_bounds.get().map_or_else(
            || {
                // Before the first frame: the window's content above the band.
                let content = crate::window_chrome::client_content_bounds(window).size;
                PixelSize {
                    width: f32::from(content.width) / zoom,
                    height: f32::from(content.height) / zoom
                        - COMPONENT_PICTURE_VIEWER_BAND_MIN_HEIGHT,
                }
            },
            |bounds| PixelSize {
                width: f32::from(bounds.size.width) / zoom,
                height: f32::from(bounds.size.height) / zoom,
            },
        );
        let area = PixelSize {
            width: (stage.width - 2.0 * COMPONENT_PICTURE_VIEWER_STAGE_PADDING).max(1.0),
            height: (stage.height - 2.0 * COMPONENT_PICTURE_VIEWER_STAGE_PADDING).max(1.0),
        };
        let shown = self.shown();
        let natural = shown.and_then(pixel_size);
        let is_zoomable = shown.is_some_and(is_viewer_picture)
            && natural.is_some_and(|natural| is_larger_than(natural, area));
        PictureLayout {
            stage,
            area,
            natural,
            is_zoomable,
        }
    }

    /// Previous or next, wrapping; a video left behind pauses.
    fn step(&mut self, is_forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let index = wrapped_index(self.index, is_forward, self.set.media.len());
        self.show(index, window, cx);
    }

    fn show(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(previous) = self.shown().cloned().filter(is_viewer_video) {
            pause_video(&previous, cx);
        }
        self.index = index;
        self.last_ready_full = None;
        self.is_actual_size = false;
        self.scroll = Point::default();
        self.drag = None;
        self.is_copied = false;
        self.copied_timer = None;
        self.loading_since = None;
        self.loading_notice_timer = None;
        if let Some(shown) = self.shown() {
            self.announcement = i18n::strings(cx)
                .picture_viewer()
                .picture_changed(
                    count(self.index + 1),
                    count(self.set.media.len()),
                    &shown.file_name,
                )
                .into();
        }
        // The zoom toggle or the video's screen may be gone: focus would leave the viewer.
        if self.picture_focus.is_focused(window) || self.video_focus.is_focused(window) {
            window.focus(&self.close_focus, cx);
        }
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(shown) = self.shown().cloned().filter(is_viewer_video) {
            pause_video(&shown, cx);
        }
        leave_viewer_fullscreen(window, cx);
        if let Some(opener) = self.opener.as_ref() {
            window.focus(opener, cx);
        }
        cx.set_global(MediaViewerHost(None));
        window.refresh();
    }

    fn toggle_fullscreen(window: &mut Window, cx: &mut Context<Self>) {
        if is_window_fullscreen_by_viewer(cx) {
            leave_viewer_fullscreen(window, cx);
        } else if window.is_fullscreen() {
            // The person's own full screen: F leaves it, as a player's button does.
            window.toggle_fullscreen();
        } else {
            enter_viewer_fullscreen(window, cx);
        }
        cx.notify();
    }

    /// Fit or actual size; from a pointer, the point under it stays there; from the keyboard,
    /// the picture's centre is centred.
    fn toggle_actual_size(
        &mut self,
        pointer: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let layout = self.layout(window);
        let Some(natural) = layout.natural.filter(|_| layout.is_zoomable) else {
            return;
        };
        cx.notify();
        if self.is_actual_size {
            self.is_actual_size = false;
            self.scroll = Point::default();
            return;
        }
        let zoom = Self::zoom(window);
        let fitted = fitted_picture_size(natural, layout.area);
        let padding = COMPONENT_PICTURE_VIEWER_STAGE_PADDING;
        let origin = self
            .stage_bounds
            .get()
            .map_or_else(Point::default, |bounds| bounds.origin);
        let axis = |pointer_at: Option<Pixels>,
                    origin_at: Pixels,
                    stage: f32,
                    fitted: f32,
                    natural: f32| {
            let (ratio, offset) = pointer_at.map_or((0.5, stage / 2.0), |pointer_at| {
                let offset = f32::from(pointer_at - origin_at) / zoom;
                let start = picture_start(fitted, padding, stage, 0.0);
                (((offset - start) / fitted).clamp(0.0, 1.0), offset)
            });
            actual_size_scroll_start(ratio, natural, padding, offset)
                .min(maximum_scroll(natural, padding, stage))
        };
        self.is_actual_size = true;
        self.scroll = Point {
            x: axis(
                pointer.map(|pointer| pointer.x),
                origin.x,
                layout.stage.width,
                fitted.width,
                natural.width,
            ),
            y: axis(
                pointer.map(|pointer| pointer.y),
                origin.y,
                layout.stage.height,
                fitted.height,
                natural.height,
            ),
        };
    }

    /// Moves a picture shown at its own size, kept inside its bounds.
    fn scroll_by(&mut self, x: f32, y: f32, window: &Window, cx: &mut Context<Self>) {
        let layout = self.layout(window);
        let Some(natural) = layout.natural.filter(|_| self.is_actual_size) else {
            return;
        };
        let padding = COMPONENT_PICTURE_VIEWER_STAGE_PADDING;
        self.scroll = Point {
            x: (self.scroll.x + x).clamp(
                0.0,
                maximum_scroll(natural.width, padding, layout.stage.width),
            ),
            y: (self.scroll.y + y).clamp(
                0.0,
                maximum_scroll(natural.height, padding, layout.stage.height),
            ),
        };
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            cx.stop_propagation();
            // Out of full screen first, as players do; then out of the viewer.
            if is_window_fullscreen_by_viewer(cx) {
                leave_viewer_fullscreen(window, cx);
                cx.notify();
            } else {
                self.close(window, cx);
            }
            return;
        }
        if self.is_actual_size {
            let (x, y) = match key {
                "left" => (-ARROW_KEY_PAN, 0.0),
                "right" => (ARROW_KEY_PAN, 0.0),
                "up" => (0.0, -ARROW_KEY_PAN),
                "down" => (0.0, ARROW_KEY_PAN),
                _ => (0.0, 0.0),
            };
            if x != 0.0 || y != 0.0 {
                cx.stop_propagation();
                self.scroll_by(x, y, window, cx);
                return;
            }
        }
        if (key == "left" || key == "right") && self.set.media.len() > 1 {
            cx.stop_propagation();
            self.step(key == "right", window, cx);
            return;
        }
        // A video's keys work from anywhere in the viewer, not only its focused screen.
        if let Some(shown) = self.shown().cloned().filter(is_viewer_video)
            && let Some(video_key) = video_key(key, false)
        {
            cx.stop_propagation();
            if !run_video_key(&shown, video_key, cx) {
                Self::toggle_fullscreen(window, cx);
            }
        }
    }

    fn on_wheel(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        let layout = self.layout(window);
        if !layout.is_zoomable {
            return;
        }
        let zoom = Self::zoom(window);
        let delta = event.delta.pixel_delta(window.line_height());
        let (x, y) = (f32::from(delta.x) / zoom, f32::from(delta.y) / zoom);
        let is_zoom_gesture = event.modifiers.control || event.modifiers.platform;
        if !self.is_actual_size {
            // Nothing scrolls at fit: the wheel up (or Ctrl+wheel) zooms in where it points.
            if y > 0.0 {
                self.toggle_actual_size(Some(event.position), window, cx);
            }
            return;
        }
        if is_zoom_gesture {
            if y < 0.0 {
                self.toggle_actual_size(None, window, cx);
            }
            return;
        }
        self.scroll_by(-x, -y, window, cx);
    }

    fn on_picture_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.is_press_on_picture = true;
        if self.is_actual_size {
            self.drag = Some(PictureDrag {
                pointer_start: event.position,
                scroll_start: self.scroll,
                has_moved: false,
            });
        }
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            return;
        }
        let zoom = Self::zoom(window);
        let moved = event.position - drag.pointer_start;
        let (x, y) = (f32::from(moved.x) / zoom, f32::from(moved.y) / zoom);
        let has_moved = drag.has_moved || x.abs() > CLICK_SLOP || y.abs() > CLICK_SLOP;
        self.drag = Some(PictureDrag { has_moved, ..drag });
        if has_moved {
            let target = Point {
                x: drag.scroll_start.x - x,
                y: drag.scroll_start.y - y,
            };
            self.scroll_by(
                target.x - self.scroll.x,
                target.y - self.scroll.y,
                window,
                cx,
            );
        }
    }

    fn on_picture_click(
        &mut self,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        self.is_press_on_picture = false;
        let was_dragged = self.drag.take().is_some_and(|drag| drag.has_moved);
        if !was_dragged {
            self.toggle_actual_size(event.mouse_position(), window, cx);
        }
    }

    /// A click on the room closes, unless its press began on the picture (a drag past its edge).
    fn on_room_click(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.drag = None;
        if std::mem::take(&mut self.is_press_on_picture) {
            return;
        }
        self.close(window, cx);
    }

    fn copy_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(link) = self.shown().and_then(|shown| attachment_link(shown.id, cx)) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(link));
        self.is_copied = true;
        // Emptied for a frame, so a second copy is announced again.
        self.announcement = SharedString::default();
        let copied: SharedString = i18n::strings(cx).picture_viewer().link_copied().into();
        let viewer = cx.entity().downgrade();
        window.on_next_frame(move |_, cx| {
            let _ = viewer.update(cx, |viewer, cx| {
                viewer.announcement = copied;
                cx.notify();
            });
        });
        let wait = delay(COMPONENT_PICTURE_VIEWER_COPIED_LABEL_DURATION);
        self.copied_timer = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |viewer, cx| {
                viewer.is_copied = false;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Whether the loading notice is due: `is_loading` for the status delay. A picture already in
    /// the cache never shows it.
    fn is_loading_notice_due(
        &mut self,
        is_loading: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !is_loading {
            self.loading_since = None;
            self.loading_notice_timer = None;
            return false;
        }
        let now = cx.background_executor().now();
        let since = *self.loading_since.get_or_insert(now);
        let wait = delay(COMPONENT_PICTURE_VIEWER_STATUS_DELAY);
        let waited = now.saturating_duration_since(since);
        if self.loading_notice_timer.is_none() && waited < wait {
            let remaining = wait.saturating_sub(waited);
            self.loading_notice_timer = Some(cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(remaining).await;
                let _ = this.update(cx, |_, cx| cx.notify());
            }));
        }
        waited >= wait
    }
}

fn count(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// An attachment's stored size, when the instance gave one.
fn pixel_size(attachment: &Attachment) -> Option<PixelSize> {
    match (attachment.width, attachment.height) {
        #[allow(clippy::cast_precision_loss)] // Picture sides are far below 2^24.
        (Some(width), Some(height)) if width > 0 && height > 0 => Some(PixelSize {
            width: width as f32,
            height: height as f32,
        }),
        _ => None,
    }
}

/// The absolute link of an attachment on the signed-in instance, which works for any member.
fn attachment_link(attachment_id: Snowflake, cx: &App) -> Option<String> {
    let rest = attachment_rest_client(cx)?;
    Some(format!(
        "{}api/v1/attachments/{attachment_id}",
        rest.instance().as_str()
    ))
}

/// What the stage says under the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StageNotice {
    Loading,
    LoadingVideo,
    FullSizeFailed,
    PictureFailed,
    VideoFailed,
}

impl Render for MediaViewer {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content_bounds = crate::window_chrome::client_content_bounds(window);
        let Some(shown) = self.shown().cloned() else {
            return div().into_any_element();
        };
        let strings = i18n::strings(cx);
        let viewer_strings = strings.picture_viewer();
        let tokens = theme(cx);
        let entity = cx.entity();
        let is_video = is_viewer_video(&shown);
        let layout = self.layout(window);
        if !layout.is_zoomable {
            // A window grown until the picture fits leaves actual size by itself.
            self.is_actual_size = false;
            self.scroll = Point::default();
        }
        let is_actual_size = self.is_actual_size;
        let padding = COMPONENT_PICTURE_VIEWER_STAGE_PADDING;
        let shown_size = match (is_video, layout.natural) {
            (true, natural) => {
                fitted_video_size(natural.unwrap_or(FALLBACK_VIDEO_SIZE), layout.area)
            }
            (false, Some(natural)) if is_actual_size => natural,
            (false, Some(natural)) => fitted_picture_size(natural, layout.area),
            // No stored size: the room's area, the picture contained in it.
            (false, None) => layout.area,
        };
        let left = picture_start(shown_size.width, padding, layout.stage.width, self.scroll.x);
        let top = picture_start(
            shown_size.height,
            padding,
            layout.stage.height,
            self.scroll.y,
        );

        // The picture: the list's preview at once (the same request, already decoded), the full
        // file over it once it arrives.
        let animation = picture_animation(cx);
        let (list_width, list_height) = picture_box(shown.width, shown.height);
        let preview_request = picture_request_for_box(
            DownloadableFile::AttachmentPreview(shown.id),
            list_width,
            list_height,
            animation,
            window,
        );
        let full_request = picture_request_for_box(
            DownloadableFile::Attachment(shown.id),
            shown_size.width,
            shown_size.height,
            animation,
            window,
        );
        let video_view = attachment_audio_view(shown.id, cx);
        let (full_status, preview_status) = if is_video {
            (PictureStatus::Loading, PictureStatus::Loading)
        } else {
            (
                picture_status(full_request.clone(), cx),
                picture_status(preview_request.clone(), cx),
            )
        };
        let is_loading = if is_video {
            video_view.playback == AudioPlayback::Loading
        } else {
            full_status == PictureStatus::Loading
        };
        let is_notice_due = self.is_loading_notice_due(is_loading, window, cx);
        let notice = match (is_video, full_status, preview_status) {
            (true, _, _) if video_view.playback == AudioPlayback::CannotPlay => {
                Some(StageNotice::VideoFailed)
            }
            (true, _, _) => is_notice_due.then_some(StageNotice::LoadingVideo),
            (false, PictureStatus::Failed, PictureStatus::Failed) => {
                Some(StageNotice::PictureFailed)
            }
            (false, PictureStatus::Failed, _) => Some(StageNotice::FullSizeFailed),
            (false, _, _) => is_notice_due.then_some(StageNotice::Loading),
        };

        // A focused control that is no longer drawn (the zoom toggle of a picture that failed or
        // now fits, "Try again" once it loads) would drop focus out of the viewer.
        let is_toggle_gone = !layout.is_zoomable || notice == Some(StageNotice::PictureFailed);
        let has_retry = matches!(
            notice,
            Some(StageNotice::FullSizeFailed | StageNotice::PictureFailed)
        );
        if (is_toggle_gone && self.picture_focus.is_focused(window))
            || (!has_retry && self.retry_focus.is_focused(window))
        {
            cx.defer_in(window, |viewer, window, cx| {
                window.focus(&viewer.close_focus, cx);
            });
        }

        let picture = if is_video {
            let fullscreen_entity = entity.clone();
            div()
                .id("media-viewer-video")
                .absolute()
                .left(length(left))
                .top(length(top))
                .w(length(shown_size.width))
                .h(length(shown_size.height))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|viewer, _: &MouseDownEvent, _, _| {
                        viewer.is_press_on_picture = true;
                    }),
                )
                .child(
                    VideoScreen::new(
                        ("media-viewer-screen", shown.id.raw()).into(),
                        shown.clone(),
                    )
                    .large()
                    .track_focus(&self.video_focus)
                    .on_fullscreen(
                        Some(window.is_fullscreen() || is_window_fullscreen_by_viewer(cx)),
                        move |window, cx| {
                            fullscreen_entity.update(cx, |_, cx| {
                                Self::toggle_fullscreen(window, cx);
                            });
                        },
                    )
                    .size_full()
                    .rounded(length(COMPONENT_PICTURE_VIEWER_PICTURE_RADIUS)),
                )
                .into_any_element()
        } else if notice == Some(StageNotice::PictureFailed) {
            div().into_any_element()
        } else {
            let is_full_ready = matches!(full_status, PictureStatus::Ready { .. });
            if is_full_ready {
                self.last_ready_full = Some(full_request.clone());
            }
            let sharper_meanwhile = self
                .last_ready_full
                .clone()
                .filter(|_| !is_full_ready)
                .filter(|ready| picture_status(ready.clone(), cx) != PictureStatus::Failed);
            let has_natural = layout.natural.is_some();
            let fit = move || {
                if has_natural {
                    ObjectFit::Fill
                } else {
                    ObjectFit::Contain
                }
            };
            let radius = length(COMPONENT_PICTURE_VIEWER_PICTURE_RADIUS);
            let is_ring_shown = layout.is_zoomable && is_focus_visible(&self.picture_focus, window);
            let cursor = match (layout.is_zoomable, is_actual_size, self.drag.is_some()) {
                (false, _, _) => CursorStyle::Arrow,
                (true, false, _) => CursorStyle::PointingHand,
                (true, true, false) => CursorStyle::OpenHand,
                (true, true, true) => CursorStyle::ClosedHand,
            };
            let (width, height) = (shown_size.width, shown_size.height);
            let element = div()
                .id(("media-viewer-picture", shown.id.raw()))
                .debug_selector(|| "media-viewer-picture".to_owned())
                .absolute()
                .left(length(left))
                .top(length(top))
                .w(length(width))
                .h(length(height))
                .cursor(cursor)
                .map(|element| {
                    if layout.is_zoomable {
                        element
                            .role(Role::Button)
                            .aria_label(viewer_strings.actual_size())
                            .aria_toggled(if is_actual_size {
                                Toggled::True
                            } else {
                                Toggled::False
                            })
                            .track_focus(&self.picture_focus)
                    } else {
                        element
                            .role(Role::Image)
                            .aria_label(shown.file_name.clone())
                    }
                })
                .on_mouse_down(MouseButton::Left, cx.listener(Self::on_picture_mouse_down))
                .on_click(cx.listener(Self::on_picture_click))
                .when_some(sharper_meanwhile.clone(), |element, ready| {
                    element.child(
                        img(picture_source(ready))
                            .absolute()
                            .size_full()
                            .object_fit(fit())
                            .rounded(radius),
                    )
                })
                .when(
                    !is_full_ready
                        && sharper_meanwhile.is_none()
                        && preview_status != PictureStatus::Failed,
                    |element| {
                        element.child(
                            img(picture_source(preview_request.clone()))
                                .absolute()
                                .size_full()
                                .object_fit(fit())
                                .rounded(radius),
                        )
                    },
                )
                .when(full_status != PictureStatus::Failed, |element| {
                    element.child(
                        img(picture_source(full_request.clone()))
                            .absolute()
                            .size_full()
                            .object_fit(fit())
                            .rounded(radius),
                    )
                })
                .when(is_ring_shown, |element| {
                    element.child(focus_ring_in(
                        COMPONENT_PICTURE_VIEWER_PICTURE_RADIUS,
                        color(tokens.component_picture_viewer_focus_ring),
                    ))
                });
            // Grows from 96 % about its centre as the room fades in; GPUI cannot scale a box, so
            // its size and place move instead.
            let (start_left, start_top) = (left, top);
            element
                .with_animation(
                    "media-viewer-picture-enter",
                    enter_animation(cx),
                    move |element, progress| {
                        let scale = COMPONENT_PICTURE_VIEWER_ENTER_SCALE
                            + (1.0 - COMPONENT_PICTURE_VIEWER_ENTER_SCALE) * progress;
                        element
                            .left(length(start_left + width * (1.0 - scale) / 2.0))
                            .top(length(start_top + height * (1.0 - scale) / 2.0))
                            .w(length(width * scale))
                            .h(length(height * scale))
                    },
                )
                .into_any_element()
        };

        let notice_element = notice.map(|notice| {
            let text = match notice {
                StageNotice::Loading => viewer_strings.loading(),
                StageNotice::LoadingVideo => viewer_strings.loading_video(),
                StageNotice::FullSizeFailed => viewer_strings.full_size_failed(),
                StageNotice::PictureFailed => viewer_strings.picture_failed(),
                StageNotice::VideoFailed => viewer_strings.video_failed(),
            };
            let is_failure = !matches!(notice, StageNotice::Loading | StageNotice::LoadingVideo);
            // A video that cannot play here would fail again: download is the way out.
            let retry = matches!(
                notice,
                StageNotice::FullSizeFailed | StageNotice::PictureFailed
            )
            .then(|| {
                let (full, preview) = (full_request.clone(), preview_request.clone());
                retry_button(full, preview, &entity, &self.retry_focus, window, cx)
            });
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(length(padding))
                .flex()
                .justify_center()
                .px(length(padding))
                .child(
                    with_type(div(), TYPE_BODY)
                        .id("media-viewer-notice")
                        .debug_selector(|| "media-viewer-notice".to_owned())
                        .occlude()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(length(COMPONENT_PICTURE_VIEWER_STATUS_GAP))
                        .p(length(COMPONENT_PICTURE_VIEWER_STATUS_PADDING))
                        .rounded(length(COMPONENT_PICTURE_VIEWER_STATUS_RADIUS))
                        .bg(color(tokens.component_picture_viewer_status_background))
                        .text_color(color(tokens.component_picture_viewer_status_text))
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .when(is_failure, |chip| {
                            chip.child(icon(
                                IconName::WarningCircle,
                                COMPONENT_PICTURE_VIEWER_STATUS_ICON_SIZE,
                                color(tokens.component_picture_viewer_status_text),
                            ))
                        })
                        .child(text)
                        .children(retry),
                )
        });

        let stage_bounds = Rc::clone(&self.stage_bounds);
        let stage = div()
            .id("media-viewer-stage")
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            // Each press starts outside the picture until the picture says otherwise: a click on
            // the video (which keeps its own click) leaves no stale flag behind.
            .capture_any_mouse_down(cx.listener(|viewer, _: &MouseDownEvent, _, _| {
                viewer.is_press_on_picture = false;
            }))
            .on_click(cx.listener(Self::on_room_click))
            .on_scroll(cx.listener(Self::on_wheel))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|viewer, _: &MouseUpEvent, _, _| viewer.drag = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|viewer, _: &MouseUpEvent, _, _| {
                    viewer.drag = None;
                    viewer.is_press_on_picture = false;
                }),
            )
            .child(
                canvas(
                    move |bounds, window, _| {
                        if stage_bounds.get() != Some(bounds) {
                            stage_bounds.set(Some(bounds));
                            // The picture is sized from the room: draw again at the new size.
                            window.request_animation_frame();
                        }
                    },
                    |_, (), _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(picture)
            .children(notice_element);

        let position_label =
            viewer_strings.position_label(count(self.index + 1), count(self.set.media.len()));
        let caption = viewer_strings.caption(
            &self.set.author_name,
            &message_timestamp_label(
                local_date_time(self.set.sent_at_unix_milliseconds),
                local_date_time(crate::session::now_unix_milliseconds()),
                strings,
            ),
            &file_size_label(shown.size_bytes, strings),
        );
        let band = self.band(&shown, caption.clone(), cx);
        let description: SharedString = if self.set.media.len() > 1 {
            format!("{caption}, {position_label}").into()
        } else {
            caption.into()
        };

        let room = div()
            .id("media-viewer")
            .debug_selector(|| "media-viewer".to_owned())
            .role(Role::Dialog)
            .aria_label(shown.file_name.clone())
            .aria_description(description)
            .track_focus(&self.container_focus)
            .tab_group()
            .key_context("MediaViewer")
            .on_key_down(cx.listener(Self::on_key_down))
            .on_action(cx.listener(|viewer, _: &FocusNextControl, window, cx| {
                focus_next_inside(&viewer.container_focus, window, cx);
            }))
            .on_action(cx.listener(|viewer, _: &FocusPreviousControl, window, cx| {
                focus_previous_inside(&viewer.container_focus, window, cx);
            }))
            .occlude()
            .map(|room| crate::window_chrome::sized_to_window_content(room, window))
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(color(tokens.component_picture_viewer_background))
            .child(stage)
            .child(band)
            .child(polite_live_region(
                "media-viewer-announcement",
                self.announcement.clone(),
            ))
            .with_animation(
                "media-viewer-enter",
                enter_animation(cx),
                gpui::Styled::opacity,
            );

        deferred(anchored().position(content_bounds.origin).child(room))
            .with_priority(VIEWER_LAYER_PRIORITY)
            .into_any_element()
    }
}

impl MediaViewer {
    /// The band under the room: caption, position and actions.
    #[allow(clippy::too_many_lines)] // Three groups of controls, each with its label.
    fn band(&self, shown: &Attachment, caption: String, cx: &mut Context<Self>) -> AnyElement {
        let tokens = theme(cx);
        let strings = i18n::strings(cx);
        let viewer_strings = strings.picture_viewer();
        let entity = cx.entity();
        let button = |id: &'static str, glyph: IconName, label: SharedString| {
            IconButton::new(id, glyph, label)
                .sizes(
                    COMPONENT_PICTURE_VIEWER_CONTROL_SIZE,
                    COMPONENT_PICTURE_VIEWER_CONTROL_ICON_SIZE,
                )
                .on_media_stage()
        };
        let (name_start, name_end) = file_name_start_and_end(&shown.file_name);
        let full_name = SharedString::from(shown.file_name.clone());
        let caption_group = div()
            .flex_grow(1.0)
            .flex_shrink(1.0)
            .flex_basis(length(COMPONENT_PICTURE_VIEWER_CAPTION_MIN_WIDTH))
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                with_type(div(), TYPE_BODY_LG)
                    .id("media-viewer-name")
                    .flex()
                    .min_w_0()
                    .text_color(color(tokens.component_picture_viewer_name_text))
                    .tooltip(move |_, cx| Tooltip::view(full_name.clone(), cx))
                    .tooltip_show_delay(Tooltip::show_delay())
                    .child(div().min_w_0().truncate().child(name_start))
                    .child(div().flex_none().child(name_end)),
            )
            .child(
                with_type(div(), TYPE_BODY_SM)
                    .truncate()
                    .text_color(color(tokens.component_picture_viewer_meta_text))
                    .child(caption),
            )
            .children(file_note(shown, cx));

        let position = (self.set.media.len() > 1).then(|| {
            let (previous_entity, next_entity) = (entity.clone(), entity.clone());
            div()
                .flex()
                .items_center()
                .gap(length(COMPONENT_PICTURE_VIEWER_CONTROL_GAP))
                .child(
                    button(
                        "media-viewer-previous",
                        IconName::CaretLeft,
                        viewer_strings.previous().into(),
                    )
                    .on_click(move |window, cx| {
                        previous_entity.update(cx, |viewer, cx| viewer.step(false, window, cx));
                    }),
                )
                .child(
                    with_type(div(), TYPE_BODY)
                        .debug_selector(|| "media-viewer-position".to_owned())
                        .text_color(color(tokens.component_picture_viewer_position_text))
                        .child(
                            viewer_strings
                                .position(count(self.index + 1), count(self.set.media.len())),
                        ),
                )
                .child(
                    button(
                        "media-viewer-next",
                        IconName::CaretRight,
                        viewer_strings.next().into(),
                    )
                    .on_click(move |window, cx| {
                        next_entity.update(cx, |viewer, cx| viewer.step(true, window, cx));
                    }),
                )
        });

        let (saved, opened) = (shown.clone(), shown.clone());
        let is_saving = save_outcome(shown.id, cx) == Some(SaveOutcome::Saving);
        let is_opening = open_outcome(shown.id, cx) == Some(OpenOutcome::Opening);
        let (copy_entity, close_entity) = (entity.clone(), entity);
        let (copy_icon, copy_label) = if self.is_copied {
            (IconName::Check, viewer_strings.link_copied())
        } else {
            (IconName::Copy, viewer_strings.copy_link())
        };
        let actions = div()
            .flex()
            .items_center()
            .gap(length(COMPONENT_PICTURE_VIEWER_CONTROL_GAP))
            .child(
                button(
                    "media-viewer-download",
                    IconName::DownloadSimple,
                    strings.messages().download(&shown.file_name).into(),
                )
                .disabled(is_saving)
                .on_click(move |_, cx| save_attachment(&saved, cx)),
            )
            .child(
                button("media-viewer-copy", copy_icon, copy_label.into()).on_click(
                    move |window, cx| {
                        copy_entity.update(cx, |viewer, cx| viewer.copy_link(window, cx));
                    },
                ),
            )
            // Only a picture or video type a viewer opens; anything else is downloaded instead.
            .when(opened_extension(&shown.content_type).is_some(), |actions| {
                actions.child(
                    button(
                        "media-viewer-open",
                        IconName::ArrowSquareOut,
                        strings.desktop().open_with_default_app().into(),
                    )
                    .disabled(is_opening)
                    .on_click(move |_, cx| open_attachment_with_default_app(&opened, cx)),
                )
            })
            .child(
                button(
                    "media-viewer-close",
                    IconName::X,
                    strings.common().close().into(),
                )
                .track_focus(&self.close_focus)
                .on_click(move |window, cx| {
                    close_entity.update(cx, |viewer, cx| viewer.close(window, cx));
                }),
            );

        div()
            .id("media-viewer-band")
            .debug_selector(|| "media-viewer-band".to_owned())
            .flex_none()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_x(length(COMPONENT_PICTURE_VIEWER_BAND_GAP))
            .gap_y(length(COMPONENT_PICTURE_VIEWER_BAND_ROW_GAP))
            .min_h(length(COMPONENT_PICTURE_VIEWER_BAND_MIN_HEIGHT))
            .px(length(COMPONENT_PICTURE_VIEWER_BAND_PADDING_INLINE))
            .py(length(COMPONENT_PICTURE_VIEWER_BAND_PADDING_BLOCK))
            // Three columns while they fit: the caption and the actions share the room left by the
            // position, which stays in the middle; narrower, they wrap (no breakpoint).
            .child(caption_group)
            .children(position)
            .child(
                div()
                    .flex_grow(1.0)
                    .flex_shrink(1.0)
                    .flex_basis(length(COMPONENT_PICTURE_VIEWER_CAPTION_MIN_WIDTH))
                    .flex()
                    .justify_end()
                    .child(actions),
            )
            .into_any_element()
    }
}

/// What became of saving or opening the shown file, under the caption and read out: the same
/// sentences as a file card's, in the stage's colours.
fn file_note(shown: &Attachment, cx: &App) -> Option<impl IntoElement + use<>> {
    let tokens = theme(cx);
    let desktop = i18n::strings(cx).desktop();
    let name = &shown.file_name;
    let (text, is_failure) = match (open_outcome(shown.id, cx), save_outcome(shown.id, cx)) {
        (Some(OpenOutcome::Opening), _) => (desktop.attachment_opening(name), false),
        (Some(OpenOutcome::Failed), _) => (desktop.attachment_open_failed(name), true),
        (None, Some(SaveOutcome::Saving)) => (desktop.attachment_saving(name), false),
        (None, Some(SaveOutcome::Saved)) => (desktop.attachment_saved(name), false),
        (None, Some(SaveOutcome::Failed)) => (desktop.attachment_save_failed(name), true),
        (None, None) => return None,
    };
    Some(
        with_type(div(), TYPE_BODY_SM)
            .debug_selector(|| "media-viewer-file-note".to_owned())
            .flex()
            .items_center()
            .gap(length(COMPONENT_PICTURE_VIEWER_STATUS_GAP))
            .text_color(color(tokens.component_picture_viewer_meta_text))
            .when(is_failure, |note| {
                note.child(icon(
                    IconName::WarningCircle,
                    COMPONENT_PICTURE_VIEWER_STATUS_ICON_SIZE,
                    color(tokens.component_picture_viewer_status_text),
                ))
            })
            .child(text.clone())
            .child(polite_live_region(
                "media-viewer-file-note-live",
                text.into(),
            )),
    )
}

/// The room's and the picture's entrance: `motion.duration.base` on the plain enter curve (no
/// settle overshoot on a photograph); GPUI jumps to its end under reduced motion.
fn enter_animation(cx: &App) -> Animation {
    Animation::new(animation_duration(MOTION_DURATION_BASE, cx).max(Duration::from_millis(1)))
        .with_easing(|progress| easing(MOTION_EASING_ENTER).sample(progress))
}

/// "Try again" in the notice: forgets both failures, so the room asks for the picture again.
fn retry_button(
    full: PictureRequest,
    preview: PictureRequest,
    entity: &Entity<MediaViewer>,
    focus_handle: &FocusHandle,
    window: &Window,
    cx: &App,
) -> impl IntoElement {
    let is_ring_shown = is_focus_visible(focus_handle, window);
    let tokens = theme(cx);
    let label = i18n::strings(cx).common().try_again();
    let entity = entity.clone();
    let border = color(tokens.component_picture_viewer_retry_border);
    with_type(div(), TYPE_BUTTON)
        .id("media-viewer-retry")
        .role(Role::Button)
        .aria_label(label)
        .track_focus(focus_handle)
        .relative()
        .flex()
        .items_center()
        .min_h(length(COMPONENT_PICTURE_VIEWER_CONTROL_SIZE))
        .px(length(COMPONENT_PICTURE_VIEWER_STATUS_PADDING))
        .rounded(length(COMPONENT_PICTURE_VIEWER_CONTROL_RADIUS))
        .border(length(COMPONENT_PICTURE_VIEWER_RETRY_BORDER_WIDTH))
        .border_color(border)
        .cursor_pointer()
        .hover(|button| {
            button.bg(color(
                tokens.component_picture_viewer_control_background_hover,
            ))
        })
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            retry_picture(full.clone(), cx);
            retry_picture(preview.clone(), cx);
            entity.update(cx, |viewer, cx| {
                viewer.loading_since = None;
                viewer.loading_notice_timer = None;
                window.focus(&viewer.close_focus, cx);
                cx.notify();
            });
        })
        .child(label)
        .when(is_ring_shown, |button| {
            button.child(focus_ring_in(
                COMPONENT_PICTURE_VIEWER_CONTROL_RADIUS,
                color(tokens.component_picture_viewer_focus_ring),
            ))
        })
}

impl std::fmt::Debug for MediaViewer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MediaViewer")
            .field("index", &self.index)
            .field("count", &self.set.media.len())
            .field("is_actual_size", &self.is_actual_size)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
