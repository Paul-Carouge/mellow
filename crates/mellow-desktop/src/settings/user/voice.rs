//! User settings, Voice (`VoiceSettings.svelte`, `VoiceLevelMeter.svelte`,
//! `PushToTalkKeyRecorder.svelte`): Discord's voice settings in Mellow's order and words. Every
//! change goes through [`change_voice_settings`] and applies at once, to a call too; the choices
//! stay in the preferences file.
//!
//! The device lists are read off the interface thread when the page opens and every few seconds
//! while it is shown (the system tells no change), so a device plugged in shows up; a chosen one
//! that is gone reads "Unplugged device" while voice falls back to the default. The meter reads
//! the call's microphone, else the test's, which only "Test the microphone" or "Hear yourself"
//! opens; it redraws at most every [`METER_REDRAW_PERIOD`] and only when the level moves a whole
//! decibel, and leaving the page stops the test.
//!
//! shortcut: the web's native selects are radio groups here (as in Invites), the threshold is a
//! slider under the meter rather than a mark on it, and the enhancement styles are radio rows
//! rather than a segmented control; the desktop has no select or segmented control yet.
//! shortcut: the web's Video part (camera choice and preview) comes with the call view (M3 step 8).

use std::time::Duration;

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, Modifiers, ModifiersChangedEvent, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _, relative,
};
use mellow_client_core::shared_logic::push_to_talk_keys::{
    ModifierKey, code_of_modifier, code_of_toolkit_key,
};
use mellow_client_core::shared_logic::voice_settings::{
    DeviceChoices, EnhancementCurve, EqualizerBand, ListedAudioDevice, MAXIMUM_DECIBELS,
    MAXIMUM_EQUALIZER_DECIBELS, MAXIMUM_PUSH_TO_TALK_RELEASE_DELAY_MILLISECONDS,
    MAXIMUM_SOUNDBOARD_VOLUME_PERCENT, MAXIMUM_VOLUME_PERCENT, MINIMUM_DECIBELS,
    MINIMUM_EQUALIZER_DECIBELS, MINIMUM_VOLUME_PERCENT, NoiseSuppression, PushToTalkKey,
    VoiceEnhancementPreset, VoiceInputMode, VoiceSettings, device_choices,
    is_chosen_device_unplugged, meter_fills, push_to_talk_key_of,
};
use mellow_media::{AudioDeviceDescription, AudioDeviceError};

use super::parts::{body_text, section_label};
use crate::components::{
    Button, ButtonVariant, ChoiceGroup, ChoiceOption, RangeBounds, RangeField, Switch, icon,
    polite_live_region, with_type,
};
use crate::i18n::{Strings, strings};
use crate::preferences::{PreferencesStore, preferences};
use crate::settings::{SettingsPage, SettingsPageEvent};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_KEY_CHIP_HEIGHT, COMPONENT_KEY_CHIP_MIN_WIDTH, COMPONENT_KEY_CHIP_PADDING_INLINE,
    COMPONENT_KEY_CHIP_RADIUS, COMPONENT_SETTINGS_PAGE_TITLE_GAP, COMPONENT_SETTINGS_SECTION_GAP,
    COMPONENT_SETTINGS_SECTION_LABEL_GAP, COMPONENT_SETTINGS_VOICE_COLUMN_MIN_WIDTH,
    COMPONENT_TEXT_FIELD_LABEL_GAP, COMPONENT_VOICE_METER_HEIGHT, COMPONENT_VOICE_METER_RADIUS,
    COMPONENT_VOICE_METER_THRESHOLD_MARK_HEIGHT, COMPONENT_VOICE_METER_THRESHOLD_MARK_WIDTH,
    COMPONENT_VOICE_PANEL_SIGNAL_ICON_SIZE, RADIUS_FULL, SIZE_HIT_WEB_MIN, SPACE_2XS, SPACE_LG,
    SPACE_MD, SPACE_SM, SPACE_XS, TYPE_BODY, TYPE_BUTTON, TYPE_CATEGORY,
};
use crate::theme::{color, length, theme};
use crate::voice::{Voice, change_voice_settings, voice};

/// How often the device lists are read again while the page is shown.
const DEVICE_REFRESH_PERIOD: Duration = Duration::from_secs(2);
/// The meter's redraw rate at most: smooth to the eye, a few redraws a second of the page.
const METER_REDRAW_PERIOD: Duration = Duration::from_millis(50);
const VOLUME_STEP_PERCENT: f32 = 1.0;
const RELEASE_DELAY_STEP_MILLISECONDS: f32 = 20.0;
const NOISE_SUPPRESSION_CHOICES: [NoiseSuppression; 3] = [
    NoiseSuppression::Advanced,
    NoiseSuppression::Standard,
    NoiseSuppression::Off,
];
const INPUT_MODES: [VoiceInputMode; 2] =
    [VoiceInputMode::VoiceActivity, VoiceInputMode::PushToTalk];
