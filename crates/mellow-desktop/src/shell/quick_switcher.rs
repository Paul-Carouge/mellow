//! The quick switcher (`QuickSwitcher.svelte`, DESIGN.md §5.10): one field, results grouped by
//! kind, arrows and Enter. The field keeps focus; the active result is announced as the field's
//! active descendant. Ctrl+K again, Esc or the scrim closes it, and focus goes back.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, Context, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable as _, InteractiveElement as _, IntoElement, KeyDownEvent,
    MouseButton, ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, anchored, deferred, div, point, prelude::FluentBuilder as _,
    px,
};
use gpui_component::input::{Input, InputEvent, InputState};
use mellow_client_core::shared_logic::quick_switcher::{
    QuickSwitcherGroupKind, QuickSwitcherIcon, QuickSwitcherItem, QuickSwitcherPlace,
    QuickSwitcherSources, QuickSwitcherTarget, quick_switcher_results,
};
use mellow_protocol::{PublicUser, RelationshipKind};

use super::channel_sidebar::can_send_in;
use crate::components::{
    FocusNextControl, FocusPreviousControl, easing, focus_ring, icon, shadow, with_type,
};
use crate::home::conversation_display_name;
use crate::i18n::{Strings, strings};
use crate::session::Session;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CHANNEL_ROW_ICON_SIZE, COMPONENT_CHANNEL_ROW_UNREAD_PILL_WIDTH,
    COMPONENT_CONTEXT_MENU_BORDER_WIDTH, COMPONENT_DIALOG_ENTER_DURATION, COMPONENT_DIALOG_MARGIN,
    COMPONENT_DIALOG_RADIUS, COMPONENT_QUICK_SWITCHER_INPUT_GAP,
    COMPONENT_QUICK_SWITCHER_INPUT_HEIGHT, COMPONENT_QUICK_SWITCHER_INPUT_ICON_SIZE,
    COMPONENT_QUICK_SWITCHER_OFFSET_TOP, COMPONENT_QUICK_SWITCHER_OPTION_HEIGHT,
    COMPONENT_QUICK_SWITCHER_RESULTS_MAX_HEIGHT, COMPONENT_QUICK_SWITCHER_WIDTH, DARK_THEME,
    MOTION_EASING_SETTLE, RADIUS_MD, RADIUS_SM, SPACE_2XS, SPACE_MD, SPACE_SM, SPACE_XS, TYPE_BODY,
    TYPE_BODY_SM, TYPE_CATEGORY, TYPE_HEADING_LG,
};
use crate::theme::{animation_duration, color, length, theme, window_width_at_default_zoom};

/// What the switcher asks the shell to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickSwitcherEvent {
    /// Closed without choosing; focus is already back on the opener.
    Dismissed,
    /// A channel, a server (`channel_id` `None`), or a conversation at Home (`guild_id` `None`).
    Open {
        guild_id: Option<mellow_protocol::Snowflake>,
        channel_id: Option<mellow_protocol::Snowflake>,
    },
    /// Someone without a conversation yet: the shell opens one, then the switcher closes.
    OpenConversationWith {
        user_id: mellow_protocol::Snowflake,
        /// "Couldn't open the conversation with Ana.", the summary of a failure.
        failure: String,
    },
}

pub struct QuickSwitcher {
    session: Entity<Session>,
    input: Entity<InputState>,
    active_index: usize,
    error: Option<String>,
    container_focus: FocusHandle,
    opener: Option<FocusHandle>,
    _input_subscription: Subscription,
}

impl std::fmt::Debug for QuickSwitcher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QuickSwitcher")
            .field("active_index", &self.active_index)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<QuickSwitcherEvent> for QuickSwitcher {}

