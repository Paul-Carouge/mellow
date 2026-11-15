//! The two ways a person's profile shows (`profile-surfaces.svelte.ts`, DESIGN.md §5.7.1 to
//! §5.7.3): the hover card, after the pointer or keyboard focus rests on a name, and the profile
//! modal, on a click. Both are drawn once by the shell, outside the rows that open them: a row's
//! own handlers, keys and menus never reach them, and a row that leaves (a member gone, a message
//! deleted) does not take an open profile with it.
//!
//! The shell owns the one [`ProfileSurfaces`]; a global keeps only a weak handle to it, so the
//! names in the message list (another view) reach it, and it dies with its shell, its window and
//! its session.

mod facts;
mod hover_card;
mod modal;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, Context, ElementId, Entity, EventEmitter, FocusHandle, Global, IntoElement,
    ParentElement as _, Pixels, Render, Subscription, Task, WeakEntity, Window, div,
};
use mellow_client_core::ClientError;
use mellow_client_core::shared_logic::markdown::parse_inline_markdown;
#[cfg(test)]
use mellow_client_core::shared_logic::profiles::UserProfileLoad;
use mellow_client_core::shared_logic::profiles::UserProfileLoader;
use mellow_protocol::{PublicUser, Snowflake, UserProfile};

pub(crate) use self::hover_card::HoverCardPlacement;
use self::modal::ProfileModal;
use crate::components::child_id;
use crate::home::PersonMenuRequest;
use crate::messages::MessageBody;
use crate::session::Session;
use crate::theme::delay;
use crate::theme::tokens::{
    COMPONENT_PROFILE_HOVER_CARD_DETAILS_DELAY, MOTION_DELAY_HOVER_CARD,
    MOTION_DELAY_HOVER_CARD_CLOSE,
};

/// About me and links are kept a minute (`readRecentUserProfile`): no gateway event says they
/// changed (ADR-0050).
const RECENT_PROFILE_LIFETIME: Duration = Duration::from_secs(60);
/// Profiles kept at most; the oldest goes first.
const MAXIMUM_RECENT_PROFILES: usize = 50;

/// A name or row that shows a hover card.
#[derive(Clone)]
pub(crate) struct ProfileHoverTrigger {
    pub user: PublicUser,
    pub placement: HoverCardPlacement,
    /// Where the trigger is drawn (`remembered_bounds`): it lives only while the trigger is drawn,
    /// so a wait that ends on a trigger gone opens nothing.
    pub anchor: WeakEntity<Option<Bounds<Pixels>>>,
    /// The column a "beside" card keeps clear of (the side sheet).
    pub column: Option<WeakEntity<Option<Bounds<Pixels>>>>,
    /// Gets focus back when the modal opened from this card closes.
    pub opener: Option<FocusHandle>,
}

impl ProfileHoverTrigger {
    /// The same trigger: re-entering it keeps its card. A text's mention chips share one anchor,
    /// told apart by whom they name.
    fn is_same(&self, other: &Self) -> bool {
        self.anchor.entity_id() == other.anchor.entity_id() && self.user.id == other.user.id
    }

    fn is_still_drawn(&self) -> bool {
        self.anchor.upgrade().is_some()
    }
}

/// What the profiles ask the shell to do.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ProfileSurfacesEvent {
    /// "Send message" opened this conversation: show it, its composer focused.
    OpenedConversation(Snowflake),
    /// Your own profile's "Edit profile": User settings on Profile.
    EditProfile,
    /// The modal's More: the person menu, without what the modal's buttons do.
    OpenPersonMenu(PersonMenuRequest),
}

/// About me and links read lately, for the account that read them.
#[derive(Default)]
struct RecentProfiles {
    reader_id: Option<Snowflake>,
    profiles: HashMap<Snowflake, (Instant, UserProfile)>,
    /// Oldest first, to bound the map.
    order: VecDeque<Snowflake>,
}