const EQUALIZER_BANDS: [EqualizerBand; 3] = [
    EqualizerBand::Bass,
    EqualizerBand::Mids,
    EqualizerBand::Treble,
];

/// What the meter last drew, in whole decibels, and whether the test failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct MeterReading {
    decibels: Option<i32>,
    has_test_failed: bool,
}

pub struct VoicePage {
    inputs: Vec<ListedAudioDevice>,
    outputs: Vec<ListedAudioDevice>,
    meter: MeterReading,
    key_recorder_focus: FocusHandle,
    is_recording_key: bool,
    /// Modifiers held at the last change, to record one pressed alone.
    held_modifiers: Modifiers,
    key_announcement: SharedString,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for VoicePage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VoicePage")
            .field("inputs", &self.inputs.len())
            .field("outputs", &self.outputs.len())
            .field("is_recording_key", &self.is_recording_key)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<SettingsPageEvent> for VoicePage {}

impl SettingsPage for VoicePage {
    fn set_attention(&mut self, _: u32, _: &mut Context<Self>) {}
}

fn listed_devices(
    listed: Result<Vec<AudioDeviceDescription>, AudioDeviceError>,
) -> Option<Vec<ListedAudioDevice>> {
    match listed {
        Ok(devices) => Some(
            devices
                .into_iter()
                .map(|device| ListedAudioDevice {
                    id: device.id,
                    name: device.name,
                    is_default: device.is_default,
                })
                .collect(),
        ),
        Err(error) => {
            tracing::warn!(%error, "cannot list the audio devices");
            None
        }
    }
}

fn voice_settings(cx: &App) -> VoiceSettings {
    preferences(cx).map_or(VoiceSettings::DEFAULT, |preferences| {
        preferences.voice_settings.clone()
    })
}

/// A slider's value as a whole number within its bounds.
fn whole(value: f32) -> i64 {
    // Slider values are snapped to whole steps within small bounds.
    #[allow(clippy::cast_possible_truncation)]
    let whole = value.round() as i64;
    whole
}

fn whole_percent(value: f32) -> u32 {
    u32::try_from(whole(value)).unwrap_or(0)
}

impl VoicePage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let key_recorder_focus = cx.focus_handle();
        let mut subscriptions = vec![
            cx.observe_global::<PreferencesStore>(|_, cx| cx.notify()),
            cx.on_focus_out(&key_recorder_focus, window, |page, _, _, cx| {
                page.stop_recording_key(cx);
            }),
            // Leaving the page frees the test's microphone and stops hearing yourself.
            cx.on_release(|_, cx| {
                if let Some(voice) = voice(cx) {
                    voice.update(cx, |voice, cx| {
                        voice.set_recording_push_to_talk_key(false);
                        voice.stop_microphone_test(cx);
                    });
                }
            }),
        ];
        let mut tasks = Vec::new();
        if let Some(voice) = voice(cx) {
            subscriptions.push(cx.observe(&voice, |_, _, cx| cx.notify()));
            tasks.push(Self::refresh_devices(cx));
            tasks.push(Self::follow_meter(cx));
        }
        Self {
            inputs: Vec::new(),
            outputs: Vec::new(),
            meter: MeterReading::default(),
            key_recorder_focus,
            is_recording_key: false,
            held_modifiers: Modifiers::default(),
            key_announcement: SharedString::default(),
            _tasks: tasks,
            _subscriptions: subscriptions,
        }
    }

    /// Reads the lists off the interface thread (enumerating blocks), then every few seconds.
    fn refresh_devices(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |page, cx| {
            loop {
                let devices = page.update(cx, |_, cx| {
                    voice(cx).map(|voice| voice.read(cx).audio_devices())
                });
                let Ok(Some(devices)) = devices else {
                    return;
                };
                let (inputs, outputs) = cx
                    .background_spawn(async move {
                        (
                            listed_devices(devices.input_devices()),
                            listed_devices(devices.output_devices()),
                        )
                    })
                    .await;
                let is_gone = page
                    .update(cx, |page, cx| {
                        let mut has_changed = false;
                        // A list that could not be read keeps the last one.
                        for (shown, listed) in
                            [(&mut page.inputs, inputs), (&mut page.outputs, outputs)]
                        {
                            if let Some(listed) = listed.filter(|listed| listed != shown) {
                                *shown = listed;
                                has_changed = true;
                            }
                        }
                        if has_changed {
                            cx.notify();
                        }
                    })
                    .is_err();
                if is_gone {
                    return;
                }
                cx.background_executor().timer(DEVICE_REFRESH_PERIOD).await;
            }
        })
    }

    /// Redraws the meter when the level moves a whole decibel, at most every redraw period.
    fn follow_meter(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |page, cx| {
            loop {
                cx.background_executor().timer(METER_REDRAW_PERIOD).await;
                let is_gone = page
                    .update(cx, |page, cx| {
                        let reading = voice(cx).map_or_else(MeterReading::default, |voice| {
                            let voice = voice.read(cx);
                            MeterReading {
                                decibels: voice
                                    .microphone_decibels()
                                    .map(|decibels| whole(decibels).try_into().unwrap_or(0)),
                                has_test_failed: voice.has_microphone_test_failed(),
                            }
                        });
                        if reading != page.meter {
                            page.meter = reading;
                            cx.notify();
                        }
                    })
                    .is_err();
                if is_gone {
                    return;
                }
            }
        })
    }

    // ---- The push-to-talk key recorder ----

    fn start_recording_key(&mut self, cx: &mut Context<Self>) {
        self.is_recording_key = true;
        self.key_announcement = strings(cx).voice_settings().recording().into();
        if let Some(voice) = voice(cx) {
            voice.update(cx, |voice, _| voice.set_recording_push_to_talk_key(true));
        }
        cx.notify();
    }

    fn stop_recording_key(&mut self, cx: &mut Context<Self>) {
        if !self.is_recording_key {
            return;
        }
        self.is_recording_key = false;
        if let Some(voice) = voice(cx) {
            voice.update(cx, |voice, _| voice.set_recording_push_to_talk_key(false));
        }
        cx.notify();
    }

    /// The next key pressed on the recorder's button becomes the key; Esc cancels, Tab leaves.
    fn record_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if !self.is_recording_key || key == "tab" {
            return;
        }
        // Esc would otherwise close the settings, Space or Enter click the button again.
        cx.stop_propagation();
        window.prevent_default();
        self.stop_recording_key(cx);
        if key == "escape" {
            self.key_announcement = SharedString::default();
            return;
        }
        let Some(code) = code_of_toolkit_key(key) else {
            return;
        };
        let typed = event.keystroke.key_char.as_deref().unwrap_or(key);
        self.save_key(&code, typed, cx);
    }

    /// A modifier pressed alone records it, as the web records `ControlLeft`.
    fn record_modifier(
        &mut self,
        event: &ModifiersChangedEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = std::mem::replace(&mut self.held_modifiers, event.modifiers);
        if !self.is_recording_key || !self.key_recorder_focus.is_focused(window) {
            return;
        }
        let pressed = [
            (
                ModifierKey::Control,
                previous.control,
                event.modifiers.control,
            ),
            (ModifierKey::Shift, previous.shift, event.modifiers.shift),
            (ModifierKey::Alt, previous.alt, event.modifiers.alt),
            (
                ModifierKey::Meta,
                previous.platform,
                event.modifiers.platform,
            ),
        ]
        .into_iter()
        .find_map(|(modifier, was_held, is_held)| (is_held && !was_held).then_some(modifier));
        if let Some(modifier) = pressed {
            self.stop_recording_key(cx);
            self.save_key(code_of_modifier(modifier), "", cx);
        }
    }

    fn save_key(&mut self, code: &str, typed: &str, cx: &mut Context<Self>) {
        let words = strings(cx).voice_settings();
        let Some(recorded) = push_to_talk_key_of(code, typed, |code| words.named_keys(code)) else {
            return;
        };
        self.key_announcement = words.key_recorded(&recorded.label).into();
        change_voice_settings(cx, move |settings| {
            settings.push_to_talk_key = Some(recorded);
        });
        cx.notify();
    }

    fn clear_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.key_announcement = strings(cx).voice_settings().key_cleared().into();
        change_voice_settings(cx, |settings| settings.push_to_talk_key = None);
        // The Clear button goes away: focus stays on the key's own button.
        window.focus(&self.key_recorder_focus, cx);
        cx.notify();
    }

    fn key_recorder(
        &self,
        key: Option<&PushToTalkKey>,
        catalogue: Strings,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let words = catalogue.voice_settings();
        let tokens = theme(cx);
        let chip_text = if self.is_recording_key {
            SharedString::from(words.recording())
        } else {
            key.map_or_else(
                || words.no_key().into(),
                |key| SharedString::from(key.label.clone()),
            )
        };
        let does_key_type_a_character =
            key.is_some_and(|key| key.label.chars().count() == 1 || key.code == "Space");
        let mut helper = catalogue
            .desktop()
            .voice_push_to_talk_focus_note()
            .to_owned();
        if does_key_type_a_character {
            helper.push(' ');
            helper.push_str(words.character_key_note());
        }
        let page = cx.entity();
        let clear_page = page.clone();
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_TEXT_FIELD_LABEL_GAP))
            .child(
                with_type(div(), TYPE_CATEGORY)
                    .text_color(color(tokens.component_text_field_label_text))
                    .child(words.shortcut()),
            )
            .child(
                div()
                    .id("voice-push-to-talk-recorder")
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(length(SPACE_XS))
                    .capture_key_down(cx.listener(Self::record_key))
                    .on_modifiers_changed(cx.listener(Self::record_modifier))
                    .child(key_chip(
                        chip_text.clone(),
                        self.is_recording_key,
                        key.is_some(),
                        cx,
                    ))
                    .child(
                        Button::new(
                            "voice-push-to-talk-record",
                            if key.is_some() {
                                words.change_key()
                            } else {
                                words.record_key()
                            },
                        )
                        .variant(ButtonVariant::Secondary)
                        .accessible_label(format!(
                            "{}, {}: {chip_text}",
                            if key.is_some() {
                                words.change_key()
                            } else {
                                words.record_key()
                            },
                            words.shortcut()
                        ))
                        .track_focus(&self.key_recorder_focus)
                        .on_click(move |_, cx| {
                            page.update(cx, Self::start_recording_key);
                        }),
                    )
                    .when(key.is_some(), |row| {
                        row.child(
                            Button::new("voice-push-to-talk-clear", words.clear_key())
                                .variant(ButtonVariant::Ghost)
                                .on_click(move |window, cx| {
                                    clear_page.update(cx, |page, cx| page.clear_key(window, cx));
                                }),
                        )
                    }),
            )
            .when(key.is_none(), |recorder| {
                recorder.child(warning_line(
                    IconName::WarningCircle,
                    words.no_key_warning(),
                    tokens.component_settings_error_text,
                ))
            })
            .child(helper_text(helper, cx))
            .child(polite_live_region(
                "voice-push-to-talk-announcement",
                self.key_announcement.clone(),
            ))
    }
}

