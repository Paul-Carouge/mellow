//! The search field (`MessageSearchField.svelte`, DESIGN.md 5.12): a combobox owning a list of
//! filter suggestions. Focus stays in the field and the active option is its active descendant.
//! Enter searches at once unless an option was chosen with the arrows (a value's first option is
//! active by itself, so "from:a" then Enter picks the first person).

use gpui::{
    AnyElement, App, Context, ElementId, Focusable as _, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, deferred, div,
    prelude::FluentBuilder as _,
};
use gpui_component::input::Input;
use mellow_client_core::shared_logic::channel_order::channel_sections;
use mellow_client_core::shared_logic::format::{display_name, format_calendar_day};
use mellow_client_core::shared_logic::search_query::{
    SearchCaretContext, SearchFilterKey, SearchHasValue, channel_filter_value,
    replace_in_search_query, search_caret_context,
};
use mellow_client_core::shared_logic::search_suggestions::{
    RelativeDay, SearchChannelSource, SearchSuggestion, SearchSuggestionSources,
    search_suggestions, suggestion_insertion,
};
use mellow_protocol::{ChannelKind, PublicUser, RelationshipKind};

use super::MessageSearchView;
use crate::components::{Avatar, AvatarPerson, IconButton, focus_ring, icon, shadow, with_type};
use crate::home::conversation_display_name;
use crate::i18n::{Strings, strings};
use crate::messages::{local_day_of, picture_animation, picture_for_box};
use crate::session::now_unix_milliseconds;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CHAT_HEADER_BUTTON_SIZE, COMPONENT_CONTEXT_MENU_BORDER_WIDTH,
    COMPONENT_CONTEXT_MENU_ITEM_PADDING_INLINE, COMPONENT_CONTEXT_MENU_ITEM_RADIUS,
    COMPONENT_SEARCH_FIELD_ICON_SIZE, COMPONENT_SEARCH_FIELD_MIN_WIDTH,
    COMPONENT_SEARCH_FIELD_WIDTH, COMPONENT_SEARCH_FIELD_WIDTH_ACTIVE,
    COMPONENT_SEARCH_SUGGESTION_AVATAR_SIZE, COMPONENT_SEARCH_SUGGESTION_HEIGHT,
    COMPONENT_SEARCH_SUGGESTION_ICON_SIZE, COMPONENT_SEARCH_SUGGESTIONS_MAX_HEIGHT,
    COMPONENT_SEARCH_SUGGESTIONS_PADDING, COMPONENT_SEARCH_SUGGESTIONS_RADIUS,
    COMPONENT_SEARCH_SUGGESTIONS_WIDTH, RADIUS_SM, RADIUS_XS, SIZE_HIT_DESKTOP_MIN, SPACE_2XS,
    SPACE_3XS, SPACE_XS, TYPE_BODY, TYPE_BODY_SM, TYPE_CATEGORY,
};
use crate::theme::{color, length, theme};

/// Where the field is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldPlacement {
    /// In the chat header, growing on focus, where the results dock beside the conversation.
    Header,
    /// At the top of the results, over the conversation in a narrow window.
    Sheet,
}

/// The shortcut's hint at the end of the header's field at rest.
const SEARCH_KEY_HINT: &str = if cfg!(target_os = "macos") {
    "⌘ F"
} else {
    "Ctrl F"
};
/// `aria-keyshortcuts` of the field.
const SEARCH_KEY_SHORTCUTS: &str = if cfg!(target_os = "macos") {
    "Meta+F"
} else {
    "Control+F"
};
/// The quick switcher's hint on the "Go to" option.
const GO_TO_KEY_HINT: &str = if cfg!(target_os = "macos") {
    "⌘ K"
} else {
    "Ctrl K"
};

fn key_icon(key: SearchFilterKey) -> IconName {
    match key {
        SearchFilterKey::From => IconName::PencilSimple,
        SearchFilterKey::Mentions => IconName::At,
        SearchFilterKey::In => IconName::Hash,
        SearchFilterKey::Has => IconName::File,
        SearchFilterKey::Before | SearchFilterKey::After | SearchFilterKey::During => {
            IconName::Clock
        }
    }
}

