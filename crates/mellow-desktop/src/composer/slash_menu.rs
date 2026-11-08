//! The composer's "/" menu (`ComposerSuggestions.svelte`'s command rows, DESIGN.md §5.5,
//! docs/design/web-slash-menu.md): a "/" that starts a line lists the commands, narrowed by what
//! follows it in the reader's language or English; Up and Down wrap, Enter or Tab applies, Esc
//! hides the list until the word changes, and a click applies without taking focus from the
//! field. Each Markdown command writes what it shows in its chip; the poll, the checklist and the
//! progress bar remove the "/query" and open their dialog. The rules live in client-core's
//! `slash_commands`.

use gpui::{
    App, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px, relative,
};
use mellow_client_core::shared_logic::slash_commands::{
    NamedSlashCommand, SlashCommandId, SlashCommandQuery, SlashCommandWords, TextEdit,
    apply_slash_command, named_slash_commands, slash_command_query, suggest_slash_commands,
};

use super::suggestions::RowHandler;
use crate::components::{icon, next_menu_item_index, shadow, with_type};
use crate::i18n::{Language, Strings};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_COMPOSER_SUGGESTIONS_AVATAR_SIZE, COMPONENT_COMPOSER_SUGGESTIONS_COMMAND_LIST_WIDTH,
    COMPONENT_CONTEXT_MENU_BORDER_WIDTH, COMPONENT_CONTEXT_MENU_ITEM_HEIGHT,
    COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE, COMPONENT_CONTEXT_MENU_ITEM_PADDING_INLINE,
    COMPONENT_CONTEXT_MENU_ITEM_RADIUS, COMPONENT_CONTEXT_MENU_PADDING,
    COMPONENT_CONTEXT_MENU_RADIUS, COMPONENT_CUSTOM_EMOJI_SUGGESTION_MAX_HEIGHT, RADIUS_XS,
    SPACE_2XS, SPACE_3XS, SPACE_XS, TYPE_BODY, TYPE_CATEGORY, TYPE_CODE,
};
use crate::theme::{color, length, theme};

/// A command's name and description in the catalogue's language.
fn words_of(id: SlashCommandId, catalogue: Strings) -> SlashCommandWords<'static> {
    use SlashCommandId as Id;
    let commands = catalogue.chat().slash_commands();
    let (name, description) = match id {
        Id::Heading1 => (
            commands.heading1().name(),
            commands.heading1().description(),
        ),
        Id::Heading2 => (
            commands.heading2().name(),
            commands.heading2().description(),
        ),
        Id::Heading3 => (
            commands.heading3().name(),
            commands.heading3().description(),
        ),
        Id::Bold => (commands.bold().name(), commands.bold().description()),
        Id::Italic => (commands.italic().name(), commands.italic().description()),
        Id::Underline => (
            commands.underline().name(),
            commands.underline().description(),
        ),
        Id::Strikethrough => (
            commands.strikethrough().name(),
            commands.strikethrough().description(),
        ),
        Id::Spoiler => (commands.spoiler().name(), commands.spoiler().description()),
        Id::InlineCode => (
            commands.inline_code().name(),
            commands.inline_code().description(),
        ),
        Id::CodeBlock => (
            commands.code_block().name(),
            commands.code_block().description(),
        ),
        Id::Quote => (commands.quote().name(), commands.quote().description()),
        Id::BulletedList => (
            commands.bulleted_list().name(),
            commands.bulleted_list().description(),
        ),
        Id::NumberedList => (
            commands.numbered_list().name(),
            commands.numbered_list().description(),
        ),
        Id::Link => (commands.link().name(), commands.link().description()),
        Id::Shrug => (commands.shrug().name(), commands.shrug().description()),
        Id::Tableflip => (
            commands.tableflip().name(),
            commands.tableflip().description(),
        ),
        Id::Poll => (commands.poll().name(), commands.poll().description()),
        Id::Checklist => (
            commands.checklist().name(),
            commands.checklist().description(),
        ),
        Id::ProgressBar => (
            commands.progress_bar().name(),
            commands.progress_bar().description(),
        ),
    };
    SlashCommandWords { name, description }
}

/// The glyph of a command's row (Phosphor, as the web's).
fn icon_of(id: SlashCommandId) -> IconName {
    use SlashCommandId as Id;
    match id {
        Id::Heading1 => IconName::TextHOne,
        Id::Heading2 => IconName::TextHTwo,
        Id::Heading3 => IconName::TextHThree,
        Id::Bold => IconName::TextB,
        Id::Italic => IconName::TextItalic,
        Id::Underline => IconName::TextUnderline,
        Id::Strikethrough => IconName::TextStrikethrough,
        Id::Spoiler => IconName::EyeSlash,
        Id::InlineCode => IconName::Code,
        Id::CodeBlock => IconName::CodeBlock,
        Id::Quote => IconName::Quotes,
        Id::BulletedList => IconName::ListBullets,
        Id::NumberedList => IconName::ListNumbers,
        Id::Link => IconName::LinkSimple,
        Id::Shrug => IconName::SmileyMeh,
        Id::Tableflip => IconName::SmileyAngry,
        Id::Poll => IconName::ChartBarHorizontal,
        Id::Checklist => IconName::ListChecks,
        Id::ProgressBar => IconName::Gauge,
    }
}

