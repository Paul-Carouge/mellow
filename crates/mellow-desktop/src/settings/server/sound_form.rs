//! "Add a sound" and "Edit" on Server settings → Soundboard (`AddSoundDialog.svelte`,
//! `EditSoundDialog.svelte`, `SoundDetailsFields.svelte`, ADR-0032): the form inside the
//! `ModalDialog` that owns the footer.
//!
//! Adding: the picked file is checked against the 20 MB limit before it is read, read and decoded
//! off the interface thread (any audio file the attachment decoder reads, at most ten minutes),
//! never sent as it is. The waveform shows all of it, the lit bars are the part kept (at most
//! 5.2 s), moved by a click or a drag on the waveform and set to the hundredth by the Start and
//! End sliders. Volume, name and emoji follow; "Preview" plays exactly what will be uploaded,
//! for the reader only. The instance gets the canonical WAV (mono, 48 kHz, 16-bit).
//!
//! Editing: the name and the emoji, sent together as the instance replaces both. The samples
//! never change: a new cut is a new sound.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, EventEmitter, FocusHandle,
    Focusable as _, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement as _, Pixels, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity, Window, div,
    prelude::FluentBuilder as _, relative,
};
use gpui_component::input::{InputEvent, InputState};
use mellow_client_core::ClientError;
use mellow_client_core::shared_logic::custom_emoji::{custom_emoji_of_reaction_key, display_emoji};
use mellow_client_core::shared_logic::emoji_picker::PickerEmoji;
use mellow_client_core::shared_logic::soundboard::{
    MAXIMUM_SOUND_NAME_LENGTH, MAXIMUM_SOUND_SOURCE_BYTES, SoundNameProblem, SoundSelection,
    SoundUploadRefusal, encode_wav, format_sound_seconds, initial_selection, render_sound,
    selection_sample_count, sound_name_from_file_name, sound_name_problem, sound_upload_refusal,
    waveform_peaks, with_selection_centred_at, with_selection_end, with_selection_start,
};
use mellow_client_core::state::reaction_key;
use mellow_media::{SoundSourceError, decode_sound_source};
use mellow_protocol::{
    CreateSoundQuery, SOUND_SAMPLE_RATE_HERTZ, Snowflake, SoundboardSound, UpdateSoundRequest,
};

use crate::components::{
    Button, ButtonVariant, ModalDialog, ModalDialogEvent, OutsideClickDismissal, RangeBounds,
    RangeField, TextField, bounds_recorder, child_id, with_type,
};
use crate::composer::read_file_within;
use crate::emoji_picker::{
    EmojiPicker, EmojiPickerEvent, EmojiPickerServer, note_emoji_used, recent_emoji,
};
use crate::i18n::{Language, Strings, strings};
use crate::messages::file_size_label;
use crate::session::Session;
use crate::sign_in_form::describe_error;
use crate::soundboard::sound_emoji;
use crate::theme::tokens::{
    COMPONENT_EMOJI_PICKER_EMOJI_SIZE, COMPONENT_TEXT_FIELD_BORDER_WIDTH,
    COMPONENT_TEXT_FIELD_HEIGHT, COMPONENT_TEXT_FIELD_LABEL_GAP,
    COMPONENT_TEXT_FIELD_PADDING_INLINE, COMPONENT_TEXT_FIELD_RADIUS, SIZE_HIT_WEB_MIN, SPACE_MD,
    SPACE_XS, TYPE_BODY, TYPE_BODY_SM, TYPE_CATEGORY,
};
use crate::theme::{color, length, theme};

/// Bars across the waveform: enough to see a word's shape, few enough to draw cheaply.
const WAVEFORM_BARS: usize = 96;
/// Shares of the waveform's height: the loudest bar, and the quietest still drawn.
const LOUDEST_BAR_SHARE: f32 = 0.92;
const QUIETEST_BAR_SHARE: f32 = 0.02;
/// Each bar takes this share of its slot, the rest is the gap between bars.
const BAR_WIDTH_SHARE: f32 = 0.7;
/// The sliders move by a hundredth of a second.
const SECONDS_STEP: f32 = 0.01;
const DEFAULT_VOLUME_PERCENT: u32 = 100;
const MAXIMUM_UPLOAD_VOLUME_PERCENT: u32 = 100;

/// What the form tells the page once the dialog has closed on success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SoundSaved(pub String);

/// The decoded file, ready to trim.
struct Trimmer {
    samples: Arc<[f32]>,
    duration_seconds: f64,
    peaks: Vec<f32>,
    selection: SoundSelection,
    volume_percent: u32,
}

enum Reading {
    InProgress,
    Unreadable,
    TooLarge { size_bytes: u64 },
    TooLong,
    Ready(Trimmer),
}

enum FormKind {
    Add { file_name: String, reading: Reading },
    Edit { sound: SoundboardSound },
}

