//! The chat's composer (`Composer.svelte`, DESIGN.md §5.5, ADR-0022 §4): the field whose base row
//! is as tall as the user card, the reply or edit bar above it, the attachment tray, the ":na" and
//! "@na" suggestions, the characters-left counter near the limit and the errors above the box.
//!
//! It owns its drafts and uploads and says what the person did through [`ComposerEvent`]; the
//! chat pane places it under the message list and turns the events into requests.

mod drafts;
mod editor;
mod saved_drafts;
mod suggestions;
mod tray;

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AnyWindowHandle, App, AppContext as _, ClipboardEntry, Context, Entity, EventEmitter,
    ExternalPaths, Focusable as _, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement as _, PathPromptOptions, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, actions, deferred,
    div, prelude::FluentBuilder as _, relative,
};
use gpui_component::input::{InputEvent, Textarea, TextareaState};
use mellow_client_core::shared_logic::emoji_suggestions::replace_in_draft;
use mellow_client_core::shared_logic::format::{FileSize, FileSizeUnit, Locale, human_file_size};
use mellow_client_core::shared_logic::message_length::{
    MAXIMUM_MESSAGE_LENGTH, MESSAGE_LENGTH_COUNTER_THRESHOLD, message_characters_left,
};
use mellow_client_core::{ClientError, RestClient};
use mellow_protocol::{ApiErrorCode, Attachment, PublicUser, Snowflake};

use crate::components::{Button, IconButton, focus_ring, is_focus_visible, with_type};
use crate::i18n::catalogue::KbOrMbOrGb;
use crate::i18n::{Language, Strings, strings};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_COMPOSER_ATTACH_BUTTON_SIZE, COMPONENT_COMPOSER_ERROR_PADDING_BLOCK,
    COMPONENT_COMPOSER_ICON_SIZE, COMPONENT_COMPOSER_MARGIN_INLINE, COMPONENT_COMPOSER_RADIUS,
    COMPONENT_COMPOSER_REPLY_BAR_BUTTON_SIZE, COMPONENT_COMPOSER_REPLY_BAR_ICON_SIZE,
    COMPONENT_COMPOSER_REPLY_BAR_MIN_HEIGHT, COMPONENT_COMPOSER_REPLY_BAR_PADDING_INLINE,
    COMPONENT_MESSAGE_EDITOR_HINT_GAP, SIZE_DOCK_HEIGHT, SIZE_DOCK_MARGIN, SPACE_2XS, SPACE_SM,
    SPACE_XS, TYPE_BODY, TYPE_MESSAGE, TYPE_MESSAGE_SUBTEXT,
};
use crate::theme::{color, length, theme};
use crate::tokio_bridge::spawn_on_network_runtime;

use drafts::ComposerDrafts;
pub use drafts::{MAXIMUM_ATTACHMENTS_PER_MESSAGE, TrayItem, TrayItemStatus};
use editor::MessageEditor;
use saved_drafts::{SavedDraft, WriteRequest};
pub use saved_drafts::{SavedDraftsFile, forget_saved_drafts, forget_saved_drafts_now};
use suggestions::{SuggestionList, SuggestionQuery, SuggestionState};

/// Drafts are written this long after typing pauses, never on every keystroke.
const DRAFT_WRITE_DELAY: Duration = Duration::from_secs(1);

/// Typing notices go out at most this often, as the web client sends them.
const TYPING_SEND_INTERVAL: Duration = Duration::from_secs(5);

/// The most read from a file when the instance states no limit (older servers): their default.
const UNSTATED_MAXIMUM_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;

/// The key context of the composer's root; its keys bind inside it.
const COMPOSER_KEY_CONTEXT: &str = "Composer";

actions!(
    composer,
    [
        /// Enter: inserts the suggestion, saves the edit or sends; Shift+Enter keeps the field's
        /// new line.
        EnterInComposer,
        /// Up: the previous suggestion, or the last own message to edit from an empty field.
        UpInComposer,
        /// Down: the next suggestion.
        DownInComposer,
        /// Tab: inserts the suggestion, or moves on as Tab does elsewhere.
        TabInComposer,
        /// Shift+Tab: moves back, as elsewhere.
        ShiftTabInComposer,
        /// Esc: hides the suggestions, cancels the edit or the reply.
        EscapeInComposer
    ]
);

/// Binds the composer's keys over the text field's own; call once at start, after
/// `gpui_component::init`, so these win over the field's bindings for the same keys.
pub fn bind_composer_keys(cx: &mut App) {
    let context = Some("Composer > Input");
    cx.bind_keys([
        KeyBinding::new("enter", EnterInComposer, context),
        KeyBinding::new("up", UpInComposer, context),
        KeyBinding::new("down", DownInComposer, context),
        KeyBinding::new("tab", TabInComposer, context),
        KeyBinding::new("shift-tab", ShiftTabInComposer, context),
        KeyBinding::new("escape", EscapeInComposer, context),
    ]);
}

/// The message a reply answers, as the bar names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyTarget {
    pub message_id: Snowflake,
    pub author_name: SharedString,
}

/// A message to send, its files uploaded already (ADR-0020).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedMessage {
    pub channel_id: Snowflake,
    /// Trimmed; may be empty when files go alone.
    pub text: String,
    pub reply_to: Option<Snowflake>,
    pub attachments: Vec<Attachment>,
}

/// What Esc gave up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cancelled {
    /// The edit of this message, or a save of unchanged text.
    Editing {
        message_id: Snowflake,
    },
    Reply,
    /// Nothing was open: the chat may mark the channel as read (its unread bar).
    Nothing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerEvent {
    Send(ComposedMessage),
    /// Save `text` as the message's new content; an empty text asks to delete it, which the
    /// chat confirms. A refusal comes back through [`Composer::give_back_refused_edit`].
    Edit {
        message_id: Snowflake,
        text: String,
    },
    /// Someone typed in this channel; throttled already.
    Typing {
        channel_id: Snowflake,
    },
    /// Up in an empty field: edit the last own message of the open channel.
    EditLast,
    Cancel(Cancelled),
    /// "Unblock" on the notice of a conversation locked by a block; the answer comes back through
    /// [`Composer::finish_unblocking`].
    Unblock(BlockedRecipient),
}

