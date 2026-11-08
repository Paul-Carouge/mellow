//! The GIF picker (`GifPicker.svelte`, DESIGN.md §6.80, docs/design/web-gif-picker.md, ADR-0042):
//! Klipy's search field, the Favourites and Trending tabs, then a contact sheet of GIFs in
//! justified rows, in Klipy's order. It drops beside the composer's GIF button. Klipy is called by
//! this app directly (`KlipyClient`), never through the instance; the instance only keeps the
//! member's starred slugs.
//!
//! Keyboard: the search field first (Down enters the tiles, Enter searches now), the tabs (Left
//! and Right), then one Tab stop for the tiles (arrows over the rows, Home and End, Enter sends,
//! Up from the first row returns to the search), then the active tile's star; Esc closes.
//!
//! shortcut: no blurred previews under loading tiles (Klipy's base64 pictures), no star icon in
//! the Favourites tab (the shared tabs take text only), and no phone sheet (the desktop window
//! keeps the popover).

mod layout;
#[cfg(test)]
mod tests;

use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable as _, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    ObjectFit, ParentElement as _, Pixels, Render, Role, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Subscription, Task, Window,
    anchored, deferred, div, img, point, prelude::FluentBuilder as _, px,
};
use gpui_component::input::{Input, InputEvent, InputState};
use mellow_client_core::shared_logic::klipy::{
    GifTileKey, GifTileMove, KLIPY_PAGE_SIZE, KlipyGif, KlipyItem, KlipyMedia, TileBounds,
    favourites_page, gifs_in_slug_order, klipy_items_url, klipy_list_url, klipy_locale_of,
    next_gif_tile, tile_aspect_ratio, with_favourite_set,
};
use mellow_client_core::{ClientError, KlipyClient, KlipyError};
use mellow_protocol::{ApiErrorCode, GifSettings, MAXIMUM_GIF_FAVOURITES};

use self::layout::{justified_rows, sheet_height};
use crate::components::{
    Button, FocusNextControl, FocusPreviousControl, MenuAnchor, Tabs, focus_next_inside,
    focus_previous_inside, focus_ring, icon, is_focus_visible, popover_position_beside_anchor,
    shadow, with_type,
};
use crate::i18n::{Strings, strings};
use crate::messages::{TOKEN_PIXELS_PER_REM, picture_animation, picture_for_box};
use crate::pictures::{Animation, PictureFile};
use crate::session::{ConnectionStatus, Session};
use crate::sign_in_form::describe_error;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE, COMPONENT_CONTEXT_MENU_VIEWPORT_MARGIN,
    COMPONENT_EMOJI_PICKER_SEARCH_HEIGHT, COMPONENT_GIF_PICKER_BORDER_WIDTH,
    COMPONENT_GIF_PICKER_GAP, COMPONENT_GIF_PICKER_HEIGHT, COMPONENT_GIF_PICKER_PADDING,
    COMPONENT_GIF_PICKER_RADIUS, COMPONENT_GIF_PICKER_WIDTH, COMPONENT_GIF_ROW_HEIGHT,
    COMPONENT_GIF_STAR_BUTTON_SIZE, COMPONENT_GIF_STAR_ICON_SIZE, COMPONENT_GIF_STAR_INSET,
    COMPONENT_GIF_TILE_GAP, COMPONENT_GIF_TILE_RADIUS, RADIUS_SM, SIZE_FOCUS_RING_WIDTH,
    SIZE_HIT_DESKTOP_MIN, SPACE_2XS, SPACE_XS, TYPE_BODY, TYPE_BODY_SM,
};
use crate::theme::{color, length, theme, window_width_at_default_zoom};
use crate::tokio_bridge::spawn_on_network_runtime;

