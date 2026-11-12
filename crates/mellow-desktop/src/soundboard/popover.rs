//! `SoundboardPopover.svelte` (`docs/design/web-soundboard.md`, ADR-0032): a popover above the
//! voice panel's button holding the call's server sounds. A click plays a sound for everyone in
//! the channel; while it cannot play, every sound dims and one sentence says why, and a bar
//! drains through the 2 s between two sounds. Each sound's menu (right-click, Shift+F10, the Menu
//! key) previews it for the reader only and makes it their join sound; a pointer also finds the
//! preview on the sound itself. One Tab stop for the sounds, arrows between them, as in the emoji
//! picker. Esc closes it and gives focus back to its button; a click outside closes it where the
//! click put focus.

use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, DismissEvent, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Pixels, Point, Render, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Task, Window, anchored, deferred, div, point,
    prelude::FluentBuilder as _, px, relative,
};
use mellow_client_core::shared_logic::emoji_picker::{
    EmojiGridKeyPress, EmojiGridMove, next_emoji_cell,
};
use mellow_client_core::shared_logic::server_settings::holds;
use mellow_client_core::shared_logic::soundboard::SoundboardBlock;
use mellow_protocol::{
    MINIMUM_MILLISECONDS_BETWEEN_SOUNDS, Permissions, Snowflake, SoundboardSound,
};

use super::sound_emoji;
use crate::components::{
    Button, ButtonVariant, ContextMenu, FocusNextControl, FocusPreviousControl, MenuAlignment,
    MenuEntry, MenuItem, MenuOrigin, bounds_recorder, focus_next_inside, focus_previous_inside,
    inset_focus_ring, is_context_menu_keystroke, menu_origin_at_pointer, menu_origin_beside,
    polite_live_region, remembered_bounds, shadow, with_type,
};
use crate::i18n::{Strings, strings};
use crate::session::{Session, now_unix_milliseconds};
use crate::sign_in_form::describe_error;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_BUTTON_DISABLED_OPACITY, COMPONENT_CONTEXT_MENU_VIEWPORT_MARGIN,
    COMPONENT_EMOJI_PICKER_BORDER_WIDTH, COMPONENT_EMOJI_PICKER_CATEGORY_ICON_SIZE,
    COMPONENT_EMOJI_PICKER_CELL_RADIUS, COMPONENT_EMOJI_PICKER_EMOJI_SIZE,
    COMPONENT_EMOJI_PICKER_GAP, COMPONENT_EMOJI_PICKER_PADDING, COMPONENT_EMOJI_PICKER_RADIUS,
    COMPONENT_EMOJI_PICKER_WIDTH, COMPONENT_TEXT_FIELD_BORDER_WIDTH, COMPONENT_VOICE_METER_HEIGHT,
    COMPONENT_VOICE_METER_RADIUS, SIZE_HIT_WEB_MIN, SPACE_2XS, SPACE_3XS, SPACE_XS, TYPE_BODY,
    TYPE_BODY_SM, TYPE_CATEGORY,
};
use crate::theme::{color, length, theme};
use crate::voice::Voice;

/// Three sounds a row, as the web's grid.
const COLUMNS: usize = 3;

/// What the popover asks its host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundboardPopoverEvent {
    /// Esc (focus goes back to the button) or a click outside (focus stays where it went).
    Dismissed { should_return_focus: bool },
    /// The empty state's "Add sounds": Server settings at Soundboard.
    AddSounds,
}

