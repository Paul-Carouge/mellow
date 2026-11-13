//! Sharing your own screen from the desktop (`voice-session.svelte.ts`'s screen share, ADR-0037,
//! ADR-0054): the share's phase (the system's capture starting, on), its problem, the in-app
//! picker's list (open beside a running share to switch source), the quality and sound kept in
//! the preferences, and the running `ScreenShare`.
//!
//! Threads: listing sources (thumbnails) and starting a share (the Wayland portal's dialog can
//! stay open for minutes) block, so each runs on its own short-lived thread; dropping a share
//! waits for the system to stop its capture, so it is dropped on another one. An answer that
//! comes after the call ended, another share started or the person stopped is dropped, by the
//! share generation it was started with.

use std::sync::Arc;

use gpui::{Context, SharedString};
use mellow_media::{
    ScreenCaptureError, ScreenChoice, ScreenPicker, ScreenShare, ScreenShareEnd,
    ScreenShareQuality, ScreenSourceKind, ScreenSources, ShareSound, ShareSoundState,
    SoundApplication, SystemScreenSources, VoiceSessionEvent,
};
use mellow_protocol::{VideoCodec, VoiceVideoRefusal};

use super::Voice;
use super::video_view::VideoPicture;
use crate::preferences::{change_preferences, preferences};

/// The qualities offered, lowest first, as the web's `SCREEN_SHARE_QUALITIES`.
pub const SCREEN_SHARE_QUALITIES: [ScreenShareQuality; 3] = [
    ScreenShareQuality::Standard,
    ScreenShareQuality::High,
    ScreenShareQuality::Highest,
];

/// The quality's name, as the web shows and stores it.
#[must_use]
pub fn screen_share_quality_label(quality: ScreenShareQuality) -> &'static str {
    match quality {
        ScreenShareQuality::Standard => "720p",
        ScreenShareQuality::High => "1080p",
        ScreenShareQuality::Highest => "1440p",
    }
}

#[must_use]
pub fn screen_share_quality_of_label(label: &str) -> Option<ScreenShareQuality> {
    SCREEN_SHARE_QUALITIES
        .into_iter()
        .find(|quality| screen_share_quality_label(*quality) == label)
}

/// Where your share stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenSharePhase {
    Off,
    /// The system's capture is starting, or its own dialog (Wayland) is open.
    Starting,
    On,
}

/// Why your share is not (or no longer) on. A cancelled choice is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenShareProblem {
    /// macOS's Screen Recording permission was refused.
    PermissionDenied,
    /// No way to capture here (no portal, too old a system).
    Unavailable,
    SourceGone,
    CouldNotStart,
    /// The running capture or its encoder failed.
    CaptureFailed,
    Refused(VoiceVideoRefusal),
    /// The instance turned it off (its Video permission removed, nothing sent for too long).
    StoppedByServer,
}

impl ScreenShareProblem {
    fn of_error(error: &ScreenCaptureError) -> Option<Self> {
        match error {
            ScreenCaptureError::Cancelled => None,
            ScreenCaptureError::PermissionDenied => Some(Self::PermissionDenied),
            ScreenCaptureError::Unavailable => Some(Self::Unavailable),
            ScreenCaptureError::SourceGone => Some(Self::SourceGone),
            ScreenCaptureError::System(_) => Some(Self::CouldNotStart),
        }
    }
}

/// A screen or window the in-app picker offers.
#[derive(Debug, Clone)]
pub struct ScreenSourceCard {
    pub id: String,
    pub kind: ScreenSourceKind,
    /// Empty for a display the system does not name.
    pub title: String,
    pub application_name: Option<String>,
    pub thumbnail: Option<VideoPicture>,
}

/// The in-app picker's list.
#[derive(Debug, Clone)]
pub enum ScreenSourceListing {
    Loading,
    Listed(Vec<ScreenSourceCard>),
    Failed(ScreenShareProblem),
}