/// Typing pauses this long before Klipy is asked: one request per word, not per letter.
const GIF_SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);
/// The loading sheet's blocks, in row shapes (no shimmer: motion only answers actions).
const PLACEHOLDER_ASPECT_RATIOS: [f32; 9] = [1.3, 0.8, 1.0, 1.6, 1.0, 0.75, 1.2, 1.4, 0.9];
/// Tiles this many sheet heights above or below the visible part still load their picture, so
/// scrolling finds them ready; farther ones wait (the web's `loading="lazy"`).
const PICTURE_LOAD_MARGIN_IN_SHEETS: f32 = 0.5;
/// The next page is asked for once the end is this close (the web's 50 % root margin).
const NEXT_PAGE_DISTANCE_IN_SHEETS: f32 = 0.5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GifPickerEvent {
    /// Send this GIF; `query` is the search that found it, empty from the tabs (Klipy's share
    /// trigger wants it).
    Pick {
        slug: String,
        message_url: String,
        query: String,
    },
    /// Closed without choosing: Esc, or a click outside (`by_pointer`, whose button must not open
    /// it again on the same click).
    Dismissed { by_pointer: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GifTab {
    Favourites,
    Trending,
}

/// Why the sheet shows nothing (the web's `KlipyFailure`, and favourites the instance did not
/// give).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GifFailure {
    Offline,
    RateLimited,
    Refused,
    Failed,
    Favourites,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SheetStatus {
    Loading,
    Shown,
    Failed(GifFailure),
}

/// What one request asks Klipy for.
#[derive(Debug, Clone)]
enum PageRequest {
    List {
        url: String,
    },
    Favourites {
        slugs: Vec<String>,
        asked_count: usize,
        has_next: bool,
    },
}

pub struct GifPicker {
    session: Entity<Session>,
    settings: GifSettings,
    klipy: Option<KlipyClient>,
    locale: String,
    anchor: MenuAnchor,
    search: Entity<InputState>,
    /// The search Klipy was asked for, after typing paused.
    searched_query: String,
    tab: GifTab,
    items: Vec<KlipyItem>,
    status: SheetStatus,
    has_next: bool,
    next_page: u32,
    /// Favourites are paged over the list as it was when their first page loaded.
    frozen_favourite_slugs: Vec<String>,
    favourites_asked_count: usize,
    is_loading_more: bool,
    more_failure: Option<GifFailure>,
    /// The active tile among the GIFs (ads are not tiles).
    active_index: usize,
    hovered_index: Option<usize>,
    /// `None` until the instance answered, and when it could not.
    favourite_slugs: Option<Vec<String>>,
    /// False until the instance's list arrived: stars changed before do not make it whole.
    are_favourites_known: bool,
    favourite_message: SharedString,
    announcement: SharedString,
    /// Only the latest request may change the sheet: each one bumps it.
    request_generation: u64,
    in_flight: Option<Task<()>>,
    search_pause: Option<Task<()>>,
    container_focus: FocusHandle,
    grid_focus: FocusHandle,
    star_focus: FocusHandle,
    scroll: ScrollHandle,
    last_connection_status: ConnectionStatus,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for GifPicker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GifPicker")
            .field("tab", &self.tab)
            .field("status", &self.status)
            .field("items", &self.items.len())
            .finish_non_exhaustive()
    }
}

impl EventEmitter<GifPickerEvent> for GifPicker {}

/// The tile's picture: the small animated one, or the still JPEG while motion is reduced; the web
/// plays Klipy's MP4 where it has no small WebP, the desktop takes the animated picture instead.
fn tile_picture(gif: &KlipyGif, animation: Animation) -> Option<&KlipyMedia> {
    let animated = gif.picture_tile.as_ref();
    match animation {
        Animation::Animate => animated.or(gif.still.as_ref()),
        Animation::StillFrame => gif.still.as_ref().or(animated),
    }
}

fn gifs_of(items: &[KlipyItem]) -> impl Iterator<Item = &KlipyGif> {
    items.iter().filter_map(|item| match item {
        KlipyItem::Gif(gif) => Some(gif),
        KlipyItem::Ad(_) => None,
    })
}

fn failure_sentence(failure: GifFailure, strings: Strings) -> &'static str {
    let gifs = strings.gifs();
    match failure {
        GifFailure::Offline => gifs.offline(),
        GifFailure::RateLimited => gifs.rate_limited(),
        GifFailure::Refused => gifs.refused(),
        GifFailure::Favourites => gifs.favourites_failed(),
        GifFailure::Failed => gifs.failed(),
    }
}

fn favourite_failure_sentence(
    error: &ClientError,
    strings: Strings,
    pending_strings: &crate::strings_pending_i18n::Strings,
) -> String {
    match error {
        ClientError::Api { status: 409, .. }
        | ClientError::Api {
            code: ApiErrorCode::LimitReached,
            ..
        } => strings
            .gifs()
            .too_many_favourites(i64::from(MAXIMUM_GIF_FAVOURITES)),
        ClientError::Api {
            code: ApiErrorCode::RateLimited,
            ..
        } => strings.gifs().favourites_busy().to_owned(),
        error => describe_error(error, strings.gifs().favourite_failed(), pending_strings),
    }
}

/// How far the sheet is scrolled once the row from `top` to `bottom` is in view: unchanged when it
/// already is, else by the shortest distance (the web's `scrollIntoView({ block: "nearest" })`).
/// All in one unit; `scrolled` is measured from the sheet's top, `visible` is its visible height.
fn scrolled_to_show(top: f32, bottom: f32, scrolled: f32, visible: f32) -> f32 {
    if top < scrolled {
        top
    } else if bottom > scrolled + visible {
        bottom - visible
    } else {
        scrolled
    }
}

/// The tile key an arrow names, as `nextGifTile` reads `event.key`.
fn tile_key(key: &str) -> Option<GifTileKey> {
    Some(match key {
        "left" => GifTileKey::Left,
        "right" => GifTileKey::Right,
        "up" => GifTileKey::Up,
        "down" => GifTileKey::Down,
        "home" => GifTileKey::Home,
        "end" => GifTileKey::End,
        _ => return None,
    })
}

