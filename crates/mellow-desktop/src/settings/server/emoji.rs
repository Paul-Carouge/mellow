//! Server settings → Emoji (`EmojiSettings.svelte`, DESIGN.md §5.8, §6.58, ADR-0028). Pictures
//! upload as soon as they are chosen, one after another, named after their file; each is renamed
//! in place (Enter or leaving the field saves, Esc puts the name back). Still and animated emoji
//! are listed apart, each kind with its own count, since each has its own room of 50. The list
//! follows the gateway.
//!
//! shortcut: an upload waiting in the list shows a file icon, not the picture's preview; add one
//! with a shared preview decoder (the composer's tray has its own).

use std::collections::HashMap;
use std::path::PathBuf;

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, KeyDownEvent, ObjectFit, ParentElement as _,
    PathPromptOptions, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledImage as _, Subscription, Window, div, img, prelude::FluentBuilder as _,
};
use gpui_component::input::{InputEvent, InputState};
use mellow_client_core::shared_logic::emoji_uploads::{
    EmojiCounts, EmojiFileProblem, EmojiRenameProblem, EmojiUploadRefusal,
    MAXIMUM_EMOJI_NAME_LENGTH, MAXIMUM_EMOJI_UPLOAD_BYTES, MAXIMUM_EMOJIS_PER_GUILD_AND_KIND,
    emoji_counts, emoji_file_problem, emoji_name_from_file_name, emoji_rename_problem,
    emoji_upload_refusal,
};
use mellow_client_core::shared_logic::server_settings::focus_after_removal;
use mellow_client_core::{ClientError, DownloadableFile};
use mellow_protocol::{CustomEmoji, Snowflake};

use super::list_rows::{list_row, secondary_line, status_notice, text_column};
use crate::components::{
    Button, ButtonVariant, IconButton, IconButtonTone, ModalDialog, ModalDialogEvent, TextField,
    icon, with_type,
};
use crate::composer::read_file_within;
use crate::i18n::{Strings, strings};
use crate::messages::{file_size_label, picture_animation, picture_for_box};
use crate::session::Session;
use crate::settings::{SettingsPage, SettingsPageEvent};
use crate::sign_in_form::{describe_error, error_reason};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_COMPOSER_TRAY_ITEM_UPLOADING_OPACITY, COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE,
    COMPONENT_CUSTOM_EMOJI_SETTINGS_NAME_MAX_WIDTH,
    COMPONENT_CUSTOM_EMOJI_SETTINGS_PICTURE_PADDING,
    COMPONENT_CUSTOM_EMOJI_SETTINGS_PICTURE_RADIUS, COMPONENT_CUSTOM_EMOJI_SETTINGS_PICTURE_SIZE,
    COMPONENT_SERVER_SETTINGS_ROW_PADDING_BLOCK, COMPONENT_SERVER_SETTINGS_TEXT_GAP,
    COMPONENT_SERVER_SETTINGS_TOOLBAR_GAP, SIZE_HIT_WEB_MIN, SPACE_2XS, SPACE_SM, TYPE_BODY,
    TYPE_BODY_LG, TYPE_BODY_SM, TYPE_CATEGORY,
};
use crate::theme::{color, length, theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UploadStatus {
    Waiting,
    Adding,
    Failed,
}

#[derive(Debug, Clone)]
struct Upload {
    key: u64,
    path: PathBuf,
    file_name: String,
    size_bytes: u64,
    name: String,
    status: UploadStatus,
    error: String,
    /// False when the file itself is wrong: trying again would fail the same way.
    can_retry: bool,
    dismiss_focus: FocusHandle,
}

/// One emoji's name field and what it last asked.
struct NameField {
    input: Entity<InputState>,
    /// The stored name the field was last set to: the field holds a draft once it differs.
    shown_stored_name: String,
    /// The name asked for, until the gateway's list shows it or the request fails.
    requested_name: Option<String>,
    error: Option<SharedString>,
    _subscription: Subscription,
}

struct DeleteDialog {
    emoji_id: Snowflake,
    dialog: Entity<ModalDialog>,
    _subscription: Subscription,
}

pub struct ServerEmojiPage {
    session: Entity<Session>,
    guild_id: Snowflake,
    uploads: Vec<Upload>,
    next_upload_key: u64,
    is_uploading: bool,
    notice: SharedString,
    name_fields: HashMap<Snowflake, NameField>,
    deleting: Option<DeleteDialog>,
    upload_focus: FocusHandle,
    /// Where focus goes once the next frame is drawn: a row went away with the control in it.
    pending_focus: Option<FocusHandle>,
    _session_changes: Subscription,
}

impl EventEmitter<SettingsPageEvent> for ServerEmojiPage {}

impl SettingsPage for ServerEmojiPage {
    fn set_attention(&mut self, _attention: u32, _cx: &mut Context<Self>) {}
}

impl std::fmt::Debug for ServerEmojiPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerEmojiPage")
            .field("guild_id", &self.guild_id)
            .field("uploads", &self.uploads.len())
            .finish_non_exhaustive()
    }
}

