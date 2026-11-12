//! Push-to-talk while another app is focused (ADR-0053). The key is held only while the reader is
//! in a voice channel with push-to-talk chosen and a key recorded
//! ([`global_push_to_talk_code`]); it is released as soon as one of those stops.
//!
//! Per system:
//! - **macOS**: a Carbon hot key (`global-hotkey`). It needs no permission (neither Input
//!   Monitoring nor Accessibility), so the app has nothing to ask for; it does not fire while a
//!   password field has Secure Input on, as for every app.
//! - **Windows**: a Win32 hot key (`RegisterHotKey`); its release is noticed by polling the key
//!   every 50 ms.
//! - **Linux, X11 session**: an X11 key grab on the root window, from `global-hotkey`'s own thread
//!   (it reads the X server every 50 ms).
//! - **Linux, Wayland session** (`WAYLAND_DISPLAY` or `XDG_SESSION_TYPE=wayland`): the XDG
//!   `GlobalShortcuts` portal (KDE Plasma, GNOME 48 and later). The stored key is only the preferred
//!   trigger: the compositor asks the person to confirm or pick another, and the panel shows the
//!   binding it reports. Without the portal (wlroots compositors, older GNOME) and with an
//!   `XWayland` display, the X11 grab is used instead, which only hears keys while an X11 window is
//!   focused, and the panel says so.
//!
//! Hot keys grab the key system-wide: while held, a character key stops typing in other apps, and
//! the panel says so. A modifier key alone cannot be a hot key, so it keeps working only in the
//! window. The in-window handler keeps running next to the listener: pressing and releasing are
//! idempotent, so a key both hear presses once.
//!
//! Threads: macOS and Windows register on the UI thread (their hot keys belong to the thread
//! running the event loop; the calls are short). On Linux each binding gets a thread that talks
//! to the portal or the X server, so nothing waits on them on the UI thread. Key events reach the
//! UI through a bounded queue; key events are never logged.

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod hotkey_listener;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod linux_listener;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Once};

use global_hotkey::GlobalHotKeyEvent;
use global_hotkey::hotkey::HotKey;
use gpui::{App, AppContext as _, Context, Entity, SharedString, Subscription, Task};
use mellow_client_core::shared_logic::push_to_talk_keys::{
    global_hotkey_accelerator_of_code, global_push_to_talk_code,
};
use tokio::sync::mpsc;

#[cfg(any(target_os = "macos", target_os = "windows"))]
use self::hotkey_listener::Listener;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use self::linux_listener::Listener;
use super::{Voice, current_settings};
use crate::preferences::PreferencesStore;

/// Key presses and releases are a few per second; the UI drains them a burst at a time.
const LISTENER_EVENT_QUEUE_CAPACITY: usize = 32;

/// No binding: events stamped with it are dropped.
const NO_BINDING: u64 = 0;

/// Whether the push-to-talk key works while another app is focused, as the voice panel says it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum GlobalPushToTalkStatus {
    /// Not in a voice channel with push-to-talk, or nothing to say yet (the portal's dialog may
    /// be open).
    #[default]
    Off,
    /// `trigger`: the binding the portal reports, when it chose one. `does_key_stop_typing`: a
    /// grabbed character key no longer types in other apps.
    Listening {
        trigger: Option<SharedString>,
        does_key_stop_typing: bool,
    },
    /// Wayland without the portal: the X11 grab hears keys only while an X11 window is focused.
    ListeningWhileX11WindowFocused,
    /// A modifier key alone cannot be grabbed.
    ModifierKeyWindowOnly,
    /// The system refused the key (another app holds it, or it has no hot key code).
    KeyUnavailable,
    /// The person declined the portal's shortcut.
    PortalRefused,
    /// Wayland without the portal or an X11 display.
    NoPortal,
}

/// What a listener reports, stamped with the binding it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ListenerEvent {
    Key {
        binding: u64,
        is_pressed: bool,
    },
    #[cfg_attr(
        any(target_os = "macos", target_os = "windows"),
        expect(
            dead_code,
            reason = "only the Linux listener threads report a status later"
        )
    )]
    Status {
        binding: u64,
        status: GlobalPushToTalkStatus,
    },
}

/// The hot key `global-hotkey` grabs for `code`; `None` for a modifier alone or a code it does
/// not know.
fn hotkey_of_code(code: &str) -> Option<HotKey> {
    global_hotkey_accelerator_of_code(code)?.parse().ok()
}

/// The status of a hot key that `code` could not become.
fn status_without_hotkey(code: &str) -> GlobalPushToTalkStatus {
    if global_hotkey_accelerator_of_code(code).is_none() {
        GlobalPushToTalkStatus::ModifierKeyWindowOnly
    } else {
        GlobalPushToTalkStatus::KeyUnavailable
    }
}

