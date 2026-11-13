//! Your camera in a call and its preview in the Voice settings (`voice-session.svelte.ts`'s
//! camera, `camera.ts`, ADR-0035): the panel's camera button turns it on or off, the settings
//! choose which camera (applied live) and preview it, mirrored.
//!
//! As on the web, the camera opens first and only a working camera is announced to the server
//! (`VoiceSession::start_video`): a permission prompt or a busy device never leaves a camera the
//! server waits on. Each camera runs on a worker thread of its own, which opens it (that blocks:
//! permission prompts, the device starting), forwards its pictures into the call once told to,
//! and converts one to a small mirrored picture for the settings while they show it, and for your
//! tile while the call view shows it. Dropping the
//! worker's handle stops it; the thread closes the device itself, never the interface's thread.
//! Its few reports (opened, failed, ended) reach the interface through a bounded channel.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::Duration;

use gpui::{App, AppContext as _, Context};
use mellow_media::{
    CameraDescription, CameraError, CameraFeed, Cameras, DecodedVideoFrame, SystemCameras,
    VideoInput, VideoSourceKind, VoiceSession, VoiceSessionEvent,
};
use mellow_protocol::{Permissions, VoiceVideoRefusal};

use super::{CallPhase, Voice, current_settings};
use crate::i18n::Strings;

/// Opens the cameras; tests hand synthetic ones.
pub type CamerasFactory = Arc<dyn Fn() -> Box<dyn Cameras + Send> + Send + Sync>;

/// The system's cameras.
#[must_use]
pub fn system_cameras() -> CamerasFactory {
    Arc::new(|| Box::new(SystemCameras))
}

/// How long the worker waits for a picture before it looks at its commands again: the delay of
/// stopping a camera that sends nothing.
const PICTURE_WAIT: Duration = Duration::from_millis(100);
/// The settings' preview is drawn at most this large (the column it sits in); your call tile
/// gets the same picture.
///
/// shortcut: your tile gets this 640 × 360 picture, blurry when featured or full screen; convert
/// at the tile's drawn size if that shows.
const PREVIEW_SIZE: (u32, u32) = (640, 360);
/// A worker reports at most twice (opened, then ended or closed).
const REPORT_QUEUE_CAPACITY: usize = 4;
/// macOS's Privacy & Security pane for the camera.
const MACOS_CAMERA_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Camera";
/// Windows' Privacy & security page for the camera.
const WINDOWS_CAMERA_SETTINGS_URL: &str = "ms-settings:privacy-webcam";

/// Where the call's camera stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraPhase {
    Off,
    /// Opening: the panel says "Starting camera…".
    Starting,
    On,
}

/// Why the camera turned off or could not turn on (`CameraProblem` on the web).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraProblem {
    /// The system or the user refused the camera to Mellow.
    Denied,
    NotFound,
    /// Another application holds it.
    InUse,
    /// It failed for a reason the system did not name.
    Failed,
    /// Unplugged or taken by the system while on.
    Ended,
    /// The instance stopped relaying it (nothing came for 10 s, or the permission went away).
    Stopped,
    Refused(VoiceVideoRefusal),
}

impl CameraProblem {
    fn of_error(error: &CameraError) -> Self {
        match error {
            CameraError::NoCamera => Self::NotFound,
            CameraError::PermissionDenied => Self::Denied,
            CameraError::Busy => Self::InUse,
            CameraError::System(_) => Self::Failed,
        }
    }

    /// What the panel or the settings say.
    #[must_use]
    pub fn sentence(self, strings: Strings) -> &'static str {
        let problems = strings.camera().problems();
        match self {
            Self::Denied if cfg!(target_os = "macos") => strings.desktop().camera_denied_macos(),
            Self::Denied if cfg!(windows) => strings.desktop().camera_denied_windows(),
            Self::Denied => strings.desktop().camera_denied_linux(),
            Self::NotFound => problems.not_found(),
            Self::InUse => problems.in_use(),
            Self::Failed => strings.desktop().camera_failed(),
            Self::Ended => problems.ended(),
            Self::Stopped => problems.stopped(),
            Self::Refused(VoiceVideoRefusal::MissingPermissions) => problems.missing_permissions(),
            Self::Refused(VoiceVideoRefusal::TooManyCameras) => problems.too_many_cameras(),
            Self::Refused(VoiceVideoRefusal::VideoUnavailable) => problems.video_unavailable(),
        }
    }

    /// The system's settings can lift it: macOS's camera privacy pane, Windows' camera privacy
    /// page.
    #[must_use]
    pub fn can_open_system_settings(self) -> bool {
        self == Self::Denied && cfg!(any(target_os = "macos", windows))
    }
}

