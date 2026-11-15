//! A server's or channel's icon on its Overview page (`IconEditor.svelte`, DESIGN.md §6.54): the
//! current icon, "Upload icon" opening the crop dialog (`picture_crop_dialog.rs`) once a picture
//! is chosen, and "Remove icon". Saved at once, apart from the page's unsaved-changes bar, as
//! avatars are. The instance crops and compresses; the gateway brings the new icon to everyone,
//! this page included.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    ObjectFit, ParentElement as _, PathPromptOptions, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Subscription, Window, div, img,
    prelude::FluentBuilder as _,
};
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::format::server_initials;
use mellow_client_core::shared_logic::user_settings::MAXIMUM_PROFILE_IMAGE_BYTES;
use mellow_protocol::{IconCropQuery, ProfileImage, Snowflake};

use super::picture_crop::icon_crop_query;
use super::picture_crop_dialog::{
    ChosenPicture, CropKind, CropStage, open_crop_dialog, prepare_picture,
};
use super::picture_upload::{describe_image_failure, describe_read_problem, read_picture};
use super::{error_alert, saved_notice};
use crate::components::{Button, ButtonVariant, ModalDialog, ModalDialogEvent, icon, with_type};
use crate::i18n::strings;
use crate::messages::{file_size_label, picture_animation, picture_for_box};
use crate::pictures::Animation;
use crate::session::Session;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_DIALOG_WIDTH, COMPONENT_ICON_PICKER_PREVIEW_RADIUS_CHANNEL,
    COMPONENT_ICON_PICKER_PREVIEW_SIZE, COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP,
    COMPONENT_SETTINGS_ACTION_GAP, COMPONENT_SETTINGS_SECTION_LABEL_GAP, TYPE_BODY, TYPE_CATEGORY,
    TYPE_HEADING_LG,
};
use crate::theme::{color, length, theme};

/// Whose icon the editor changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconTarget {
    Server(Snowflake),
    Channel(Snowflake),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IconBusy {
    /// The file picker is open, then the chosen file is read and measured, before the dialog
    /// opens: a second click must not open another picker.
    Opening,
    Upload,
    Remove,
}

pub struct IconEditor {
    session: Entity<Session>,
    target: IconTarget,
    busy: Option<IconBusy>,
    error_message: Option<SharedString>,
    notice: Option<SharedString>,
    upload_focus: FocusHandle,
    crop_dialog: Option<(Entity<ModalDialog>, Entity<CropStage>, [Subscription; 2])>,
    _session_observation: Subscription,
}

impl std::fmt::Debug for IconEditor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IconEditor")
            .field("target", &self.target)
            .field("busy", &self.busy)
            .finish_non_exhaustive()
    }
}

/// The stored icon and the name the editor shows (initials, samples).
fn icon_and_name(
    session: &Entity<Session>,
    target: IconTarget,
    cx: &App,
) -> (Option<ProfileImage>, String) {
    let state = session.read(cx).state();
    match target {
        IconTarget::Server(guild_id) => state
            .guild(guild_id)
            .map(|guild| (guild.icon, guild.name.clone()))
            .unwrap_or_default(),
        IconTarget::Channel(channel_id) => state
            .guild_channel(channel_id)
            .map(|channel| (channel.icon, channel.name.clone()))
            .unwrap_or_default(),
    }
}

impl IconEditor {
    pub fn new(session: Entity<Session>, target: IconTarget, cx: &mut Context<Self>) -> Self {
        let observation = cx.observe(&session, |_, _, cx| cx.notify());
        Self {
            session,
            target,
            busy: None,
            error_message: None,
            notice: None,
            upload_focus: cx.focus_handle().tab_stop(true),
            crop_dialog: None,
            _session_observation: observation,
        }
    }