/// The commands that open a dialog instead of writing Markdown.
#[must_use]
pub fn is_dialog_command(id: SlashCommandId) -> bool {
    matches!(
        id,
        SlashCommandId::Poll | SlashCommandId::Checklist | SlashCommandId::ProgressBar
    )
}

/// What the menu shows and which row the arrows are on.
#[derive(Debug, Default)]
pub struct SlashMenuState {
    /// The commands named in the reader's language, built once per language.
    named: Option<(Language, Vec<NamedSlashCommand>)>,
    query: Option<SlashCommandQuery>,
    suggestions: Vec<NamedSlashCommand>,
    /// Esc hides the list until the word changes: where the "/" it was pressed on starts.
    dismissed_start: Option<usize>,
    active_index: usize,
    /// "Send polls" and "Send checklists" in the open channel offer their commands.
    can_create_polls: bool,
    can_create_checklists: bool,
}

impl SlashMenuState {
    pub fn set_card_permissions(&mut self, can_create_polls: bool, can_create_checklists: bool) {
        self.can_create_polls = can_create_polls;
        self.can_create_checklists = can_create_checklists;
    }

    /// Reads the "/query" before the caret again; `caret` is `None` while text is selected or
    /// another list (":" or "@") has the word.
    pub fn refresh(&mut self, text: &str, caret: Option<usize>, catalogue: Strings) {
        let query = caret.and_then(|caret| slash_command_query(text, caret));
        if query.as_ref().map(|query| (query.start, &query.query))
            != self.query.as_ref().map(|query| (query.start, &query.query))
        {
            self.active_index = 0;
        }
        if query.as_ref().map(|query| query.start) != self.dismissed_start {
            self.dismissed_start = None;
        }
        self.suggestions = match &query {
            None => Vec::new(),
            Some(query) => {
                let language = catalogue.language();
                if self
                    .named
                    .as_ref()
                    .is_none_or(|(named_in, _)| *named_in != language)
                {
                    let english = Strings::new(Language::English);
                    let named = named_slash_commands(
                        |id| words_of(id, catalogue),
                        |id| words_of(id, english),
                    );
                    self.named = Some((language, named));
                }
                let offered: Vec<NamedSlashCommand> = self
                    .named
                    .as_ref()
                    .map(|(_, named)| named.as_slice())
                    .unwrap_or_default()
                    .iter()
                    .filter(|named| match named.command.id {
                        SlashCommandId::Poll => self.can_create_polls,
                        SlashCommandId::Checklist | SlashCommandId::ProgressBar => {
                            self.can_create_checklists
                        }
                        _ => true,
                    })
                    .cloned()
                    .collect();
                suggest_slash_commands(&query.query, &offered)
                    .into_iter()
                    .cloned()
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
        self.dismissed_start = self.query.as_ref().map(|query| query.start);
    }

    #[must_use]
    pub fn query(&self) -> Option<&SlashCommandQuery> {
        self.query.as_ref()
    }

    /// The rows shown: none once dismissed, or when no command matches ("/usr" is text).
    #[must_use]
    pub fn visible(&self) -> &[NamedSlashCommand] {
        match &self.query {
            Some(query) if Some(query.start) != self.dismissed_start => &self.suggestions,
            _ => &[],
        }
    }

    #[must_use]
    pub fn active_index(&self) -> usize {
        self.active_index
            .min(self.visible().len().saturating_sub(1))
    }

    /// `"up"` or `"down"`, wrapping; false when no list shows.
    pub fn move_active(&mut self, key: &str) -> bool {
        match next_menu_item_index(key, Some(self.active_index()), self.visible().len()) {
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

    /// The command at `index` applied to `text`: the field's text and selection after it, and the
    /// command, whose dialog opens when it has one.
    #[must_use]
    pub fn apply(
        &self,
        index: usize,
        text: &str,
        catalogue: Strings,
    ) -> Option<(TextEdit, SlashCommandId)> {
        let named = self.visible().get(index)?;
        let query = self.query.as_ref()?;
        let edit = apply_slash_command(
            text,
            query.start..query.end,
            &named.command,
            catalogue.chat().link_text_placeholder(),
        );
        Some((edit, named.command.id))
    }
}

/// The command list, drawn above the composer's top edge, wider than the ":" and "@" lists so
/// descriptions keep their words.
#[derive(IntoElement)]
pub struct SlashMenuList {
    pub title: SharedString,
    /// "16 commands. Up and Down to choose, Enter or Tab to apply."
    pub count_sentence: SharedString,
    pub commands: Vec<NamedSlashCommand>,
    pub active_index: usize,
    pub on_choose: RowHandler,
    pub on_hover: RowHandler,
}

impl RenderOnce for SlashMenuList {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let count = self.commands.len();
        let rows = self
            .commands
            .into_iter()
            .enumerate()
            .map(|(index, named)| {
                let is_active = index == self.active_index;
                let on_choose = self.on_choose.clone();
                let on_hover = self.on_hover.clone();
                let markup = named.command.markup;
                let spoken = if markup.is_empty() {
                    format!("{}, {}", named.name, named.description)
                } else {
                    format!("{}, {}, {markup}", named.name, named.description)
                };
                let text_colour = color(if is_active {
                    tokens.component_context_menu_item_text_focus
                } else {
                    tokens.component_context_menu_item_text
                });
                div()
                    .id(ElementId::NamedInteger(
                        "composer-command".into(),
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
                    .text_color(text_colour)
                    .on_mouse_move(move |_, window, cx| on_hover(index, window, cx))
                    .on_click(move |_, window, cx| on_choose(index, window, cx))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(length(COMPONENT_COMPOSER_SUGGESTIONS_AVATAR_SIZE))
                            .child(icon(
                                icon_of(named.command.id),
                                COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE,
                                text_colour,
                            )),
                    )
                    .child(
                        with_type(div(), TYPE_BODY)
                            .flex_none()
                            .max_w(relative(0.5))
                            .truncate()
                            .child(named.name),
                    )
                    .child(
                        with_type(div(), TYPE_BODY)
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_color(color(tokens.color_text_secondary))
                            .child(named.description),
                    )
                    .when(!markup.is_empty(), |row| {
                        row.child(
                            with_type(div(), TYPE_CODE)
                                .flex_none()
                                .px(length(SPACE_3XS))
                                .rounded(length(RADIUS_XS))
                                .bg(color(tokens.component_message_code_background))
                                .text_color(color(tokens.color_text_normal))
                                .child(markup),
                        )
                    })
            })
            .collect::<Vec<_>>();

        div()
            .id("composer-commands")
            .occlude()
            .w_full()
            .max_w(length(COMPONENT_COMPOSER_SUGGESTIONS_COMMAND_LIST_WIDTH))
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
                    .id("composer-commands-listbox")
                    .role(Role::ListBox)
                    .aria_label(self.title)
                    .aria_description(self.count_sentence)
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

impl std::fmt::Debug for SlashMenuList {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlashMenuList")
            .field("title", &self.title)
            .field("active_index", &self.active_index)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn french() -> Strings {
        Strings::new(Language::French)
    }

    fn ids(state: &SlashMenuState) -> Vec<SlashCommandId> {
        state
            .visible()
            .iter()
            .map(|named| named.command.id)
            .collect()
    }

    #[test]
    fn lists_the_commands_in_the_reader_s_language_and_finds_english_names() {
        let mut state = SlashMenuState::default();
        state.set_card_permissions(true, true);
        state.refresh("/gr", Some(3), french());
        assert_eq!(ids(&state), [SlashCommandId::Bold]);
        assert_eq!(state.visible()[0].name, "Gras");
        state.refresh("/bold", Some(5), french());
        assert_eq!(ids(&state), [SlashCommandId::Bold]);
        state.refresh("Salut /gras", Some(11), french());
        assert_eq!(state.visible(), []);
    }

    #[test]
    fn offers_the_dialogs_only_with_their_permission() {
        let mut state = SlashMenuState::default();
        state.refresh("/", Some(1), french());
        assert!(!ids(&state).iter().any(|id| is_dialog_command(*id)));
        state.set_card_permissions(true, false);
        state.refresh("/", Some(1), french());
        assert_eq!(ids(&state).last(), Some(&SlashCommandId::Poll));
        state.set_card_permissions(false, true);
        state.refresh("/", Some(1), french());
        let all = ids(&state);
        assert_eq!(
            all[all.len() - 2..],
            [SlashCommandId::Checklist, SlashCommandId::ProgressBar]
        );
    }

    #[test]
    fn arrows_wrap_esc_hides_until_the_word_changes_and_a_choice_writes_its_markdown() {
        let mut state = SlashMenuState::default();
        state.refresh("/co", Some(3), french());
        assert_eq!(
            ids(&state),
            [SlashCommandId::InlineCode, SlashCommandId::CodeBlock]
        );
        assert!(state.move_active("up"));
        assert_eq!(state.active_index(), 1);
        let (edit, id) = state.apply(1, "/co", french()).expect("a command at row 1");
        assert_eq!(id, SlashCommandId::CodeBlock);
        assert_eq!(
            (edit.text.as_str(), edit.selection_start),
            ("```\n\n```", 4)
        );
        state.dismiss();
        assert_eq!(state.visible(), []);
        state.refresh("/cod", Some(4), french());
        assert_eq!(state.visible(), []);
        state.refresh("a\n/cod", Some(6), french());
        assert_eq!(state.visible().len(), 2);
    }
}
