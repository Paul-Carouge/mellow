//! Sharing your screen from the voice panel (`VoicePanel.svelte`'s share row,
//! `ScreenSoundHelpDialog.svelte`, ADR-0037, ADR-0054): a split button on its own row, its share
//! half starting or stopping the share and its quality half opening the share options (quality,
//! sound, and on Wayland the app whose sound goes with a window); "You're sharing your screen"
//! with the live chip and what the share sends as sound; the problem, with System Settings at
//! hand when macOS refused the recording. The in-app picker (macOS, X11) is a modal dialog of
//! cards, screens then windows, each a picture and a name: pressing one shares it. On Wayland the
//! desktop's own dialog is the picker, and the row waits for it.

use gpui::{
    AnyElement, AppContext as _, Bounds, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div,
    prelude::FluentBuilder as _,
};
use mellow_media::{ScreenPicker, ScreenSourceKind};
use mellow_protocol::{VideoCodec, VoiceVideoRefusal};

use super::DuskShell;
use super::call_view::live_chip;
use crate::components::{
    Button, ButtonVariant, ContextMenu, IconButton, MenuAlignment, MenuAnchor, MenuEntry, MenuItem,
    MenuOrigin, ModalDialog, ModalDialogEvent, bounds_recorder, focus_ring, icon, is_focus_visible,
    remembered_focus_handle, remembered_hover, with_type,
};
use crate::i18n::{Strings, strings};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_SCREEN_PICKER_CARD_PADDING, COMPONENT_SCREEN_PICKER_CARD_RADIUS,
    COMPONENT_SCREEN_PICKER_CARD_WIDTH, COMPONENT_SCREEN_PICKER_GAP,
    COMPONENT_SCREEN_PICKER_LIST_MAX_HEIGHT, COMPONENT_SCREEN_PICKER_THUMBNAIL_HEIGHT,
    COMPONENT_SCREEN_PICKER_THUMBNAIL_ICON_SIZE, COMPONENT_SCREEN_PICKER_THUMBNAIL_RADIUS,
    COMPONENT_USER_PANEL_BUTTON_SIZE, COMPONENT_VOICE_PANEL_SHARE_HEIGHT,
    COMPONENT_VOICE_PANEL_SHARE_RADIUS, COMPONENT_VOICE_PANEL_SHARE_SPLIT_GAP,
    COMPONENT_VOICE_PANEL_SIGNAL_ICON_SIZE, SPACE_2XS, SPACE_3XS, SPACE_XS, TYPE_BODY,
    TYPE_BODY_SM, TYPE_BUTTON, TYPE_HEADING_MD,
};
use crate::theme::{color, length, theme};
use crate::voice::Voice;
use crate::voice::screen_share::{
    SCREEN_SHARE_QUALITIES, ScreenSharePhase, ScreenShareProblem, ScreenSourceCard,
    ScreenSourceListing, SharedSound, screen_share_quality_label,
};
use crate::voice::video_view::{VideoFit, video_view};

/// macOS's Screen & System Audio Recording pane.
const SCREEN_RECORDING_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

pub(super) struct ScreenShareHost {
    options_menu: Option<(Entity<ContextMenu>, Subscription)>,
    picker: Option<(Entity<ModalDialog>, Subscription)>,
    sound_help: Option<(Entity<ModalDialog>, Subscription)>,
    row_bounds: Entity<Option<Bounds<Pixels>>>,
    share_focus: FocusHandle,
    options_focus: FocusHandle,
}

impl ScreenShareHost {
    pub(super) fn new(cx: &mut Context<DuskShell>) -> Self {
        Self {
            options_menu: None,
            picker: None,
            sound_help: None,
            row_bounds: cx.new(|_| None),
            share_focus: cx.focus_handle().tab_stop(true),
            options_focus: cx.focus_handle().tab_stop(true),
        }
    }

    /// A menu or dialog of the share holds focus.
    pub(super) fn is_open(&self) -> bool {
        self.options_menu.is_some() || self.picker.is_some() || self.sound_help.is_some()
    }
}