    fn change_failed(cx: &App) -> &'static str {
        strings(cx).icon_editor().change_failed()
    }

    fn choose_picture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() || self.crop_dialog.is_some() {
            return;
        }
        self.busy = Some(IconBusy::Opening);
        self.error_message = None;
        self.notice = None;
        cx.notify();
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        let window_handle = window.window_handle();
        cx.spawn(async move |editor, cx| {
            let path = match picked.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) | Err(_) => None,
                Ok(Err(error)) => {
                    tracing::warn!(%error, "the file picker failed");
                    None
                }
            };
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = editor.update(cx, |editor, cx| {
                    if let Some(path) = path {
                        editor.open_picked_file(path, window, cx);
                    } else {
                        editor.busy = None;
                        cx.notify();
                    }
                });
            });
        })
        .detach();
    }

    /// Reads and measures the picked file off the UI thread, then opens the crop dialog on it.
    fn open_picked_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        // `choose_picture` set `Opening`; anything else means the editor moved on.
        if self.busy != Some(IconBusy::Opening) || self.crop_dialog.is_some() {
            return;
        }
        let window_handle = window.window_handle();
        cx.spawn(async move |editor, cx| {
            let prepared = cx
                .background_spawn(async move { read_picture(&path).and_then(prepare_picture) })
                .await;
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = editor.update(cx, |editor, cx| {
                    editor.busy = None;
                    match prepared {
                        Ok(picture) => editor.open_crop_dialog(picture, window, cx),
                        Err(problem) => {
                            editor.error_message = Some(
                                describe_read_problem(&problem, Self::change_failed(cx), cx).into(),
                            );
                        }
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn open_crop_dialog(
        &mut self,
        picture: ChosenPicture,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let kind = match self.target {
            IconTarget::Server(_) => CropKind::ServerIcon,
            IconTarget::Channel(_) => CropKind::ChannelIcon {
                name: icon_and_name(&self.session, self.target, cx).1,
            },
        };
        let bytes = Arc::clone(&picture.bytes);
        let (dialog, stage, enter) = open_crop_dialog(picture, kind, window, cx);
        let subscribed_stage = stage.clone();
        let subscription = cx.subscribe_in(
            &dialog,
            window,
            move |editor, dialog, event: &ModalDialogEvent, window, cx| match event {
                ModalDialogEvent::Closed => {
                    editor.crop_dialog = None;
                    cx.notify();
                }
                ModalDialogEvent::Submitted => {
                    let crop = icon_crop_query(subscribed_stage.read(cx).crop_to_send());
                    editor.upload(bytes.as_ref().clone(), crop, dialog, window, cx);
                }
            },
        );
        self.crop_dialog = Some((dialog, stage, [subscription, enter]));
    }

    fn upload(
        &mut self,
        bytes: Vec<u8>,
        crop: IconCropQuery,
        dialog: &Entity<ModalDialog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(IconBusy::Upload);
        self.error_message = None;
        self.notice = None;
        dialog.update(cx, |dialog, cx| {
            dialog.set_error(None, cx);
            dialog.set_busy(true, cx);
        });
        cx.notify();
        let editor = cx.entity().downgrade();
        let dialog = dialog.downgrade();
        let window_handle = window.window_handle();
        let target = self.target;
        self.session.update(cx, |session, cx| {
            session.send_settings_request(
                "change icon",
                move |rest| async move {
                    match target {
                        IconTarget::Server(guild_id) => {
                            rest.set_guild_icon(guild_id, bytes, &crop).await.map(drop)
                        }
                        IconTarget::Channel(channel_id) => rest
                            .set_channel_icon(channel_id, bytes, &crop)
                            .await
                            .map(drop),
                    }
                },
                cx,
                move |answer, cx| {
                    let _ = window_handle.update(cx, |_, window, cx| {
                        let failure = answer.as_ref().err().map(|error| {
                            SharedString::from(describe_image_failure(
                                error,
                                Self::change_failed(cx),
                                cx,
                            ))
                        });
                        let _ = dialog.update(cx, |dialog, cx| {
                            dialog.set_busy(false, cx);
                            match &failure {
                                // The dialog stays open with its square, so trying again needs
                                // no new pick.
                                Some(message) => dialog.set_error(Some(message.clone()), cx),
                                None => dialog.request_close(window, cx),
                            }
                        });
                        let _ = editor.update(cx, |editor, cx| {
                            editor.busy = None;
                            if failure.is_none() {
                                editor.notice = Some(strings(cx).icon_editor().saved().into());
                            }
                            cx.notify();
                        });
                    });
                },
            );
        });
    }

    fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(IconBusy::Remove);
        self.error_message = None;
        self.notice = None;
        cx.notify();
        let editor = cx.entity().downgrade();
        let window_handle = window.window_handle();
        let target = self.target;
        let failure = strings(cx).icon_editor().remove_failed().to_owned();
        self.session.update(cx, |session, cx| {
            session.send_settings_change(
                "remove icon",
                failure,
                move |rest| async move {
                    match target {
                        IconTarget::Server(guild_id) => {
                            rest.remove_guild_icon(guild_id).await.map(drop)
                        }
                        IconTarget::Channel(channel_id) => {
                            rest.remove_channel_icon(channel_id).await.map(drop)
                        }
                    }
                },
                cx,
                move |outcome, cx| {
                    let _ = window_handle.update(cx, |_, window, cx| {
                        let _ = editor.update(cx, |editor, cx| {
                            editor.busy = None;
                            match outcome {
                                Ok(()) => {
                                    editor.notice =
                                        Some(strings(cx).icon_editor().removed().into());
                                    // "Remove icon" goes with the icon: focus stays in the editor.
                                    window.focus(&editor.upload_focus, cx);
                                }
                                Err(message) => editor.error_message = Some(message.into()),
                            }
                            cx.notify();
                        });
                    });
                },
            );
        });
    }

    fn render_preview(
        &self,
        icon_image: Option<ProfileImage>,
        name: &str,
        window: &Window,
        cx: &App,
    ) -> gpui::Div {
        let tokens = theme(cx);
        let is_server = matches!(self.target, IconTarget::Server(_));
        let preview = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .size(length(COMPONENT_ICON_PICKER_PREVIEW_SIZE))
            .bg(color(tokens.component_icon_picker_preview_background))
            .text_color(color(tokens.component_icon_picker_preview_text))
            .map(|preview| {
                if is_server {
                    preview.rounded_full()
                } else {
                    preview.rounded(length(COMPONENT_ICON_PICKER_PREVIEW_RADIUS_CHANNEL))
                }
            });
        match icon_image {
            Some(image) => {
                // The preview plays an animated server icon unless reduced motion keeps
                // pictures still; channel icons are always stored still.
                let animation = if is_server {
                    picture_animation(cx)
                } else {
                    Animation::StillFrame
                };
                preview.child(
                    img(picture_for_box(
                        DownloadableFile::Icon(image.id),
                        COMPONENT_ICON_PICKER_PREVIEW_SIZE,
                        COMPONENT_ICON_PICKER_PREVIEW_SIZE,
                        animation,
                        window,
                    ))
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    // GPUI clips children to rectangles: the picture rounds its own corners.
                    .map(|picture| {
                        if is_server {
                            picture.rounded_full()
                        } else {
                            picture.rounded(length(COMPONENT_ICON_PICKER_PREVIEW_RADIUS_CHANNEL))
                        }
                    }),
                )
            }
            None if is_server => {
                preview.child(with_type(div(), TYPE_HEADING_LG).child(server_initials(name)))
            }
            None => preview.child(icon(
                IconName::Hash,
                COMPONENT_ICON_PICKER_PREVIEW_SIZE / 2.0,
                color(tokens.component_icon_picker_preview_text),
            )),
        }
    }
}

impl Render for IconEditor {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let editor_words = catalogue.icon_editor();
        let (icon_image, name) = icon_and_name(&self.session, self.target, cx);
        let has_icon = icon_image.is_some();
        let (title, empty_text, note) = match self.target {
            IconTarget::Server(_) => {
                let words = editor_words.server();
                (words.title(), words.empty_text(), words.note())
            }
            IconTarget::Channel(_) => {
                let words = editor_words.channel();
                (words.title(), words.empty_text(), words.note())
            }
        };
        let formats =
            editor_words.formats(&file_size_label(MAXIMUM_PROFILE_IMAGE_BYTES, catalogue));
        let description = format!("{} {formats}", if has_icon { note } else { empty_text });
        let is_busy = self.busy.is_some();
        let upload_label = match self.busy {
            Some(IconBusy::Opening) => editor_words.crop().opening(),
            Some(IconBusy::Upload) => editor_words.uploading(),
            _ => editor_words.upload(),
        };
        let remove_label = if self.busy == Some(IconBusy::Remove) {
            editor_words.removing()
        } else {
            editor_words.remove()
        };
        let editor = cx.entity().downgrade();
        let remove_editor = editor.clone();
        div()
            .id("icon-editor")
            .role(Role::Group)
            .aria_label(title)
            .flex()
            .flex_wrap()
            .items_start()
            .gap(length(COMPONENT_SETTINGS_ACTION_GAP))
            .child(self.render_preview(icon_image, &name, window, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .max_w(length(COMPONENT_DIALOG_WIDTH))
                    .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                    .child(
                        with_type(div().id("icon-editor-title"), TYPE_CATEGORY)
                            .role(Role::Heading)
                            .text_color(color(tokens.component_settings_section_label_text))
                            .child(title),
                    )
                    .child(
                        with_type(div(), TYPE_BODY)
                            .text_color(color(tokens.component_settings_body_text))
                            .child(description),
                    )
                    .child(
                        div()
                            .mt(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                            .flex()
                            .flex_wrap()
                            .gap(length(COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP))
                            .child(
                                Button::new("icon-editor-upload", upload_label)
                                    .variant(ButtonVariant::Secondary)
                                    .disabled(is_busy)
                                    .track_focus(&self.upload_focus)
                                    .on_click(move |window, cx| {
                                        let _ = editor.update(cx, |editor, cx| {
                                            editor.choose_picture(window, cx);
                                        });
                                    }),
                            )
                            .when(has_icon, |buttons| {
                                buttons.child(
                                    Button::new("icon-editor-remove", remove_label)
                                        .variant(ButtonVariant::Secondary)
                                        .disabled(is_busy)
                                        .on_click(move |window, cx| {
                                            let _ = remove_editor.update(cx, |editor, cx| {
                                                editor.remove(window, cx);
                                            });
                                        }),
                                )
                            }),
                    )
                    // Outside the column's gap, so the empty live regions take no room.
                    .child(
                        div()
                            .when(self.error_message.is_none(), |alert| {
                                alert.mt(-length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                            })
                            .child(error_alert(
                                "icon-editor-error",
                                self.error_message.clone(),
                                cx,
                            ))
                            .child(saved_notice("icon-editor-notice", self.notice.clone(), cx)),
                    ),
            )
            .when_some(self.crop_dialog.as_ref(), |editor, (dialog, _, _)| {
                editor.child(dialog.clone())
            })
    }
}