pub(super) struct SoundForm {
    session: Entity<Session>,
    guild_id: Snowflake,
    kind: FormKind,
    pub(super) name_input: Entity<InputState>,
    emoji_key: Option<String>,
    pub(super) name_error: Option<SharedString>,
    dialog: Option<WeakEntity<ModalDialog>>,
    window_handle: AnyWindowHandle,
    emoji_button_focus: FocusHandle,
    emoji_button_bounds: Entity<Option<Bounds<Pixels>>>,
    emoji_picker: Option<(Entity<EmojiPicker>, Subscription)>,
    picker_dismissal: OutsideClickDismissal,
    waveform_bounds: Entity<Option<Bounds<Pixels>>>,
    is_dragging_waveform: bool,
    /// Ends the "Stop" state when the preview has played out.
    preview: Option<Task<()>>,
    subscriptions: Vec<Subscription>,
}

impl EventEmitter<SoundSaved> for SoundForm {}

/// The dialog and its form; the page keeps both while it is open.
pub(super) struct OpenSoundDialog {
    pub dialog: Entity<ModalDialog>,
    pub form: Entity<SoundForm>,
}

impl SoundForm {
    /// "Add a sound" for the file at `path`: read and decoded at once, off the interface thread.
    pub(super) fn open_add<Host: 'static>(
        session: Entity<Session>,
        guild_id: Snowflake,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Host>,
    ) -> OpenSoundDialog {
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = sound_name_from_file_name(&file_name);
        let catalogue = strings(cx);
        let words = catalogue.server_settings().sounds();
        let open = Self::open(
            session,
            guild_id,
            FormKind::Add {
                file_name: file_name.clone(),
                reading: Reading::InProgress,
            },
            &name,
            None,
            ModalDialogTexts {
                title: words.add_title().into(),
                subtitle: Some(file_name.into()),
                submit: None,
            },
            window,
            cx,
        );
        open.form.update(cx, |_, cx| Self::read_source(path, cx));
        open
    }

    /// "Edit" on `sound`.
    pub(super) fn open_edit<Host: 'static>(
        session: Entity<Session>,
        guild_id: Snowflake,
        sound: SoundboardSound,
        window: &mut Window,
        cx: &mut Context<Host>,
    ) -> OpenSoundDialog {
        let catalogue = strings(cx);
        let words = catalogue.server_settings().sounds();
        let name = sound.name.clone();
        let emoji = sound.emoji.clone();
        let title = words.edit_title(&sound.name);
        Self::open(
            session,
            guild_id,
            FormKind::Edit { sound },
            &name,
            emoji,
            ModalDialogTexts {
                title: title.into(),
                subtitle: None,
                submit: Some((words.save().into(), catalogue.common().saving().into())),
            },
            window,
            cx,
        )
    }

    #[allow(clippy::too_many_arguments)] // The two dialogs' differences, in one place.
    fn open<Host: 'static>(
        session: Entity<Session>,
        guild_id: Snowflake,
        kind: FormKind,
        name: &str,
        emoji_key: Option<String>,
        texts: ModalDialogTexts,
        window: &mut Window,
        cx: &mut Context<Host>,
    ) -> OpenSoundDialog {
        let catalogue = strings(cx);
        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(name.to_owned())
                .validate(|text, _| text.chars().count() <= MAXIMUM_SOUND_NAME_LENGTH)
        });
        let window_handle = window.window_handle();
        let form = cx.new(|cx| Self {
            session,
            guild_id,
            kind,
            name_input: name_input.clone(),
            emoji_key,
            name_error: None,
            dialog: None,
            window_handle,
            emoji_button_focus: cx.focus_handle().tab_stop(true),
            emoji_button_bounds: cx.new(|_| None),
            emoji_picker: None,
            picker_dismissal: OutsideClickDismissal::default(),
            waveform_bounds: cx.new(|_| None),
            is_dragging_waveform: false,
            preview: None,
            subscriptions: Vec::new(),
        });
        let dialog = cx.new(|cx| {
            let mut dialog = ModalDialog::new(texts.title, catalogue.common().cancel(), window, cx)
                .body(form.clone().into());
            if let Some(subtitle) = texts.subtitle {
                dialog = dialog.subtitle(subtitle);
            }
            if let Some((label, busy_label)) = texts.submit {
                dialog = dialog.submit(label, busy_label, false);
            }
            dialog
        });
        form.update(cx, |form, cx| {
            form.subscriptions = vec![
                cx.subscribe(&name_input, |form: &mut Self, _, event: &InputEvent, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        form.submit(cx);
                    }
                }),
                cx.subscribe(
                    &dialog,
                    |form: &mut Self, _, event: &ModalDialogEvent, cx| match event {
                        ModalDialogEvent::Submitted => form.submit(cx),
                        ModalDialogEvent::Closed => form.stop_preview(cx),
                    },
                ),
            ];
            form.dialog = Some(dialog.downgrade());
        });
        // Editing starts in the name; adding waits on Cancel while the file is read.
        if matches!(form.read(cx).kind, FormKind::Edit { .. }) {
            let focus = name_input.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        OpenSoundDialog { dialog, form }
    }

    pub(super) fn dialog(&self) -> Option<Entity<ModalDialog>> {
        self.dialog.as_ref().and_then(WeakEntity::upgrade)
    }

    // ---- Reading the file ----

    /// Size first (a file too large is never read), then the bytes and the decoding off the
    /// interface thread.
    fn read_source(path: PathBuf, cx: &mut Context<Self>) {
        let work = cx.background_spawn(async move { read_and_decode(&path) });
        cx.spawn(async move |form, cx| {
            let reading = work.await;
            let _ = form.update(cx, |form, cx| form.finish_reading(reading, cx));
        })
        .detach();
    }

    fn finish_reading(&mut self, reading: Reading, cx: &mut Context<Self>) {
        let is_ready = matches!(reading, Reading::Ready(_));
        if let FormKind::Add { reading: shown, .. } = &mut self.kind {
            *shown = reading;
        }
        if is_ready && let Some(dialog) = self.dialog() {
            let words = strings(cx).server_settings().sounds();
            dialog.update(cx, |dialog, cx| {
                dialog.set_submit_labels(Some((words.submit().into(), words.adding().into())), cx);
            });
            // Focus waited on Cancel while the file was read: the trimmer is the first thing to
            // do, and its sliders are the keyboard's way to it.
            let window = self.window_handle;
            let start = child_id(&"sound-trim".into(), "start");
            let _ = window.update(cx, |_, window, cx| {
                let focus = crate::components::remembered_focus_handle(&start, window, cx);
                window.focus(&focus, cx);
            });
        }
        cx.notify();
    }

    fn trimmer(&self) -> Option<&Trimmer> {
        match &self.kind {
            FormKind::Add {
                reading: Reading::Ready(trimmer),
                ..
            } => Some(trimmer),
            _ => None,
        }
    }

    fn trimmer_mut(&mut self) -> Option<&mut Trimmer> {
        match &mut self.kind {
            FormKind::Add {
                reading: Reading::Ready(trimmer),
                ..
            } => Some(trimmer),
            _ => None,
        }
    }

    fn change_selection(
        &mut self,
        change: impl FnOnce(SoundSelection, f64) -> SoundSelection,
        cx: &mut Context<Self>,
    ) {
        if let Some(trimmer) = self.trimmer_mut() {
            trimmer.selection = change(trimmer.selection, trimmer.duration_seconds);
            cx.notify();
        }
    }

    /// A press or drag on the waveform: the selection centres there.
    fn move_selection_to(&mut self, position_x: Pixels, cx: &mut Context<Self>) {
        let Some(bounds) = *self.waveform_bounds.read(cx) else {
            return;
        };
        let width = f32::from(bounds.size.width);
        if width <= 0.0 {
            return;
        }
        let fraction = f64::from((f32::from(position_x - bounds.origin.x) / width).clamp(0.0, 1.0));
        self.change_selection(
            |selection, duration| {
                with_selection_centred_at(selection, fraction * duration, duration)
            },
            cx,
        );
    }

    // ---- Preview ----

    fn toggle_preview(&mut self, cx: &mut Context<Self>) {
        if self.preview.is_some() {
            self.stop_preview(cx);
            return;
        }
        let Some(trimmer) = self.trimmer() else {
            return;
        };
        let Some(voice) = crate::voice::voice(cx) else {
            return;
        };
        let rendered: Arc<[f32]> =
            render_sound(&trimmer.samples, trimmer.selection, trimmer.volume_percent).into();
        // At most 249 600 samples: exact in `f64`.
        #[allow(clippy::cast_precision_loss)]
        let length = Duration::from_secs_f64(
            selection_sample_count(trimmer.selection) as f64 / f64::from(SOUND_SAMPLE_RATE_HERTZ),
        );
        voice.update(cx, |voice, cx| {
            voice.preview_soundboard_samples(rendered, cx);
        });
        self.preview = Some(cx.spawn(async move |form, cx| {
            cx.background_executor().timer(length).await;
            let _ = form.update(cx, |form, cx| {
                form.preview = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Nothing keeps playing for a dialog that closed or a preview stopped.
    fn stop_preview(&mut self, cx: &mut Context<Self>) {
        if self.preview.take().is_some()
            && let Some(voice) = crate::voice::voice(cx)
        {
            voice.update(cx, |voice, _| voice.stop_soundboard_previews());
        }
        cx.notify();
    }

    // ---- The emoji ----

    fn toggle_emoji_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .picker_dismissal
            .is_closing_click(window.mouse_position(), &())
        {
            return;
        }
        if self.emoji_picker.is_some() {
            self.close_emoji_picker(true, window, cx);
            return;
        }
        let Some(anchor) = *self.emoji_button_bounds.read(cx) else {
            return;
        };
        let state = self.session.read(cx).state();
        let reader_id = state.current_user().map(|user| user.id);
        let server = state.guild(self.guild_id).map(|guild| EmojiPickerServer {
            name: guild.name.clone().into(),
            emojis: guild.emojis.clone(),
        });
        let recent = recent_emoji(reader_id, cx);
        let picker =
            cx.new(|cx| EmojiPicker::open(anchor, recent, server, window, cx).over_dialogs());
        let subscription = cx.subscribe_in(
            &picker,
            window,
            move |form, _, event: &EmojiPickerEvent, window, cx| match event {
                EmojiPickerEvent::Picked(emoji) => {
                    form.emoji_key = Some(match emoji {
                        PickerEmoji::Unicode(unicode) => {
                            note_emoji_used(reader_id, unicode, cx);
                            reaction_key(unicode)
                        }
                        PickerEmoji::Custom(_) => emoji.reaction_emoji(),
                    });
                    form.close_emoji_picker(true, window, cx);
                }
                EmojiPickerEvent::Dismissed {
                    should_return_focus,
                } => {
                    let position = window.mouse_position();
                    let is_on_button = form
                        .emoji_button_bounds
                        .read(cx)
                        .is_some_and(|bounds| bounds.contains(&position));
                    if !should_return_focus && is_on_button {
                        form.picker_dismissal.remember(position, ());
                    } else {
                        form.picker_dismissal.forget();
                    }
                    form.close_emoji_picker(*should_return_focus, window, cx);
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

    fn remove_emoji(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.emoji_key = None;
        window.focus(&self.emoji_button_focus, cx);
        cx.notify();
    }

    // ---- Sending ----

    pub(super) fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.dialog() else {
            return;
        };
        if dialog.read(cx).is_busy() {
            return;
        }
        let name = self.name_input.read(cx).value().trim().to_owned();
        let words = strings(cx).server_settings().sounds();
        self.name_error = sound_name_problem(&name).map(|problem| {
            match problem {
                SoundNameProblem::Length => words.name_length(),
                SoundNameProblem::ControlCharacter => words.name_control(),
            }
            .into()
        });
        if self.name_error.is_some() {
            let focus = self.name_input.read(cx).focus_handle(cx);
            let _ = self
                .window_handle
                .update(cx, |_, window, cx| window.focus(&focus, cx));
            cx.notify();
            return;
        }
        match &self.kind {
            FormKind::Edit { sound } => {
                if name == sound.name && self.emoji_key == sound.emoji {
                    let _ = self.window_handle.update(cx, |_, window, cx| {
                        dialog.update(cx, |dialog, cx| dialog.request_close(window, cx));
                    });
                    return;
                }
                let sound_id = sound.id;
                self.send_edit(sound_id, name, &dialog, cx);
            }
            FormKind::Add { .. } => {
                let Some(trimmer) = self.trimmer() else {
                    return;
                };
                let (samples, selection, volume_percent) = (
                    trimmer.samples.clone(),
                    trimmer.selection,
                    trimmer.volume_percent,
                );
                self.stop_preview(cx);
                self.send_add(name, samples, selection, volume_percent, &dialog, cx);
            }
        }
    }

    fn send_edit(
        &mut self,
        sound_id: Snowflake,
        name: String,
        dialog: &Entity<ModalDialog>,
        cx: &mut Context<Self>,
    ) {
        dialog.update(cx, |dialog, cx| {
            dialog.set_busy(true, cx);
            dialog.set_error(None, cx);
        });
        let request = UpdateSoundRequest {
            name: name.clone(),
            emoji: self.emoji_key.clone(),
        };
        let guild_id = self.guild_id;
        let form = cx.entity().downgrade();
        let session = self.session.downgrade();
        self.session.update(cx, |session_now, cx| {
            session_now.send_settings_request(
                "edit a sound",
                move |rest| async move { rest.update_sound(guild_id, sound_id, &request).await },
                cx,
                move |answer, cx| {
                    if let Ok(updated) = &answer {
                        let updated = updated.clone();
                        let _ = session.update(cx, |session, cx| {
                            session.apply_sound_answer(
                                |state| state.replace_updated_sound(guild_id, updated),
                                cx,
                            );
                        });
                    }
                    let _ = form.update(cx, |form, cx| {
                        let sentence =
                            answer.map(|_| strings(cx).server_settings().sounds().saved(&name));
                        form.finish(
                            sentence.map_err(|error| {
                                describe_error(
                                    &error,
                                    strings(cx).server_settings().sounds().save_failed(),
                                    crate::strings_pending_i18n::strings(cx),
                                )
                            }),
                            cx,
                        );
                    });
                },
            );
        });
    }

    #[allow(clippy::too_many_arguments)] // What goes into the upload.
    fn send_add(
        &mut self,
        name: String,
        samples: Arc<[f32]>,
        selection: SoundSelection,
        volume_percent: u32,
        dialog: &Entity<ModalDialog>,
        cx: &mut Context<Self>,
    ) {
        dialog.update(cx, |dialog, cx| {
            dialog.set_busy(true, cx);
            dialog.set_error(None, cx);
        });
        let query = CreateSoundQuery {
            name,
            emoji: self.emoji_key.clone(),
        };
        let guild_id = self.guild_id;
        let session = self.session.clone();
        let encode = cx.background_spawn(async move {
            encode_wav(&render_sound(&samples, selection, volume_percent))
        });
        cx.spawn(async move |form, cx| {
            // The trimmer never makes an empty or too long sound.
            let Ok(wav) = encode.await else {
                let _ = form.update(cx, |form, cx| {
                    let sentence = strings(cx).server_settings().sounds().refused().to_owned();
                    form.finish(Err(sentence), cx);
                });
                return;
            };
            let session_later = session.downgrade();
            session.update(cx, |session, cx| {
                session.send_settings_request(
                    "add a sound",
                    move |rest| async move { rest.create_sound(guild_id, &query, wav).await },
                    cx,
                    move |answer, cx| {
                        if let Ok(created) = &answer {
                            let created = created.clone();
                            let _ = session_later.update(cx, |session, cx| {
                                session.apply_sound_answer(
                                    |state| state.add_created_sound(guild_id, created),
                                    cx,
                                );
                            });
                        }
                        let _ = form.update(cx, |form, cx| {
                            let sentence = answer
                                .map(|created| {
                                    strings(cx).server_settings().sounds().added(&created.name)
                                })
                                .map_err(|error| upload_failure(&error, cx));
                            form.finish(sentence, cx);
                        });
                    },
                );
            });
        })
        .detach();
    }

    /// Closes on success (the page announces `sentence`); a failure stays in the dialog.
    fn finish(&mut self, outcome: Result<String, String>, cx: &mut Context<Self>) {
        let Some(dialog) = self.dialog() else {
            return;
        };
        dialog.update(cx, |dialog, cx| dialog.set_busy(false, cx));
        match outcome {
            Ok(sentence) => {
                cx.emit(SoundSaved(sentence));
                let _ = self.window_handle.update(cx, |_, window, cx| {
                    dialog.update(cx, |dialog, cx| dialog.request_close(window, cx));
                });
            }
            Err(sentence) => {
                dialog.update(cx, |dialog, cx| dialog.set_error(Some(sentence.into()), cx));
            }
        }
    }

    // ---- Drawing ----

    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render_trimmer(
        &self,
        file_name: &str,
        catalogue: Strings,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let trimmer = self.trimmer()?;
        let tokens = theme(cx);
        let words = catalogue.server_settings().sounds();
        let separator = decimal_separator(catalogue.language());
        let duration = trimmer.duration_seconds;
        let selection = trimmer.selection;
        let kept = words.kept(
            &catalogue
                .soundboard()
                .duration(selection.kept_milliseconds()),
        );
        // The file's own seconds: f32 is precise to well under a millisecond over ten minutes.
        #[allow(clippy::cast_possible_truncation)]
        let seconds_bounds = RangeBounds {
            minimum: 0.0,
            maximum: duration as f32,
            step: SECONDS_STEP,
        };
        let time_text = move |value: f32| -> SharedString {
            format_sound_seconds(f64::from(value), separator).into()
        };
        let bars: Vec<gpui::AnyElement> = trimmer
            .peaks
            .iter()
            .enumerate()
            .map(|(index, peak)| {
                // Bar index and count are small: exact in `f64`.
                #[allow(clippy::cast_precision_loss)]
                let (bar_start, bar_end) = (
                    index as f64 / WAVEFORM_BARS as f64 * duration,
                    (index + 1) as f64 / WAVEFORM_BARS as f64 * duration,
                );
                let is_kept =
                    bar_end > selection.start_seconds && bar_start < selection.end_seconds;
                div()
                    .flex()
                    .flex_1()
                    .h_full()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .w(relative(BAR_WIDTH_SHARE))
                            .h(relative((peak * LOUDEST_BAR_SHARE).max(QUIETEST_BAR_SHARE)))
                            .bg(color(if is_kept {
                                tokens.component_voice_meter_level_heard
                            } else {
                                tokens.component_voice_meter_level_quiet
                            })),
                    )
                    .into_any_element()
            })
            .collect();
        let start_form = cx.entity().downgrade();
        let end_form = start_form.clone();
        let volume_form = start_form.clone();
        let preview_form = start_form.clone();
        let words_preview = if self.preview.is_some() {
            words.stop_preview()
        } else {
            words.preview_selection()
        };
        Some(
            div()
                .id("sound-trim")
                .role(Role::Group)
                .aria_label(words.trim())
                .flex()
                .flex_col()
                .gap(length(SPACE_XS))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .justify_between()
                        .gap(length(SPACE_XS))
                        .child(
                            with_type(div(), TYPE_CATEGORY)
                                .text_color(color(tokens.component_text_field_label_text))
                                .child(words.trim()),
                        )
                        .child(
                            with_type(div(), TYPE_BODY_SM)
                                .text_color(color(tokens.component_slider_value_text))
                                .child(kept.clone()),
                        ),
                )
                // A pointer target only: the sliders below are the keyboard's way to the same
                // selection.
                .child(
                    div()
                        .id("sound-waveform")
                        .role(Role::Image)
                        .aria_label(words.waveform(file_name))
                        .relative()
                        .flex()
                        .w_full()
                        .h(length(2.0 * SIZE_HIT_WEB_MIN))
                        .rounded(length(COMPONENT_TEXT_FIELD_RADIUS))
                        .bg(color(tokens.component_voice_meter_track))
                        .cursor_ew_resize()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|form, event: &MouseDownEvent, _, cx| {
                                form.is_dragging_waveform = true;
                                form.move_selection_to(event.position.x, cx);
                            }),
                        )
                        .on_mouse_move(cx.listener(|form, event: &MouseMoveEvent, _, cx| {
                            if form.is_dragging_waveform && event.dragging() {
                                form.move_selection_to(event.position.x, cx);
                            }
                        }))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|form, _, _, cx| {
                                form.is_dragging_waveform = false;
                                cx.notify();
                            }),
                        )
                        .on_mouse_up_out(
                            MouseButton::Left,
                            cx.listener(|form, _, _, _| form.is_dragging_waveform = false),
                        )
                        .children(bars)
                        .child(bounds_recorder(self.waveform_bounds.clone())),
                )
                .child(
                    with_type(div(), TYPE_BODY)
                        .text_color(color(tokens.component_text_field_helper_text))
                        .child(words.trim_help()),
                )
                .child(
                    div()
                        .flex()
                        .gap(length(SPACE_MD))
                        .child(
                            div().flex_1().min_w_0().child(
                                RangeField::new(
                                    child_id(&"sound-trim".into(), "start"),
                                    words.start(),
                                    seconds_of(selection.start_seconds),
                                    seconds_bounds,
                                    time_text,
                                )
                                .on_change(move |value, _, cx| {
                                    let _ = start_form.update(cx, |form, cx| {
                                        form.change_selection(
                                            |selection, duration| {
                                                with_selection_start(
                                                    selection,
                                                    f64::from(value),
                                                    duration,
                                                )
                                            },
                                            cx,
                                        );
                                    });
                                }),
                            ),
                        )
                        .child(
                            div().flex_1().min_w_0().child(
                                RangeField::new(
                                    child_id(&"sound-trim".into(), "end"),
                                    words.end(),
                                    seconds_of(selection.end_seconds),
                                    seconds_bounds,
                                    time_text,
                                )
                                .on_change(move |value, _, cx| {
                                    let _ = end_form.update(cx, |form, cx| {
                                        form.change_selection(
                                            |selection, duration| {
                                                with_selection_end(
                                                    selection,
                                                    f64::from(value),
                                                    duration,
                                                )
                                            },
                                            cx,
                                        );
                                    });
                                }),
                            ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_end()
                        .gap(length(SPACE_MD))
                        .child(
                            div().flex_1().min_w_0().child(
                                RangeField::new(
                                    child_id(&"sound-trim".into(), "volume"),
                                    words.volume(),
                                    percent_of(trimmer.volume_percent),
                                    RangeBounds {
                                        minimum: 0.0,
                                        maximum: percent_of(MAXIMUM_UPLOAD_VOLUME_PERCENT),
                                        step: 1.0,
                                    },
                                    move |value| {
                                        // Whole percents from 0 to 100.
                                        #[allow(clippy::cast_possible_truncation)]
                                        let percent = value.round() as i64;
                                        catalogue.voice_settings().percent(percent).into()
                                    },
                                )
                                .on_change(move |value, _, cx| {
                                    let _ = volume_form.update(cx, |form, cx| {
                                        if let Some(trimmer) = form.trimmer_mut() {
                                            // Snapped to whole percents within 0 to 100.
                                            #[allow(
                                                clippy::cast_possible_truncation,
                                                clippy::cast_sign_loss
                                            )]
                                            let percent = value.round().max(0.0) as u32;
                                            trimmer.volume_percent =
                                                percent.min(MAXIMUM_UPLOAD_VOLUME_PERCENT);
                                            cx.notify();
                                        }
                                    });
                                }),
                            ),
                        )
                        .child(Button::new("sound-preview", words_preview).on_click(
                            move |_, cx| {
                                let _ = preview_form.update(cx, Self::toggle_preview);
                            },
                        )),
                )
                .child(crate::components::polite_live_region(
                    "sound-kept",
                    kept.into(),
                ))
                .into_any_element(),
        )
    }

    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render_details(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.server_settings().sounds();
        let is_busy = self
            .dialog()
            .is_some_and(|dialog| dialog.read(cx).is_busy());
        let mut name_field = TextField::new("sound-name", words.name(), &self.name_input)
            .helper(words.name_helper());
        if let Some(error) = self.name_error.clone() {
            name_field = name_field.error(error);
        }
        let emoji_label = self
            .emoji_key
            .as_deref()
            .map(|key| emoji_name(key, &self.session, cx));
        let picture = sound_emoji(
            self.emoji_key.as_deref(),
            COMPONENT_EMOJI_PICKER_EMOJI_SIZE,
            color(tokens.component_text_field_icon),
            self.session.read(cx).state(),
            window,
            cx,
        );
        let emoji_form = cx.entity().downgrade();
        let remove_form = emoji_form.clone();
        let button_label = if self.emoji_key.is_some() {
            words.change()
        } else {
            words.choose_emoji()
        };
        let accessible_label: SharedString = emoji_label.as_ref().map_or_else(
            || format!("{button_label}, {}", words.emoji_helper()).into(),
            |label| format!("{}, {}", words.change_emoji(label), words.emoji_helper()).into(),
        );
        div()
            .flex()
            .flex_col()
            .gap(length(SPACE_MD))
            .child(name_field)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_TEXT_FIELD_LABEL_GAP))
                    .child(
                        with_type(div(), TYPE_CATEGORY)
                            .text_color(color(tokens.component_text_field_label_text))
                            .child(words.emoji()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(length(SPACE_XS))
                            .child(
                                div()
                                    .relative()
                                    .child(
                                        with_type(div(), TYPE_BODY)
                                            .id("sound-emoji")
                                            .role(Role::Button)
                                            .aria_label(accessible_label)
                                            .aria_expanded(self.emoji_picker.is_some())
                                            .track_focus(&self.emoji_button_focus)
                                            .flex()
                                            .items_center()
                                            .gap(length(SPACE_XS))
                                            .h(length(COMPONENT_TEXT_FIELD_HEIGHT))
                                            .px(length(COMPONENT_TEXT_FIELD_PADDING_INLINE))
                                            .rounded(length(COMPONENT_TEXT_FIELD_RADIUS))
                                            .border(length(COMPONENT_TEXT_FIELD_BORDER_WIDTH))
                                            .border_color(color(tokens.component_text_field_border))
                                            .bg(color(tokens.component_text_field_background))
                                            .text_color(color(tokens.component_text_field_text))
                                            .when(!is_busy, |button| {
                                                button.cursor_pointer().on_click(
                                                    move |_, window, cx| {
                                                        let _ =
                                                            emoji_form.update(cx, |form, cx| {
                                                                form.toggle_emoji_picker(
                                                                    window, cx,
                                                                );
                                                            });
                                                    },
                                                )
                                            })
                                            .on_key_down(cx.listener(
                                                |form, event: &gpui::KeyDownEvent, window, cx| {
                                                    let key = event.keystroke.key.as_str();
                                                    if matches!(key, "enter" | "space")
                                                        && !event.keystroke.modifiers.modified()
                                                    {
                                                        cx.stop_propagation();
                                                        form.toggle_emoji_picker(window, cx);
                                                    }
                                                },
                                            ))
                                            .child(picture)
                                            .child(button_label),
                                    )
                                    .child(bounds_recorder(self.emoji_button_bounds.clone())),
                            )
                            .when(self.emoji_key.is_some(), |row| {
                                row.child(
                                    Button::new("sound-emoji-remove", words.remove_emoji())
                                        .variant(ButtonVariant::Ghost)
                                        .disabled(is_busy)
                                        .on_click(move |window, cx| {
                                            let _ = remove_form.update(cx, |form, cx| {
                                                form.remove_emoji(window, cx);
                                            });
                                        }),
                                )
                            }),
                    )
                    .child(
                        with_type(div(), TYPE_BODY)
                            .text_color(color(tokens.component_text_field_helper_text))
                            .child(words.emoji_helper()),
                    ),
            )
            .when_some(self.emoji_picker.as_ref(), |column, (picker, _)| {
                column.child(picker.clone())
            })
            .into_any_element()
    }
}