/// Sends `global-hotkey`'s events into `events`, stamped with the binding current when the system
/// delivered them. The crate's handler is process-wide and set once.
fn install_hotkey_event_handler(events: mpsc::Sender<ListenerEvent>, binding: Arc<AtomicU64>) {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(move || {
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            let binding = binding.load(Ordering::Acquire);
            if binding == NO_BINDING {
                return;
            }
            let is_pressed = event.state() == global_hotkey::HotKeyState::Pressed;
            send_listener_event(
                &events,
                ListenerEvent::Key {
                    binding,
                    is_pressed,
                },
            );
        }));
    });
}

/// Never blocks: called from the system's event callbacks.
fn send_listener_event(events: &mpsc::Sender<ListenerEvent>, event: ListenerEvent) {
    if events.try_send(event).is_err() {
        tracing::warn!("global push-to-talk event dropped: the interface is not keeping up");
    }
}

/// Follows the voice channel and the settings, and holds the key while push-to-talk needs it.
struct GlobalPushToTalk {
    voice: Entity<Voice>,
    listener: Listener,
    bound_code: Option<String>,
    binding: u64,
    _event_pump: Task<()>,
    _subscriptions: Vec<Subscription>,
}

/// Starts the listener for the app's voice.
pub fn install_global_push_to_talk(voice: &Entity<Voice>, cx: &mut App) {
    let global_push_to_talk = cx.new(|cx| GlobalPushToTalk::new(voice.clone(), cx));
    // Lives as long as the app: the voice it follows does.
    cx.set_global(GlobalPushToTalkGlobal {
        _entity: global_push_to_talk,
    });
}

struct GlobalPushToTalkGlobal {
    _entity: Entity<GlobalPushToTalk>,
}

impl gpui::Global for GlobalPushToTalkGlobal {}

impl GlobalPushToTalk {
    fn new(voice: Entity<Voice>, cx: &mut Context<Self>) -> Self {
        let (sender, mut receiver) = mpsc::channel(LISTENER_EVENT_QUEUE_CAPACITY);
        let event_pump = cx.spawn(async move |this, cx| {
            while let Some(event) = receiver.recv().await {
                if this
                    .update(cx, |this, cx| this.follow_listener(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let subscriptions = vec![
            cx.observe(&voice, |this, _, cx| this.follow_voice(cx)),
            cx.observe_global::<PreferencesStore>(Self::follow_voice),
        ];
        let mut this = Self {
            voice,
            listener: Listener::new(sender),
            bound_code: None,
            binding: NO_BINDING,
            _event_pump: event_pump,
            _subscriptions: subscriptions,
        };
        this.follow_voice(cx);
        this
    }

    /// Binds, rebinds or unbinds the key when the channel, the mode or the key changed.
    fn follow_voice(&mut self, cx: &mut Context<Self>) {
        let settings = current_settings(cx);
        let is_in_voice_channel = self.voice.read(cx).channel_id().is_some();
        let wanted = global_push_to_talk_code(&settings, is_in_voice_channel);
        if wanted == self.bound_code.as_deref() {
            return;
        }
        let wanted = wanted.map(str::to_owned);
        if self.bound_code.take().is_some() {
            self.listener.unbind();
            self.binding = NO_BINDING;
            self.voice.update(cx, |voice, cx| {
                voice.global_push_to_talk_key_changed(false, cx);
            });
        }
        let status = match wanted {
            Some(code) => {
                self.binding = next_binding();
                let shortcut_name = crate::i18n::strings(cx)
                    .desktop()
                    .global_push_to_talk_shortcut_name();
                let status = self.listener.bind(&code, self.binding, shortcut_name);
                self.bound_code = Some(code);
                status
            }
            None => GlobalPushToTalkStatus::Off,
        };
        self.set_status(status, cx);
    }

    fn follow_listener(&mut self, event: ListenerEvent, cx: &mut Context<Self>) {
        match event {
            ListenerEvent::Key {
                binding,
                is_pressed,
            } if binding == self.binding => {
                self.voice.update(cx, |voice, cx| {
                    voice.global_push_to_talk_key_changed(is_pressed, cx);
                });
            }
            ListenerEvent::Status { binding, status } if binding == self.binding => {
                self.set_status(status, cx);
            }
            ListenerEvent::Key { .. } | ListenerEvent::Status { .. } => {}
        }
    }

    fn set_status(&self, status: GlobalPushToTalkStatus, cx: &mut Context<Self>) {
        self.voice.update(cx, |voice, cx| {
            voice.set_global_push_to_talk_status(status, cx);
        });
    }
}

/// Bindings are numbered for the whole process, so a late event from a listener thread of an
/// earlier binding is never taken for the current one.
fn next_binding() -> u64 {
    static NEXT_BINDING: AtomicU64 = AtomicU64::new(NO_BINDING + 1);
    NEXT_BINDING.fetch_add(1, Ordering::Relaxed)
}
