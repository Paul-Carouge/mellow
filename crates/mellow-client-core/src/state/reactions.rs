//! Reactions as the client keeps them (the web's `reactions.ts`): the instance's emoji keys, and
//! the counts updated from events and from the user's own clicks.

use mellow_protocol::ReactionSummary;

const TEXT_PRESENTATION_SELECTOR: char = '\u{FE0E}';
const EMOJI_PRESENTATION_SELECTOR: char = '\u{FE0F}';

/// The key the instance stores a reaction under: the emoji without variation selectors.
#[must_use]
pub fn reaction_key(emoji: &str) -> String {
    emoji
        .chars()
        .filter(|character| {
            *character != TEXT_PRESENTATION_SELECTOR && *character != EMOJI_PRESENTATION_SELECTOR
        })
        .collect()
}

/// One more reaction with `key`, a new emoji going last. The user's own is counted once: their
/// click already counted it, so the event that follows changes nothing. True when it changed.
pub fn add_reaction(reactions: &mut Vec<ReactionSummary>, key: &str, is_mine: bool) -> bool {
    let Some(existing) = reactions.iter_mut().find(|reaction| reaction.emoji == key) else {
        reactions.push(ReactionSummary {
            emoji: key.to_owned(),
            count: 1,
            me: is_mine,
        });
        return true;
    };
    if is_mine && existing.me {
        return false;
    }
    existing.count = existing.count.saturating_add(1);
    existing.me |= is_mine;
    true
}

/// One reaction with `key` fewer; the emoji goes at zero. The user's own is removed once. True
/// when it changed.
pub fn remove_reaction(reactions: &mut Vec<ReactionSummary>, key: &str, is_mine: bool) -> bool {
    let Some(index) = reactions.iter().position(|reaction| reaction.emoji == key) else {
        return false;
    };
    let Some(existing) = reactions.get_mut(index) else {
        return false;
    };
    if is_mine && !existing.me {
        return false;
    }
    existing.count = existing.count.saturating_sub(1);
    existing.me &= !is_mine;
    if existing.count == 0 {
        reactions.remove(index);
    }
    true
}

/// A moderator removed every reaction with `key`. True when it changed.
pub fn remove_emoji(reactions: &mut Vec<ReactionSummary>, key: &str) -> bool {
    let before = reactions.len();
    reactions.retain(|reaction| reaction.emoji != key);
    reactions.len() != before
}

/// The `key` entry taken from `source` (gone if `source` has none); the other emoji stay as they
/// are. True when it changed.
pub fn take_emoji_from(
    reactions: &mut Vec<ReactionSummary>,
    key: &str,
    source: &[ReactionSummary],
) -> bool {
    let Some(replacement) = source.iter().find(|reaction| reaction.emoji == key) else {
        return remove_emoji(reactions, key);
    };
    match reactions.iter_mut().find(|reaction| reaction.emoji == key) {
        Some(existing) if existing == replacement => false,
        Some(existing) => {
            existing.clone_from(replacement);
            true
        }
        None => {
            reactions.push(replacement.clone());
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reaction(emoji: &str, count: u32, me: bool) -> ReactionSummary {
        ReactionSummary {
            emoji: emoji.into(),
            count,
            me,
        }
    }

    fn thumbs() -> ReactionSummary {
        reaction("👍", 2, false)
    }

    #[test]
    fn a_new_emoji_goes_last_and_others_reactions_count_each_time() {
        let mut reactions = vec![thumbs()];
        assert!(add_reaction(&mut reactions, "❤", false));
        assert_eq!(reactions, [thumbs(), reaction("❤", 1, false)]);
        let mut reactions = vec![thumbs()];
        add_reaction(&mut reactions, "👍", false);
        assert_eq!(reactions, [reaction("👍", 3, false)]);
    }

    #[test]
    fn the_users_own_reaction_counts_once_so_the_event_after_their_click_changes_nothing() {
        let mut reactions = vec![thumbs()];
        add_reaction(&mut reactions, "👍", true);
        assert_eq!(reactions, [reaction("👍", 3, true)]);
        assert!(!add_reaction(&mut reactions, "👍", true));
        assert_eq!(reactions, [reaction("👍", 3, true)]);
        assert!(remove_reaction(&mut reactions, "👍", true));
        assert_eq!(reactions, [thumbs()]);
        assert!(!remove_reaction(&mut reactions, "👍", true));
        assert_eq!(reactions, [thumbs()]);
    }

    #[test]
    fn an_emoji_goes_at_zero_and_when_a_moderator_removes_it() {
        let mut reactions = vec![reaction("👍", 1, false)];
        remove_reaction(&mut reactions, "👍", false);
        assert_eq!(reactions, []);
        let mut reactions = vec![thumbs(), reaction("❤", 1, true)];
        assert!(remove_emoji(&mut reactions, "👍"));
        assert_eq!(reactions, [reaction("❤", 1, true)]);
    }

    #[test]
    fn one_emoji_is_taken_from_another_copy_and_the_others_stay() {
        let heart = reaction("❤", 1, true);
        let stored = [reaction("👍", 5, true)];
        let mut reactions = vec![thumbs(), heart.clone()];
        take_emoji_from(&mut reactions, "👍", &stored);
        assert_eq!(reactions, [stored[0].clone(), heart.clone()]);
        let mut reactions = vec![heart.clone()];
        take_emoji_from(&mut reactions, "👍", &stored);
        assert_eq!(reactions, [heart.clone(), stored[0].clone()]);
        let mut reactions = vec![thumbs(), heart.clone()];
        take_emoji_from(&mut reactions, "👍", &[]);
        assert_eq!(reactions, [heart]);
    }

    #[test]
    fn keys_drop_variation_selectors_as_the_instance_stores_them() {
        assert_eq!(reaction_key("❤️"), "❤");
        assert_eq!(reaction_key("1️⃣"), "1⃣");
        assert_eq!(reaction_key("👍"), "👍");
    }
}
