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
//! window. A hot key fires only with exactly its modifiers: while Shift or Control is held (to
//! sprint or crouch in a game), the key does not reach the listener; grabbing every combination
//! would take Ctrl+V and Shift+letters from every app, so this stays a limit (ADR-0053).
//!
//! On macOS, Carbon names a key by its position on an ANSI keyboard while the stored code names
//! the character the layout types (GPUI records keys so); `macos_keyboard_layout` finds the
//! position typing it on the current layout, and the key is taken again when the layout changes.
//!
//! The in-window handler keeps running next to the listener: pressing and releasing are
//! idempotent, so a key both hear presses once.
//!
//! Threads: macOS and Windows register on the UI thread (their hot keys belong to the thread
//! running the event loop; the calls are short). On Linux each binding gets a thread that talks
//! to the portal or the X server, so nothing waits on them on the UI thread. Key events reach the
//! UI through a bounded queue, which only wakes the UI: the newest key state is kept apart, so a
//! full queue never loses a release. Key events are never logged.

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod hotkey_listener;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod linux_listener;
#[cfg(target_os = "macos")]
mod macos_keyboard_layout;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Once};

use global_hotkey::GlobalHotKeyEvent;
use global_hotkey::hotkey::HotKey;
use gpui::{App, AppContext as _, Context, Entity, SharedString, Subscription, Task};
use mellow_client_core::shared_logic::push_to_talk_keys::{
    global_push_to_talk_code, modifier_of_code, portal_trigger_of_code,
};
use tokio::sync::mpsc;

#[cfg(any(target_os = "macos", target_os = "windows"))]
use self::hotkey_listener::Listener;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use self::linux_listener::Listener;
use super::{Voice, current_settings};
use crate::preferences::PreferencesStore;

/// Statuses are a few per binding and key changes only wake the UI; the UI drains them a burst
/// at a time.
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
    /// `trigger`: the binding the portal reports. `is_key_taken_from_other_apps`: a grabbed key
    /// does nothing else in other apps during the call.
    Listening {
        trigger: Option<SharedString>,
        is_key_taken_from_other_apps: bool,
    },
    /// Wayland without the portal: the X11 grab hears keys only while an X11 window is focused.
    ListeningWhileX11WindowFocused,
    /// The portal took the shortcut but no key triggers it yet (Hyprland until the person binds
    /// it, or a binding removed in the system's settings).
    PortalBoundWithoutKey,
    /// A modifier key alone cannot be grabbed.
    ModifierKeyWindowOnly,
    /// The system refused the key (another app holds it, or it has no hot key code).
    KeyUnavailable,
    /// macOS: no key of the current keyboard layout types the recorded key's character.
    KeyNotOnKeyboardLayout,
    /// The person declined the portal's shortcut.
    PortalRefused,
    /// Wayland without the portal or an X11 display.
    NoPortal,
    /// The portal answered with an error after its session opened, or went away.
    PortalFailed,
}

/// What a listener reports, stamped with the binding it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ListenerEvent {
    /// The newest key state changed: read it with [`ListenerReports::latest_key`].
    KeyChanged,
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

/// Where listeners report, from any thread and without blocking. The key's state is kept apart
/// from the queue so a full queue never loses a release: the queue only wakes the UI, which
/// reads the newest state.
#[derive(Debug, Clone)]
struct ListenerReports {
    queue: mpsc::Sender<ListenerEvent>,
    /// The binding shifted left once, and the lowest bit set while the key is down.
    latest_key: Arc<AtomicU64>,
}

impl ListenerReports {
    fn new(queue: mpsc::Sender<ListenerEvent>) -> Self {
        Self {
            queue,
            latest_key: Arc::new(AtomicU64::new(NO_BINDING)),
        }
    }

    /// Never blocks: called from the system's event callbacks.
    fn key(&self, binding: u64, is_pressed: bool) {
        let packed = (binding << 1) | u64::from(is_pressed);
        // A late report of an earlier binding never hides the current binding's state.
        let is_newest = self
            .latest_key
            .try_update(Ordering::AcqRel, Ordering::Acquire, |previous| {
                (previous >> 1 <= binding).then_some(packed)
            })
            .is_ok();
        // A full queue already holds a wake-up, which will read the state stored above; a closed
        // one means the app is quitting.
        if is_newest
            && let Err(mpsc::error::TrySendError::Full(_)) =
                self.queue.try_send(ListenerEvent::KeyChanged)
        {
            tracing::debug!("global push-to-talk wake-up merged into a pending one");
        }
    }

    /// Never blocks: called from listener threads.
    #[cfg_attr(
        any(target_os = "macos", target_os = "windows"),
        expect(
            dead_code,
            reason = "only the Linux listener threads report a status later"
        )
    )]
    fn status(&self, binding: u64, status: GlobalPushToTalkStatus) {
        if self
            .queue
            .try_send(ListenerEvent::Status { binding, status })
            .is_err()
        {
            tracing::warn!("global push-to-talk status dropped: the interface is not keeping up");
        }
    }

    /// The binding that reported last, and whether its key is down.
    fn latest_key(&self) -> (u64, bool) {
        let packed = self.latest_key.load(Ordering::Acquire);
        (packed >> 1, packed & 1 == 1)
    }
}

