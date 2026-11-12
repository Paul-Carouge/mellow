//! A role's icon (`RoleIconEditor.svelte`, ADR-0050, DESIGN.md §5.8), on the Roles page's Display
//! tab: a picture (the instance keeps its centred square) or a Unicode emoji, one replacing the
//! other. Saved at once, as on Discord; the new icon comes back with the gateway's role list.

use std::path::{Path, PathBuf};

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, ObjectFit, ParentElement as _, PathPromptOptions, Pixels, Point, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, StyledImage as _, Subscription,
    Window, div, img, prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::custom_emoji::display_emoji;
use mellow_client_core::shared_logic::emoji_picker::PickerEmoji;
use mellow_client_core::shared_logic::role_icons::has_role_icon;
use mellow_client_core::shared_logic::server_settings::{
    MAXIMUM_ROLE_ICON_UPLOAD_BYTES, can_manage_role, role_manager_of,
};
use mellow_client_core::state::reaction_key;
use mellow_client_core::{ClientError, DownloadableFile};
use mellow_protocol::{ApiErrorCode, IconCropQuery, Role as GuildRole, Snowflake};

use crate::components::{Button, ButtonVariant, bounds_recorder, icon, with_type};
use crate::emoji_picker::{EmojiPicker, EmojiPickerEvent, note_emoji_used, recent_emoji};
use crate::i18n::strings;
use crate::messages::{picture_animation, picture_for_box};
use crate::session::Session;
use crate::settings::{error_alert, saved_notice};
use crate::sign_in_form::describe_error;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_ROLE_ICON_EDITOR_GAP, COMPONENT_ROLE_ICON_EDITOR_PREVIEW_SIZE,
    COMPONENT_ROLE_ICON_EDITOR_TILE_RADIUS, COMPONENT_ROLE_ICON_EDITOR_TILE_SIZE,
    COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP, COMPONENT_SETTINGS_SECTION_LABEL_GAP,
    COMPONENT_TEXT_FIELD_BORDER_WIDTH, COMPONENT_TEXT_FIELD_ICON_SIZE, TYPE_BODY, TYPE_BODY_SM,
    TYPE_CATEGORY,
};
use crate::theme::{color, length, theme};

/// The picture types the instance takes (`PICTURE_UPLOAD_TYPES`), by file extension.
const PICTURE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];
const BYTES_PER_KILOBYTE: u64 = 1024;

/// Refused before reading: a known extension that is not a picture. A file without one goes to
/// the instance, which reads its bytes (the web lets a file of unknown type through too).
fn is_unsupported_picture(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            !PICTURE_EXTENSIONS
                .iter()
                .any(|picture| extension.eq_ignore_ascii_case(picture))
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IconChange {
    Picture,
    Emoji,
    Remove,
}

pub struct RoleIconEditor {
    session: Entity<Session>,
    guild_id: Snowflake,
    role_id: Snowflake,
    busy: Option<IconChange>,
    error_message: Option<SharedString>,
    notice: Option<SharedString>,
    emoji_button_focus: FocusHandle,
    emoji_button_bounds: Entity<Option<Bounds<Pixels>>>,
    emoji_picker: Option<(Entity<EmojiPicker>, Subscription)>,
    /// Where a click outside closed the picker: that same click on the button must not reopen it
    /// (`createPopoverToggle`).
    picker_dismissed_at: Option<Point<Pixels>>,
}

impl std::fmt::Debug for RoleIconEditor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RoleIconEditor")
            .field("role_id", &self.role_id)
            .field("busy", &self.busy)
            .finish_non_exhaustive()
    }
}

impl RoleIconEditor {
    pub fn new(
        session: Entity<Session>,
        guild_id: Snowflake,
        role_id: Snowflake,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self {
            session,
            guild_id,
            role_id,
            busy: None,
            error_message: None,
            notice: None,
            emoji_button_focus: cx.focus_handle().tab_stop(true),
            emoji_button_bounds: cx.new(|_| None),
            emoji_picker: None,
            picker_dismissed_at: None,
        }
    }

    /// The role and whether the reader may change it, as stored now.
    fn role_and_editability(&self, cx: &App) -> Option<(GuildRole, bool)> {
        let state = self.session.read(cx).state();
        let guild = state.guild(self.guild_id)?;
        let role = guild.roles.iter().find(|role| role.id == self.role_id)?;
        let manager = role_manager_of(guild, state.current_user().map(|user| user.id));
        Some((role.clone(), can_manage_role(&manager, role, guild.id)))
    }