/// The key cap (the web's `<kbd>`): apricot-edged while recording, dimmed when no key is set.
fn key_chip(text: SharedString, is_recording: bool, has_key: bool, cx: &App) -> gpui::Div {
    let tokens = theme(cx);
    let text_color = if !is_recording && has_key {
        tokens.component_key_chip_text
    } else {
        tokens.component_key_chip_empty_text
    };
    with_type(div(), TYPE_BUTTON)
        .flex()
        .items_center()
        .justify_center()
        .h(length(COMPONENT_KEY_CHIP_HEIGHT))
        .min_w(length(COMPONENT_KEY_CHIP_MIN_WIDTH))
        .px(length(COMPONENT_KEY_CHIP_PADDING_INLINE))
        .rounded(length(COMPONENT_KEY_CHIP_RADIUS))
        .border_1()
        .border_color(color(if is_recording {
            tokens.component_key_chip_border_recording
        } else {
            tokens.component_key_chip_border
        }))
        .bg(color(tokens.component_key_chip_background))
        .text_color(color(text_color))
        .child(text)
}

/// A sentence under a field, in the helper colour.
fn helper_text(text: impl Into<SharedString>, cx: &App) -> gpui::Div {
    with_type(div(), TYPE_BODY)
        .text_color(color(theme(cx).component_text_field_helper_text))
        .child(text.into())
}

