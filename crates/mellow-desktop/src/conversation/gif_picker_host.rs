//! The GIF picker in the chat pane (`ChatPane.svelte`, ADR-0042): the composer's GIF button opens
//! it beside the button; Esc, a click outside or another place close it. A picked GIF goes out
//! through the composer as its own message and focus goes back to the composer; dismissed with
//! focus inside, focus goes back to the button.

use gpui::{AppContext as _, Context, Entity, Focusable as _, Window};

use mellow_client_core::KlipyClient;

use super::ConversationView;
use crate::gif_picker::{GifPicker, GifPickerEvent};

impl ConversationView {
    /// The composer's GIF button: opens the picker, or closes it.
    pub(super) fn toggle_gif_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The click outside that closed it lands on the button: it must not open it again.
        let is_same_click_as_dismissal = self
            .gif_picker_dismissed_by_pointer_at
            .take()
            .is_some_and(|position| position == window.mouse_position());
        if is_same_click_as_dismissal {
            return;
        }
        if self.gif_picker.is_some() {
            self.close_gif_picker(cx);
            return;
        }
        let Some(settings) = self.session.read(cx).gif_settings().cloned() else {
            return;
        };
        let Some(anchor) = self.composer.read(cx).gif_button_bounds(cx) else {
            return;
        };
        let session = self.session.clone();
        let klipy = KlipyClient::new()
            .inspect_err(|error| tracing::warn!(%error, "cannot reach KLIPY"))
            .ok();
        let picker = cx.new(|cx| GifPicker::new(session, settings, klipy, anchor, window, cx));
        let subscription = cx.subscribe_in(&picker, window, Self::follow_gif_picker);
        self.gif_picker = Some((picker, subscription));
        self.composer
            .update(cx, |composer, cx| composer.set_gif_picker_open(true, cx));
        cx.notify();
    }

    fn follow_gif_picker(
        &mut self,
        picker: &Entity<GifPicker>,
        event: &GifPickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            GifPickerEvent::Pick { message_url, .. } => {
                self.close_gif_picker(cx);
                let message_url = message_url.clone();
                self.composer
                    .update(cx, |composer, cx| composer.send_gif(message_url, cx));
                self.focus_composer(window, cx);
            }
            GifPickerEvent::Dismissed { by_pointer } => {
                let had_focus = picker
                    .read(cx)
                    .focus_handle(cx)
                    .contains_focused(window, cx);
                if *by_pointer {
                    self.gif_picker_dismissed_by_pointer_at = Some(window.mouse_position());
                }
                self.close_gif_picker(cx);
                if had_focus {
                    self.composer
                        .update(cx, |composer, cx| composer.focus_gif_button(window, cx));
                }
            }
        }
    }

    pub(super) fn close_gif_picker(&mut self, cx: &mut Context<Self>) {
        if self.gif_picker.take().is_some() {
            self.composer
                .update(cx, |composer, cx| composer.set_gif_picker_open(false, cx));
            cx.notify();
        }
    }

    /// The GIF button shows only when the instance has GIFs.
    pub(super) fn follow_gif_settings(&self, cx: &mut Context<Self>) {
        let can_choose_gifs = self.session.read(cx).gif_settings().is_some();
        self.composer.update(cx, |composer, cx| {
            composer.set_can_choose_gifs(can_choose_gifs, cx);
        });
    }
}
