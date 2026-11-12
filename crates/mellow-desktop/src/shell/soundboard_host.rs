//! The voice panel's soundboard button and its popover (`VoicePanel.svelte`): the button opens
//! the call's server sounds above it; a click outside that lands on the button closes the popover
//! without opening it again (`OutsideClickDismissal`). Leaving the call takes the popover with it.

use gpui::{
    AnyElement, AppContext as _, Bounds, Context, Entity, FocusHandle, IntoElement,
    ParentElement as _, Pixels, Styled as _, Subscription, Window, div,
};
use mellow_client_core::shared_logic::server_settings::ServerSettingsSection;

use super::DuskShell;
use crate::components::{IconButton, OutsideClickDismissal, bounds_recorder};
use crate::i18n::strings;
use crate::settings::server::offered_sections;
use crate::soundboard::{SoundboardPopover, SoundboardPopoverEvent};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_USER_PANEL_BUTTON_ICON_SIZE, COMPONENT_USER_PANEL_BUTTON_SIZE,
};

pub(super) struct SoundboardHost {
    popover: Option<(Entity<SoundboardPopover>, Subscription)>,
    dismissal: OutsideClickDismissal,
    button_bounds: Entity<Option<Bounds<Pixels>>>,
    button_focus: FocusHandle,
}

impl SoundboardHost {
    pub(super) fn new(cx: &mut Context<DuskShell>) -> Self {
        Self {
            popover: None,
            dismissal: OutsideClickDismissal::default(),
            button_bounds: cx.new(|_| None),
            button_focus: cx.focus_handle().tab_stop(true),
        }
    }
}

impl DuskShell {
    /// The panel's soundboard button, before Disconnect.
    pub(super) fn render_soundboard_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let shell = cx.entity().downgrade();
        div()
            .relative()
            .flex_none()
            .child(
                IconButton::new(
                    "voice-soundboard",
                    IconName::SpeakerHigh,
                    strings(cx).soundboard().open(),
                )
                .sizes(
                    COMPONENT_USER_PANEL_BUTTON_SIZE,
                    COMPONENT_USER_PANEL_BUTTON_ICON_SIZE,
                )
                .expanded(self.soundboard.popover.is_some())
                .track_focus(&self.soundboard.button_focus)
                .on_click(move |window, cx| {
                    let _ = shell.update(cx, |shell, cx| shell.toggle_soundboard(window, cx));
                }),
            )
            .child(bounds_recorder(self.soundboard.button_bounds.clone()))
            .into_any_element()
    }

    fn toggle_soundboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .soundboard
            .dismissal
            .is_closing_click(window.mouse_position(), &())
        {
            return;
        }
        if self.soundboard.popover.is_some() {
            self.close_soundboard(true, window, cx);
            return;
        }
        let (Some(voice), Some(anchor)) =
            (self.voice.clone(), *self.soundboard.button_bounds.read(cx))
        else {
            return;
        };
        let Some((guild_id, _)) = voice.read(cx).connected_place() else {
            return;
        };
        let can_add_sounds = self
            .session
            .read(cx)
            .state()
            .guild(guild_id)
            .is_some_and(|guild| {
                offered_sections(guild).contains(&ServerSettingsSection::Soundboard)
            });
        let session = self.session.clone();
        let popover = cx.new(|cx| {
            SoundboardPopover::open(anchor, session, voice, guild_id, can_add_sounds, window, cx)
        });
        let subscription = cx.subscribe_in(
            &popover,
            window,
            move |shell, _, event: &SoundboardPopoverEvent, window, cx| match event {
                SoundboardPopoverEvent::Dismissed {
                    should_return_focus,
                } => {
                    let position = window.mouse_position();
                    let is_on_button = shell
                        .soundboard
                        .button_bounds
                        .read(cx)
                        .is_some_and(|bounds| bounds.contains(&position));
                    if !should_return_focus && is_on_button {
                        shell.soundboard.dismissal.remember(position, ());
                    } else {
                        shell.soundboard.dismissal.forget();
                    }
                    shell.close_soundboard(*should_return_focus, window, cx);
                }
                SoundboardPopoverEvent::AddSounds => {
                    shell.close_soundboard(false, window, cx);
                    shell.open_server_settings(
                        guild_id,
                        Some(ServerSettingsSection::Soundboard),
                        window,
                        cx,
                    );
                }
            },
        );
        self.soundboard.popover = Some((popover, subscription));
        cx.notify();
    }

    fn close_soundboard(
        &mut self,
        should_return_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.soundboard.popover.take().is_some() && should_return_focus {
            window.focus(&self.soundboard.button_focus, cx);
        }
        cx.notify();
    }

    /// The open popover, while the call lasts.
    pub(super) fn render_soundboard_popover(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (popover, _) = self.soundboard.popover.as_ref()?;
        let is_in_call = self
            .voice
            .as_ref()
            .is_some_and(|voice| voice.read(cx).channel_id().is_some());
        if !is_in_call {
            cx.defer_in(window, |shell, window, cx| {
                shell.close_soundboard(false, window, cx);
            });
            return None;
        }
        Some(popover.clone().into_any_element())
    }
}