/// The problem in words (`strings.screenShare.problems`, and the desktop's own).
pub(super) fn screen_share_problem_sentence(
    strings: Strings,
    problem: ScreenShareProblem,
) -> &'static str {
    let desktop = strings.desktop();
    let problems = strings.screen_share().problems();
    match problem {
        ScreenShareProblem::PermissionDenied => desktop.screen_share_permission_denied(),
        ScreenShareProblem::Unavailable => desktop.screen_share_unavailable(),
        ScreenShareProblem::SourceGone => desktop.screen_share_source_gone(),
        ScreenShareProblem::CouldNotStart => desktop.screen_share_failed(),
        ScreenShareProblem::CaptureFailed => desktop.screen_share_capture_failed(),
        ScreenShareProblem::Refused(VoiceVideoRefusal::MissingPermissions) => {
            problems.missing_permissions()
        }
        ScreenShareProblem::Refused(VoiceVideoRefusal::TooManyCameras) => {
            problems.too_many_cameras()
        }
        ScreenShareProblem::Refused(VoiceVideoRefusal::VideoUnavailable) => {
            problems.video_unavailable()
        }
        ScreenShareProblem::StoppedByServer => problems.stopped(),
    }
}

/// What the running share sends as sound, under the live line; `None` says nothing.
fn shared_sound_sentence(strings: Strings, sound: &SharedSound) -> (IconName, String) {
    let desktop = strings.desktop();
    match sound {
        SharedSound::FromSource => (
            IconName::SpeakerHigh,
            strings.screen_share().shared_sound().screen().to_owned(),
        ),
        SharedSound::Application(name) => (
            IconName::SpeakerHigh,
            desktop.screen_share_sound_of_application(name),
        ),
        SharedSound::Off => (
            IconName::SpeakerSlash,
            strings.screen_share().shared_sound().none().to_owned(),
        ),
        SharedSound::NotAvailableOnWayland => (
            IconName::SpeakerSlash,
            desktop.screen_share_no_sound_on_wayland().to_owned(),
        ),
        SharedSound::NotAvailableOnThisSystem => (
            IconName::SpeakerSlash,
            desktop.screen_share_no_sound_on_this_system().to_owned(),
        ),
    }
}

/// A display has no title on macOS: the picker names it by its place among the displays.
fn card_name(strings: Strings, card: &ScreenSourceCard, display_number: usize) -> String {
    if !card.title.is_empty() {
        return card.title.clone();
    }
    match card.kind {
        ScreenSourceKind::Display => strings
            .desktop()
            .screen_picker_screen_name(i64::try_from(display_number).unwrap_or(i64::MAX)),
        ScreenSourceKind::Window => card.application_name.clone().unwrap_or_default(),
    }
}

