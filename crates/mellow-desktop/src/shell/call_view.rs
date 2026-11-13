//! The call view and the watched screen (`CallView.svelte`, `CallTile.svelte`,
//! `StreamView.svelte`, ADR-0035, ADR-0037), in the conversation sheet's place while the chat
//! stays underneath. The call: the chat header's grammar (the channel, the layout toggles, full
//! screen and Back to the chat), a dark stage with one tile per person (nine at most, the others
//! as avatars under them) and a band of call controls. The grid is the default; the speaker view
//! puts one person large, the one picked or else whoever talks, over a strip of the others; full
//! screen gives that person the whole window. A shared screen is a tile too; opening it watches
//! it large with its own sound band (mute and volume, apart from the person's voice). Esc leaves
//! full screen first, then goes back to the chat.
//!
//! Pictures come from [`VideoPictures`], which only this view observes; the watch list follows
//! the tiles drawn and their sizes in physical pixels, sent only when it changes.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Pixels, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Toggled,
    Window, canvas, div, prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::call_view::{
    CallLayout, CallPerson, CallVideo, MAXIMUM_CALL_TILES, WatchedStreamStatus, arrange_call_tiles,
    featured_person_of, grid_shape_for, next_speaker, watched_pictures_of, watched_stream_status,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::voice_settings::{
    MAXIMUM_VOLUME_PERCENT, MINIMUM_VOLUME_PERCENT,
};
use mellow_protocol::{PublicUser, Snowflake, VideoTileSize, VoiceState};

use super::profile_picture;
use crate::components::{
    Avatar, AvatarPerson, IconButton, RangeBounds, RangeField, focus_ring, icon, inset_focus_ring,
    is_focus_visible, polite_live_region, remembered_focus_handle, with_type,
};
use crate::i18n::strings;
use crate::session::Session;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CALL_VIEW_AVATAR_SIZE_LARGE, COMPONENT_CALL_VIEW_AVATAR_SIZE_SMALL,
    COMPONENT_CALL_VIEW_CONTROL_BAR_HEIGHT, COMPONENT_CALL_VIEW_CONTROL_GAP,
    COMPONENT_CALL_VIEW_CONTROL_RADIUS, COMPONENT_CALL_VIEW_CONTROL_SIZE,
    COMPONENT_CALL_VIEW_LABEL_HEIGHT, COMPONENT_CALL_VIEW_LABEL_ICON_SIZE,
    COMPONENT_CALL_VIEW_LABEL_PADDING_INLINE, COMPONENT_CALL_VIEW_LABEL_RADIUS,
    COMPONENT_CALL_VIEW_OVERFLOW_AVATAR_SIZE, COMPONENT_CALL_VIEW_SPEAKING_RING_WIDTH,
    COMPONENT_CALL_VIEW_STAGE_INSET, COMPONENT_CALL_VIEW_STAGE_PADDING,
    COMPONENT_CALL_VIEW_STAGE_RADIUS, COMPONENT_CALL_VIEW_STRIP_TILE_HEIGHT,
    COMPONENT_CALL_VIEW_TILE_GAP, COMPONENT_CALL_VIEW_TILE_RADIUS,
    COMPONENT_CHAT_HEADER_BUTTON_SIZE, COMPONENT_CHAT_HEADER_HEIGHT,
    COMPONENT_CHAT_HEADER_ICON_SIZE, COMPONENT_CHAT_HEADER_PADDING_INLINE,
    COMPONENT_LIVE_BADGE_HEIGHT, COMPONENT_LIVE_BADGE_PADDING_INLINE, COMPONENT_LIVE_BADGE_RADIUS,
    COMPONENT_SHEET_RADIUS, COMPONENT_STREAM_VIEW_AVATAR_SIZE,
    COMPONENT_STREAM_VIEW_ENDED_CLOSE_DELAY, COMPONENT_STREAM_VIEW_SLOW_CONNECT_DELAY,
    COMPONENT_STREAM_VIEW_SOUND_BAR_GAP, COMPONENT_STREAM_VIEW_SOUND_BAR_HEIGHT,
    COMPONENT_STREAM_VIEW_STAGE_INSET, COMPONENT_STREAM_VIEW_STAGE_RADIUS,
    COMPONENT_STREAM_VIEW_VOLUME_WIDTH, SPACE_2XS, SPACE_SM, SPACE_XS, TYPE_BADGE, TYPE_BODY,
    TYPE_BODY_SM, TYPE_HEADING_MD,
};
use crate::theme::{color, length, theme};
use crate::voice::video_view::{VideoFit, VideoPictures, video_view};
use crate::voice::{CameraPhase, ShownPicture, Voice};

/// 16:9, the shape of cameras and of most screens.
const TILE_ASPECT_RATIO: f32 = 16.0 / 9.0;

/// The call view's state; the shell keeps it while the call or a watched screen shows.
pub(crate) struct CallView {
    session: Entity<Session>,
    voice: Entity<Voice>,
    pictures: Entity<VideoPictures>,
    layout: CallLayout,
    picked_user_id: Option<Snowflake>,
    /// Whoever talked last stays large through silences: history, not a derivation of now.
    speaker_user_id: Option<Snowflake>,
    /// The featured tile or the watched screen fills the window.
    is_fullscreen: bool,
    /// The view put the window in full screen itself, so leaving undoes it; a window the user
    /// had already made full screen stays so.
    has_made_window_fullscreen: bool,
    /// The layout full screen replaced, restored when the watched screen it showed ends.
    layout_before_fullscreen: CallLayout,
    focus: FocusHandle,
    /// The stage's size, measured as it is drawn, for the tiles' sizes.
    stage_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The cameras last asked for, so a frame that changes nothing sends nothing.
    shown_cameras: Vec<ShownPicture>,
    /// The watched screen's size last asked for, in physical pixels.
    shown_stream_size: Option<(u32, u32)>,
    /// The watched screen's picture has not come for a while.
    is_picture_late: bool,
    late_timer: Option<Task<()>>,
    ended_timer: Option<Task<()>>,
    /// The screen these timers are about.
    timed_stream_user_id: Option<Snowflake>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for CallView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CallView")
            .field("layout", &self.layout)
            .field("picked_user_id", &self.picked_user_id)
            .field("is_fullscreen", &self.is_fullscreen)
            .finish_non_exhaustive()
    }
}

