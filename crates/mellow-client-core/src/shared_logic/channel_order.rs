//! How a guild's channels are listed and moved (`web/src/lib/channels.ts` and
//! `web/src/components/channel-order.ts`, DESIGN.md §5.3): pure rules over the sidebar's
//! sections, so the drag, the menus and the pending order share one model. The instance decides
//! (ADR-0007); these rules only build the whole new order it wants
//! (`PUT /guilds/{id}/channels/order`) and hide moves it would refuse.

use std::collections::HashMap;

use mellow_protocol::{Channel, ChannelKind, ChannelPlacement, Snowflake};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSection {
    /// `None` for the channels outside any category, listed first.
    pub category: Option<Channel>,
    pub channels: Vec<Channel>,
}

impl ChannelSection {
    fn category_id(&self) -> Option<Snowflake> {
        self.category.as_ref().map(|category| category.id)
    }
}

/// Uncategorised channels, then each category with its channels, text and voice mixed by
/// position (the managers choose the order). A channel in a category hidden from the reader is
/// listed with the uncategorised ones: it is visible, and its heading is not. It keeps its
/// `parent_id`, so a reorder sends that category back unchanged.
#[must_use]
pub fn channel_sections(channels: &[Channel]) -> Vec<ChannelSection> {
    let by_position = |left: &&Channel, right: &&Channel| {
        left.position
            .cmp(&right.position)
            .then(left.id.cmp(&right.id))
    };
    let mut categories: Vec<&Channel> = channels
        .iter()
        .filter(|channel| channel.kind == ChannelKind::Category)
        .collect();
    categories.sort_by(by_position);
    let mut listed: Vec<&Channel> = channels
        .iter()
        .filter(|channel| channel.kind != ChannelKind::Category)
        .collect();
    listed.sort_by(by_position);
    let is_category = |id: Snowflake| categories.iter().any(|category| category.id == id);
    let uncategorised: Vec<Channel> = listed
        .iter()
        .filter(|channel| channel.parent_id.is_none_or(|parent| !is_category(parent)))
        .map(|channel| (*channel).clone())
        .collect();
    let mut sections = Vec::with_capacity(categories.len() + 1);
    if !uncategorised.is_empty() {
        sections.push(ChannelSection {
            category: None,
            channels: uncategorised,
        });
    }
    sections.extend(categories.iter().map(|category| {
        ChannelSection {
            category: Some((*category).clone()),
            channels: listed
                .iter()
                .filter(|channel| channel.parent_id == Some(category.id))
                .map(|channel| (*channel).clone())
                .collect(),
        }
    }));
    sections
}

/// Where a channel goes: a category (`None`: none) and its index there, as the list shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelDestination {
    pub category_id: Option<Snowflake>,
    pub index: usize,
    /// "No category" chosen on purpose ("Move to category…"): a channel kept in a category hidden
    /// from the reader leaves it too. A drag among the uncategorised rows keeps it there.
    pub leaves_hidden_category: bool,
}

/// Every listed channel once, in the sidebar's order: each category, then its channels. An
/// uncategorised channel sends its own `parent_id`, which is a hidden category's for a channel
/// the reader sees in a category they do not (the instance keeps it there).
#[must_use]
pub fn placements_of(sections: &[ChannelSection]) -> Vec<ChannelPlacement> {
    sections
        .iter()
        .flat_map(|section| {
            let category = section.category.as_ref().map(|category| ChannelPlacement {
                id: category.id,
                parent_id: None,
            });
            let parent = section.category_id();
            category
                .into_iter()
                .chain(
                    section
                        .channels
                        .iter()
                        .map(move |channel| ChannelPlacement {
                            id: channel.id,
                            parent_id: parent.or(channel.parent_id),
                        }),
                )
        })
        .collect()
}

/// Whether a pending order can stop showing: the instance's order now matches it (its snapshot
/// came, before or after the answer), or the instance answered and the guild's channels were
/// replaced since (a snapshot came that differs: the instance's order wins). Revisions count the
/// guild's channel snapshots, where the web compares array identity.
#[must_use]
pub fn is_pending_order_settled(
    pending_placements: &[ChannelPlacement],
    channels_revision_when_answered: Option<u64>,
    instance_placements: &[ChannelPlacement],
    channels_revision_now: u64,
) -> bool {
    instance_placements == pending_placements
        || channels_revision_when_answered.is_some_and(|revision| revision != channels_revision_now)
}

