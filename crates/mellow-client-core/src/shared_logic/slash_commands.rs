//! The composer's "/" menu (the web's `slash-commands.ts`, docs/design/web-slash-menu.md): finding
//! the "/co" being typed, ranking the commands with English aliases, and writing each command.
//! Every command writes the Markdown that messages render, in the open: the field stays plain text.
//! Names and descriptions come from the client's catalogue; positions are byte offsets.

use super::search_text::fold_for_search;
use super::unicode_classes::{is_js_whitespace, is_letter_or_number, js_trim, js_trim_start};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlashCommandId {
    Heading1,
    Heading2,
    Heading3,
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Spoiler,
    InlineCode,
    CodeBlock,
    Quote,
    BulletedList,
    NumberedList,
    Link,
    Shrug,
    Tableflip,
    Poll,
    Checklist,
    ProgressBar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashCommandEffect {
    /// Starts the line: the spaces and the "/query" in front of it are replaced.
    LinePrefix(&'static str),
    /// A pair of marks with the caret between them.
    Wrap(&'static str),
    CodeBlock,
    /// `[text](https://)` with the text selected, ready to be typed over.
    Link,
    Text(&'static str),
    /// Opens a dialog (a poll, a checklist or a progress bar): the "/query" goes, nothing is written.
    Dialog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommand {
    pub id: SlashCommandId,
    pub effect: SlashCommandEffect,
    /// What the command writes, shown at the end of its row; empty for a dialog.
    pub markup: &'static str,
    /// Words that find it in any language: Markdown's own names ("h1"), short forms.
    pub keywords: &'static [&'static str],
}

/// `¯\_(ツ)_/¯` as Markdown: escaped so its backslash shows and its underscores never open an
/// emphasis.
pub const SHRUG_MARKDOWN: &str = "¯\\\\\\_(ツ)\\_/¯";
const TABLEFLIP_TEXT: &str = "(╯°□°)╯︵ ┻━┻";
const CODE_FENCE: &str = "```";

const fn command(
    id: SlashCommandId,
    effect: SlashCommandEffect,
    markup: &'static str,
    keywords: &'static [&'static str],
) -> SlashCommand {
    SlashCommand {
        id,
        effect,
        markup,
        keywords,
    }
}

/// In the order the menu lists them before anything is typed: headings, inline marks, blocks,
/// extras, then the poll, the checklist and the progress bar, which open a dialog.
pub const SLASH_COMMANDS: [SlashCommand; 19] = {
    use SlashCommandEffect::{CodeBlock, Dialog, LinePrefix, Link, Text, Wrap};
    use SlashCommandId as Id;
    [
        command(Id::Heading1, LinePrefix("# "), "#", &["h1"]),
        command(Id::Heading2, LinePrefix("## "), "##", &["h2"]),
        command(Id::Heading3, LinePrefix("### "), "###", &["h3"]),
        command(Id::Bold, Wrap("**"), "**", &["strong"]),
        command(Id::Italic, Wrap("*"), "*", &["em"]),
        command(Id::Underline, Wrap("__"), "__", &[]),
        command(Id::Strikethrough, Wrap("~~"), "~~", &["strike"]),
        command(Id::Spoiler, Wrap("||"), "||", &[]),
        command(Id::InlineCode, Wrap("`"), "`", &[]),
        command(Id::CodeBlock, CodeBlock, CODE_FENCE, &[]),
        command(Id::Quote, LinePrefix("> "), ">", &["blockquote"]),
        command(Id::BulletedList, LinePrefix("- "), "-", &["ul"]),
        command(Id::NumberedList, LinePrefix("1. "), "1.", &["ol"]),
        command(Id::Link, Link, "[ ]( )", &["url"]),
        command(Id::Shrug, Text(SHRUG_MARKDOWN), "¯\\_(ツ)_/¯", &[]),
        command(Id::Tableflip, Text(TABLEFLIP_TEXT), TABLEFLIP_TEXT, &[]),
        command(Id::Poll, Dialog, "", &["vote"]),
        command(Id::Checklist, Dialog, "", &["todo", "tasks"]),
        command(Id::ProgressBar, Dialog, "", &["progress"]),
    ]
};

// --- Finding the "/co" being typed -----------------------------------------------------------

/// The "/" and the word after it, up to the caret: what a chosen command replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommandQuery {
    pub start: usize,
    pub end: usize,
    pub query: String,
}

/// Longer than any command's name in any language: past it, the "/" is text.
const MAXIMUM_SLASH_QUERY_LENGTH: usize = 32;

/// The "/co" ending at `caret`, when its "/" starts its line (only spaces before it): a "/" after
/// text, inside a word or in "https://" is text. The replaced range ends at the caret, so a "/"
/// typed in front of existing words never eats them.
#[must_use]
pub fn slash_command_query(text: &str, caret: usize) -> Option<SlashCommandQuery> {
    let before_caret = text.get(..caret)?;
    let mut start = caret;
    let mut length = 0;
    for character in before_caret.chars().rev() {
        if length > MAXIMUM_SLASH_QUERY_LENGTH || !is_letter_or_number(character) {
            break;
        }
        start -= character.len_utf8();
        length += 1;
    }
    let slash = start.checked_sub(1)?;
    if text.as_bytes()[slash] != b'/' || length > MAXIMUM_SLASH_QUERY_LENGTH {
        return None;
    }
    let line_start = line_start_of(text, slash);
    if !js_trim(&text[line_start..slash]).is_empty() || is_inside_code_fence(text, line_start) {
        return None;
    }
    Some(SlashCommandQuery {
        start: slash,
        end: caret,
        query: text[start..caret].to_owned(),
    })
}

/// Whether the line starting at `line_start` is inside a fenced code block: an odd number of fence
/// lines before it.
fn is_inside_code_fence(text: &str, line_start: usize) -> bool {
    let fences = text[..line_start]
        .split('\n')
        .filter(|line| js_trim_start(line).starts_with(CODE_FENCE))
        .count();
    fences % 2 == 1
}

fn line_start_of(text: &str, position: usize) -> usize {
    text[..position]
        .rfind('\n')
        .map_or(0, |newline| newline + 1)
}

// --- Choosing among them ---------------------------------------------------------------------

/// A command's name and description in one language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommandWords<'words> {
    pub name: &'words str,
    pub description: &'words str,
}

/// A command with its words in the reader's language and the folded names it is found by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedSlashCommand {
    pub command: SlashCommand,
    pub name: String,
    pub description: String,
    /// The reader's name first, then English names and keywords; folded, also without spaces.
    folded_names: Vec<String>,
    folded_name_words: Vec<String>,
}

