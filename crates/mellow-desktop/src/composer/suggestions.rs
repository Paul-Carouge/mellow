//! The ":na" and "@na" suggestions above the composer (`ComposerSuggestions.svelte`, DESIGN.md
//! §5.5, §6.58): they follow the caret, Up and Down wrap, Enter or Tab inserts, Esc hides them
//! until the word changes, and a click inserts without taking focus from the field. "@na" offers
//! people, then `@everyone` and the roles the writer may mention (ADR-0040).

use std::sync::Arc;

use gpui::{
    App, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, Role,
    SharedString, StatefulInteractiveElement as _, Styled, Window, div,
    prelude::FluentBuilder as _, px, relative,
};
use mellow_client_core::shared_logic::emoji_suggestions::{
    EmojiSuggestion, EmojiSuggestionQuery, emoji_suggestion_query, suggest_emoji,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::mentions::{
    EVERYONE_MENTION_WORD, MentionSuggestionQuery, mention_suggestion_query, suggest_mentions,
};
use mellow_client_core::shared_logic::role_mentions::{
    RoleMentionSuggestion, suggest_role_mentions,
};
use mellow_protocol::{PublicUser, Role as ServerRole, Snowflake};

use crate::components::{Avatar, AvatarPerson, icon, next_menu_item_index, shadow, with_type};
use crate::i18n;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_COMPOSER_SUGGESTIONS_AVATAR_SIZE, COMPONENT_CONTEXT_MENU_BORDER_WIDTH,
    COMPONENT_CONTEXT_MENU_ITEM_HEIGHT, COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE,
    COMPONENT_CONTEXT_MENU_ITEM_PADDING_INLINE, COMPONENT_CONTEXT_MENU_ITEM_RADIUS,
    COMPONENT_CONTEXT_MENU_PADDING, COMPONENT_CONTEXT_MENU_RADIUS,
    COMPONENT_CUSTOM_EMOJI_SUGGESTION_MAX_HEIGHT, COMPONENT_CUSTOM_EMOJI_SUGGESTION_WIDTH,
    COMPONENT_ROLES_ROW_DOT_SIZE, SPACE_2XS, SPACE_XS, TYPE_BODY, TYPE_CATEGORY,
};
use crate::theme::{color, length, theme};

/// The word being completed before the caret: ":" opens emoji, "@" people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuggestionQuery {
    Emoji(EmojiSuggestionQuery),
    Mention(MentionSuggestionQuery),
}

impl SuggestionQuery {
    /// The query at `caret` (a byte offset); a word is one or the other.
    #[must_use]
    pub fn at(text: &str, caret: usize) -> Option<Self> {
        emoji_suggestion_query(text, caret)
            .map(Self::Emoji)
            .or_else(|| mention_suggestion_query(text, caret).map(Self::Mention))
    }