fn has_value_name(strings: Strings, value: SearchHasValue) -> &'static str {
    let values = strings.search().has_values();
    match value {
        SearchHasValue::Link => values.link(),
        SearchHasValue::File => values.file(),
        SearchHasValue::Image => values.image(),
        SearchHasValue::Pinned => values.pinned(),
    }
}

pub(super) fn filter_description(
    strings: Strings,
    key: SearchFilterKey,
    is_in_server: bool,
) -> &'static str {
    let descriptions = strings.search().filter_descriptions();
    match key {
        SearchFilterKey::From => descriptions.from(),
        SearchFilterKey::Mentions => descriptions.mentions(),
        SearchFilterKey::In if is_in_server => descriptions.r#in(),
        SearchFilterKey::In => strings.search().in_conversation_description(),
        SearchFilterKey::Has => descriptions.has(),
        SearchFilterKey::Before => descriptions.before(),
        SearchFilterKey::After => descriptions.after(),
        SearchFilterKey::During => descriptions.during(),
    }
}

fn relative_day_name(strings: Strings, day: RelativeDay) -> &'static str {
    let days = strings.search().relative_days();
    match day {
        RelativeDay::Today => days.today(),
        RelativeDay::Yesterday => days.yesterday(),
        RelativeDay::AWeekAgo => days.a_week_ago(),
        RelativeDay::AMonthAgo => days.a_month_ago(),
        RelativeDay::Typed => days.typed(),
    }
}

/// "pizza" or "from:a": whether `dismissed_at` still hides the list for this text and caret.
fn is_dismissed(dismissed_at: Option<&(String, usize)>, draft: &str, caret: usize) -> bool {
    dismissed_at.is_some_and(|(text, at)| text == draft && *at == caret)
}

/// What a suggestion's option shows: its name, its detail and how it leads.
struct OptionLook {
    main: String,
    detail: String,
    is_key: bool,
    leading: OptionLeading,
}

enum OptionLeading {
    Icon(IconName),
    Person(PublicUser),
    Nothing,
}

/// People and channels the suggestions name, only gathered for the filter being typed.
struct SuggestionSources {
    people: Vec<PublicUser>,
    channels: Vec<SearchChannelSource>,
}