/// What a running share sends as sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SharedSound {
    /// The shared screen's or window's own sound.
    FromSource,
    Application(String),
    /// Turned off in the share options.
    Off,
    /// Asked for, but the system cannot tell a window's app (Wayland).
    NotAvailableOnWayland,
    /// Asked for, but the system captures no sound (macOS before 13).
    NotAvailableOnThisSystem,
}

/// Your share, kept by [`Voice`].
pub(super) struct ScreenSharing {
    sources: Arc<dyn ScreenSources>,
    /// Asked once: the session kind does not change while the app runs.
    picker: ScreenPicker,
    phase: ScreenSharePhase,
    problem: Option<ScreenShareProblem>,
    /// The in-app picker's list while it is open.
    listing: Option<ScreenSourceListing>,
    /// Bumped by each opening of the picker: a late list of an earlier one is dropped.
    listing_generation: u64,
    share: Option<ScreenShare>,
    codec: VideoCodec,
    sound: SharedSound,
    /// Applications playing sound, offered on Wayland, where a window's own sound is unknown.
    sound_applications: Vec<SoundApplication>,
    /// Chosen among them for the next share, for this run of the app only (their ids do not
    /// outlive the applications).
    sound_application: Option<SoundApplication>,
    /// Read out politely: the share started, ended or was cancelled.
    announcement: SharedString,
    /// Bumped by each start, stop and end: a late answer of an earlier one is dropped.
    generation: u64,
}

impl ScreenSharing {
    pub(super) fn new(sources: Arc<dyn ScreenSources>) -> Self {
        let picker = sources.picker();
        Self {
            sources,
            picker,
            phase: ScreenSharePhase::Off,
            problem: None,
            listing: None,
            listing_generation: 0,
            share: None,
            codec: VideoCodec::Vp8,
            sound: SharedSound::Off,
            sound_applications: Vec::new(),
            sound_application: None,
            announcement: SharedString::default(),
            generation: 0,
        }
    }

    pub(super) fn system() -> Self {
        Self::new(Arc::new(SystemScreenSources))
    }

    /// Turns the share off: its capture stops off the UI thread, late answers are dropped.
    fn turn_off(&mut self) {
        self.generation += 1;
        self.phase = ScreenSharePhase::Off;
        if let Some(share) = self.share.take() {
            stop_off_the_ui_thread(share);
        }
    }

    /// The call ended: nothing of its share remains.
    pub(super) fn end_call(&mut self) {
        self.turn_off();
        self.listing = None;
        self.problem = None;
        self.announcement = SharedString::default();
    }
}

/// Runs `work` on a thread of its own; `None` when the thread cannot start.
fn run_on_own_thread<T: Send + 'static>(
    name: &str,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<tokio::sync::oneshot::Receiver<T>> {
    let (answer, answered) = tokio::sync::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            // The receiver gone means the interface no longer waits.
            let _ = answer.send(work());
        });
    match spawned {
        Ok(_) => Some(answered),
        Err(error) => {
            tracing::warn!(error = %error.kind(), "cannot start a screen sharing thread");
            None
        }
    }
}

/// Drops `share` on a thread of its own: it waits for the system to stop the capture.
fn stop_off_the_ui_thread(share: ScreenShare) {
    if let Err(error) = std::thread::Builder::new()
        .name("mellow-screen-share-stop".into())
        .spawn(move || drop(share))
    {
        // The share was moved into the closure that failed to start, and dropped with it.
        tracing::warn!(error = %error.kind(), "cannot start the thread stopping a screen share");
    }
}

fn shared_sound_of(
    state: ShareSoundState,
    sound: &ShareSound,
    picker: ScreenPicker,
) -> SharedSound {
    match (state, sound) {
        (ShareSoundState::Off, _) | (_, ShareSound::Off) => SharedSound::Off,
        (ShareSoundState::Captured, ShareSound::Application(_)) => {
            // The name is filled in by the caller, which knows it.
            SharedSound::Application(String::new())
        }
        (ShareSoundState::Captured, ShareSound::FromSource) => SharedSound::FromSource,
        (ShareSoundState::NotAvailable, _) if picker == ScreenPicker::System => {
            SharedSound::NotAvailableOnWayland
        }
        (ShareSoundState::NotAvailable, _) => SharedSound::NotAvailableOnThisSystem,
    }
}