impl DuskShell {
    /// The share row and its lines, under the panel's first row while connected.
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    pub(super) fn render_screen_share_section(
        &mut self,
        voice_entity: &Entity<Voice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let voice = voice_entity.read(cx);
        let phase = voice.screen_share_phase();
        let problem = voice.screen_share_problem();
        let can_share = voice.can_share_screen();
        if !can_share && phase == ScreenSharePhase::Off && problem.is_none() {
            return None;
        }
        let strings = strings(cx);
        let tokens = theme(cx);
        let picker = voice.screen_picker();
        let sound = voice.shared_sound().clone();
        let codec = voice.screen_share_codec();
        let quality = Voice::screen_share_quality(cx);
        let is_on = phase == ScreenSharePhase::On;
        let is_starting = phase == ScreenSharePhase::Starting;
        let is_menu_open = self.screen_share.options_menu.is_some();
        let hint_ink = color(tokens.component_voice_panel_hint_text);

        let share_id = gpui::ElementId::from("voice-screen-share");
        let share_hover = remembered_hover(&share_id, window, cx);
        let is_share_hovered = *share_hover.read(cx);
        let options_id = gpui::ElementId::from("voice-screen-share-options");
        let options_hover = remembered_hover(&options_id, window, cx);
        let is_options_hovered = *options_hover.read(cx);
        let (share_background, share_ink) = match (is_on, is_share_hovered) {
            (true, false) => (
                tokens.component_voice_panel_share_background_active,
                tokens.component_voice_panel_share_text_active,
            ),
            (true, true) => (
                tokens.component_voice_panel_share_background_active_hover,
                tokens.component_voice_panel_share_text_active_hover,
            ),
            (false, false) => (
                tokens.component_voice_panel_share_background,
                tokens.component_voice_panel_share_text,
            ),
            (false, true) => (
                tokens.component_voice_panel_share_background_hover,
                tokens.component_voice_panel_share_text_hover,
            ),
        };
        let (options_background, options_ink) = if is_menu_open || is_options_hovered {
            (
                tokens.component_voice_panel_share_background_hover,
                tokens.component_voice_panel_share_text_hover,
            )
        } else {
            (
                tokens.component_voice_panel_share_background,
                tokens.component_voice_panel_share_text,
            )
        };
        let share_label = if is_on {
            strings.screen_share().stop()
        } else if is_starting {
            strings.desktop().screen_share_starting()
        } else {
            strings.screen_share().share()
        };
        let quality_label = screen_share_quality_label(quality);
        let is_share_ring_shown = is_focus_visible(&self.screen_share.share_focus, window);
        let is_options_ring_shown = is_focus_visible(&self.screen_share.options_focus, window);
        let half = |id: gpui::ElementId, focus: &FocusHandle, background, ink| {
            with_type(div(), TYPE_BUTTON)
                .id(id)
                .role(Role::Button)
                .track_focus(focus)
                .relative()
                .h(length(COMPONENT_VOICE_PANEL_SHARE_HEIGHT))
                .min_w_0()
                .flex()
                .items_center()
                .gap(length(SPACE_2XS))
                .rounded(length(COMPONENT_VOICE_PANEL_SHARE_RADIUS))
                .bg(color(background))
                .text_color(color(ink))
        };
        let shell = cx.entity().downgrade();
        let options_shell = shell.clone();
        let share_half = half(
            share_id,
            &self.screen_share.share_focus,
            share_background,
            share_ink,
        )
        .debug_selector(|| "voice-screen-share-button".into())
        .aria_label(share_label)
        .flex_1()
        .px(length(SPACE_XS))
        .when(!is_starting, gpui::Styled::cursor_pointer)
        .on_hover(move |is_hovered, _, cx| {
            share_hover.update(cx, |remembered, cx| {
                *remembered = *is_hovered;
                cx.notify();
            });
        })
        .on_click(move |_, window, cx| {
            let _ = shell.update(cx, |shell, cx| shell.press_screen_share(window, cx));
        })
        .when(!is_on, |half| {
            half.child(icon(
                IconName::MonitorArrowUp,
                COMPONENT_VOICE_PANEL_SIGNAL_ICON_SIZE,
                color(share_ink),
            ))
        })
        .child(div().min_w_0().truncate().child(share_label))
        .when(is_share_ring_shown, |half| {
            half.child(focus_ring(COMPONENT_VOICE_PANEL_SHARE_RADIUS, cx))
        });
        let options_half = half(
            options_id,
            &self.screen_share.options_focus,
            options_background,
            options_ink,
        )
        .debug_selector(|| "voice-screen-share-options-button".into())
        .aria_label(strings.screen_share().options_button(quality_label))
        .aria_expanded(is_menu_open)
        .flex_none()
        .px(length(SPACE_2XS))
        .cursor_pointer()
        .on_hover(move |is_hovered, _, cx| {
            options_hover.update(cx, |remembered, cx| {
                *remembered = *is_hovered;
                cx.notify();
            });
        })
        .on_click(move |_, window, cx| {
            let _ = options_shell.update(cx, |shell, cx| {
                shell.toggle_screen_share_options(window, cx);
            });
        })
        .child(quality_label)
        .child(icon(
            IconName::CaretUp,
            COMPONENT_VOICE_PANEL_SIGNAL_ICON_SIZE,
            color(options_ink),
        ))
        .when(is_options_ring_shown, |half| {
            half.child(focus_ring(COMPONENT_VOICE_PANEL_SHARE_RADIUS, cx))
        });

        let line = |icon_name: Option<(IconName, gpui::Hsla)>, text: SharedString| {
            with_type(div(), TYPE_BODY_SM)
                .mt(length(SPACE_2XS))
                .flex()
                .items_start()
                .gap(length(SPACE_2XS))
                .text_color(hint_ink)
                .when_some(icon_name, |line, (icon_name, ink)| {
                    line.child(div().mt(length(SPACE_3XS)).child(icon(
                        icon_name,
                        COMPONENT_VOICE_PANEL_SIGNAL_ICON_SIZE,
                        ink,
                    )))
                })
                .child(div().min_w_0().flex_1().child(text))
        };
        let problem_ink = color(tokens.component_voice_panel_problem_icon);
        let dismiss_voice = voice_entity.downgrade();
        let problem_row = problem.map(|problem| {
            let is_permission = problem == ScreenShareProblem::PermissionDenied;
            div()
                .debug_selector(|| "voice-screen-share-problem".into())
                .mt(length(SPACE_2XS))
                .flex()
                .items_start()
                .gap(length(SPACE_2XS))
                .child(
                    line(
                        Some((IconName::WarningCircle, problem_ink)),
                        screen_share_problem_sentence(strings, problem).into(),
                    )
                    .mt_0()
                    .flex_1()
                    .when(is_permission && cfg!(target_os = "macos"), |line| {
                        line.child(
                            Button::new(
                                "voice-screen-share-settings",
                                strings.desktop().open_system_settings(),
                            )
                            .variant(ButtonVariant::Secondary)
                            .on_click(|_, cx| cx.open_url(SCREEN_RECORDING_SETTINGS_URL)),
                        )
                        .flex_wrap()
                    }),
                )
                .child(
                    IconButton::new(
                        "voice-screen-share-problem-close",
                        IconName::X,
                        strings.common().close(),
                    )
                    .sizes(
                        COMPONENT_USER_PANEL_BUTTON_SIZE,
                        COMPONENT_VOICE_PANEL_SIGNAL_ICON_SIZE,
                    )
                    .on_click(move |_, cx| {
                        let _ = dismiss_voice.update(cx, Voice::dismiss_screen_share_problem);
                    }),
                )
        });
        let (sound_icon, sound_sentence) = shared_sound_sentence(strings, &sound);
        // Linux with the portal: VP8 means VA-API's H.264 was not found (ADR-0054).
        let is_software_encoded = picker == ScreenPicker::System && codec == VideoCodec::Vp8;
        Some(
            div()
                .debug_selector(|| "voice-screen-share-section".into())
                .flex()
                .flex_col()
                .when(can_share || is_on || is_starting, |section| {
                    section.child(
                        div()
                            .mt(length(SPACE_XS))
                            .relative()
                            .flex()
                            .gap(length(COMPONENT_VOICE_PANEL_SHARE_SPLIT_GAP))
                            .child(share_half)
                            .child(options_half)
                            .child(bounds_recorder(self.screen_share.row_bounds.clone())),
                    )
                })
                .when(is_starting && picker == ScreenPicker::System, |section| {
                    section.child(
                        line(
                            None,
                            strings.desktop().screen_share_waiting_for_system().into(),
                        )
                        .debug_selector(|| "voice-screen-share-waiting".into()),
                    )
                })
                .when(is_on, |section| {
                    section
                        .child(
                            with_type(div(), TYPE_BODY_SM)
                                .debug_selector(|| "voice-screen-share-live".into())
                                .mt(length(SPACE_2XS))
                                .flex()
                                .items_center()
                                .gap(length(SPACE_2XS))
                                .text_color(hint_ink)
                                .child(live_chip(strings.screen_share().live(), cx))
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .child(strings.screen_share().you_are_sharing()),
                                ),
                        )
                        .child(line(Some((sound_icon, hint_ink)), sound_sentence.into()))
                        .when(is_software_encoded, |section| {
                            section.child(
                                line(
                                    None,
                                    strings.desktop().screen_share_software_encoding().into(),
                                )
                                .debug_selector(|| "voice-screen-share-software-encoding".into()),
                            )
                        })
                })
                .children(problem_row)
                .into_any_element(),
        )
    }

    /// The share half: stop while sharing, else the picker (in-app) or the system's dialog.
    fn press_screen_share(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(voice) = self.voice.clone() else {
            return;
        };
        match voice.read(cx).screen_share_phase() {
            ScreenSharePhase::On => voice.update(cx, Voice::stop_screen_share),
            ScreenSharePhase::Starting => {}
            ScreenSharePhase::Off => self.open_screen_picker(&voice, window, cx),
        }
    }

    /// Opens the in-app picker over the window (macOS, X11), or starts the share through the
    /// system's dialog (Wayland). Also "switch source" from the share options.
    fn open_screen_picker(
        &mut self,
        voice: &Entity<Voice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        voice.update(cx, Voice::start_screen_share);
        if voice.read(cx).screen_sources().is_none() || self.screen_share.picker.is_some() {
            return;
        }
        let strings = strings(cx);
        let quality = screen_share_quality_label(Voice::screen_share_quality(cx));
        let summary = if Voice::is_screen_share_sound_on(cx) {
            strings.desktop().screen_picker_with_sound(quality)
        } else {
            strings.desktop().screen_picker_without_sound(quality)
        };
        let body = cx.new(|cx| ScreenPickerBody::new(voice.clone(), cx));
        let dialog = cx.new(|cx| {
            ModalDialog::new(
                strings.screen_share().share(),
                strings.common().cancel(),
                window,
                cx,
            )
            .subtitle(summary)
            .wide()
            .body(body.clone().into())
        });
        body.update(cx, |body, _| body.dialog = Some(dialog.downgrade()));
        let closed_voice = voice.downgrade();
        let subscription = cx.subscribe(&dialog, move |shell, _, event: &ModalDialogEvent, cx| {
            if *event == ModalDialogEvent::Closed {
                shell.screen_share.picker = None;
                let _ = closed_voice.update(cx, Voice::close_screen_picker);
                cx.notify();
            }
        });
        self.screen_share.picker = Some((dialog, subscription));
        cx.notify();
    }

    /// The share options above the row: quality, sound, switching source, the sound's help.
    #[allow(clippy::too_many_lines)] // One menu's entries, in the order they are drawn.
    fn toggle_screen_share_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen_share.options_menu.take().is_some() {
            cx.notify();
            return;
        }
        let (Some(voice), Some(row)) = (self.voice.clone(), *self.screen_share.row_bounds.read(cx))
        else {
            return;
        };
        voice.update(cx, Voice::refresh_sound_applications);
        let strings = strings(cx);
        let current_quality = Voice::screen_share_quality(cx);
        let is_sound_on = Voice::is_screen_share_sound_on(cx);
        let (picker, is_on, can_switch_source, applications, chosen) = {
            let voice = voice.read(cx);
            (
                voice.screen_picker(),
                voice.is_sharing_screen(),
                voice.can_switch_screen_source(),
                voice.sound_applications().to_vec(),
                voice
                    .chosen_sound_application()
                    .map(|chosen| chosen.id.clone()),
            )
        };
        let hints = strings.screen_share().quality_hints();
        let mut entries: Vec<MenuEntry> = SCREEN_SHARE_QUALITIES
            .into_iter()
            .map(|quality| {
                let voice = voice.downgrade();
                MenuEntry::Item(
                    MenuItem::new(
                        screen_share_quality_label(quality),
                        IconName::Check,
                        move |_, cx| {
                            let _ = voice.update(cx, |voice, cx| {
                                voice.set_screen_share_quality(quality, cx);
                            });
                        },
                    )
                    .description(match quality {
                        mellow_media::ScreenShareQuality::Standard => hints.key_720p(),
                        mellow_media::ScreenShareQuality::High => hints.key_1080p(),
                        mellow_media::ScreenShareQuality::Highest => hints.key_1440p(),
                    })
                    .checked(Some(quality == current_quality)),
                )
            })
            .collect();
        entries.push(MenuEntry::Separator);
        let sound_voice = voice.downgrade();
        let mut sound_item = MenuItem::new(
            strings.desktop().screen_share_sound(),
            IconName::SpeakerHigh,
            move |_, cx| {
                let _ = sound_voice.update(cx, |voice, cx| {
                    voice.set_screen_share_sound_on(!is_sound_on, cx);
                });
            },
        )
        .checked(Some(is_sound_on));
        if is_on {
            sound_item = sound_item.description(strings.desktop().screen_share_sound_next_share());
        }
        entries.push(MenuEntry::Item(sound_item));
        if picker == ScreenPicker::System && is_sound_on {
            for application in applications {
                let voice = voice.downgrade();
                let is_chosen = chosen.as_deref() == Some(application.id.as_str());
                let label = strings.desktop().screen_share_sound_of(&application.name);
                entries.push(MenuEntry::Item(
                    MenuItem::new(label, IconName::MusicNotes, move |_, cx| {
                        let choice = (!is_chosen).then(|| application.clone());
                        let _ = voice.update(cx, |voice, cx| {
                            voice.choose_sound_application(choice, cx);
                        });
                    })
                    .description(strings.desktop().screen_share_sound_of_hint())
                    .checked(Some(is_chosen)),
                ));
            }
        }
        entries.push(MenuEntry::Separator);
        if is_on && can_switch_source {
            let shell = cx.entity().downgrade();
            let switch_voice = voice.clone();
            entries.push(MenuEntry::Item(MenuItem::new(
                strings.desktop().screen_share_switch_source(),
                IconName::MonitorArrowUp,
                move |window, cx| {
                    let _ = shell.update(cx, |shell, cx| {
                        shell.open_screen_picker(&switch_voice, window, cx);
                    });
                },
            )));
        }
        let help_shell = cx.entity().downgrade();
        entries.push(MenuEntry::Item(MenuItem::new(
            strings.desktop().screen_sound_help_open(),
            IconName::Lightbulb,
            move |window, cx| {
                let _ = help_shell.update(cx, |shell, cx| {
                    shell.open_screen_sound_help(window, cx);
                });
            },
        )));
        let origin = MenuOrigin::Anchor {
            bounds: MenuAnchor {
                top: f32::from(row.top()),
                bottom: f32::from(row.bottom()),
                left: f32::from(row.left()),
                right: f32::from(row.right()),
            },
            alignment: MenuAlignment::Stretch,
        };
        let opener = Some(self.screen_share.options_focus.clone());
        let label = strings.screen_share().options_menu();
        let menu = cx.new(|cx| ContextMenu::open(label, entries, origin, opener, window, cx));
        let subscription = cx.subscribe(&menu, |shell, _, _: &gpui::DismissEvent, cx| {
            shell.screen_share.options_menu = None;
            cx.notify();
        });
        self.screen_share.options_menu = Some((menu, subscription));
        cx.notify();
    }

    fn open_screen_sound_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(voice) = self.voice.as_ref() else {
            return;
        };
        let picker = voice.read(cx).screen_picker();
        let strings = strings(cx);
        window.focus(&self.screen_share.options_focus, cx);
        let body = cx.new(|_| ScreenSoundHelp { picker });
        let dialog = cx.new(|cx| {
            ModalDialog::new(
                strings.desktop().screen_sound_help_title(),
                strings.screen_share().sound_help().close(),
                window,
                cx,
            )
            .body(body.into())
        });
        let subscription = cx.subscribe(&dialog, |shell, _, event: &ModalDialogEvent, cx| {
            if *event == ModalDialogEvent::Closed {
                shell.screen_share.sound_help = None;
                cx.notify();
            }
        });
        self.screen_share.sound_help = Some((dialog, subscription));
        cx.notify();
    }

    /// The share's menu and dialogs, while the call lasts.
    pub(super) fn render_screen_share_overlays(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let is_in_call = self
            .voice
            .as_ref()
            .is_some_and(|voice| voice.read(cx).channel_id().is_some());
        let is_picker_wanted = self
            .voice
            .as_ref()
            .is_some_and(|voice| voice.read(cx).screen_sources().is_some());
        if !is_in_call && self.screen_share.is_open() {
            cx.defer_in(window, |shell, _, cx| {
                shell.screen_share.options_menu = None;
                shell.screen_share.picker = None;
                shell.screen_share.sound_help = None;
                cx.notify();
            });
            return Vec::new();
        }
        // The share started, or the call ended: the picker goes, focus back on the row.
        if !is_picker_wanted && let Some((dialog, _)) = self.screen_share.picker.as_ref() {
            let dialog = dialog.clone();
            cx.defer_in(window, move |_, window, cx| {
                dialog.update(cx, |dialog, cx| dialog.request_close(window, cx));
            });
        }
        let mut overlays: Vec<AnyElement> = self
            .screen_share
            .options_menu
            .iter()
            .map(|(menu, _)| menu.clone().into_any_element())
            .collect();
        overlays.extend(
            self.screen_share
                .picker
                .iter()
                .chain(self.screen_share.sound_help.iter())
                .map(|(dialog, _)| dialog.clone().into_any_element()),
        );
        overlays
    }
}

