//! Saving an attachment to a file (the web's download link, `MessageAttachments.svelte`): the
//! file card's download button asks where to save, in the Downloads folder first, then fetches
//! the file on the authenticated download path and writes it off the interface thread. Files the
//! app does not show or play (a codec it lacks, a size past the player's) are kept this way; the
//! card reads out "saved" or the failure.

use std::collections::HashMap;

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
                .spawn(async move { std::fs::write(&path, &file.bytes) })
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
