//! Linux: the XDG `GlobalShortcuts` portal on Wayland, an X11 key grab on X11 (and on Wayland
//! without the portal, through `XWayland`). Each binding runs on its own thread, which first waits
//! for the previous one to let the key go.

use std::ffi::OsStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;

use ashpd::desktop::CreateSessionOptions;
use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use futures_lite::StreamExt as _;
use futures_lite::future::{block_on, or};
use global_hotkey::GlobalHotKeyManager;
use mellow_client_core::shared_logic::push_to_talk_keys::{
    does_grab_take_key_from_other_apps, portal_trigger_of_code,
};
use tokio::sync::oneshot;

use super::{
    GlobalPushToTalkStatus, KeyListener, ListenerReports, NO_BINDING, hotkey_of_code,
    install_hotkey_event_handler, status_without_hotkey,
};

/// The shortcut's id in the portal session; the compositor remembers the binding under it.
const PORTAL_SHORTCUT_ID: &str = "push-to-talk";

/// Whether GPUI runs on Wayland: it picks Wayland when `WAYLAND_DISPLAY` is set.
fn is_wayland_session(wayland_display: Option<&OsStr>, session_type: Option<&OsStr>) -> bool {
    wayland_display.is_some_and(|display| !display.is_empty())
        || session_type.is_some_and(|session_type| session_type == "wayland")
}

pub(super) struct Listener {
    reports: ListenerReports,
    /// Read by the X11 grab's callback to stamp its events.
    binding: Arc<AtomicU64>,
    /// Dropping it tells the binding's thread to let the key go.
    stop: Option<oneshot::Sender<()>>,
    last_thread: Option<JoinHandle<()>>,
}

impl Listener {
    pub(super) fn new(reports: ListenerReports) -> Self {
        Self {
            reports,
            binding: Arc::new(AtomicU64::new(NO_BINDING)),
            stop: None,
            last_thread: None,
        }
    }
}

impl KeyListener for Listener {
    /// Starts listening for `code` on a thread, which reports the status when it knows it.
    fn bind(&mut self, code: &str, binding: u64, shortcut_name: &str) -> GlobalPushToTalkStatus {
        install_hotkey_event_handler(self.reports.clone(), Arc::clone(&self.binding));
        self.binding.store(binding, Ordering::Release);
        let (stop, stopped) = oneshot::channel();
        let listener_binding = ListenerBinding {
            code: code.to_owned(),
            shortcut_name: shortcut_name.to_owned(),
            binding,
            is_wayland: is_wayland_session(
                std::env::var_os("WAYLAND_DISPLAY").as_deref(),
                std::env::var_os("XDG_SESSION_TYPE").as_deref(),
            ),
            has_x11_display: std::env::var_os("DISPLAY").is_some_and(|display| !display.is_empty()),
            reports: self.reports.clone(),
            stopped,
        };
        let previous = self.last_thread.take();
        let spawned = std::thread::Builder::new()
            .name("mellow-push-to-talk".into())
            .spawn(move || {
                if let Some(previous) = previous
                    && previous.join().is_err()
                {
                    tracing::warn!("the previous push-to-talk listener ended with a panic");
                }
                block_on(listener_binding.run());
            });
        match spawned {
            Ok(thread) => {
                self.stop = Some(stop);
                self.last_thread = Some(thread);
                GlobalPushToTalkStatus::Off
            }
            Err(error) => {
                tracing::warn!(error = %error.kind(), "cannot start the push-to-talk listener");
                self.binding.store(NO_BINDING, Ordering::Release);
                GlobalPushToTalkStatus::KeyUnavailable
            }
        }
    }

    fn unbind(&mut self) {
        self.binding.store(NO_BINDING, Ordering::Release);
        self.stop = None;
    }
}

/// One binding's work, on its thread.
struct ListenerBinding {
    code: String,
    shortcut_name: String,
    binding: u64,
    is_wayland: bool,
    has_x11_display: bool,
    reports: ListenerReports,
    stopped: oneshot::Receiver<()>,
}

/// How the portal attempt ended.
enum PortalOutcome {
    Stopped,
    Missing,
    Refused,
    Failed,
}