impl GifPicker {
    /// Opens beside `anchor` (the GIF button's bounds), the search focused, on Favourites when
    /// the member has some, else on Trending. Without `klipy` (no TLS stack) it says Klipy cannot
    /// be reached.
    pub fn new(
        session: Entity<Session>,
        settings: GifSettings,
        klipy: Option<KlipyClient>,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = strings(cx).gifs().placeholder();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        search.update(cx, |search, cx| search.focus(window, cx));
        let last_connection_status = session.read(cx).status();
        let subscriptions = vec![
            cx.subscribe(&search, |picker, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    picker.follow_query(cx);
                }
            }),
            cx.observe(&session, |picker, session, cx| {
                let status = session.read(cx).status();
                if status != picker.last_connection_status {
                    picker.last_connection_status = status;
                    if status == ConnectionStatus::Connected {
                        picker.retry_after_reconnecting(cx);
                    }
                }
            }),
        ];
        let mut picker = Self {
            session,
            settings,
            klipy,
            locale: klipy_locale_of(&sys_locale::get_locale().unwrap_or_default()),
            anchor: MenuAnchor {
                top: f32::from(anchor.top()),
                bottom: f32::from(anchor.bottom()),
                left: f32::from(anchor.left()),
                right: f32::from(anchor.right()),
            },
            search,
            searched_query: String::new(),
            tab: GifTab::Trending,
            items: Vec::new(),
            status: SheetStatus::Loading,
            has_next: false,
            next_page: 1,
            frozen_favourite_slugs: Vec::new(),
            favourites_asked_count: 0,
            is_loading_more: false,
            more_failure: None,
            active_index: 0,
            hovered_index: None,
            favourite_slugs: None,
            are_favourites_known: false,
            favourite_message: SharedString::default(),
            announcement: SharedString::default(),
            request_generation: 0,
            in_flight: None,
            search_pause: None,
            container_focus: cx.focus_handle(),
            grid_focus: cx.focus_handle().tab_stop(true),
            star_focus: cx.focus_handle().tab_stop(true),
            scroll: ScrollHandle::new(),
            last_connection_status,
            _subscriptions: subscriptions,
        };
        picker.open_on_favourites_or_trending(cx);
        picker
    }

    fn is_searching(&self) -> bool {
        !self.searched_query.is_empty()
    }

    fn start_request(&mut self) -> u64 {
        self.request_generation += 1;
        self.request_generation
    }

    /// Favourites first when there are some, as on Discord; trending otherwise. A tab chosen or a
    /// search typed meanwhile starts its own request and wins.
    fn open_on_favourites_or_trending(&mut self, cx: &mut Context<Self>) {
        let opening = self.start_request();
        self.load_favourite_slugs(cx, move |picker, cx| {
            if picker.request_generation != opening {
                return;
            }
            picker.tab = if picker
                .favourite_slugs
                .as_ref()
                .is_some_and(|slugs| !slugs.is_empty())
            {
                GifTab::Favourites
            } else {
                GifTab::Trending
            };
            picker.load_first_page(cx);
        });
    }

    fn load_favourite_slugs(
        &mut self,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        let picker = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.list_gif_favourites(cx, move |answer, cx| {
                let _ = picker.update(cx, |picker, cx| {
                    match answer {
                        Ok(slugs) => {
                            picker.favourite_slugs = Some(slugs);
                            picker.are_favourites_known = true;
                        }
                        Err(_) => picker.favourite_slugs = None,
                    }
                    then(picker, cx);
                    cx.notify();
                });
            });
        });
    }

    /// The first page of whatever the sheet shows now.
    fn load_first_page(&mut self, cx: &mut Context<Self>) {
        let request = self.start_request();
        self.in_flight = None;
        self.next_page = 1;
        self.favourites_asked_count = 0;
        self.items.clear();
        self.has_next = false;
        self.more_failure = None;
        self.is_loading_more = false;
        self.active_index = 0;
        self.scroll.set_offset(point(px(0.0), px(0.0)));
        self.status = SheetStatus::Loading;
        if !self.is_searching() && self.tab == GifTab::Favourites && !self.are_favourites_known {
            self.load_favourite_slugs(cx, move |picker, cx| {
                if picker.request_generation != request {
                    return;
                }
                if picker.favourite_slugs.is_none() {
                    picker.status = SheetStatus::Failed(GifFailure::Favourites);
                    return;
                }
                picker.frozen_favourite_slugs = picker.favourite_slugs.clone().unwrap_or_default();
                picker.fetch(request, true, cx);
            });
            cx.notify();
            return;
        }
        self.frozen_favourite_slugs = self.favourite_slugs.clone().unwrap_or_default();
        self.fetch(request, true, cx);
        cx.notify();
    }

    /// "More GIFs", or the end of the sheet coming into view.
    fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.is_loading_more || !self.has_next || self.status != SheetStatus::Shown {
            return;
        }
        let request = self.start_request();
        self.is_loading_more = true;
        self.more_failure = None;
        self.fetch(request, false, cx);
        cx.notify();
    }

    fn next_page_request(&self) -> PageRequest {
        if self.is_searching() || self.tab == GifTab::Trending {
            return PageRequest::List {
                url: klipy_list_url(
                    &self.settings,
                    &self.searched_query,
                    self.next_page,
                    &self.locale,
                ),
            };
        }
        let next = favourites_page(
            &self.frozen_favourite_slugs,
            self.favourites_asked_count,
            KLIPY_PAGE_SIZE,
        );
        PageRequest::Favourites {
            slugs: next.slugs,
            asked_count: next.asked_count,
            has_next: next.has_next,
        }
    }

    fn fetch(&mut self, request: u64, is_first_page: bool, cx: &mut Context<Self>) {
        let page_request = self.next_page_request();
        let url = match &page_request {
            PageRequest::List { url } => Some(url.clone()),
            PageRequest::Favourites { slugs, .. } if slugs.is_empty() => None,
            PageRequest::Favourites { slugs, .. } => Some(klipy_items_url(&self.settings, slugs)),
        };
        let Some(url) = url else {
            // No favourite to ask for: an empty sheet, without a request.
            self.receive(
                request,
                is_first_page,
                page_request,
                Ok((Vec::new(), false)),
                cx,
            );
            return;
        };
        let Some(klipy) = self.klipy.clone() else {
            self.receive(
                request,
                is_first_page,
                page_request,
                Err(KlipyError::Failed),
                cx,
            );
            return;
        };
        let answer = spawn_on_network_runtime(cx, async move {
            klipy
                .page(&url)
                .await
                .map(|page| (page.items, page.has_next))
        });
        // Replacing the task drops the previous one, which cancels its request.
        self.in_flight = Some(cx.spawn(async move |picker, cx| {
            let answer = answer.await.unwrap_or(Err(KlipyError::Unreachable));
            let _ = picker.update(cx, |picker, cx| {
                picker.receive(request, is_first_page, page_request, answer, cx);
            });
        }));
    }

    fn receive(
        &mut self,
        request: u64,
        is_first_page: bool,
        page_request: PageRequest,
        answer: Result<(Vec<KlipyItem>, bool), KlipyError>,
        cx: &mut Context<Self>,
    ) {
        if self.request_generation != request {
            return;
        }
        self.is_loading_more = false;
        match answer {
            Ok((items, has_next)) => {
                let items = match page_request {
                    PageRequest::List { .. } => {
                        self.next_page += 1;
                        self.has_next = has_next;
                        items
                    }
                    PageRequest::Favourites {
                        slugs,
                        asked_count,
                        has_next,
                    } => {
                        self.favourites_asked_count = asked_count;
                        self.has_next = has_next;
                        gifs_in_slug_order(&items, &slugs)
                            .into_iter()
                            .map(KlipyItem::Gif)
                            .collect()
                    }
                };
                if is_first_page {
                    self.items = items;
                    self.status = SheetStatus::Shown;
                } else {
                    self.items.extend(items);
                }
                let count = gifs_of(&self.items).count();
                self.announcement = strings(cx)
                    .gifs()
                    .count(i64::try_from(count).unwrap_or(i64::MAX))
                    .into();
            }
            Err(error) => {
                let failure = self.failure_of(error, cx);
                if is_first_page {
                    self.status = SheetStatus::Failed(failure);
                } else {
                    self.more_failure = Some(failure);
                }
            }
        }
        cx.notify();
    }

    fn failure_of(&self, error: KlipyError, cx: &App) -> GifFailure {
        match error {
            // The desktop's "offline": the instance's link is down too.
            KlipyError::Unreachable
                if self.session.read(cx).status() == ConnectionStatus::Reconnecting =>
            {
                GifFailure::Offline
            }
            KlipyError::RateLimited => GifFailure::RateLimited,
            KlipyError::Refused => GifFailure::Refused,
            KlipyError::Unreachable | KlipyError::Failed | KlipyError::TooLarge => {
                GifFailure::Failed
            }
        }
    }

    /// The web's `online` listener: what failed while offline is asked again.
    fn retry_after_reconnecting(&mut self, cx: &mut Context<Self>) {
        if self.status == SheetStatus::Failed(GifFailure::Offline) {
            self.load_first_page(cx);
        } else if self.more_failure == Some(GifFailure::Offline) {
            self.more_failure = None;
            self.load_more(cx);
        }
    }

    fn show_tab(&mut self, tab: GifTab, cx: &mut Context<Self>) {
        // While the picker opens, even the tab already shown is a choice that must stick.
        if tab == self.tab && !self.is_searching() && self.status != SheetStatus::Loading {
            return;
        }
        self.tab = tab;
        self.load_first_page(cx);
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().trim().to_owned()
    }

    fn follow_query(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        if query.is_empty() {
            // Back to the tabs at once: nothing to wait for.
            self.search_pause = None;
            if self.is_searching() {
                self.searched_query.clear();
                self.load_first_page(cx);
            }
            return;
        }
        if query == self.searched_query {
            // Back to the words already searched: nothing new to ask.
            self.search_pause = None;
            return;
        }
        self.search_pause = Some(cx.spawn(async move |picker, cx| {
            cx.background_executor().timer(GIF_SEARCH_DEBOUNCE).await;
            let _ = picker.update(cx, Self::search_now);
        }));
    }

    fn search_now(&mut self, cx: &mut Context<Self>) {
        self.search_pause = None;
        let query = self.query(cx);
        if query.is_empty() {
            return;
        }
        self.searched_query = query;
        self.load_first_page(cx);
    }

    /// Optimistic: the star changes at once and comes back if the instance refuses.
    fn toggle_favourite(&mut self, slug: String, cx: &mut Context<Self>) {
        let slugs = self.favourite_slugs.clone().unwrap_or_default();
        let should_star = !slugs.contains(&slug);
        self.favourite_message = SharedString::default();
        let catalogue = strings(cx);
        if should_star && slugs.len() >= MAXIMUM_GIF_FAVOURITES as usize {
            self.favourite_message = catalogue
                .gifs()
                .too_many_favourites(i64::from(MAXIMUM_GIF_FAVOURITES))
                .into();
            cx.notify();
            return;
        }
        self.favourite_slugs = Some(with_favourite_set(&slugs, &slug, should_star));
        cx.notify();
        let picker = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.set_gif_favourite(slug.clone(), should_star, cx, move |answer, cx| {
                let _ = picker.update(cx, |picker, cx| {
                    match answer {
                        Ok(()) => {
                            picker.announcement = if should_star {
                                catalogue.gifs().favourite_added()
                            } else {
                                catalogue.gifs().favourite_removed()
                            }
                            .into();
                        }
                        Err(error) => {
                            // Only this GIF goes back: other stars changed meanwhile stay.
                            let slugs = picker.favourite_slugs.clone().unwrap_or_default();
                            picker.favourite_slugs =
                                Some(with_favourite_set(&slugs, &slug, !should_star));
                            picker.favourite_message = favourite_failure_sentence(
                                &error,
                                catalogue,
                                crate::strings_pending_i18n::strings(cx),
                            )
                            .into();
                        }
                    }
                    cx.notify();
                });
            });
        });
    }

    fn pick(&mut self, gif_index: usize, cx: &mut Context<Self>) {
        let Some(gif) = gifs_of(&self.items).nth(gif_index) else {
            return;
        };
        // Klipy's share trigger, as their API asks; it outlives the picker, which closes now.
        if let Some(klipy) = self.klipy.clone() {
            let settings = self.settings.clone();
            let slug = gif.slug.clone();
            let query = self.searched_query.clone();
            spawn_on_network_runtime(cx, async move {
                if let Err(error) = klipy.trigger_share(&settings, &slug, &query).await {
                    tracing::debug!(%error, "KLIPY's share trigger failed");
                }
            })
            .detach();
        }
        cx.emit(GifPickerEvent::Pick {
            slug: gif.slug.clone(),
            message_url: gif.message_url.clone(),
            query: self.searched_query.clone(),
        });
    }

    // ---- Layout ----

    /// The panel's width and height in token pixels, inside the window.
    fn panel_size(window: &Window) -> (f32, f32) {
        let viewport_height = f32::from(window.viewport_size().height) * TOKEN_PIXELS_PER_REM
            / f32::from(window.rem_size());
        let margin = 2.0 * SPACE_XS;
        (
            COMPONENT_GIF_PICKER_WIDTH.min(window_width_at_default_zoom(window) - margin),
            COMPONENT_GIF_PICKER_HEIGHT.min(viewport_height - margin),
        )
    }

    fn sheet_width(window: &Window) -> f32 {
        Self::panel_size(window).0
            - 2.0 * (COMPONENT_GIF_PICKER_PADDING + COMPONENT_GIF_PICKER_BORDER_WIDTH)
    }

    /// Each item's place, and each GIF tile's among the items.
    fn placements(&self, window: &Window) -> Vec<TileBounds> {
        let ratios: Vec<f32> = self.items.iter().map(tile_aspect_ratio).collect();
        justified_rows(
            &ratios,
            Self::sheet_width(window),
            COMPONENT_GIF_ROW_HEIGHT,
            COMPONENT_GIF_TILE_GAP,
        )
    }

    fn gif_bounds(&self, window: &Window) -> Vec<TileBounds> {
        self.items
            .iter()
            .zip(self.placements(window))
            .filter(|(item, _)| matches!(item, KlipyItem::Gif(_)))
            .map(|(_, bounds)| bounds)
            .collect()
    }

    // ---- Keys ----

    #[allow(clippy::unused_self)] // A `cx.listener`, which hands the view in.
    fn on_container_key_down(
        &mut self,
        event: &KeyDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified() {
            cx.stop_propagation();
            cx.emit(GifPickerEvent::Dismissed { by_pointer: false });
        }
    }

    /// Down enters the tiles; Enter searches now rather than after the pause (it never sends).
    fn on_search_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        match event.keystroke.key.as_str() {
            "down" if gifs_of(&self.items).next().is_some() => {
                cx.stop_propagation();
                self.focus_tile(0, window, cx);
            }
            "enter" => {
                cx.stop_propagation();
                self.search_now(cx);
            }
            _ => {}
        }
    }

    fn on_grid_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        let key = event.keystroke.key.as_str();
        if key == "enter" || key == "space" {
            cx.stop_propagation();
            self.pick(self.active_index, cx);
            return;
        }
        let Some(tile_key) = tile_key(key) else {
            return;
        };
        let bounds = self.gif_bounds(window);
        let Some(next) = next_gif_tile(&bounds, self.active_index, tile_key) else {
            return;
        };
        cx.stop_propagation();
        match next {
            GifTileMove::Search => {
                let focus = self.search.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
            GifTileMove::Tile(index) => self.focus_tile(index, window, cx),
        }
    }

    fn focus_tile(&mut self, gif_index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active_index = gif_index;
        window.focus(&self.grid_focus, cx);
        // Scroll the tile's row into view by offset: the rows sit inside the grid, not directly
        // in the scrolled sheet, so `scroll_to_item` would not know their bounds.
        let visible = f32::from(self.scroll.bounds().size.height);
        if let Some(tile) = self.gif_bounds(window).get(gif_index)
            && visible > 0.0
        {
            let pixels_per_token = f32::from(window.rem_size()) / TOKEN_PIXELS_PER_REM;
            let scrolled = scrolled_to_show(
                tile.top * pixels_per_token,
                (tile.top + tile.height) * pixels_per_token,
                -f32::from(self.scroll.offset().y),
                visible,
            );
            self.scroll.set_offset(point(px(0.0), px(-scrolled)));
        }
        cx.notify();
    }
}