    fn send(&mut self, kind: IconChange, request: IconRequest, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(kind);
        self.error_message = None;
        self.notice = None;
        cx.notify();
        let (guild_id, role_id) = (self.guild_id, self.role_id);
        let editor = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_settings_request(
                "change role icon",
                move |rest| async move {
                    match request {
                        IconRequest::Picture(picture) => {
                            rest.set_role_icon_picture(
                                guild_id,
                                role_id,
                                picture,
                                &IconCropQuery::default(),
                            )
                            .await
                        }
                        IconRequest::Emoji(emoji) => {
                            rest.set_role_icon_emoji(guild_id, role_id, &emoji).await
                        }
                        IconRequest::Remove => rest.remove_role_icon(guild_id, role_id).await,
                    }
                },
                cx,
                move |answer, cx| {
                    let _ = editor.update(cx, |editor, cx| {
                        editor.busy = None;
                        let words = strings(cx).role_icons();
                        match answer {
                            Ok(_) => {
                                editor.notice = Some(
                                    if kind == IconChange::Remove {
                                        words.removed()
                                    } else {
                                        words.saved()
                                    }
                                    .into(),
                                );
                            }
                            Err(error) => {
                                editor.error_message = Some(describe_icon_error(&error, kind, cx));
                            }
                        }
                        cx.notify();
                    });
                },
            );
        });
    }

    fn choose_picture(window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |editor, cx| match picked.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    editor.update(cx, |editor, cx| editor.upload(path, cx)).ok();
                }
            }
            Ok(Ok(None)) | Err(_) => {}
            Ok(Err(error)) => tracing::warn!(%error, "the file picker failed"),
        })
        .detach();
    }

    /// Reads the picture off the UI thread (never more than the instance takes), then sends it.
    fn upload(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let words = strings(cx);
        let change_failed = words.role_icons().change_failed();
        self.notice = None;
        if is_unsupported_picture(&path) {
            self.error_message = Some(
                words
                    .errors()
                    .with_reason(change_failed, words.profile().not_a_picture())
                    .into(),
            );
            cx.notify();
            return;
        }
        self.busy = Some(IconChange::Picture);
        self.error_message = None;
        cx.notify();
        let read = cx.background_spawn(async move { read_picture(&path) });
        cx.spawn(async move |editor, cx| {
            let picture = read.await;
            editor
                .update(cx, |editor, cx| {
                    editor.busy = None;
                    match picture {
                        Ok(picture) => {
                            editor.send(IconChange::Picture, IconRequest::Picture(picture), cx);
                        }
                        Err(failure) => {
                            editor.error_message =
                                Some(describe_picture_read_failure(&failure, cx));
                            cx.notify();
                        }
                    }
                })
                .ok();
        })
        .detach();
    }

    fn toggle_emoji_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let is_same_click_as_dismissal = self
            .picker_dismissed_at
            .take()
            .is_some_and(|position| position == window.mouse_position());
        if is_same_click_as_dismissal {
            return;
        }
        if self.emoji_picker.is_some() {
            self.close_emoji_picker(true, window, cx);
            return;
        }
        let Some(anchor) = *self.emoji_button_bounds.read(cx) else {
            return;
        };
        let reader_id = self
            .session
            .read(cx)
            .state()
            .current_user()
            .map(|user| user.id);
        let recent = recent_emoji(reader_id, cx);
        // A role icon is a Unicode emoji: the server's own emoji are not offered.
        let picker = cx.new(|cx| EmojiPicker::open(anchor, recent, None, window, cx));
        let subscription = cx.subscribe_in(
            &picker,
            window,
            move |editor, _, event: &EmojiPickerEvent, window, cx| match event {
                EmojiPickerEvent::Picked(PickerEmoji::Unicode(emoji)) => {
                    note_emoji_used(reader_id, emoji, cx);
                    let key = reaction_key(emoji);
                    editor.close_emoji_picker(true, window, cx);
                    editor.send(IconChange::Emoji, IconRequest::Emoji(key), cx);
                }
                EmojiPickerEvent::Picked(PickerEmoji::Custom(_)) => {
                    editor.close_emoji_picker(true, window, cx);
                }
                EmojiPickerEvent::Dismissed {
                    should_return_focus,
                } => {
                    let position = window.mouse_position();
                    let is_on_button = editor
                        .emoji_button_bounds
                        .read(cx)
                        .is_some_and(|bounds| bounds.contains(&position));
                    editor.picker_dismissed_at =
                        (!should_return_focus && is_on_button).then_some(position);
                    editor.close_emoji_picker(*should_return_focus, window, cx);
                }
            },
        );
        self.emoji_picker = Some((picker, subscription));
        cx.notify();
    }

    fn close_emoji_picker(
        &mut self,
        should_return_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.emoji_picker.take().is_some() && should_return_focus {
            window.focus(&self.emoji_button_focus, cx);
        }
        cx.notify();
    }
}