struct ModalDialogTexts {
    title: SharedString,
    subtitle: Option<SharedString>,
    /// `None` until there is something to send (adding waits for the file).
    submit: Option<(SharedString, SharedString)>,
}

/// The file at `path` decoded and ready to trim, or why not; runs off the interface thread.
fn read_and_decode(path: &Path) -> Reading {
    let size_bytes = match std::fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            tracing::warn!(error = %error.kind(), "cannot read a picked sound file");
            return Reading::Unreadable;
        }
    };
    if size_bytes > MAXIMUM_SOUND_SOURCE_BYTES {
        return Reading::TooLarge { size_bytes };
    }
    let bytes = match read_file_within(path, MAXIMUM_SOUND_SOURCE_BYTES) {
        Ok(Some(bytes)) => bytes,
        // The file grew past the limit after its size was read.
        Ok(None) => return Reading::TooLarge { size_bytes },
        Err(error) => {
            tracing::warn!(error = %error.kind(), "cannot read a picked sound file");
            return Reading::Unreadable;
        }
    };
    match decode_sound_source(bytes.into()) {
        Ok(samples) => {
            let peaks = waveform_peaks(&[&samples], WAVEFORM_BARS);
            // Samples of at most ten minutes: exact in `f64`.
            #[allow(clippy::cast_precision_loss)]
            let duration_seconds = samples.len() as f64 / f64::from(SOUND_SAMPLE_RATE_HERTZ);
            Reading::Ready(Trimmer {
                samples: samples.into(),
                duration_seconds,
                peaks,
                selection: initial_selection(duration_seconds),
                volume_percent: DEFAULT_VOLUME_PERCENT,
            })
        }
        Err(SoundSourceError::TooLong) => Reading::TooLong,
        Err(SoundSourceError::Unreadable) => Reading::Unreadable,
    }
}

