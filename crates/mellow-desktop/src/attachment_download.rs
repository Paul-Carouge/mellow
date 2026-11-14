//! Saving an attachment to a file (the web's download link, `MessageAttachments.svelte`): the
//! file card's download button asks where to save, in the Downloads folder first, then fetches
//! the file on the authenticated download path and writes it off the interface thread. Files the
//! app does not show or play (a codec it lacks, a size past the player's) are kept this way; the
//! card reads out "saved" or the failure. A saved file is marked as downloaded from the internet,
//! as browsers mark theirs, so the system checks it before it opens.
//!
//! The media viewer's "Open with the default app" fetches the file the same way into a folder of
//! Mellow's cache (per profile, emptied of files older than a day and at sign-out), under its
//! sent name made safe, then hands it to the system: the instance's link needs the session, which
//! a browser does not have.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

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

/// Files opened with the default app are kept this long: the app may still read them.
const OPENED_FILES_LIFETIME: Duration = Duration::from_hours(24);
/// The cache's folder of opened attachments.
const OPENED_DIRECTORY: &str = "opened";
/// The longest file name most file systems take, in bytes.
const MAXIMUM_FILE_NAME_BYTES: usize = 255;
/// A name with nothing left once made safe.
const FALLBACK_FILE_NAME: &str = "attachment";

/// Where opening an attachment with the default app stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenOutcome {
    Opening,
    Failed,
}

#[derive(Default)]
struct OpenedAttachments(HashMap<Snowflake, OpenOutcome>);

impl Global for OpenedAttachments {}

#[must_use]
pub(crate) fn open_outcome(attachment_id: Snowflake, cx: &App) -> Option<OpenOutcome> {
    cx.try_global::<OpenedAttachments>()?
        .0
        .get(&attachment_id)
        .copied()
}

fn set_open_outcome(attachment_id: Snowflake, outcome: Option<OpenOutcome>, cx: &mut App) {
    let opened = &mut cx.default_global::<OpenedAttachments>().0;
    match outcome {
        Some(outcome) => opened.insert(attachment_id, outcome),
        None => opened.remove(&attachment_id),
    };
    cx.refresh_windows();
}

/// `<user cache>/mellow[-profile]/opened`.
fn opened_directory() -> Option<PathBuf> {
    Some(
        dirs::cache_dir()?
            .join(&*crate::profile::with_profile_suffix(
                crate::pictures::disk_cache::APPLICATION_DIRECTORY,
            ))
            .join(OPENED_DIRECTORY),
    )
}

/// A sent file name safe to write: no folders, no control characters, not hidden, not too long.
#[must_use]
pub(crate) fn safe_file_name(file_name: &str) -> String {
    let cleaned: String = file_name
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                )
            {
                '_'
            } else {
                character
            }
        })
        .collect();
    let trimmed = cleaned
        .trim()
        .trim_start_matches('.')
        .trim_end_matches(['.', ' ']);
    let mut name = String::new();
    for character in trimmed.chars() {
        if name.len() + character.len_utf8() > MAXIMUM_FILE_NAME_BYTES {
            break;
        }
        name.push(character);
    }
    if name.is_empty() {
        FALLBACK_FILE_NAME.to_owned()
    } else {
        name
    }
}

/// The extension written for a type the default app may open: pictures and videos only, so a
/// viewer or player opens it, never a program, a shortcut or a browser (`.exe`, `.desktop`,
/// `.lnk`, `.svg`, `.html`).
#[must_use]
pub(crate) fn opened_extension(content_type: &str) -> Option<&'static str> {
    Some(match content_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/avif" => "avif",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "video/webm" => "webm",
        _ => return None,
    })
}

