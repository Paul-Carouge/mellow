//! "Copy text" (DESIGN.md §5.4).

/// The clipboard handle is created on first use and then kept for the whole session: on X11 the
/// copying application owns the clipboard content, so dropping the handle right after a copy
/// loses it when no clipboard manager runs. A failure (e.g. a headless session) only loses this copy.
pub fn copy_text(clipboard: &mut Option<arboard::Clipboard>, text: &str) {
    if clipboard.is_none() {
        *clipboard = arboard::Clipboard::new()
            .inspect_err(|error| tracing::warn!(%error, "clipboard unavailable"))
            .ok();
    }
    if let Some(clipboard) = clipboard.as_mut()
        && let Err(error) = clipboard.set_text(text.to_owned())
    {
        tracing::warn!(%error, "copy text failed");
    }
}