/// The instance's refusals in the web's words.
fn upload_failure(error: &ClientError, cx: &App) -> String {
    let words = strings(cx).server_settings().sounds();
    match sound_upload_refusal(error) {
        SoundUploadRefusal::LimitReached => words.limit_reached().to_owned(),
        SoundUploadRefusal::TooMany => words.too_many().to_owned(),
        SoundUploadRefusal::Refused => words.refused().to_owned(),
        SoundUploadRefusal::Other => describe_error(
            error,
            words.add_failed(),
            crate::strings_pending_i18n::strings(cx),
        ),
    }
}

/// The emoji as a screen reader names it: the character, or `:name:` for a server's own.
fn emoji_name(key: &str, session: &Entity<Session>, cx: &App) -> String {
    match custom_emoji_of_reaction_key(key) {
        Some(emoji_id) => session
            .read(cx)
            .state()
            .guilds()
            .iter()
            .flat_map(|guild| guild.emojis.iter())
            .find(|emoji| emoji.id == emoji_id)
            .map_or_else(String::new, |emoji| format!(":{}:", emoji.name)),
        None => display_emoji(key),
    }
}

fn decimal_separator(language: Language) -> &'static str {
    match language {
        Language::English => ".",
        Language::French => ",",
    }
}

/// A slider's value: seconds of a file of at most ten minutes, precise enough in `f32`.
#[allow(clippy::cast_possible_truncation)]
fn seconds_of(seconds: f64) -> f32 {
    seconds as f32
}