/// The sections a pending order shows, with the channels as they are now (a rename meanwhile
/// shows). `None` once the channels and the order no longer list the same ids (one was created
/// or deleted meanwhile): the instance's order shows instead.
#[must_use]
pub fn sections_from_placements(
    channels: &[Channel],
    placements: &[ChannelPlacement],
) -> Option<Vec<ChannelSection>> {
    let by_id: HashMap<Snowflake, &Channel> = channels
        .iter()
        .map(|channel| (channel.id, channel))
        .collect();
    if by_id.len() != placements.len()
        || placements
            .iter()
            .any(|placement| !by_id.contains_key(&placement.id))
    {
        return None;
    }
    let mut uncategorised = ChannelSection {
        category: None,
        channels: Vec::new(),
    };
    let mut category_sections: Vec<ChannelSection> = placements
        .iter()
        .filter_map(|placement| {
            by_id
                .get(&placement.id)
                .filter(|channel| channel.kind == ChannelKind::Category)
        })
        .map(|category| ChannelSection {
            category: Some((*category).clone()),
            channels: Vec::new(),
        })
        .collect();
    for placement in placements {
        let Some(channel) = by_id
            .get(&placement.id)
            .filter(|channel| channel.kind != ChannelKind::Category)
        else {
            continue;
        };
        let moved = Channel {
            parent_id: placement.parent_id,
            ..(*channel).clone()
        };
        let section = placement
            .parent_id
            .and_then(|parent| {
                category_sections
                    .iter_mut()
                    .find(|section| section.category_id() == Some(parent))
            })
            .unwrap_or(&mut uncategorised);
        section.channels.push(moved);
    }
    let mut sections = Vec::with_capacity(category_sections.len() + 1);
    if !uncategorised.channels.is_empty() {
        sections.push(uncategorised);
    }
    sections.append(&mut category_sections);
    Some(sections)
}

fn section_of_channel(
    sections: &[ChannelSection],
    channel_id: Snowflake,
) -> Option<&ChannelSection> {
    sections.iter().find(|section| {
        section
            .channels
            .iter()
            .any(|channel| channel.id == channel_id)
    })
}

/// The category the channel is listed under (`Some(None)`: none); `None` when it is not listed.
#[must_use]
pub fn listed_category_of(
    sections: &[ChannelSection],
    channel_id: Snowflake,
) -> Option<Option<Snowflake>> {
    section_of_channel(sections, channel_id).map(ChannelSection::category_id)
}

/// The sections with the channel moved to `destination`, or `None` when nothing would change (or
/// either end is gone). The index counts the rows as listed, the moved one included, so a drop
/// below a row of the same section lands where the line was drawn.
#[must_use]
pub fn with_channel_moved(
    sections: &[ChannelSection],
    channel_id: Snowflake,
    destination: ChannelDestination,
) -> Option<Vec<ChannelSection>> {
    let source = section_of_channel(sections, channel_id)?;
    let source_index = source
        .channels
        .iter()
        .position(|channel| channel.id == channel_id)?;
    let channel = source.channels.get(source_index)?;
    if let Some(category_id) = destination.category_id
        && !sections
            .iter()
            .any(|section| section.category_id() == Some(category_id))
    {
        return None;
    }
    let is_same_section = source.category_id() == destination.category_id;
    let index = if is_same_section && source_index < destination.index {
        destination.index - 1
    } else {
        destination.index
    };
    // Out of a category: none. Within the uncategorised rows: its own parent (a hidden category's).
    let parent_id =
        destination
            .category_id
            .or(if is_same_section && !destination.leaves_hidden_category {
                channel.parent_id
            } else {
                None
            });
    let moved = Channel {
        parent_id,
        ..channel.clone()
    };
    let mut next: Vec<ChannelSection> = sections
        .iter()
        .map(|section| ChannelSection {
            category: section.category.clone(),
            channels: section
                .channels
                .iter()
                .filter(|candidate| candidate.id != channel_id)
                .cloned()
                .collect(),
        })
        .collect();
    if destination.category_id.is_none() && !next.iter().any(|section| section.category.is_none()) {
        next.insert(
            0,
            ChannelSection {
                category: None,
                channels: Vec::new(),
            },
        );
    }
    let target = next
        .iter_mut()
        .find(|section| section.category_id() == destination.category_id)?;
    let insert_at = index.min(target.channels.len());
    target.channels.insert(insert_at, moved);
    next.retain(|section| section.category.is_some() || !section.channels.is_empty());
    (placements_of(&next) != placements_of(sections)).then_some(next)
}