impl ServerEmojiPage {
    pub fn new(
        session: Entity<Session>,
        guild_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let session_changes = cx.observe_in(&session, window, |page, _, window, cx| {
            page.follow_stored_names(window, cx);
            cx.notify();
        });
        let mut page = Self {
            session,
            guild_id,
            uploads: Vec::new(),
            next_upload_key: 0,
            is_uploading: false,
            notice: SharedString::default(),
            name_fields: HashMap::new(),
            deleting: None,
            upload_focus: cx.focus_handle(),
            pending_focus: None,
            _session_changes: session_changes,
        };
        page.follow_stored_names(window, cx);
        page
    }

    fn emojis(&self, cx: &gpui::App) -> Vec<CustomEmoji> {
        self.session
            .read(cx)
            .state()
            .guild(self.guild_id)
            .map(|guild| guild.emojis.clone())
            .unwrap_or_default()
    }

    /// One field per emoji; a rename by someone else shows at once in an untouched field, and the
    /// stored name catching up with the asked one ends the wait.
    fn follow_stored_names(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let emojis = self.emojis(cx);
        self.name_fields
            .retain(|emoji_id, _| emojis.iter().any(|emoji| emoji.id == *emoji_id));
        for emoji in emojis {
            if let Some(field) = self.name_fields.get_mut(&emoji.id) {
                if field.shown_stored_name == emoji.name {
                    continue;
                }
                if field.input.read(cx).value() == field.shown_stored_name {
                    field.input.update(cx, |input, cx| {
                        input.set_value(emoji.name.clone(), window, cx);
                    });
                }
                if field.requested_name.as_deref() == Some(emoji.name.as_str()) {
                    field.requested_name = None;
                }
                field.shown_stored_name.clone_from(&emoji.name);
                continue;
            }
            let input = cx.new(|cx| {
                let mut input = InputState::new(window, cx)
                    .validate(|text, _| text.len() <= MAXIMUM_EMOJI_NAME_LENGTH);
                input.set_value(emoji.name.clone(), window, cx);
                input
            });
            let emoji_id = emoji.id;
            let subscription =
                cx.subscribe_in(&input, window, move |page, _, event: &InputEvent, _, cx| {
                    match event {
                        InputEvent::PressEnter { .. } | InputEvent::Blur => {
                            page.rename(emoji_id, cx);
                        }
                        _ => {}
                    }
                });
            self.name_fields.insert(
                emoji.id,
                NameField {
                    input,
                    shown_stored_name: emoji.name.clone(),
                    requested_name: None,
                    error: None,
                    _subscription: subscription,
                },
            );
        }
    }

    // ---- Uploads ----