/// The other person of a one-to-one conversation the user blocked (ADR-0025): the field gives way
/// to a notice with "Unblock".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedRecipient {
    pub user_id: Snowflake,
    pub name: SharedString,
}

/// A failure said above the box until the next send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComposerNotice {
    TooManyAttachments,
    UploadsRunning,
}

/// Whether a typing notice may go out now, given when the last one went.
fn is_typing_notice_due(last_sent_at: Option<Instant>, now: Instant) -> bool {
    last_sent_at
        .is_none_or(|sent_at| now.saturating_duration_since(sent_at) >= TYPING_SEND_INTERVAL)
}

/// The drafts kept across launches (`saved_drafts`): where, and what the file holds.
#[derive(Debug)]
struct KeptDrafts {
    file: Option<SavedDraftsFile>,
    /// The kept drafts are still being read: nothing is written until they are merged in.
    is_reading: bool,
    /// What the file holds, so an unchanged set is not written again.
    written: Vec<SavedDraft>,
    /// Writes once typing pauses; replaced (so cancelled) by every keystroke.
    pending_write: Option<Task<()>>,
    /// The window of the field the open channel's kept text goes back to.
    window: AnyWindowHandle,
}

/// The draft put aside while a message is edited in the same field, and the edit's original.
#[derive(Debug)]
struct EditSession {
    draft_aside: String,
    /// `None` after a refused save reopened: whatever is there then is saved.
    original_text: Option<String>,
}