/// Whole percents up to 100: exact in `f32`.
#[allow(clippy::cast_precision_loss)]
fn percent_of(percent: u32) -> f32 {
    percent as f32
}

impl Render for SoundForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.server_settings().sounds();
        let column = div().flex().flex_col().gap(length(SPACE_MD));
        let FormKind::Add { file_name, reading } = &self.kind else {
            return column.child(self.render_details(window, cx));
        };
        let file_name = file_name.clone();
        match reading {
            Reading::InProgress => column
                .child(crate::components::polite_live_region(
                    "sound-reading",
                    words.reading(&file_name).into(),
                ))
                .child(
                    with_type(div(), TYPE_BODY)
                        .text_color(color(tokens.component_text_field_helper_text))
                        .child(words.reading(&file_name)),
                ),
            Reading::Unreadable | Reading::TooLarge { .. } | Reading::TooLong => {
                let sentence = match reading {
                    Reading::TooLarge { size_bytes } => {
                        words.too_large_file(&file_size_label(*size_bytes, catalogue))
                    }
                    Reading::TooLong => words.too_long_file().to_owned(),
                    _ => words.unreadable(&file_name),
                };
                column.child(crate::settings::error_alert(
                    "sound-reading-error",
                    Some(sentence.into()),
                    cx,
                ))
            }
            Reading::Ready(_) => column
                .children(self.render_trimmer(&file_name, catalogue, cx))
                .child(self.render_details(window, cx)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_over_20_mb_is_refused_before_it_is_read_and_noise_is_unreadable() {
        let directory = tempfile::tempdir().unwrap();
        let noise = directory.path().join("noise.mp3");
        std::fs::write(&noise, b"not a sound at all").unwrap();
        assert!(matches!(read_and_decode(&noise), Reading::Unreadable));
        let large = directory.path().join("large.wav");
        let file = std::fs::File::create(&large).unwrap();
        file.set_len(MAXIMUM_SOUND_SOURCE_BYTES + 1).unwrap();
        assert!(matches!(
            read_and_decode(&large),
            Reading::TooLarge { size_bytes } if size_bytes == MAXIMUM_SOUND_SOURCE_BYTES + 1
        ));
        assert!(matches!(
            read_and_decode(&directory.path().join("gone.wav")),
            Reading::Unreadable
        ));
    }

    #[test]
    fn a_short_wav_opens_whole_in_the_trimmer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("horn.wav");
        let wav = encode_wav(&vec![0.5; 24_000]).unwrap();
        std::fs::write(&path, wav).unwrap();
        let Reading::Ready(trimmer) = read_and_decode(&path) else {
            panic!("a canonical WAV reads");
        };
        assert!((trimmer.duration_seconds - 0.5).abs() < 1e-9);
        assert!((trimmer.selection.end_seconds - 0.5).abs() < 1e-9);
        assert_eq!(trimmer.peaks.len(), WAVEFORM_BARS);
        assert!(trimmer.peaks.iter().all(|peak| (peak - 0.5).abs() < 1e-3));
    }
}