/// Built once per language: `words` are the reader's, `english_words` are aliases in any language.
#[must_use]
pub fn named_slash_commands<'words>(
    words: impl Fn(SlashCommandId) -> SlashCommandWords<'words>,
    english_words: impl Fn(SlashCommandId) -> SlashCommandWords<'words>,
) -> Vec<NamedSlashCommand> {
    SLASH_COMMANDS
        .iter()
        .map(|&command| {
            let readers = words(command.id);
            let names: Vec<String> = [readers.name, english_words(command.id).name]
                .into_iter()
                .chain(command.keywords.iter().copied())
                .map(fold_for_search)
                .collect();
            let mut folded_names: Vec<String> = Vec::new();
            for name in names
                .iter()
                .flat_map(|name| [name.clone(), name.replace(' ', "")])
            {
                if !folded_names.contains(&name) {
                    folded_names.push(name);
                }
            }
            NamedSlashCommand {
                command,
                name: readers.name.to_owned(),
                description: readers.description.to_owned(),
                folded_names,
                folded_name_words: names
                    .iter()
                    .flat_map(|name| name.split(' ').map(str::to_owned))
                    .collect(),
            }
        })
        .collect()
}

/// The commands `query` names, best first: the reader's name starting with it, then another name
/// ("/bold" in French), then a word inside a name ("/num" for "Liste numérotée"); ties keep the
/// menu's order. Case and accents are ignored; an empty query lists them all.
#[must_use]
pub fn suggest_slash_commands<'commands>(
    query: &str,
    commands: &'commands [NamedSlashCommand],
) -> Vec<&'commands NamedSlashCommand> {
    let folded_query = fold_for_search(query);
    let mut ranked: Vec<(u8, &NamedSlashCommand)> = commands
        .iter()
        .filter_map(|named| {
            slash_command_match_rank(named, &folded_query).map(|rank| (rank, named))
        })
        .collect();
    // Stable: equal ranks stay in the menu's order.
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().map(|(_, named)| named).collect()
}