pub struct SoundboardPopover {
    session: Entity<Session>,
    voice: Entity<Voice>,
    guild_id: Snowflake,
    can_add_sounds: bool,
    focus_handle: FocusHandle,
    grid_focus: FocusHandle,
    add_focus: FocusHandle,
    /// The bottom left corner, on the button's top left.
    position: Point<Pixels>,
    max_height: Pixels,
    active_index: usize,
    hovered_index: Option<usize>,
    announcement: SharedString,
    menu: Option<(Entity<ContextMenu>, Subscription)>,
    /// Redraws when the cooldown ends, so the sounds light up then.
    cooldown_redraw: Option<(u64, Task<()>)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SoundboardPopoverEvent> for SoundboardPopover {}

impl Focusable for SoundboardPopover {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl std::fmt::Debug for SoundboardPopover {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SoundboardPopover")
            .field("guild_id", &self.guild_id)
            .field("active_index", &self.active_index)
            .finish_non_exhaustive()
    }
}

impl SoundboardPopover {
    /// Opens above `anchor` (the voice panel's button) and takes focus: the first sound, else the
    /// "Add sounds" button, else the popover itself, so Esc and Tab still work.
    pub fn open(
        anchor: Bounds<Pixels>,
        session: Entity<Session>,
        voice: Entity<Voice>,
        guild_id: Snowflake,
        can_add_sounds: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.observe(&voice, |_, _, cx| cx.notify()),
        ];
        let margin = px(COMPONENT_CONTEXT_MENU_VIEWPORT_MARGIN);
        let popover = Self {
            session,
            voice,
            guild_id,
            can_add_sounds,
            focus_handle: cx.focus_handle(),
            grid_focus: cx.focus_handle().tab_stop(true),
            add_focus: cx.focus_handle().tab_stop(true),
            position: point(anchor.left(), anchor.top()),
            max_height: (anchor.top() - margin * 2.0).max(px(0.0)),
            active_index: 0,
            hovered_index: None,
            announcement: SharedString::default(),
            menu: None,
            cooldown_redraw: None,
            _subscriptions: subscriptions,
        };
        let first = if !popover.sounds(cx).is_empty() {
            &popover.grid_focus
        } else if can_add_sounds {
            &popover.add_focus
        } else {
            &popover.focus_handle
        };
        window.focus(first, cx);
        popover
    }

    fn sounds(&self, cx: &App) -> Vec<SoundboardSound> {
        self.session
            .read(cx)
            .state()
            .sounds_of(self.guild_id)
            .to_vec()
    }

    fn play(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(sound) = self.sounds(cx).get(index).cloned() else {
            return;
        };
        self.active_index = index;
        self.voice
            .update(cx, |voice, cx| voice.play_soundboard_sound(sound.id, cx));
        cx.notify();
    }

    fn preview(&self, sound_id: Snowflake, cx: &mut App) {
        self.voice
            .update(cx, |voice, cx| voice.preview_soundboard_sound(sound_id, cx));
    }