    /// The bytes a chosen suggestion replaces.
    #[must_use]
    pub fn range(&self) -> std::ops::Range<usize> {
        match self {
            Self::Emoji(query) => query.start..query.end,
            Self::Mention(query) => query.start..query.end,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Suggestion {
    Emoji(EmojiSuggestion),
    Person(PublicUser),
    /// `@everyone`, offered only to a writer holding `MENTION_EVERYONE` in the channel.
    Everyone,
    Role(ServerRole),
}

impl Suggestion {
    /// What choosing it writes: the emoji itself, or "@" and the unique name (ADR-0048), which
    /// the longest-name rule reads back as the mention, spaces included; a role's "@Name" becomes
    /// its markup when the message is sent (`with_role_names_as_markup`).
    #[must_use]
    pub fn written_text(&self) -> String {
        match self {
            Self::Emoji(suggestion) => suggestion.emoji.to_owned(),
            Self::Person(person) => format!("@{}", person.username),
            Self::Everyone => format!("@{EVERYONE_MENTION_WORD}"),
            Self::Role(role) => format!("@{}", role.name),
        }
    }
}

/// Who "@na" may name in the open channel.
#[derive(Debug, Clone, Copy, Default)]
pub struct MentionSources<'a> {
    /// The channel's people.
    pub people: &'a [PublicUser],
    /// Left out of the people: mentioning oneself notifies no one.
    pub writer_id: Option<Snowflake>,
    /// The roles the writer may mention, highest first (`roles_writer_may_mention`); empty
    /// outside a server channel.
    pub mentionable_roles: &'a [ServerRole],
    pub can_mention_everyone: bool,
}

/// What the list shows and which row the arrows are on.
#[derive(Debug, Default)]
pub struct SuggestionState {
    query: Option<SuggestionQuery>,
    suggestions: Vec<Suggestion>,
    /// Esc hides the list until the word changes: the start of the word it was pressed on.
    dismissed_start: Option<usize>,
    active_index: usize,
}

impl SuggestionState {
    /// Reads the word before the caret again; `caret` is `None` while text is selected.
    pub fn refresh(&mut self, text: &str, caret: Option<usize>, sources: MentionSources<'_>) {
        let query = caret.and_then(|caret| SuggestionQuery::at(text, caret));
        if query != self.query {
            self.active_index = 0;
        }
        if query.as_ref().map(|query| query.range().start) != self.dismissed_start {
            self.dismissed_start = None;
        }
        self.suggestions = match &query {
            None => Vec::new(),
            Some(SuggestionQuery::Emoji(emoji)) => suggest_emoji(&emoji.query)
                .into_iter()
                .map(Suggestion::Emoji)
                .collect(),
            Some(SuggestionQuery::Mention(mention)) => {
                // Without a known writer no one is left out (an id no one has).
                let writer_id = sources.writer_id.unwrap_or(Snowflake::from_raw(0));
                let roles: Vec<&ServerRole> = sources.mentionable_roles.iter().collect();
                // People first, then `@everyone` and the roles, each list at most ten long.
                suggest_mentions(&mention.query, sources.people, writer_id)
                    .into_iter()
                    .cloned()
                    .map(Suggestion::Person)
                    .chain(
                        suggest_role_mentions(&mention.query, &roles, sources.can_mention_everyone)
                            .into_iter()
                            .map(|suggestion| match suggestion {
                                RoleMentionSuggestion::Everyone => Suggestion::Everyone,
                                RoleMentionSuggestion::Role(role) => Suggestion::Role(role.clone()),
                            }),
                    )
                    .collect()
            }
        };
        self.query = query;
    }

    pub fn close(&mut self) {
        self.query = None;
        self.suggestions.clear();
    }

    pub fn dismiss(&mut self) {
        self.dismissed_start = self.query.as_ref().map(|query| query.range().start);
    }

    #[must_use]
    pub fn query(&self) -> Option<&SuggestionQuery> {
        self.query.as_ref()
    }

    /// The rows shown: none once dismissed.
    #[must_use]
    pub fn visible(&self) -> &[Suggestion] {
        match &self.query {
            Some(query) if Some(query.range().start) != self.dismissed_start => &self.suggestions,
            _ => &[],
        }
    }

    /// Within the list as it is now: an update can shorten it under the arrows.
    #[must_use]
    pub fn active_index(&self) -> usize {
        self.active_index
            .min(self.visible().len().saturating_sub(1))
    }

    /// `"up"` or `"down"`, wrapping; false when no list shows.
    pub fn move_active(&mut self, key: &str) -> bool {
        let count = self.visible().len();
        match next_menu_item_index(key, Some(self.active_index()), count) {
            Some(index) => {
                self.active_index = index;
                true
            }
            None => false,
        }
    }

    pub fn hover(&mut self, index: usize) {
        self.active_index = index;
    }

    /// The suggestion at `index` and the bytes it replaces.
    #[must_use]
    pub fn choice(&self, index: usize) -> Option<(Suggestion, std::ops::Range<usize>)> {
        let suggestion = self.visible().get(index)?.clone();
        Some((suggestion, self.query.as_ref()?.range()))
    }
}

/// Runs with the index of a row.
pub type RowHandler = Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>;

/// The list itself, drawn above the composer's top edge.
#[derive(IntoElement)]
pub struct SuggestionList {
    pub title: SharedString,
    pub suggestions: Vec<Suggestion>,
    pub active_index: usize,
    pub on_choose: RowHandler,
    pub on_hover: RowHandler,
}

impl RenderOnce for SuggestionList {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let chat = i18n::strings(cx).chat();
        let rows = self
            .suggestions
            .into_iter()
            .enumerate()
            .map(|(index, suggestion)| {
                let is_active = index == self.active_index;
                let on_choose = self.on_choose.clone();
                let on_hover = self.on_hover.clone();
                // The second word ("Role", the hint) in the secondary text, as on the web.
                let mut hint: Option<SharedString> = None;
                let (leading, label, spoken): (gpui::AnyElement, SharedString, SharedString) =
                    match &suggestion {
                        Suggestion::Emoji(emoji) => (
                            div()
                                .flex_none()
                                .w(length(COMPONENT_COMPOSER_SUGGESTIONS_AVATAR_SIZE))
                                .flex()
                                .justify_center()
                                .font_weight(crate::emoji_picker::emoji_glyph_weight())
                                .child(emoji.emoji)
                                .into_any_element(),
                            emoji.name.into(),
                            format!("{} {}", emoji.emoji, emoji.name).into(),
                        ),
                        Suggestion::Person(person) => {
                            let name: SharedString = display_name(person).to_owned().into();
                            (
                                Avatar::new(
                                    AvatarPerson::from_user(person, None),
                                    COMPONENT_COMPOSER_SUGGESTIONS_AVATAR_SIZE,
                                )
                                .into_any_element(),
                                name.clone(),
                                name,
                            )
                        }
                        Suggestion::Everyone => {
                            let label: SharedString = format!("@{EVERYONE_MENTION_WORD}").into();
                            let explained = chat.everyone_mention_hint();
                            hint = Some(explained.into());
                            (
                                leading_slot(
                                    icon(
                                        IconName::Users,
                                        COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE,
                                        color(tokens.color_text_secondary),
                                    )
                                    .into_any_element(),
                                ),
                                label.clone(),
                                format!("{label}, {explained}").into(),
                            )
                        }
                        Suggestion::Role(role) => {
                            let label: SharedString = format!("@{}", role.name).into();
                            let kind = chat.role_mention_hint();
                            hint = Some(kind.into());
                            // The role's colour, or the neutral dot of a role without one.
                            let dot = if role.color == NO_ROLE_COLOUR {
                                color(tokens.component_roles_neutral_color)
                            } else {
                                color((role.color << 8) | 0xFF)
                            };
                            (
                                leading_slot(
                                    div()
                                        .size(length(COMPONENT_ROLES_ROW_DOT_SIZE))
                                        .rounded_full()
                                        .bg(dot)
                                        .into_any_element(),
                                ),
                                label.clone(),
                                format!("{label}, {kind}").into(),
                            )
                        }
                    };
                div()
                    .id(ElementId::NamedInteger(
                        "composer-suggestion".into(),
                        index as u64,
                    ))
                    .role(Role::ListBoxOption)
                    .aria_label(spoken)
                    .aria_selected(is_active)
                    .aria_position_in_set(index + 1)
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(length(SPACE_XS))
                    .h(length(COMPONENT_CONTEXT_MENU_ITEM_HEIGHT))
                    .px(length(COMPONENT_CONTEXT_MENU_ITEM_PADDING_INLINE))
                    .rounded(length(COMPONENT_CONTEXT_MENU_ITEM_RADIUS))
                    .cursor_pointer()
                    .when(is_active, |row| {
                        row.bg(color(tokens.component_context_menu_item_background_focus))
                    })
                    .text_color(color(if is_active {
                        tokens.component_context_menu_item_text_focus
                    } else {
                        tokens.component_context_menu_item_text
                    }))
                    .on_mouse_move(move |_, window, cx| on_hover(index, window, cx))
                    .on_click(move |_, window, cx| on_choose(index, window, cx))
                    .child(leading)
                    .child(
                        with_type(div(), TYPE_BODY)
                            .min_w_0()
                            .when(hint.is_none(), Styled::flex_1)
                            .truncate()
                            .child(label),
                    )
                    .when_some(hint, |row, hint| {
                        row.child(
                            with_type(div(), TYPE_BODY)
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .text_color(color(tokens.color_text_secondary))
                                .child(hint),
                        )
                    })
            })
            .collect::<Vec<_>>();
        let count = rows.len();

        div()
            .id("composer-suggestions")
            .occlude()
            .w_full()
            .max_w(length(COMPONENT_CUSTOM_EMOJI_SUGGESTION_WIDTH))
            .max_h(length(COMPONENT_CUSTOM_EMOJI_SUGGESTION_MAX_HEIGHT))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(length(COMPONENT_CONTEXT_MENU_RADIUS))
            .border(length(COMPONENT_CONTEXT_MENU_BORDER_WIDTH))
            .border_color(color(tokens.component_context_menu_border))
            .bg(color(tokens.component_context_menu_background))
            .shadow(shadow(tokens.component_context_menu_shadow))
            .child(
                with_type(div(), TYPE_CATEGORY)
                    .flex_none()
                    .truncate()
                    .px(length(COMPONENT_CONTEXT_MENU_ITEM_PADDING_INLINE))
                    .pt(length(COMPONENT_CONTEXT_MENU_PADDING))
                    .pb(length(SPACE_2XS))
                    .text_color(color(tokens.component_emoji_picker_heading_text))
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .id("composer-suggestions-listbox")
                    .role(Role::ListBox)
                    .aria_label(self.title)
                    .aria_size_of_set(count)
                    .min_h(px(0.0))
                    .flex_shrink(1.0)
                    .h(relative(1.0))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .p(length(COMPONENT_CONTEXT_MENU_PADDING))
                    .pt_0()
                    .children(rows),
            )
    }
}

/// A role's colour of 0 means it has none (the web's `NO_ROLE_COLOR`).
const NO_ROLE_COLOUR: u32 = 0;

/// The leading slot of a row without an avatar: the avatar's box, so every name starts on the
/// same line.
fn leading_slot(content: gpui::AnyElement) -> gpui::AnyElement {
    div()
        .flex_none()
        .w(length(COMPONENT_COMPOSER_SUGGESTIONS_AVATAR_SIZE))
        .flex()
        .justify_center()
        .child(content)
        .into_any_element()
}

impl std::fmt::Debug for SuggestionList {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SuggestionList")
            .field("title", &self.title)
            .field("active_index", &self.active_index)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(id: u64, username: &str) -> PublicUser {
        PublicUser {
            id: Snowflake::from_raw(id),
            username: username.to_owned(),
            display_name: String::new(),
            avatar: None,
            banner: None,
        }
    }