/// The picker's cards, inside the dialog that owns the footer. Follows the voice's list.
struct ScreenPickerBody {
    voice: Entity<Voice>,
    dialog: Option<WeakEntity<ModalDialog>>,
    _observation: Subscription,
}

impl ScreenPickerBody {
    fn new(voice: Entity<Voice>, cx: &mut Context<Self>) -> Self {
        let observation = cx.observe(&voice, |_, _, cx| cx.notify());
        Self {
            voice,
            dialog: None,
            _observation: observation,
        }
    }

    fn render_card(
        &self,
        card: &ScreenSourceCard,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let strings = strings(cx);
        let id = gpui::ElementId::from(SharedString::from(format!("screen-source-{}", card.id)));
        let focus = remembered_focus_handle(&id, window, cx);
        let hover = remembered_hover(&id, window, cx);
        let is_hovered = *hover.read(cx);
        let is_ring_shown = is_focus_visible(&focus, window);
        let source_id = card.id.clone();
        let voice = self.voice.downgrade();
        let dialog = self.dialog.clone();
        let label = strings.desktop().screen_picker_share_source(&name);
        let application = card
            .application_name
            .clone()
            .filter(|application| card.kind == ScreenSourceKind::Window && *application != name);
        let placeholder_icon = match card.kind {
            ScreenSourceKind::Display => IconName::MonitorArrowUp,
            ScreenSourceKind::Window => IconName::SquaresFour,
        };
        div()
            .id(id)
            .debug_selector({
                let source_id = card.id.clone();
                move || format!("screen-source-{source_id}")
            })
            .role(Role::Button)
            .aria_label(label)
            .track_focus(&focus)
            .relative()
            .flex_none()
            .w(length(COMPONENT_SCREEN_PICKER_CARD_WIDTH))
            .p(length(COMPONENT_SCREEN_PICKER_CARD_PADDING))
            .rounded(length(COMPONENT_SCREEN_PICKER_CARD_RADIUS))
            .when(is_hovered, |card| {
                card.bg(color(tokens.component_screen_picker_card_background_hover))
            })
            .cursor_pointer()
            .flex()
            .flex_col()
            .gap(length(SPACE_2XS))
            .on_hover(move |is_hovered, _, cx| {
                hover.update(cx, |remembered, cx| {
                    *remembered = *is_hovered;
                    cx.notify();
                });
            })
            .on_click(move |_, window, cx| {
                if let Some(dialog) = dialog.as_ref().and_then(WeakEntity::upgrade) {
                    dialog.update(cx, |dialog, cx| dialog.request_close(window, cx));
                }
                let source_id = source_id.clone();
                let _ = voice.update(cx, |voice, cx| voice.share_screen_source(source_id, cx));
            })
            .child(
                div()
                    .h(length(COMPONENT_SCREEN_PICKER_THUMBNAIL_HEIGHT))
                    .w_full()
                    .overflow_hidden()
                    .rounded(length(COMPONENT_SCREEN_PICKER_THUMBNAIL_RADIUS))
                    .bg(color(tokens.component_screen_picker_thumbnail_background))
                    .flex()
                    .items_center()
                    .justify_center()
                    .map(|frame| match card.thumbnail.as_ref() {
                        Some(picture) => frame.child(video_view(picture, VideoFit::Contain)),
                        None => frame.child(icon(
                            placeholder_icon,
                            COMPONENT_SCREEN_PICKER_THUMBNAIL_ICON_SIZE,
                            color(tokens.component_screen_picker_thumbnail_icon),
                        )),
                    }),
            )
            .child(
                with_type(div(), TYPE_BODY)
                    .min_w_0()
                    .truncate()
                    .text_color(color(tokens.component_screen_picker_card_text))
                    .child(name),
            )
            .children(application.map(|application| {
                with_type(div(), TYPE_BODY_SM)
                    .min_w_0()
                    .truncate()
                    .text_color(color(tokens.component_screen_picker_card_hint_text))
                    .child(application)
            }))
            .when(is_ring_shown, |card| {
                card.child(focus_ring(COMPONENT_SCREEN_PICKER_CARD_RADIUS, cx))
            })
            .into_any_element()
    }
}