    fn on_grid_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let count = self.sounds(cx).len();
        if count == 0 {
            return;
        }
        if is_context_menu_keystroke(keystroke) {
            cx.stop_propagation();
            let id = cell_id(self.active_index);
            let bounds = *remembered_bounds(&id, window, cx).read(cx);
            if let Some(bounds) = bounds {
                self.open_sound_menu(
                    self.active_index,
                    menu_origin_beside(bounds, MenuAlignment::Start),
                    window,
                    cx,
                );
            }
            return;
        }
        let modifiers = keystroke.modifiers;
        if modifiers.alt || modifiers.shift {
            return;
        }
        if matches!(keystroke.key.as_str(), "enter" | "space") && !modifiers.modified() {
            cx.stop_propagation();
            self.play(self.active_index, cx);
            return;
        }
        let is_control_held = if cfg!(target_os = "macos") {
            modifiers.platform
        } else {
            modifiers.control
        };
        let next = next_emoji_cell(
            &[count],
            COLUMNS,
            self.active_index.min(count - 1),
            EmojiGridKeyPress {
                key: &keystroke.key,
                is_control_held,
            },
        );
        // Up from the first row has no search field to go back to here.
        if let Some(EmojiGridMove::Cell(next)) = next {
            cx.stop_propagation();
            self.active_index = next;
            cx.notify();
        }
    }

    // ---- A sound's menu ----

    fn open_sound_menu(
        &mut self,
        index: usize,
        origin: MenuOrigin,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(sound) = self.sounds(cx).get(index).cloned() else {
            return;
        };
        self.active_index = index;
        let catalogue = strings(cx);
        let words = catalogue.soundboard();
        let state = self.session.read(cx).state();
        let Some(guild) = state.guild(self.guild_id) else {
            return;
        };
        let is_join_sound = guild.my_join_sound_id == Some(sound.id);
        let can_choose_join_sound = holds(guild, Permissions::USE_SOUNDBOARD);
        let popover = cx.entity().downgrade();
        let sound_id = sound.id;
        let mut entries = vec![MenuEntry::Item(MenuItem::new(
            words.preview(),
            IconName::SpeakerHigh,
            {
                let popover = popover.clone();
                move |_, cx| {
                    let _ = popover.update(cx, |popover, cx| popover.preview(sound_id, cx));
                }
            },
        ))];
        if can_choose_join_sound {
            let (label, icon) = if is_join_sound {
                (words.clear_join_sound(), IconName::X)
            } else {
                (words.set_join_sound(), IconName::SpeakerHigh)
            };
            let name = sound.name.clone();
            entries.push(MenuEntry::Item(MenuItem::new(label, icon, move |_, cx| {
                let name = name.clone();
                let _ = popover.update(cx, |popover, cx| {
                    popover.set_join_sound((!is_join_sound).then_some(sound_id), name, cx);
                });
            })));
        }
        // Focus goes back to the grid, which holds the sound.
        let opener = Some(self.grid_focus.clone());
        let menu = cx.new(|cx| {
            ContextMenu::open(
                words.sound_options(&sound.name),
                entries,
                origin,
                opener,
                window,
                cx,
            )
        });
        let subscription = cx.subscribe(&menu, |popover, _, _: &DismissEvent, cx| {
            popover.menu = None;
            cx.notify();
        });
        self.menu = Some((menu, subscription));
        cx.notify();
    }

    fn set_join_sound(
        &mut self,
        sound_id: Option<Snowflake>,
        name: String,
        cx: &mut Context<Self>,
    ) {
        let guild_id = self.guild_id;
        let popover = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_settings_request(
                "set the join sound",
                move |rest| async move { rest.set_join_sound(guild_id, sound_id).await },
                cx,
                move |answer, cx| {
                    let words = strings(cx).soundboard();
                    let sentence = match answer {
                        Ok(()) if sound_id.is_some() => words.join_sound_set(&name),
                        Ok(()) => words.join_sound_cleared().to_owned(),
                        Err(error) => describe_error(
                            &error,
                            words.join_sound_failed(),
                            crate::strings_pending_i18n::strings(cx),
                        ),
                    };
                    let _ = popover.update(cx, |popover, cx| {
                        popover.announcement = sentence.into();
                        cx.notify();
                    });
                },
            );
        });
    }

    // ---- The cooldown ----

    /// The bar drains every frame (still under reduced motion); the sounds light up at its end.
    fn follow_cooldown(&mut self, ends_at: u64, now: u64, window: &Window, cx: &mut Context<Self>) {
        if now >= ends_at {
            return;
        }
        if !cx.reduce_motion() {
            window.request_animation_frame();
        }
        if self
            .cooldown_redraw
            .as_ref()
            .is_some_and(|(scheduled, _)| *scheduled == ends_at)
        {
            return;
        }
        let wait = Duration::from_millis(ends_at - now);
        let redraw = cx.spawn(async move |popover, cx| {
            cx.background_executor().timer(wait).await;
            let _ = popover.update(cx, |_, cx| cx.notify());
        });
        self.cooldown_redraw = Some((ends_at, redraw));
    }

    // ---- Drawing ----

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // One cell, as the web's `{#each}` draws it.
    fn render_cell(
        &self,
        index: usize,
        sound: &SoundboardSound,
        is_join_sound: bool,
        block: Option<SoundboardBlock>,
        reason: &SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let words = strings(cx).soundboard();
        let id = cell_id(index);
        let bounds = remembered_bounds(&id, window, cx);
        let is_grid_focused = self.grid_focus.is_focused(window);
        let is_active = index == self.active_index;
        let is_hovered = self.hovered_index == Some(index);
        let is_ring_shown = is_active && is_grid_focused && window.last_input_was_keyboard();
        let label = words.sound_label(&sound.name, is_join_sound);
        let description: SharedString = if block.is_some() {
            format!("{reason} {}", words.help()).into()
        } else {
            words.help().into()
        };
        let picture = sound_emoji(
            sound.emoji.as_deref(),
            COMPONENT_EMOJI_PICKER_EMOJI_SIZE,
            color(tokens.component_emoji_picker_category_icon),
            self.session.read(cx).state(),
            window,
            cx,
        );
        let sound_id = sound.id;
        let preview_name = words.preview_label(&sound.name);
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .aria_description(description)
            .when(is_active && is_grid_focused, |cell| {
                cell.aria_active_descendant()
            })
            .relative()
            .flex()
            .min_w_0()
            .items_center()
            .gap(length(SPACE_2XS))
            .min_h(length(SIZE_HIT_WEB_MIN))
            .px(length(SPACE_XS))
            .py(length(SPACE_2XS))
            .pr(length(SIZE_HIT_WEB_MIN))
            .rounded(length(COMPONENT_EMOJI_PICKER_CELL_RADIUS))
            .border(length(COMPONENT_TEXT_FIELD_BORDER_WIDTH))
            .border_color(if is_join_sound {
                color(tokens.component_custom_emoji_settings_name_border_hover)
            } else {
                gpui::transparent_black()
            })
            .bg(color(if is_hovered && block.is_none() {
                tokens.component_emoji_picker_cell_background_hover
            } else {
                tokens.component_custom_emoji_settings_picture_background
            }))
            .when(block.is_none(), gpui::Styled::cursor_pointer)
            .on_hover(cx.listener(move |popover, is_hovered: &bool, _, cx| {
                popover.hovered_index = if *is_hovered {
                    Some(index)
                } else {
                    popover.hovered_index.filter(|hovered| *hovered != index)
                };
                cx.notify();
            }))
            .on_click(cx.listener(move |popover, _, window, cx| {
                window.focus(&popover.grid_focus, cx);
                popover.play(index, cx);
            }))
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener(move |popover, event: &gpui::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    popover.open_sound_menu(
                        index,
                        menu_origin_at_pointer(event.position),
                        window,
                        cx,
                    );
                }),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap(length(SPACE_2XS))
                    .when(block.is_some(), |content| {
                        content.opacity(COMPONENT_BUTTON_DISABLED_OPACITY)
                    })
                    .child(picture)
                    .child(
                        with_type(div(), TYPE_BODY_SM)
                            .min_w_0()
                            .line_clamp(2)
                            .text_color(color(tokens.color_text_normal))
                            .child(sound.name.clone()),
                    ),
            )
            // Pointer shortcut to the menu's first item; keyboards reach it through the menu.
            .when(is_hovered, |cell| {
                cell.child(
                    div()
                        .id(ElementId::named_usize("sound-preview", index))
                        .role(Role::Button)
                        .aria_label(preview_name)
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_0()
                        .w(length(SIZE_HIT_WEB_MIN))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(length(COMPONENT_EMOJI_PICKER_CELL_RADIUS))
                        .cursor_pointer()
                        .on_click(cx.listener(move |popover, _, _, cx| {
                            cx.stop_propagation();
                            popover.preview(sound_id, cx);
                        }))
                        .child(crate::components::icon(
                            IconName::SpeakerHigh,
                            COMPONENT_EMOJI_PICKER_CATEGORY_ICON_SIZE,
                            color(tokens.component_emoji_picker_category_icon_hover),
                        )),
                )
            })
            .child(bounds_recorder(bounds))
            .when(is_ring_shown, |cell| {
                cell.child(inset_focus_ring(COMPONENT_EMOJI_PICKER_CELL_RADIUS, cx))
            })
            .into_any_element()
    }

    fn render_empty(
        &self,
        guild_name: &str,
        catalogue: Strings,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let words = catalogue.soundboard();
        let sentence = format!(
            "{} {}",
            words.empty(guild_name),
            if self.can_add_sounds {
                words.empty_can_add()
            } else {
                words.empty_ask()
            }
        );
        let popover = cx.entity().downgrade();
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(length(SPACE_XS))
            .px(length(SPACE_2XS))
            .pb(length(SPACE_2XS))
            .child(
                with_type(div(), TYPE_BODY)
                    .text_color(color(tokens.component_emoji_picker_empty_text))
                    .child(sentence),
            )
            .when(self.can_add_sounds, |column| {
                column.child(
                    Button::new("soundboard-add-sounds", words.add_sounds())
                        .variant(ButtonVariant::Primary)
                        .track_focus(&self.add_focus)
                        .on_click(move |_, cx| {
                            let _ = popover
                                .update(cx, |_, cx| cx.emit(SoundboardPopoverEvent::AddSounds));
                        }),
                )
            })
            .into_any_element()
    }
}

