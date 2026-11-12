//! macOS and Windows: one `global-hotkey` hot key, registered on the UI thread, whose event loop
//! receives it (Carbon's application target, the Win32 hidden window's messages).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use global_hotkey::GlobalHotKeyManager;
use global_hotkey::hotkey::HotKey;
use mellow_client_core::shared_logic::push_to_talk_keys::does_grab_take_key_from_other_apps;

use super::{
    GlobalPushToTalkStatus, KeyListener, ListenerReports, NO_BINDING, hotkey_of_code,
    install_hotkey_event_handler, status_without_hotkey,
};

pub(super) struct Listener {
    reports: ListenerReports,
    /// Read by the hot key callback to stamp its events.
    binding: Arc<AtomicU64>,
    /// Created at the first binding, then kept: creating it installs the system handler.
    manager: Option<GlobalHotKeyManager>,
    registered: Option<HotKey>,
}

impl Listener {
    pub(super) fn new(reports: ListenerReports) -> Self {
        Self {
            reports,
            binding: Arc::new(AtomicU64::new(NO_BINDING)),
            manager: None,
            registered: None,
        }
    }

    /// Tried again at the next binding when it fails.
    fn create_manager_once(&mut self) {
        if self.manager.is_some() {
            return;
        }
        install_hotkey_event_handler(self.reports.clone(), Arc::clone(&self.binding));
        match GlobalHotKeyManager::new() {
            Ok(manager) => self.manager = Some(manager),
            Err(error) => tracing::warn!(%error, "global push-to-talk is unavailable"),
        }
    }
}

/// The hot key for `code` on the keyboard layout in use: Carbon names keys by their ANSI
/// position, Win32 by the layout's own virtual key, which already follows it.
fn hotkey_on_current_layout(code: &str) -> Result<HotKey, GlobalPushToTalkStatus> {
    let hotkey = hotkey_of_code(code).ok_or_else(|| status_without_hotkey(code))?;
    #[cfg(target_os = "macos")]
    let hotkey = super::macos_keyboard_layout::hotkey_on_current_layout(hotkey)
        .ok_or(GlobalPushToTalkStatus::KeyNotOnKeyboardLayout)?;
    Ok(hotkey)
}

impl KeyListener for Listener {
    /// Grabs `code` for `binding`; the portal's shortcut name is not used here.
    fn bind(&mut self, code: &str, binding: u64, _shortcut_name: &str) -> GlobalPushToTalkStatus {
        let hotkey = match hotkey_on_current_layout(code) {
            Ok(hotkey) => hotkey,
            Err(status) => return status,
        };
        self.create_manager_once();
        let Some(manager) = self.manager.as_ref() else {
            return GlobalPushToTalkStatus::KeyUnavailable;
        };
        // Stamped before registering, so the first press is not dropped.
        self.binding.store(binding, Ordering::Release);
        if manager.register(hotkey).is_err() {
            // The error names the key: left out of the log.
            tracing::warn!("the push-to-talk key could not be registered system-wide");
            self.binding.store(NO_BINDING, Ordering::Release);
            return GlobalPushToTalkStatus::KeyUnavailable;
        }
        self.registered = Some(hotkey);
        GlobalPushToTalkStatus::Listening {
            trigger: None,
            is_key_taken_from_other_apps: does_grab_take_key_from_other_apps(code),
        }
    }

    fn unbind(&mut self) {
        self.binding.store(NO_BINDING, Ordering::Release);
        if let (Some(manager), Some(hotkey)) = (self.manager.as_ref(), self.registered.take())
            && manager.unregister(hotkey).is_err()
        {
            tracing::warn!("the push-to-talk key could not be released system-wide");
        }
    }
}