    fn people_only(people: &[PublicUser], writer_id: Option<Snowflake>) -> MentionSources<'_> {
        MentionSources {
            people,
            writer_id,
            ..MentionSources::default()
        }
    }

    fn role(id: u64, name: &str, position: u32) -> ServerRole {
        ServerRole {
            id: Snowflake::from_raw(id),
            name: name.to_owned(),
            color: 0,
            position,
            permissions: mellow_protocol::Permissions::empty(),
            is_displayed_separately: false,
            is_mentionable: true,
            icon: None,
            unicode_emoji: None,
        }
    }

    #[test]
    fn people_come_first_then_everyone_and_roles_the_writer_may_mention() {
        let people = [person(2, "modeste")];
        let roles = [role(20, "Modérateurs", 2), role(21, "Musique", 1)];
        let mut state = SuggestionState::default();
        let sources = MentionSources {
            people: &people,
            writer_id: None,
            mentionable_roles: &roles,
            can_mention_everyone: false,
        };

        state.refresh("@mod", Some(4), sources);
        assert_eq!(
            state.visible(),
            [
                Suggestion::Person(person(2, "modeste")),
                Suggestion::Role(role(20, "Modérateurs", 2))
            ]
        );
        assert_eq!(
            state
                .choice(1)
                .map(|(chosen, range)| (chosen.written_text(), range)),
            Some(("@Modérateurs".to_owned(), 0..4))
        );

        // `@everyone` only with the permission, before the roles.
        state.refresh("@", Some(1), sources);
        assert!(!state.visible().contains(&Suggestion::Everyone));
        state.refresh(
            "@ever",
            Some(5),
            MentionSources {
                can_mention_everyone: true,
                ..sources
            },
        );
        assert_eq!(state.visible(), [Suggestion::Everyone]);
        assert_eq!(Suggestion::Everyone.written_text(), "@everyone");
    }