impl MessageSearchView {
    fn suggestion_sources(&self, context: &SearchCaretContext, cx: &App) -> SuggestionSources {
        let state = self.session.read(cx).state();
        let guild = self
            .search
            .scope_guild_id()
            .and_then(|guild_id| state.guild(guild_id));
        let wanted_key = match context {
            SearchCaretContext::Value { key, .. } => Some(*key),
            _ => None,
        };
        let people = match (wanted_key, guild) {
            (Some(SearchFilterKey::From | SearchFilterKey::Mentions), Some(guild)) => state
                .members(guild.id)
                .iter()
                .map(|member| member.user.clone())
                .collect(),
            (Some(SearchFilterKey::From | SearchFilterKey::Mentions), None) => {
                let mut people: Vec<PublicUser> = Vec::new();
                let mut push = |user: &PublicUser| {
                    if !people.iter().any(|known| known.id == user.id) {
                        people.push(user.clone());
                    }
                };
                if let Some(me) = state.current_user() {
                    push(&PublicUser {
                        id: me.id,
                        username: me.username.clone(),
                        display_name: me.display_name.clone(),
                        avatar: me.avatar,
                        banner: me.banner,
                    });
                }
                for conversation in state.direct_message_channels() {
                    conversation.recipients.iter().for_each(&mut push);
                }
                for relationship in state.relationships_of_kind(RelationshipKind::Friend) {
                    push(&relationship.user);
                }
                people
            }
            _ => Vec::new(),
        };
        let channels = match (wanted_key, guild) {
            (Some(SearchFilterKey::In), Some(guild)) => channel_sections(&guild.channels)
                .into_iter()
                .flat_map(|section| section.channels)
                // Voice channels hold no messages.
                .filter(|channel| channel.kind == ChannelKind::Text)
                .map(|channel| SearchChannelSource {
                    id: channel.id,
                    value: channel_filter_value(channel.id, &channel.name),
                    name: channel.name,
                    is_conversation: false,
                })
                .collect(),
            (Some(SearchFilterKey::In), None) => {
                let current_user_id = state.current_user().map(|user| user.id);
                state
                    .direct_message_channels()
                    .iter()
                    .map(|conversation| SearchChannelSource {
                        id: conversation.id,
                        name: conversation_display_name(conversation, current_user_id, strings(cx)),
                        is_conversation: true,
                        value: conversation.id.raw().to_string(),
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        SuggestionSources { people, channels }
    }

    fn option_look(&self, suggestion: &SearchSuggestion<'_>, strings: Strings) -> OptionLook {
        let is_in_server = self.search.scope_guild_id().is_some();
        match suggestion {
            SearchSuggestion::Key(key) => OptionLook {
                main: format!("{}:", key.as_str()),
                detail: filter_description(strings, *key, is_in_server).to_owned(),
                is_key: true,
                leading: OptionLeading::Icon(key_icon(*key)),
            },
            SearchSuggestion::Person { user, .. } => OptionLook {
                main: display_name(user).to_owned(),
                detail: user.username.clone(),
                is_key: false,
                leading: OptionLeading::Person((*user).clone()),
            },
            SearchSuggestion::Channel(channel) => OptionLook {
                main: channel.name.clone(),
                detail: String::new(),
                is_key: false,
                leading: OptionLeading::Icon(if channel.is_conversation {
                    IconName::At
                } else {
                    IconName::Hash
                }),
            },
            SearchSuggestion::Has(value) => OptionLook {
                main: has_value_name(strings, *value).to_owned(),
                detail: format!("has:{}", value.as_str()),
                is_key: false,
                leading: OptionLeading::Nothing,
            },
            SearchSuggestion::Day {
                day, relative_day, ..
            } => OptionLook {
                main: relative_day_name(strings, *relative_day).to_owned(),
                detail: format_calendar_day(crate::messages::locale(strings), day)
                    .unwrap_or_else(|| day.clone()),
                is_key: false,
                leading: OptionLeading::Nothing,
            },
        }
    }

    /// The suggestions for the caret now; empty while hidden. With the "Go to" door last while
    /// the field is empty.
    fn listed_options(
        &self,
        window: &Window,
        cx: &App,
    ) -> (
        SearchCaretContext,
        Vec<OptionLook>,
        Vec<(String, bool)>,
        bool,
    ) {
        let input = self.input.read(cx);
        let draft = input.value().to_string();
        let caret = input.cursor();
        let context = search_caret_context(&draft, caret);
        let has_focus = input.focus_handle(cx).is_focused(window);
        let is_hidden = !has_focus || is_dismissed(self.dismissed_at.as_ref(), &draft, caret);
        // Every filter is listed only in an empty field: between words, the first letters of one
        // bring it, so the list does not cover the chat after each choice.
        let lists_suggestions = !is_hidden
            && (!matches!(context, SearchCaretContext::Between { .. }) || draft.trim().is_empty());
        let mut looks = Vec::new();
        let mut insertions = Vec::new();
        if lists_suggestions {
            let sources = self.suggestion_sources(&context, cx);
            let suggestions = search_suggestions(
                &context,
                SearchSuggestionSources {
                    people: &sources.people,
                    channels: &sources.channels,
                    today: local_day_of(now_unix_milliseconds()),
                },
            );
            let strings = strings(cx);
            for suggestion in &suggestions {
                looks.push(self.option_look(suggestion, strings));
                insertions.push(suggestion_insertion(suggestion));
            }
        }
        let offers_go_to = !is_hidden && draft.trim().is_empty();
        (context, looks, insertions, offers_go_to)
    }

    /// The option at `index` was chosen: its text goes in at the caret, and a finished part
    /// searches at once; the last option past the suggestions is "Go to".
    pub(super) fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (context, _, insertions, offers_go_to) = self.listed_options(window, cx);
        if index == insertions.len() && offers_go_to {
            cx.emit(super::MessageSearchEvent::OpenQuickSwitcher);
            return;
        }
        let Some((text, ends_part)) = insertions.get(index).cloned() else {
            return;
        };
        let (start, end) = match context {
            SearchCaretContext::Between { start, end }
            | SearchCaretContext::Key { start, end, .. }
            | SearchCaretContext::Value { start, end, .. } => (start, end),
            SearchCaretContext::None => return,
        };
        let draft = self.input.read(cx).value().to_string();
        let Some((query, caret)) = replace_in_search_query(&draft, start, end, &text, ends_part)
        else {
            return;
        };
        self.set_field_text(&query, Some(caret), window, cx);
        if ends_part {
            self.search_now(query, cx);
        } else {
            self.note_typed(query, caret, cx);
        }
        cx.notify();
    }

    /// Arrows move through the options, round the ends; Enter chooses or searches; Esc hides the
    /// list, then empties the header's field. Caught before the field, whose arrows move the caret.
    fn on_field_key_down(
        &mut self,
        placement: FieldPlacement,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.modified() {
            return;
        }
        let (_, looks, _, offers_go_to) = self.listed_options(window, cx);
        let option_count = looks.len() + usize::from(offers_go_to);
        let active = self.active_index.filter(|index| *index < option_count);
        let draft = self.input.read(cx).value().to_string();
        let caret = self.input.read(cx).cursor();
        match keystroke.key.as_str() {
            "up" | "down" if option_count > 0 => {
                cx.stop_propagation();
                let last = option_count - 1;
                self.active_index = Some(if keystroke.key == "down" {
                    active.map_or(0, |index| if index >= last { 0 } else { index + 1 })
                } else {
                    active.map_or(last, |index| if index == 0 { last } else { index - 1 })
                });
                cx.notify();
            }
            "enter" => {
                cx.stop_propagation();
                if let Some(index) = active {
                    self.choose(index, window, cx);
                } else {
                    self.search_now(draft.clone(), cx);
                    self.dismissed_at = Some((draft, caret));
                }
            }
            "escape" if option_count > 0 => {
                // The results' Esc would close them otherwise.
                cx.stop_propagation();
                self.dismissed_at = Some((draft, caret));
                cx.notify();
            }
            "escape" if !draft.is_empty() && placement == FieldPlacement::Header => {
                cx.stop_propagation();
                self.search.clear();
                self.after_cleared(window, cx);
            }
            _ => {}
        }
    }

    /// The field, where `placement` says, with its suggestions under it.
    #[allow(clippy::too_many_lines)] // One combobox and its list, in the order they are drawn.
    pub fn render_field(
        &mut self,
        placement: FieldPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let search_strings = catalogue.search();
        let state = self.session.read(cx).state();
        let guild_name = self
            .search
            .scope_guild_id()
            .and_then(|guild_id| state.guild(guild_id))
            .map(|guild| guild.name.clone());
        let input = self.input.read(cx);
        let draft = input.value().to_string();
        let has_focus = input.focus_handle(cx).is_focused(window);
        // The header's field at rest is too narrow for its scope, and a sentence cut mid-word
        // reads broken: the scope shows once focus widens it, and is always in the field's name.
        let placeholder: SharedString = if placement == FieldPlacement::Header && !has_focus {
            search_strings.placeholder().into()
        } else {
            match &guild_name {
                Some(name) => search_strings.placeholder_in_server(name).into(),
                None => search_strings.placeholder_in_conversations().into(),
            }
        };
        if self.shown_placeholder != placeholder {
            self.shown_placeholder = placeholder.clone();
            self.input.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx);
            });
        }
        let label: SharedString = match &guild_name {
            Some(name) => search_strings.field_label_in_server(name).into(),
            None => search_strings.field_label_in_conversations().into(),
        };

        let (context, looks, _, offers_go_to) = self.listed_options(window, cx);
        let option_count = looks.len() + usize::from(offers_go_to);
        // A new list starts on its first option only for values: there Enter means "this one".
        let context_key = format!("{context:?}");
        if context_key != self.shown_context_key {
            self.shown_context_key = context_key;
            self.active_index = matches!(context, SearchCaretContext::Value { .. }).then_some(0);
        }
        let active = self.active_index.filter(|index| *index < option_count);
        let group_label = match &context {
            SearchCaretContext::Value { key, .. } => {
                let groups = search_strings.groups();
                match key {
                    SearchFilterKey::From | SearchFilterKey::Mentions => groups.people(),
                    SearchFilterKey::In if guild_name.is_some() => groups.channels(),
                    SearchFilterKey::In => groups.conversations(),
                    SearchFilterKey::Has => groups.contents(),
                    SearchFilterKey::Before | SearchFilterKey::After | SearchFilterKey::During => {
                        groups.days()
                    }
                }
            }
            _ => search_strings.groups().filters(),
        };
        let is_wide = placement == FieldPlacement::Header && (has_focus || !draft.is_empty());

        let mut options: Vec<AnyElement> = Vec::with_capacity(option_count + 1);
        for (index, look) in looks.into_iter().enumerate() {
            options.push(Self::render_option(
                index,
                look,
                active == Some(index),
                window,
                cx,
            ));
        }
        if offers_go_to {
            options.push(
                div()
                    .my(length(SPACE_2XS))
                    .h(gpui::px(1.0))
                    .bg(color(tokens.color_border_divider))
                    .into_any_element(),
            );
            let index = option_count - 1;
            options.push(Self::render_option(
                index,
                OptionLook {
                    main: search_strings.go_to().to_owned(),
                    detail: GO_TO_KEY_HINT.to_owned(),
                    is_key: false,
                    leading: OptionLeading::Nothing,
                },
                active == Some(index),
                window,
                cx,
            ));
        }

        let list = (option_count > 0).then(|| {
            deferred(
                div()
                    .id("search-suggestions")
                    .occlude()
                    .absolute()
                    .top(length(
                        if placement == FieldPlacement::Header {
                            COMPONENT_CHAT_HEADER_BUTTON_SIZE
                        } else {
                            SIZE_HIT_DESKTOP_MIN.max(COMPONENT_CHAT_HEADER_BUTTON_SIZE)
                        } + SPACE_2XS,
                    ))
                    .when(placement == FieldPlacement::Header, |list| {
                        list.right_0().w(length(COMPONENT_SEARCH_SUGGESTIONS_WIDTH))
                    })
                    .when(placement == FieldPlacement::Sheet, |list| {
                        list.left_0().right_0()
                    })
                    .max_h(length(COMPONENT_SEARCH_SUGGESTIONS_MAX_HEIGHT))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded(length(COMPONENT_SEARCH_SUGGESTIONS_RADIUS))
                    .border(length(COMPONENT_CONTEXT_MENU_BORDER_WIDTH))
                    .border_color(color(tokens.component_context_menu_border))
                    .bg(color(tokens.component_search_suggestions_background))
                    .shadow(shadow(tokens.component_context_menu_shadow))
                    // Keeps the field's focus: a press on the list is no click outside.
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    // shortcut: a press outside that keeps the field's focus hides the list here;
                    // the shared outside-click helper (refactor/desktop-outside-click-popovers) may replace it.
                    .on_mouse_down_out(cx.listener(|view, _, window, cx| {
                        let input = view.input.read(cx);
                        if input.focus_handle(cx).is_focused(window) {
                            view.dismissed_at = Some((input.value().to_string(), input.cursor()));
                            cx.notify();
                        }
                    }))
                    .child(
                        with_type(div(), TYPE_CATEGORY)
                            .flex_none()
                            .px(length(COMPONENT_CONTEXT_MENU_ITEM_PADDING_INLINE))
                            .pt(length(COMPONENT_SEARCH_SUGGESTIONS_PADDING))
                            .pb(length(SPACE_2XS))
                            .text_color(color(tokens.component_search_suggestion_group_text))
                            .child(group_label),
                    )
                    .child(
                        div()
                            .id("search-suggestions-list")
                            .role(Role::ListBox)
                            .aria_label(group_label)
                            .min_h_0()
                            .overflow_y_scroll()
                            .px(length(COMPONENT_SEARCH_SUGGESTIONS_PADDING))
                            .pb(length(COMPONENT_SEARCH_SUGGESTIONS_PADDING))
                            .children(options),
                    ),
            )
            .with_priority(1)
        });

        let suggestions_count = if option_count > 0 {
            search_strings.suggestions_count(i64::try_from(option_count).unwrap_or(i64::MAX))
        } else {
            String::new()
        };
        let clear_view = cx.entity().downgrade();
        div()
            .relative()
            .min_w_0()
            .when(placement == FieldPlacement::Header, |field| {
                field
                    .flex_shrink(1.0)
                    .min_w(length(COMPONENT_SEARCH_FIELD_MIN_WIDTH))
                    .w(length(if is_wide {
                        COMPONENT_SEARCH_FIELD_WIDTH_ACTIVE
                    } else {
                        COMPONENT_SEARCH_FIELD_WIDTH
                    }))
            })
            .when(placement == FieldPlacement::Sheet, gpui::Styled::w_full)
            .child(
                div()
                    .id(match placement {
                        FieldPlacement::Header => "search-field-header",
                        FieldPlacement::Sheet => "search-field-sheet",
                    })
                    .role(Role::ComboBox)
                    .aria_label(label.clone())
                    .aria_expanded(option_count > 0)
                    .relative()
                    .capture_key_down(cx.listener(move |view, event: &KeyDownEvent, window, cx| {
                        view.on_field_key_down(placement, event, window, cx);
                    }))
                    .h(length(match placement {
                        FieldPlacement::Header => COMPONENT_CHAT_HEADER_BUTTON_SIZE,
                        FieldPlacement::Sheet => {
                            SIZE_HIT_DESKTOP_MIN.max(COMPONENT_CHAT_HEADER_BUTTON_SIZE)
                        }
                    }))
                    .flex()
                    .items_center()
                    .gap(length(SPACE_XS))
                    .pl(length(SPACE_XS))
                    .pr(length(SPACE_3XS))
                    .rounded(length(RADIUS_SM))
                    .bg(color(tokens.color_surface_input))
                    .child(icon(
                        IconName::MagnifyingGlass,
                        COMPONENT_SEARCH_FIELD_ICON_SIZE,
                        color(tokens.component_search_field_icon),
                    ))
                    .child(
                        with_type(div(), TYPE_BODY)
                            .flex_1()
                            .min_w_0()
                            .text_color(color(tokens.color_text_normal))
                            .child(Input::new(&self.input).appearance(false).aria_label(label)),
                    )
                    .when(!draft.is_empty(), |field| {
                        field.child(
                            IconButton::new("search-clear", IconName::X, search_strings.clear())
                                .sizes(
                                    COMPONENT_CHAT_HEADER_BUTTON_SIZE - 2.0 * SPACE_2XS,
                                    COMPONENT_SEARCH_FIELD_ICON_SIZE,
                                )
                                .on_click(move |window, cx| {
                                    let _ = clear_view.update(cx, |view, cx| {
                                        view.search.clear();
                                        view.after_cleared(window, cx);
                                        view.input.update(cx, |input, cx| input.focus(window, cx));
                                    });
                                }),
                        )
                    })
                    .when(
                        draft.is_empty() && placement == FieldPlacement::Header && !has_focus,
                        |field| {
                            field.child(
                                with_type(div(), TYPE_BODY_SM)
                                    .id("search-key-hint")
                                    .aria_label(SEARCH_KEY_SHORTCUTS)
                                    .flex_none()
                                    .mr(length(SPACE_2XS))
                                    .px(length(SPACE_2XS))
                                    .rounded(length(RADIUS_XS))
                                    .border_1()
                                    .border_color(color(tokens.color_border_divider))
                                    .text_color(color(tokens.color_text_muted))
                                    .child(SEARCH_KEY_HINT),
                            )
                        },
                    )
                    .when(has_focus && window.last_input_was_keyboard(), |field| {
                        field.child(focus_ring(RADIUS_SM, cx))
                    }),
            )
            .children(list)
            .child(
                div()
                    .id("search-suggestions-status")
                    .role(Role::Status)
                    .aria_label(suggestions_count)
                    .size_0()
                    .overflow_hidden(),
            )
            .into_any_element()
    }