impl RecentProfiles {
    /// A profile read less than a minute ago by `reader_id`; the reader's own is always read again
    /// (they may just have edited it).
    fn get(
        &mut self,
        user_id: Snowflake,
        reader_id: Option<Snowflake>,
        now: Instant,
    ) -> Option<UserProfile> {
        if reader_id != self.reader_id {
            *self = Self {
                reader_id,
                ..Self::default()
            };
        }
        if Some(user_id) == reader_id {
            return None;
        }
        self.profiles
            .get(&user_id)
            .filter(|(read_at, _)| {
                now.saturating_duration_since(*read_at) < RECENT_PROFILE_LIFETIME
            })
            .map(|(_, profile)| profile.clone())
    }

    fn remember(&mut self, user_id: Snowflake, profile: UserProfile, now: Instant) {
        self.order.retain(|kept| *kept != user_id);
        self.order.push_back(user_id);
        self.profiles.insert(user_id, (now, profile));
        while self.order.len() > MAXIMUM_RECENT_PROFILES {
            if let Some(oldest) = self.order.pop_front() {
                self.profiles.remove(&oldest);
            }
        }
    }
}

/// The hover card: shown, waiting to show or close, and its About me.
#[derive(Default)]
struct HoverCard {
    shown: Option<ProfileHoverTrigger>,
    /// The running wait, and what it shows when it ends (`None` closes).
    wait: Option<(Task<()>, Option<ProfileHoverTrigger>)>,
    details: UserProfileLoader,
    details_wait: Option<Task<()>>,
    /// Its height last frame, to place it above its name when there is no room below.
    measured_height: Rc<std::cell::Cell<Option<Pixels>>>,
    /// The window's size when it opened: a resize closes it.
    window_size: Option<gpui::Size<Pixels>>,
    /// Whether it was drawn once measured, so placed and visible.
    #[cfg(test)]
    is_drawn_measured: bool,
}

