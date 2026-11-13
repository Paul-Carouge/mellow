//! COM and Media Foundation set up for the thread that calls them: every Windows camera, screen,
//! sound or H.264 call goes through [`ensure_media_foundation`] first.

use std::sync::OnceLock;

use windows::Win32::Media::MediaFoundation::{MF_VERSION, MFSTARTUP_NOSOCKET, MFStartup};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

/// Uninitialises COM when its thread ends, if this thread initialised it.
struct ComOnThisThread {
    should_uninitialize: bool,
}

impl Drop for ComOnThisThread {
    fn drop(&mut self) {
        if self.should_uninitialize {
            // SAFETY: paired with the successful `CoInitializeEx` of this same thread.
            unsafe { CoUninitialize() };
        }
    }
}

thread_local! {
    static COM_ON_THIS_THREAD: ComOnThisThread = {
        // SAFETY: no reserved pointer. On a thread already in another apartment (GPUI's
        // interface thread is single-threaded) this fails with `RPC_E_CHANGED_MODE` and the
        // thread keeps its own apartment, which Media Foundation accepts as well.
        let status = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        ComOnThisThread { should_uninitialize: status.is_ok() }
    };
}

/// Initialises COM on this thread, once, undone when the thread ends.
pub(crate) fn ensure_com() {
    COM_ON_THIS_THREAD.with(|_| ());
}

/// [`ensure_com`], and Media Foundation for the process (once, kept for its life: stopping it
/// would break other threads' objects).
pub(crate) fn ensure_media_foundation() -> Result<(), String> {
    static STARTUP: OnceLock<Result<(), String>> = OnceLock::new();
    ensure_com();
    STARTUP
        .get_or_init(|| {
            // SAFETY: a plain call; Windows N editions without the Media Feature Pack fail here.
            unsafe { MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET) }
                .map_err(|error| format!("Media Foundation is not available: {error}"))
        })
        .clone()
}

/// The size of `T` as the `u32` Win32 structures carry it (every one is far below 4 GiB).
pub(crate) fn size_as_u32<T>() -> u32 {
    u32::try_from(size_of::<T>()).unwrap_or(u32::MAX)
}