impl QuickSwitcher {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let opener = window.focused(cx);
        let placeholder = strings(cx).quick_switcher().placeholder();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        input.update(cx, |input, cx| input.focus(window, cx));
        let subscription = cx.subscribe(&input, |switcher, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                switcher.active_index = 0;
                switcher.error = None;
                cx.notify();
            }
        });
        Self {
            session,
            input,
            active_index: 0,
            error: None,
            container_focus: cx.focus_handle(),
            opener,
            _input_subscription: subscription,
        }
    }

    /// What went wrong opening a conversation, said in the switcher, which stays open.
    pub fn show_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.error = Some(message);
        cx.notify();
    }

    /// Gives focus back to whatever had it before the switcher opened.
    pub fn give_focus_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(opener) = self.opener.as_ref() {
            window.focus(opener, cx);
        }
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.give_focus_back(window, cx);
        cx.emit(QuickSwitcherEvent::Dismissed);
    }

    /// The results for what is typed now.
    #[must_use]
    pub fn results(&self, cx: &App) -> Vec<(QuickSwitcherGroupKind, Vec<QuickSwitcherItem>)> {
        let strings = strings(cx);
        let state = self.session.read(cx).state();
        let query = self.input.read(cx).value().to_string();
        let current_user_id = state.current_user().map(|user| user.id);
        let known_people: Vec<&PublicUser> = state
            .guilds()
            .iter()
            .flat_map(|guild| state.members(guild.id))
            .map(|member| &member.user)
            .collect();
        let is_friend = |user_id| {
            state
                .relationship(user_id)
                .is_some_and(|relationship| relationship.kind == RelationshipKind::Friend)
        };
        let has_unread = |channel_id| state.has_unread(channel_id);
        let can_send = |channel_id| can_send_in(state, channel_id);
        let conversation_name = |conversation: &mellow_protocol::DirectMessageChannel| {
            conversation_display_name(conversation, current_user_id, strings)
        };
        let sources = QuickSwitcherSources {
            guilds: state.guilds(),
            conversations: state.direct_message_channels(),
            known_people,
            current_user_id,
            is_friend: &is_friend,
            has_unread: &has_unread,
            can_send_in: &can_send,
            conversation_name: &conversation_name,
        };
        quick_switcher_results(&query, &sources)
            .into_iter()
            .map(|group| (group.kind, group.items))
            .collect()
    }

    fn open(&mut self, item: &QuickSwitcherItem, cx: &mut Context<Self>) {
        self.error = None;
        let event = match item.target {
            QuickSwitcherTarget::Channel {
                guild_id,
                channel_id,
            } => QuickSwitcherEvent::Open {
                guild_id: Some(guild_id),
                channel_id: Some(channel_id),
            },
            QuickSwitcherTarget::Server { guild_id } => QuickSwitcherEvent::Open {
                guild_id: Some(guild_id),
                channel_id: None,
            },
            QuickSwitcherTarget::Conversation { conversation_id }
            | QuickSwitcherTarget::Person {
                conversation_id: Some(conversation_id),
                ..
            } => QuickSwitcherEvent::Open {
                guild_id: None,
                channel_id: Some(conversation_id),
            },
            QuickSwitcherTarget::Person {
                user_id,
                conversation_id: None,
            } => QuickSwitcherEvent::OpenConversationWith {
                user_id,
                failure: strings(cx)
                    .quick_switcher()
                    .open_conversation_failed(&item.label),
            },
        };
        cx.emit(event);
    }

    /// Arrows move through every result, round the ends; Enter opens; Esc closes. Caught before
    /// the field, whose own arrows would move the caret.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.modified() {
            return;
        }
        match keystroke.key.as_str() {
            "escape" => {
                cx.stop_propagation();
                self.dismiss(window, cx);
            }
            "up" | "down" => {
                cx.stop_propagation();
                let count: usize = self.results(cx).iter().map(|(_, items)| items.len()).sum();
                if count > 0 {
                    let current = self.active_index.min(count - 1);
                    self.active_index = if keystroke.key == "down" {
                        (current + 1) % count
                    } else {
                        (current + count - 1) % count
                    };
                    cx.notify();
                }
            }
            "enter" => {
                cx.stop_propagation();
                let items: Vec<QuickSwitcherItem> = self
                    .results(cx)
                    .into_iter()
                    .flat_map(|(_, items)| items)
                    .collect();
                if let Some(item) = items.get(self.active_index.min(items.len().saturating_sub(1)))
                {
                    self.open(&item.clone(), cx);
                }
            }
            _ => {}
        }
    }
}

fn group_label(strings: Strings, kind: QuickSwitcherGroupKind) -> &'static str {
    let quick_switcher = strings.quick_switcher();
    match kind {
        QuickSwitcherGroupKind::Channels => quick_switcher.channels(),
        QuickSwitcherGroupKind::People => quick_switcher.people(),
        QuickSwitcherGroupKind::Servers => quick_switcher.servers(),
    }
}

fn place_label(strings: Strings, place: &QuickSwitcherPlace) -> String {
    match place {
        QuickSwitcherPlace::InServer(name) => name.clone(),
        QuickSwitcherPlace::DirectMessage => strings.quick_switcher().direct_message().to_owned(),
        QuickSwitcherPlace::Group => strings.groups().group().to_owned(),
        QuickSwitcherPlace::Server => strings.quick_switcher().server().to_owned(),
    }
}