enum IconRequest {
    Picture(Vec<u8>),
    Emoji(String),
    Remove,
}

/// Why a local picture never reached the instance.
enum PictureReadFailure {
    Unreadable,
    TooLarge,
}

/// The picture's bytes, never more than the instance takes.
fn read_picture(path: &Path) -> Result<Vec<u8>, PictureReadFailure> {
    use std::io::Read as _;

    let unreadable = |error: std::io::Error| {
        tracing::warn!(%error, "a role icon picture could not be read");
        PictureReadFailure::Unreadable
    };
    let file = std::fs::File::open(path).map_err(unreadable)?;
    let mut picture = Vec::new();
    // One byte past the limit tells a file too large from one exactly at it.
    file.take(MAXIMUM_ROLE_ICON_UPLOAD_BYTES + 1)
        .read_to_end(&mut picture)
        .map_err(unreadable)?;
    if picture.len() as u64 > MAXIMUM_ROLE_ICON_UPLOAD_BYTES {
        return Err(PictureReadFailure::TooLarge);
    }
    Ok(picture)
}

/// The web's wording of a picture that could not be read or is too large.
fn describe_picture_read_failure(failure: &PictureReadFailure, cx: &App) -> SharedString {
    let words = strings(cx);
    let reason = match failure {
        PictureReadFailure::Unreadable => words.role_icons().unreadable().to_owned(),
        PictureReadFailure::TooLarge => words.role_icons().too_large(maximum_kilobytes()),
    };
    words
        .errors()
        .with_reason(words.role_icons().change_failed(), &reason)
        .into()
}

/// The web's wording: too large and unreadable pictures say so, other failures their reason.
fn describe_icon_error(error: &ClientError, kind: IconChange, cx: &App) -> SharedString {
    let words = strings(cx);
    let summary = if kind == IconChange::Remove {
        words.role_icons().remove_failed()
    } else {
        words.role_icons().change_failed()
    };
    let maximum_kilobytes = maximum_kilobytes();
    match error {
        ClientError::FileTooLarge => words
            .errors()
            .with_reason(summary, &words.role_icons().too_large(maximum_kilobytes)),
        ClientError::Api {
            code: ApiErrorCode::ValidationFailed,
            ..
        } if kind == IconChange::Picture => words
            .errors()
            .with_reason(summary, words.role_icons().unreadable()),
        _ => describe_error(error, summary, crate::strings_pending_i18n::strings(cx)),
    }
    .into()
}

fn maximum_kilobytes() -> i64 {
    i64::try_from(MAXIMUM_ROLE_ICON_UPLOAD_BYTES / BYTES_PER_KILOBYTE).unwrap_or(i64::MAX)
}

/// The role's picture, or its emoji, at `size`; decorative (the tile names it).
fn role_icon_preview(role: &GuildRole, size: f32, window: &Window, cx: &App) -> gpui::AnyElement {
    if let Some(picture) = role.icon {
        return img(picture_for_box(
            DownloadableFile::Icon(picture.id),
            size,
            size,
            picture_animation(cx),
            window,
        ))
        .flex_none()
        .size(length(size))
        .object_fit(ObjectFit::Contain)
        .into_any_element();
    }
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(length(size))
        .text_size(length(size))
        .line_height(length(size))
        .child(display_emoji(
            role.unicode_emoji.as_deref().unwrap_or_default(),
        ))
        .into_any_element()
}