/// Opens the system's camera privacy settings (the Denied problem's button).
pub fn open_camera_system_settings(cx: &App) {
    cx.open_url(if cfg!(windows) {
        WINDOWS_CAMERA_SETTINGS_URL
    } else {
        MACOS_CAMERA_SETTINGS_URL
    });
}

/// The label of the Denied problem's button.
#[must_use]
pub fn open_camera_settings_label(strings: Strings) -> &'static str {
    if cfg!(windows) {
        strings.desktop().open_camera_settings_windows()
    } else {
        strings.desktop().open_camera_settings()
    }
}

/// Why the camera button cannot turn the camera on now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraBlock {
    /// Not connected to a call yet.
    NotConnected,
    /// No `STREAM` ("Video") permission in this channel.
    MissingPermission,
    /// The system lists no camera.
    NoCamera,
}

impl CameraBlock {
    /// Said by the disabled button.
    #[must_use]
    pub fn sentence(self, strings: Strings) -> &'static str {
        match self {
            Self::NotConnected => strings.camera().turn_on(),
            Self::MissingPermission => strings.camera().problems().missing_permissions(),
            Self::NoCamera => strings.camera().problems().not_found(),
        }
    }
}

/// What a worker tells the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CameraReport {
    Opened,
    Failed(CameraProblem),
    /// The camera stopped by itself.
    Ended,
    /// The call took its input back (a screen replaced the camera): the worker stopped.
    Closed,
}

enum WorkerCommand {
    SendTo(VideoInput),
}

/// Shared between a worker and the interface.
#[derive(Default)]
struct WorkerShared {
    /// The settings show this camera: each picture is also converted for them.
    is_preview_wanted: AtomicBool,
    /// The newest converted picture, latest-wins.
    preview: Mutex<Option<DecodedVideoFrame>>,
}

/// A running camera; dropping it stops the camera (on the worker's thread).
struct CameraWorker {
    id: u64,
    commands: mpsc::Sender<WorkerCommand>,
    shared: Arc<WorkerShared>,
}

impl CameraWorker {
    fn send_to(&self, input: VideoInput) {
        // The worker gone has reported why (or is about to): nothing to do here.
        let _ = self.commands.send(WorkerCommand::SendTo(input));
    }

    fn set_preview_wanted(&self, is_wanted: bool) {
        self.shared
            .is_preview_wanted
            .store(is_wanted, Ordering::Relaxed);
        if !is_wanted {
            self.shared
                .preview
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
        }
    }