impl ListenerBinding {
    async fn run(mut self) {
        let x11_status = if self.is_wayland {
            match self.listen_through_portal().await {
                PortalOutcome::Stopped => return,
                PortalOutcome::Missing if self.has_x11_display => {
                    GlobalPushToTalkStatus::ListeningWhileX11WindowFocused
                }
                PortalOutcome::Missing => return self.report(GlobalPushToTalkStatus::NoPortal),
                PortalOutcome::Refused => {
                    return self.report(GlobalPushToTalkStatus::PortalRefused);
                }
                PortalOutcome::Failed => {
                    return self.report(GlobalPushToTalkStatus::PortalFailed);
                }
            }
        } else {
            GlobalPushToTalkStatus::Listening {
                trigger: None,
                is_key_taken_from_other_apps: does_grab_take_key_from_other_apps(&self.code),
            }
        };
        self.listen_through_x11(x11_status).await;
    }

    fn report(&self, status: GlobalPushToTalkStatus) {
        self.reports.status(self.binding, status);
    }

    fn report_key(&self, is_pressed: bool) {
        self.reports.key(self.binding, is_pressed);
    }

    /// Every portal call waits under the stop signal: a portal that never answers keeps no
    /// thread once the binding ends.
    async fn listen_through_portal(&mut self) -> PortalOutcome {
        let mut stopped = std::mem::replace(&mut self.stopped, oneshot::channel().1);
        let opened = or(
            async {
                // A dropped sender is the stop signal.
                let _ = (&mut stopped).await;
                Err(PortalOutcome::Stopped)
            },
            open_portal_session(),
        )
        .await;
        let (portal, session) = match opened {
            Ok(opened) => opened,
            Err(outcome) => {
                self.stopped = stopped;
                return outcome;
            }
        };
        let outcome = or(
            async {
                let _ = (&mut stopped).await;
                PortalOutcome::Stopped
            },
            self.bind_and_listen(&portal, &session),
        )
        .await;
        self.stopped = stopped;
        // Also dismisses the compositor's dialog when stopped while it is open.
        if let Err(error) = session.close().await {
            tracing::debug!(%error, "the global shortcuts session did not close");
        }
        outcome
    }

    async fn bind_and_listen(
        &self,
        portal: &GlobalShortcuts,
        session: &ashpd::desktop::Session<GlobalShortcuts>,
    ) -> PortalOutcome {
        let trigger = portal_trigger_of_code(&self.code);
        let shortcut = NewShortcut::new(PORTAL_SHORTCUT_ID, self.shortcut_name.as_str())
            .preferred_trigger(trigger.as_deref());
        // Subscribed before binding, so a press right after the dialog is not missed.
        let (Ok(activated), Ok(deactivated), Ok(changed)) = (
            portal.receive_activated().await,
            portal.receive_deactivated().await,
            portal.receive_shortcuts_changed().await,
        ) else {
            return PortalOutcome::Failed;
        };
        let request = portal
            .bind_shortcuts(session, &[shortcut], None, BindShortcutsOptions::default())
            .await;
        let bound = match request.and_then(|request| request.response()) {
            Ok(bound) => bound,
            Err(ashpd::Error::Response(_)) => return PortalOutcome::Refused,
            Err(error) => {
                tracing::warn!(%error, "the global shortcuts portal could not bind");
                return PortalOutcome::Failed;
            }
        };
        let Some(shortcut) = bound
            .shortcuts()
            .iter()
            .find(|shortcut| shortcut.id() == PORTAL_SHORTCUT_ID)
        else {
            return PortalOutcome::Refused;
        };
        self.report(status_of_trigger(shortcut.trigger_description()));
        let mut signals = activated
            .filter(|signal| signal.shortcut_id() == PORTAL_SHORTCUT_ID)
            .map(|_| PortalSignal::Pressed)
            .or(deactivated
                .filter(|signal| signal.shortcut_id() == PORTAL_SHORTCUT_ID)
                .map(|_| PortalSignal::Released))
            .or(changed.filter_map(|signal| {
                signal
                    .shortcuts()
                    .iter()
                    .find(|shortcut| shortcut.id() == PORTAL_SHORTCUT_ID)
                    .map(|shortcut| {
                        PortalSignal::Rebound(shortcut.trigger_description().to_owned())
                    })
            }));
        while let Some(signal) = signals.next().await {
            match signal {
                PortalSignal::Pressed => self.report_key(true),
                PortalSignal::Released => self.report_key(false),
                PortalSignal::Rebound(trigger) => {
                    // A key held while its binding was removed would never come up.
                    if trigger.is_empty() {
                        self.report_key(false);
                    }
                    self.report(status_of_trigger(&trigger));
                }
            }
        }
        // The portal went away (its service restarted): a held key would never come up; the
        // window still hears it.
        self.report_key(false);
        PortalOutcome::Failed
    }