pub(crate) struct ProfileSurfaces {
    session: Entity<Session>,
    /// The server open in the shell: its roles and join date show in profiles.
    guild_id: Option<Snowflake>,
    hover: HoverCard,
    modal: Option<ProfileModal>,
    /// The open modal's About me and links.
    modal_details: UserProfileLoader,
    recent_profiles: RecentProfiles,
    /// "About me" parsed once per text, for the card and the modal.
    parsed_about_me: Option<(String, Rc<MessageBody>)>,
    /// Focus the modal gave back as it closed: that row must not open its hover card again.
    returning_focus: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ProfileSurfacesEvent> for ProfileSurfaces {}

/// The shell's surfaces, reached by names drawn in other views.
struct ProfileSurfacesHost(WeakEntity<ProfileSurfaces>);

impl Global for ProfileSurfacesHost {}

fn surfaces(cx: &App) -> Option<Entity<ProfileSurfaces>> {
    cx.try_global::<ProfileSurfacesHost>()?.0.upgrade()
}

/// Whether the profile modal covers the shell: the shell's shortcuts wait until it closes, since
/// GPUI runs key bindings before the modal's key handler.
#[must_use]
/// Whether a hover card shows: Esc closes it first, before any other use of the key.
pub(crate) fn is_profile_hover_card_shown(cx: &App) -> bool {
    surfaces(cx).is_some_and(|surfaces| surfaces.read(cx).hover.shown.is_some())
}

pub(crate) fn is_profile_modal_open(cx: &App) -> bool {
    surfaces(cx).is_some_and(|surfaces| surfaces.read(cx).modal.is_some())
}

/// Opens `user`'s profile modal (replacing one open); focus goes back to `opener`, or to what has
/// focus now, when it closes.
pub(crate) fn open_profile_modal(
    user: PublicUser,
    opener: Option<FocusHandle>,
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(surfaces) = surfaces(cx) {
        surfaces.update(cx, |surfaces, cx| {
            surfaces.open_modal(user, opener, window, cx);
        });
    }
}

/// The pointer entered a trigger, or keyboard focus landed on one.
pub(crate) fn enter_profile_trigger(trigger: ProfileHoverTrigger, cx: &mut App) {
    if let Some(surfaces) = surfaces(cx) {
        surfaces.update(cx, |surfaces, cx| surfaces.enter_trigger(trigger, cx));
    }
}

/// The pointer or focus left a trigger.
pub(crate) fn leave_profile_trigger(left: &ProfileHoverTrigger, cx: &mut App) {
    if let Some(surfaces) = surfaces(cx) {
        surfaces.update(cx, |surfaces, cx| surfaces.leave_trigger(left, cx));
    }
}

/// A press, a menu, a scroll: the card goes at once.
pub(crate) fn close_profile_hover_card(cx: &mut App) {
    if let Some(surfaces) = surfaces(cx) {
        surfaces.update(cx, ProfileSurfaces::close_hover_card);
    }
}

/// Shows `trigger`'s card when keyboard focus rests on the row `id` (not a click's focus, nor the
/// focus the modal gives back). The listeners live while the row is drawn.
pub(crate) fn follow_keyboard_focus(
    id: &ElementId,
    focus_handle: &FocusHandle,
    trigger: ProfileHoverTrigger,
    window: &mut Window,
    cx: &mut App,
) {
    let focus_handle = focus_handle.clone();
    window.use_keyed_state(
        child_id(id, "profile-trigger-focus"),
        cx,
        move |window, cx| {
            let focused = focus_handle.clone();
            let left = trigger.clone();
            [
                window.on_focus_in(&focus_handle, cx, move |window, cx| {
                    let Some(surfaces) = surfaces(cx) else {
                        return;
                    };
                    let is_returned = surfaces.update(cx, |surfaces, _| {
                        surfaces
                            .returning_focus
                            .take()
                            .is_some_and(|returned| returned == focused)
                    });
                    if window.last_input_was_keyboard() && !is_returned {
                        enter_profile_trigger(trigger.clone(), cx);
                    }
                }),
                window.on_focus_out(&focus_handle, cx, move |_, _, cx| {
                    leave_profile_trigger(&left, cx);
                }),
            ]
        },
    );
}

impl ProfileSurfaces {
    /// Made by the shell, which keeps it; the names of other views reach it through a weak handle.
    pub(crate) fn new(
        session: Entity<Session>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let host = ProfileSurfacesHost(cx.entity().downgrade());
        cx.set_global(host);
        let surfaces = cx.entity().downgrade();
        let subscriptions = vec![
            // Esc closes a shown card wherever focus is (WCAG 1.4.13), and only the card. Before
            // key bindings: the composer's Esc would cancel an edit, the emoji search would clear.
            cx.intercept_keystrokes(move |event, _, cx| {
                let keystroke = &event.keystroke;
                if keystroke.key != "escape" || keystroke.modifiers.modified() {
                    return;
                }
                // The media viewer's Esc is its own; no card shows under it anyway.
                if crate::media_viewer::is_media_viewer_open(cx) {
                    return;
                }
                let _ = surfaces.update(cx, |surfaces, cx| {
                    if surfaces.hover.shown.is_some() {
                        cx.stop_propagation();
                        surfaces.close_hover_card(cx);
                    }
                });
            }),
            // Live: presence, roles, relationship and pictures follow the state.
            cx.observe(&session, |_, _, cx| cx.notify()),
            // Leaving the window closes the card (DESIGN.md §5.7.2).
            cx.observe_window_activation(window, |surfaces, window, cx| {
                if !window.is_window_active() {
                    surfaces.close_hover_card(cx);
                }
            }),
        ];
        Self {
            session,
            guild_id: None,
            hover: HoverCard::default(),
            modal: None,
            modal_details: UserProfileLoader::default(),
            recent_profiles: RecentProfiles::default(),
            parsed_about_me: None,
            returning_focus: None,
            _subscriptions: subscriptions,
        }
    }

    /// The shell opened another place: profiles show that server's roles and join date.
    pub(crate) fn set_guild(&mut self, guild_id: Option<Snowflake>, cx: &mut Context<Self>) {
        if self.guild_id != guild_id {
            self.guild_id = guild_id;
            cx.notify();
        }
    }

    /// Whose profile the modal shows.
    #[cfg(test)]
    pub(crate) fn modal_user_id(&self) -> Option<Snowflake> {
        self.modal.as_ref().map(|modal| modal.user.id)
    }