/// The sections with the category (and its channels) moved to `index` among the categories, the
/// moved one included as for channels; `None` when nothing would change.
#[must_use]
pub fn with_category_moved(
    sections: &[ChannelSection],
    category_id: Snowflake,
    index: usize,
) -> Option<Vec<ChannelSection>> {
    let category_sections: Vec<&ChannelSection> = sections
        .iter()
        .filter(|section| section.category.is_some())
        .collect();
    let source_index = category_sections
        .iter()
        .position(|section| section.category_id() == Some(category_id))?;
    let target_index = if source_index < index {
        index - 1
    } else {
        index
    };
    let mut reordered: Vec<&ChannelSection> = category_sections.clone();
    let moving = reordered.remove(source_index);
    reordered.insert(target_index.min(reordered.len()), moving);
    let result: Vec<ChannelSection> = sections
        .iter()
        .filter(|section| section.category.is_none())
        .chain(reordered)
        .cloned()
        .collect();
    (placements_of(&result) != placements_of(sections)).then_some(result)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Up,
    Down,
}

/// "Move up" and "Move down" of a channel: within its section, `None` at its edge.
#[must_use]
pub fn channel_step_destination(
    sections: &[ChannelSection],
    channel_id: Snowflake,
    step: Step,
) -> Option<ChannelDestination> {
    let section = section_of_channel(sections, channel_id)?;
    let index = section
        .channels
        .iter()
        .position(|channel| channel.id == channel_id)?;
    // As listed, the moved row included: one down is past the next row.
    let target = match step {
        Step::Up => index.checked_sub(1)?,
        Step::Down => Some(index + 1).filter(|target| *target < section.channels.len())? + 1,
    };
    Some(ChannelDestination {
        category_id: section.category_id(),
        index: target,
        leaves_hidden_category: false,
    })
}

/// "Move up" and "Move down" of a category, as an index for [`with_category_moved`]; `None` at an edge.
#[must_use]
pub fn category_step_index(
    sections: &[ChannelSection],
    category_id: Snowflake,
    step: Step,
) -> Option<usize> {
    let categories: Vec<Snowflake> = sections
        .iter()
        .filter_map(ChannelSection::category_id)
        .collect();
    let index = categories.iter().position(|id| *id == category_id)?;
    match step {
        Step::Up => index.checked_sub(1),
        Step::Down => Some(index + 1)
            .filter(|target| *target < categories.len())
            .map(|target| target + 1),
    }
}

// --- Dropping ----------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListedKind {
    Category,
    Channel,
}