impl Focusable for CallView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// One tile's look, from the call's state.
#[allow(clippy::struct_excessive_bools)] // Independent marks of one person, read by one tile.
struct TileLook {
    person: PublicUser,
    video: Option<CallVideo>,
    is_own: bool,
    is_speaking: bool,
    is_microphone_off: bool,
    is_paused: bool,
    sentence: Option<&'static str>,
    is_small: bool,
    is_in_strip: bool,
    /// The toggle's name, whether it is on, and whether it watches a screen.
    action_label: SharedString,
    is_pressed: Option<bool>,
}

impl CallView {
    pub(crate) fn new(
        session: Entity<Session>,
        voice: Entity<Voice>,
        cx: &mut Context<Self>,
    ) -> Self {
        let pictures = voice.read(cx).video_pictures();
        let subscriptions = vec![
            cx.observe(&pictures, |_, _, cx| cx.notify()),
            cx.observe(&voice, |_, _, cx| cx.notify()),
            cx.observe(&session, |_, _, cx| cx.notify()),
        ];
        Self {
            session,
            voice,
            pictures,
            layout: CallLayout::Grid,
            picked_user_id: None,
            speaker_user_id: None,
            is_fullscreen: false,
            has_made_window_fullscreen: false,
            layout_before_fullscreen: CallLayout::Grid,
            focus: cx.focus_handle(),
            stage_bounds: Rc::new(Cell::new(None)),
            shown_cameras: Vec::new(),
            shown_stream_size: None,
            is_picture_late: false,
            late_timer: None,
            ended_timer: None,
            timed_stream_user_id: None,
            _subscriptions: subscriptions,
        }
    }

    /// The shell draws a full-screen view over everything else.
    #[must_use]
    pub(crate) fn is_fullscreen(&self) -> bool {
        self.is_fullscreen
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn layout(&self) -> CallLayout {
        self.layout
    }

    fn choose_layout(&mut self, layout: CallLayout, cx: &mut Context<Self>) {
        self.layout = layout;
        if layout == CallLayout::Grid {
            self.picked_user_id = None;
            self.is_fullscreen = false;
        }
        cx.notify();
    }

    /// A tile's toggle: puts that person forward in the speaker view, or back.
    pub(crate) fn toggle_picked(&mut self, user_id: Snowflake, cx: &mut Context<Self>) {
        if self.layout == CallLayout::Speaker && self.picked_user_id == Some(user_id) {
            self.picked_user_id = None;
        } else {
            self.layout = CallLayout::Speaker;
            self.picked_user_id = Some(user_id);
        }
        cx.notify();
    }

    /// Full screen: the featured tile or the watched screen fills the window, and the window
    /// the screen.
    pub(crate) fn toggle_fullscreen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_fullscreen {
            if self.leave_fullscreen() && window.is_fullscreen() {
                window.toggle_fullscreen();
            }
        } else {
            self.is_fullscreen = true;
            self.layout_before_fullscreen = self.layout;
            self.layout = CallLayout::Speaker;
            self.has_made_window_fullscreen = !window.is_fullscreen();
            if self.has_made_window_fullscreen {
                window.toggle_fullscreen();
            }
        }
        cx.notify();
    }

    /// Leaves the view's full screen; `true` when the window must leave its own too.
    fn leave_fullscreen(&mut self) -> bool {
        self.is_fullscreen = false;
        std::mem::take(&mut self.has_made_window_fullscreen)
    }

    /// The view put the window in full screen, so its closing must undo it.
    #[must_use]
    pub(crate) fn has_made_window_fullscreen(&self) -> bool {
        self.has_made_window_fullscreen
    }

    /// Esc: leaves full screen first, then closes the watched screen or the call view.
    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_fullscreen {
            self.toggle_fullscreen(window, cx);
            return;
        }
        self.close(cx);
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.voice.update(cx, |voice, cx| {
            if voice.watched_stream_user_id().is_some() {
                voice.watch_stream(None, cx);
            } else {
                voice.close_call_view(cx);
            }
        });
    }

    fn person_of(&self, user_id: Snowflake, guild_id: Option<Snowflake>, cx: &App) -> PublicUser {
        let state = self.session.read(cx).state();
        guild_id
            .and_then(|guild_id| {
                state
                    .members(guild_id)
                    .iter()
                    .find(|member| member.user.id == user_id)
                    .map(|member| member.user.clone())
            })
            .or_else(|| {
                state
                    .current_user_as_public_user()
                    .filter(|user| user.id == user_id)
            })
            .unwrap_or_else(|| {
                let someone = strings(cx).voice().someone().to_owned();
                PublicUser {
                    id: user_id,
                    username: someone.clone(),
                    display_name: someone,
                    avatar: None,
                    banner: None,
                }
            })
    }

    /// Sends the cameras shown and their sizes once the frame is drawn (never while drawing).
    fn watch(&mut self, cameras: Vec<ShownPicture>, cx: &mut Context<Self>) {
        if cameras == self.shown_cameras {
            return;
        }
        self.shown_cameras.clone_from(&cameras);
        let voice = self.voice.clone();
        cx.defer(move |cx| voice.update(cx, |voice, cx| voice.watch_cameras(cameras, cx)));
    }

    fn stage_size(&self) -> (f32, f32) {
        self.stage_bounds.get().map_or((0.0, 0.0), |bounds| {
            (f32::from(bounds.size.width), f32::from(bounds.size.height))
        })
    }

    /// Measures the stage as it is drawn; a new size draws again.
    fn stage_probe(&self) -> impl IntoElement + use<> {
        let bounds_cell = Rc::clone(&self.stage_bounds);
        canvas(
            move |bounds, window, _| {
                if bounds_cell.get() != Some(bounds) {
                    bounds_cell.set(Some(bounds));
                    window.refresh();
                }
            },
            |_, (), _, _| {},
        )
        .absolute()
        .size_full()
    }
}

/// Physical pixels of a tile `width` logical pixels wide, 16:9.
fn shown_picture(user_id: Snowflake, size: VideoTileSize, width: f32, scale: f32) -> ShownPicture {
    // Tile sizes are a few thousand pixels at most.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let to_pixels = |logical: f32| (logical * scale).round().max(1.0) as u32;
    ShownPicture {
        user_id,
        tile_size: size,
        display_width: to_pixels(width),
        display_height: to_pixels(width / TILE_ASPECT_RATIO),
    }
}

/// The people of the call, in join order, you included even before your state comes back.
pub(crate) fn call_people(
    states: &[VoiceState],
    own_user_id: Option<Snowflake>,
) -> Vec<CallPerson> {
    let mut people: Vec<CallPerson> = states.iter().map(CallPerson::of_voice_state).collect();
    if let Some(own_user_id) = own_user_id
        && !people.iter().any(|person| person.user_id == own_user_id)
    {
        people.push(CallPerson {
            user_id: own_user_id,
            video: None,
        });
    }
    people
}

