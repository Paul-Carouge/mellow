//! The emoji picker's rules (DESIGN.md §6.52): which sections it shows (`EmojiPicker.svelte`),
//! how keys move through its grid (`emoji-grid.ts`), and the recently used emoji and quick
//! reactions (`reactions.ts`, `recent-emoji.svelte.ts`).

use mellow_protocol::CustomEmoji;

use super::custom_emoji::{display_emoji, is_catalogue_emoji};
use super::emoji_catalogue::{EMOJI_CATALOGUE, EmojiCategory, search_emoji};
use super::search_text::fold_for_search;
use crate::state::reaction_key;

// ---- What the picker shows ----

/// One emoji the picker offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerEmoji {
    /// As sent and shown, selectors included.
    Unicode(String),
    /// One of the open server's own emoji (ADR-0028).
    Custom(CustomEmoji),
}

impl PickerEmoji {
    /// What a reaction with it is sent as: the emoji itself, or `:id` (`customEmojiReactionKey`).
    #[must_use]
    pub fn reaction_emoji(&self) -> String {
        match self {
            Self::Unicode(emoji) => emoji.clone(),
            Self::Custom(emoji) => format!(":{}", emoji.id),
        }
    }

    /// What it writes into the composer (`composerTextOf`): the emoji, or `:name:`, which becomes
    /// the markup on send.
    #[must_use]
    pub fn composer_text(&self) -> String {
        match self {
            Self::Unicode(emoji) => emoji.clone(),
            Self::Custom(emoji) => format!(":{}:", emoji.name),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerSectionKind {
    /// What a search found, server emoji first.
    Results,
    Recent,
    /// The open server's own emoji, titled with its name.
    Server,
    Category(EmojiCategory),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerSection {
    pub kind: PickerSectionKind,
    /// Never empty: the keys and the category bar count on it.
    pub emoji: Vec<PickerEmoji>,
}

/// The picker's sections: with a query, one section of results (none when nothing matches);
/// otherwise the recently used, the open server's emoji (`None` in a conversation, where custom
/// emoji are not offered) and the built-in set by category.
#[must_use]
pub fn picker_sections(
    query: &str,
    recent: &[String],
    server_emojis: Option<&[CustomEmoji]>,
) -> Vec<PickerSection> {
    let server_emojis = server_emojis.unwrap_or_default();
    let query = query.trim();
    if !query.is_empty() {
        let folded = fold_for_search(query).replace(' ', "_");
        let results: Vec<PickerEmoji> = server_emojis
            .iter()
            .filter(|emoji| emoji.name.to_lowercase().contains(&folded))
            .cloned()
            .map(PickerEmoji::Custom)
            .chain(
                search_emoji(query)
                    .into_iter()
                    .map(|entry| PickerEmoji::Unicode(entry.emoji.to_owned())),
            )
            .collect();
        return if results.is_empty() {
            Vec::new()
        } else {
            vec![PickerSection {
                kind: PickerSectionKind::Results,
                emoji: results,
            }]
        };
    }
    let mut sections = Vec::new();
    if !recent.is_empty() {
        sections.push(PickerSection {
            kind: PickerSectionKind::Recent,
            emoji: recent
                .iter()
                .map(|emoji| PickerEmoji::Unicode(display_emoji(&reaction_key(emoji))))
                .collect(),
        });
    }
    if !server_emojis.is_empty() {
        sections.push(PickerSection {
            kind: PickerSectionKind::Server,
            emoji: server_emojis
                .iter()
                .cloned()
                .map(PickerEmoji::Custom)
                .collect(),
        });
    }
    for category in EmojiCategory::ALL {
        sections.push(PickerSection {
            kind: PickerSectionKind::Category(category),
            emoji: EMOJI_CATALOGUE
                .iter()
                .filter(|entry| entry.category == category)
                .map(|entry| PickerEmoji::Unicode(entry.emoji.to_owned()))
                .collect(),
        });
    }
    sections
}

// ---- Moving through the grid (emoji-grid.ts) ----

/// Where a key moves focus: a cell's index over all sections, or the search field above the
/// first row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmojiGridMove {
    Cell(usize),
    Search,
}

/// A key as the grid reads it (`EmojiGridKeyPress`): GPUI's key names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmojiGridKeyPress<'key> {
    pub key: &'key str,
    /// Ctrl (Cmd on macOS) turns Home and End into the first and last emoji of all.
    pub is_control_held: bool,
}

struct GridPosition {
    section: usize,
    start: usize,
    length: usize,
    row: usize,
    column: usize,
}

fn position_of(section_lengths: &[usize], columns: usize, index: usize) -> Option<GridPosition> {
    let mut start = 0;
    for (section, &length) in section_lengths.iter().enumerate() {
        if index < start + length {
            let offset = index - start;
            return Some(GridPosition {
                section,
                start,
                length,
                row: offset / columns,
                column: offset % columns,
            });
        }
        start += length;
    }
    None
}

/// The cell a key moves to from `current` (`nextEmojiCell`): each section is its own grid of
/// `columns` cells, the last row often short, and the keys move as in one grid that runs on from
/// section to section, keeping the column where it can. Arrows move by one cell or one row (Up
/// from the first row goes back to the search field), Home and End to the row's ends (with Ctrl,
/// the first and last emoji), Page Up and Page Down to the start of the previous and next
/// section. `None` for other keys. Sections are never empty.
#[must_use]
pub fn next_emoji_cell(
    section_lengths: &[usize],
    columns: usize,
    current: usize,
    press: EmojiGridKeyPress<'_>,
) -> Option<EmojiGridMove> {
    use EmojiGridMove::{Cell, Search};
    let total: usize = section_lengths.iter().sum();
    let width = columns.max(1);
    let GridPosition {
        section,
        start,
        length,
        row,
        column,
    } = position_of(section_lengths, width, current)?;
    let row_start = start + row * width;
    let last_section = section_lengths.len() - 1;
    let start_of = |section: usize| section_lengths[..section].iter().sum::<usize>();
    let cell = match press.key {
        "right" => (current + 1).min(total - 1),
        "left" => current.saturating_sub(1),
        "down" => {
            if row_start + width < start + length {
                (row_start + width + column).min(start + length - 1)
            } else if section == last_section {
                current
            } else {
                let next_length = section_lengths[section + 1];
                start + length + column.min(next_length - 1)
            }
        }
        "up" => {
            if row > 0 {
                row_start - width + column
            } else if section == 0 {
                return Some(Search);
            } else {
                let previous_length = section_lengths[section - 1];
                let previous_start = start - previous_length;
                let last_row_start = (previous_length - 1) / width * width;
                previous_start + (last_row_start + column).min(previous_length - 1)
            }
        }
        "home" if press.is_control_held => 0,
        "home" => row_start,
        "end" if press.is_control_held => total - 1,
        "end" => (row_start + width).min(start + length) - 1,
        "pagedown" if section == last_section => total - 1,
        "pagedown" => start + length,
        "pageup" if current > start || section == 0 => start,
        "pageup" => start_of(section - 1),
        _ => return None,
    };
    Some(Cell(cell))
}

// ---- Recently used emoji and quick reactions (reactions.ts) ----

/// The recently used emoji kept per account.
pub const MAXIMUM_RECENT_EMOJI: usize = 16;
/// The quick reactions until the user has picked some of their own, as on Discord.
pub const DEFAULT_QUICK_REACTIONS: [&str; 3] = ["👍", "❤️", "😂"];
const QUICK_REACTION_COUNT: usize = 3;

/// What was stored, kept only if it is an emoji the picker knows, and bounded
/// (`readRecentEmoji`'s validation: anything may have written the file).
#[must_use]
pub fn recent_emoji_from(stored: Vec<String>) -> Vec<String> {
    stored
        .into_iter()
        .filter(|emoji| is_catalogue_emoji(emoji))
        .take(MAXIMUM_RECENT_EMOJI)
        .collect()
}

/// `emoji` first, without duplicates, bounded (`rememberEmoji`).
#[must_use]
pub fn remember_emoji(recent: &[String], emoji: &str) -> Vec<String> {
    let key = reaction_key(emoji);
    std::iter::once(emoji.to_owned())
        .chain(
            recent
                .iter()
                .filter(|candidate| reaction_key(candidate) != key)
                .cloned(),
        )
        .take(MAXIMUM_RECENT_EMOJI)
        .collect()
}

/// The three quick reactions: the latest picked, then the defaults (`quickReactions`).
#[must_use]
pub fn quick_reactions(recent: &[String]) -> Vec<String> {
    let mut chosen: Vec<String> = Vec::with_capacity(QUICK_REACTION_COUNT);
    for emoji in recent
        .iter()
        .map(String::as_str)
        .chain(DEFAULT_QUICK_REACTIONS)
    {
        if chosen.len() == QUICK_REACTION_COUNT {
            break;
        }
        if !chosen
            .iter()
            .any(|candidate| reaction_key(candidate) == reaction_key(emoji))
        {
            chosen.push(emoji.to_owned());
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    use mellow_protocol::Snowflake;

    use super::*;

    // Ported from `emoji-grid.test.ts`. Two sections of 4 columns: recent (6 cells: a full row
    // and a row of 2), people (9 cells).
    //   recent  0  1  2  3      people   6  7  8  9
    //           4  5                    10 11 12 13
    //                                   14
    const SECTIONS: [usize; 2] = [6, 9];

    fn press(key: &str, is_control_held: bool) -> EmojiGridKeyPress<'_> {
        EmojiGridKeyPress {
            key,
            is_control_held,
        }
    }

    fn step(current: usize, key: &str) -> Option<EmojiGridMove> {
        next_emoji_cell(&SECTIONS, 4, current, press(key, false))
    }

    fn step_with_control(current: usize, key: &str) -> Option<EmojiGridMove> {
        next_emoji_cell(&SECTIONS, 4, current, press(key, true))
    }

    #[allow(clippy::unnecessary_wraps)] // Compared with `next_emoji_cell`'s answers.
    const fn cell(index: usize) -> Option<EmojiGridMove> {
        Some(EmojiGridMove::Cell(index))
    }

    #[test]
    fn moves_one_cell_left_and_right_across_sections_stopping_at_both_ends() {
        assert_eq!(step(5, "right"), cell(6));
        assert_eq!(step(6, "left"), cell(5));
        assert_eq!(step(0, "left"), cell(0));
        assert_eq!(step(14, "right"), cell(14));
    }

    #[test]
    fn moves_down_a_row_onto_a_short_rows_last_cell_then_into_the_next_sections_column() {
        assert_eq!(step(1, "down"), cell(5));
        assert_eq!(step(3, "down"), cell(5));
        assert_eq!(step(5, "down"), cell(7));
        assert_eq!(step(13, "down"), cell(14));
        assert_eq!(step(14, "down"), cell(14));
    }

    #[test]
    fn moves_up_a_row_into_the_previous_sections_last_row_and_from_the_first_row_to_the_search() {
        assert_eq!(step(11, "up"), cell(7));
        assert_eq!(step(7, "up"), cell(5));
        assert_eq!(step(6, "up"), cell(4));
        assert_eq!(step(2, "up"), Some(EmojiGridMove::Search));
    }

    #[test]
    fn goes_to_the_rows_ends_with_home_and_end_and_to_the_first_and_last_emoji_with_ctrl() {
        assert_eq!(step(12, "home"), cell(10));
        assert_eq!(step(10, "end"), cell(13));
        assert_eq!(step(4, "end"), cell(5));
        assert_eq!(step_with_control(12, "home"), cell(0));
        assert_eq!(step_with_control(1, "end"), cell(14));
    }

    #[test]
    fn jumps_between_sections_with_page_up_and_page_down() {
        assert_eq!(step(2, "pagedown"), cell(6));
        assert_eq!(step(8, "pagedown"), cell(14));
        assert_eq!(step(8, "pageup"), cell(6));
        assert_eq!(step(6, "pageup"), cell(0));
        assert_eq!(step(3, "pageup"), cell(0));
    }

    #[test]
    fn ignores_other_keys_and_cells_outside_the_sections() {
        assert_eq!(step(3, "a"), None);
        assert_eq!(step(15, "right"), None);
    }

    #[test]
    fn works_with_a_single_column() {
        assert_eq!(
            next_emoji_cell(&[2, 2], 1, 1, press("down", false)),
            cell(2)
        );
        assert_eq!(next_emoji_cell(&[2, 2], 1, 2, press("up", false)), cell(1));
    }

    // Ported from `reactions.test.ts` ("recently used emoji").

    #[test]
    fn the_latest_comes_first_without_duplicates_in_a_bounded_list() {
        let mut recent = remember_emoji(&[], "👍");
        recent = remember_emoji(&recent, "❤️");
        assert_eq!(remember_emoji(&recent, "❤"), ["❤", "👍"]);
        for entry in EMOJI_CATALOGUE.iter().take(30) {
            recent = remember_emoji(&recent, entry.emoji);
        }
        assert_eq!(recent.len(), MAXIMUM_RECENT_EMOJI);
    }

    #[test]
    fn stored_emoji_the_picker_does_not_know_are_dropped() {
        let stored = ["👍", "<b>", "lol", "❤"].map(str::to_owned).to_vec();
        assert_eq!(recent_emoji_from(stored), ["👍", "❤"]);
        let too_many = EMOJI_CATALOGUE
            .iter()
            .take(30)
            .map(|entry| entry.emoji.to_owned())
            .collect();
        assert_eq!(recent_emoji_from(too_many).len(), MAXIMUM_RECENT_EMOJI);
    }

    #[test]
    fn quick_reactions_are_the_latest_three_then_the_defaults() {
        let owned = |list: &[&str]| {
            list.iter()
                .map(|&emoji| emoji.to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(quick_reactions(&[]), ["👍", "❤️", "😂"]);
        assert_eq!(quick_reactions(&owned(&["🔥"])), ["🔥", "👍", "❤️"]);
        assert_eq!(
            quick_reactions(&owned(&["❤", "🎉", "🔥", "👀"])),
            ["❤", "🎉", "🔥"]
        );
    }

    // The sections `EmojiPicker.svelte` derives.

    fn party() -> CustomEmoji {
        CustomEmoji {
            id: Snowflake::from_raw(501),
            name: "party_parrot".into(),
            is_animated: false,
        }
    }

    #[test]
    fn browsing_shows_recent_then_the_servers_emoji_then_every_category() {
        let server = [party()];
        let sections = picker_sections("  ", &["❤".to_owned()], Some(&server));
        let kinds: Vec<PickerSectionKind> = sections.iter().map(|section| section.kind).collect();
        assert_eq!(kinds[0], PickerSectionKind::Recent);
        assert_eq!(kinds[1], PickerSectionKind::Server);
        assert_eq!(kinds.len(), 2 + EmojiCategory::ALL.len());
        // The stored key shows with its selector back.
        assert_eq!(sections[0].emoji, [PickerEmoji::Unicode("❤️".to_owned())]);
        assert!(sections.iter().all(|section| !section.emoji.is_empty()));
    }

    #[test]
    fn a_conversation_or_a_first_run_leaves_out_the_empty_sections() {
        let sections = picker_sections("", &[], None);
        assert!(matches!(
            sections[0].kind,
            PickerSectionKind::Category(EmojiCategory::People)
        ));
        let empty_server = picker_sections("", &[], Some(&[]));
        assert_eq!(empty_server.len(), sections.len());
    }

    #[test]
    fn a_search_finds_server_emoji_by_name_first_then_built_in_ones() {
        let server = [party()];
        let sections = picker_sections("Party Par", &[], Some(&server));
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].kind, PickerSectionKind::Results);
        assert_eq!(sections[0].emoji[0], PickerEmoji::Custom(party()));
        let hearts = picker_sections("coeur", &[], Some(&server));
        assert!(
            hearts[0]
                .emoji
                .contains(&PickerEmoji::Unicode("❤️".to_owned()))
        );
        assert_eq!(picker_sections("zzzz-nothing", &[], Some(&server)), []);
    }

    #[test]
    fn a_picked_emoji_reacts_and_writes_as_the_web_does() {
        let heart = PickerEmoji::Unicode("❤️".to_owned());
        assert_eq!(heart.reaction_emoji(), "❤️");
        assert_eq!(heart.composer_text(), "❤️");
        let parrot = PickerEmoji::Custom(party());
        assert_eq!(parrot.reaction_emoji(), ":501");
        assert_eq!(parrot.composer_text(), ":party_parrot:");
    }
}
