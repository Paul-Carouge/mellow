//! macOS and Windows: one `global-hotkey` hot key, registered on the UI thread, whose event loop
//! receives it (Carbon's application target, the Win32 hidden window's messages).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use global_hotkey::GlobalHotKeyManager;
use global_hotkey::hotkey::HotKey;
use mellow_client_core::shared_logic::push_to_talk_keys::does_code_type_a_character;
use tokio::sync::mpsc;

use super::{
    GlobalPushToTalkStatus, ListenerEvent, NO_BINDING, hotkey_of_code,
    install_hotkey_event_handler, status_without_hotkey,
};

pub(super) struct Listener {
    events: mpsc::Sender<ListenerEvent>,
    /// Read by the hot key callback to stamp its events.
    binding: Arc<AtomicU64>,
    /// Created at the first binding, then kept: creating it installs the system handler.
    manager: Option<GlobalHotKeyManager>,
    registered: Option<HotKey>,
}

impl Listener {
    pub(super) fn new(events: mpsc::Sender<ListenerEvent>) -> Self {
        Self {
            events,
            binding: Arc::new(AtomicU64::new(NO_BINDING)),
            manager: None,
            registered: None,
        }
    }

    /// Grabs `code` for `binding`; the portal's shortcut name is not used here.
    pub(super) fn bind(
        &mut self,
        code: &str,
        binding: u64,
        _shortcut_name: &str,
    ) -> GlobalPushToTalkStatus {
        let Some(hotkey) = hotkey_of_code(code) else {
            return status_without_hotkey(code);
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
            does_key_stop_typing: does_code_type_a_character(code),
        }
    }

    pub(super) fn unbind(&mut self) {
        self.binding.store(NO_BINDING, Ordering::Release);
        if let (Some(manager), Some(hotkey)) = (self.manager.as_ref(), self.registered.take())
            && manager.unregister(hotkey).is_err()
        {
            tracing::warn!("the push-to-talk key could not be released system-wide");
        }
    }

    /// Tried again at the next binding when it fails.
    fn create_manager_once(&mut self) {
        if self.manager.is_some() {
            return;
        }
        install_hotkey_event_handler(self.events.clone(), Arc::clone(&self.binding));
        match GlobalHotKeyManager::new() {
            Ok(manager) => self.manager = Some(manager),
            Err(error) => tracing::warn!(%error, "global push-to-talk is unavailable"),
        }
    }
}