fn cell_id(index: usize) -> ElementId {
    ElementId::named_usize("sound-cell", index)
}

impl Render for SoundboardPopover {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.soundboard();
        let now = now_unix_milliseconds();
        let block = self.voice.read(cx).soundboard_block(now, cx);
        let state = self.session.read(cx).state();
        let cooldown_ends_at = state.sound_cooldown_ends_at_unix_milliseconds();
        let guild = state.guild(self.guild_id).cloned();
        let sounds = self.sounds(cx);
        if self.active_index >= sounds.len() {
            self.active_index = sounds.len().saturating_sub(1);
        }
        self.follow_cooldown(cooldown_ends_at, now, window, cx);

        let title = guild.as_ref().map_or_else(
            || words.soundboard().to_owned(),
            |guild| words.sounds(&guild.name),
        );
        let join_sound = guild.as_ref().and_then(|guild| {
            guild
                .my_join_sound_id
                .and_then(|id| sounds.iter().find(|sound| sound.id == id))
        });
        // The instance announces your join sound only where you may use the soundboard and speak.
        let is_join_sound_played_here = !matches!(
            block,
            Some(SoundboardBlock::NoPermission | SoundboardBlock::CannotSpeak)
        );
        let join_line = join_sound.map(|sound| {
            with_type(div(), TYPE_BODY_SM)
                .text_color(color(tokens.component_emoji_picker_empty_text))
                .child(if is_join_sound_played_here {
                    words.your_join_sound(&sound.name)
                } else {
                    words.your_join_sound_not_here(&sound.name)
                })
        });
        let reason: SharedString = match block {
            Some(SoundboardBlock::NotConnected) => words.blocks().not_connected(),
            Some(SoundboardBlock::NoPermission) => words.blocks().no_permission(),
            Some(SoundboardBlock::CannotSpeak) => words.blocks().cannot_speak(),
            Some(SoundboardBlock::Deafened) => words.blocks().deafened(),
            Some(SoundboardBlock::CoolingDown) => words.blocks().cooling_down(),
            None => words.hint(),
        }
        .into();