    #[test]
    fn opens_emoji_after_a_colon_and_people_after_an_at_leaving_the_writer_out() {
        let people = [person(1, "paul"), person(2, "pauline")];
        let mut state = SuggestionState::default();

        state.refresh("hi :thumbs", Some(10), people_only(&people, None));
        assert!(matches!(state.query(), Some(SuggestionQuery::Emoji(_))));
        assert!(
            state
                .visible()
                .iter()
                .all(|suggestion| matches!(suggestion, Suggestion::Emoji(_)))
        );

        state.refresh(
            "hi @pau",
            Some(7),
            people_only(&people, Some(Snowflake::from_raw(1))),
        );
        assert_eq!(state.visible(), [Suggestion::Person(person(2, "pauline"))]);
        assert_eq!(
            state
                .choice(0)
                .map(|(chosen, range)| (chosen.written_text(), range)),
            Some(("@pauline".to_owned(), 3..7))
        );
    }

    #[test]
    fn closes_while_text_is_selected_or_no_word_is_typed() {
        let people = [person(2, "zoe")];
        let mut state = SuggestionState::default();
        state.refresh("@z", None, people_only(&people, None));
        assert_eq!(state.visible(), []);
        state.refresh("hello", Some(5), people_only(&people, None));
        assert!(state.query().is_none());
    }