/// A sentence with its icon (an unplugged device, a missing key).
fn warning_line(icon_name: IconName, text: &'static str, text_color: u32) -> gpui::Div {
    with_type(div(), TYPE_BODY)
        .flex()
        .gap(length(SPACE_2XS))
        .text_color(color(text_color))
        .child(icon(
            icon_name,
            COMPONENT_VOICE_PANEL_SIGNAL_ICON_SIZE,
            color(text_color),
        ))
        .child(div().min_w_0().child(text))
}

/// The radio rows of a device: "Default (name)", the devices, and "Unplugged device" while the
/// chosen one is gone. `choose` stores the choice (`None` is the default).
fn device_group(
    id: &'static str,
    label: &'static str,
    choices: &DeviceChoices,
    chosen_id: Option<&str>,
    missing_sentence: &'static str,
    choose: fn(&mut VoiceSettings, Option<String>),
    cx: &App,
) -> impl IntoElement {
    let words = strings(cx).voice_settings();
    let is_unplugged = is_chosen_device_unplugged(choices, chosen_id);
    let mut options = vec![ChoiceOption::new(
        words.default_device(choices.default_device_name.as_deref()),
    )];
    options.extend(
        choices
            .devices
            .iter()
            .map(|device| ChoiceOption::new(device.label.clone())),
    );
    if is_unplugged {
        options.push(ChoiceOption::new(words.unplugged_device()));
    }
    let checked_index = match chosen_id {
        None => 0,
        Some(chosen) => choices
            .devices
            .iter()
            .position(|device| device.device_id == chosen)
            .map_or(options.len() - 1, |position| position + 1),
    };
    let device_ids: Vec<String> = choices
        .devices
        .iter()
        .map(|device| device.device_id.clone())
        .collect();
    div()
        .flex()
        .flex_col()
        .gap(length(SPACE_XS))
        .child(
            ChoiceGroup::new(id, label, options, checked_index).on_select(move |index, _, cx| {
                let choice = match index {
                    0 => None,
                    // The unplugged row is the choice already made.
                    _ => match device_ids.get(index - 1) {
                        Some(device_id) => Some(device_id.clone()),
                        None => return,
                    },
                };
                change_voice_settings(cx, move |settings| choose(settings, choice));
            }),
        )
        .when(is_unplugged, |group| {
            group.child(warning_line(
                IconName::WarningCircle,
                missing_sentence,
                theme(cx).component_text_field_helper_text,
            ))
        })
}