impl Render for ScreenPickerBody {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let strings = strings(cx);
        let tokens = theme(cx);
        let sentence = |text: &'static str| {
            with_type(div(), TYPE_BODY)
                .text_color(color(tokens.component_screen_picker_card_hint_text))
                .child(text)
                .into_any_element()
        };
        let listing = self.voice.read(cx).screen_sources().cloned();
        let content: AnyElement = match listing {
            None | Some(ScreenSourceListing::Loading) => {
                sentence(strings.desktop().screen_picker_loading())
            }
            Some(ScreenSourceListing::Failed(problem)) => div()
                .flex()
                .flex_col()
                .items_start()
                .gap(length(SPACE_XS))
                .child(sentence(screen_share_problem_sentence(strings, problem)))
                .when(
                    problem == ScreenShareProblem::PermissionDenied && cfg!(target_os = "macos"),
                    |content| {
                        content.child(
                            Button::new(
                                "screen-picker-settings",
                                strings.desktop().open_system_settings(),
                            )
                            .variant(ButtonVariant::Secondary)
                            .on_click(|_, cx| cx.open_url(SCREEN_RECORDING_SETTINGS_URL)),
                        )
                    },
                )
                .into_any_element(),
            Some(ScreenSourceListing::Listed(cards)) if cards.is_empty() => {
                sentence(strings.desktop().screen_picker_empty())
            }
            Some(ScreenSourceListing::Listed(cards)) => {
                let mut display_number = 0;
                let mut screens = Vec::new();
                let mut windows = Vec::new();
                for card in &cards {
                    if card.kind == ScreenSourceKind::Display {
                        display_number += 1;
                    }
                    let name = card_name(strings, card, display_number);
                    let element = self.render_card(card, name, window, cx);
                    match card.kind {
                        ScreenSourceKind::Display => screens.push(element),
                        ScreenSourceKind::Window => windows.push(element),
                    }
                }
                let group = |id: &'static str, title: &'static str, cards: Vec<AnyElement>| {
                    (!cards.is_empty()).then(|| {
                        div()
                            .id(id)
                            .role(Role::Group)
                            .aria_label(title)
                            .flex()
                            .flex_col()
                            .gap(length(SPACE_2XS))
                            .child(
                                with_type(div(), TYPE_HEADING_MD)
                                    .text_color(color(tokens.component_screen_picker_card_text))
                                    .child(title),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap(length(COMPONENT_SCREEN_PICKER_GAP))
                                    .children(cards),
                            )
                    })
                };
                div()
                    .id("screen-picker-sources")
                    .max_h(length(COMPONENT_SCREEN_PICKER_LIST_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(length(SPACE_XS))
                    .children(group(
                        "screen-picker-screens",
                        strings.desktop().screen_picker_screens(),
                        screens,
                    ))
                    .children(group(
                        "screen-picker-windows",
                        strings.desktop().screen_picker_windows(),
                        windows,
                    ))
                    .into_any_element()
            }
        };
        div()
            .debug_selector(|| "screen-picker".into())
            .child(content)
    }
}

/// The share's sound, in a few sentences adapted to this system (the web's dialog explains
/// `pactl` commands a browser needs; the desktop captures the sound itself).
struct ScreenSoundHelp {
    picker: ScreenPicker,
}

impl Render for ScreenSoundHelp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let strings = strings(cx);
        let desktop = strings.desktop();
        let tokens = theme(cx);
        let sentences = [
            Some(desktop.screen_sound_help_screen()),
            Some(desktop.screen_sound_help_window()),
            (self.picker == ScreenPicker::System).then(|| desktop.screen_sound_help_wayland()),
            cfg!(target_os = "macos").then(|| desktop.screen_sound_help_macos()),
            Some(desktop.screen_sound_help_off()),
        ];
        div()
            .debug_selector(|| "screen-sound-help".into())
            .flex()
            .flex_col()
            .gap(length(SPACE_XS))
            .children(sentences.into_iter().flatten().map(|sentence| {
                with_type(div(), TYPE_BODY)
                    .text_color(color(tokens.component_dialog_body_text))
                    .child(sentence)
            }))
    }
}

#[cfg(test)]
#[path = "screen_share_host_tests.rs"]
mod tests;
