//! Each channel keeps its own unsent text and attached files (`composer-drafts.svelte.ts`,
//! DESIGN.md §5.5): the open channel's tray lives here and its text in the field; the others wait
//! aside, in memory. Files stay with their channel: they were uploaded to it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{RenderImage, Task};
use mellow_protocol::{Attachment, Snowflake};

/// The instance links at most this many files to one message (ADR-0020).
pub const MAXIMUM_ATTACHMENTS_PER_MESSAGE: usize = 10;

/// A file chosen for the next message: it uploads at once, then waits for Send (ADR-0020).
#[derive(Debug)]
pub struct TrayItem {
    pub key: u64,
    /// The channel open when the file was attached: it uploads there, whatever is open by then.
    pub channel_id: Snowflake,
    pub path: PathBuf,
    pub file_name: String,
    pub size_bytes: u64,
    pub status: TrayItemStatus,
    /// A small picture decoded from the file once read; its atlas slot is freed when it leaves.
    pub preview: Option<Arc<RenderImage>>,
    /// The running upload; dropping it (removal, a cleared tray) stops the request.
    pub upload: Option<Task<()>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayItemStatus {
    Uploading,
    Uploaded(Attachment),
    /// Why, as a sentence; the file is not sent and waits to be tried again.
    Failed(String),
}

#[derive(Debug, Default)]
struct ChannelDraft {
    text: String,
    tray: Vec<TrayItem>,
}

#[derive(Debug, Default)]
pub struct ComposerDrafts {
    open_channel_id: Option<Snowflake>,
    tray: Vec<TrayItem>,
    drafts_aside: HashMap<Snowflake, ChannelDraft>,
    // Shared by every channel: a tray brought back never meets a key it already holds.
    next_tray_item_key: u64,
}

impl ComposerDrafts {
    #[must_use]
    pub fn open_channel_id(&self) -> Option<Snowflake> {
        self.open_channel_id
    }

    /// Puts the open channel's draft aside (its text is `open_text`, read from the field) and
    /// brings back `channel_id`'s text to show; `None` when that channel is already open.
    pub fn open(&mut self, channel_id: Option<Snowflake>, open_text: String) -> Option<String> {
        if channel_id == self.open_channel_id {
            return None;
        }
        if let Some(open_channel_id) = self.open_channel_id {
            let tray = std::mem::take(&mut self.tray);
            if !open_text.is_empty() || !tray.is_empty() {
                self.drafts_aside.insert(
                    open_channel_id,
                    ChannelDraft {
                        text: open_text,
                        tray,
                    },
                );
            }
        }
        self.open_channel_id = channel_id;
        let draft = channel_id
            .and_then(|channel_id| self.drafts_aside.remove(&channel_id))
            .unwrap_or_default();
        self.tray = draft.tray;
        Some(draft.text)
    }

    #[must_use]
    pub fn tray(&self) -> &[TrayItem] {
        &self.tray
    }

    /// Room left in the open tray.
    #[must_use]
    pub fn tray_room(&self) -> usize {
        MAXIMUM_ATTACHMENTS_PER_MESSAGE.saturating_sub(self.tray.len())
    }

    /// Adds a file to the open channel's tray, uploading; `None` without an open channel.
    pub fn attach(&mut self, path: PathBuf, file_name: String, size_bytes: u64) -> Option<u64> {
        let channel_id = self.open_channel_id?;
        let key = self.next_tray_item_key;
        self.next_tray_item_key += 1;
        self.tray.push(TrayItem {
            key,
            channel_id,
            path,
            file_name,
            size_bytes,
            status: TrayItemStatus::Uploading,
            preview: None,
            upload: None,
        });
        Some(key)
    }

    /// The item with `key`, open or put aside: an upload's answer reaches it after a switch.
    pub fn tray_item_mut(&mut self, key: u64) -> Option<&mut TrayItem> {
        self.tray
            .iter_mut()
            .chain(
                self.drafts_aside
                    .values_mut()
                    .flat_map(|draft| draft.tray.iter_mut()),
            )
            .find(|item| item.key == key)
    }

    #[must_use]
    pub fn is_in_open_tray(&self, key: u64) -> bool {
        self.tray.iter().any(|item| item.key == key)
    }

    /// Sets a failed file uploading again; the caller starts its new request.
    pub fn restart_upload(&mut self, key: u64) -> Option<&mut TrayItem> {
        let item = self.tray_item_mut(key)?;
        item.status = TrayItemStatus::Uploading;
        item.upload = None;
        Some(item)
    }

    /// Removes a file from the open tray, stopping its upload; returns it to free its preview.
    pub fn remove_tray_item(&mut self, key: u64) -> Option<TrayItem> {
        let index = self.tray.iter().position(|item| item.key == key)?;
        Some(self.tray.remove(index))
    }

    /// Empties the open tray after a send, stopping what still uploads; other channels keep theirs.
    /// Returns the files to free their previews.
    pub fn clear_tray(&mut self) -> Vec<TrayItem> {
        std::mem::take(&mut self.tray)
    }

    #[must_use]
    pub fn is_uploading(&self) -> bool {
        self.tray
            .iter()
            .any(|item| item.status == TrayItemStatus::Uploading)
    }