    /// Whose hover card shows.
    #[cfg(test)]
    pub(crate) fn hover_card_user_id(&self) -> Option<Snowflake> {
        self.hover.shown.as_ref().map(|shown| shown.user.id)
    }

    #[cfg(test)]
    pub(crate) fn hover_card_details(&self) -> &UserProfileLoad {
        self.hover.details.current()
    }

    // ---- The hover card (`HoverCardController`) ----

    fn enter_trigger(&mut self, trigger: ProfileHoverTrigger, cx: &mut Context<Self>) {
        // Nothing opens under the profile modal or the media viewer (this entity is being
        // updated: `shell::is_modal_layer_open` would read it again).
        if self.modal.is_some() || crate::media_viewer::is_media_viewer_open(cx) {
            return;
        }
        if self
            .hover
            .shown
            .as_ref()
            .is_some_and(|shown| shown.is_same(&trigger))
        {
            self.hover.wait = None;
            return;
        }
        // Already waiting for this one: the wait goes on.
        if let Some((_, Some(pending))) = &self.hover.wait
            && pending.is_same(&trigger)
        {
            return;
        }
        // A card open switches after the close grace: no close-and-reopen flash.
        let wait = if self.hover.shown.is_none() {
            MOTION_DELAY_HOVER_CARD
        } else {
            MOTION_DELAY_HOVER_CARD_CLOSE
        };
        self.wait(delay(wait), Some(trigger), cx);
    }

    /// The pointer or focus left `left`. A wait for another trigger goes on: entering the next
    /// row may come before leaving this one.
    fn leave_trigger(&mut self, left: &ProfileHoverTrigger, cx: &mut Context<Self>) {
        if let Some((_, Some(pending))) = &self.hover.wait
            && !pending.is_same(left)
        {
            return;
        }
        if self.hover.shown.is_none() {
            self.hover.wait = None;
            return;
        }
        self.wait(delay(MOTION_DELAY_HOVER_CARD_CLOSE), None, cx);
    }

    /// The pointer left the card: it closes after the grace, unless a switch is on its way.
    fn leave_card(&mut self, cx: &mut Context<Self>) {
        if self.hover.shown.is_none() || self.hover.wait.is_some() {
            return;
        }
        self.wait(delay(MOTION_DELAY_HOVER_CARD_CLOSE), None, cx);
    }

    /// The pointer reached the card: it stays, and a switch begun on the way is dropped.
    fn enter_card(&mut self) {
        if self.hover.shown.is_some() {
            self.hover.wait = None;
        }
    }

    pub(crate) fn close_hover_card(&mut self, cx: &mut Context<Self>) {
        self.hover.wait = None;
        self.show_hover_card(None, cx);
    }

    fn wait(
        &mut self,
        duration: Duration,
        next: Option<ProfileHoverTrigger>,
        cx: &mut Context<Self>,
    ) {
        let task = cx.spawn(async move |surfaces, cx| {
            cx.background_executor().timer(duration).await;
            let _ = surfaces.update(cx, |surfaces, cx| {
                let Some((task, next)) = surfaces.hover.wait.take() else {
                    return;
                };
                // This very task is running: let it finish rather than cancel it from inside.
                task.detach();
                // Switching to a trigger gone closes the card rather than leaving the old one up.
                let next = next.filter(ProfileHoverTrigger::is_still_drawn);
                surfaces.show_hover_card(next, cx);
            });
        });
        self.hover.wait = Some((task, next));
    }