fn icon_name(item_icon: QuickSwitcherIcon) -> IconName {
    match item_icon {
        QuickSwitcherIcon::Hash => IconName::Hash,
        QuickSwitcherIcon::Lock => IconName::Lock,
        QuickSwitcherIcon::At => IconName::At,
        QuickSwitcherIcon::Users => IconName::Users,
    }
}

impl Render for QuickSwitcher {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let strings = strings(cx);
        let viewport = window.viewport_size();
        let groups = self.results(cx);
        let count: usize = groups.iter().map(|(_, items)| items.len()).sum();
        let active_index = self.active_index.min(count.saturating_sub(1));
        let query = self.input.read(cx).value().trim().to_owned();
        let width = COMPONENT_QUICK_SWITCHER_WIDTH
            .min(window_width_at_default_zoom(window) - 2.0 * COMPONENT_DIALOG_MARGIN);
        let viewport_height = f32::from(viewport.height);
        let is_field_focused = self.input.read(cx).focus_handle(cx).is_focused(window);

        let mut index = 0;
        let mut rows: Vec<AnyElement> = Vec::new();
        for (kind, items) in groups {
            rows.push(
                with_type(div(), TYPE_CATEGORY)
                    .px(length(SPACE_XS))
                    .pt(length(SPACE_XS))
                    .pb(length(SPACE_2XS))
                    .text_color(color(tokens.color_text_muted))
                    .child(group_label(strings, kind))
                    .into_any_element(),
            );
            for item in items {
                let item_index = index;
                index += 1;
                let is_active = item_index == active_index;
                let ink = color(if is_active {
                    tokens.color_text_heading
                } else {
                    tokens.color_text_normal
                });
                let place = place_label(strings, &item.place);
                let label = if item.has_unread {
                    format!(
                        "{}, {place}, {}",
                        item.label,
                        strings.common().unread_suffix()
                    )
                } else {
                    format!("{}, {place}", item.label)
                };
                let chosen = item.clone();
                rows.push(
                    div()
                        .id(ElementId::named_usize("quick-switcher-option", item_index))
                        .role(Role::ListBoxOption)
                        .aria_label(label)
                        .aria_selected(is_active)
                        .when(
                            is_active,
                            gpui::StatefulInteractiveElement::aria_active_descendant,
                        )
                        .h(length(COMPONENT_QUICK_SWITCHER_OPTION_HEIGHT))
                        .flex()
                        .items_center()
                        .gap(length(SPACE_XS))
                        .px(length(SPACE_XS))
                        .rounded(length(RADIUS_SM))
                        .when(is_active, |row| row.bg(color(tokens.color_state_selected)))
                        .text_color(ink)
                        .cursor_pointer()
                        .on_mouse_move(cx.listener(move |switcher, _, _, cx| {
                            if switcher.active_index != item_index {
                                switcher.active_index = item_index;
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |switcher, _, _, cx| {
                            switcher.open(&chosen, cx);
                        }))
                        .child(icon(
                            icon_name(item.icon),
                            COMPONENT_CHANNEL_ROW_ICON_SIZE,
                            color(tokens.color_text_muted),
                        ))
                        .child(
                            with_type(div(), TYPE_BODY)
                                .min_w_0()
                                .truncate()
                                .child(item.label),
                        )
                        .when(item.has_unread, |row| {
                            row.child(
                                div()
                                    .flex_none()
                                    .size(length(COMPONENT_CHANNEL_ROW_UNREAD_PILL_WIDTH))
                                    .rounded_full()
                                    .bg(color(tokens.color_unread_pill)),
                            )
                        })
                        // A long server name gives way first: the result is what is chosen.
                        .child(
                            with_type(div(), TYPE_BODY_SM)
                                .ml_auto()
                                .max_w(gpui::relative(0.5))
                                .min_w_0()
                                .truncate()
                                .text_color(color(tokens.color_text_muted))
                                .child(place),
                        )
                        .into_any_element(),
                );
            }
        }
        let status_text = if count == 0 {
            strings.quick_switcher().no_results(&query)
        } else {
            strings
                .quick_switcher()
                .result_count(i64::try_from(count).unwrap_or(i64::MAX))
        };
        // Dark floating layers get a 1 px divider outline (`dark:[outline…]` on the web).
        let is_dark = *tokens == DARK_THEME;

        let card = div()
            .id("quick-switcher")
            .role(Role::Dialog)
            .aria_label(strings.quick_switcher().label())
            .track_focus(&self.container_focus)
            .capture_key_down(cx.listener(Self::on_key_down))
            // Tab has nowhere else to go inside: the field keeps focus.
            .on_action(|_: &FocusNextControl, _, cx| cx.stop_propagation())
            .on_action(|_: &FocusPreviousControl, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .mt(length(
                COMPONENT_QUICK_SWITCHER_OFFSET_TOP * viewport_height,
            ))
            .w(length(width))
            .p(length(SPACE_MD))
            .rounded(length(COMPONENT_DIALOG_RADIUS))
            .bg(color(tokens.component_dialog_background))
            .text_color(color(tokens.component_dialog_body_text))
            .shadow(shadow(tokens.elevation_high))
            .when(is_dark, |card| {
                card.border(length(COMPONENT_CONTEXT_MENU_BORDER_WIDTH))
                    .border_color(color(tokens.color_border_divider))
            })
            .child(
                // The header's search field, grown: the magnifier leads the typed text.
                div()
                    .relative()
                    .h(length(COMPONENT_QUICK_SWITCHER_INPUT_HEIGHT))
                    .flex()
                    .items_center()
                    .gap(length(COMPONENT_QUICK_SWITCHER_INPUT_GAP))
                    .pl(length(SPACE_MD))
                    .pr(length(SPACE_MD))
                    .rounded(length(RADIUS_MD))
                    .bg(color(tokens.color_surface_input))
                    .child(icon(
                        IconName::MagnifyingGlass,
                        COMPONENT_QUICK_SWITCHER_INPUT_ICON_SIZE,
                        color(tokens.component_quick_switcher_input_icon),
                    ))
                    .child(
                        with_type(div(), TYPE_HEADING_LG)
                            .flex_1()
                            .min_w_0()
                            .text_color(color(tokens.color_text_heading))
                            .child(
                                Input::new(&self.input)
                                    .appearance(false)
                                    .aria_label(strings.quick_switcher().placeholder()),
                            ),
                    )
                    .when(
                        is_field_focused && window.last_input_was_keyboard(),
                        |field| field.child(focus_ring(RADIUS_MD, cx)),
                    ),
            )
            .child(
                div()
                    .id("quick-switcher-results")
                    .role(Role::ListBox)
                    .aria_label(strings.quick_switcher().results())
                    .mt(length(SPACE_SM))
                    .max_h(length(
                        COMPONENT_QUICK_SWITCHER_RESULTS_MAX_HEIGHT * viewport_height,
                    ))
                    .overflow_y_scroll()
                    .children(rows),
            )
            .child(
                // Outside the list, and announced: the count, or why there is nothing to choose.
                with_type(div(), TYPE_BODY)
                    .id("quick-switcher-status")
                    .role(Role::Status)
                    .aria_label(status_text.clone())
                    .when(count > 0, |status| status.h_0().overflow_hidden())
                    .when(count == 0, |status| {
                        status
                            .px(length(SPACE_XS))
                            .pt(length(SPACE_XS))
                            .pb(length(SPACE_2XS))
                            .text_color(color(tokens.color_text_muted))
                            .child(status_text)
                    }),
            )
            .when_some(self.error.clone(), |card, error| {
                card.child(
                    with_type(div(), TYPE_BODY)
                        .id("quick-switcher-error")
                        .role(Role::Alert)
                        .aria_label(SharedString::from(error.clone()))
                        .mt(length(SPACE_SM))
                        .text_color(color(tokens.component_dialog_error_text))
                        .child(error),
                )
            })
            .child(
                with_type(div(), TYPE_BODY_SM)
                    .mt(length(SPACE_SM))
                    .text_color(color(tokens.color_text_muted))
                    .child(strings.quick_switcher().help()),
            )
            // GPUI cannot scale a box yet, so the card settles in by fading only (as the dialog).
            .with_animation(
                "quick-switcher-enter",
                Animation::new(
                    animation_duration(COMPONENT_DIALOG_ENTER_DURATION, cx)
                        .max(Duration::from_millis(1)),
                )
                .with_easing(|progress| easing(MOTION_EASING_SETTLE).sample(progress)),
                |card, progress| card.opacity(progress.min(1.0)),
            );

        let scrim = div()
            .id("quick-switcher-scrim")
            .occlude()
            .w(px(f32::from(viewport.width)))
            .h(px(viewport_height))
            .flex()
            .flex_col()
            .items_center()
            .bg(color(tokens.component_dialog_scrim))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|switcher, _, window, cx| switcher.dismiss(window, cx)),
            )
            .child(card);
        deferred(anchored().position(point(px(0.0), px(0.0))).child(scrim)).with_priority(2)
    }
}