    async fn listen_through_x11(self, listening: GlobalPushToTalkStatus) {
        let Some(hotkey) = hotkey_of_code(&self.code) else {
            return self.report(status_without_hotkey(&self.code));
        };
        let manager = match GlobalHotKeyManager::new() {
            Ok(manager) => manager,
            Err(error) => {
                tracing::warn!(%error, "global push-to-talk is unavailable");
                return self.report(GlobalPushToTalkStatus::KeyUnavailable);
            }
        };
        // shortcut: global-hotkey reports success when it could not reach the X server at all
        // (its thread ended), so a broken DISPLAY shows as listening; check the connection
        // ourselves if that is ever reported.
        if manager.register(hotkey).is_err() {
            // The error names the key: left out of the log.
            tracing::warn!("the push-to-talk key could not be grabbed on X11");
            return self.report(GlobalPushToTalkStatus::KeyUnavailable);
        }
        self.report(listening);
        // A dropped sender is the stop signal.
        let _ = self.stopped.await;
        if manager.unregister(hotkey).is_err() {
            tracing::warn!("the push-to-talk key could not be released on X11");
        }
    }
}

enum PortalSignal {
    Pressed,
    Released,
    Rebound(String),
}

/// What the panel says of the binding the portal reports: a key, or none chosen yet.
fn status_of_trigger(trigger_description: &str) -> GlobalPushToTalkStatus {
    if trigger_description.is_empty() {
        return GlobalPushToTalkStatus::PortalBoundWithoutKey;
    }
    GlobalPushToTalkStatus::Listening {
        trigger: Some(trigger_description.to_owned().into()),
        // The compositor may have chosen another key: say only which.
        is_key_taken_from_other_apps: false,
    }
}

/// Opens a portal session. A failure to open one counts as no portal: without the service,
/// ashpd still builds the proxy (it reads a missing version as 1) and only the session fails,
/// and the `XWayland` grab may still work.
async fn open_portal_session()
-> Result<(GlobalShortcuts, ashpd::desktop::Session<GlobalShortcuts>), PortalOutcome> {
    register_host_app_once().await;
    let portal = GlobalShortcuts::new().await.map_err(|error| {
        tracing::info!(%error, "no global shortcuts portal");
        PortalOutcome::Missing
    })?;
    let session = portal
        .create_session(CreateSessionOptions::default())
        .await
        .map_err(|error| {
            tracing::info!(%error, "the global shortcuts portal opened no session");
            PortalOutcome::Missing
        })?;
    Ok((portal, session))
}

/// Tells the portal which app this unsandboxed process is, once per process and before any other
/// portal call on the connection, so the compositor names and remembers the shortcut as Mellow's.
async fn register_host_app_once() {
    static HAS_TRIED: AtomicBool = AtomicBool::new(false);
    if HAS_TRIED.swap(true, Ordering::AcqRel) {
        return;
    }
    let registered = match ashpd::AppID::try_from(&*crate::profile::profile_app_identifier(
        crate::APP_IDENTIFIER,
    )) {
        Ok(app_id) => ashpd::register_host_app(app_id).await,
        Err(error) => Err(error),
    };
    if let Err(error) = registered {
        // Older portals have no registry: they guess the app from the process.
        tracing::debug!(%error, "the portal registry did not take the app's id");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_portal_binding_without_a_key_is_not_called_listening() {
        assert_eq!(
            status_of_trigger(""),
            GlobalPushToTalkStatus::PortalBoundWithoutKey
        );
        assert_eq!(
            status_of_trigger("Ctrl+V"),
            GlobalPushToTalkStatus::Listening {
                trigger: Some("Ctrl+V".into()),
                is_key_taken_from_other_apps: false,
            }
        );
    }

    #[test]
    fn follows_gpui_in_telling_wayland_from_x11() {
        let wayland = Some(OsStr::new("wayland"));
        assert!(is_wayland_session(Some(OsStr::new("wayland-0")), None));
        assert!(is_wayland_session(None, wayland));
        assert!(!is_wayland_session(
            Some(OsStr::new("")),
            Some(OsStr::new("x11"))
        ));
        assert!(!is_wayland_session(None, None));
    }
}
