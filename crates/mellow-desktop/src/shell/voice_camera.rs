//! The voice panel's camera (`VoicePanel.svelte`, ADR-0035): a toggle before the soundboard,
//! washed while the camera is on, its label saying the action; disabled with the reason when no
//! camera is plugged in or the channel withholds the Video permission. Below the card, why the
//! camera stopped, with macOS's privacy settings a click away when macOS refused it. A polite
//! live region says when the camera is on, and the problem.
//!
//! shortcut: the web's caret menu choosing the camera beside the button is the Voice settings'
//! camera choice here (applied live as well); add the menu with the call view's control bar.

use gpui::{
    AnyElement, App, Context, IntoElement, ParentElement as _, SharedString, Styled as _, div,
    prelude::FluentBuilder as _,
};

use super::DuskShell;
use crate::components::{Button, ButtonVariant, IconButton, icon, polite_live_region, with_type};
use crate::i18n::strings;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_USER_PANEL_BUTTON_ICON_SIZE, COMPONENT_USER_PANEL_BUTTON_SIZE,
    COMPONENT_VOICE_PANEL_SIGNAL_ICON_SIZE, SPACE_2XS, SPACE_3XS, SPACE_XS, TYPE_BODY_SM,
};
use crate::theme::{color, length, theme};
use crate::voice::{
    CameraBlock, CameraPhase, Voice, open_camera_settings_label, open_camera_system_settings,
};

impl DuskShell {
    /// The camera toggle, in the call's row.
    pub(super) fn render_camera_button(&self, cx: &App) -> AnyElement {
        let Some(voice) = self.voice.clone() else {
            return div().into_any_element();
        };
        let strings = strings(cx);
        let (phase, block) = {
            let voice = voice.read(cx);
            (voice.camera_phase(), voice.camera_block(cx))
        };
        let words = strings.camera();
        // Turning it off is always allowed; turning it on says why it cannot.
        let block = block.filter(|_| phase == CameraPhase::Off);
        let label: SharedString = match (phase, block) {
            (CameraPhase::Starting, _) => words.starting().into(),
            (CameraPhase::On, _) => words.turn_off().into(),
            // Connecting: the plain action, the button simply waits.
            (CameraPhase::Off, Some(block)) if block != CameraBlock::NotConnected => {
                format!("{} ({})", words.turn_on(), block.sentence(strings)).into()
            }
            (CameraPhase::Off, _) => words.turn_on().into(),
        };
        let toggle_voice = voice.downgrade();
        IconButton::new("voice-camera", IconName::VideoCamera, label)
            .sizes(
                COMPONENT_USER_PANEL_BUTTON_SIZE,
                COMPONENT_USER_PANEL_BUTTON_ICON_SIZE,
            )
            .toggled(phase != CameraPhase::Off)
            .disabled(block.is_some())
            .on_click(move |_, cx| {
                let _ = toggle_voice.update(cx, Voice::toggle_camera);
            })
            .into_any_element()
    }

    /// Why the camera stopped, and the camera's announcements; under the call's lines.
    pub(super) fn render_camera_notice(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(voice) = self.voice.clone() else {
            return Vec::new();
        };
        let strings = strings(cx);
        let (phase, problem) = {
            let voice = voice.read(cx);
            (voice.camera_phase(), voice.camera_problem())
        };
        let announcement: SharedString = match (problem, phase) {
            (Some(problem), _) => problem.sentence(strings).into(),
            (None, CameraPhase::On) => strings.camera().is_on().into(),
            (None, _) => SharedString::default(),
        };
        let mut notice =
            vec![polite_live_region("voice-camera-announcement", announcement).into_any_element()];
        let Some(problem) = problem else {
            return notice;
        };
        let tokens = theme(cx);
        let dismiss_voice = voice.downgrade();
        notice.push(
            with_type(div(), TYPE_BODY_SM)
                .mt(length(SPACE_2XS))
                .flex()
                .items_start()
                .gap(length(SPACE_2XS))
                .text_color(color(tokens.component_voice_panel_hint_text))
                .child(div().mt(length(SPACE_3XS)).child(icon(
                    IconName::WarningCircle,
                    COMPONENT_VOICE_PANEL_SIGNAL_ICON_SIZE,
                    color(tokens.component_voice_panel_problem_icon),
                )))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(length(SPACE_XS))
                        .child(problem.sentence(strings))
                        .when(problem.can_open_system_settings(), |column| {
                            column.child(
                                div().flex().child(
                                    Button::new(
                                        "voice-camera-open-settings",
                                        open_camera_settings_label(strings),
                                    )
                                    .variant(ButtonVariant::Secondary)
                                    .on_click(|_, cx| open_camera_system_settings(cx)),
                                ),
                            )
                        }),
                )
                .child(
                    IconButton::new(
                        "voice-camera-problem-close",
                        IconName::X,
                        strings.common().close(),
                    )
                    .sizes(
                        COMPONENT_USER_PANEL_BUTTON_SIZE,
                        COMPONENT_VOICE_PANEL_SIGNAL_ICON_SIZE,
                    )
                    .on_click(move |_, cx| {
                        let _ = dismiss_voice.update(cx, Voice::dismiss_camera_problem);
                    }),
                )
                .into_any_element(),
        );
        notice
    }
}
