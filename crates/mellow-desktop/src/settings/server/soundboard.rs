//! Server settings → Soundboard (`SoundSettings.svelte`, `docs/design/web-soundboard.md`,
//! ADR-0032), beside Emoji and with the same permission. One sound at a time goes through "Add a
//! sound" (trim, volume, name, emoji); each row previews, edits and deletes. The count sits on
//! the list it limits; the list follows the gateway (`GUILD_SOUNDS_UPDATE`), so there is nothing
//! to wait for but the upload.

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, PathPromptOptions, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::server_settings::focus_after_removal;
use mellow_client_core::shared_logic::soundboard::MAXIMUM_SOUNDS_PER_GUILD;
use mellow_protocol::{Snowflake, SoundboardSound};

use super::list_rows::{empty_sentence, list_row, secondary_line, text_column};
use super::sound_form::{OpenSoundDialog, SoundForm, SoundSaved};
use crate::components::{
    Button, ButtonVariant, IconButton, IconButtonTone, ModalDialog, ModalDialogEvent,
    polite_live_region, with_type,
};
use crate::i18n::strings;
use crate::session::Session;
use crate::settings::{SettingsPage, SettingsPageEvent};
use crate::sign_in_form::describe_error;
use crate::soundboard::sound_emoji;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE, COMPONENT_CUSTOM_EMOJI_SETTINGS_PICTURE_RADIUS,
    COMPONENT_CUSTOM_EMOJI_SETTINGS_PICTURE_SIZE, COMPONENT_EMOJI_PICKER_EMOJI_SIZE,
    COMPONENT_SERVER_SETTINGS_TEXT_GAP, COMPONENT_SERVER_SETTINGS_TOOLBAR_GAP, SIZE_HIT_WEB_MIN,
    SPACE_2XS, SPACE_SM, TYPE_BODY_LG, TYPE_CATEGORY,
};
use crate::theme::{color, length, theme};

struct OpenDialog {
    dialog: Entity<ModalDialog>,
    /// The dialog keeps its form alive; tests reach it here.
    #[cfg(test)]
    form: Entity<SoundForm>,
    _subscriptions: Vec<Subscription>,
}

/// A deleted row took focus with it: the next row's Delete takes it, else Upload.
#[derive(Debug, Clone, Copy)]
enum FocusAfterDelete {
    Row(Snowflake),
    Upload,
}

struct DeleteDialog {
    sound_id: Snowflake,
    dialog: Entity<ModalDialog>,
    _subscription: Subscription,
}

pub struct ServerSoundboardPage {
    session: Entity<Session>,
    guild_id: Snowflake,
    notice: SharedString,
    adding_or_editing: Option<OpenDialog>,
    deleting: Option<DeleteDialog>,
    upload_focus: FocusHandle,
    /// A deleted row took focus with it: the next row's Delete takes it once drawn, else Upload.
    focus_after_delete: Option<FocusAfterDelete>,
    _session_changes: Subscription,
}

impl EventEmitter<SettingsPageEvent> for ServerSoundboardPage {}

impl SettingsPage for ServerSoundboardPage {
    fn set_attention(&mut self, _attention: u32, _cx: &mut Context<Self>) {}
}

impl std::fmt::Debug for ServerSoundboardPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerSoundboardPage")
            .field("guild_id", &self.guild_id)
            .finish_non_exhaustive()
    }
}

impl ServerSoundboardPage {
    pub fn new(session: Entity<Session>, guild_id: Snowflake, cx: &mut Context<Self>) -> Self {
        let session_changes = cx.observe(&session, |_, _, cx| cx.notify());
        Self {
            session,
            guild_id,
            notice: SharedString::default(),
            adding_or_editing: None,
            deleting: None,
            upload_focus: cx.focus_handle().tab_stop(true),
            focus_after_delete: None,
            _session_changes: session_changes,
        }
    }

    fn sounds(&self, cx: &gpui::App) -> Vec<SoundboardSound> {
        self.session
            .read(cx)
            .state()
            .sounds_of(self.guild_id)
            .to_vec()
    }

    fn is_full(&self, cx: &gpui::App) -> bool {
        self.sounds(cx).len() >= MAXIMUM_SOUNDS_PER_GUILD
    }

    fn row_focus(sound_id: Snowflake, window: &mut Window, cx: &mut gpui::App) -> FocusHandle {
        crate::components::remembered_focus_handle(&delete_id(sound_id), window, cx)
    }

    // ---- Adding and editing ----