// ---- Drawing ----

impl GifPicker {
    #[allow(clippy::too_many_lines)] // One tile and its star, as the web's markup.
    fn render_tile(
        &self,
        gif: &KlipyGif,
        gif_index: usize,
        bounds: TileBounds,
        is_picture_wanted: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let title: SharedString = if gif.title.is_empty() {
            catalogue.gifs().untitled().into()
        } else {
            gif.title.clone().into()
        };
        let is_starred = self
            .favourite_slugs
            .as_ref()
            .is_some_and(|slugs| slugs.contains(&gif.slug));
        let is_active = gif_index == self.active_index;
        let is_grid_ring_shown = is_active && is_focus_visible(&self.grid_focus, window);
        let is_star_focused = is_active && self.star_focus.is_focused(window);
        let is_star_shown = is_starred
            || self.hovered_index == Some(gif_index)
            || (is_active && (self.grid_focus.is_focused(window) || is_star_focused));
        let animation = picture_animation(cx);
        let picture = is_picture_wanted
            .then(|| tile_picture(gif, animation))
            .flatten()
            .map(|media| {
                img(picture_for_box(
                    PictureFile::Klipy(media.url.clone().into()),
                    bounds.width,
                    bounds.height,
                    animation,
                    window,
                ))
                .size_full()
                .object_fit(ObjectFit::Cover)
            });
        let star_label: SharedString = if is_starred {
            catalogue.gifs().remove_favourite(&title)
        } else {
            catalogue.gifs().add_favourite(&title)
        }
        .into();
        let slug = gif.slug.clone();
        let star_ink = color(if is_starred {
            tokens.component_gif_star_icon_on
        } else {
            tokens.component_gif_star_icon
        });
        let star = is_star_shown.then(|| {
            div()
                .id(ElementId::named_usize("gif-star", gif_index))
                .role(Role::Button)
                .aria_label(star_label.clone())
                .aria_toggled(if is_starred {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .when(is_active, |star| star.track_focus(&self.star_focus))
                .absolute()
                .top(length(COMPONENT_GIF_STAR_INSET))
                .right(length(COMPONENT_GIF_STAR_INSET))
                .flex()
                .items_center()
                .justify_center()
                .size(length(COMPONENT_GIF_STAR_BUTTON_SIZE))
                .rounded_full()
                .bg(color(tokens.component_gif_star_background))
                .cursor_pointer()
                .tooltip(move |_, cx| crate::components::Tooltip::view(star_label.clone(), cx))
                .on_click(cx.listener(move |picker, _, _, cx| {
                    cx.stop_propagation();
                    picker.toggle_favourite(slug.clone(), cx);
                }))
                .on_key_down(cx.listener(move |picker, event: &KeyDownEvent, _, cx| {
                    let key = event.keystroke.key.as_str();
                    if (key == "enter" || key == "space") && !event.keystroke.modifiers.modified() {
                        cx.stop_propagation();
                        let slug = gifs_of(&picker.items)
                            .nth(gif_index)
                            .map(|gif| gif.slug.clone());
                        if let Some(slug) = slug {
                            picker.toggle_favourite(slug, cx);
                        }
                    }
                }))
                .child(icon(
                    if is_starred {
                        IconName::Star
                    } else {
                        IconName::StarOutline
                    },
                    COMPONENT_GIF_STAR_ICON_SIZE,
                    star_ink,
                ))
                .when(
                    is_star_focused && window.last_input_was_keyboard(),
                    |star| star.child(tile_focus_ring(true, cx)),
                )
        });
        div()
            .id(ElementId::named_usize("gif-tile", gif_index))
            .role(Role::Button)
            .aria_label(title.clone())
            .when(
                is_active,
                gpui::StatefulInteractiveElement::aria_active_descendant,
            )
            .relative()
            .flex_none()
            .w(length(bounds.width))
            .h(length(bounds.height))
            .overflow_hidden()
            .rounded(length(COMPONENT_GIF_TILE_RADIUS))
            .bg(color(tokens.component_gif_tile_placeholder))
            .cursor_pointer()
            .tooltip(move |_, cx| crate::components::Tooltip::view(title.clone(), cx))
            .on_hover(cx.listener(move |picker, is_hovered: &bool, _, cx| {
                let hovered = if *is_hovered {
                    Some(gif_index)
                } else {
                    picker.hovered_index.filter(|hovered| *hovered != gif_index)
                };
                if hovered != picker.hovered_index {
                    picker.hovered_index = hovered;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |picker, _, _, cx| picker.pick(gif_index, cx)))
            .children(picture)
            .children(star)
            .when(is_grid_ring_shown, |tile| {
                tile.child(tile_focus_ring(false, cx))
            })
            .into_any_element()
    }

    fn render_ad(index: usize, bounds: TileBounds, cx: &App) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        // Klipy's ads are HTML for a web view, never rendered here: the tile keeps its place.
        with_type(div(), TYPE_BODY_SM)
            .id(ElementId::named_usize("gif-ad", index))
            .role(Role::Image)
            .aria_label(catalogue.gifs().sponsored_label())
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .w(length(bounds.width))
            .h(length(bounds.height))
            .rounded(length(COMPONENT_GIF_TILE_RADIUS))
            .bg(color(tokens.component_gif_tile_placeholder))
            .text_color(color(tokens.component_gif_ad_text))
            .child(catalogue.gifs().sponsored())
            .into_any_element()
    }

    /// Rows of tiles from `placements`, each row a child of the grid.
    fn rows_of(placements: &[TileBounds], mut tiles: Vec<AnyElement>) -> Vec<AnyElement> {
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut current: Vec<AnyElement> = Vec::new();
        let mut current_top = None;
        for (bounds, tile) in placements.iter().zip(tiles.drain(..)) {
            if current_top.is_some_and(|top: f32| (top - bounds.top).abs() > f32::EPSILON) {
                rows.push(Self::row(std::mem::take(&mut current)));
            }
            current_top = Some(bounds.top);
            current.push(tile);
        }
        if !current.is_empty() {
            rows.push(Self::row(current));
        }
        rows
    }

    fn row(tiles: Vec<AnyElement>) -> AnyElement {
        div()
            .flex()
            .flex_none()
            .gap(length(COMPONENT_GIF_TILE_GAP))
            .h(length(COMPONENT_GIF_ROW_HEIGHT))
            .children(tiles)
            .into_any_element()
    }

    fn render_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let status_text = color(tokens.component_gif_status_text);
        match self.status {
            SheetStatus::Loading => {
                let placements = justified_rows(
                    &PLACEHOLDER_ASPECT_RATIOS,
                    Self::sheet_width(window),
                    COMPONENT_GIF_ROW_HEIGHT,
                    COMPONENT_GIF_TILE_GAP,
                );
                let blocks = placements
                    .iter()
                    .map(|bounds| {
                        div()
                            .flex_none()
                            .w(length(bounds.width))
                            .h(length(bounds.height))
                            .rounded(length(COMPONENT_GIF_TILE_RADIUS))
                            .bg(color(tokens.component_gif_tile_placeholder))
                            .into_any_element()
                    })
                    .collect();
                div()
                    .id("gif-loading")
                    .role(Role::Status)
                    .aria_label(catalogue.gifs().loading())
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_GIF_TILE_GAP))
                    .children(Self::rows_of(&placements, blocks))
                    .into_any_element()
            }
            SheetStatus::Failed(failure) => div()
                .id("gif-failure")
                .role(Role::Alert)
                .flex()
                .flex_col()
                .items_start()
                .gap(length(SPACE_XS))
                .p(length(SPACE_XS))
                .child(
                    with_type(div(), TYPE_BODY)
                        .text_color(status_text)
                        .child(failure_sentence(failure, catalogue)),
                )
                .when(
                    !matches!(failure, GifFailure::Offline | GifFailure::Refused),
                    |failure| {
                        let picker = cx.entity().downgrade();
                        failure.child(Button::new("gif-retry", catalogue.gifs().retry()).on_click(
                            move |_, cx| {
                                let _ = picker.update(cx, GifPicker::load_first_page);
                            },
                        ))
                    },
                )
                .into_any_element(),
            SheetStatus::Shown if self.items.is_empty() => {
                let sentence = if self.is_searching() {
                    catalogue.gifs().no_results(&self.searched_query)
                } else if self.tab == GifTab::Favourites {
                    catalogue.gifs().no_favourites().to_owned()
                } else {
                    catalogue.gifs().nothing_trending().to_owned()
                };
                with_type(div(), TYPE_BODY)
                    .id("gif-empty")
                    .role(Role::Status)
                    .p(length(SPACE_XS))
                    .text_color(status_text)
                    .child(sentence)
                    .into_any_element()
            }
            SheetStatus::Shown => self.render_tiles(window, cx),
        }
    }