/// A volume slider in percent; `store` writes it into the settings.
fn percent_slider(
    id: &'static str,
    label: &'static str,
    value: u32,
    maximum: u32,
    store: fn(&mut VoiceSettings, u32),
    catalogue: Strings,
) -> RangeField {
    // Percentages within 0 to 200: exact in `f32`.
    #[allow(clippy::cast_precision_loss)]
    let (value, minimum, maximum) = (value as f32, MINIMUM_VOLUME_PERCENT as f32, maximum as f32);
    RangeField::new(
        id,
        label,
        value,
        RangeBounds {
            minimum,
            maximum,
            step: VOLUME_STEP_PERCENT,
        },
        move |value| catalogue.voice_settings().percent(whole(value)).into(),
    )
    .on_change(move |value, _, cx| {
        let percent = whole_percent(value);
        change_voice_settings(cx, move |settings| store(settings, percent));
    })
}

/// The microphone's level (`VoiceLevelMeter.svelte`): with a manual sensitivity, the part past the
/// threshold lights apricot, the colour of the speaking ring, because that part is what others
/// hear. It redraws without transitions, so reduced motion changes nothing here.
fn level_meter(
    decibels: Option<i32>,
    threshold: Option<i32>,
    catalogue: Strings,
    cx: &App,
) -> impl IntoElement {
    let tokens = theme(cx);
    // Whole decibels within −100 to 0: exact in `f32`.
    #[allow(clippy::cast_precision_loss)]
    let fills = meter_fills(decibels.map(|decibels| decibels as f32), threshold);
    let shown = decibels.unwrap_or(MINIMUM_DECIBELS);
    div()
        .relative()
        .flex()
        .items_center()
        .h(length(SIZE_HIT_WEB_MIN))
        .child(
            div()
                .id("voice-level-meter")
                .role(Role::Meter)
                .aria_label(catalogue.voice_settings().level())
                .aria_min_numeric_value(f64::from(MINIMUM_DECIBELS))
                .aria_max_numeric_value(f64::from(MAXIMUM_DECIBELS))
                .aria_numeric_value(f64::from(shown))
                .aria_value(catalogue.voice_settings().decibels(i64::from(shown)))
                .relative()
                .w_full()
                .h(length(COMPONENT_VOICE_METER_HEIGHT))
                .overflow_hidden()
                .rounded(length(COMPONENT_VOICE_METER_RADIUS))
                .bg(color(tokens.component_voice_meter_track))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(relative(fills.quiet_width))
                        .bg(color(tokens.component_voice_meter_level_quiet)),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(relative(fills.heard_start))
                        .w(relative(fills.heard_width))
                        .bg(color(tokens.component_voice_meter_level_heard)),
                ),
        )
        .when(threshold.is_some(), |meter| {
            meter.child(
                div()
                    .absolute()
                    .left(relative(fills.heard_start))
                    .w(length(COMPONENT_VOICE_METER_THRESHOLD_MARK_WIDTH))
                    .h(length(COMPONENT_VOICE_METER_THRESHOLD_MARK_HEIGHT))
                    .rounded(length(RADIUS_FULL))
                    .bg(color(tokens.component_voice_meter_threshold_mark)),
            )
        })
}

impl Render for VoicePage {
    #[allow(clippy::too_many_lines)] // the page, in the web's order
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalogue = strings(cx);
        let words = catalogue.voice_settings();
        let desktop_words = catalogue.desktop();
        let settings = voice_settings(cx);
        let voice_entity: Option<Entity<Voice>> = voice(cx);
        let (is_live, is_test_running, is_monitoring) =
            voice_entity
                .as_ref()
                .map_or((false, false, false), |voice| {
                    let voice = voice.read(cx);
                    (
                        voice.is_microphone_live(),
                        voice.is_microphone_test_running(),
                        voice.is_monitoring(),
                    )
                });
        let inputs = device_choices(&self.inputs, |position| {
            words.unnamed_microphone(i64::try_from(position).unwrap_or(i64::MAX))
        });
        let outputs = device_choices(&self.outputs, |position| {
            words.unnamed_output(i64::try_from(position).unwrap_or(i64::MAX))
        });
        let is_manual_sensitivity = settings.input_mode == VoiceInputMode::VoiceActivity
            && !settings.is_sensitivity_automatic;
        let column = || {
            div()
                .flex()
                .flex_col()
                .gap(length(SPACE_SM))
                .flex_1()
                .min_w(length(COMPONENT_SETTINGS_VOICE_COLUMN_MIN_WIDTH))
        };

