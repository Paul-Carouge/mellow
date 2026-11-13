//! COM and Media Foundation for the engine's Windows code: every camera, screen, sound or H.264
//! call goes through [`ensure_com`] or [`ensure_media_foundation`] first.

use std::sync::OnceLock;

use windows::Win32::Media::MediaFoundation::{MF_VERSION, MFSTARTUP_NOSOCKET, MFStartup};
use windows::Win32::System::Com::CoIncrementMTAUsage;

/// Keeps COM's multithreaded apartment alive for the whole process (once), so every thread that
/// did not join an apartment of its own works in it. Never initialising and uninitialising per
/// thread matters: when the last thread of the apartment uninitialises, the apartment goes, and
/// with it the factories `windows` caches for the process (`GraphicsCaptureSession`'s,
/// `Direct3D11CaptureFramePool`'s), which the next caller then reads freed. Threads already in
/// an apartment (GPUI's interface thread) keep theirs.
pub(crate) fn ensure_com() {
    static APARTMENT: OnceLock<()> = OnceLock::new();
    APARTMENT.get_or_init(|| {
        // SAFETY: a plain call; its cookie is never given back, so the apartment lives as long
        // as the process.
        if let Err(error) = unsafe { CoIncrementMTAUsage() } {
            tracing::warn!(%error, "COM's multithreaded apartment could not be kept alive");
        }
    });
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