    fn render_option(
        index: usize,
        look: OptionLook,
        is_active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let text_colour = color(if is_active {
            tokens.component_context_menu_item_text_focus
        } else {
            tokens.component_context_menu_item_text
        });
        let detail_colour = color(if is_active {
            tokens.component_context_menu_item_text_focus
        } else if look.is_key {
            tokens.color_text_secondary
        } else {
            tokens.component_search_suggestion_detail_text
        });
        let accessible_label = if look.detail.is_empty() {
            look.main.clone()
        } else {
            format!("{}, {}", look.main, look.detail)
        };
        let leading: Option<AnyElement> = match look.leading {
            OptionLeading::Icon(name) => Some(
                icon(
                    name,
                    COMPONENT_SEARCH_SUGGESTION_ICON_SIZE,
                    color(tokens.color_text_muted),
                )
                .into_any_element(),
            ),
            OptionLeading::Person(user) => {
                let picture = user.avatar.as_ref().map(|avatar| {
                    picture_for_box(
                        mellow_client_core::DownloadableFile::ProfileImage(avatar.id),
                        COMPONENT_SEARCH_SUGGESTION_AVATAR_SIZE,
                        COMPONENT_SEARCH_SUGGESTION_AVATAR_SIZE,
                        picture_animation(cx),
                        window,
                    )
                });
                Some(
                    Avatar::new(
                        AvatarPerson::from_user(&user, picture),
                        COMPONENT_SEARCH_SUGGESTION_AVATAR_SIZE,
                    )
                    .into_any_element(),
                )
            }
            OptionLeading::Nothing => None,
        };
        div()
            .id(ElementId::named_usize("search-suggestion", index))
            .role(Role::ListBoxOption)
            .aria_label(accessible_label)
            .aria_selected(is_active)
            .when(
                is_active,
                gpui::StatefulInteractiveElement::aria_active_descendant,
            )
            .min_h(length(COMPONENT_SEARCH_SUGGESTION_HEIGHT))
            .flex()
            .items_center()
            .gap(length(SPACE_XS))
            .px(length(COMPONENT_CONTEXT_MENU_ITEM_PADDING_INLINE))
            .py(length(SPACE_3XS))
            .rounded(length(COMPONENT_CONTEXT_MENU_ITEM_RADIUS))
            .when(is_active, |option| {
                option.bg(color(tokens.component_context_menu_item_background_focus))
            })
            .text_color(text_colour)
            .cursor_pointer()
            .on_mouse_move(cx.listener(move |view, _, _, cx| {
                if view.active_index != Some(index) {
                    view.active_index = Some(index);
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.choose(index, window, cx);
            }))
            .children(leading)
            // A filter's name stays whole and its description follows it, truncating first, as
            // in the "/" menu; a value's name comes first.
            .child(
                with_type(div(), TYPE_BODY)
                    .truncate()
                    .when(look.is_key, |name| {
                        name.flex_none().font_weight(gpui::FontWeight::SEMIBOLD)
                    })
                    .when(!look.is_key, |name| name.flex_1().min_w_0())
                    .child(look.main),
            )
            .when(!look.detail.is_empty(), |option| {
                option.child(
                    with_type(div(), TYPE_BODY_SM)
                        .truncate()
                        .when(look.is_key, |detail| detail.flex_1().min_w_0())
                        .when(!look.is_key, |detail| {
                            detail.flex_none().max_w(gpui::relative(0.5))
                        })
                        .text_color(detail_colour)
                        .child(look.detail),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_hides_the_list_only_for_the_text_and_caret_it_was_pressed_at() {
        let dismissed = Some(("from:a".to_owned(), 6));
        assert!(is_dismissed(dismissed.as_ref(), "from:a", 6));
        assert!(!is_dismissed(dismissed.as_ref(), "from:ad", 7));
        assert!(!is_dismissed(dismissed.as_ref(), "from:a", 5));
        assert!(!is_dismissed(None, "from:a", 6));
    }
}
