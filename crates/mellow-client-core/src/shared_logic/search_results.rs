//! Search results as the side sheet lists them (`web/src/components/search-results.ts`, DESIGN.md
//! 5.12): labelled by channel where the channel changes, and facets that narrow the query.

use mellow_protocol::{MessageSearchHit, Snowflake};

use super::search_query::{
    SearchFilterKey, channel_filter_value, search_query_parts, with_search_filter,
};

/// For each hit in the instance's order, whether it opens a run of results from one channel:
/// that channel's label shows above it.
#[must_use]
pub fn starts_channel_runs(hits: &[MessageSearchHit]) -> Vec<bool> {
    hits.iter()
        .enumerate()
        .map(|(index, hit)| {
            index
                .checked_sub(1)
                .and_then(|previous| hits.get(previous))
                .is_none_or(|previous| previous.message.channel_id != hit.message.channel_id)
        })
        .collect()
}

/// The query narrowed to a channel: by name in a server (when the name stays one part), by id for
/// a conversation (`name` `None`), whose only name is its people's.
#[must_use]
pub fn query_narrowed_to_channel(query: &str, channel_id: Snowflake, name: Option<&str>) -> String {
    let value = match name {
        Some(name) => channel_filter_value(channel_id, name),
        None => channel_id.raw().to_string(),
    };
    with_search_filter(query, SearchFilterKey::In, &value)
}

#[must_use]
pub fn query_narrowed_to_author(query: &str, username: &str) -> String {
    with_search_filter(query, SearchFilterKey::From, username)
}

/// Whether the query already narrows by `key`: its facets then have nothing left to offer.
#[must_use]
pub fn is_narrowed_by(query: &str, key: SearchFilterKey) -> bool {
    search_query_parts(query)
        .iter()
        .any(|part| part.filter.as_ref().is_some_and(|filter| filter.key == key))
}

#[cfg(test)]
pub(crate) mod tests {
    use mellow_protocol::{Message, PublicUser};

    use super::*;

    pub(crate) fn hit(id: u64, channel_id: u64) -> MessageSearchHit {
        MessageSearchHit {
            message: Message {
                id: Snowflake::from_raw(id),
                channel_id: Snowflake::from_raw(channel_id),
                author: PublicUser {
                    id: Snowflake::from_raw(1),
                    username: "ada".into(),
                    display_name: "Ada".into(),
                    avatar: None,
                    banner: None,
                },
                content: format!("message {id}"),
                reply_to_id: None,
                edited_at_unix_milliseconds: None,
                nonce: None,
                attachments: Vec::new(),
                reactions: Vec::new(),
                pinned: false,
                mention_ids: Vec::new(),
                mention_role_ids: Vec::new(),
                link_previews: Vec::new(),
                poll: None,
                checklist: None,
                progress_bar: None,
                forwarded_from: None,
            },
            highlights: Vec::new(),
        }
    }

    #[test]
    fn labels_the_first_result_of_each_run_from_one_channel() {
        let hits = [hit(1, 10), hit(2, 10), hit(3, 11), hit(4, 10)];
        assert_eq!(starts_channel_runs(&hits), vec![true, false, true, true]);
    }

    #[test]
    fn narrows_the_query_to_a_facets_channel_or_author() {
        let id = Snowflake::from_raw(9);
        assert_eq!(
            query_narrowed_to_channel("pizza", id, Some("général")),
            "pizza in:général"
        );
        assert_eq!(
            query_narrowed_to_channel("pizza", id, Some("Game night")),
            "pizza in:9"
        );
        assert_eq!(query_narrowed_to_channel("pizza", id, None), "pizza in:9");
        assert_eq!(
            query_narrowed_to_author("pizza in:9", "ada"),
            "pizza in:9 from:ada"
        );
        assert_eq!(
            query_narrowed_to_author("pizza", "Paul Carouge"),
            "pizza from:\"Paul Carouge\""
        );
    }

    #[test]
    fn knows_which_filters_already_narrow_the_query() {
        assert!(is_narrowed_by("pizza in:9", SearchFilterKey::In));
        assert!(!is_narrowed_by("pizza in:9", SearchFilterKey::From));
    }
}
