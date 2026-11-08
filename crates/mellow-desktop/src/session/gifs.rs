//! The instance's GIFs (ADR-0042, the web's `loadGifSettings` and its favourites routes): Klipy's
//! key and this member's opaque customer id, asked once per session at its first `Ready`, and the
//! member's starred slugs. Klipy itself is called by the GIF picker, never through the instance.

use gpui::{App, Context};
use mellow_client_core::ClientError;
use mellow_protocol::{ApiErrorCode, GifSettings};

use super::Session;

/// What the session knows of the instance's GIFs.
#[derive(Debug, Default)]
pub(super) struct GifSettingsLoad {
    /// `None` while unknown, and for an instance without GIFs.
    settings: Option<GifSettings>,
    is_loading: bool,
}

impl Session {
    /// Klipy's key and this member's customer id; `None` while unknown or when the instance has no
    /// GIFs, which hides the GIF button and keeps Klipy addresses as links.
    #[must_use]
    pub fn gif_settings(&self) -> Option<&GifSettings> {
        self.gifs.settings.as_ref()
    }

    /// Asked at a `Ready` until it is known: a failure leaves GIFs off until the next one, and an
    /// answer for an earlier session is dropped (`run_request`).
    pub(super) fn load_gif_settings(&mut self, cx: &mut Context<Self>) {
        if self.gifs.settings.is_some() || self.gifs.is_loading {
            return;
        }
        let Some(rest) = self.rest_client().cloned() else {
            return;
        };
        self.gifs.is_loading = true;
        self.run_request(
            cx,
            async move { rest.gif_settings().await },
            |session, answer, cx| {
                session.gifs.is_loading = false;
                match answer {
                    Ok(settings) => {
                        session.gifs.settings = Some(settings);
                        cx.notify();
                    }
                    // The instance without GIFs: nothing to report.
                    Err(ClientError::Api {
                        code: ApiErrorCode::NotFound,
                        ..
                    }) => {}
                    Err(error) => session.report_request_error(&error, "load GIF settings", cx),
                }
            },
        );
    }

    /// The member's starred Klipy slugs, newest first.
    pub fn list_gif_favourites(
        &mut self,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<Vec<String>, ClientError>, &mut App) + 'static,
    ) {
        let Some(rest) = self.rest_client().cloned() else {
            cx.defer(move |cx| on_done(Err(ClientError::NotSignedIn), cx));
            return;
        };
        self.run_request(
            cx,
            async move { rest.list_gif_favourites().await },
            move |session, answer, cx| {
                if let Err(error) = &answer {
                    session.report_request_error(error, "load GIF favourites", cx);
                }
                // Deferred: the picker reads the session from its callback.
                cx.defer(move |cx| on_done(answer, cx));
            },
        );
    }

    /// Stars or unstars a GIF by its Klipy slug.
    pub fn set_gif_favourite(
        &mut self,
        slug: String,
        is_starred: bool,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<(), ClientError>, &mut App) + 'static,
    ) {
        let Some(rest) = self.rest_client().cloned() else {
            cx.defer(move |cx| on_done(Err(ClientError::NotSignedIn), cx));
            return;
        };
        self.run_request(
            cx,
            async move {
                if is_starred {
                    rest.add_gif_favourite(&slug).await
                } else {
                    rest.remove_gif_favourite(&slug).await
                }
            },
            move |session, answer, cx| {
                if let Err(error) = &answer {
                    session.report_request_error(error, "change GIF favourite", cx);
                }
                cx.defer(move |cx| on_done(answer, cx));
            },
        );
    }
}