    #[allow(clippy::too_many_lines)] // The sheet, then its next-page line, as the web's markup.
    fn render_tiles(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let placements = self.placements(window);
        // Which rows are near the visible part, in token pixels: only they load pictures.
        let token_pixels_per_pixel = TOKEN_PIXELS_PER_REM / f32::from(window.rem_size());
        let scrolled = -f32::from(self.scroll.offset().y) * token_pixels_per_pixel;
        let visible_height = f32::from(self.scroll.bounds().size.height) * token_pixels_per_pixel;
        let visible_height = if visible_height > 0.0 {
            visible_height
        } else {
            COMPONENT_GIF_PICKER_HEIGHT
        };
        let margin = visible_height * PICTURE_LOAD_MARGIN_IN_SHEETS;
        let is_near = |bounds: &TileBounds| {
            bounds.top + bounds.height >= scrolled - margin
                && bounds.top <= scrolled + visible_height + margin
        };
        let items = self.items.clone();
        let mut gif_index = 0;
        let tiles: Vec<AnyElement> = items
            .iter()
            .zip(&placements)
            .enumerate()
            .map(|(index, (item, bounds))| match item {
                KlipyItem::Gif(gif) => {
                    let tile =
                        self.render_tile(gif, gif_index, *bounds, is_near(bounds), window, cx);
                    gif_index += 1;
                    tile
                }
                KlipyItem::Ad(_) => Self::render_ad(index, *bounds, cx),
            })
            .collect();

        // The end in view asks for the next page by itself; "More GIFs" stays for keyboards.
        let remaining = sheet_height(&placements) - scrolled - visible_height;
        if self.has_next
            && !self.is_loading_more
            && self.more_failure.is_none()
            && remaining < visible_height * NEXT_PAGE_DISTANCE_IN_SHEETS
        {
            cx.defer_in(window, |picker, _, cx| picker.load_more(cx));
        }
        let status_text = color(tokens.component_gif_status_text);
        let more: Option<AnyElement> = if let Some(failure) = self.more_failure {
            let picker = cx.entity().downgrade();
            Some(
                div()
                    .id("gif-more-failure")
                    .role(Role::Alert)
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(length(SPACE_XS))
                    .p(length(SPACE_XS))
                    .child(
                        with_type(div(), TYPE_BODY)
                            .text_color(status_text)
                            .child(failure_sentence(failure, catalogue)),
                    )
                    .when(
                        !matches!(failure, GifFailure::Offline | GifFailure::Refused),
                        |line| {
                            line.child(
                                Button::new("gif-more-retry", catalogue.gifs().retry())
                                    .variant(crate::components::ButtonVariant::Ghost)
                                    .on_click(move |_, cx| {
                                        let _ = picker.update(cx, |picker, cx| {
                                            picker.more_failure = None;
                                            picker.load_more(cx);
                                        });
                                    }),
                            )
                        },
                    )
                    .into_any_element(),
            )
        } else if self.has_next {
            let picker = cx.entity().downgrade();
            Some(
                div()
                    .mt(length(COMPONENT_GIF_TILE_GAP))
                    .child(
                        Button::new(
                            "gif-more",
                            if self.is_loading_more {
                                catalogue.gifs().loading()
                            } else {
                                catalogue.gifs().load_more()
                            },
                        )
                        .variant(crate::components::ButtonVariant::Ghost)
                        .disabled(self.is_loading_more)
                        .full_width()
                        .on_click(move |_, cx| {
                            let _ = picker.update(cx, GifPicker::load_more);
                        }),
                    )
                    .into_any_element(),
            )
        } else {
            None
        };
        let mut children = Self::rows_of(&placements, tiles);
        children.extend(more);
        div()
            .id("gif-tiles")
            .role(Role::Grid)
            .aria_label(if self.is_searching() {
                catalogue.gifs().results(&self.searched_query)
            } else if self.tab == GifTab::Favourites {
                catalogue.gifs().favourites().to_owned()
            } else {
                catalogue.gifs().trending().to_owned()
            })
            .track_focus(&self.grid_focus)
            .on_key_down(cx.listener(Self::on_grid_key_down))
            .flex()
            .flex_col()
            .gap(length(COMPONENT_GIF_TILE_GAP))
            .children(children)
            .into_any_element()
    }
}