    fn choose_files(window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn_in(window, async move |page, cx| {
            let paths = match picked.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(error)) => {
                    tracing::warn!(%error, "the file picker failed");
                    return;
                }
            };
            let sized = cx
                .background_spawn(async move {
                    paths
                        .into_iter()
                        .map(|path| {
                            let size_bytes = std::fs::metadata(&path).map(|meta| meta.len());
                            (path, size_bytes.ok())
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = page.update(cx, |page, cx| page.add_files(sized, cx));
        })
        .detach();
    }

    /// Each chosen file waits its turn, or fails at once when its problem is known before sending;
    /// a file whose size could not be read (`None`) is listed as unreadable.
    fn add_files(&mut self, files: Vec<(PathBuf, Option<u64>)>, cx: &mut Context<Self>) {
        let emojis = self.emojis(cx);
        let counts = emoji_counts(&emojis);
        let mut taken_names: Vec<String> = emojis
            .iter()
            .map(|emoji| emoji.name.clone())
            .chain(self.uploads.iter().map(|upload| upload.name.clone()))
            .collect();
        let catalogue = strings(cx);
        for (path, read_size) in files {
            let file_name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let name =
                emoji_name_from_file_name(&file_name, taken_names.iter().map(String::as_str));
            taken_names.push(name.clone());
            let size_bytes = read_size.unwrap_or_default();
            let problem = read_size.and_then(|size| emoji_file_problem(&file_name, size, counts));
            let unreadable = read_size.is_none().then(|| unreadable_text(&file_name, cx));
            let error = problem
                .map(|problem| {
                    let reasons = catalogue.server_settings().emoji();
                    let reason = match problem {
                        EmojiFileProblem::NotAPicture => reasons.not_a_picture().to_owned(),
                        EmojiFileProblem::TooLarge => {
                            reasons.too_large(&file_size_label(size_bytes, catalogue))
                        }
                        EmojiFileProblem::NoRoom => reasons.no_room().to_owned(),
                    };
                    failure_text(&file_name, &reason, catalogue)
                })
                .or(unreadable)
                .unwrap_or_default();
            self.uploads.push(Upload {
                key: self.next_upload_key,
                path,
                file_name,
                size_bytes,
                name,
                status: if problem.is_some() || read_size.is_none() {
                    UploadStatus::Failed
                } else {
                    UploadStatus::Waiting
                },
                error,
                can_retry: read_size.is_some()
                    && matches!(problem, None | Some(EmojiFileProblem::NoRoom)),
                dismiss_focus: cx.focus_handle(),
            });
            self.next_upload_key += 1;
        }
        self.upload_next(cx);
        cx.notify();
    }

    /// One at a time, so a refusal for too many pictures (the budget is shared with avatars and
    /// icons) stops the rest: they wait with "Try again" rather than all failing the same way.
    fn upload_next(&mut self, cx: &mut Context<Self>) {
        if self.is_uploading {
            return;
        }
        let Some(upload) = self
            .uploads
            .iter_mut()
            .find(|upload| upload.status == UploadStatus::Waiting)
        else {
            return;
        };
        upload.status = UploadStatus::Adding;
        self.is_uploading = true;
        let (key, path, name) = (upload.key, upload.path.clone(), upload.name.clone());
        let (guild_id, session) = (self.guild_id, self.session.clone());
        let read =
            cx.background_spawn(async move { read_file_within(&path, MAXIMUM_EMOJI_UPLOAD_BYTES) });
        cx.spawn(async move |page, cx| {
            let picture = match read.await {
                Ok(Some(bytes)) => bytes,
                // The file grew past the limit after its size was read.
                Ok(None) => {
                    let _ = page.update(cx, |page, cx| {
                        page.finish_upload(key, Err(ClientError::FileTooLarge), cx);
                    });
                    return;
                }
                Err(error) => {
                    tracing::warn!(%error, "cannot read a picture picked for an emoji");
                    let _ = page.update(cx, |page, cx| page.fail_unreadable(key, cx));
                    return;
                }
            };
            let () = session.update(cx, |session, cx| {
                session.send_settings_request(
                    "create an emoji",
                    move |rest| async move { rest.create_emoji(guild_id, &name, picture).await },
                    cx,
                    move |answer, cx| {
                        let _ = page.update(cx, |page, cx| page.finish_upload(key, answer, cx));
                    },
                );
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_upload(
        &mut self,
        key: u64,
        answer: Result<CustomEmoji, ClientError>,
        cx: &mut Context<Self>,
    ) {
        self.is_uploading = false;
        let catalogue = strings(cx);
        let reasons = catalogue.server_settings().emoji();
        match answer {
            Ok(created) => {
                self.uploads.retain(|upload| upload.key != key);
                self.notice = reasons.added(&created.name).into();
            }
            Err(error) => {
                let refusal = emoji_upload_refusal(&error);
                if let Some(upload) = self.uploads.iter_mut().find(|upload| upload.key == key) {
                    let reason = match refusal {
                        EmojiUploadRefusal::TooLarge => {
                            reasons.too_large(&file_size_label(upload.size_bytes, catalogue))
                        }
                        EmojiUploadRefusal::NoRoom => reasons.no_room().to_owned(),
                        EmojiUploadRefusal::Unusable => reasons.unusable().to_owned(),
                        EmojiUploadRefusal::TooMany => reasons.too_many().to_owned(),
                        EmojiUploadRefusal::Other => {
                            error_reason(&error, crate::strings_pending_i18n::strings(cx))
                                .to_owned()
                        }
                    };
                    upload.status = UploadStatus::Failed;
                    upload.error = failure_text(&upload.file_name, &reason, catalogue);
                    self.notice = upload.error.clone().into();
                }
                if refusal == EmojiUploadRefusal::TooMany {
                    for waiting in self
                        .uploads
                        .iter_mut()
                        .filter(|upload| upload.status == UploadStatus::Waiting)
                    {
                        waiting.status = UploadStatus::Failed;
                        waiting.error =
                            failure_text(&waiting.file_name, reasons.too_many(), catalogue);
                    }
                }
            }
        }
        self.upload_next(cx);
        cx.notify();
    }

    /// A picked file that could not be read here: nothing was sent, and trying again won't help.
    fn fail_unreadable(&mut self, key: u64, cx: &mut Context<Self>) {
        self.is_uploading = false;
        if let Some(upload) = self.uploads.iter_mut().find(|upload| upload.key == key) {
            upload.status = UploadStatus::Failed;
            upload.error = unreadable_text(&upload.file_name, cx);
            upload.can_retry = false;
            self.notice = upload.error.clone().into();
        }
        self.upload_next(cx);
        cx.notify();
    }

    fn dismiss_upload(&mut self, key: u64, cx: &mut Context<Self>) {
        let keys: Vec<u64> = self.uploads.iter().map(|upload| upload.key).collect();
        let next = focus_after_removal(&keys, key);
        self.uploads.retain(|upload| upload.key != key);
        self.pending_focus = Some(
            next.and_then(|next| self.uploads.iter().find(|upload| upload.key == next))
                .map_or_else(
                    || self.upload_focus.clone(),
                    |upload| upload.dismiss_focus.clone(),
                ),
        );
        cx.notify();
    }

    fn retry_upload(&mut self, key: u64, cx: &mut Context<Self>) {
        // Named again: an emoji added meanwhile may have taken the name.
        let taken: Vec<String> = self
            .emojis(cx)
            .into_iter()
            .map(|emoji| emoji.name)
            .chain(
                self.uploads
                    .iter()
                    .filter(|upload| upload.key != key)
                    .map(|upload| upload.name.clone()),
            )
            .collect();
        if let Some(upload) = self.uploads.iter_mut().find(|upload| upload.key == key) {
            upload.name =
                emoji_name_from_file_name(&upload.file_name, taken.iter().map(String::as_str));
            upload.status = UploadStatus::Waiting;
            upload.error.clear();
        }
        self.upload_next(cx);
        cx.notify();
    }

    // ---- Renaming ----

    fn rename(&mut self, emoji_id: Snowflake, cx: &mut Context<Self>) {
        let emojis = self.emojis(cx);
        let Some(stored) = emojis.iter().find(|emoji| emoji.id == emoji_id) else {
            return;
        };
        let Some(field) = self.name_fields.get_mut(&emoji_id) else {
            return;
        };
        let name = field.input.read(cx).value().trim().to_owned();
        if field.requested_name.as_deref() == Some(stored.name.as_str()) {
            // The gateway's list brought it: nothing is waiting any more.
            field.requested_name = None;
        }
        if name == stored.name {
            field.error = None;
            cx.notify();
            return;
        }
        // Enter, then leaving the field, would ask twice while the gateway's list is on its way.
        if field.requested_name.as_deref() == Some(name.as_str()) {
            return;
        }
        let words = strings(cx).server_settings().emoji();
        if let Some(problem) = emoji_rename_problem(&name, emoji_id, &emojis) {
            field.error = Some(
                match problem {
                    EmojiRenameProblem::Taken => words.name_taken(),
                    EmojiRenameProblem::Invalid => words.name_invalid(),
                }
                .into(),
            );
            cx.notify();
            return;
        }
        field.error = None;
        field.requested_name = Some(name.clone());
        let guild_id = self.guild_id;
        let page = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            // The field keeps the typed name; the gateway's list brings the same one back.
            session.send_settings_change(
                "rename an emoji",
                words.rename_failed().to_owned(),
                move |rest| async move { rest.rename_emoji(guild_id, emoji_id, &name).await },
                cx,
                move |outcome, cx| {
                    if let Err(sentence) = outcome {
                        let _ = page.update(cx, |page, cx| {
                            if let Some(field) = page.name_fields.get_mut(&emoji_id) {
                                field.requested_name = None;
                                field.error = Some(sentence.into());
                            }
                            cx.notify();
                        });
                    }
                },
            );
        });
        cx.notify();
    }

    /// Esc first puts the name back; a second Esc closes the settings.
    fn restore_name(
        &mut self,
        emoji_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(field) = self.name_fields.get_mut(&emoji_id) else {
            return false;
        };
        let is_changed = field.input.read(cx).value() != field.shown_stored_name;
        if !is_changed && field.error.is_none() {
            return false;
        }
        let stored = field.shown_stored_name.clone();
        field
            .input
            .update(cx, |input, cx| input.set_value(stored, window, cx));
        field.error = None;
        cx.notify();
        true
    }

    // ---- Deleting ----

    fn ask_to_delete(&mut self, emoji: &CustomEmoji, window: &mut Window, cx: &mut Context<Self>) {
        let catalogue = strings(cx);
        let words = catalogue.server_settings().emoji();
        let dialog = cx.new(|cx| {
            ModalDialog::new(
                words.delete_title(&emoji.name),
                catalogue.common().cancel(),
                window,
                cx,
            )
            .subtitle(words.delete_body())
            .submit(words.delete_submit(), words.deleting(), true)
        });
        let subscription = cx.subscribe(
            &dialog,
            |page, _, event: &ModalDialogEvent, cx| match event {
                ModalDialogEvent::Submitted => page.delete(cx),
                ModalDialogEvent::Closed => {
                    page.deleting = None;
                    cx.notify();
                }
            },
        );
        self.deleting = Some(DeleteDialog {
            emoji_id: emoji.id,
            dialog,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(deleting) = self.deleting.as_ref() else {
            return;
        };
        let (emoji_id, dialog) = (deleting.emoji_id, deleting.dialog.clone());
        if dialog.read(cx).is_busy() {
            return;
        }
        let Some(name) = self
            .emojis(cx)
            .into_iter()
            .find(|emoji| emoji.id == emoji_id)
            .map(|emoji| emoji.name)
        else {
            self.deleting = None;
            cx.notify();
            return;
        };
        dialog.update(cx, |dialog, cx| {
            dialog.set_busy(true, cx);
            dialog.set_error(None, cx);
        });
        // In the order shown: still ones, then animated ones.
        let shown_ids: Vec<Snowflake> = kinds_in_order(&self.emojis(cx))
            .into_iter()
            .flat_map(|(_, list)| list)
            .map(|emoji| emoji.id)
            .collect();
        let guild_id = self.guild_id;
        let page = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_settings_request(
                "delete an emoji",
                move |rest| async move { rest.delete_emoji(guild_id, emoji_id).await },
                cx,
                move |answer, cx| {
                    dialog.update(cx, |dialog, cx| dialog.set_busy(false, cx));
                    match answer {
                        Ok(()) => {
                            let _ = page.update(cx, |page, cx| {
                                let next = focus_after_removal(&shown_ids, emoji_id);
                                page.pending_focus = Some(
                                    next.and_then(|next| page.name_fields.get(&next))
                                        .map_or_else(
                                            || page.upload_focus.clone(),
                                            |field| field.input.focus_handle(cx),
                                        ),
                                );
                                page.deleting = None;
                                page.notice =
                                    strings(cx).server_settings().emoji().deleted(&name).into();
                                cx.notify();
                            });
                        }
                        Err(error) => {
                            let summary = strings(cx).server_settings().emoji().delete_failed();
                            let sentence = describe_error(
                                &error,
                                summary,
                                crate::strings_pending_i18n::strings(cx),
                            );
                            dialog.update(cx, |dialog, cx| {
                                dialog.set_error(Some(sentence.into()), cx);
                            });
                        }
                    }
                },
            );
        });
    }

    // ---- Drawing ----

    fn render_upload_row(upload: &Upload, cx: &mut Context<Self>) -> gpui::AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.server_settings().emoji();
        let key = upload.key;
        let is_failed = upload.status == UploadStatus::Failed;
        let status_line = if is_failed {
            with_type(div(), TYPE_BODY_SM)
                .text_color(color(tokens.component_settings_error_text))
                .child(upload.error.clone())
        } else {
            secondary_line(
                if upload.status == UploadStatus::Adding {
                    words.adding()
                } else {
                    words.waiting()
                },
                cx,
            )
        };
        let retry_page = cx.entity().downgrade();
        let dismiss_page = retry_page.clone();
        list_row(cx)
            .id(("emoji-upload", key))
            .role(Role::ListItem)
            .child(
                picture_frame(cx)
                    .when(!is_failed, |frame| {
                        frame.opacity(COMPONENT_COMPOSER_TRAY_ITEM_UPLOADING_OPACITY)
                    })
                    .child(icon(
                        IconName::File,
                        COMPONENT_CUSTOM_EMOJI_SETTINGS_PICTURE_SIZE
                            - 2.0 * COMPONENT_CUSTOM_EMOJI_SETTINGS_PICTURE_PADDING,
                        color(tokens.component_server_settings_secondary_text),
                    )),
            )
            .child(
                text_column()
                    .child(
                        with_type(div(), TYPE_BODY_LG)
                            .truncate()
                            .text_color(color(tokens.component_server_settings_code_text))
                            .child(upload.name.clone()),
                    )
                    .child(status_line),
            )
            .when(is_failed && upload.can_retry, |row| {
                row.child(
                    Button::new(("emoji-upload-retry", key), catalogue.common().try_again())
                        .accessible_label(words.retry(&upload.file_name))
                        .on_click(move |_, cx| {
                            let _ = retry_page.update(cx, |page, cx| page.retry_upload(key, cx));
                        }),
                )
            })
            .when(is_failed, |row| {
                row.child(
                    IconButton::new(
                        ("emoji-upload-dismiss", key),
                        IconName::X,
                        words.dismiss(&upload.file_name),
                    )
                    .sizes(SIZE_HIT_WEB_MIN, COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE)
                    .track_focus(&upload.dismiss_focus)
                    .on_click(move |_, cx| {
                        let _ = dismiss_page.update(cx, |page, cx| page.dismiss_upload(key, cx));
                    }),
                )
            })
            .into_any_element()
    }

    fn render_emoji_row(
        &self,
        emoji: &CustomEmoji,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let words = strings(cx).server_settings().emoji();
        let Some(field) = self.name_fields.get(&emoji.id) else {
            return div().into_any_element();
        };
        let mut name_field = TextField::new(
            ("emoji-name", emoji.id.raw()),
            words.name_label(&emoji.name),
            &field.input,
        )
        .label_hidden();
        if let Some(error) = field.error.clone() {
            name_field = name_field.error(error);
        }
        let emoji_id = emoji.id;
        let escape_page = cx.entity().downgrade();
        let delete_page = escape_page.clone();
        let deleted = emoji.clone();
        list_row(cx)
            .id(("emoji", emoji.id.raw()))
            .role(Role::ListItem)
            .child(
                picture_frame(cx).child(
                    img(picture_for_box(
                        DownloadableFile::Icon(emoji.id),
                        COMPONENT_CUSTOM_EMOJI_SETTINGS_PICTURE_SIZE,
                        COMPONENT_CUSTOM_EMOJI_SETTINGS_PICTURE_SIZE,
                        picture_animation(cx),
                        window,
                    ))
                    .size_full()
                    .object_fit(ObjectFit::Contain),
                ),
            )
            .child(
                text_column().child(
                    div()
                        .max_w(length(COMPONENT_CUSTOM_EMOJI_SETTINGS_NAME_MAX_WIDTH))
                        .on_key_down(move |event: &KeyDownEvent, window, cx| {
                            if event.keystroke.key != "escape"
                                || event.keystroke.modifiers.modified()
                            {
                                return;
                            }
                            let is_restored = escape_page
                                .update(cx, |page, cx| page.restore_name(emoji_id, window, cx))
                                .unwrap_or(false);
                            if is_restored {
                                cx.stop_propagation();
                            }
                        })
                        .child(name_field),
                ),
            )
            .child(
                IconButton::new(
                    ("emoji-delete", emoji.id.raw()),
                    IconName::Trash,
                    words.delete_label(&emoji.name),
                )
                .sizes(SIZE_HIT_WEB_MIN, COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE)
                .tone(IconButtonTone::Danger)
                .on_click(move |window, cx| {
                    let _ =
                        delete_page.update(cx, |page, cx| page.ask_to_delete(&deleted, window, cx));
                }),
            )
            .into_any_element()
    }

    fn render_kind(
        &self,
        is_animated: bool,
        list: &[CustomEmoji],
        counts: EmojiCounts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.server_settings().emoji();
        let (title, used, is_full, empty) = if is_animated {
            (
                words.animated(),
                counts.animated,
                counts.is_animated_full(),
                words.no_animated(),
            )
        } else {
            (
                words.still(),
                counts.still,
                counts.is_still_full(),
                words.no_still(),
            )
        };
        let used = i64::try_from(used).unwrap_or(i64::MAX);
        let maximum = i64::try_from(MAXIMUM_EMOJIS_PER_GUILD_AND_KIND).unwrap_or(i64::MAX);
        let count = if is_full {
            format!("{} · {}", words.count(used, maximum), words.full())
        } else {
            words.count(used, maximum)
        };
        let heading = with_type(div(), TYPE_CATEGORY)
            .id(if is_animated {
                "emoji-animated-heading"
            } else {
                "emoji-still-heading"
            })
            .role(Role::Heading)
            .aria_label(format!("{title}, {}", words.count_label(used, maximum)))
            .flex()
            .items_baseline()
            .justify_between()
            .gap(length(SPACE_SM))
            .pb(length(SPACE_2XS))
            .border_b_1()
            .border_color(color(tokens.component_server_settings_row_divider))
            .text_color(color(tokens.component_settings_nav_section_text))
            .child(title)
            .child(
                div()
                    .when(is_full, |count| {
                        count.text_color(color(
                            tokens.component_custom_emoji_settings_count_full_text,
                        ))
                    })
                    .child(count),
            );
        let body = if list.is_empty() {
            with_type(div(), TYPE_BODY)
                .py(length(COMPONENT_SERVER_SETTINGS_ROW_PADDING_BLOCK))
                .text_color(color(tokens.component_server_settings_secondary_text))
                .child(empty)
                .into_any_element()
        } else {
            div()
                .id(if is_animated {
                    "emoji-animated-list"
                } else {
                    "emoji-still-list"
                })
                .role(Role::List)
                .aria_label(title)
                .children(
                    list.iter()
                        .map(|emoji| self.render_emoji_row(emoji, window, cx))
                        .collect::<Vec<_>>(),
                )
                .into_any_element()
        };
        div().child(heading).child(body).into_any_element()
    }
}

/// "Couldn't add party.png: it isn't a PNG, JPEG, GIF or WebP picture."
fn failure_text(file_name: &str, reason: &str, catalogue: Strings) -> String {
    catalogue.errors().with_reason(
        &catalogue.server_settings().emoji().add_failed(file_name),
        reason,
    )
}

fn unreadable_text(file_name: &str, cx: &gpui::App) -> String {
    let reason = crate::strings_pending_i18n::strings(cx)
        .error_reasons
        .file_unreadable;
    failure_text(file_name, reason, strings(cx))
}

/// Still emoji, then animated ones, each in the server's order.
fn kinds_in_order(emojis: &[CustomEmoji]) -> [(bool, Vec<CustomEmoji>); 2] {
    let (animated, still): (Vec<_>, Vec<_>) =
        emojis.iter().cloned().partition(|emoji| emoji.is_animated);
    [(false, still), (true, animated)]
}

fn picture_frame(cx: &gpui::App) -> gpui::Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(length(COMPONENT_CUSTOM_EMOJI_SETTINGS_PICTURE_SIZE))
        .p(length(COMPONENT_CUSTOM_EMOJI_SETTINGS_PICTURE_PADDING))
        .rounded(length(COMPONENT_CUSTOM_EMOJI_SETTINGS_PICTURE_RADIUS))
        .bg(color(
            theme(cx).component_custom_emoji_settings_picture_background,
        ))
}

impl Render for ServerEmojiPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(focus) = self.pending_focus.take() {
            window.focus(&focus, cx);
        }
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.server_settings().emoji();
        let emojis = self.emojis(cx);
        let counts = emoji_counts(&emojis);
        let is_both_full = counts.is_still_full() && counts.is_animated_full();
        let page = cx.entity().downgrade();
        let uploads: Vec<gpui::AnyElement> = self
            .uploads
            .iter()
            .map(|upload| Self::render_upload_row(upload, cx))
            .collect();
        let lists: Vec<gpui::AnyElement> = if emojis.is_empty() {
            vec![
                with_type(div(), TYPE_BODY_LG)
                    .text_color(color(tokens.component_settings_body_text))
                    .child(words.empty())
                    .into_any_element(),
            ]
        } else {
            kinds_in_order(&emojis)
                .iter()
                .map(|(is_animated, list)| self.render_kind(*is_animated, list, counts, window, cx))
                .collect()
        };
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_SERVER_SETTINGS_TOOLBAR_GAP))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_SERVER_SETTINGS_TEXT_GAP))
                    .child(
                        with_type(div(), TYPE_BODY_LG)
                            .text_color(color(tokens.component_settings_body_text))
                            .child(words.intro()),
                    )
                    .child(secondary_line(words.rules(), cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(length(COMPONENT_SERVER_SETTINGS_TEXT_GAP))
                    .child(
                        Button::new("emoji-upload", words.upload())
                            .variant(ButtonVariant::Primary)
                            .disabled(is_both_full)
                            .track_focus(&self.upload_focus)
                            .on_click(move |window, cx| {
                                let _ = page.update(cx, |_, cx| Self::choose_files(window, cx));
                            }),
                    )
                    .when(is_both_full, |column| {
                        column.child(secondary_line(words.both_full(), cx))
                    }),
            )
            .when(!uploads.is_empty(), |column| {
                column.child(
                    div()
                        .id("emoji-uploads")
                        .role(Role::List)
                        .aria_label(words.upload())
                        .children(uploads),
                )
            })
            .children(lists)
            .child(status_notice("emoji-notice", self.notice.clone()))
            .children(
                self.deleting
                    .as_ref()
                    .map(|deleting| deleting.dialog.clone()),
            )
    }
}

#[cfg(test)]
#[path = "emoji_tests.rs"]
mod tests;