    /// The open tray's uploaded files, in tray order; failed ones are left out.
    #[must_use]
    pub fn uploaded_attachments(&self) -> Vec<Attachment> {
        self.tray
            .iter()
            .filter_map(|item| match &item.status {
                TrayItemStatus::Uploaded(attachment) => Some(attachment.clone()),
                TrayItemStatus::Uploading | TrayItemStatus::Failed(_) => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(id: u64) -> Snowflake {
        Snowflake::from_raw(id)
    }

    fn attach(drafts: &mut ComposerDrafts, name: &str) -> u64 {
        drafts
            .attach(PathBuf::from(name), name.to_owned(), 1)
            .expect("a channel is open")
    }

    fn names(drafts: &ComposerDrafts) -> Vec<&str> {
        drafts
            .tray()
            .iter()
            .map(|item| item.file_name.as_str())
            .collect()
    }

    fn attachment(id: u64) -> Attachment {
        Attachment {
            id: Snowflake::from_raw(id),
            file_name: "photo.png".into(),
            content_type: "image/png".into(),
            size_bytes: 1,
            width: None,
            height: None,
        }
    }

    #[test]
    fn brings_each_channels_text_and_files_back_after_a_switch() {
        let mut drafts = ComposerDrafts::default();
        assert_eq!(
            drafts.open(Some(channel(1)), String::new()),
            Some(String::new())
        );
        attach(&mut drafts, "one.png");

        assert_eq!(
            drafts.open(Some(channel(2)), "hello".into()),
            Some(String::new())
        );
        assert!(drafts.tray().is_empty());
        attach(&mut drafts, "two.png");

        assert_eq!(
            drafts.open(Some(channel(1)), "bye".into()),
            Some("hello".into())
        );
        assert_eq!(names(&drafts), ["one.png"]);
        assert_eq!(
            drafts.open(Some(channel(2)), String::new()),
            Some("bye".into())
        );
        assert_eq!(names(&drafts), ["two.png"]);
    }

    #[test]
    fn keeps_the_drafts_through_a_moment_without_a_channel() {
        let mut drafts = ComposerDrafts::default();
        drafts.open(Some(channel(1)), String::new());
        assert_eq!(drafts.open(None, "kept".into()), Some(String::new()));
        assert_eq!(drafts.attach(PathBuf::from("x"), "x".into(), 1), None);
        assert_eq!(
            drafts.open(Some(channel(1)), String::new()),
            Some("kept".into())
        );
    }

    #[test]
    fn tells_whether_the_channel_changed_and_leaves_the_draft_alone_when_it_did_not() {
        let mut drafts = ComposerDrafts::default();
        drafts.open(Some(channel(1)), String::new());
        attach(&mut drafts, "one.png");
        assert_eq!(drafts.open(Some(channel(1)), "typed".into()), None);
        assert_eq!(names(&drafts), ["one.png"]);
    }

    #[test]
    fn updates_a_file_put_aside_so_its_upload_result_shows_when_its_channel_comes_back() {
        let mut drafts = ComposerDrafts::default();
        drafts.open(Some(channel(1)), String::new());
        let key = attach(&mut drafts, "one.png");
        drafts.open(Some(channel(2)), String::new());
        assert!(!drafts.is_in_open_tray(key));

        drafts.tray_item_mut(key).expect("kept aside").status =
            TrayItemStatus::Uploaded(attachment(9));

        drafts.open(Some(channel(1)), String::new());
        assert_eq!(drafts.uploaded_attachments(), [attachment(9)]);
    }

    #[test]
    fn never_gives_two_files_the_same_key_across_channels() {
        let mut drafts = ComposerDrafts::default();
        drafts.open(Some(channel(1)), String::new());
        let first = attach(&mut drafts, "a");
        drafts.open(Some(channel(2)), String::new());
        let second = attach(&mut drafts, "b");
        assert_ne!(first, second);
    }

    #[test]
    fn empties_the_open_tray_after_sending_leaving_the_other_channels_trays() {
        let mut drafts = ComposerDrafts::default();
        drafts.open(Some(channel(1)), String::new());
        attach(&mut drafts, "one.png");
        drafts.open(Some(channel(2)), String::new());
        attach(&mut drafts, "two.png");

        let _ = drafts.clear_tray();

        assert!(drafts.tray().is_empty());
        drafts.open(Some(channel(1)), String::new());
        assert_eq!(names(&drafts), ["one.png"]);
    }

    #[test]
    fn binds_each_file_to_the_channel_open_when_it_was_attached() {
        let mut drafts = ComposerDrafts::default();
        drafts.open(Some(channel(1)), String::new());
        let key = attach(&mut drafts, "one.png");
        drafts.open(Some(channel(2)), String::new());
        assert_eq!(
            drafts.tray_item_mut(key).map(|item| item.channel_id),
            Some(channel(1))
        );
    }

    #[test]
    fn starts_a_failed_file_over_and_counts_only_uploaded_ones_as_sendable() {
        let mut drafts = ComposerDrafts::default();
        drafts.open(Some(channel(1)), String::new());
        let failed = attach(&mut drafts, "failed.png");
        let uploaded = attach(&mut drafts, "uploaded.png");
        drafts.tray_item_mut(failed).expect("in the tray").status =
            TrayItemStatus::Failed("Couldn't upload failed.png.".into());
        drafts.tray_item_mut(uploaded).expect("in the tray").status =
            TrayItemStatus::Uploaded(attachment(3));
        assert!(!drafts.is_uploading());
        assert_eq!(drafts.uploaded_attachments(), [attachment(3)]);

        drafts.restart_upload(failed);

        assert!(drafts.is_uploading());
        let _ = drafts.remove_tray_item(failed);
        assert_eq!(names(&drafts), ["uploaded.png"]);
        assert_eq!(drafts.tray_room(), MAXIMUM_ATTACHMENTS_PER_MESSAGE - 1);
    }
}