/// The stage's sky ring (`component.gif.tile-focus-ring`), drawn inside a tile so the sheet does
/// not clip it, or around the round star.
fn tile_focus_ring(is_round: bool, cx: &App) -> impl IntoElement {
    let tokens = theme(cx);
    div()
        .absolute()
        .inset_0()
        .when(is_round, gpui::Styled::rounded_full)
        .when(!is_round, |ring| {
            ring.rounded(length(COMPONENT_GIF_TILE_RADIUS))
        })
        .border(length(SIZE_FOCUS_RING_WIDTH))
        .border_color(color(tokens.component_gif_tile_focus_ring))
}

impl Render for GifPicker {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let (width, height) = Self::panel_size(window);
        let viewport = window.viewport_size();
        let token_pixels_per_pixel = TOKEN_PIXELS_PER_REM / f32::from(window.rem_size());
        let to_tokens = |side: f32| side * token_pixels_per_pixel;
        // The anchor is in window pixels; the placement works in token pixels, then back.
        let anchor = MenuAnchor {
            top: to_tokens(self.anchor.top),
            bottom: to_tokens(self.anchor.bottom),
            left: to_tokens(self.anchor.left),
            right: to_tokens(self.anchor.right),
        };
        let (top, left) = popover_position_beside_anchor(
            anchor,
            (width, height),
            (
                to_tokens(f32::from(viewport.width)),
                to_tokens(f32::from(viewport.height)),
            ),
            COMPONENT_GIF_PICKER_GAP,
        );
        let is_search_focused = self.search.read(cx).focus_handle(cx).is_focused(window);
        let sheet = self.render_sheet(window, cx);
        let tabs = (!self.is_searching()).then(|| {
            let picker = cx.entity().downgrade();
            Tabs::new(
                "gif-tabs",
                catalogue.gifs().lists(),
                vec![
                    catalogue.gifs().favourites().into(),
                    catalogue.gifs().trending().into(),
                ],
                usize::from(self.tab == GifTab::Trending),
            )
            .on_select(move |index, _, cx| {
                let tab = if index == 0 {
                    GifTab::Favourites
                } else {
                    GifTab::Trending
                };
                let _ = picker.update(cx, |picker, cx| picker.show_tab(tab, cx));
            })
        });
        let card = div()
            .id("gif-picker")
            .role(Role::Dialog)
            .aria_label(catalogue.gifs().label())
            .aria_description(catalogue.gifs().help())
            .track_focus(&self.container_focus)
            .key_context("GifPicker")
            .capture_key_down(cx.listener(Self::on_container_key_down))
            .on_action(cx.listener(|picker, _: &FocusNextControl, window, cx| {
                cx.stop_propagation();
                focus_next_inside(&picker.container_focus, window, cx);
            }))
            .on_action(cx.listener(|picker, _: &FocusPreviousControl, window, cx| {
                cx.stop_propagation();
                focus_previous_inside(&picker.container_focus, window, cx);
            }))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| {
                cx.emit(GifPickerEvent::Dismissed { by_pointer: true });
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(length(width))
            .h(length(height))
            .flex()
            .flex_col()
            .gap(length(COMPONENT_GIF_PICKER_GAP))
            .p(length(COMPONENT_GIF_PICKER_PADDING))
            .rounded(length(COMPONENT_GIF_PICKER_RADIUS))
            .border(length(COMPONENT_GIF_PICKER_BORDER_WIDTH))
            .border_color(color(tokens.component_gif_picker_border))
            .bg(color(tokens.component_gif_picker_background))
            .shadow(shadow(tokens.component_gif_picker_shadow))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(length(SPACE_XS))
                    .h(length(
                        COMPONENT_EMOJI_PICKER_SEARCH_HEIGHT.max(SIZE_HIT_DESKTOP_MIN),
                    ))
                    .px(length(SPACE_XS))
                    .rounded(length(RADIUS_SM))
                    .bg(color(tokens.component_emoji_picker_search_background))
                    .capture_key_down(cx.listener(Self::on_search_key_down))
                    .child(icon(
                        IconName::MagnifyingGlass,
                        COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE,
                        color(tokens.component_emoji_picker_search_placeholder),
                    ))
                    .child(
                        with_type(div(), TYPE_BODY)
                            .flex_1()
                            .min_w_0()
                            .text_color(color(tokens.component_emoji_picker_search_text))
                            .child(
                                Input::new(&self.search)
                                    .appearance(false)
                                    .aria_label(catalogue.gifs().search()),
                            ),
                    )
                    .when(
                        is_search_focused && window.last_input_was_keyboard(),
                        |field| field.child(focus_ring(RADIUS_SM, cx)),
                    ),
            )
            .children(tabs.map(|tabs| div().flex_none().px(length(SPACE_2XS)).child(tabs)))
            .child(
                div()
                    .id("gif-sheet")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(sheet),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(length(SPACE_XS))
                    .px(length(SPACE_2XS))
                    .child(
                        with_type(div(), TYPE_BODY_SM)
                            .id("gif-favourite-status")
                            .role(Role::Status)
                            .flex_1()
                            .min_w_0()
                            .text_color(color(tokens.component_gif_status_text))
                            .child(self.favourite_message.clone()),
                    )
                    .child(
                        with_type(div(), TYPE_BODY_SM)
                            .flex_none()
                            .text_color(color(tokens.component_gif_credit_text))
                            .child(catalogue.gifs().powered_by()),
                    ),
            )
            .child(
                div()
                    .id("gif-announcement")
                    .role(Role::Status)
                    .when(!self.announcement.is_empty(), |region| {
                        region.aria_value(self.announcement.clone())
                    })
                    // Read politely, as the conversation's announcements are.
                    .a11y_synthetic_children(|subtree| {
                        subtree
                            .parent_node()
                            .set_live(gpui::accesskit::Live::Polite);
                    })
                    .size_0()
                    .overflow_hidden(),
            );
        let margin = px(COMPONENT_CONTEXT_MENU_VIEWPORT_MARGIN);
        deferred(
            anchored()
                .position(point(
                    px(left / token_pixels_per_pixel),
                    px(top / token_pixels_per_pixel),
                ))
                .snap_to_window_with_margin(margin)
                .child(card),
        )
        .with_priority(1)
    }
}

#[cfg(test)]
impl GifPicker {
    /// The sheet as if Klipy had answered with `items`.
    pub(crate) fn show_items(&mut self, items: Vec<KlipyItem>, cx: &mut Context<Self>) {
        self.request_generation += 1;
        self.in_flight = None;
        self.items = items;
        self.has_next = false;
        self.status = SheetStatus::Shown;
        cx.notify();
    }
}

impl gpui::Focusable for GifPicker {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.container_focus.clone()
    }
}