impl Render for RoleIconEditor {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some((role, is_editable)) = self.role_and_editability(cx) else {
            return div().into_any_element();
        };
        let tokens = theme(cx);
        let words = strings(cx).role_icons();
        let has_icon = has_role_icon(&role);
        let is_idle = self.busy.is_none();
        let editor = cx.entity().downgrade();
        let tile_label: SharedString = if has_icon {
            words.current_icon(&role.name).into()
        } else {
            words.none().into()
        };
        let tile = div()
            .id("role-icon-tile")
            .role(Role::Image)
            .aria_label(tile_label)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(length(COMPONENT_ROLE_ICON_EDITOR_TILE_SIZE))
            .rounded(length(COMPONENT_ROLE_ICON_EDITOR_TILE_RADIUS))
            .bg(color(tokens.component_role_icon_editor_tile_background))
            .border(length(COMPONENT_TEXT_FIELD_BORDER_WIDTH))
            .map(|tile| {
                if has_icon {
                    tile.border_color(color(tokens.component_role_icon_editor_tile_border))
                        .child(role_icon_preview(
                            &role,
                            COMPONENT_ROLE_ICON_EDITOR_PREVIEW_SIZE,
                            window,
                            cx,
                        ))
                } else {
                    tile.border_dashed()
                        .border_color(color(tokens.component_role_icon_editor_empty_icon))
                        .child(icon(
                            IconName::Plus,
                            COMPONENT_TEXT_FIELD_ICON_SIZE,
                            color(tokens.component_role_icon_editor_empty_icon),
                        ))
                }
            });
        let upload_label = if self.busy == Some(IconChange::Picture) {
            words.uploading()
        } else {
            words.upload_picture()
        };
        let emoji_label = if self.busy == Some(IconChange::Emoji) {
            words.uploading()
        } else {
            words.choose_emoji()
        };
        let formats = words.formats(maximum_kilobytes());
        let upload_editor = editor.clone();
        let emoji_editor = editor.clone();
        let mut buttons = div()
            .flex()
            .flex_wrap()
            .gap(length(COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP))
            .child(
                Button::new("role-icon-upload", upload_label)
                    .accessible_label(format!("{upload_label}, {formats}"))
                    .disabled(!is_editable || !is_idle)
                    .on_click(move |window, cx| {
                        let _ = upload_editor.update(cx, |_, cx| Self::choose_picture(window, cx));
                    }),
            )
            .child(
                div()
                    .relative()
                    .child(
                        Button::new("role-icon-emoji", emoji_label)
                            .track_focus(&self.emoji_button_focus)
                            .disabled(!is_editable || !is_idle)
                            .on_click(move |window, cx| {
                                let _ = emoji_editor.update(cx, |editor, cx| {
                                    editor.toggle_emoji_picker(window, cx);
                                });
                            }),
                    )
                    .child(bounds_recorder(self.emoji_button_bounds.clone())),
            );
        if has_icon {
            let remove_label = if self.busy == Some(IconChange::Remove) {
                words.removing()
            } else {
                words.remove()
            };
            buttons = buttons.child(
                Button::new("role-icon-remove", remove_label)
                    .variant(ButtonVariant::Ghost)
                    .disabled(!is_editable || !is_idle)
                    .on_click(move |_, cx| {
                        let _ = editor.update(cx, |editor, cx| {
                            editor.send(IconChange::Remove, IconRequest::Remove, cx);
                        });
                    }),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
            .child(
                with_type(div(), TYPE_CATEGORY)
                    .text_color(color(tokens.component_settings_section_label_text))
                    .child(words.title()),
            )
            .child(
                with_type(div(), TYPE_BODY)
                    .text_color(color(tokens.component_roles_permission_description_text))
                    .child(words.intro()),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(length(COMPONENT_ROLE_ICON_EDITOR_GAP))
                    .child(tile)
                    .child(buttons),
            )
            .child(
                with_type(div(), TYPE_BODY_SM)
                    .text_color(color(tokens.component_roles_note_text))
                    .child(formats.clone()),
            )
            .child(error_alert(
                "role-icon-error",
                self.error_message.clone(),
                cx,
            ))
            .child(saved_notice("role-icon-notice", self.notice.clone(), cx))
            .when_some(self.emoji_picker.as_ref(), |column, (picker, _)| {
                column.child(picker.clone())
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_known_extension_that_is_not_a_picture_is_refused_early() {
        assert!(!is_unsupported_picture(Path::new("icon.PNG")));
        assert!(!is_unsupported_picture(Path::new("icon.webp")));
        assert!(!is_unsupported_picture(Path::new("icon")));
        assert!(is_unsupported_picture(Path::new("notes.txt")));
    }
}