        let body = if sounds.is_empty() {
            self.render_empty(
                guild.as_ref().map_or("", |guild| guild.name.as_str()),
                catalogue,
                cx,
            )
        } else {
            let join_sound_id = guild.as_ref().and_then(|guild| guild.my_join_sound_id);
            let cells: Vec<AnyElement> = sounds
                .iter()
                .enumerate()
                .map(|(index, sound)| {
                    self.render_cell(
                        index,
                        sound,
                        join_sound_id == Some(sound.id),
                        block,
                        &reason,
                        window,
                        cx,
                    )
                })
                .collect();
            // Remaining share of the 2 s, from 0 to 1.
            #[allow(clippy::cast_precision_loss)]
            let remaining_share = (cooldown_ends_at.saturating_sub(now) as f32
                / MINIMUM_MILLISECONDS_BETWEEN_SOUNDS as f32)
                .clamp(0.0, 1.0);
            let is_cooling_down = block == Some(SoundboardBlock::CoolingDown);
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .gap(length(COMPONENT_EMOJI_PICKER_GAP))
                .child(
                    div()
                        .id("soundboard-grid")
                        .role(Role::Group)
                        .aria_label(title.clone())
                        .track_focus(&self.grid_focus)
                        .on_key_down(cx.listener(Self::on_grid_key_down))
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .grid()
                        .grid_cols(u16::try_from(COLUMNS).unwrap_or(3))
                        .gap(length(SPACE_2XS))
                        .children(cells),
                )
                // The bar and the sentence keep their room, so the grid never jumps.
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(length(SPACE_2XS))
                        .px(length(SPACE_2XS))
                        .child(
                            div()
                                .h(length(COMPONENT_VOICE_METER_HEIGHT))
                                .rounded(length(COMPONENT_VOICE_METER_RADIUS))
                                .when(is_cooling_down, |track| {
                                    track.bg(color(tokens.component_voice_meter_track)).child(
                                        div()
                                            .h_full()
                                            .w(relative(remaining_share))
                                            .rounded(length(COMPONENT_VOICE_METER_RADIUS))
                                            .bg(color(tokens.component_voice_meter_level_heard)),
                                    )
                                }),
                        )
                        .child(
                            with_type(div(), TYPE_BODY_SM)
                                .text_color(color(tokens.component_emoji_picker_empty_text))
                                .child(reason.clone()),
                        ),
                )
                .into_any_element()
        };

        let menu = self.menu.as_ref().map(|(menu, _)| menu.clone());
        let card = div()
            .id("soundboard")
            .role(Role::Dialog)
            .aria_label(title.clone())
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified() {
                    cx.stop_propagation();
                    cx.emit(SoundboardPopoverEvent::Dismissed {
                        should_return_focus: true,
                    });
                }
            }))
            // Tab stays inside, as in the web's popover, until Esc or a click outside.
            .on_action(cx.listener(|popover, _: &FocusNextControl, window, cx| {
                focus_next_inside(&popover.focus_handle, window, cx);
            }))
            .on_action(
                cx.listener(|popover, _: &FocusPreviousControl, window, cx| {
                    focus_previous_inside(&popover.focus_handle, window, cx);
                }),
            )
            .on_mouse_down_out(cx.listener(|popover, _, _, cx| {
                // A click in a sound's menu is outside the card, but not outside the popover.
                if popover.menu.is_none() {
                    cx.emit(SoundboardPopoverEvent::Dismissed {
                        should_return_focus: false,
                    });
                }
            }))
            .occlude()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_EMOJI_PICKER_GAP))
            .w(length(COMPONENT_EMOJI_PICKER_WIDTH))
            .max_h(self.max_height)
            .p(length(COMPONENT_EMOJI_PICKER_PADDING))
            .rounded(length(COMPONENT_EMOJI_PICKER_RADIUS))
            .border(length(COMPONENT_EMOJI_PICKER_BORDER_WIDTH))
            .border_color(color(tokens.component_emoji_picker_border))
            .bg(color(tokens.component_emoji_picker_background))
            .shadow(shadow(tokens.component_emoji_picker_shadow))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .gap(length(SPACE_3XS))
                    .px(length(SPACE_2XS))
                    .child(
                        with_type(div(), TYPE_CATEGORY)
                            .id("soundboard-title")
                            .role(Role::Heading)
                            .truncate()
                            .text_color(color(tokens.component_emoji_picker_heading_text))
                            .child(title),
                    )
                    .children(join_line),
            )
            .child(body)
            .child(polite_live_region(
                "soundboard-announcement",
                self.announcement.clone(),
            ))
            .children(menu);
        deferred(
            anchored()
                .anchor(gpui::Anchor::BottomLeft)
                .position(self.position)
                .snap_to_window_with_margin(px(COMPONENT_CONTEXT_MENU_VIEWPORT_MARGIN))
                .child(card),
        )
        .with_priority(1)
    }
}

#[cfg(test)]
#[path = "popover_tests.rs"]
mod tests;