fn slash_command_match_rank(named: &NamedSlashCommand, folded_query: &str) -> Option<u8> {
    // The first two folded names, as the web reads them after removing repeats.
    if named
        .folded_names
        .iter()
        .take(2)
        .any(|name| name.starts_with(folded_query))
    {
        return Some(0);
    }
    if named
        .folded_names
        .iter()
        .any(|name| name.starts_with(folded_query))
    {
        return Some(1);
    }
    named
        .folded_name_words
        .iter()
        .any(|word| word.starts_with(folded_query))
        .then_some(2)
}

// --- Writing them ----------------------------------------------------------------------------

/// The field's text and selection after a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub text: String,
    pub selection_start: usize,
    pub selection_end: usize,
}

/// `text` with the "/query" in `start..end` replaced by `command`'s Markdown.
/// `link_text_placeholder` is the selected word of a new link ("texte"), in the reader's language.
#[must_use]
pub fn apply_slash_command(
    text: &str,
    range: std::ops::Range<usize>,
    command: &SlashCommand,
    link_text_placeholder: &str,
) -> TextEdit {
    let line_start = line_start_of(text, range.start);
    let before = &text[..range.start];
    let after = &text[range.end..];
    match command.effect {
        SlashCommandEffect::LinePrefix(prefix) => caret_after(&text[..line_start], prefix, after),
        SlashCommandEffect::Wrap(marks) => caret_after(before, marks, &format!("{marks}{after}")),
        SlashCommandEffect::CodeBlock => {
            // Words after the "/" on its line go after the block: a fence must end its line.
            let closing_fence = if after.is_empty() || after.starts_with('\n') {
                format!("\n{CODE_FENCE}")
            } else {
                format!("\n{CODE_FENCE}\n")
            };
            caret_after(
                &text[..line_start],
                &format!("{CODE_FENCE}\n"),
                &format!("{closing_fence}{after}"),
            )
        }
        SlashCommandEffect::Link => {
            let selection_start = before.len() + 1;
            TextEdit {
                text: format!("{before}[{link_text_placeholder}](https://){after}"),
                selection_start,
                selection_end: selection_start + link_text_placeholder.len(),
            }
        }
        SlashCommandEffect::Text(inserted) => {
            let is_spaced = after.chars().next().is_none_or(is_js_whitespace);
            if is_spaced {
                caret_after(before, inserted, after)
            } else {
                caret_after(before, &format!("{inserted} "), after)
            }
        }
        SlashCommandEffect::Dialog => caret_after(before, "", after),
    }
}

fn caret_after(before: &str, inserted: &str, after: &str) -> TextEdit {
    let caret = before.len() + inserted.len();
    TextEdit {
        text: format!("{before}{inserted}{after}"),
        selection_start: caret,
        selection_end: caret,
    }
}

#[cfg(test)]
mod tests {
    use super::super::markdown::{BlockNode, InlineNode, parse_inline_markdown};
    use super::*;