    #[test]
    fn arrows_wrap_and_a_new_word_starts_at_the_top() {
        let people = [person(2, "ana"), person(3, "anaïs"), person(4, "anouk")];
        let mut state = SuggestionState::default();
        state.refresh("@an", Some(3), people_only(&people, None));
        assert_eq!(state.visible().len(), 3);

        assert!(state.move_active("up"));
        assert_eq!(state.active_index(), 2);
        assert!(state.move_active("down"));
        assert_eq!(state.active_index(), 0);
        state.hover(1);

        state.refresh("@ana", Some(4), people_only(&people, None));
        assert_eq!(state.active_index(), 0);
    }

    #[test]
    fn esc_hides_the_list_until_the_word_changes() {
        let people = [person(2, "ana"), person(3, "léo")];
        let mut state = SuggestionState::default();
        state.refresh("@a", Some(2), people_only(&people, None));
        state.dismiss();
        assert_eq!(state.visible(), []);
        assert!(!state.move_active("down"));

        // Typing on in the same word keeps it hidden; a new word brings it back.
        state.refresh("@an", Some(3), people_only(&people, None));
        assert_eq!(state.visible(), []);
        state.refresh("@an @l", Some(6), people_only(&people, None));
        assert_eq!(state.visible(), [Suggestion::Person(person(3, "léo"))]);
    }
}
