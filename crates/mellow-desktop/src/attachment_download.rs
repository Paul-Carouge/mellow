//! Saving an attachment to a file (the web's download link, `MessageAttachments.svelte`): the
//! file card's download button asks where to save, in the Downloads folder first, then fetches
//! the file on the authenticated download path and writes it off the interface thread. Files the
//! app does not show or play (a codec it lacks, a size past the player's) are kept this way; the
//! card reads out "saved" or the failure. A saved file is marked as downloaded from the internet,
//! as browsers mark theirs, so the system checks it before it opens.

use std::collections::HashMap;
use std::path::Path;

use gpui::{App, ElementId, Global, IntoElement, ParentElement as _, Styled as _, div};
use mellow_client_core::{Download, DownloadableFile};
use mellow_protocol::{Attachment, Snowflake};

use crate::attachment_audio::attachment_rest_client;
use crate::components::{IconButton, child_id, polite_live_region, with_type};
use crate::i18n::strings;
use crate::theme::icons::IconName;
use crate::theme::tokens::TYPE_BODY_SM;
use crate::theme::{color, theme};
use crate::tokio_bridge::spawn_on_network_runtime;

/// The most a saved file may weigh: it is held in memory before being written.
// shortcut: the whole file is buffered (the client core's downloads are); stream it to disk if
// instances raise their upload limit past a few hundred megabytes.
const MAXIMUM_SAVED_ATTACHMENT_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SaveOutcome {
    Saving,
    Saved,
    Failed,
}

/// The outcome of each attachment saved during this run, shown on its card.
#[derive(Default)]
struct SavedAttachments(HashMap<Snowflake, SaveOutcome>);

impl Global for SavedAttachments {}

fn set_outcome(attachment_id: Snowflake, outcome: SaveOutcome, cx: &mut App) {
    cx.default_global::<SavedAttachments>()
        .0
        .insert(attachment_id, outcome);
    cx.refresh_windows();
}

#[must_use]
pub(crate) fn save_outcome(attachment_id: Snowflake, cx: &App) -> Option<SaveOutcome> {
    cx.try_global::<SavedAttachments>()?
        .0
        .get(&attachment_id)
        .copied()
}

/// Asks where to save `attachment`, then downloads and writes it there.
pub(crate) fn save_attachment(attachment: &Attachment, cx: &mut App) {
    if save_outcome(attachment.id, cx) == Some(SaveOutcome::Saving) {
        return;
    }
    let Some(rest) = attachment_rest_client(cx) else {
        return;
    };
    let attachment_id = attachment.id;
    let directory = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_default();
    let picked = cx.prompt_for_new_path(&directory, Some(&attachment.file_name));
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(path))) = picked.await else {
            return;
        };
        cx.update(|cx| set_outcome(attachment_id, SaveOutcome::Saving, cx));
        let download = cx.update(|cx| {
            spawn_on_network_runtime(cx, async move {
                rest.download(
                    DownloadableFile::Attachment(attachment_id),
                    None,
                    MAXIMUM_SAVED_ATTACHMENT_BYTES,
                )
                .await
            })
        });
        let written = match download.await {
            Ok(Ok(Download::Fetched(file))) => cx
                .background_executor()
                .spawn(async move {
                    std::fs::write(&path, &file.bytes)?;
                    // The file is saved either way: a volume without extended attributes or
                    // streams (FAT) cannot hold the mark, as for browsers' downloads.
                    if let Err(error) = mark_as_downloaded(&path) {
                        tracing::warn!(error = %error.kind(), "cannot mark a saved attachment");
                    }
                    Ok::<(), std::io::Error>(())
                })
                .await
                .map_err(
                    |error| tracing::warn!(error = %error.kind(), "cannot save an attachment"),
                ),
            Ok(Err(error)) => {
                tracing::warn!(%error, "cannot download an attachment to save");
                Err(())
            }
            // No cached copy was offered, so `NotModified` never comes; or the task stopped.
            Ok(Ok(Download::NotModified)) | Err(_) => Err(()),
        };
        cx.update(|cx| {
            set_outcome(
                attachment_id,
                if written.is_ok() {
                    SaveOutcome::Saved
                } else {
                    SaveOutcome::Failed
                },
                cx,
            );
        });
    })
    .detach();
}

/// Marks a saved file as downloaded from the internet, so `Gatekeeper` on macOS, and `SmartScreen`
/// and Office's protected view on Windows, check it before it opens, as they do browsers'
/// downloads: a member can send a `.command`, an unsigned app or a script.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn mark_as_downloaded(path: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt as _;
    // Flags 0081: downloaded, not opened yet; then the time in hexadecimal and the agent.
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let value = format!("0081;{seconds:08x};Mellow;");
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: a NUL-terminated path and attribute name, and a value of the length given.
    let status = unsafe {
        libc::setxattr(
            path.as_ptr(),
            c"com.apple.quarantine".as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
            0,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// See the macOS version: the Internet zone (3) in the file's `Zone.Identifier` stream.
#[cfg(windows)]
fn mark_as_downloaded(path: &Path) -> std::io::Result<()> {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    std::fs::write(stream, "[ZoneTransfer]\r\nZoneId=3\r\n")
}

/// Linux desktops keep no such mark; the file is written without the permission to run.
#[cfg(not(any(target_os = "macos", windows)))]
#[allow(clippy::unnecessary_wraps)] // The same signature as on macOS and Windows.
fn mark_as_downloaded(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// The card's download button, and what became of the last save, read out when it changes.
pub(crate) fn download_control(
    parent: &ElementId,
    attachment: &Attachment,
    cx: &App,
) -> (IconButton, Option<impl IntoElement + use<>>) {
    let outcome = save_outcome(attachment.id, cx);
    let words = strings(cx);
    let name = &attachment.file_name;
    let saved = attachment.clone();
    let button = IconButton::new(
        child_id(parent, "download"),
        IconName::DownloadSimple,
        words.messages().download(name),
    )
    .disabled(outcome == Some(SaveOutcome::Saving))
    .on_click(move |_, cx| save_attachment(&saved, cx));
    let note = outcome.map(|outcome| {
        let desktop = words.desktop();
        let text = match outcome {
            SaveOutcome::Saving => desktop.attachment_saving(name),
            SaveOutcome::Saved => desktop.attachment_saved(name),
            SaveOutcome::Failed => desktop.attachment_save_failed(name),
        };
        let tokens = theme(cx);
        with_type(div(), TYPE_BODY_SM)
            .text_color(color(if outcome == SaveOutcome::Failed {
                tokens.color_text_danger
            } else {
                tokens.component_attachment_playback_note_text
            }))
            .child(text.clone())
            .child(polite_live_region(
                child_id(parent, "download-outcome"),
                text.into(),
            ))
    });
    (button, note)
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    #[test]
    fn a_saved_file_is_quarantined() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("report.command");
        std::fs::write(&path, b"#!/bin/sh").unwrap();
        super::mark_as_downloaded(&path).unwrap();
        let mark = std::process::Command::new("xattr")
            .args(["-p", "com.apple.quarantine"])
            .arg(&path)
            .output()
            .unwrap();
        let mark = String::from_utf8(mark.stdout).unwrap();
        assert!(
            mark.starts_with("0081;") && mark.trim_end().ends_with(";Mellow;"),
            "{mark}"
        );
    }
}