impl Voice {
    // ---- What the interface reads ----

    /// How this system lets the person pick what to share.
    #[must_use]
    pub fn screen_picker(&self) -> ScreenPicker {
        self.screen.picker
    }

    /// The share button is offered: connected, on a system that can capture (the web's
    /// `canShareScreen`; the instance decides the Video permission and says so).
    #[must_use]
    pub fn can_share_screen(&self) -> bool {
        self.phase() == Some(super::CallPhase::Connected)
            && self.screen.picker != ScreenPicker::Unavailable
    }

    #[must_use]
    pub fn screen_share_phase(&self) -> ScreenSharePhase {
        self.screen.phase
    }

    #[must_use]
    pub fn is_sharing_screen(&self) -> bool {
        self.screen.phase == ScreenSharePhase::On
    }

    #[must_use]
    pub fn screen_share_problem(&self) -> Option<ScreenShareProblem> {
        self.screen.problem
    }

    /// The in-app picker's list, while it is open.
    #[must_use]
    pub fn screen_sources(&self) -> Option<&ScreenSourceListing> {
        self.screen.listing.as_ref()
    }

    /// What the running share sends as sound.
    #[must_use]
    pub fn shared_sound(&self) -> &SharedSound {
        &self.screen.sound
    }

    /// The codec watchers receive: VP8 unless a GPU encodes H.264 (ADR-0054).
    #[must_use]
    pub fn screen_share_codec(&self) -> VideoCodec {
        self.screen.codec
    }

    #[must_use]
    pub fn screen_share_announcement(&self) -> SharedString {
        self.screen.announcement.clone()
    }

    #[must_use]
    pub fn sound_applications(&self) -> &[SoundApplication] {
        &self.screen.sound_applications
    }

    #[must_use]
    pub fn chosen_sound_application(&self) -> Option<&SoundApplication> {
        self.screen.sound_application.as_ref()
    }

    #[must_use]
    pub fn screen_share_quality(cx: &gpui::App) -> ScreenShareQuality {
        preferences(cx).map_or_else(ScreenShareQuality::default, |preferences| {
            preferences.screen_share_quality
        })
    }

    #[must_use]
    pub fn is_screen_share_sound_on(cx: &gpui::App) -> bool {
        preferences(cx).is_none_or(|preferences| !preferences.is_screen_share_sound_off)
    }

    // ---- What the interface does ----