    fn choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_full(cx) {
            return;
        }
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |page, cx| match picked.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = page.update_in(cx, |page, window, cx| {
                        let open = SoundForm::open_add(
                            page.session.clone(),
                            page.guild_id,
                            path,
                            window,
                            cx,
                        );
                        page.show_dialog(open, cx);
                    });
                }
            }
            Ok(Ok(None)) | Err(_) => {}
            Ok(Err(error)) => tracing::warn!(%error, "the file picker failed"),
        })
        .detach();
    }

    fn edit(&mut self, sound: SoundboardSound, window: &mut Window, cx: &mut Context<Self>) {
        let open = SoundForm::open_edit(self.session.clone(), self.guild_id, sound, window, cx);
        self.show_dialog(open, cx);
    }

    fn show_dialog(&mut self, open: OpenSoundDialog, cx: &mut Context<Self>) {
        let OpenSoundDialog { dialog, form } = open;
        let subscriptions = vec![
            cx.subscribe(&form, |page, _, saved: &SoundSaved, cx| {
                page.notice = saved.0.clone().into();
                cx.notify();
            }),
            cx.subscribe(&dialog, |page, _, event: &ModalDialogEvent, cx| {
                if *event == ModalDialogEvent::Closed {
                    page.adding_or_editing = None;
                    cx.notify();
                }
            }),
        ];
        self.adding_or_editing = Some(OpenDialog {
            dialog,
            #[cfg(test)]
            form,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    // ---- Deleting ----

    fn ask_to_delete(
        &mut self,
        sound: &SoundboardSound,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let catalogue = strings(cx);
        let words = catalogue.server_settings().sounds();
        let dialog = cx.new(|cx| {
            ModalDialog::new(
                words.delete_title(&sound.name),
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
            sound_id: sound.id,
            dialog,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(deleting) = self.deleting.as_ref() else {
            return;
        };
        let (sound_id, dialog) = (deleting.sound_id, deleting.dialog.clone());
        if dialog.read(cx).is_busy() {
            return;
        }
        let sounds = self.sounds(cx);
        let Some(name) = sounds
            .iter()
            .find(|sound| sound.id == sound_id)
            .map(|sound| sound.name.clone())
        else {
            self.deleting = None;
            cx.notify();
            return;
        };
        let shown_ids: Vec<Snowflake> = sounds.iter().map(|sound| sound.id).collect();
        dialog.update(cx, |dialog, cx| {
            dialog.set_busy(true, cx);
            dialog.set_error(None, cx);
        });
        let guild_id = self.guild_id;
        let page = cx.entity().downgrade();
        let session_later = self.session.downgrade();
        self.session.update(cx, |session, cx| {
            session.send_settings_request(
                "delete a sound",
                move |rest| async move { rest.delete_sound(guild_id, sound_id).await },
                cx,
                move |answer, cx| {
                    dialog.update(cx, |dialog, cx| dialog.set_busy(false, cx));
                    match answer {
                        Ok(()) => {
                            let _ = session_later.update(cx, |session, cx| {
                                session.apply_sound_answer(
                                    |state| state.remove_deleted_sound(guild_id, sound_id),
                                    cx,
                                );
                            });
                            let _ = page.update(cx, |page, cx| {
                                page.focus_after_delete = Some(
                                    focus_after_removal(&shown_ids, sound_id)
                                        .map_or(FocusAfterDelete::Upload, FocusAfterDelete::Row),
                                );
                                page.deleting = None;
                                page.notice =
                                    strings(cx).server_settings().sounds().deleted(&name).into();
                                cx.notify();
                            });
                        }
                        Err(error) => {
                            let sentence = describe_error(
                                &error,
                                strings(cx).server_settings().sounds().delete_failed(),
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

    fn render_row(
        &self,
        sound: &SoundboardSound,
        join_sound_id: Option<Snowflake>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.server_settings().sounds();
        let soundboard_words = catalogue.soundboard();
        let duration = soundboard_words.duration(i64::from(sound.duration_milliseconds));
        let secondary = if join_sound_id == Some(sound.id) {
            format!(
                "{duration}, {}",
                soundboard_words.join_sound().to_lowercase()
            )
        } else {
            duration
        };
        let picture = sound_emoji(
            sound.emoji.as_deref(),
            COMPONENT_EMOJI_PICKER_EMOJI_SIZE,
            color(tokens.component_server_settings_secondary_text),
            self.session.read(cx).state(),
            window,
            cx,
        );
        let sound_id = sound.id;
        let page = cx.entity().downgrade();
        let edit_page = page.clone();
        let delete_page = page.clone();
        let edited = sound.clone();
        let deleted = sound.clone();
        list_row(cx)
            .id(("sound", sound.id.raw()))
            .role(Role::ListItem)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(length(COMPONENT_CUSTOM_EMOJI_SETTINGS_PICTURE_SIZE))
                    .rounded(length(COMPONENT_CUSTOM_EMOJI_SETTINGS_PICTURE_RADIUS))
                    .bg(color(
                        tokens.component_custom_emoji_settings_picture_background,
                    ))
                    .child(picture),
            )
            .child(
                text_column()
                    .child(
                        with_type(div(), TYPE_BODY_LG)
                            .truncate()
                            .text_color(color(tokens.color_text_heading))
                            .child(sound.name.clone()),
                    )
                    .child(secondary_line(secondary, cx)),
            )
            .child(
                IconButton::new(
                    ("sound-preview", sound.id.raw()),
                    IconName::Headphones,
                    soundboard_words.preview_label(&sound.name),
                )
                .sizes(SIZE_HIT_WEB_MIN, COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE)
                .on_click(move |_, cx| {
                    let _ = page.update(cx, |_, cx| {
                        if let Some(voice) = crate::voice::voice(cx) {
                            voice.update(cx, |voice, cx| {
                                voice.preview_soundboard_sound(sound_id, cx);
                            });
                        }
                    });
                }),
            )
            .child(
                IconButton::new(
                    ("sound-edit", sound.id.raw()),
                    IconName::PencilSimple,
                    words.edit_label(&sound.name),
                )
                .sizes(SIZE_HIT_WEB_MIN, COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE)
                .on_click(move |window, cx| {
                    let _ = edit_page.update(cx, |page, cx| page.edit(edited.clone(), window, cx));
                }),
            )
            .child(
                IconButton::new(
                    delete_id(sound.id),
                    IconName::Trash,
                    words.delete_label(&sound.name),
                )
                .sizes(SIZE_HIT_WEB_MIN, COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE)
                .tone(IconButtonTone::Danger)
                .track_focus(&Self::row_focus(sound.id, window, cx))
                .on_click(move |window, cx| {
                    let _ =
                        delete_page.update(cx, |page, cx| page.ask_to_delete(&deleted, window, cx));
                }),
            )
            .into_any_element()
    }
}

fn delete_id(sound_id: Snowflake) -> gpui::ElementId {
    gpui::ElementId::from(("sound-delete", sound_id.raw()))
}

impl Render for ServerSoundboardPage {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(next) = self.focus_after_delete.take() {
            let focus = match next {
                FocusAfterDelete::Row(sound_id) => Self::row_focus(sound_id, window, cx),
                FocusAfterDelete::Upload => self.upload_focus.clone(),
            };
            window.focus(&focus, cx);
        }
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.server_settings().sounds();
        let sounds = self.sounds(cx);
        let is_full = sounds.len() >= MAXIMUM_SOUNDS_PER_GUILD;
        let join_sound_id = self
            .session
            .read(cx)
            .state()
            .guild(self.guild_id)
            .and_then(|guild| guild.my_join_sound_id);
        let used = i64::try_from(sounds.len()).unwrap_or(i64::MAX);
        let maximum = i64::try_from(MAXIMUM_SOUNDS_PER_GUILD).unwrap_or(i64::MAX);
        let upload_page = cx.entity().downgrade();
        let list = if sounds.is_empty() {
            empty_sentence(words.empty(), cx).into_any_element()
        } else {
            let count = if is_full {
                format!("{} · {}", words.count(used, maximum), words.full())
            } else {
                words.count(used, maximum)
            };
            let rows: Vec<gpui::AnyElement> = sounds
                .iter()
                .map(|sound| self.render_row(sound, join_sound_id, window, cx))
                .collect();
            div()
                .child(
                    with_type(div(), TYPE_CATEGORY)
                        .id("sounds-heading")
                        .role(Role::Heading)
                        .aria_label(format!(
                            "{}, {}",
                            words.list_label(),
                            words.count_label(used, maximum)
                        ))
                        .flex()
                        .items_baseline()
                        .justify_between()
                        .gap(length(SPACE_SM))
                        .pb(length(SPACE_2XS))
                        .border_b_1()
                        .border_color(color(tokens.component_server_settings_row_divider))
                        .text_color(color(tokens.component_settings_nav_section_text))
                        .child(words.list_label())
                        .child(
                            div()
                                .when(is_full, |count| {
                                    count.text_color(color(
                                        tokens.component_custom_emoji_settings_count_full_text,
                                    ))
                                })
                                .child(count),
                        ),
                )
                .child(
                    div()
                        .id("sounds-list")
                        .role(Role::List)
                        .aria_label(words.list_label())
                        .children(rows),
                )
                .into_any_element()
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
                        Button::new("sound-upload", words.upload())
                            .variant(ButtonVariant::Primary)
                            .accessible_label(format!(
                                "{}, {}",
                                words.upload(),
                                if is_full {
                                    words.full_sentence()
                                } else {
                                    words.rules()
                                }
                            ))
                            .disabled(is_full)
                            .track_focus(&self.upload_focus)
                            .on_click(move |window, cx| {
                                let _ =
                                    upload_page.update(cx, |page, cx| page.choose_file(window, cx));
                            }),
                    )
                    .when(is_full, |column| {
                        column.child(secondary_line(words.full_sentence(), cx))
                    }),
            )
            .child(list)
            .child(polite_live_region("sounds-notice", self.notice.clone()))
            .children(
                self.adding_or_editing
                    .as_ref()
                    .map(|open| open.dialog.clone()),
            )
            .children(
                self.deleting
                    .as_ref()
                    .map(|deleting| deleting.dialog.clone()),
            )
    }
}

#[cfg(test)]
#[path = "soundboard_tests.rs"]
mod tests;