/// The hot key `global-hotkey` grabs for `code`; `None` for a modifier alone or a code that is
/// not a key of the keyboard itself. The portal's keysym names are that list: media keys stay
/// out, since `global-hotkey` hears them on macOS through an event tap, which needs the
/// Accessibility permission (ADR-0053).
fn hotkey_of_code(code: &str) -> Option<HotKey> {
    portal_trigger_of_code(code)?;
    code.parse().ok()
}

/// The status of a hot key that `code` could not become.
fn status_without_hotkey(code: &str) -> GlobalPushToTalkStatus {
    if modifier_of_code(code).is_some() {
        GlobalPushToTalkStatus::ModifierKeyWindowOnly
    } else {
        GlobalPushToTalkStatus::KeyUnavailable
    }
}

/// Sends `global-hotkey`'s events to `reports`, stamped with the binding current when the
/// system delivered them. The crate's handler is process-wide and set once.
fn install_hotkey_event_handler(reports: ListenerReports, binding: Arc<AtomicU64>) {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(move || {
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            let binding = binding.load(Ordering::Acquire);
            if binding != NO_BINDING {
                reports.key(
                    binding,
                    event.state() == global_hotkey::HotKeyState::Pressed,
                );
            }
        }));
    });
}

/// A system's key listener: the I/O boundary, which tests stand in for.
trait KeyListener {
    /// Starts listening for `code`; the status, or `Off` while a thread finds it out.
    fn bind(&mut self, code: &str, binding: u64, shortcut_name: &str) -> GlobalPushToTalkStatus;
    fn unbind(&mut self);
}

/// Follows the voice channel and the settings, and holds the key while push-to-talk needs it.
struct GlobalPushToTalk<L> {
    voice: Entity<Voice>,
    listener: L,
    reports: ListenerReports,
    bound_code: Option<String>,
    binding: u64,
    _event_pump: Task<()>,
    _subscriptions: Vec<Subscription>,
}

/// Starts the listener for the app's voice.
pub fn install_global_push_to_talk(voice: &Entity<Voice>, cx: &mut App) {
    let global_push_to_talk = cx.new(|cx| GlobalPushToTalk::new(voice.clone(), Listener::new, cx));
    // Lives as long as the app: the voice it follows does.
    cx.set_global(GlobalPushToTalkHolder {
        _entity: global_push_to_talk,
    });
}

struct GlobalPushToTalkHolder {
    _entity: Entity<GlobalPushToTalk<Listener>>,
}

impl gpui::Global for GlobalPushToTalkHolder {}

impl<L: KeyListener + 'static> GlobalPushToTalk<L> {
    fn new(
        voice: Entity<Voice>,
        listener_reporting_to: impl FnOnce(ListenerReports) -> L,
        cx: &mut Context<Self>,
    ) -> Self {
        let (sender, mut receiver) = mpsc::channel(LISTENER_EVENT_QUEUE_CAPACITY);
        let reports = ListenerReports::new(sender);
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
        #[cfg_attr(
            not(target_os = "macos"),
            expect(unused_mut, reason = "macOS adds one")
        )]
        let mut subscriptions = vec![
            cx.observe(&voice, |this, _, cx| this.follow_voice(cx)),
            cx.observe_global::<PreferencesStore>(Self::follow_voice),
        ];
        // The key typing the recorded character moves with the layout (`macos_keyboard_layout`).
        #[cfg(target_os = "macos")]
        {
            let this = cx.weak_entity();
            subscriptions.push(cx.on_keyboard_layout_change(move |cx| {
                // Gone only when the app quits.
                if this.update(cx, GlobalPushToTalk::rebind).is_err() {
                    tracing::debug!("keyboard layout changed after global push-to-talk ended");
                }
            }));
        }
        let mut this = Self {
            voice,
            listener: listener_reporting_to(reports.clone()),
            reports,
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
        self.unbind(cx);
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

    /// Lets the key go, and releases push-to-talk if the listener held it.
    fn unbind(&mut self, cx: &mut Context<Self>) {
        if self.bound_code.take().is_some() {
            self.listener.unbind();
            self.binding = NO_BINDING;
            self.voice.update(cx, Voice::global_push_to_talk_unbound);
        }
    }

    /// Takes the key again, as if newly chosen.
    #[cfg_attr(
        not(target_os = "macos"),
        expect(dead_code, reason = "macOS rebinds on a layout change")
    )]
    fn rebind(&mut self, cx: &mut Context<Self>) {
        self.unbind(cx);
        self.follow_voice(cx);
    }

    fn follow_listener(&mut self, event: ListenerEvent, cx: &mut Context<Self>) {
        match event {
            ListenerEvent::KeyChanged => {
                let (binding, is_pressed) = self.reports.latest_key();
                if binding != NO_BINDING && binding == self.binding {
                    self.voice.update(cx, |voice, cx| {
                        voice.global_push_to_talk_key_changed(is_pressed, cx);
                    });
                }
            }
            ListenerEvent::Status { binding, status } if binding == self.binding => {
                self.set_status(status, cx);
            }
            ListenerEvent::Status { .. } => {}
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