        let devices = div()
            .mt(length(COMPONENT_SETTINGS_SECTION_GAP))
            .flex()
            .flex_wrap()
            .gap_x(length(SPACE_LG))
            .gap_y(length(SPACE_MD))
            .child(
                column()
                    .child(device_group(
                        "voice-input-device",
                        words.input_device(),
                        &inputs,
                        settings.input_device_id.as_deref(),
                        words.input_device_missing(),
                        |settings, device_id| settings.input_device_id = device_id,
                        cx,
                    ))
                    .child(percent_slider(
                        "voice-input-volume",
                        words.input_volume(),
                        settings.input_volume_percent,
                        MAXIMUM_VOLUME_PERCENT,
                        |settings, percent| settings.input_volume_percent = percent,
                        catalogue,
                    )),
            )
            .child(
                column()
                    .child(device_group(
                        "voice-output-device",
                        words.output_device(),
                        &outputs,
                        settings.output_device_id.as_deref(),
                        words.output_device_missing(),
                        |settings, device_id| settings.output_device_id = device_id,
                        cx,
                    ))
                    .child(percent_slider(
                        "voice-output-volume",
                        words.output_volume(),
                        settings.output_volume_percent,
                        MAXIMUM_VOLUME_PERCENT,
                        |settings, percent| settings.output_volume_percent = percent,
                        catalogue,
                    ))
                    .child(percent_slider(
                        "voice-soundboard-volume",
                        words.soundboard_volume(),
                        settings.soundboard_volume_percent,
                        MAXIMUM_SOUNDBOARD_VOLUME_PERCENT,
                        |settings, percent| settings.soundboard_volume_percent = percent,
                        catalogue,
                    ))
                    .child(helper_text(words.soundboard_volume_description(), cx)),
            );

        let input_mode_index = INPUT_MODES
            .iter()
            .position(|mode| *mode == settings.input_mode)
            .unwrap_or_default();
        let input_mode = div().mt(length(COMPONENT_SETTINGS_SECTION_GAP)).child(
            ChoiceGroup::new(
                "voice-input-mode",
                words.input_mode(),
                vec![
                    ChoiceOption::new(words.voice_activity())
                        .description(words.voice_activity_description()),
                    ChoiceOption::new(words.push_to_talk())
                        .description(words.push_to_talk_description()),
                ],
                input_mode_index,
            )
            .on_select(|index, _, cx| {
                if let Some(mode) = INPUT_MODES.get(index).copied() {
                    change_voice_settings(cx, move |settings| settings.input_mode = mode);
                }
            }),
        );