    fn french(id: SlashCommandId) -> SlashCommandWords<'static> {
        use SlashCommandId as Id;
        let (name, description) = match id {
            Id::Heading1 => ("Titre 1", "Grand titre de section"),
            Id::Heading2 => ("Titre 2", "Titre de section moyen"),
            Id::Heading3 => ("Titre 3", "Petit titre de section"),
            Id::Bold => ("Gras", "Mise en avant forte"),
            Id::Italic => ("Italique", "Mise en avant légère"),
            Id::Underline => ("Souligné", "Texte souligné"),
            Id::Strikethrough => ("Barré", "Texte barré"),
            Id::Spoiler => ("Spoiler", "Caché jusqu’à ce qu’on le révèle"),
            Id::InlineCode => ("Code", "Du code dans une phrase"),
            Id::CodeBlock => ("Bloc de code", "Plusieurs lignes de code"),
            Id::Quote => ("Citation", "Ligne citée"),
            Id::BulletedList => ("Liste à puces", "Élément de liste avec une puce"),
            Id::NumberedList => ("Liste numérotée", "Élément de liste numéroté"),
            Id::Link => ("Lien", "Des mots qui ouvrent une adresse"),
            Id::Shrug => ("shrug", "Hausse les épaules"),
            Id::Tableflip => ("tableflip", "Renverse la table"),
            Id::Poll => ("Sondage", "Pose une question avec des réponses à voter"),
            Id::Checklist => ("Checklist", "Une liste que le salon coche ensemble"),
            Id::ProgressBar => (
                "Progression",
                "Une barre qui montre où en est quelque chose",
            ),
        };
        SlashCommandWords { name, description }
    }

    fn english(id: SlashCommandId) -> SlashCommandWords<'static> {
        use SlashCommandId as Id;
        let (name, description) = match id {
            Id::Heading1 => ("Heading 1", "Large section title"),
            Id::Heading2 => ("Heading 2", "Medium section title"),
            Id::Heading3 => ("Heading 3", "Small section title"),
            Id::Bold => ("Bold", "Strong emphasis"),
            Id::Italic => ("Italic", "Light emphasis"),
            Id::Underline => ("Underline", "Underlined text"),
            Id::Strikethrough => ("Strikethrough", "Crossed-out text"),
            Id::Spoiler => ("Spoiler", "Hidden until someone reveals it"),
            Id::InlineCode => ("Code", "Code within a sentence"),
            Id::CodeBlock => ("Code block", "Several lines of code"),
            Id::Quote => ("Quote", "Quoted line"),
            Id::BulletedList => ("Bulleted list", "List item with a bullet"),
            Id::NumberedList => ("Numbered list", "List item with a number"),
            Id::Link => ("Link", "Words that open an address"),
            Id::Shrug => ("shrug", "Adds a shrug"),
            Id::Tableflip => ("tableflip", "Flips the table"),
            Id::Poll => ("Poll", "Ask a question with answers to vote on"),
            Id::Checklist => ("Checklist", "A list the channel ticks together"),
            Id::ProgressBar => ("Progress bar", "A bar showing how far something is"),
        };
        SlashCommandWords { name, description }
    }

    fn ids_for(query: &str, commands: &[NamedSlashCommand]) -> Vec<SlashCommandId> {
        suggest_slash_commands(query, commands)
            .iter()
            .map(|named| named.command.id)
            .collect()
    }

    fn french_ids_for(query: &str) -> Vec<SlashCommandId> {
        ids_for(query, &named_slash_commands(french, english))
    }

    fn query(text: &str, caret: usize) -> Option<(usize, usize, String)> {
        slash_command_query(text, caret).map(|query| (query.start, query.end, query.query))
    }

    /// Applies `id` to `text_with_caret`, where "|" marks the caret after the typed "/query".
    fn apply(id: SlashCommandId, text_with_caret: &str) -> TextEdit {
        let caret = text_with_caret.find('|').expect("a caret in the test text");
        let text = text_with_caret.replacen('|', "", 1);
        let query = slash_command_query(&text, caret).expect("a \"/\" query in the test text");
        let command = SLASH_COMMANDS
            .iter()
            .find(|command| command.id == id)
            .expect("every id has a command");
        apply_slash_command(&text, query.start..query.end, command, "texte")
    }

    /// The edit's text with `[` and `]` around its selection, or `|` at a collapsed caret.
    fn shown(edit: &TextEdit) -> String {
        let TextEdit {
            text,
            selection_start: start,
            selection_end: end,
        } = edit;
        if start == end {
            format!("{}|{}", &text[..*start], &text[*start..])
        } else {
            format!(
                "{}[{}]{}",
                &text[..*start],
                &text[*start..*end],
                &text[*end..]
            )
        }
    }

    #[test]
    fn opens_on_a_slash_at_the_start_of_the_message_or_of_a_line() {
        assert_eq!(query("/", 1), Some((0, 1, String::new())));
        assert_eq!(query("/co", 3), Some((0, 3, "co".into())));
        assert_eq!(query("Salut\n/gras", 11), Some((6, 11, "gras".into())));
        assert_eq!(query("Salut\n   /tit", 13), Some((9, 13, "tit".into())));
        assert_eq!(query("/souligné", 10), Some((0, 10, "souligné".into())));
    }

    #[test]
    fn never_opens_on_a_slash_after_text_inside_a_word_or_in_an_address() {
        assert_eq!(query("et/ou", 3), None);
        assert_eq!(query("Salut /gras", 11), None);
        assert_eq!(query("https://", 8), None);
        assert_eq!(query("https://ex", 10), None);
        assert_eq!(query("/usr/bin", 8), None);
        assert_eq!(query("/usr/", 5), None);
        assert_eq!(query("# /gras", 7), None);
    }

    #[test]
    fn closes_once_a_space_ends_the_word_or_past_a_name_s_length() {
        assert_eq!(query("/gras ", 6), None);
        assert_eq!(query("bonjour", 7), None);
        assert_eq!(query("", 0), None);
        assert_eq!(query("/bonjour", 1), Some((0, 1, String::new())));
        let long = format!("/{}", "a".repeat(40));
        assert_eq!(query(&long, long.len()), None);
    }

    #[test]
    fn stays_closed_inside_a_fenced_code_block_and_opens_after_it() {
        let inside = "```\n/co";
        let after = "```\nx\n```\n/co";
        assert_eq!(query(inside, inside.len()), None);
        assert!(query(after, after.len()).is_some());
    }

    #[test]
    fn lists_every_command_in_the_menu_s_order_before_anything_is_typed() {
        let all: Vec<SlashCommandId> = SLASH_COMMANDS.iter().map(|command| command.id).collect();
        assert_eq!(french_ids_for(""), all);
    }

    #[test]
    fn finds_commands_by_the_reader_s_name_ignoring_case_and_accents() {
        use SlashCommandId as Id;
        assert_eq!(french_ids_for("co"), [Id::InlineCode, Id::CodeBlock]);
        assert_eq!(french_ids_for("GRAS"), [Id::Bold]);
        assert_eq!(french_ids_for("souligne"), [Id::Underline]);
        assert_eq!(french_ids_for("barre"), [Id::Strikethrough]);
        assert_eq!(
            french_ids_for("titre"),
            [Id::Heading1, Id::Heading2, Id::Heading3]
        );
        assert_eq!(french_ids_for("titre2"), [Id::Heading2]);
    }

    #[test]
    fn also_finds_them_by_english_names_and_markdown_s_words() {
        use SlashCommandId as Id;
        assert_eq!(french_ids_for("bold"), [Id::Bold]);
        assert_eq!(french_ids_for("h2"), [Id::Heading2]);
        assert_eq!(french_ids_for("quote"), [Id::Quote]);
        assert_eq!(
            french_ids_for("li"),
            [Id::BulletedList, Id::NumberedList, Id::Link]
        );
        assert_eq!(french_ids_for("lie"), [Id::Link]);
    }

    #[test]
    fn finds_a_word_inside_a_name_last_and_nothing_for_other_words() {
        use SlashCommandId as Id;
        assert_eq!(french_ids_for("num"), [Id::NumberedList]);
        assert_eq!(french_ids_for("puce"), [Id::BulletedList]);
        assert_eq!(french_ids_for("block"), [Id::Quote, Id::CodeBlock]);
        assert_eq!(french_ids_for("usr"), []);
        assert_eq!(french_ids_for("zz"), []);
    }

    #[test]
    fn speaks_the_reader_s_language() {
        let english_commands = named_slash_commands(english, english);
        assert_eq!(ids_for("bo", &english_commands), [SlashCommandId::Bold]);
        let french_commands = named_slash_commands(french, english);
        let first = suggest_slash_commands("gr", &french_commands)[0];
        assert_eq!(first.name, "Gras");
        assert_eq!(first.description, "Mise en avant forte");
    }

    #[test]
    fn starts_the_line_for_headings_quotes_and_lists() {
        use SlashCommandId as Id;
        assert_eq!(shown(&apply(Id::Heading1, "/tit|")), "# |");
        assert_eq!(
            shown(&apply(Id::Heading2, "Salut\n/titre2|")),
            "Salut\n## |"
        );
        assert_eq!(shown(&apply(Id::Heading3, "/|bonjour")), "### |bonjour");
        assert_eq!(shown(&apply(Id::Quote, "  /cit|")), "> |");
        assert_eq!(shown(&apply(Id::BulletedList, "a\n/li|\nb")), "a\n- |\nb");
        assert_eq!(shown(&apply(Id::NumberedList, "/num|")), "1. |");
    }

    #[test]
    fn writes_an_empty_pair_of_marks_with_the_caret_inside() {
        use SlashCommandId as Id;
        assert_eq!(shown(&apply(Id::Bold, "/gras|")), "**|**");
        assert_eq!(shown(&apply(Id::Italic, "/it|")), "*|*");
        assert_eq!(shown(&apply(Id::Underline, "/sou|")), "__|__");
        assert_eq!(shown(&apply(Id::Strikethrough, "/barr|")), "~~|~~");
        assert_eq!(shown(&apply(Id::Spoiler, "/spo|")), "|||||");
        assert_eq!(shown(&apply(Id::InlineCode, "Salut\n/code|")), "Salut\n`|`");
    }

    #[test]
    fn opens_a_code_block_with_the_caret_on_its_empty_line() {
        use SlashCommandId as Id;
        assert_eq!(shown(&apply(Id::CodeBlock, "/bloc|")), "```\n|\n```");
        assert_eq!(
            shown(&apply(Id::CodeBlock, "Regarde :\n/bloc|\nmerci")),
            "Regarde :\n```\n|\n```\nmerci"
        );
        assert_eq!(
            shown(&apply(Id::CodeBlock, "/|reste")),
            "```\n|\n```\nreste"
        );
    }

    #[test]
    fn writes_a_link_with_its_words_selected_and_the_extras_in_place_of_the_command() {
        use SlashCommandId as Id;
        assert_eq!(shown(&apply(Id::Link, "/lien|")), "[[texte]](https://)");
        assert_eq!(
            shown(&apply(Id::Shrug, "/shrug|")),
            format!("{SHRUG_MARKDOWN}|")
        );
        assert_eq!(
            shown(&apply(Id::Tableflip, "/table|bonjour")),
            "(╯°□°)╯︵ ┻━┻ |bonjour"
        );
    }

    #[test]
    fn writes_markdown_that_messages_show_as_meant() {
        let blocks = parse_inline_markdown(SHRUG_MARKDOWN);
        let [BlockNode::Paragraph(children)] = blocks.as_slice() else {
            panic!("one paragraph, got {blocks:?}");
        };
        let text: String = children
            .iter()
            .map(|child| match child {
                InlineNode::Text(text) => text.as_str(),
                other => panic!("plain text only, got {other:?}"),
            })
            .collect();
        assert_eq!(text, "¯\\_(ツ)_/¯");
    }

    #[test]
    fn shows_each_command_s_markdown_and_none_for_the_dialogs() {
        for command in SLASH_COMMANDS {
            assert_eq!(
                command.markup.is_empty(),
                command.effect == SlashCommandEffect::Dialog,
                "{:?}",
                command.id
            );
        }
    }

    #[test]
    fn finds_the_dialogs_by_their_names_in_either_language_and_only_removes_the_query() {
        use SlashCommandId as Id;
        assert_eq!(french_ids_for("sond").first(), Some(&Id::Poll));
        assert_eq!(french_ids_for("poll").first(), Some(&Id::Poll));
        assert_eq!(
            apply(Id::Poll, "Salut\n/sond|"),
            TextEdit {
                text: "Salut\n".into(),
                selection_start: 6,
                selection_end: 6,
            }
        );
        assert_eq!(french_ids_for("checklist").first(), Some(&Id::Checklist));
        assert_eq!(
            french_ids_for("progression").first(),
            Some(&Id::ProgressBar)
        );
        assert_eq!(french_ids_for("progress").first(), Some(&Id::ProgressBar));
        assert_eq!(french_ids_for("todo").first(), Some(&Id::Checklist));
    }
}