#[allow(clippy::struct_excessive_bools)] // Independent flags of one view, each read on its own.
pub struct Composer {
    input: Entity<TextareaState>,
    drafts: ComposerDrafts,
    editor: MessageEditor,
    edit_session: Option<EditSession>,
    reply_target: Option<ReplyTarget>,
    channel_label: SharedString,
    mention_candidates: Vec<PublicUser>,
    current_user_id: Option<Snowflake>,
    can_send_messages: bool,
    can_attach_files: bool,
    blocked_recipient: Option<BlockedRecipient>,
    is_unblocking: bool,
    /// The field takes focus at the next render, once it is drawn again (after an unblock).
    is_field_focus_pending: bool,
    rest: Option<RestClient>,
    maximum_attachment_bytes: Option<u64>,
    suggestions: SuggestionState,
    /// The caret and text the suggestions were last read from.
    suggestions_read_at: Option<(usize, usize)>,
    has_tried_sending_too_long: bool,
    notice: Option<ComposerNotice>,
    /// The latest upload result, for screen readers.
    announcement: SharedString,
    last_typing_sent_at: Option<Instant>,
    maximum_rows: usize,
    kept_drafts: KeptDrafts,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ComposerEvent> for Composer {}

impl Composer {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 1));
        let subscriptions = vec![
            cx.subscribe_in(&input, window, Self::on_input_event),
            // Caret moves (arrows, clicks) change the word the suggestions read.
            cx.observe(&input, |composer, _, cx| composer.refresh_suggestions(cx)),
            // Typing within the last second is written before the app goes: at quit while the
            // window is open (Cmd+Q), or as the composer goes with its closed window, where quit
            // observers come too late. The file is small: one blocking write at teardown.
            cx.on_app_quit(|composer, cx| {
                let write = composer.write_kept_drafts(cx);
                async move {
                    if let Some(write) = write {
                        write.await;
                    }
                }
            }),
            cx.on_release(|composer, cx| {
                if let Some((file, drafts, request)) = composer.kept_drafts_to_write(cx) {
                    file.write(&drafts, request);
                }
            }),
        ];
        Self {
            input,
            drafts: ComposerDrafts::default(),
            editor: MessageEditor::default(),
            edit_session: None,
            reply_target: None,
            channel_label: SharedString::default(),
            mention_candidates: Vec::new(),
            current_user_id: None,
            can_send_messages: true,
            can_attach_files: true,
            blocked_recipient: None,
            is_unblocking: false,
            is_field_focus_pending: false,
            rest: None,
            maximum_attachment_bytes: None,
            suggestions: SuggestionState::default(),
            suggestions_read_at: None,
            has_tried_sending_too_long: false,
            notice: None,
            announcement: SharedString::default(),
            last_typing_sent_at: None,
            maximum_rows: 1,
            kept_drafts: KeptDrafts {
                file: None,
                is_reading: false,
                written: Vec::new(),
                pending_write: None,
                window: window.window_handle(),
            },
            _subscriptions: subscriptions,
        }
    }

    // --- Setters the chat pane calls -------------------------------------------------------

    /// Opens `channel_id`'s draft, putting the current one aside; `label` is the field's name and
    /// placeholder ("Message #general"). An edit or reply in progress ends with the switch.
    pub fn set_channel(
        &mut self,
        channel_id: Option<Snowflake>,
        label: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if channel_id != self.drafts.open_channel_id() {
            self.leave_edit_session(window, cx);
            let open_text = self.text(cx);
            if let Some(text) = self.drafts.open(channel_id, open_text) {
                self.set_text(text, window, cx);
            }
            self.reply_target = None;
            self.notice = None;
            self.has_tried_sending_too_long = false;
            self.suggestions.close();
        }
        let placeholder = label.clone();
        self.channel_label = label;
        self.input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
        });
        cx.notify();
    }

    /// Starts or ends a reply; starting one gives the field focus (DESIGN.md §5.5).
    pub fn set_reply_target(
        &mut self,
        reply_target: Option<ReplyTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_starting = reply_target.is_some();
        self.reply_target = reply_target;
        if is_starting && self.can_send_messages {
            self.focus(window, cx);
        }
        cx.notify();
    }

    /// Who "@na" suggests in the open channel, and the writer, who is left out.
    pub fn set_mention_candidates(
        &mut self,
        people: Vec<PublicUser>,
        current_user_id: Option<Snowflake>,
        cx: &mut Context<Self>,
    ) {
        self.mention_candidates = people;
        self.current_user_id = current_user_id;
        self.suggestions_read_at = None;
        self.refresh_suggestions(cx);
    }

    /// "Send messages" and "Attach files" in the open channel; without the first the field is
    /// replaced by the sentence that says so.
    pub fn set_permissions(
        &mut self,
        can_send_messages: bool,
        can_attach_files: bool,
        cx: &mut Context<Self>,
    ) {
        self.can_send_messages = can_send_messages;
        self.can_attach_files = can_attach_files;
        cx.notify();
    }

    /// The person whose block locks the open conversation, if any; `set_permissions` says the
    /// reader cannot send there too.
    pub fn set_blocked_recipient(
        &mut self,
        blocked_recipient: Option<BlockedRecipient>,
        cx: &mut Context<Self>,
    ) {
        if self.blocked_recipient != blocked_recipient {
            self.blocked_recipient = blocked_recipient;
            cx.notify();
        }
    }

    /// The instance answered "Unblock": on success the field, back once the conversation unlocks,
    /// takes focus, as on the web.
    pub fn finish_unblocking(&mut self, is_unblocked: bool, cx: &mut Context<Self>) {
        self.is_unblocking = false;
        self.is_field_focus_pending = is_unblocked;
        cx.notify();
    }

    /// Where the drafts are kept across launches (`None`: in memory only). A new file is read
    /// first: its drafts come back where nothing was typed since, the open channel's in its field.
    pub fn keep_drafts_in(&mut self, file: Option<SavedDraftsFile>, cx: &mut Context<Self>) {
        if self.kept_drafts.file == file {
            return;
        }
        self.kept_drafts.file.clone_from(&file);
        self.kept_drafts.is_reading = file.is_some();
        self.kept_drafts.written.clear();
        self.kept_drafts.pending_write = None;
        let Some(file) = file else {
            return;
        };
        let reading = file.clone();
        let read = cx.background_spawn(async move { reading.read() });
        let window = self.kept_drafts.window;
        cx.spawn(async move |composer, cx| {
            let kept = read.await;
            let _ = cx.update_window(window, |_, window, cx| {
                composer.update(cx, |composer, cx| {
                    composer.restore_kept_drafts(&file, kept, window, cx);
                })
            });
        })
        .detach();
    }

    fn restore_kept_drafts(
        &mut self,
        file: &SavedDraftsFile,
        kept: Vec<SavedDraft>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.kept_drafts.file.as_ref() != Some(file) || !self.kept_drafts.is_reading {
            return;
        }
        self.kept_drafts.is_reading = false;
        self.kept_drafts.written.clone_from(&kept);
        let open_text = self.drafts.restore_texts(kept);
        if let Some(text) = open_text
            && self.edit_session.is_none()
            && self.text(cx).is_empty()
        {
            self.set_text(text, window, cx);
        }
        // Something typed while the file was read is written now.
        self.schedule_kept_drafts_write(cx);
        cx.notify();
    }

    /// Writes the drafts once typing pauses.
    fn schedule_kept_drafts_write(&mut self, cx: &mut Context<Self>) {
        if self.kept_drafts.file.is_none() || self.kept_drafts.is_reading {
            return;
        }
        self.kept_drafts.pending_write = Some(cx.spawn(async move |composer, cx| {
            cx.background_executor().timer(DRAFT_WRITE_DELAY).await;
            let write = composer.update(cx, Self::write_kept_drafts).ok().flatten();
            if let Some(write) = write {
                write.await;
            }
        }));
    }

    /// Writes the drafts off the UI thread when they changed; the write, to wait for at quit.
    fn write_kept_drafts(&mut self, cx: &mut Context<Self>) -> Option<Task<()>> {
        let (file, drafts, request) = self.kept_drafts_to_write(cx)?;
        Some(cx.background_spawn(async move { file.write(&drafts, request) }))
    }

    /// The write the drafts need when they changed since the last one, counted as written.
    fn kept_drafts_to_write(
        &mut self,
        cx: &App,
    ) -> Option<(SavedDraftsFile, Vec<SavedDraft>, WriteRequest)> {
        let file = self
            .kept_drafts
            .file
            .clone()
            .filter(|_| !self.kept_drafts.is_reading)?;
        // During an edit the field holds the message; the draft waits aside.
        let open_text = match &self.edit_session {
            Some(session) => session.draft_aside.clone(),
            None => self.text(cx),
        };
        let drafts = self.drafts.texts_to_keep(&open_text);
        if drafts == self.kept_drafts.written {
            return None;
        }
        self.kept_drafts.written.clone_from(&drafts);
        let request = file.next_write();
        Some((file, drafts, request))
    }

    /// Where files upload, and the instance's size limit from `Ready` (checked before upload).
    pub fn set_uploads(
        &mut self,
        rest: Option<RestClient>,
        maximum_attachment_bytes: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        self.rest = rest;
        self.maximum_attachment_bytes = maximum_attachment_bytes;
        cx.notify();
    }

    /// Edits `message_id` in the field: the draft waits aside and comes back when the edit ends.
    pub fn start_editing(
        &mut self,
        message_id: Snowflake,
        content: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_send_messages {
            return;
        }
        let text = self.editor.open(message_id, content);
        self.begin_edit_session(Some(content.to_owned()), text, window, cx);
    }

    /// A refused save: the editor reopens with `text` when it is free and the message is still in
    /// the open chat (`can_reopen`); otherwise the text waits for the message's next edit.
    pub fn give_back_refused_edit(
        &mut self,
        message_id: Snowflake,
        text: String,
        can_reopen: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Never over a message being written: the text waits for the message's next edit instead.
        let can_reopen = can_reopen && self.edit_session.is_none() && self.text(cx).is_empty();
        if let Some(text) = self.editor.give_back(message_id, text, can_reopen) {
            self.begin_edit_session(None, text, window, cx);
        }
    }

    /// A send the instance refused for good: its text comes back to the field when the field is
    /// empty and still open on `channel_id`, and false leaves it with the caller (the outbox keeps
    /// it as failed), so a message being written is never replaced.
    pub fn give_back_refused_send(
        &mut self,
        channel_id: Snowflake,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let is_free = self.drafts.open_channel_id() == Some(channel_id)
            && self.edit_session.is_none()
            && self.text(cx).is_empty()
            && self.can_send_messages;
        if is_free {
            self.set_text(text, window, cx);
            self.focus(window, cx);
            cx.notify();
        }
        is_free
    }

    /// A printable key pressed in the message list: the field takes focus and the text, as the
    /// web lets typing anywhere in the chat go to the composer.
    /// Types `text` into the field and focuses it; false where the reader cannot send.
    pub fn type_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.can_send_messages {
            return false;
        }
        self.input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.insert(text.to_owned(), window, cx);
        });
        self.note_typing(cx);
        self.suggestions_read_at = None;
        self.refresh_suggestions(cx);
        cx.notify();
        true
    }

    /// Who locks the open conversation, for the conversation view's tests.
    #[cfg(test)]
    pub(crate) fn locking_recipient(&self) -> Option<&BlockedRecipient> {
        self.blocked_recipient
            .as_ref()
            .filter(|_| !self.can_send_messages)
    }

    /// The field's text, for the conversation view's tests.
    #[cfg(test)]
    pub(crate) fn text_in_field(&self, cx: &App) -> String {
        self.text(cx)
    }

    /// The field's focus, for the shell's `F6`.
    #[must_use]
    pub fn field_focus(&self, cx: &App) -> gpui::FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }

    /// Whether the field is drawn: the reader may send in the open channel.
    #[must_use]
    pub fn has_field(&self) -> bool {
        self.can_send_messages
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Files dropped or chosen for the open channel: each uploads at once and waits in the tray.
    pub fn attach_files(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let channel_id = self.drafts.open_channel_id();
        self.attach_files_to(channel_id, paths, window, cx);
    }

    /// Files meant for `channel_id`, the channel open when they were chosen; dropped if another
    /// one is open by the time they arrive, so a file never lands in a channel it was not meant for.
    fn attach_files_to(
        &mut self,
        channel_id: Option<Snowflake>,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_send_messages
            || !self.can_attach_files
            || paths.is_empty()
            || channel_id.is_none()
            || channel_id != self.drafts.open_channel_id()
        {
            return;
        }
        let room = self.drafts.tray_room();
        if paths.len() > room {
            self.notice = Some(ComposerNotice::TooManyAttachments);
        }
        let picked: Vec<PathBuf> = paths.into_iter().take(room).collect();
        // Sizes are read off the UI thread: a network folder can be slow to answer.
        let sizes = cx.background_spawn(async move {
            picked
                .into_iter()
                .filter_map(|path| {
                    let metadata = std::fs::metadata(&path).ok()?;
                    // Only files the person picked, never a folder's contents.
                    metadata.is_file().then_some((path, metadata.len()))
                })
                .collect::<Vec<_>>()
        });
        cx.spawn_in(window, async move |composer, cx| {
            let files = sizes.await;
            composer
                .update(cx, |composer, cx| {
                    if composer.drafts.open_channel_id() != channel_id {
                        return;
                    }
                    for (path, size_bytes) in files {
                        composer.add_to_tray(path, size_bytes, cx);
                    }
                })
                .ok();
        })
        .detach();
        cx.notify();
    }

    // --- Text and caret ---------------------------------------------------------------------

    fn text(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    /// Writes `text` without an input event: no typing notice, suggestions closed.
    fn set_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let end = text.len();
        self.input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.set_selected_range(end..end, cx);
        });
        self.suggestions.close();
        self.suggestions_read_at = None;
    }

    fn on_input_event(
        &mut self,
        _: &Entity<TextareaState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                self.schedule_kept_drafts_write(cx);
                self.note_typing(cx);
                self.suggestions_read_at = None;
                self.refresh_suggestions(cx);
                cx.notify();
            }
            InputEvent::Blur => {
                self.suggestions.close();
                cx.notify();
            }
            InputEvent::PressEnter { .. } | InputEvent::Focus => {}
        }
    }

    fn note_typing(&mut self, cx: &mut Context<Self>) {
        let Some(channel_id) = self.drafts.open_channel_id() else {
            return;
        };
        if self.edit_session.is_some() {
            return;
        }
        let now = Instant::now();
        if is_typing_notice_due(self.last_typing_sent_at, now) {
            self.last_typing_sent_at = Some(now);
            cx.emit(ComposerEvent::Typing { channel_id });
        }
    }

    /// Reads the word before the caret again when the caret or the text moved.
    fn refresh_suggestions(&mut self, cx: &mut Context<Self>) {
        let input = self.input.read(cx);
        let selection = input.selected_range();
        let read_at = (selection.start, selection.end);
        if self.suggestions_read_at == Some(read_at) {
            return;
        }
        self.suggestions_read_at = Some(read_at);
        let caret = (selection.start == selection.end).then_some(selection.end);
        let text = input.value();
        self.suggestions
            .refresh(&text, caret, &self.mention_candidates, self.current_user_id);
        cx.notify();
    }

    fn choose_suggestion(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((suggestion, range)) = self.suggestions.choice(index) else {
            return;
        };
        let (text, caret) = replace_in_draft(
            &self.text(cx),
            range.start,
            range.end,
            &suggestion.written_text(),
        );
        self.set_text(text, window, cx);
        self.input.update(cx, |input, cx| {
            input.set_selected_range(caret..caret, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    // --- Keys ---------------------------------------------------------------------------------

    fn enter(&mut self, _: &EnterInComposer, window: &mut Window, cx: &mut Context<Self>) {
        if !self.suggestions.visible().is_empty() {
            self.choose_suggestion(self.suggestions.active_index(), window, cx);
        } else if self.edit_session.is_some() {
            self.save_edit(window, cx);
        } else {
            self.send(window, cx);
        }
    }

    fn up(&mut self, _: &UpInComposer, _: &mut Window, cx: &mut Context<Self>) {
        if self.suggestions.move_active("up") {
            cx.notify();
        } else if self.edit_session.is_none() && self.text(cx).is_empty() {
            cx.emit(ComposerEvent::EditLast);
        } else {
            cx.propagate();
        }
    }

    fn down(&mut self, _: &DownInComposer, _: &mut Window, cx: &mut Context<Self>) {
        if self.suggestions.move_active("down") {
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn tab(&mut self, _: &TabInComposer, window: &mut Window, cx: &mut Context<Self>) {
        if self.suggestions.visible().is_empty() {
            window.focus_next(cx);
        } else {
            self.choose_suggestion(self.suggestions.active_index(), window, cx);
        }
    }

    fn escape(&mut self, _: &EscapeInComposer, window: &mut Window, cx: &mut Context<Self>) {
        if !self.suggestions.visible().is_empty() {
            self.suggestions.dismiss();
        } else if let Some(message_id) = self.editor.message_id() {
            self.editor.close();
            self.leave_edit_session(window, cx);
            cx.emit(ComposerEvent::Cancel(Cancelled::Editing { message_id }));
        } else if self.reply_target.take().is_some() {
            cx.emit(ComposerEvent::Cancel(Cancelled::Reply));
        } else {
            cx.emit(ComposerEvent::Cancel(Cancelled::Nothing));
        }
        cx.notify();
    }

    // --- Sending and editing --------------------------------------------------------------------

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.drafts.is_uploading() {
            self.notice = Some(ComposerNotice::UploadsRunning);
            cx.notify();
            return;
        }
        let text = self.text(cx);
        if message_characters_left(&text) < 0 {
            self.has_tried_sending_too_long = true;
            cx.notify();
            return;
        }
        let attachments = self.drafts.uploaded_attachments();
        let trimmed = text.trim();
        let Some(channel_id) = self.drafts.open_channel_id() else {
            return;
        };
        if trimmed.is_empty() && attachments.is_empty() {
            return;
        }
        let message = ComposedMessage {
            channel_id,
            text: trimmed.to_owned(),
            reply_to: self
                .reply_target
                .take()
                .map(|reply_target| reply_target.message_id),
            attachments,
        };
        self.has_tried_sending_too_long = false;
        self.notice = None;
        for preview in self
            .drafts
            .clear_tray()
            .into_iter()
            .filter_map(|item| item.preview)
        {
            let _ = window.drop_image(preview);
        }
        self.set_text(String::new(), window, cx);
        self.schedule_kept_drafts_write(cx);
        cx.emit(ComposerEvent::Send(message));
        cx.notify();
    }

    fn begin_edit_session(
        &mut self,
        original_text: Option<String>,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let draft_aside = match self.edit_session.take() {
            Some(session) => session.draft_aside,
            None => self.text(cx),
        };
        self.edit_session = Some(EditSession {
            draft_aside,
            original_text,
        });
        self.set_text(text, window, cx);
        self.focus(window, cx);
        cx.notify();
    }

    /// Gives the draft back to the field after an edit.
    fn leave_edit_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = self.edit_session.take() {
            self.editor.close();
            self.set_text(session.draft_aside, window, cx);
        }
    }

    fn save_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let is_unchanged = self
            .edit_session
            .as_ref()
            .and_then(|session| session.original_text.as_deref())
            .is_some_and(|original_text| original_text.trim() == text.trim());
        let is_too_long = message_characters_left(&text) < 0;
        let Some((message_id, text)) = self.editor.take_text_to_save(text, is_too_long) else {
            cx.notify();
            return;
        };
        self.leave_edit_session(window, cx);
        cx.emit(if is_unchanged {
            ComposerEvent::Cancel(Cancelled::Editing { message_id })
        } else {
            ComposerEvent::Edit {
                message_id,
                text: text.trim().to_owned(),
            }
        });
        cx.notify();
    }

    // --- Attachments ------------------------------------------------------------------------------

    fn open_file_picker(&self, window: &mut Window, cx: &mut Context<Self>) {
        // The picker is not modal on macOS: the person may switch channels while it is open.
        let channel_id = self.drafts.open_channel_id();
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn_in(window, async move |composer, cx| match picked.await {
            Ok(Ok(Some(paths))) => {
                composer
                    .update_in(cx, |composer, window, cx| {
                        composer.attach_files_to(channel_id, paths, window, cx);
                        composer.focus(window, cx);
                    })
                    .ok();
            }
            Ok(Ok(None)) | Err(_) => {}
            Ok(Err(error)) => tracing::warn!(%error, "the file picker failed"),
        })
        .detach();
    }

    fn add_to_tray(&mut self, path: PathBuf, size_bytes: u64, cx: &mut Context<Self>) {
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Checked again here: two drops close together both saw the room left before either landed.
        if self.drafts.tray_room() == 0 {
            self.notice = Some(ComposerNotice::TooManyAttachments);
            return;
        }
        let Some(key) = self.drafts.attach(path, file_name, size_bytes) else {
            return;
        };
        self.start_upload(key, cx);
        cx.notify();
    }

    /// Reads the file (never more than the limit), previews it and uploads it; Retry comes here
    /// too, so a file refused for its size is refused again without being read.
    fn start_upload(&mut self, key: u64, cx: &mut Context<Self>) {
        let maximum_bytes = self
            .maximum_attachment_bytes
            .unwrap_or(UNSTATED_MAXIMUM_ATTACHMENT_BYTES);
        let Some(item) = self.drafts.tray_item_mut(key) else {
            return;
        };
        if item.size_bytes > maximum_bytes {
            self.fail_upload(key, &ClientError::FileTooLarge, cx);
            return;
        }
        let wants_preview = item.preview.is_none();
        let (path, file_name, channel_id) =
            (item.path.clone(), item.file_name.clone(), item.channel_id);
        let Some(rest) = self.rest.clone() else {
            self.fail_upload(key, &ClientError::NotSignedIn, cx);
            return;
        };
        let task = cx.spawn(async move |composer, cx| {
            let read = cx
                .background_spawn(async move {
                    let bytes = read_file_within(&path, maximum_bytes)?;
                    let preview = bytes
                        .as_deref()
                        .filter(|_| wants_preview)
                        .and_then(tray::decode_preview);
                    Ok::<_, std::io::Error>((bytes, preview))
                })
                .await;
            let answer = match read {
                Ok((Some(bytes), preview)) => {
                    let upload = composer.update(cx, |composer, cx| {
                        if let Some(item) = composer.drafts.tray_item_mut(key) {
                            item.preview = item.preview.take().or(preview);
                            cx.notify();
                        }
                        spawn_on_network_runtime(cx, async move {
                            rest.upload_attachment(channel_id, &file_name, bytes).await
                        })
                    });
                    match upload {
                        Ok(upload) => upload.await.unwrap_or(Err(ClientError::UnexpectedResponse(
                            "the upload task stopped".to_owned(),
                        ))),
                        // The composer is gone: nothing to show.
                        Err(_) => return,
                    }
                }
                // The file grew past the limit after its size was read.
                Ok((None, _)) => Err(ClientError::FileTooLarge),
                Err(error) => {
                    tracing::warn!(%error, "cannot read a file picked for upload");
                    Err(ClientError::UnexpectedResponse(
                        "unreadable file".to_owned(),
                    ))
                }
            };
            composer
                .update(cx, |composer, cx| composer.finish_upload(key, answer, cx))
                .ok();
        });
        if let Some(item) = self.drafts.tray_item_mut(key) {
            item.upload = Some(task);
        }
    }

    fn finish_upload(
        &mut self,
        key: u64,
        answer: Result<Attachment, ClientError>,
        cx: &mut Context<Self>,
    ) {
        match answer {
            Ok(attachment) => {
                let Some(item) = self.drafts.tray_item_mut(key) else {
                    return;
                };
                let sentence = strings(cx).chat().uploaded(&item.file_name);
                item.status = TrayItemStatus::Uploaded(attachment);
                item.upload = None;
                self.announce_upload(key, sentence);
            }
            Err(error) => self.fail_upload(key, &error, cx),
        }
        cx.notify();
    }

    fn fail_upload(&mut self, key: u64, error: &ClientError, cx: &mut Context<Self>) {
        let catalogue = strings(cx);
        let host = self
            .rest
            .as_ref()
            .map(|rest| rest.instance().host().to_owned())
            .unwrap_or_default();
        let Some(item) = self.drafts.tray_item_mut(key) else {
            return;
        };
        let sentence =
            upload_error_sentence(error, &item.file_name, item.size_bytes, &host, catalogue);
        item.status = TrayItemStatus::Failed(sentence.clone());
        item.upload = None;
        self.announce_upload(key, sentence);
    }

    /// Only for files still in the open channel's tray: other channels' stay quiet.
    fn announce_upload(&mut self, key: u64, sentence: String) {
        if self.drafts.is_in_open_tray(key) {
            self.announcement = sentence.into();
        }
    }

    fn retry_upload(&mut self, key: u64, cx: &mut Context<Self>) {
        if self.drafts.restart_upload(key).is_some() {
            self.start_upload(key, cx);
            cx.notify();
        }
    }

    fn remove_tray_item(&mut self, key: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(preview) = self
            .drafts
            .remove_tray_item(key)
            .and_then(|item| item.preview)
        {
            // Only fails for a window without an atlas, which holds nothing to free.
            let _ = window.drop_image(preview);
        }
        // The Remove button is gone with its item.
        self.focus(window, cx);
        cx.notify();
    }

    /// Copied files pasted into the field attach; anything else is pasted as text.
    fn paste_files(
        &mut self,
        item: &gpui::ClipboardItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let paths: Vec<PathBuf> = item
            .entries()
            .iter()
            .filter_map(|entry| match entry {
                ClipboardEntry::ExternalPaths(paths) => Some(paths.paths().to_vec()),
                ClipboardEntry::String(_) | ClipboardEntry::Image(_) => None,
            })
            .flatten()
            .collect();
        if paths.is_empty() || !self.can_attach_files {
            return false;
        }
        self.attach_files(paths, window, cx);
        true
    }

    /// As many rows as fit in half the window (DESIGN.md, desktop: "up to half the smallest
    /// window"), then the field scrolls.
    fn follow_window_height(&mut self, window: &Window, cx: &mut Context<Self>) {
        let line_height =
            length(TYPE_MESSAGE.font_size * TYPE_MESSAGE.line_height).to_pixels(window.rem_size());
        let half_window = window.viewport_size().height / 2.0;
        let rows = (half_window / line_height).floor().max(1.0);
        // A window height in rows is a small positive number.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let rows = rows as usize;
        if rows != self.maximum_rows {
            self.maximum_rows = rows;
            self.input
                .update(cx, |input, cx| input.set_auto_grow(1, rows, cx));
        }
    }
}

/// The file's bytes, or `None` when it holds more than `maximum_bytes`: reads at most one byte
/// past the limit, even from a file that grew since its size was read.
fn read_file_within(path: &Path, maximum_bytes: u64) -> std::io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(maximum_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    let is_within = u64::try_from(bytes.len()).is_ok_and(|length| length <= maximum_bytes);
    Ok(is_within.then_some(bytes))
}

/// Why a file did not upload, naming it (DESIGN.md §8): the size and the instance for a file too
/// large, otherwise the instance's reason, never its own text.
fn upload_error_sentence(
    error: &ClientError,
    file_name: &str,
    size_bytes: u64,
    instance_host: &str,
    catalogue: Strings,
) -> String {
    if matches!(error, ClientError::FileTooLarge) {
        return catalogue.chat().file_too_large(
            file_name,
            &written_file_size(size_bytes, catalogue),
            instance_host,
        );
    }
    let reasons = catalogue.errors().reasons();
    let reason = match error {
        ClientError::Network(_) | ClientError::TransferStalled => reasons.network(),
        ClientError::Api { code, .. } => match code {
            ApiErrorCode::ValidationFailed => reasons.validation_failed(),
            ApiErrorCode::RateLimited => reasons.rate_limited(),
            ApiErrorCode::ServerBusy => reasons.server_busy(),
            ApiErrorCode::MissingPermissions => reasons.missing_permissions(),
            ApiErrorCode::NotFound => reasons.not_found(),
            ApiErrorCode::LimitReached => reasons.limit_reached(),
            ApiErrorCode::Conflict => reasons.conflict(),
            _ => catalogue.errors().unknown_reason(),
        },
        _ => catalogue.errors().unknown_reason(),
    };
    catalogue
        .errors()
        .with_reason(&catalogue.chat().upload_failed(file_name), reason)
}

/// "48 MB", « 48 Mo », as the web's `humanFileSize`.
fn written_file_size(bytes: u64, catalogue: Strings) -> String {
    let locale = match catalogue.language() {
        Language::English => Locale::English,
        Language::French => Locale::French,
    };
    match human_file_size(locale, bytes) {
        FileSize::Bytes(bytes) => catalogue
            .file_size()
            .bytes(i64::try_from(bytes).unwrap_or(i64::MAX)),
        FileSize::InUnit { value, unit } => catalogue.file_size().in_unit(
            &value,
            match unit {
                FileSizeUnit::Kilobytes => KbOrMbOrGb::Kb,
                FileSizeUnit::Megabytes => KbOrMbOrGb::Mb,
                FileSizeUnit::Gigabytes => KbOrMbOrGb::Gb,
            },
        ),
    }
}

impl Render for Composer {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.follow_window_height(window, cx);
        if self.is_field_focus_pending && self.can_send_messages {
            self.is_field_focus_pending = false;
            self.focus(window, cx);
        }
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let chat = catalogue.chat();
        let entity = cx.entity();
        let text = self.input.read(cx).value();
        let characters_left = message_characters_left(&text);
        let is_too_long = characters_left < 0;
        let is_editing = self.edit_session.is_some();
        let has_tried_too_long = if is_editing {
            self.editor.has_tried_saving_too_long()
        } else {
            self.has_tried_sending_too_long
        };
        let is_counter_shown = characters_left <= MESSAGE_LENGTH_COUNTER_THRESHOLD;
        let input_focus = self.input.read(cx).focus_handle(cx);
        let is_ring_shown = is_focus_visible(&input_focus, window);

        // The field's name, then what describes it: the reply and the characters left.
        let mut field_label = self.channel_label.to_string();
        if let Some(reply_target) = &self.reply_target {
            field_label = format!(
                "{field_label}, {}",
                chat.reply_hint(&reply_target.author_name)
            );
        }
        if is_counter_shown {
            field_label = format!(
                "{field_label}, {}",
                chat.message_characters_left(characters_left)
            );
        }

        let errors: Vec<SharedString> = self
            .notice
            .map(|notice| match notice {
                ComposerNotice::TooManyAttachments => chat.too_many_attachments(
                    i64::try_from(MAXIMUM_ATTACHMENTS_PER_MESSAGE).unwrap_or(i64::MAX),
                ),
                ComposerNotice::UploadsRunning => chat.uploads_running().to_owned(),
            })
            .into_iter()
            .chain((is_too_long && has_tried_too_long).then(|| {
                chat.message_too_long(i64::try_from(MAXIMUM_MESSAGE_LENGTH).unwrap_or(i64::MAX))
            }))
            .map(SharedString::from)
            .collect();
        let error_lines =
            div()
                .id("composer-errors")
                .role(Role::Alert)
                .children(errors.into_iter().map(|error| {
                    with_type(div(), TYPE_BODY)
                        .py(length(COMPONENT_COMPOSER_ERROR_PADDING_BLOCK))
                        .text_color(color(tokens.component_composer_error_text))
                        .child(error)
                }));

        let suggestion_rows = self.suggestions.visible().to_vec();
        let suggestion_list = (self.can_send_messages && !suggestion_rows.is_empty()).then(|| {
            let title = match self.suggestions.query() {
                Some(SuggestionQuery::Emoji(query)) => chat.emoji_suggestions_title(&query.query),
                Some(SuggestionQuery::Mention(query)) => {
                    chat.mention_suggestions_title(&query.query)
                }
                None => String::new(),
            };
            let choose = entity.clone();
            let hover = entity.clone();
            deferred(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(relative(1.0))
                    .pb(length(SPACE_XS))
                    .child(SuggestionList {
                        title: title.into(),
                        suggestions: suggestion_rows,
                        active_index: self.suggestions.active_index(),
                        on_choose: Arc::new(move |index, window, cx| {
                            choose.update(cx, |composer, cx| {
                                composer.choose_suggestion(index, window, cx);
                            });
                        }),
                        on_hover: Arc::new(move |index, _, cx| {
                            hover.update(cx, |composer, cx| {
                                if composer.suggestions.active_index() != index {
                                    composer.suggestions.hover(index);
                                    cx.notify();
                                }
                            });
                        }),
                    }),
            )
        });

        let bar = |content: gpui::Div| {
            with_type(div(), TYPE_BODY)
                .flex()
                .items_center()
                .gap(length(SPACE_XS))
                .min_h(length(COMPONENT_COMPOSER_REPLY_BAR_MIN_HEIGHT))
                .pl(length(COMPONENT_COMPOSER_REPLY_BAR_PADDING_INLINE))
                .pr(length(SPACE_2XS))
                .bg(color(tokens.component_composer_reply_bar_background))
                .text_color(color(tokens.component_composer_reply_bar_text))
                .child(content.min_w_0().flex_1().truncate())
        };
        let messages = catalogue.messages();
        let top_bar = if is_editing {
            let link = |id: &'static str, word: &'static str, label: &'static str| {
                div()
                    .id(id)
                    .role(Role::Button)
                    .aria_label(label)
                    .cursor_pointer()
                    .text_color(color(tokens.component_message_editor_hint_link))
                    .hover(gpui::Styled::underline)
                    .child(word)
            };
            let cancel = entity.clone();
            let save = entity.clone();
            Some(
                bar(div().child(messages.edit_message())).child(
                    div()
                        .flex()
                        .flex_none()
                        .gap(length(COMPONENT_MESSAGE_EDITOR_HINT_GAP))
                        .pr(length(SPACE_XS))
                        .text_color(color(tokens.component_message_editor_hint_text))
                        .child(messages.esc_to())
                        .child(
                            link(
                                "composer-cancel-edit",
                                messages.cancel_lowercase(),
                                messages.cancel_editing(),
                            )
                            .on_click(move |_, window, cx| {
                                cancel.update(cx, |composer, cx| {
                                    composer.escape(&EscapeInComposer, window, cx);
                                });
                            }),
                        )
                        .child(messages.enter_to())
                        .child(
                            link(
                                "composer-save-edit",
                                messages.save_lowercase(),
                                messages.save_changes(),
                            )
                            .on_click(move |_, window, cx| {
                                save.update(cx, |composer, cx| composer.save_edit(window, cx));
                            }),
                        ),
                ),
            )
        } else {
            self.reply_target
                .as_ref()
                .filter(|_| self.can_send_messages)
                .map(|reply_target| {
                    let close = entity.clone();
                    bar(div()
                        .child(format!("{} ", chat.replying_to()))
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(color(tokens.component_composer_reply_bar_author_text))
                                .child(reply_target.author_name.clone()),
                        )
                        .flex())
                    .child(
                        IconButton::new(
                            "composer-cancel-reply",
                            IconName::XCircle,
                            chat.cancel_reply(),
                        )
                        .sizes(
                            COMPONENT_COMPOSER_REPLY_BAR_BUTTON_SIZE,
                            COMPONENT_COMPOSER_REPLY_BAR_ICON_SIZE,
                        )
                        .on_click(move |window, cx| {
                            close.update(cx, |composer, cx| {
                                composer.reply_target = None;
                                composer.focus(window, cx);
                                cx.emit(ComposerEvent::Cancel(Cancelled::Reply));
                                cx.notify();
                            });
                        }),
                    )
                })
        };

        let tray = (self.can_send_messages && !self.drafts.tray().is_empty())
            .then(|| tray::tray(self.drafts.tray(), &entity, catalogue, window, cx));

        // The base row is as tall as the user card; buttons sit centred on it.
        let button_margin = (SIZE_DOCK_HEIGHT - COMPONENT_COMPOSER_ATTACH_BUTTON_SIZE) / 2.0;
        let base_row = if self.can_send_messages {
            let attach = entity.clone();
            let paste = entity.clone();
            div()
                .flex()
                .items_end()
                .gap(length(SPACE_2XS))
                .px(length(SPACE_XS))
                .when(self.can_attach_files, |row| {
                    row.child(
                        div().my(length(button_margin)).child(
                            IconButton::new(
                                "composer-attach",
                                IconName::PlusCircle,
                                chat.attach_files(),
                            )
                            .sizes(
                                COMPONENT_COMPOSER_ATTACH_BUTTON_SIZE,
                                COMPONENT_COMPOSER_ICON_SIZE,
                            )
                            .on_click(move |window, cx| {
                                attach.update(cx, |composer, cx| {
                                    composer.open_file_picker(window, cx);
                                });
                            }),
                        ),
                    )
                })
                .child(
                    with_type(div(), TYPE_MESSAGE)
                        .flex_1()
                        .min_w_0()
                        // Centred in a row as tall as the user card: the field's own line
                        // height is the text input's, not the message type's.
                        .min_h(length(SIZE_DOCK_HEIGHT))
                        .flex()
                        .flex_col()
                        .justify_center()
                        .px(length(SPACE_2XS))
                        .text_color(color(tokens.component_composer_text))
                        .child(
                            Textarea::new(&self.input)
                                .appearance(false)
                                .aria_label(SharedString::from(field_label))
                                .on_paste(move |item, window, cx| {
                                    paste.update(cx, |composer, cx| {
                                        composer.paste_files(item, window, cx)
                                    })
                                }),
                        ),
                )
                .when(is_counter_shown, |row| {
                    // Discord's counter: only near the limit, the number alone on screen.
                    row.child(
                        with_type(div(), TYPE_MESSAGE_SUBTEXT)
                            .flex()
                            .flex_none()
                            .items_center()
                            .min_h(length(SIZE_DOCK_HEIGHT))
                            .px(length(SPACE_2XS))
                            .text_color(color(if is_too_long {
                                tokens.component_composer_error_text
                            } else {
                                tokens.component_composer_placeholder
                            }))
                            .child(characters_left.to_string()),
                    )
                })
        } else if let Some(recipient) = self.blocked_recipient.clone() {
            // `Composer.svelte`: the sentence, then "Unblock", wrapping below it when narrow.
            let unblock = entity.clone();
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_x(length(SPACE_SM))
                .gap_y(length(SPACE_2XS))
                .min_h(length(SIZE_DOCK_HEIGHT))
                .px(length(SPACE_XS))
                .py(length(SPACE_2XS))
                .child(
                    with_type(div(), TYPE_MESSAGE)
                        .text_color(color(tokens.component_composer_locked_text))
                        .child(chat.you_blocked(&recipient.name)),
                )
                .child(
                    Button::new("composer-unblock", catalogue.friends().unblock())
                        .disabled(self.is_unblocking)
                        .on_click(move |_, cx| {
                            unblock.update(cx, |composer, cx| {
                                if composer.is_unblocking {
                                    return;
                                }
                                composer.is_unblocking = true;
                                cx.emit(ComposerEvent::Unblock(recipient.clone()));
                                cx.notify();
                            });
                        }),
                )
        } else {
            with_type(div(), TYPE_MESSAGE)
                .flex()
                .items_center()
                .min_h(length(SIZE_DOCK_HEIGHT))
                .px(length(SPACE_XS))
                .text_color(color(tokens.component_composer_locked_text))
                .child(chat.cannot_send())
        };

        let can_drop = self.can_send_messages && self.can_attach_files;
        let drop_target = entity.clone();
        let hover_wash = color(tokens.color_state_hover);
        let composer_box = div()
            .id("composer-box")
            .debug_selector(|| "composer".to_owned())
            .relative()
            .rounded(length(COMPONENT_COMPOSER_RADIUS))
            .bg(color(tokens.component_composer_background))
            .can_drop(move |dragged, _, _| can_drop && dragged.is::<ExternalPaths>())
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(hover_wash))
            .on_drop(move |paths: &ExternalPaths, window, cx| {
                drop_target.update(cx, |composer, cx| {
                    composer.attach_files(paths.paths().to_vec(), window, cx);
                });
            })
            .child(
                // The field's top corners square off under the bar (DESIGN.md §5.5).
                div()
                    .rounded(length(COMPONENT_COMPOSER_RADIUS))
                    .overflow_hidden()
                    .children(top_bar)
                    .children(tray)
                    .child(base_row),
            )
            .children(suggestion_list)
            .when(is_ring_shown, |composer_box| {
                composer_box.child(focus_ring(COMPONENT_COMPOSER_RADIUS, cx))
            });

        div()
            .id("composer")
            .key_context(COMPOSER_KEY_CONTEXT)
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::tab))
            .on_action(|_: &ShiftTabInComposer, window, cx| window.focus_prev(cx))
            .on_action(cx.listener(Self::escape))
            .flex_none()
            .flex()
            .flex_col()
            .px(length(COMPONENT_COMPOSER_MARGIN_INLINE))
            .pb(length(SIZE_DOCK_MARGIN))
            .child(error_lines)
            .child(composer_box)
            .child(
                // Upload results, for screen readers only.
                div()
                    .id("composer-announcement")
                    .role(Role::Status)
                    .aria_label(self.announcement.clone())
                    .size_0()
                    .overflow_hidden(),
            )
    }
}

impl std::fmt::Debug for Composer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Composer")
            .field("channel_id", &self.drafts.open_channel_id())
            .field("editing", &self.editor.message_id())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