        let mode_details = if settings.input_mode == VoiceInputMode::PushToTalk {
            // Within 0 to 2 000 ms: exact in `f32`.
            #[allow(clippy::cast_precision_loss)]
            let (delay, maximum_delay) = (
                settings.push_to_talk_release_delay_milliseconds as f32,
                MAXIMUM_PUSH_TO_TALK_RELEASE_DELAY_MILLISECONDS as f32,
            );
            div()
                .mt(length(SPACE_MD))
                .flex()
                .flex_col()
                .gap(length(SPACE_MD))
                .child(self.key_recorder(settings.push_to_talk_key.as_ref(), catalogue, cx))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(length(SPACE_2XS))
                        .child(
                            RangeField::new(
                                "voice-release-delay",
                                words.release_delay(),
                                delay,
                                RangeBounds {
                                    minimum: 0.0,
                                    maximum: maximum_delay,
                                    step: RELEASE_DELAY_STEP_MILLISECONDS,
                                },
                                move |value| words.milliseconds(whole(value)).into(),
                            )
                            .on_change(|value, _, cx| {
                                let delay = whole_percent(value);
                                change_voice_settings(cx, move |settings| {
                                    settings.push_to_talk_release_delay_milliseconds = delay;
                                });
                            }),
                        )
                        .child(helper_text(words.release_delay_description(), cx)),
                )
                .child(
                    Switch::new(
                        "voice-push-to-talk-sounds",
                        settings.are_push_to_talk_sounds_on,
                        words.push_to_talk_sounds(),
                    )
                    .description(words.push_to_talk_sounds_description())
                    .on_change(|is_on, _, cx| {
                        change_voice_settings(cx, move |settings| {
                            settings.are_push_to_talk_sounds_on = is_on;
                        });
                    }),
                )
        } else {
            div()
                .mt(length(COMPONENT_SETTINGS_SECTION_GAP))
                .flex()
                .flex_col()
                .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                .child(section_label("voice-sensitivity", words.sensitivity(), cx))
                .child(
                    Switch::new(
                        "voice-automatic-sensitivity",
                        settings.is_sensitivity_automatic,
                        words.automatic_sensitivity(),
                    )
                    .description(desktop_words.voice_automatic_sensitivity_description())
                    .on_change(|is_on, _, cx| {
                        change_voice_settings(cx, move |settings| {
                            settings.is_sensitivity_automatic = is_on;
                        });
                    }),
                )
        };

        let noise_modes = words.noise_suppression_modes();
        let noise_index = NOISE_SUPPRESSION_CHOICES
            .iter()
            .position(|mode| *mode == settings.noise_suppression)
            .unwrap_or_default();
        let processing = div()
            .mt(length(COMPONENT_SETTINGS_SECTION_GAP))
            .flex()
            .flex_col()
            .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
            .child(section_label(
                "voice-processing",
                words.voice_processing(),
                cx,
            ))
            .child(
                ChoiceGroup::new(
                    "voice-noise-suppression",
                    words.noise_suppression(),
                    vec![
                        ChoiceOption::new(noise_modes.advanced().label())
                            .description(noise_modes.advanced().description()),
                        ChoiceOption::new(desktop_words.voice_standard_noise_suppression())
                            .description(
                                desktop_words.voice_standard_noise_suppression_description(),
                            ),
                        ChoiceOption::new(noise_modes.off().label())
                            .description(noise_modes.off().description()),
                    ],
                    noise_index,
                )
                .on_select(|index, _, cx| {
                    if let Some(mode) = NOISE_SUPPRESSION_CHOICES.get(index).copied() {
                        change_voice_settings(cx, move |settings| {
                            settings.noise_suppression = mode;
                        });
                    }
                }),
            )
            .child(
                div().mt(length(SPACE_SM)).child(
                    Switch::new(
                        "voice-enhancement",
                        settings.is_voice_enhancement_on,
                        words.voice_enhancement(),
                    )
                    .description(desktop_words.voice_enhancement_description())
                    .on_change(|is_on, _, cx| {
                        change_voice_settings(cx, move |settings| {
                            settings.is_voice_enhancement_on = is_on;
                        });
                    }),
                ),
            )
            .when(settings.is_voice_enhancement_on, |processing| {
                processing.child(enhancement_details(&settings, catalogue))
            });

        let threshold = is_manual_sensitivity.then_some(settings.sensitivity_threshold_decibels);
        let mut test_description = if is_live {
            words.live_description().to_owned()
        } else {
            words.test_description().to_owned()
        };
        if is_manual_sensitivity {
            test_description.push(' ');
            test_description.push_str(words.threshold_description());
        }
        let voice_for_test = voice_entity.clone();
        let voice_for_monitoring = voice_entity;
        let test = div()
            .mt(length(COMPONENT_SETTINGS_SECTION_GAP))
            .flex()
            .flex_col()
            .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
            .child(section_label(
                "voice-microphone-test",
                words.microphone_test(),
                cx,
            ))
            .child(helper_text(test_description, cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x(length(SPACE_MD))
                    .child(
                        div()
                            .flex_1()
                            .min_w(length(COMPONENT_SETTINGS_VOICE_COLUMN_MIN_WIDTH))
                            .child(level_meter(self.meter.decibels, threshold, catalogue, cx)),
                    )
                    .when(!is_live, |row| {
                        row.child(
                            Button::new(
                                "voice-microphone-test-toggle",
                                if is_test_running {
                                    words.stop_test()
                                } else {
                                    words.start_test()
                                },
                            )
                            .variant(ButtonVariant::Secondary)
                            .on_click(move |_, cx| {
                                if let Some(voice) = voice_for_test.as_ref() {
                                    voice.update(cx, |voice, cx| {
                                        if voice.is_microphone_test_running() {
                                            voice.stop_microphone_test(cx);
                                        } else {
                                            voice.start_microphone_test(cx);
                                        }
                                    });
                                }
                            }),
                        )
                    }),
            )
            .when_some(threshold, |test, threshold| {
                // Within −100 to 0: exact in `f32`.
                #[allow(clippy::cast_precision_loss)]
                let (value, minimum, maximum) = (
                    threshold as f32,
                    MINIMUM_DECIBELS as f32,
                    MAXIMUM_DECIBELS as f32,
                );
                test.child(
                    RangeField::new(
                        "voice-threshold",
                        words.threshold(),
                        value,
                        RangeBounds {
                            minimum,
                            maximum,
                            step: 1.0,
                        },
                        move |value| words.decibels(whole(value)).into(),
                    )
                    .on_change(|value, _, cx| {
                        let threshold = i32::try_from(whole(value)).unwrap_or(MINIMUM_DECIBELS);
                        change_voice_settings(cx, move |settings| {
                            settings.sensitivity_threshold_decibels = threshold;
                        });
                    }),
                )
            })
            .when(self.meter.has_test_failed, |test| {
                test.child(warning_line(
                    IconName::MicrophoneSlash,
                    desktop_words.voice_test_no_microphone(),
                    theme(cx).component_settings_error_text,
                ))
            })
            .child(polite_live_region(
                "voice-microphone-test-problem",
                if self.meter.has_test_failed {
                    desktop_words.voice_test_no_microphone().into()
                } else {
                    SharedString::default()
                },
            ))
            .child(
                Switch::new("voice-hear-yourself", is_monitoring, words.hear_yourself())
                    .description(words.hear_yourself_description())
                    .on_change(move |is_on, _, cx| {
                        if let Some(voice) = voice_for_monitoring.as_ref() {
                            voice.update(cx, |voice, cx| voice.set_monitoring(is_on, cx));
                        }
                    }),
            );

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .mt(length(COMPONENT_SETTINGS_PAGE_TITLE_GAP))
                    .child(body_text(desktop_words.appearance_intro(), cx)),
            )
            .child(devices)
            .child(input_mode)
            .child(mode_details)
            .child(processing)
            .child(test)
    }
}