impl Render for CallView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let watched_stream = self.voice.read(cx).watched_stream_user_id();
        let body = if let Some(user_id) = watched_stream {
            self.watch(Vec::new(), cx);
            self.render_stream(user_id, window, cx)
        } else {
            if self.timed_stream_user_id.is_some() && self.is_fullscreen {
                // The watched screen ended or was closed: its full screen ends with it, as the
                // web's StreamView leaves full screen when it goes away.
                self.layout = self.layout_before_fullscreen;
                if self.leave_fullscreen() {
                    window.defer(cx, |window, _| {
                        if window.is_fullscreen() {
                            window.toggle_fullscreen();
                        }
                    });
                }
            }
            self.cancel_stream_timers();
            self.render_call(window, cx)
        };
        let is_fullscreen = self.is_fullscreen;
        div()
            .id("call-view")
            .track_focus(&self.focus)
            .key_context("CallView")
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .overflow_hidden()
            .bg(color(tokens.component_call_view_background))
            .when(!is_fullscreen, |view| {
                view.rounded(length(COMPONENT_SHEET_RADIUS))
            })
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    view.escape(window, cx);
                }
            }))
            .child(body)
    }
}

impl CallView {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render_call(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let tokens = theme(cx);
        let strings = strings(cx);
        let voice = self.voice.read(cx);
        let place = voice.connected_place();
        let toggles = voice.toggles();
        let is_self_speaking = voice.is_self_speaking();
        let state = self.session.read(cx).state();
        let own_user_id = state.current_user().map(|user| user.id);
        let channel_name = place
            .and_then(|(_, channel_id)| state.guild_channel(channel_id))
            .map(|channel| channel.name.clone())
            .unwrap_or_default();
        let states: Vec<VoiceState> = place
            .map(|(_, channel_id)| state.voice_participants(channel_id).cloned().collect())
            .unwrap_or_default();
        let speaking: HashSet<Snowflake> = states
            .iter()
            .map(|voice_state| voice_state.user_id)
            .filter(|user_id| state.is_speaking(*user_id))
            .chain(own_user_id.filter(|_| is_self_speaking))
            .collect();
        let people = call_people(&states, own_user_id);
        let present: Vec<Snowflake> = people.iter().map(|person| person.user_id).collect();
        self.speaker_user_id = next_speaker(self.speaker_user_id, &speaking, own_user_id, &present);
        let featured = (self.layout == CallLayout::Speaker)
            .then(|| {
                featured_person_of(
                    &people,
                    self.picked_user_id,
                    self.speaker_user_id,
                    own_user_id,
                )
            })
            .flatten();
        let arrangement = arrange_call_tiles(&people, featured, MAXIMUM_CALL_TILES);
        let tiles = arrangement.tiles;
        let is_speaker = self.layout == CallLayout::Speaker;
        let is_fullscreen = self.is_fullscreen;
        let strip_count = if is_speaker && !is_fullscreen {
            tiles.len().saturating_sub(1)
        } else {
            0
        };
        let (stage_width, stage_height) = self.stage_size();
        let gap = COMPONENT_CALL_VIEW_TILE_GAP;
        let grid = grid_shape_for(tiles.len(), stage_width, stage_height, gap);
        let featured_height = if strip_count > 0 {
            stage_height - COMPONENT_CALL_VIEW_STRIP_TILE_HEIGHT - gap
        } else {
            stage_height
        };
        let featured_width = grid_shape_for(1, stage_width, featured_height, 0.0).tile_width;
        let strip_width = COMPONENT_CALL_VIEW_STRIP_TILE_HEIGHT * TILE_ASPECT_RATIO;
        let scale = window.scale_factor();
        // The pictures shown, and how large: as `watchedCamerasOf`, with the drawn sizes.
        let first_user_id = tiles.first().map(|person| person.user_id);
        let cameras: Vec<ShownPicture> = watched_pictures_of(&tiles, self.layout, own_user_id)
            .into_iter()
            // Full screen shows the featured tile alone.
            .filter(|picture| !is_fullscreen || Some(picture.user_id) == first_user_id)
            .map(|picture| {
                let is_featured = is_speaker && Some(picture.user_id) == first_user_id;
                let width = if !is_speaker {
                    grid.tile_width
                } else if is_featured {
                    featured_width
                } else {
                    strip_width
                };
                shown_picture(picture.user_id, picture.size, width, scale)
            })
            .collect();
        self.watch(cameras, cx);

        let guild_id = place.map(|(guild_id, _)| guild_id);
        let look_of =
            |view: &Self, person: &CallPerson, is_small: bool, is_in_strip: bool, cx: &App| {
                let voice = view.voice.read(cx);
                let is_own = Some(person.user_id) == own_user_id;
                let voice_state = states
                    .iter()
                    .find(|candidate| candidate.user_id == person.user_id);
                let shown = view.person_of(person.user_id, guild_id, cx);
                let name = display_name(&shown).to_owned();
                let is_undecodable = person.video.is_some()
                    && !is_own
                    && !Voice::can_show_video_in(voice_state.and_then(|state| state.video_codec));
                let is_microphone_off = if is_own {
                    toggles.is_muted()
                } else {
                    voice_state.is_some_and(|state| state.self_mute || state.suppressed)
                };
                let (action_label, is_pressed) =
                    if person.video == Some(CallVideo::Screen) && !is_own {
                        (strings.screen_share().watch(&name), None)
                    } else {
                        let shown_name = if is_own {
                            strings.call().you(&name)
                        } else {
                            name.clone()
                        };
                        (
                            strings.call().feature(&shown_name),
                            Some(
                                view.layout == CallLayout::Speaker
                                    && view.picked_user_id == Some(person.user_id),
                            ),
                        )
                    };
                TileLook {
                    person: shown,
                    video: person.video,
                    is_own,
                    is_speaking: speaking.contains(&person.user_id),
                    is_microphone_off,
                    is_paused: !is_own && voice.is_video_paused(person.user_id),
                    sentence: if is_own && voice.camera_phase() == CameraPhase::Starting {
                        Some(strings.call().starting_camera())
                    } else {
                        is_undecodable.then(|| strings.desktop().call_undecodable())
                    },
                    is_small,
                    is_in_strip,
                    action_label: action_label.into(),
                    is_pressed,
                }
            };

        let tiles_area: AnyElement = if is_speaker {
            let featured_tile = tiles.first().map(|person| {
                let look = look_of(self, person, false, false, cx);
                div()
                    .w(length(featured_width))
                    .h(length(featured_width / TILE_ASPECT_RATIO))
                    .child(self.render_tile(&look, window, cx))
            });
            let strip: Vec<AnyElement> = if strip_count > 0 {
                tiles[1..]
                    .iter()
                    .map(|person| {
                        let look = look_of(self, person, true, true, cx);
                        div()
                            .flex_none()
                            .h_full()
                            .w(length(strip_width))
                            .child(self.render_tile(&look, window, cx))
                            .into_any_element()
                    })
                    .collect()
            } else {
                Vec::new()
            };
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap(length(gap))
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(self.stage_probe())
                        .children(featured_tile),
                )
                .when(!strip.is_empty(), |stage| {
                    stage.child(
                        div()
                            .id("call-view-strip")
                            .role(Role::List)
                            .aria_label(strings.voice().participants_in(&channel_name))
                            .flex_none()
                            .h(length(COMPONENT_CALL_VIEW_STRIP_TILE_HEIGHT))
                            .flex()
                            .justify_center()
                            .gap(length(gap))
                            .overflow_x_scroll()
                            .children(strip),
                    )
                })
                .into_any_element()
        } else {
            let is_small = tiles.len() > 4;
            let grid_tiles: Vec<AnyElement> = tiles
                .iter()
                .map(|person| {
                    let look = look_of(self, person, is_small, false, cx);
                    div()
                        .flex_none()
                        .w(length(grid.tile_width))
                        .h(length(grid.tile_width / TILE_ASPECT_RATIO))
                        .child(self.render_tile(&look, window, cx))
                        .into_any_element()
                })
                .collect();
            div()
                .id("call-view-grid")
                .role(Role::List)
                .aria_label(strings.voice().participants_in(&channel_name))
                .relative()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_wrap()
                .content_center()
                .justify_center()
                .gap(length(gap))
                .child(self.stage_probe())
                .children(grid_tiles)
                .into_any_element()
        };