    fn take_preview(&self) -> Option<DecodedVideoFrame> {
        self.shared
            .preview
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

fn report(reports: &tokio::sync::mpsc::Sender<CameraReport>, report: CameraReport) {
    // At most a few per worker into a queue this deep; a gone interface needs none.
    let _ = reports.try_send(report);
}

/// The worker's thread: opens the camera, then hands its pictures on until its handle is
/// dropped, the camera ends or the call takes the input back.
fn run_camera_worker(
    cameras: &dyn Cameras,
    device_id: Option<&str>,
    commands: &mpsc::Receiver<WorkerCommand>,
    shared: &WorkerShared,
    reports: &tokio::sync::mpsc::Sender<CameraReport>,
) {
    let feed = match CameraFeed::open(cameras, device_id) {
        Ok(feed) => feed,
        Err(error) => {
            tracing::warn!(%error, "the camera could not open");
            report(
                reports,
                CameraReport::Failed(CameraProblem::of_error(&error)),
            );
            return;
        }
    };
    report(reports, CameraReport::Opened);
    let mut input: Option<VideoInput> = None;
    loop {
        loop {
            match commands.try_recv() {
                Ok(WorkerCommand::SendTo(new_input)) => input = Some(new_input),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }
        if feed.has_ended() {
            report(reports, CameraReport::Ended);
            return;
        }
        if input.as_ref().is_some_and(|input| !input.is_open()) {
            report(reports, CameraReport::Closed);
            return;
        }
        let Some(picture) = feed.wait_picture(PICTURE_WAIT) else {
            continue;
        };
        if shared.is_preview_wanted.load(Ordering::Relaxed)
            && let Some(preview) = picture.to_mirrored_bgra(PREVIEW_SIZE)
        {
            *shared
                .preview
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(preview);
        }
        if let Some(input) = input.as_ref() {
            input.push(picture);
        }
    }
}

/// Which camera a worker is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CameraUse {
    Call,
    Preview,
}

/// The settings' preview outside the call's camera.
struct PreviewCamera {
    worker: CameraWorker,
    is_open: bool,
}

/// The camera part of [`Voice`].
pub(super) struct CameraState {
    cameras: CamerasFactory,
    phase: CameraPhase,
    problem: Option<CameraProblem>,
    worker: Option<CameraWorker>,
    /// While another camera opens live, the one still sending: kept until the new one works.
    previous_worker: Option<CameraWorker>,
    /// The call took the camera's input back with nothing replacing it: the server's reason
    /// (refused, stopped), which travels apart, may still arrive.
    is_server_reason_awaited: bool,
    preview: Option<PreviewCamera>,
    preview_problem: Option<CameraProblem>,
    /// The settings' Video part is shown: the call's camera converts pictures for it.
    is_preview_shown: bool,
    /// The call view is open: your tile shows your camera from the same converted pictures.
    is_own_tile_shown: bool,
    /// `None` until listed once.
    listed: Option<Vec<CameraDescription>>,
    /// Ids for workers, so a stopped one's late reports are ignored.
    next_worker_id: u64,
}

impl CameraState {
    pub(super) fn new(cameras: CamerasFactory) -> Self {
        Self {
            cameras,
            phase: CameraPhase::Off,
            problem: None,
            worker: None,
            previous_worker: None,
            is_server_reason_awaited: false,
            preview: None,
            preview_problem: None,
            is_preview_shown: false,
            is_own_tile_shown: false,
            listed: None,
            next_worker_id: 0,
        }
    }

    /// Whether the call's camera converts pictures for the settings or your tile.
    fn is_preview_wanted(&self) -> bool {
        self.is_preview_shown || self.is_own_tile_shown
    }

    /// Leaving the call: the camera goes with it (the engine, gone, needs no telling).
    pub(super) fn end_with_call(&mut self) {
        self.worker = None;
        self.previous_worker = None;
        self.is_server_reason_awaited = false;
        self.phase = CameraPhase::Off;
        self.problem = None;
    }
}

impl Voice {
    // ---- What the interface reads ----

    #[must_use]
    pub fn camera_phase(&self) -> CameraPhase {
        self.camera.phase
    }

    #[must_use]
    pub fn camera_problem(&self) -> Option<CameraProblem> {
        self.camera.problem
    }

    /// The cameras last listed; `None` before the first listing.
    #[must_use]
    pub fn listed_cameras(&self) -> Option<&[CameraDescription]> {
        self.camera.listed.as_deref()
    }

    /// Why the camera cannot turn on now, or `None`. Turning it off is always allowed.
    #[must_use]
    pub fn camera_block(&self, cx: &App) -> Option<CameraBlock> {
        let Some(channel_id) = self
            .channel_id()
            .filter(|_| self.phase() == Some(CallPhase::Connected) && self.engine().is_some())
        else {
            return Some(CameraBlock::NotConnected);
        };
        let permissions = self
            .session
            .read(cx)
            .state()
            .my_channel_permissions(channel_id);
        if permissions.is_some_and(|permissions| !permissions.contains(Permissions::STREAM)) {
            return Some(CameraBlock::MissingPermission);
        }
        if self.camera.listed.as_ref().is_some_and(Vec::is_empty) {
            return Some(CameraBlock::NoCamera);
        }
        None
    }

    /// Uses `cameras` from now on (tests hand synthetic ones).
    pub fn set_cameras(&mut self, cameras: CamerasFactory) {
        self.camera.cameras = cameras;
    }

    // ---- The call's camera ----

    /// The panel's camera button: on, or off (also while it starts).
    pub fn toggle_camera(&mut self, cx: &mut Context<Self>) {
        if self.camera.phase == CameraPhase::Off {
            self.start_camera(cx);
        } else {
            self.stop_camera(cx);
        }
    }

    fn start_camera(&mut self, cx: &mut Context<Self>) {
        if self.camera_block(cx).is_some() {
            return;
        }
        // The call takes the device: the settings now show it from the call.
        self.camera.preview = None;
        self.camera.preview_problem = None;
        self.camera.problem = None;
        self.camera.is_server_reason_awaited = false;
        self.camera.phase = CameraPhase::Starting;
        let worker = self.start_camera_worker(CameraUse::Call, cx);
        worker.set_preview_wanted(self.camera.is_preview_wanted());
        self.camera.worker = Some(worker);
        self.follow_own_camera_tile(cx);
        cx.notify();
    }

    /// Turns the camera off and tells the server.
    pub fn stop_camera(&mut self, cx: &mut Context<Self>) {
        self.camera.worker = None;
        self.camera.previous_worker = None;
        self.camera.phase = CameraPhase::Off;
        if let Some(engine) = self.engine()
            && engine.video_source() == Some(VideoSourceKind::Camera)
        {
            engine.stop_video();
        }
        cx.notify();
    }

    pub fn dismiss_camera_problem(&mut self, cx: &mut Context<Self>) {
        self.camera.problem = None;
        cx.notify();
    }

    /// The engine said the server refused or stopped the video: the camera, if it was on, stops.
    /// A shared screen's refusal is the screen's business.
    pub(super) fn note_video_stopped_by_server(
        &mut self,
        event: &VoiceSessionEvent,
        cx: &mut Context<Self>,
    ) {
        if self.camera.phase == CameraPhase::Off && !self.camera.is_server_reason_awaited {
            return;
        }
        let problem = match event {
            VoiceSessionEvent::VideoRefused(reason) => CameraProblem::Refused(*reason),
            _ => CameraProblem::Stopped,
        };
        self.stop_camera(cx);
        self.camera.is_server_reason_awaited = false;
        self.camera.problem = Some(problem);
    }

    /// The chosen camera changed: the camera on (or opening) and the preview move to it. A
    /// camera on keeps sending until the new one works, and stays if the new one fails, as on
    /// the web.
    pub(super) fn follow_camera_device(&mut self, cx: &mut Context<Self>) {
        if self.camera.phase != CameraPhase::Off {
            let replaced = self.camera.worker.take();
            // Only a camera that sends is worth keeping; an earlier switch's keeps its place.
            if self.camera.phase == CameraPhase::On && self.camera.previous_worker.is_none() {
                self.camera.previous_worker = replaced;
            }
            let worker = self.start_camera_worker(CameraUse::Call, cx);
            worker.set_preview_wanted(self.camera.is_preview_wanted());
            self.camera.worker = Some(worker);
        }
        if self.camera.preview.is_some() {
            self.start_camera_preview(cx);
        }
        cx.notify();
    }

    // ---- The settings ----

    /// Lists the cameras off the interface's thread (listing may wait on the system).
    pub fn refresh_cameras(&mut self, cx: &mut Context<Self>) {
        let cameras = (self.camera.cameras)();
        let listing = cx.background_spawn(async move { cameras.cameras() });
        cx.spawn(async move |voice, cx| {
            let listed = match listing.await {
                Ok(listed) => listed,
                Err(error) => {
                    tracing::warn!(%error, "cannot list the cameras");
                    return;
                }
            };
            let _ = voice.update(cx, |voice, cx| {
                if voice.camera.listed.as_ref() != Some(&listed) {
                    voice.camera.listed = Some(listed);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The settings' Video part shows or hides: while shown, the call's camera converts its
    /// pictures for it; hiding it also stops the test preview.
    pub fn set_camera_preview_shown(&mut self, is_shown: bool, cx: &mut Context<Self>) {
        self.camera.is_preview_shown = is_shown;
        self.follow_preview_wish();
        if !is_shown {
            self.stop_camera_preview(cx);
        }
    }

    /// The call view opens or closes: while open, the call's camera converts its pictures for
    /// your tile too.
    pub(super) fn set_own_camera_tile_shown(&mut self, is_shown: bool) {
        self.camera.is_own_tile_shown = is_shown;
        self.follow_preview_wish();
    }

    fn follow_preview_wish(&self) {
        let is_wanted = self.camera.is_preview_wanted();
        for worker in [&self.camera.worker, &self.camera.previous_worker]
            .into_iter()
            .flatten()
        {
            worker.set_preview_wanted(is_wanted);
        }
    }

    /// The newest picture of the call's camera for your tile; `None` while it is off.
    ///
    /// shortcut: the settings preview and your tile take from one slot, so with both shown each
    /// gets about half the pictures; give each its own slot if that ever shows.
    #[must_use]
    pub(super) fn take_own_camera_picture(&self) -> Option<DecodedVideoFrame> {
        if self.camera.phase == CameraPhase::Off {
            return None;
        }
        self.take_camera_preview_picture()
    }

    /// "Test the camera", outside the call's camera.
    pub fn start_camera_preview(&mut self, cx: &mut Context<Self>) {
        if self.camera.phase != CameraPhase::Off {
            return;
        }
        self.camera.preview_problem = None;
        let worker = self.start_camera_worker(CameraUse::Preview, cx);
        worker.set_preview_wanted(true);
        self.camera.preview = Some(PreviewCamera {
            worker,
            is_open: false,
        });
        cx.notify();
    }

    pub fn stop_camera_preview(&mut self, cx: &mut Context<Self>) {
        self.camera.preview = None;
        self.camera.preview_problem = None;
        cx.notify();
    }

    /// Whether the test preview runs (opening included).
    #[must_use]
    pub fn is_camera_preview_running(&self) -> bool {
        self.camera.preview.is_some()
    }

    /// The camera the settings show is still opening.
    #[must_use]
    pub fn is_camera_preview_starting(&self) -> bool {
        match self.camera.phase {
            CameraPhase::Off => self
                .camera
                .preview
                .as_ref()
                .is_some_and(|preview| !preview.is_open),
            CameraPhase::Starting => true,
            CameraPhase::On => false,
        }
    }

    #[must_use]
    pub fn camera_preview_problem(&self) -> Option<CameraProblem> {
        self.camera.preview_problem
    }

    /// The newest picture for the settings since the last call: the call's camera when on, else
    /// the test's.
    #[must_use]
    pub fn take_camera_preview_picture(&self) -> Option<DecodedVideoFrame> {
        match self.camera.phase {
            CameraPhase::Off => self
                .camera
                .preview
                .as_ref()
                .and_then(|preview| preview.worker.take_preview()),
            _ => self
                .camera
                .worker
                .as_ref()
                .and_then(CameraWorker::take_preview)
                .or_else(|| {
                    self.camera
                        .previous_worker
                        .as_ref()
                        .and_then(CameraWorker::take_preview)
                }),
        }
    }

    // ---- Workers ----

    fn start_camera_worker(
        &mut self,
        camera_use: CameraUse,
        cx: &mut Context<Self>,
    ) -> CameraWorker {
        self.camera.next_worker_id += 1;
        let id = self.camera.next_worker_id;
        let (commands, received_commands) = mpsc::channel();
        let (reports, mut received_reports) = tokio::sync::mpsc::channel(REPORT_QUEUE_CAPACITY);
        let shared = Arc::new(WorkerShared::default());
        let cameras = (self.camera.cameras)();
        let device_id = current_settings(cx).camera_device_id;
        let worker_shared = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name("mellow-camera-worker".into())
            .spawn(move || {
                run_camera_worker(
                    cameras.as_ref(),
                    device_id.as_deref(),
                    &received_commands,
                    &worker_shared,
                    &reports,
                );
            });
        match spawned {
            Ok(_) => {
                cx.spawn(async move |voice, cx| {
                    while let Some(report) = received_reports.recv().await {
                        let is_gone = voice
                            .update(cx, |voice, cx| {
                                voice.follow_camera_report(camera_use, id, report, cx);
                            })
                            .is_err();
                        if is_gone {
                            return;
                        }
                    }
                })
                .detach();
            }
            Err(error) => {
                tracing::warn!(error = %error.kind(), "cannot start the camera's thread");
                let problem = Some(CameraProblem::Failed);
                // Shown at once; the worker returned below never reports.
                match camera_use {
                    CameraUse::Call => self.camera.problem = problem,
                    CameraUse::Preview => self.camera.preview_problem = problem,
                }
            }
        }
        CameraWorker {
            id,
            commands,
            shared,
        }
    }

    fn follow_camera_report(
        &mut self,
        camera_use: CameraUse,
        worker_id: u64,
        report: CameraReport,
        cx: &mut Context<Self>,
    ) {
        match camera_use {
            CameraUse::Call => self.follow_call_camera_report(worker_id, report, cx),
            CameraUse::Preview => self.follow_preview_report(worker_id, report, cx),
        }
        cx.notify();
    }

    fn follow_call_camera_report(
        &mut self,
        worker_id: u64,
        report: CameraReport,
        cx: &mut Context<Self>,
    ) {
        let is_of = |worker: &Option<CameraWorker>| {
            worker.as_ref().is_some_and(|worker| worker.id == worker_id)
        };
        if is_of(&self.camera.previous_worker) {
            self.follow_previous_camera_report(report, cx);
            return;
        }
        if !is_of(&self.camera.worker) {
            return;
        }
        let is_switching = self.camera.previous_worker.is_some();
        match report {
            CameraReport::Opened => {
                // Released before the new input replaces its own, so it never reports Closed.
                self.camera.previous_worker = None;
                let Some(engine) = self.engine() else {
                    self.stop_camera(cx);
                    return;
                };
                // Announced only now that it works; a new camera replaces the old one's input.
                let input = engine.start_video(VideoSourceKind::Camera);
                if let Some(worker) = self.camera.worker.as_ref() {
                    worker.send_to(input);
                }
                self.camera.phase = CameraPhase::On;
                // The call sends one video: the camera replaced a shared screen.
                self.note_camera_replaced_screen_share(cx);
                self.refresh_cameras(cx);
            }
            CameraReport::Failed(problem) if is_switching => {
                // The camera that was on stays on; only the problem shows.
                self.camera.worker = self.camera.previous_worker.take();
                self.camera.problem = Some(problem);
            }
            CameraReport::Failed(problem) => {
                self.stop_camera(cx);
                self.camera.problem = Some(problem);
            }
            CameraReport::Ended => {
                self.stop_camera(cx);
                self.camera.problem = Some(CameraProblem::Ended);
            }
            CameraReport::Closed => self.note_camera_input_closed(),
        }
    }

    /// A report from the camera still sending while another one opens.
    fn follow_previous_camera_report(&mut self, report: CameraReport, cx: &mut Context<Self>) {
        match report {
            // Unplugged meanwhile: the new camera, when it opens, carries on alone.
            CameraReport::Ended | CameraReport::Failed(_) => self.camera.previous_worker = None,
            CameraReport::Closed => {
                self.stop_camera(cx);
                self.note_camera_input_closed();
            }
            CameraReport::Opened => {}
        }
    }

    /// The call took the camera's input back: a shared screen replaced it, or the server
    /// refused or stopped it (whose reason, sent apart, may come after this).
    fn note_camera_input_closed(&mut self) {
        self.camera.worker = None;
        self.camera.previous_worker = None;
        self.camera.phase = CameraPhase::Off;
        if self.engine().and_then(VoiceSession::video_source).is_none() {
            self.camera.problem = Some(CameraProblem::Stopped);
            self.camera.is_server_reason_awaited = true;
        }
    }

    fn follow_preview_report(
        &mut self,
        worker_id: u64,
        report: CameraReport,
        cx: &mut Context<Self>,
    ) {
        let Some(preview) = self
            .camera
            .preview
            .as_mut()
            .filter(|preview| preview.worker.id == worker_id)
        else {
            return;
        };
        match report {
            CameraReport::Opened => {
                preview.is_open = true;
                // Names show the first time a camera works on some systems.
                self.refresh_cameras(cx);
            }
            CameraReport::Failed(problem) => {
                self.camera.preview = None;
                self.camera.preview_problem = Some(problem);
            }
            CameraReport::Ended | CameraReport::Closed => {
                self.camera.preview = None;
                self.camera.preview_problem = Some(CameraProblem::Ended);
            }
        }
    }
}

#[cfg(test)]
#[path = "camera_tests.rs"]
mod tests;