/// The enhancement's style and its three bands.
fn enhancement_details(settings: &VoiceSettings, catalogue: Strings) -> impl IntoElement {
    let words = catalogue.voice_settings();
    let presets = words.voice_enhancement_presets();
    let mut options: Vec<ChoiceOption> = VoiceEnhancementPreset::ALL
        .iter()
        .map(|preset| {
            let (label, description) = match preset {
                VoiceEnhancementPreset::Natural => {
                    (presets.natural().label(), presets.natural().description())
                }
                VoiceEnhancementPreset::Streamer => {
                    (presets.streamer().label(), presets.streamer().description())
                }
                VoiceEnhancementPreset::Warm => {
                    (presets.warm().label(), presets.warm().description())
                }
                VoiceEnhancementPreset::Clear => {
                    (presets.clear().label(), presets.clear().description())
                }
            };
            ChoiceOption::new(label).description(description)
        })
        .collect();
    let checked_index = match settings.voice_enhancement_curve {
        EnhancementCurve::Preset(chosen) => VoiceEnhancementPreset::ALL
            .iter()
            .position(|preset| *preset == chosen)
            .unwrap_or_default(),
        EnhancementCurve::Custom => {
            options.push(
                ChoiceOption::new(presets.custom().label())
                    .description(presets.custom().description()),
            );
            options.len() - 1
        }
    };
    let equalizer = settings.equalizer_decibels;
    div()
        .mt(length(SPACE_MD))
        .flex()
        .flex_col()
        .gap(length(SPACE_MD))
        .child(
            ChoiceGroup::new(
                "voice-enhancement-style",
                words.voice_enhancement_style(),
                options,
                checked_index,
            )
            .on_select(|index, _, cx| {
                // The "Custom" row is the curve already made.
                if let Some(preset) = VoiceEnhancementPreset::ALL.get(index).copied() {
                    change_voice_settings(cx, move |settings| {
                        *settings = settings.with_enhancement_preset(preset);
                    });
                }
            }),
        )
        .children(EQUALIZER_BANDS.map(|band| {
            let bands = words.equalizer_bands();
            let (id, label, value) = match band {
                EqualizerBand::Bass => ("voice-equalizer-bass", bands.bass(), equalizer.bass),
                EqualizerBand::Mids => ("voice-equalizer-mids", bands.mids(), equalizer.mids),
                EqualizerBand::Treble => {
                    ("voice-equalizer-treble", bands.treble(), equalizer.treble)
                }
            };
            // Within −12 to 12: exact in `f32`.
            #[allow(clippy::cast_precision_loss)]
            let (value, minimum, maximum) = (
                value as f32,
                MINIMUM_EQUALIZER_DECIBELS as f32,
                MAXIMUM_EQUALIZER_DECIBELS as f32,
            );
            RangeField::new(
                id,
                label,
                value,
                RangeBounds {
                    minimum,
                    maximum,
                    step: 1.0,
                },
                move |value| words.equalizer_decibels(whole(value)).into(),
            )
            .on_change(move |value, _, cx| {
                let decibels = i32::try_from(whole(value)).unwrap_or(0);
                change_voice_settings(cx, move |settings| {
                    *settings = settings.with_equalizer_band(band, decibels);
                });
            })
        }))
}

#[cfg(test)]
#[path = "voice_tests.rs"]
mod tests;