        let overflow: Vec<AnyElement> = arrangement
            .overflow
            .iter()
            .map(|person| {
                let shown = self.person_of(person.user_id, guild_id, cx);
                let picture = shown.avatar.map(|image| {
                    profile_picture(image, COMPONENT_CALL_VIEW_OVERFLOW_AVATAR_SIZE, window)
                });
                let is_speaking = speaking.contains(&person.user_id);
                div()
                    .id(("call-overflow", person.user_id.raw()))
                    .role(Role::ListItem)
                    .aria_label(SharedString::from(display_name(&shown).to_owned()))
                    .rounded_full()
                    .when(is_speaking, |avatar| {
                        avatar
                            .border(length(COMPONENT_CALL_VIEW_SPEAKING_RING_WIDTH))
                            .border_color(color(tokens.component_call_view_speaking_ring))
                    })
                    .child(Avatar::new(
                        AvatarPerson::from_user(&shown, picture),
                        COMPONENT_CALL_VIEW_OVERFLOW_AVATAR_SIZE,
                    ))
                    .into_any_element()
            })
            .collect();
        let overflow_count = i64::try_from(overflow.len()).unwrap_or(i64::MAX);

        let header = self.render_header(
            icon(
                IconName::SpeakerHigh,
                COMPONENT_CHAT_HEADER_ICON_SIZE,
                color(tokens.component_chat_header_icon),
            )
            .into_any_element(),
            channel_name.clone().into(),
            None,
            true,
            cx,
        );
        let stage_box = div()
            .when(!is_fullscreen, |stage| {
                stage
                    .mx(length(COMPONENT_CALL_VIEW_STAGE_INSET))
                    .rounded(length(COMPONENT_CALL_VIEW_STAGE_RADIUS))
            })
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(length(gap))
            .overflow_hidden()
            .bg(color(tokens.component_call_view_stage))
            .p(length(COMPONENT_CALL_VIEW_STAGE_PADDING))
            .child(tiles_area)
            .when(!overflow.is_empty(), |stage| {
                stage.child(
                    div()
                        .id("call-view-overflow")
                        .role(Role::List)
                        .aria_label(strings.call().overflow(overflow_count))
                        .flex_none()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .justify_center()
                        .gap(length(SPACE_XS))
                        .children(overflow),
                )
            });
        div()
            .id("call-view-content")
            .role(Role::Region)
            .aria_label(strings.call().view_of(if channel_name.is_empty() {
                strings.voice().voice_channel()
            } else {
                &channel_name
            }))
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(stage_box)
            .when(!is_fullscreen, |view| {
                view.child(self.render_controls(toggles.is_muted(), window, cx))
            })
            .into_any_element()
    }

    /// The chat header's grammar: a leading mark, the title, then the actions.
    fn render_header(
        &mut self,
        leading: AnyElement,
        title: SharedString,
        live_chip: Option<AnyElement>,
        has_layout_toggles: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let strings = strings(cx);
        let entity = cx.entity();
        let header_button = |id: &'static str, icon_name: IconName, label: &'static str| {
            IconButton::new(id, icon_name, label).sizes(
                COMPONENT_CHAT_HEADER_BUTTON_SIZE,
                COMPONENT_CHAT_HEADER_ICON_SIZE,
            )
        };
        let layout = self.layout;
        let fullscreen_label = if self.is_fullscreen {
            strings.screen_share().exit_fullscreen()
        } else {
            strings.screen_share().fullscreen()
        };
        let fullscreen_icon = if self.is_fullscreen {
            IconName::CornersIn
        } else {
            IconName::CornersOut
        };
        let close_label = if self.voice.read(cx).watched_stream_user_id().is_some() {
            strings.screen_share().close()
        } else {
            strings.call().close()
        };
        let grid_entity = entity.clone();
        let speaker_entity = entity.clone();
        let fullscreen_entity = entity.clone();
        div()
            .flex_none()
            .h(length(COMPONENT_CHAT_HEADER_HEIGHT))
            .pl(length(COMPONENT_CHAT_HEADER_PADDING_INLINE))
            .pr(length(SPACE_XS))
            .flex()
            .items_center()
            .gap(length(SPACE_XS))
            .child(leading)
            .child(
                with_type(div(), TYPE_HEADING_MD)
                    .id("call-view-title")
                    .role(Role::Heading)
                    .aria_level(1)
                    .min_w_0()
                    .truncate()
                    .text_color(color(tokens.component_chat_header_name_text))
                    .child(title),
            )
            .children(live_chip)
            .child(div().flex_1())
            .when(has_layout_toggles && !self.is_fullscreen, |header| {
                header.child(
                    div()
                        .id("call-view-layout")
                        .role(Role::Group)
                        .aria_label(strings.call().layout())
                        .flex()
                        .gap(length(SPACE_2XS))
                        .child(
                            header_button(
                                "call-layout-grid",
                                IconName::SquaresFour,
                                strings.call().grid(),
                            )
                            .toggled(layout == CallLayout::Grid)
                            .on_click(move |_, cx| {
                                grid_entity.update(cx, |view, cx| {
                                    view.choose_layout(CallLayout::Grid, cx);
                                });
                            }),
                        )
                        .child(
                            header_button(
                                "call-layout-speaker",
                                IconName::UserFocus,
                                strings.call().speaker(),
                            )
                            .toggled(layout == CallLayout::Speaker)
                            .on_click(move |_, cx| {
                                speaker_entity.update(cx, |view, cx| {
                                    view.choose_layout(CallLayout::Speaker, cx);
                                });
                            }),
                        ),
                )
            })
            .child(
                header_button("call-view-fullscreen", fullscreen_icon, fullscreen_label)
                    .when(self.is_fullscreen, |button| button.key_shortcuts("Escape"))
                    .on_click(move |window, cx| {
                        fullscreen_entity.update(cx, |view, cx| view.toggle_fullscreen(window, cx));
                    }),
            )
            .when(!self.is_fullscreen, |header| {
                header.child(
                    header_button("call-view-close", IconName::X, close_label)
                        .key_shortcuts("Escape")
                        .on_click(move |_, cx| entity.update(cx, Self::close)),
                )
            })
            .into_any_element()
    }

    /// The call's own controls: the microphone and hanging up.
    fn render_controls(
        &self,
        is_muted: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let strings = strings(cx);
        let microphone_voice = self.voice.downgrade();
        let leave_voice = self.voice.downgrade();
        let camera_voice = self.voice.downgrade();
        let (camera_phase, can_use_camera) = {
            let voice = self.voice.read(cx);
            (voice.camera_phase(), voice.camera_block(cx).is_none())
        };
        let camera_words = strings.camera();
        div()
            .id("call-view-controls")
            .role(Role::Group)
            .aria_label(strings.call().controls())
            .flex_none()
            .h(length(COMPONENT_CALL_VIEW_CONTROL_BAR_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .gap(length(COMPONENT_CALL_VIEW_CONTROL_GAP))
            .px(length(SPACE_XS))
            .child(call_control(
                "call-control-microphone",
                if is_muted {
                    IconName::MicrophoneSlash
                } else {
                    IconName::Microphone
                },
                strings.user_card().mute_microphone(),
                Some(is_muted),
                false,
                move |_, cx| {
                    let _ = microphone_voice.update(cx, Voice::toggle_microphone);
                },
                window,
                cx,
            ))
            // As on the web: shown while the camera can be turned on, or to turn it off.
            .when(
                can_use_camera || camera_phase != CameraPhase::Off,
                |controls| {
                    controls.child(call_control(
                        "call-control-camera",
                        if camera_phase == CameraPhase::Off {
                            IconName::VideoCameraSlash
                        } else {
                            IconName::VideoCamera
                        },
                        match camera_phase {
                            CameraPhase::Starting => camera_words.starting(),
                            CameraPhase::On => camera_words.turn_off(),
                            CameraPhase::Off => camera_words.turn_on(),
                        },
                        Some(camera_phase != CameraPhase::Off),
                        false,
                        move |_, cx| {
                            let _ = camera_voice.update(cx, Voice::toggle_camera);
                        },
                        window,
                        cx,
                    ))
                },
            )
            .child(call_control(
                "call-control-leave",
                IconName::PhoneDisconnect,
                strings.voice().disconnect(),
                None,
                true,
                move |_, cx| {
                    let _ = leave_voice.update(cx, Voice::leave);
                },
                window,
                cx,
            ))
            .into_any_element()
    }

    /// One person's tile: their picture filling it (a camera cropped, a screen letterboxed),
    /// else their avatar with a sentence; their name in a chip at the lower start corner after a
    /// crossed microphone; a screen adds the live chip. Whoever talks gets the lamp, a ring
    /// inside the edge. The whole tile is one toggle.
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render_tile(
        &self,
        look: &TileLook,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let strings = strings(cx);
        let user_id = look.person.id;
        let picture = if look.is_own {
            // Your camera, mirrored as on the web; never your shared screen.
            (look.video == Some(CallVideo::Camera))
                .then(|| self.pictures.read(cx).own_camera().cloned())
                .flatten()
        } else if look.is_paused {
            None
        } else {
            self.pictures.read(cx).picture(user_id).cloned()
        };
        let name = display_name(&look.person).to_owned();
        let shown_name = if look.is_own {
            strings.call().you(&name)
        } else {
            name
        };
        let sentence = if look.is_paused {
            Some(strings.call().paused())
        } else {
            look.sentence
        };
        let states: Vec<&str> = [
            (look.is_microphone_off, strings.voice().microphone_off()),
            (
                look.video == Some(CallVideo::Camera),
                strings.camera().camera_on(),
            ),
            (
                look.video == Some(CallVideo::Screen),
                strings.screen_share().shares_screen(),
            ),
            (look.is_paused, strings.call().paused()),
        ]
        .into_iter()
        .filter_map(|(is_shown, word)| is_shown.then_some(word))
        .collect();
        let description = std::iter::once(shown_name.as_str())
            .chain(states.iter().copied())
            .collect::<Vec<_>>()
            .join(", ");
        let id = gpui::ElementId::from(("call-tile", user_id.raw()));
        let focus_handle = remembered_focus_handle(&id, window, cx);
        let is_ring_shown = is_focus_visible(&focus_handle, window);
        let avatar_size = if look.is_small {
            COMPONENT_CALL_VIEW_AVATAR_SIZE_SMALL
        } else {
            COMPONENT_CALL_VIEW_AVATAR_SIZE_LARGE
        };
        let avatar_picture = look
            .person
            .avatar
            .map(|image| profile_picture(image, avatar_size, window));
        let is_screen = look.video == Some(CallVideo::Screen);
        let entity = cx.entity();
        let voice = self.voice.downgrade();
        let watches_screen = is_screen && !look.is_own;
        let label_ink = color(tokens.component_call_view_tile_text);
        div()
            .id(id)
            .debug_selector(move || format!("call-tile-{}", user_id.raw()))
            .role(Role::Button)
            .aria_label(look.action_label.clone())
            .aria_description(SharedString::from(description))
            .when_some(look.is_pressed, |tile, is_pressed| {
                tile.aria_toggled(if is_pressed {
                    Toggled::True
                } else {
                    Toggled::False
                })
            })
            .track_focus(&focus_handle)
            .cursor_pointer()
            .relative()
            .size_full()
            .overflow_hidden()
            .rounded(length(COMPONENT_CALL_VIEW_TILE_RADIUS))
            .bg(color(tokens.component_call_view_tile_background))
            .text_color(label_ink)
            .tooltip({
                let label = look.action_label.clone();
                move |_, cx| crate::components::Tooltip::view(label.clone(), cx)
            })
            .tooltip_show_delay(crate::components::Tooltip::show_delay())
            .on_click(move |_, window, cx| {
                if watches_screen {
                    let _ = voice.update(cx, |voice, cx| voice.watch_stream(Some(user_id), cx));
                } else {
                    entity.update(cx, |view, cx| view.toggle_picked(user_id, cx));
                }
                // The tile may move (to the large place, or back to the strip): focus follows
                // it through its remembered handle.
                window.refresh();
            })
            .when_some(picture.as_ref(), |tile, picture| {
                tile.child(
                    div()
                        .debug_selector(move || format!("call-picture-{}", user_id.raw()))
                        .absolute()
                        .inset_0()
                        .child(video_view(
                            picture,
                            if is_screen {
                                VideoFit::Contain
                            } else {
                                VideoFit::Cover
                            },
                        )),
                )
            })
            .when(picture.is_none(), |tile| {
                tile.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(length(SPACE_XS))
                        .p(length(SPACE_XS))
                        .child(Avatar::new(
                            AvatarPerson::from_user(&look.person, avatar_picture),
                            avatar_size,
                        ))
                        .when_some(sentence.filter(|_| !look.is_in_strip), |stage, sentence| {
                            let is_paused = look.is_paused;
                            stage.child(
                                with_type(div(), TYPE_BODY_SM)
                                    .debug_selector(move || {
                                        let kind = if is_paused { "paused" } else { "sentence" };
                                        format!("call-{kind}-{}", user_id.raw())
                                    })
                                    .flex()
                                    .items_center()
                                    .gap(length(SPACE_2XS))
                                    .text_center()
                                    .when(look.is_paused, |line| {
                                        line.child(icon(
                                            IconName::WifiSlash,
                                            COMPONENT_CALL_VIEW_LABEL_ICON_SIZE,
                                            label_ink,
                                        ))
                                    })
                                    .child(sentence),
                            )
                        }),
                )
            })
            .when(is_screen, |tile| {
                tile.child(
                    div()
                        .absolute()
                        .top(length(SPACE_XS))
                        .left(length(SPACE_XS))
                        .child(live_chip(strings.screen_share().live(), cx)),
                )
            })
            .child(
                with_type(div(), TYPE_BODY_SM)
                    .absolute()
                    .left(length(SPACE_XS))
                    .bottom(length(SPACE_XS))
                    .max_w(gpui::relative(0.9))
                    .h(length(COMPONENT_CALL_VIEW_LABEL_HEIGHT))
                    .px(length(COMPONENT_CALL_VIEW_LABEL_PADDING_INLINE))
                    .rounded(length(COMPONENT_CALL_VIEW_LABEL_RADIUS))
                    .bg(color(tokens.component_call_view_label_background))
                    .flex()
                    .items_center()
                    .gap(length(SPACE_2XS))
                    .when(look.is_paused && look.is_in_strip, |chip| {
                        chip.child(icon(
                            IconName::WifiSlash,
                            COMPONENT_CALL_VIEW_LABEL_ICON_SIZE,
                            label_ink,
                        ))
                    })
                    .when(look.is_microphone_off, |chip| {
                        chip.child(icon(
                            IconName::MicrophoneSlash,
                            COMPONENT_CALL_VIEW_LABEL_ICON_SIZE,
                            label_ink,
                        ))
                    })
                    .child(div().min_w_0().truncate().child(shown_name)),
            )
            .when(look.is_speaking, |tile| {
                // Dusk's lamp, inside the edge so nothing shifts.
                tile.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(length(COMPONENT_CALL_VIEW_TILE_RADIUS))
                        .border(length(COMPONENT_CALL_VIEW_SPEAKING_RING_WIDTH))
                        .border_color(color(tokens.component_call_view_speaking_ring)),
                )
            })
            .when(is_ring_shown, |tile| {
                tile.child(inset_focus_ring(COMPONENT_CALL_VIEW_TILE_RADIUS, cx))
            })
            .into_any_element()
    }

    fn cancel_stream_timers(&mut self) {
        self.late_timer = None;
        self.ended_timer = None;
        self.is_picture_late = false;
        self.timed_stream_user_id = None;
        self.shown_stream_size = None;
    }

    /// A shared screen watched large, with its own sound band.
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render_stream(
        &mut self,
        user_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let strings = strings(cx);
        if self.timed_stream_user_id != Some(user_id) {
            self.cancel_stream_timers();
            self.timed_stream_user_id = Some(user_id);
        }
        let voice = self.voice.read(cx);
        let call_channel_id = voice.connected_place().map(|(_, channel_id)| channel_id);
        let guild_id = voice.connected_place().map(|(guild_id, _)| guild_id);
        let is_paused = voice.is_video_paused(user_id);
        let is_deafened = voice.toggles().is_deafened();
        let state = self.session.read(cx).state();
        let voice_state = state.voice_state_of(user_id).cloned();
        let picture = self.pictures.read(cx).picture(user_id).cloned();
        let status =
            watched_stream_status(voice_state.as_ref(), call_channel_id, picture.is_some());
        let person = self.person_of(user_id, guild_id, cx);
        let name = display_name(&person).to_owned();
        let is_undecodable = status != WatchedStreamStatus::Ended
            && !Voice::can_show_video_in(voice_state.as_ref().and_then(|state| state.video_codec));
        let is_waiting = !is_undecodable && status == WatchedStreamStatus::Connecting;
        self.follow_stream_timers(is_waiting, status == WatchedStreamStatus::Ended, cx);
        let sentence: Option<String> = match status {
            WatchedStreamStatus::Ended => Some(strings.screen_share().ended(&name)),
            _ if is_paused => Some(strings.screen_share().paused().to_owned()),
            _ if is_undecodable => Some(strings.desktop().screen_undecodable().to_owned()),
            _ if is_waiting && self.is_picture_late => Some(strings.screen_share().slow(&name)),
            _ if is_waiting => Some(strings.screen_share().connecting().to_owned()),
            _ => None,
        };
        // The drawn size, in physical pixels, for the layer and the decoder's scaling.
        let (width, height) = self.stage_size();
        if width > 0.0 && height > 0.0 {
            let scale = window.scale_factor();
            // Window sizes: a few thousand pixels at most.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let size = (
                (width * scale).round() as u32,
                (height * scale).round() as u32,
            );
            if self.shown_stream_size != Some(size) {
                self.shown_stream_size = Some(size);
                let voice = self.voice.clone();
                cx.defer(move |cx| {
                    voice.update(cx, |voice, cx| {
                        voice.set_stream_display_size(size.0, size.1, cx);
                    });
                });
            }
        }
        let avatar_picture = person
            .avatar
            .map(|image| profile_picture(image, COMPONENT_STREAM_VIEW_AVATAR_SIZE, window));
        let leading = Avatar::new(
            AvatarPerson::from_user(
                &person,
                person
                    .avatar
                    .map(|image| profile_picture(image, COMPONENT_CHAT_HEADER_ICON_SIZE, window)),
            ),
            COMPONENT_CHAT_HEADER_ICON_SIZE,
        )
        .into_any_element();
        let title: SharedString = strings.screen_share().screen_of(&name).into();
        let chip = (status != WatchedStreamStatus::Ended)
            .then(|| live_chip(strings.screen_share().live(), cx).into_any_element());
        let header = self.render_header(leading, title.clone(), chip, false, cx);
        let is_fullscreen = self.is_fullscreen;
        let shows_picture = picture.is_some() && !is_paused && status != WatchedStreamStatus::Ended;
        let picture_stage = div()
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .bg(color(tokens.component_stream_view_stage))
            .when(!is_fullscreen, |stage| {
                stage
                    .mx(length(COMPONENT_STREAM_VIEW_STAGE_INSET))
                    .rounded(length(COMPONENT_STREAM_VIEW_STAGE_RADIUS))
            })
            .child(self.stage_probe())
            .when_some(picture.filter(|_| shows_picture), |stage, picture| {
                stage.child(video_view(&picture, VideoFit::Contain))
            })
            .when_some(sentence.clone(), |stage, sentence| {
                stage.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(length(SPACE_SM))
                        .p(length(SPACE_SM))
                        .child(Avatar::new(
                            AvatarPerson::from_user(&person, avatar_picture),
                            COMPONENT_STREAM_VIEW_AVATAR_SIZE,
                        ))
                        .child(
                            with_type(div(), TYPE_BODY)
                                .debug_selector(|| "stream-sentence".to_owned())
                                .text_center()
                                .text_color(color(tokens.component_stream_view_stage_text))
                                .child(sentence),
                        ),
                )
            });
        let sound_bar = (status != WatchedStreamStatus::Ended && !is_fullscreen)
            .then(|| self.render_stream_sound(user_id, is_deafened, cx));
        div()
            .id("stream-view")
            .debug_selector(|| "stream-view".to_owned())
            .role(Role::Region)
            .aria_label(title)
            .size_full()
            .flex()
            .flex_col()
            .bg(color(tokens.component_stream_view_background))
            .child(header)
            .child(picture_stage)
            .when(!is_fullscreen, |view| {
                view.child(
                    div()
                        .flex_none()
                        .h(length(COMPONENT_STREAM_VIEW_SOUND_BAR_HEIGHT))
                        .flex()
                        .items_center()
                        .gap(length(COMPONENT_STREAM_VIEW_SOUND_BAR_GAP))
                        .pl(length(COMPONENT_STREAM_VIEW_STAGE_INSET))
                        .pr(length(COMPONENT_CHAT_HEADER_PADDING_INLINE))
                        .children(sound_bar),
                )
            })
            // Exists from the start, so "connecting" then "stopped" are read out as they come.
            .child(polite_live_region(
                "stream-view-announcement",
                sentence.unwrap_or_default().into(),
            ))
            .into_any_element()
    }

    /// The screen's own sound, apart from the person's voice: mute and volume.
    fn render_stream_sound(
        &self,
        user_id: Snowflake,
        is_deafened: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let strings = strings(cx);
        let setting = Voice::stream_sound_setting(user_id, cx);
        let mute_voice = self.voice.downgrade();
        let volume_voice = self.voice.downgrade();
        let bounds = RangeBounds {
            minimum: f32::from(u16::try_from(MINIMUM_VOLUME_PERCENT).unwrap_or(0)),
            maximum: f32::from(u16::try_from(MAXIMUM_VOLUME_PERCENT).unwrap_or(200)),
            step: 1.0,
        };
        // Whole percents from 0 to 200: exact in `f32`.
        #[allow(clippy::cast_precision_loss)]
        let volume = setting.volume_percent as f32;
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(length(COMPONENT_STREAM_VIEW_SOUND_BAR_GAP))
            .child(
                IconButton::new(
                    "stream-mute",
                    if setting.is_locally_muted {
                        IconName::SpeakerX
                    } else {
                        IconName::SpeakerHigh
                    },
                    strings.screen_share().mute_stream(),
                )
                .sizes(
                    COMPONENT_CHAT_HEADER_BUTTON_SIZE,
                    COMPONENT_CHAT_HEADER_ICON_SIZE,
                )
                .toggled(setting.is_locally_muted)
                .on_click(move |_, cx| {
                    let _ =
                        mute_voice.update(cx, |voice, cx| voice.toggle_stream_muted(user_id, cx));
                }),
            )
            .child(
                div()
                    .w(length(COMPONENT_STREAM_VIEW_VOLUME_WIDTH))
                    .min_w_0()
                    .child(
                        RangeField::new(
                            "stream-volume",
                            strings.screen_share().stream_volume(),
                            volume,
                            bounds,
                            move |value| {
                                // Whole percents within 0 to 200.
                                #[allow(clippy::cast_possible_truncation)]
                                let percent = value.round() as i64;
                                strings.voice_settings().percent(percent).into()
                            },
                        )
                        .label_hidden()
                        .on_change(move |value, _, cx| {
                            // Snapped to whole percents within 0 to 200 by the slider.
                            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                            let percent = value.round().max(0.0) as u32;
                            let _ = volume_voice.update(cx, |voice, cx| {
                                voice.set_stream_volume(user_id, percent, cx);
                            });
                        }),
                    ),
            )
            .child(
                with_type(div(), TYPE_BODY_SM)
                    .flex_none()
                    .text_color(color(tokens.component_stream_view_sound_status_text))
                    .child(
                        strings
                            .voice_settings()
                            .percent(i64::from(setting.volume_percent)),
                    ),
            )
            .child(div().flex_1())
            .when(is_deafened, |bar| {
                bar.child(
                    with_type(div(), TYPE_BODY_SM)
                        .flex()
                        .items_center()
                        .gap(length(SPACE_2XS))
                        .text_color(color(tokens.component_stream_view_sound_status_text))
                        .child(icon(
                            IconName::Headphones,
                            COMPONENT_CALL_VIEW_LABEL_ICON_SIZE,
                            color(tokens.component_stream_view_sound_status_text),
                        ))
                        .child(strings.screen_share().deafened()),
                )
            })
            .into_any_element()
    }

    /// No picture after a while: say it is not coming. Ended: the sentence stays for a reading
    /// time, then the view goes; sharing again meanwhile cancels it.
    fn follow_stream_timers(&mut self, is_waiting: bool, is_ended: bool, cx: &mut Context<Self>) {
        if !is_waiting {
            self.late_timer = None;
            self.is_picture_late = false;
        } else if self.late_timer.is_none() && !self.is_picture_late {
            self.late_timer = Some(cx.spawn(async move |view, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(
                        COMPONENT_STREAM_VIEW_SLOW_CONNECT_DELAY.0,
                    ))
                    .await;
                let _ = view.update(cx, |view, cx| {
                    view.is_picture_late = true;
                    view.late_timer = None;
                    cx.notify();
                });
            }));
        }
        if !is_ended {
            self.ended_timer = None;
        } else if self.ended_timer.is_none() {
            self.ended_timer = Some(cx.spawn(async move |view, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(
                        COMPONENT_STREAM_VIEW_ENDED_CLOSE_DELAY.0,
                    ))
                    .await;
                let _ = view.update(cx, |view, cx| {
                    view.voice
                        .update(cx, |voice, cx| voice.watch_stream(None, cx));
                });
            }));
        }
    }
}

