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
    does_code_type_a_character, portal_trigger_of_code,
};
use tokio::sync::{mpsc, oneshot};

use super::{
    GlobalPushToTalkStatus, ListenerEvent, NO_BINDING, hotkey_of_code,
    install_hotkey_event_handler, send_listener_event, status_without_hotkey,
};

/// The shortcut's id in the portal session; the compositor remembers the binding under it.
const PORTAL_SHORTCUT_ID: &str = "push-to-talk";

/// Whether GPUI runs on Wayland: it picks Wayland when `WAYLAND_DISPLAY` is set.
fn is_wayland_session(wayland_display: Option<&OsStr>, session_type: Option<&OsStr>) -> bool {
    wayland_display.is_some_and(|display| !display.is_empty())
        || session_type.is_some_and(|session_type| session_type == "wayland")
}

pub(super) struct Listener {
    events: mpsc::Sender<ListenerEvent>,
    /// Read by the X11 grab's callback to stamp its events.
    binding: Arc<AtomicU64>,
    /// Dropping it tells the binding's thread to let the key go.
    stop: Option<oneshot::Sender<()>>,
    last_thread: Option<JoinHandle<()>>,
}

impl Listener {
    pub(super) fn new(events: mpsc::Sender<ListenerEvent>) -> Self {
        Self {
            events,
            binding: Arc::new(AtomicU64::new(NO_BINDING)),
            stop: None,
            last_thread: None,
        }
    }

    /// Starts listening for `code` on a thread, which reports the status when it knows it.
    pub(super) fn bind(
        &mut self,
        code: &str,
        binding: u64,
        shortcut_name: &str,
    ) -> GlobalPushToTalkStatus {
        install_hotkey_event_handler(self.events.clone(), Arc::clone(&self.binding));
        self.binding.store(binding, Ordering::Release);
        let (stop, stopped) = oneshot::channel();
        let job = Job {
            code: code.to_owned(),
            shortcut_name: shortcut_name.to_owned(),
            binding,
            is_wayland: is_wayland_session(
                std::env::var_os("WAYLAND_DISPLAY").as_deref(),
                std::env::var_os("XDG_SESSION_TYPE").as_deref(),
            ),
            has_x11_display: std::env::var_os("DISPLAY").is_some_and(|display| !display.is_empty()),
            events: self.events.clone(),
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
                block_on(job.run());
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

    pub(super) fn unbind(&mut self) {
        self.binding.store(NO_BINDING, Ordering::Release);
        self.stop = None;
    }
}

/// One binding's work, on its thread.
struct Job {
    code: String,
    shortcut_name: String,
    binding: u64,
    is_wayland: bool,
    has_x11_display: bool,
    events: mpsc::Sender<ListenerEvent>,
    stopped: oneshot::Receiver<()>,
}

/// How the portal attempt ended.
enum PortalOutcome {
    Stopped,
    Missing,
    Refused,
    Failed,
}

impl Job {
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
                    return self.report(GlobalPushToTalkStatus::KeyUnavailable);
                }
            }
        } else {
            GlobalPushToTalkStatus::Listening {
                trigger: None,
                does_key_stop_typing: does_code_type_a_character(&self.code),
            }
        };
        self.listen_through_x11(x11_status).await;
    }

    fn report(&self, status: GlobalPushToTalkStatus) {
        send_listener_event(
            &self.events,
            ListenerEvent::Status {
                binding: self.binding,
                status,
            },
        );
    }

    fn report_key(&self, is_pressed: bool) {
        send_listener_event(
            &self.events,
            ListenerEvent::Key {
                binding: self.binding,
                is_pressed,
            },
        );
    }

    async fn listen_through_portal(&mut self) -> PortalOutcome {
        register_host_app_once().await;
        let portal = match GlobalShortcuts::new().await {
            Ok(portal) => portal,
            Err(error) => {
                tracing::info!(%error, "no global shortcuts portal");
                return PortalOutcome::Missing;
            }
        };
        let session = match portal.create_session(CreateSessionOptions::default()).await {
            Ok(session) => session,
            Err(error) => {
                tracing::warn!(%error, "the global shortcuts portal refused a session");
                return PortalOutcome::Failed;
            }
        };
        let mut stopped = std::mem::replace(&mut self.stopped, oneshot::channel().1);
        let outcome = or(
            async {
                // A dropped sender is the stop signal.
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
        self.report(listening_with(shortcut.trigger_description()));
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
                PortalSignal::Rebound(trigger) => self.report(listening_with(&trigger)),
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

fn listening_with(trigger_description: &str) -> GlobalPushToTalkStatus {
    GlobalPushToTalkStatus::Listening {
        trigger: (!trigger_description.is_empty()).then(|| trigger_description.to_owned().into()),
        // The compositor may have chosen another key: say only which.
        does_key_stop_typing: false,
    }
}

/// Tells the portal which app this unsandboxed process is, once per process and before any other
/// portal call on the connection, so the compositor names and remembers the shortcut as Mellow's.
async fn register_host_app_once() {
    static HAS_TRIED: AtomicBool = AtomicBool::new(false);
    if HAS_TRIED.swap(true, Ordering::AcqRel) {
        return;
    }
    let registered = match ashpd::AppID::try_from(crate::APP_IDENTIFIER) {
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