/// A row or category header as laid out, top to bottom, in one vertical coordinate system.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListedBox {
    pub kind: ListedKind,
    pub id: Snowflake,
    pub top: f64,
    pub bottom: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DropIndicator {
    Line { y: f64 },
    Category { id: Snowflake },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelDrop {
    pub destination: ChannelDestination,
    pub indicator: DropIndicator,
}

/// The candidate nearest to `y`; the first one on a tie.
fn nearest_by_y<T>(candidates: impl Iterator<Item = (f64, T)>, y: f64) -> Option<(f64, T)> {
    candidates.fold(None, |best, candidate| match best {
        Some(best) if (candidate.0 - y).abs() >= (best.0 - y).abs() => Some(best),
        _ => Some(candidate),
    })
}

/// Where a carried channel would land with the pointer at `y`: on a category header, into that
/// category at its top (the only way into a collapsed one); otherwise at the nearest gap between
/// rows. Without `may_change_category` only its own section's gaps count. `None` when the drop
/// would change nothing.
#[must_use]
pub fn channel_drop_at(
    sections: &[ChannelSection],
    boxes: &[ListedBox],
    channel_id: Snowflake,
    y: f64,
    may_change_category: bool,
) -> Option<ChannelDrop> {
    let own_category_id = listed_category_of(sections, channel_id)?;
    let is_allowed =
        |category_id: Option<Snowflake>| may_change_category || category_id == own_category_id;
    let result_for = |drop: ChannelDrop| {
        with_channel_moved(sections, channel_id, drop.destination).map(|_| drop)
    };
    let destination = |category_id: Option<Snowflake>, index: usize| ChannelDestination {
        category_id,
        index,
        leaves_hidden_category: false,
    };

    let header = boxes.iter().find(|candidate| {
        candidate.kind == ListedKind::Category && y >= candidate.top && y <= candidate.bottom
    });
    if let Some(header) = header.filter(|header| is_allowed(Some(header.id))) {
        return result_for(ChannelDrop {
            destination: destination(Some(header.id), 0),
            indicator: DropIndicator::Category { id: header.id },
        });
    }

    let mut gaps: Vec<(f64, ChannelDestination)> = Vec::new();
    if let Some(first) = boxes
        .first()
        .filter(|_| !sections.iter().any(|section| section.category.is_none()))
    {
        gaps.push((first.top, destination(None, 0)));
    }
    for listed in boxes
        .iter()
        .filter(|listed| listed.kind == ListedKind::Channel)
    {
        let Some(section) = section_of_channel(sections, listed.id) else {
            continue;
        };
        let Some(index) = section
            .channels
            .iter()
            .position(|channel| channel.id == listed.id)
        else {
            continue;
        };
        let category_id = section.category_id();
        gaps.push((listed.top, destination(category_id, index)));
        gaps.push((listed.bottom, destination(category_id, index + 1)));
    }
    // An expanded category with no channel yet: just under its header.
    for listed in boxes
        .iter()
        .filter(|listed| listed.kind == ListedKind::Category)
    {
        if sections
            .iter()
            .any(|section| section.category_id() == Some(listed.id) && section.channels.is_empty())
        {
            gaps.push((listed.bottom, destination(Some(listed.id), 0)));
        }
    }
    let (gap_y, nearest) = nearest_by_y(
        gaps.into_iter()
            .filter(|(_, gap)| is_allowed(gap.category_id)),
        y,
    )?;
    result_for(ChannelDrop {
        destination: nearest,
        indicator: DropIndicator::Line { y: gap_y },
    })
}

/// Where a carried category would land: before the nearest category header, or after the last
/// row. Returns the index for [`with_category_moved`] and the line's `y`; `None` when that changes nothing.
#[must_use]
pub fn category_drop_at(
    sections: &[ChannelSection],
    boxes: &[ListedBox],
    category_id: Snowflake,
    y: f64,
) -> Option<(usize, f64)> {
    let categories: Vec<Snowflake> = sections
        .iter()
        .filter_map(ChannelSection::category_id)
        .collect();
    let headers: Vec<&ListedBox> = boxes
        .iter()
        .filter(|listed| listed.kind == ListedKind::Category)
        .collect();
    let last_bottom = boxes
        .iter()
        .map(|listed| listed.bottom)
        .fold(f64::NEG_INFINITY, f64::max);
    let gaps = headers
        .iter()
        .filter_map(|header| {
            categories
                .iter()
                .position(|id| *id == header.id)
                .map(|index| (header.top, index))
        })
        .chain((!headers.is_empty()).then_some((last_bottom, categories.len())));
    let (gap_y, index) = nearest_by_y(gaps, y)?;
    with_category_moved(sections, category_id, index).map(|_| (index, gap_y))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOBBY: u64 = 1;
    const TEXT: u64 = 2;
    const GENERAL: u64 = 3;
    const LINKS: u64 = 4;
    const GAMES: u64 = 5;
    const CLIPS: u64 = 6;
    const EMPTY: u64 = 7;
    const SECRET: u64 = 8;
    const OPEN: u64 = 9;
    const HIDDEN: u64 = 99;
    const MISSING: u64 = 999;

    fn id(raw: u64) -> Snowflake {
        Snowflake::from_raw(raw)
    }

    fn channel(raw_id: u64, kind: ChannelKind, position: u32, parent: Option<u64>) -> Channel {
        Channel {
            id: id(raw_id),
            guild_id: id(1_000),
            kind,
            name: raw_id.to_string(),
            topic: String::new(),
            parent_id: parent.map(id),
            position,
            icon: None,
            permission_overwrites: Vec::new(),
            user_limit: None,
        }
    }

    // lobby (uncategorised) · Text: general, links · Games: clips · Empty: (none)
    fn channels() -> Vec<Channel> {
        vec![
            channel(LOBBY, ChannelKind::Text, 0, None),
            channel(TEXT, ChannelKind::Category, 1, None),
            channel(GENERAL, ChannelKind::Text, 2, Some(TEXT)),
            channel(LINKS, ChannelKind::Text, 3, Some(TEXT)),
            channel(GAMES, ChannelKind::Category, 4, None),
            channel(CLIPS, ChannelKind::Text, 5, Some(GAMES)),
            channel(EMPTY, ChannelKind::Category, 6, None),
        ]
    }

    fn sections() -> Vec<ChannelSection> {
        channel_sections(&channels())
    }

    /// Each section as (category or none, channel ids).
    fn shape(sections: Option<&[ChannelSection]>) -> Option<Vec<(Option<u64>, Vec<u64>)>> {
        sections.map(|sections| {
            sections
                .iter()
                .map(|section| {
                    (
                        section.category_id().map(Snowflake::raw),
                        section
                            .channels
                            .iter()
                            .map(|channel| channel.id.raw())
                            .collect(),
                    )
                })
                .collect()
        })
    }

    fn placement(raw_id: u64, parent: Option<u64>) -> ChannelPlacement {
        ChannelPlacement {
            id: id(raw_id),
            parent_id: parent.map(id),
        }
    }

    fn to(category: Option<u64>, index: usize) -> ChannelDestination {
        ChannelDestination {
            category_id: category.map(id),
            index,
            leaves_hidden_category: false,
        }
    }

    #[test]
    fn lists_uncategorised_channels_first_then_categories_by_position_voice_among_text() {
        let guild = vec![
            channel(20, ChannelKind::Category, 1, None),
            channel(10, ChannelKind::Category, 0, None),
            channel(21, ChannelKind::Text, 0, Some(20)),
            channel(3, ChannelKind::Text, 1, None),
            channel(2, ChannelKind::Text, 0, None),
        ];
        assert_eq!(
            shape(Some(&channel_sections(&guild))),
            Some(vec![
                (None, vec![2, 3]),
                (Some(10), vec![]),
                (Some(20), vec![21])
            ])
        );
        let voice = vec![
            channel(10, ChannelKind::Category, 0, None),
            channel(12, ChannelKind::Text, 1, Some(10)),
            channel(11, ChannelKind::Voice, 0, Some(10)),
            channel(13, ChannelKind::Voice, 2, Some(10)),
        ];
        assert_eq!(
            shape(Some(&channel_sections(&voice))),
            Some(vec![(Some(10), vec![11, 12, 13])])
        );
        let hidden_parent = channel_sections(&[channel(30, ChannelKind::Text, 2, Some(HIDDEN))]);
        assert_eq!(
            hidden_parent.first().map(|section| (
                section.category.is_none(),
                section.channels.first().and_then(|c| c.parent_id)
            )),
            Some((true, Some(id(HIDDEN))))
        );
    }

    #[test]
    fn places_every_channel_once_each_category_before_its_channels() {
        assert_eq!(
            placements_of(&sections()),
            vec![
                placement(LOBBY, None),
                placement(TEXT, None),
                placement(GENERAL, Some(TEXT)),
                placement(LINKS, Some(TEXT)),
                placement(GAMES, None),
                placement(CLIPS, Some(GAMES)),
                placement(EMPTY, None),
            ]
        );
        let hidden = channel_sections(&[
            channel(SECRET, ChannelKind::Text, 0, Some(HIDDEN)),
            channel(OPEN, ChannelKind::Text, 1, None),
        ]);
        assert_eq!(
            placements_of(&hidden),
            vec![placement(SECRET, Some(HIDDEN)), placement(OPEN, None)]
        );
    }

    #[test]
    fn moves_down_within_a_section_counting_the_moved_row_as_listed() {
        assert_eq!(
            shape(with_channel_moved(&sections(), id(GENERAL), to(Some(TEXT), 2)).as_deref()),
            Some(vec![
                (None, vec![LOBBY]),
                (Some(TEXT), vec![LINKS, GENERAL]),
                (Some(GAMES), vec![CLIPS]),
                (Some(EMPTY), vec![])
            ])
        );
    }

    #[test]
    fn moves_into_another_category_an_empty_one_and_out_of_any() {
        let section_at = |moved: Option<Vec<ChannelSection>>, index: usize| {
            shape(moved.as_deref()).and_then(|shape| shape.get(index).cloned())
        };
        assert_eq!(
            section_at(
                with_channel_moved(&sections(), id(GENERAL), to(Some(GAMES), 0)),
                2
            ),
            Some((Some(GAMES), vec![GENERAL, CLIPS]))
        );
        assert_eq!(
            section_at(
                with_channel_moved(&sections(), id(CLIPS), to(Some(EMPTY), 0)),
                3
            ),
            Some((Some(EMPTY), vec![CLIPS]))
        );
        let out = with_channel_moved(&sections(), id(CLIPS), to(None, 1));
        assert_eq!(section_at(out.clone(), 0), Some((None, vec![LOBBY, CLIPS])));
        assert!(placements_of(&out.unwrap_or_default()).contains(&placement(CLIPS, None)));
    }

    #[test]
    fn creates_the_uncategorised_rows_when_there_were_none_and_drops_them_once_empty() {
        let without_lobby: Vec<Channel> = channels()
            .into_iter()
            .filter(|channel| channel.id != id(LOBBY))
            .collect();
        let moved = with_channel_moved(&channel_sections(&without_lobby), id(CLIPS), to(None, 0));
        assert_eq!(
            shape(moved.as_deref()).and_then(|shape| shape.first().cloned()),
            Some((None, vec![CLIPS]))
        );
        let moved = with_channel_moved(&sections(), id(LOBBY), to(Some(GAMES), 1));
        assert_eq!(
            shape(moved.as_deref()).and_then(|shape| shape.first().cloned()),
            Some((Some(TEXT), vec![GENERAL, LINKS]))
        );
    }

    #[test]
    fn returns_none_when_nothing_changes_or_an_end_is_gone() {
        assert_eq!(
            with_channel_moved(&sections(), id(GENERAL), to(Some(TEXT), 0)),
            None
        );
        assert_eq!(
            with_channel_moved(&sections(), id(GENERAL), to(Some(TEXT), 1)),
            None
        );
        assert_eq!(
            with_channel_moved(&sections(), id(MISSING), to(Some(TEXT), 0)),
            None
        );
        assert_eq!(
            with_channel_moved(&sections(), id(GENERAL), to(Some(MISSING), 0)),
            None
        );
    }

    #[test]
    fn keeps_a_hidden_categorys_channel_in_it_unless_no_category_is_chosen_on_purpose() {
        let hidden = channel_sections(&[
            channel(OPEN, ChannelKind::Text, 0, None),
            channel(SECRET, ChannelKind::Text, 1, Some(HIDDEN)),
        ]);
        let moved = with_channel_moved(&hidden, id(SECRET), to(None, 0)).unwrap_or_default();
        assert_eq!(
            placements_of(&moved),
            vec![placement(SECRET, Some(HIDDEN)), placement(OPEN, None)]
        );
        let leaving = ChannelDestination {
            leaves_hidden_category: true,
            ..to(None, 2)
        };
        let moved = with_channel_moved(&hidden, id(SECRET), leaving).unwrap_or_default();
        assert_eq!(
            placements_of(&moved),
            vec![placement(OPEN, None), placement(SECRET, None)]
        );
    }

    fn category_order(sections: Option<&[ChannelSection]>) -> Option<Vec<Option<u64>>> {
        shape(sections).map(|shape| shape.into_iter().map(|(category, _)| category).collect())
    }

    #[test]
    fn moves_a_category_with_its_channels_uncategorised_rows_staying_first() {
        assert_eq!(
            shape(with_category_moved(&sections(), id(GAMES), 0).as_deref()),
            Some(vec![
                (None, vec![LOBBY]),
                (Some(GAMES), vec![CLIPS]),
                (Some(TEXT), vec![GENERAL, LINKS]),
                (Some(EMPTY), vec![])
            ])
        );
        assert_eq!(
            category_order(with_category_moved(&sections(), id(TEXT), 3).as_deref()),
            Some(vec![None, Some(GAMES), Some(EMPTY), Some(TEXT)])
        );
        assert_eq!(with_category_moved(&sections(), id(GAMES), 1), None);
        assert_eq!(with_category_moved(&sections(), id(GAMES), 2), None);
        assert_eq!(with_category_moved(&sections(), id(MISSING), 0), None);
    }

    #[test]
    fn steps_channels_within_their_section_and_categories_among_categories() {
        let sections = sections();
        assert_eq!(
            channel_step_destination(&sections, id(GENERAL), Step::Up),
            None
        );
        assert_eq!(
            channel_step_destination(&sections, id(LINKS), Step::Down),
            None
        );
        let second_section = |moved: Option<Vec<ChannelSection>>| {
            shape(moved.as_deref()).and_then(|shape| shape.get(1).cloned())
        };
        let down =
            channel_step_destination(&sections, id(GENERAL), Step::Down).unwrap_or(to(None, 0));
        assert_eq!(
            second_section(with_channel_moved(&sections, id(GENERAL), down)),
            Some((Some(TEXT), vec![LINKS, GENERAL]))
        );
        let up = channel_step_destination(&sections, id(LINKS), Step::Up).unwrap_or(to(None, 0));
        assert_eq!(
            second_section(with_channel_moved(&sections, id(LINKS), up)),
            Some((Some(TEXT), vec![LINKS, GENERAL]))
        );
        assert_eq!(category_step_index(&sections, id(TEXT), Step::Up), None);
        assert_eq!(category_step_index(&sections, id(EMPTY), Step::Down), None);
        let text_down = category_step_index(&sections, id(TEXT), Step::Down).unwrap_or(0);
        assert_eq!(
            category_order(with_category_moved(&sections, id(TEXT), text_down).as_deref()),
            Some(vec![None, Some(GAMES), Some(TEXT), Some(EMPTY)])
        );
        let empty_up = category_step_index(&sections, id(EMPTY), Step::Up).unwrap_or(0);
        assert_eq!(
            category_order(with_category_moved(&sections, id(EMPTY), empty_up).as_deref()),
            Some(vec![None, Some(TEXT), Some(EMPTY), Some(GAMES)])
        );
    }

    #[test]
    fn shows_a_pending_order_with_the_channels_as_they_are_now() {
        let moved =
            with_channel_moved(&sections(), id(CLIPS), to(Some(TEXT), 0)).unwrap_or_default();
        let renamed: Vec<Channel> = channels()
            .into_iter()
            .map(|channel| {
                if channel.id == id(CLIPS) {
                    Channel {
                        name: "Clips".into(),
                        ..channel
                    }
                } else {
                    channel
                }
            })
            .collect();
        let shown = sections_from_placements(&renamed, &placements_of(&moved));
        assert_eq!(shape(shown.as_deref()), shape(Some(&moved)));
        let first_in_text = shown
            .as_ref()
            .and_then(|shown| shown.get(1))
            .and_then(|section| section.channels.first());
        assert_eq!(
            first_in_text.map(|channel| (channel.name.as_str(), channel.parent_id)),
            Some(("Clips", Some(id(TEXT))))
        );
    }

    #[test]
    fn gives_way_once_a_channel_was_created_or_deleted_meanwhile_and_round_trips() {
        let placements = placements_of(&sections());
        assert_eq!(
            sections_from_placements(channels().get(1..).unwrap_or_default(), &placements),
            None
        );
        let mut with_new = channels();
        with_new.push(channel(10, ChannelKind::Text, 9, None));
        assert_eq!(sections_from_placements(&with_new, &placements), None);
        let round_trip = sections_from_placements(&channels(), &placements).unwrap_or_default();
        assert_eq!(placements_of(&round_trip), placements);
    }

    #[test]
    fn settles_a_pending_order_when_echoed_or_once_answered_channels_are_replaced() {
        let before = placements_of(&sections());
        let pending: Vec<ChannelPlacement> = before
            .iter()
            .rev()
            .map(|placement| ChannelPlacement {
                parent_id: None,
                ..*placement
            })
            .collect();
        assert!(is_pending_order_settled(&pending, None, &pending, 1));
        assert!(!is_pending_order_settled(&pending, None, &before, 2));
        assert!(!is_pending_order_settled(&pending, Some(1), &before, 1));
        assert!(is_pending_order_settled(&pending, Some(1), &before, 2));
    }

    // Rows 32 px tall, headers 24 px, top to bottom as the sidebar lays them out.
    fn boxes() -> Vec<ListedBox> {
        let listed = |kind, raw_id, top, bottom| ListedBox {
            kind,
            id: id(raw_id),
            top,
            bottom,
        };
        vec![
            listed(ListedKind::Channel, LOBBY, 0.0, 32.0),
            listed(ListedKind::Category, TEXT, 48.0, 72.0),
            listed(ListedKind::Channel, GENERAL, 72.0, 104.0),
            listed(ListedKind::Channel, LINKS, 106.0, 138.0),
            listed(ListedKind::Category, GAMES, 154.0, 178.0),
            listed(ListedKind::Channel, CLIPS, 178.0, 210.0),
            listed(ListedKind::Category, EMPTY, 226.0, 250.0),
        ]
    }

    #[test]
    fn puts_a_channel_at_the_nearest_gap_or_into_a_category_from_its_header() {
        let (sections, boxes) = (sections(), boxes());
        assert_eq!(
            channel_drop_at(&sections, &boxes, id(GENERAL), 140.0, true),
            Some(ChannelDrop {
                destination: to(Some(TEXT), 2),
                indicator: DropIndicator::Line { y: 138.0 }
            })
        );
        assert_eq!(
            channel_drop_at(&sections, &boxes, id(GENERAL), 205.0, true),
            Some(ChannelDrop {
                destination: to(Some(GAMES), 1),
                indicator: DropIndicator::Line { y: 210.0 }
            })
        );
        assert_eq!(
            channel_drop_at(&sections, &boxes, id(CLIPS), 60.0, true),
            Some(ChannelDrop {
                destination: to(Some(TEXT), 0),
                indicator: DropIndicator::Category { id: id(TEXT) }
            })
        );
        assert_eq!(
            channel_drop_at(&sections, &boxes, id(CLIPS), 240.0, true).map(|drop| drop.destination),
            Some(to(Some(EMPTY), 0))
        );
        assert_eq!(
            channel_drop_at(&sections, &boxes, id(GENERAL), 80.0, true),
            None
        );
        assert_eq!(
            channel_drop_at(&sections, &boxes, id(GENERAL), 104.0, true),
            None
        );
        assert_eq!(
            channel_drop_at(&sections, &boxes, id(GENERAL), 205.0, false)
                .map(|drop| drop.destination),
            Some(to(Some(TEXT), 2))
        );
        assert_eq!(
            channel_drop_at(&sections, &boxes, id(GENERAL), 166.0, false)
                .map(|drop| drop.destination),
            Some(to(Some(TEXT), 2))
        );
    }

    #[test]
    fn offers_the_top_of_the_list_as_uncategorised_when_no_channel_is() {
        let without_lobby: Vec<Channel> = channels()
            .into_iter()
            .filter(|channel| channel.id != id(LOBBY))
            .collect();
        let boxes = boxes();
        assert_eq!(
            channel_drop_at(
                &channel_sections(&without_lobby),
                boxes.get(1..).unwrap_or_default(),
                id(CLIPS),
                40.0,
                true
            ),
            Some(ChannelDrop {
                destination: to(None, 0),
                indicator: DropIndicator::Line { y: 48.0 }
            })
        );
    }

    #[test]
    fn puts_a_category_before_the_nearest_header_or_after_the_last_row() {
        let (sections, boxes) = (sections(), boxes());
        assert_eq!(
            category_drop_at(&sections, &boxes, id(GAMES), 50.0),
            Some((0, 48.0))
        );
        assert_eq!(
            category_drop_at(&sections, &boxes, id(GAMES), 260.0),
            Some((3, 250.0))
        );
        assert_eq!(category_drop_at(&sections, &boxes, id(GAMES), 160.0), None);
    }
}