/// The "LIVE" chip beside someone who shares their screen.
pub(crate) fn live_chip(label: &'static str, cx: &App) -> impl IntoElement + use<> {
    let tokens = theme(cx);
    with_type(div(), TYPE_BADGE)
        .flex_none()
        .h(length(COMPONENT_LIVE_BADGE_HEIGHT))
        .px(length(COMPONENT_LIVE_BADGE_PADDING_INLINE))
        .rounded(length(COMPONENT_LIVE_BADGE_RADIUS))
        .bg(color(tokens.component_live_badge_background))
        .text_color(color(tokens.component_live_badge_text))
        .flex()
        .items_center()
        .child(label)
}

/// A round call control on the call view's band, with the call view's own colours.
#[allow(clippy::too_many_arguments)] // One control's look and action, read at the call site.
fn call_control(
    id: &'static str,
    icon_name: IconName,
    label: &'static str,
    is_pressed: Option<bool>,
    is_hang_up: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Stateful<gpui::Div> {
    let tokens = theme(cx);
    let id = gpui::ElementId::from(id);
    let focus_handle = remembered_focus_handle(&id, window, cx);
    let is_ring_shown = is_focus_visible(&focus_handle, window);
    let is_active = is_pressed == Some(true);
    let (background, hover_background, ink, hover_ink) = if is_active {
        (
            tokens.component_call_view_control_background_active,
            tokens.component_call_view_control_background_active_hover,
            tokens.component_call_view_control_text_active,
            tokens.component_call_view_control_text_active_hover,
        )
    } else {
        (
            tokens.component_call_view_control_background,
            tokens.component_call_view_control_background_hover,
            tokens.component_call_view_control_text,
            if is_hang_up {
                tokens.component_call_view_hang_up_text_hover
            } else {
                tokens.component_call_view_control_text_hover
            },
        )
    };
    let tooltip: SharedString = label.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .when_some(is_pressed, |button, is_pressed| {
            button.aria_toggled(if is_pressed {
                Toggled::True
            } else {
                Toggled::False
            })
        })
        .track_focus(&focus_handle)
        .cursor_pointer()
        .relative()
        .size(length(COMPONENT_CALL_VIEW_CONTROL_SIZE))
        .rounded(length(COMPONENT_CALL_VIEW_CONTROL_RADIUS))
        .flex()
        .items_center()
        .justify_center()
        .bg(color(background))
        .text_color(color(ink))
        .hover(move |style| {
            style
                .bg(color(hover_background))
                .text_color(color(hover_ink))
        })
        .tooltip(move |_, cx| crate::components::Tooltip::view(tooltip.clone(), cx))
        .tooltip_show_delay(crate::components::Tooltip::show_delay())
        .on_click(move |_, window, cx| on_click(window, cx))
        .child(icon(icon_name, COMPONENT_CHAT_HEADER_ICON_SIZE, color(ink)))
        .when(is_ring_shown, |button| {
            button.child(focus_ring(COMPONENT_CALL_VIEW_CONTROL_RADIUS, cx))
        })
}

impl super::DuskShell {
    /// Shows the call view while the call view is open or a screen is watched, focused, and
    /// gives focus back to what had it when it closes.
    pub(super) fn follow_call_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(voice) = self.voice.clone() else {
            return;
        };
        let is_wanted = {
            let voice = voice.read(cx);
            voice.is_call_view_open() || voice.watched_stream_user_id().is_some()
        };
        match (is_wanted, self.call_view.is_some()) {
            (true, false) => {
                let session = self.session.clone();
                let view = cx.new(|cx| CallView::new(session, voice, cx));
                let opener = window.focused(cx);
                let focus = view.read(cx).focus.clone();
                focus.focus(window, cx);
                self.call_view = Some((view, opener));
            }
            (false, true) => {
                if let Some((view, opener)) = self.call_view.take() {
                    if view.read(cx).has_made_window_fullscreen() && window.is_fullscreen() {
                        window.toggle_fullscreen();
                    }
                    let focus = opener.unwrap_or_else(|| self.root_focus.clone());
                    focus.focus(window, cx);
                }
            }
            _ => {}
        }
    }

    /// The call view in the sheet's place, unless it fills the window.
    pub(super) fn sheet_call_view(&self, cx: &App) -> Option<Entity<CallView>> {
        self.call_view
            .as_ref()
            .map(|(view, _)| view.clone())
            .filter(|view| !view.read(cx).is_fullscreen())
    }

    /// The call view over the whole window, in full screen.
    pub(super) fn render_fullscreen_call_view(&self, cx: &App) -> Option<AnyElement> {
        let (view, _) = self.call_view.as_ref()?;
        view.read(cx).is_fullscreen().then(|| {
            div()
                .absolute()
                .inset_0()
                .child(view.clone())
                .into_any_element()
        })
    }
}

#[cfg(test)]
#[path = "call_view_tests.rs"]
mod tests;