    fn show_hover_card(&mut self, next: Option<ProfileHoverTrigger>, cx: &mut Context<Self>) {
        let is_same = match (&self.hover.shown, &next) {
            (Some(shown), Some(next)) => shown.is_same(next),
            (None, None) => true,
            _ => false,
        };
        if is_same {
            return;
        }
        let user_id = next.as_ref().map(|next| next.user.id);
        self.hover.shown = next;
        self.hover.window_size = None;
        self.hover.measured_height.set(None);
        self.hover.details.show(None);
        self.hover.details_wait = user_id.map(|user_id| {
            // About me is read only once the card has stayed open a moment: sweeping the member
            // list reads nothing for the cards it only crossed (the instance limits profile reads).
            cx.spawn(async move |surfaces, cx| {
                cx.background_executor()
                    .timer(delay(COMPONENT_PROFILE_HOVER_CARD_DETAILS_DELAY))
                    .await;
                let _ = surfaces.update(cx, |surfaces, cx| {
                    if let Some(ticket) = surfaces.hover.details.show(Some(user_id)) {
                        surfaces.read_profile(user_id, cx, move |surfaces, answer, cx| {
                            if surfaces.hover.details.finish(ticket, answer) {
                                cx.notify();
                            }
                        });
                    }
                });
            })
        });
        cx.notify();
    }

    // ---- The modal ----

    fn open_modal(
        &mut self,
        user: PublicUser,
        opener: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Under the media viewer nothing else opens.
        if crate::media_viewer::is_media_viewer_open(cx) {
            return;
        }
        self.close_hover_card(cx);
        let opener = opener.or_else(|| window.focused(cx));
        let user_id = user.id;
        let modal = ProfileModal::new(user, opener, cx);
        window.focus(&modal.primary_focus, cx);
        self.modal = Some(modal);
        // One loader for every modal: its generation tells a late answer for the profile shown
        // before from this one's.
        if let Some(ticket) = self.modal_details.show(Some(user_id)) {
            self.read_profile(user_id, cx, move |surfaces, answer, cx| {
                if surfaces.modal_details.finish(ticket, answer) {
                    cx.notify();
                }
            });
        }
        cx.notify();
    }

    /// Esc, Close, the scrim, or an action that leaves: focus goes back to what opened it, without
    /// opening that row's hover card again.
    pub(crate) fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.modal.take() else {
            return;
        };
        self.modal_details.show(None);
        if let Some(opener) = modal.opener {
            self.returning_focus = Some(opener.clone());
            window.focus(&opener, cx);
        }
        cx.notify();
    }

    // ---- About me and links ----

    /// Reads `user_id`'s About me and links, or takes them from the last minute's reads.
    fn read_profile(
        &mut self,
        user_id: Snowflake,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(&mut Self, Result<UserProfile, ClientError>, &mut Context<Self>) + 'static,
    ) {
        let reader_id = self
            .session
            .read(cx)
            .state()
            .current_user()
            .map(|user| user.id);
        let now = cx.background_executor().now();
        if let Some(profile) = self.recent_profiles.get(user_id, reader_id, now) {
            // Later, as an answer would come: the caller is still updating.
            cx.spawn(async move |surfaces, cx| {
                let _ = surfaces.update(cx, |surfaces, cx| on_done(surfaces, Ok(profile), cx));
            })
            .detach();
            return;
        }
        let surfaces = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.load_user_profile(user_id, cx, move |answer, cx| {
                let _ = surfaces.update(cx, |surfaces, cx| {
                    if let Ok(profile) = &answer {
                        let now = cx.background_executor().now();
                        surfaces
                            .recent_profiles
                            .remember(user_id, profile.clone(), now);
                    }
                    on_done(surfaces, answer, cx);
                });
            });
        });
    }

    /// "About me" parsed as inline Markdown, once per text; `None` when empty.
    fn about_body(&mut self, about_me: &str) -> Option<Rc<MessageBody>> {
        if self
            .parsed_about_me
            .as_ref()
            .is_none_or(|(text, _)| text != about_me)
        {
            let body = MessageBody {
                blocks: parse_inline_markdown(about_me),
                is_jumbo: false,
                names_reader: false,
            };
            self.parsed_about_me = Some((about_me.to_owned(), Rc::new(body)));
        }
        self.parsed_about_me
            .as_ref()
            .filter(|(_, body)| !body.blocks.is_empty())
            .map(|(_, body)| Rc::clone(body))
    }
}

impl Render for ProfileSurfaces {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let card = self.render_hover_card(window, cx);
        let modal = self.render_modal(window, cx);
        div().children(card).children(modal)
    }
}