/// Whether `bytes` start as `content_type` does. The instance sniffs every upload's type from its
/// first bytes; this checks again what reached us, so a file is never opened as a type it is not.
#[must_use]
pub(crate) fn starts_as(bytes: &[u8], content_type: &str) -> bool {
    match content_type {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.get(..4) == Some(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        "image/avif" => matches!(bytes.get(4..12), Some(b"ftypavif" | b"ftypavis")),
        _ => mellow_media::video_container(bytes, content_type).is_some(),
    }
}

/// The name an opened file is written under: the sent name, made safe, without any of its own
/// extensions (`photo.exe.png` gives `photo`), then the extension of its checked type. `None`
/// for a type the default app may not open.
#[must_use]
pub(crate) fn opened_file_name(file_name: &str, content_type: &str) -> Option<String> {
    let extension = opened_extension(content_type)?;
    let safe = safe_file_name(file_name);
    let stem = safe.split('.').next().map(str::trim).unwrap_or_default();
    let stem = if stem.is_empty() {
        FALLBACK_FILE_NAME
    } else {
        stem
    };
    // Room for the extension within the file system's limit.
    let mut kept = String::new();
    for character in stem.chars() {
        if kept.len() + character.len_utf8() + 1 + extension.len() > MAXIMUM_FILE_NAME_BYTES {
            break;
        }
        kept.push(character);
    }
    Some(format!("{kept}.{extension}"))
}

/// Fetches `attachment` into the cache and opens it with the system's default app. Only pictures
/// and videos whose bytes are what their type says are opened; anything else is left to
/// "Download".
pub(crate) fn open_attachment_with_default_app(attachment: &Attachment, cx: &mut App) {
    if open_outcome(attachment.id, cx) == Some(OpenOutcome::Opening) {
        return;
    }
    let content_type = attachment.content_type.clone();
    let Some(file_name) = opened_file_name(&attachment.file_name, &content_type) else {
        set_open_outcome(attachment.id, Some(OpenOutcome::Failed), cx);
        return;
    };
    let (Some(rest), Some(directory)) = (attachment_rest_client(cx), opened_directory()) else {
        set_open_outcome(attachment.id, Some(OpenOutcome::Failed), cx);
        return;
    };
    let attachment_id = attachment.id;
    let folder = directory.join(attachment_id.to_string());
    let path = folder.join(file_name);
    set_open_outcome(attachment_id, Some(OpenOutcome::Opening), cx);
    cx.spawn(async move |cx| {
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
            Ok(Ok(Download::Fetched(file))) if !starts_as(&file.bytes, &content_type) => {
                tracing::warn!("an attachment to open is not the type it was sent as");
                Err(())
            }
            Ok(Ok(Download::Fetched(file))) => {
                let written_path = path.clone();
                cx.background_executor()
                    .spawn(async move {
                        remove_old_opened_files(&directory, &folder);
                        std::fs::create_dir_all(&folder)?;
                        std::fs::write(&written_path, &file.bytes)?;
                        if let Err(error) = mark_as_downloaded(&written_path) {
                            tracing::warn!(error = %error.kind(), "cannot mark an opened attachment");
                        }
                        Ok::<(), std::io::Error>(())
                    })
                    .await
                    .map_err(|error| {
                        tracing::warn!(error = %error.kind(), "cannot write an attachment to open");
                    })
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, "cannot download an attachment to open");
                Err(())
            }
            Ok(Ok(Download::NotModified)) | Err(_) => Err(()),
        };
        cx.update(|cx| {
            if written.is_ok() {
                cx.open_with_system(&path);
                set_open_outcome(attachment_id, None, cx);
            } else {
                set_open_outcome(attachment_id, Some(OpenOutcome::Failed), cx);
            }
        });
    })
    .detach();
}

/// Removes the opened files older than a day, except `kept`'s.
fn remove_old_opened_files(directory: &Path, kept: &Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > OPENED_FILES_LIFETIME);
        if is_old && path != kept {
            // An app may still hold it (Windows): it goes next time.
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// Signing out removes the opened files with the session's other copies.
pub(crate) fn forget_opened_attachments(cx: &mut App) {
    cx.default_global::<OpenedAttachments>().0.clear();
    if let Some(directory) = opened_directory() {
        cx.background_executor()
            .spawn(async move {
                if directory.exists()
                    && let Err(error) = std::fs::remove_dir_all(&directory)
                {
                    tracing::warn!(error = %error.kind(), "cannot remove the opened attachments");
                }
            })
            .detach();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_opened_file_takes_its_checked_type_s_extension_never_the_sender_s() {
        use super::opened_file_name;
        let png = |name| opened_file_name(name, "image/png");
        assert_eq!(png("photo.exe").as_deref(), Some("photo.png"));
        assert_eq!(png("photo.desktop").as_deref(), Some("photo.png"));
        assert_eq!(png("photo.lnk").as_deref(), Some("photo.png"));
        assert_eq!(png("photo.scr").as_deref(), Some("photo.png"));
        assert_eq!(png("photo.command").as_deref(), Some("photo.png"));
        assert_eq!(png("photo.svg").as_deref(), Some("photo.png"));
        assert_eq!(png("photo.html").as_deref(), Some("photo.png"));
        assert_eq!(png("photo.exe.png").as_deref(), Some("photo.png"));
        assert_eq!(png("photo").as_deref(), Some("photo.png"));
        assert_eq!(png(".exe").as_deref(), Some("exe.png"));
        assert_eq!(png("...").as_deref(), Some("attachment.png"));
        assert_eq!(
            opened_file_name("clip.lnk", "video/quicktime").as_deref(),
            Some("clip.mov")
        );
        assert!(png(&"é".repeat(300)).is_some_and(|name| name.len() <= 255
            && std::path::Path::new(&name).extension() == Some("png".as_ref())));
        // Types a default app could run or render as a page are not opened at all.
        for content_type in [
            "image/svg+xml",
            "text/html",
            "application/octet-stream",
            "application/x-msdownload",
        ] {
            assert_eq!(opened_file_name("photo.png", content_type), None);
        }
    }

    #[test]
    fn a_file_is_opened_only_when_its_bytes_are_its_type() {
        use super::starts_as;
        assert!(starts_as(b"\x89PNG\r\n\x1a\nrest", "image/png"));
        assert!(!starts_as(b"MZ\x90\x00", "image/png"));
        assert!(!starts_as(b"<svg xmlns=", "image/png"));
        assert!(starts_as(b"\0\0\0\x18ftypmp42", "video/mp4"));
        assert!(!starts_as(b"[Desktop Entry]", "video/mp4"));
        assert!(!starts_as(b"\x89PNG\r\n\x1a\n", "text/html"));
    }

    #[test]
    fn a_sent_name_is_made_safe_to_write() {
        use super::safe_file_name;
        assert_eq!(safe_file_name("soirée.jpg"), "soirée.jpg");
        assert_eq!(safe_file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_file_name(".bashrc"), "bashrc");
        assert_eq!(safe_file_name("a\u{0}b:c.txt"), "a_b_c.txt");
        assert_eq!(safe_file_name("..."), "attachment");
        assert!(safe_file_name(&"é".repeat(300)).len() <= 255);
    }

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