    /// The share button, and "switch source" while sharing: opens the in-app picker and lists
    /// the sources (a running share goes on meanwhile), or (Wayland) starts a share at once, the
    /// desktop's dialog asking what to share.
    pub fn start_screen_share(&mut self, cx: &mut Context<Self>) {
        if !self.can_share_screen()
            || self.screen.phase == ScreenSharePhase::Starting
            || self.screen.listing.is_some()
        {
            return;
        }
        self.screen.problem = None;
        if self.screen.picker == ScreenPicker::System {
            self.begin_screen_share(ScreenChoice::SystemPicker, cx);
            return;
        }
        self.screen.listing_generation += 1;
        let generation = self.screen.listing_generation;
        self.screen.listing = Some(ScreenSourceListing::Loading);
        let sources = Arc::clone(&self.screen.sources);
        let Some(listed) =
            run_on_own_thread("mellow-screen-sources", move || sources.sources(true))
        else {
            self.screen.listing = Some(ScreenSourceListing::Failed(
                ScreenShareProblem::CouldNotStart,
            ));
            cx.notify();
            return;
        };
        cx.spawn(async move |this, cx| {
            let Ok(listed) = listed.await else {
                return;
            };
            let _ = this.update(cx, |voice, cx| {
                if voice.screen.listing_generation != generation || voice.screen.listing.is_none() {
                    return;
                }
                voice.screen.listing = Some(match listed {
                    Ok(sources) => ScreenSourceListing::Listed(
                        sources
                            .into_iter()
                            .map(|source| ScreenSourceCard {
                                thumbnail: source.thumbnail.and_then(|picture| {
                                    VideoPicture::from_bgra(
                                        picture.width,
                                        picture.height,
                                        picture.bytes,
                                    )
                                }),
                                id: source.id,
                                kind: source.kind,
                                title: source.title,
                                application_name: source.application_name,
                            })
                            .collect(),
                    ),
                    Err(error) => ScreenSourceListing::Failed(
                        ScreenShareProblem::of_error(&error)
                            .unwrap_or(ScreenShareProblem::CouldNotStart),
                    ),
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// A card of the in-app picker: shares that source.
    pub fn share_screen_source(&mut self, source_id: String, cx: &mut Context<Self>) {
        if !self.can_share_screen() || self.screen.phase == ScreenSharePhase::Starting {
            return;
        }
        self.begin_screen_share(ScreenChoice::Source(source_id), cx);
    }

    /// The picker closed (a choice, or none): a running share goes on.
    pub fn close_screen_picker(&mut self, cx: &mut Context<Self>) {
        if self.screen.listing.take().is_some() {
            cx.notify();
        }
    }

    fn begin_screen_share(&mut self, choice: ScreenChoice, cx: &mut Context<Self>) {
        let Some(share_starter) = self
            .engine()
            .map(mellow_media::VoiceSession::screen_share_starter)
        else {
            return;
        };
        // Starting replaces a running share (switching source), as on the web.
        if let Some(share) = self.screen.share.take() {
            stop_off_the_ui_thread(share);
        }
        self.screen.generation += 1;
        let generation = self.screen.generation;
        self.screen.phase = ScreenSharePhase::Starting;
        self.screen.listing = None;
        self.screen.problem = None;
        let quality = Self::screen_share_quality(cx);
        let sound = self.share_sound(cx);
        let sources = Arc::clone(&self.screen.sources);
        let started_sound = sound.clone();
        let Some(answer) = run_on_own_thread("mellow-screen-share-start", move || {
            share_starter.start(sources.as_ref(), &choice, quality, &started_sound)
        }) else {
            self.screen.phase = ScreenSharePhase::Off;
            self.screen.problem = Some(ScreenShareProblem::CouldNotStart);
            cx.notify();
            return;
        };
        cx.spawn(async move |this, cx| {
            let Ok(started) = answer.await else {
                return;
            };
            let unclaimed = this
                .update(cx, |voice, cx| {
                    voice.take_started_screen_share(generation, started, &sound, cx)
                })
                .unwrap_or_else(|_| None);
            if let Some(share) = unclaimed {
                stop_off_the_ui_thread(share);
            }
        })
        .detach();
        cx.notify();
    }

    /// The sound the next share asks for.
    fn share_sound(&self, cx: &gpui::App) -> ShareSound {
        if !Self::is_screen_share_sound_on(cx) {
            return ShareSound::Off;
        }
        match self.screen.sound_application.as_ref() {
            Some(application) => ShareSound::Application(application.id.clone()),
            None => ShareSound::FromSource,
        }
    }

    /// The capture answered: kept if still wanted, else handed back to be stopped.
    fn take_started_screen_share(
        &mut self,
        generation: u64,
        started: Result<ScreenShare, ScreenCaptureError>,
        sound: &ShareSound,
        cx: &mut Context<Self>,
    ) -> Option<ScreenShare> {
        if self.screen.generation != generation || self.screen.phase != ScreenSharePhase::Starting {
            return started.ok();
        }
        let strings = crate::i18n::strings(cx);
        match started {
            Ok(share) if share.is_on() => {
                self.screen.phase = ScreenSharePhase::On;
                self.screen.codec = share.codec();
                self.screen.sound = match shared_sound_of(share.sound(), sound, self.screen.picker)
                {
                    SharedSound::Application(_) => SharedSound::Application(
                        self.screen
                            .sound_application
                            .as_ref()
                            .map(|application| application.name.clone())
                            .unwrap_or_default(),
                    ),
                    other => other,
                };
                self.screen.share = Some(share);
                self.screen.announcement = strings.screen_share().you_are_sharing().into();
                cx.notify();
                None
            }
            // Turned off again before its answer came (the server refused at once).
            Ok(share) => {
                self.screen.phase = ScreenSharePhase::Off;
                cx.notify();
                Some(share)
            }
            Err(error) => {
                self.screen.phase = ScreenSharePhase::Off;
                self.screen.problem = ScreenShareProblem::of_error(&error);
                if self.screen.problem.is_none() {
                    self.screen.announcement = strings.screen_share().problems().denied().into();
                } else {
                    tracing::info!(%error, "the screen share could not start");
                }
                cx.notify();
                None
            }
        }
    }

    /// Stop sharing.
    pub fn stop_screen_share(&mut self, cx: &mut Context<Self>) {
        if self.screen.phase == ScreenSharePhase::Off {
            return;
        }
        let was_on = self.screen.phase == ScreenSharePhase::On;
        self.screen.turn_off();
        if was_on {
            // Turned off at once for the server; the capture stops on its own thread.
            if let Some(engine) = self.engine() {
                engine.stop_video();
            }
            self.screen.announcement = crate::i18n::strings(cx)
                .desktop()
                .screen_share_source_closed()
                .into();
        }
        cx.notify();
    }

    /// Remembered for the next share, and applied at once to the running one.
    pub fn set_screen_share_quality(
        &mut self,
        quality: ScreenShareQuality,
        cx: &mut Context<Self>,
    ) {
        change_preferences(cx, move |preferences| {
            preferences.screen_share_quality = quality;
        });
        if self.screen.phase == ScreenSharePhase::On
            && let Some(engine) = self.engine()
        {
            engine.change_screen_share_quality(quality);
        }
        cx.notify();
    }

    /// Remembered for the next share (a running capture keeps its sound).
    pub fn set_screen_share_sound_on(&mut self, is_on: bool, cx: &mut Context<Self>) {
        change_preferences(cx, move |preferences| {
            preferences.is_screen_share_sound_off = !is_on;
        });
        cx.notify();
    }

    /// The application whose sound the next share sends, or the source's own with `None`.
    pub fn choose_sound_application(
        &mut self,
        application: Option<SoundApplication>,
        cx: &mut Context<Self>,
    ) {
        self.screen.sound_application = application;
        cx.notify();
    }

    /// Lists the applications playing sound, where they are offered (Wayland), off the UI
    /// thread; the share options show them once known.
    pub fn refresh_sound_applications(&mut self, cx: &mut Context<Self>) {
        if self.screen.picker != ScreenPicker::System {
            return;
        }
        let sources = Arc::clone(&self.screen.sources);
        let Some(listed) = run_on_own_thread("mellow-sound-applications", move || {
            sources.sound_applications()
        }) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let Ok(applications) = listed.await else {
                return;
            };
            let _ = this.update(cx, |voice, cx| {
                // A chosen application that stopped playing is forgotten.
                if let Some(chosen) = voice.screen.sound_application.as_ref()
                    && !applications.iter().any(|listed| listed.id == chosen.id)
                {
                    voice.screen.sound_application = None;
                }
                voice.screen.sound_applications = applications;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn dismiss_screen_share_problem(&mut self, cx: &mut Context<Self>) {
        self.screen.problem = None;
        cx.notify();
    }

    /// The engine's word on the share: it ended by itself, or the instance refused or stopped
    /// the video. Only a share (starting or on) is concerned; the camera has its own.
    pub(super) fn follow_screen_share_event(
        &mut self,
        event: &VoiceSessionEvent,
        cx: &mut Context<Self>,
    ) {
        if !matches!(
            self.screen.phase,
            ScreenSharePhase::On | ScreenSharePhase::Starting
        ) {
            return;
        }
        let problem = match event {
            VoiceSessionEvent::ScreenShareEnded(ScreenShareEnd::SourceClosed) => None,
            VoiceSessionEvent::ScreenShareEnded(ScreenShareEnd::Failed(_)) => {
                Some(ScreenShareProblem::CaptureFailed)
            }
            VoiceSessionEvent::VideoRefused(refusal) => Some(ScreenShareProblem::Refused(*refusal)),
            VoiceSessionEvent::VideoStoppedByServer => Some(ScreenShareProblem::StoppedByServer),
            _ => return,
        };
        // A refusal while the capture starts: its answer finds the share off and is dropped.
        self.screen.turn_off();
        self.screen.problem = problem;
        if problem.is_none() {
            self.screen.announcement = crate::i18n::strings(cx)
                .desktop()
                .screen_share_source_closed()
                .into();
        }
        cx.notify();
    }
}

#[cfg(test)]
impl Voice {
    /// Shares from `sources` instead of the system's.
    pub(crate) fn set_screen_sources_for_tests(&mut self, sources: Arc<dyn ScreenSources>) {
        self.screen = ScreenSharing::new(sources);
    }

    /// As the engine's events, for views drawn without an engine.
    pub(crate) fn follow_screen_share_event_for_tests(
        &mut self,
        event: &VoiceSessionEvent,
        cx: &mut Context<Self>,
    ) {
        self.follow_screen_share_event(event, cx);
    }

    /// A phase reached without an engine, for views drawn without a server.
    pub(crate) fn pretend_screen_share_phase_for_tests(
        &mut self,
        phase: ScreenSharePhase,
        sound: SharedSound,
        codec: VideoCodec,
        cx: &mut Context<Self>,
    ) {
        self.screen.phase = phase;
        self.screen.sound = sound;
        self.screen.codec = codec;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualities_read_back_from_their_labels_and_nothing_else() {
        for quality in SCREEN_SHARE_QUALITIES {
            assert_eq!(
                screen_share_quality_of_label(screen_share_quality_label(quality)),
                Some(quality)
            );
        }
        assert_eq!(screen_share_quality_of_label("4k"), None);
    }

    #[test]
    fn a_cancelled_choice_is_no_problem_and_the_rest_are() {
        assert_eq!(
            ScreenShareProblem::of_error(&ScreenCaptureError::Cancelled),
            None
        );
        assert_eq!(
            ScreenShareProblem::of_error(&ScreenCaptureError::PermissionDenied),
            Some(ScreenShareProblem::PermissionDenied)
        );
        assert_eq!(
            ScreenShareProblem::of_error(&ScreenCaptureError::System("x".into())),
            Some(ScreenShareProblem::CouldNotStart)
        );
    }

    #[test]
    fn the_shared_sound_says_why_there_is_none() {
        let from_source = ShareSound::FromSource;
        assert_eq!(
            shared_sound_of(ShareSoundState::Captured, &from_source, ScreenPicker::InApp),
            SharedSound::FromSource
        );
        assert_eq!(
            shared_sound_of(ShareSoundState::Off, &ShareSound::Off, ScreenPicker::InApp),
            SharedSound::Off
        );
        assert_eq!(
            shared_sound_of(
                ShareSoundState::NotAvailable,
                &from_source,
                ScreenPicker::System
            ),
            SharedSound::NotAvailableOnWayland
        );
        assert_eq!(
            shared_sound_of(
                ShareSoundState::NotAvailable,
                &from_source,
                ScreenPicker::InApp
            ),
            SharedSound::NotAvailableOnThisSystem
        );
    }
}
