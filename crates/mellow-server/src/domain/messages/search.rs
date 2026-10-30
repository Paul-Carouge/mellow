//! Message search (ADR-0029): in the guild channels where the requester may read history, as
//! decided by `channel_permissions`, or in their own conversations, as decided by
//! `direct_message_permissions` (ADR-0007). Channels outside that scope are never queried, so
//! they cannot show in results, counts or facets.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use mellow_protocol::{
    AuthorSearchFacet, ChannelSearchFacet, MAXIMUM_COUNTED_SEARCH_MATCHES, MAXIMUM_SEARCH_FACETS,
    MessageSearchHit, MessageSearchQuery, MessageSearchResults, MessageSearchSort, Permissions,
    PublicUser, Snowflake,
};

use super::{MessageError, Messages};
use crate::domain::guilds::GuildError;
use crate::domain::message_rules::MessageRuleViolation;
use crate::domain::permissions::direct_message_permissions;
use crate::domain::profile_images::fill_profile_images;
use crate::domain::search_query::{
    HIGHLIGHT_END_MARK, HIGHLIGHT_START_MARK, SearchRequest, highlight_ranges,
    validated_search_request,
};
use crate::storage::direct_messages as direct_message_storage;
use crate::storage::messages as message_storage;
use crate::storage::search::{self as search_storage, MatchCount, SearchCriteria, SearchOrder};

/// A channel the requester may search, as `in:` can name it.
struct SearchableChannel {
    id: Snowflake,
    /// `None` for conversations, which `in:` names by id only.
    name: Option<String>,
}

impl Messages {
    /// Searches the guild's text channels where `user_id` may view and read history. A guild
    /// they are not a member of is `NotFound`.
    pub(crate) async fn search_guild_messages(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        query: &MessageSearchQuery,
    ) -> Result<MessageSearchResults, MessageError> {
        let request = validated_search_request(query).map_err(MessageRuleViolation::from)?;
        let channels: Vec<SearchableChannel> = self
            .guilds
            .channels_with_readable_history(user_id, guild_id)
            .await?
            .into_iter()
            .map(|channel| SearchableChannel {
                id: channel.id,
                name: Some(channel.name),
            })
            .collect();
        self.search_in(user_id, &channels, &request).await
    }

    /// Searches the conversations and groups `user_id` takes part in.
    pub(crate) async fn search_my_conversation_messages(
        &self,
        user_id: Snowflake,
        query: &MessageSearchQuery,
    ) -> Result<MessageSearchResults, MessageError> {
        let request = validated_search_request(query).map_err(MessageRuleViolation::from)?;
        let readable = Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY;
        let channels: Vec<SearchableChannel> =
            direct_message_storage::list_user_conversations(&self.database, user_id)
                .await
                .map_err(GuildError::from)?
                .into_iter()
                // Listed because they take part; reading needs no reachability (ADR-0025).
                .filter(|conversation| {
                    direct_message_permissions(true, conversation.owner_id.is_some(), false)
                        .contains(readable)
                })
                .map(|conversation| SearchableChannel {
                    id: conversation.id,
                    name: None,
                })
                .collect();
        self.search_in(user_id, &channels, &request).await
    }

    async fn search_in(
        &self,
        user_id: Snowflake,
        channels: &[SearchableChannel],
        request: &SearchRequest,
    ) -> Result<MessageSearchResults, MessageError> {
        let _one_search_of_this_user = self.start_search_of(user_id)?;
        let _reader_permit = self
            .search_permits
            .acquire()
            .await
            .map_err(|error| MessageError::Internal(Box::new(error)))?;
        let parsed = &request.parsed;
        let channel_ids = channels_named_by(channels, &parsed.channel_references);
        if parsed.is_empty() || channel_ids.is_empty() {
            return Ok(no_results());
        }
        let (lowest_id, id_below) = parsed.id_bounds(request.utc_offset_minutes);
        let criteria = SearchCriteria {
            channel_ids,
            match_expression: parsed.match_expression(),
            exclusion_expression: parsed.standalone_exclusion_expression(),
            author_usernames: parsed.author_usernames.clone(),
            mentioned_usernames: parsed.mentioned_usernames.clone(),
            required_contents: parsed.required_contents,
            lowest_id,
            id_below,
        };
        let order = match request.sort {
            MessageSearchSort::Relevance => SearchOrder::Relevance,
            MessageSearchSort::Newest => SearchOrder::Newest,
        };
        let page_ids = search_storage::search_message_ids(
            &self.database,
            &criteria,
            order,
            request.offset,
            request.limit,
        )
        .await?;
        let counted = search_storage::count_matches(
            &self.database,
            &criteria,
            MAXIMUM_COUNTED_SEARCH_MATCHES,
        )
        .await?;
        let counts = counted.counts;
        let hits = self
            .hits_in_order(user_id, &page_ids, criteria.match_expression.as_deref())
            .await?;
        let author_facets = self.author_facets(&counts).await?;
        Ok(MessageSearchResults {
            hits,
            total_count: counts.iter().map(|count| count.count).sum(),
            total_count_is_lower_bound: counted.has_more,
            channel_facets: channel_facets(&counts),
            author_facets,
        })
    }

    /// The messages of `page_ids` in that order, with their highlights; one deleted since the
    /// page was read is left out.
    async fn hits_in_order(
        &self,
        viewer_id: Snowflake,
        page_ids: &[Snowflake],
        match_expression: Option<&str>,
    ) -> Result<Vec<MessageSearchHit>, MessageError> {
        let mut stored: HashMap<Snowflake, _> =
            message_storage::find_messages_by_ids(&self.database, page_ids)
                .await?
                .into_iter()
                .map(|message| (message.id, message))
                .collect();
        let ordered = page_ids.iter().filter_map(|id| stored.remove(id)).collect();
        let messages = self.messages_for_viewer(viewer_id, ordered).await?;
        let mut marked_contents: HashMap<Snowflake, String> = match match_expression {
            Some(expression) => search_storage::highlighted_contents(
                &self.database,
                expression,
                page_ids,
                HIGHLIGHT_START_MARK,
                HIGHLIGHT_END_MARK,
            )
            .await?
            .into_iter()
            .collect(),
            None => HashMap::new(),
        };
        Ok(messages
            .into_iter()
            .map(|message| MessageSearchHit {
                highlights: marked_contents
                    .remove(&message.id)
                    .map(|marked| highlight_ranges(&marked, &message.content))
                    .unwrap_or_default(),
                message,
            })
            .collect())
    }

    async fn author_facets(
        &self,
        counts: &[MatchCount],
    ) -> Result<Vec<AuthorSearchFacet>, MessageError> {
        let top_authors = largest_totals(counts.iter().map(|count| (count.author_id, count.count)));
        let author_ids: Vec<Snowflake> = top_authors.iter().map(|(id, _)| *id).collect();
        let mut users: HashMap<Snowflake, PublicUser> =
            message_storage::mentionable_users(&self.database, &author_ids)
                .await?
                .into_iter()
                .map(|(id, username, display_name)| {
                    let user = PublicUser {
                        id,
                        username,
                        display_name,
                        avatar: None,
                        banner: None,
                    };
                    (id, user)
                })
                .collect();
        fill_profile_images(&self.database, users.values_mut()).await?;
        Ok(top_authors
            .into_iter()
            .filter_map(|(id, count)| {
                let author = users.remove(&id)?;
                Some(AuthorSearchFacet { author, count })
            })
            .collect())
    }
}

/// Marks a user's search as running until dropped (`Messages::start_search_of`).
#[derive(Debug)]
pub(crate) struct RunningSearch {
    running: Arc<Mutex<HashSet<Snowflake>>>,
    user_id: Snowflake,
}

impl Drop for RunningSearch {
    fn drop(&mut self) {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.user_id);
    }
}

impl Messages {
    /// One search at a time per user: a second one while the first runs is refused, so a client
    /// typing fast cannot hold several readers at once (ADR-0029).
    fn start_search_of(&self, user_id: Snowflake) -> Result<RunningSearch, MessageError> {
        let is_new = self
            .running_searches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(user_id);
        if is_new {
            Ok(RunningSearch {
                running: self.running_searches.clone(),
                user_id,
            })
        } else {
            Err(MessageError::SearchAlreadyRunning)
        }
    }
}

fn no_results() -> MessageSearchResults {
    MessageSearchResults {
        hits: Vec::new(),
        total_count: 0,
        total_count_is_lower_bound: false,
        channel_facets: Vec::new(),
        author_facets: Vec::new(),
    }
}

/// The searchable channels `in:` names (by id, or by name where channels have one, ignoring
/// case), or all of them without `in:`. A reference to a channel outside the scope names
/// nothing, exactly like one to a channel that does not exist.
fn channels_named_by(channels: &[SearchableChannel], references: &[String]) -> Vec<Snowflake> {
    if references.is_empty() {
        return channels.iter().map(|channel| channel.id).collect();
    }
    channels
        .iter()
        .filter(|channel| {
            references.iter().any(|reference| {
                reference.parse::<Snowflake>().ok() == Some(channel.id)
                    || channel
                        .name
                        .as_deref()
                        .is_some_and(|name| name.to_lowercase() == reference.to_lowercase())
            })
        })
        .map(|channel| channel.id)
        .collect()
}

fn channel_facets(counts: &[MatchCount]) -> Vec<ChannelSearchFacet> {
    largest_totals(counts.iter().map(|count| (count.channel_id, count.count)))
        .into_iter()
        .map(|(channel_id, count)| ChannelSearchFacet { channel_id, count })
        .collect()
}

/// The `MAXIMUM_SEARCH_FACETS` largest totals per id, largest first, then by id.
fn largest_totals(counts: impl Iterator<Item = (Snowflake, u32)>) -> Vec<(Snowflake, u32)> {
    let mut totals: HashMap<Snowflake, u32> = HashMap::new();
    for (id, count) in counts {
        *totals.entry(id).or_default() += count;
    }
    let mut totals: Vec<(Snowflake, u32)> = totals.into_iter().collect();
    totals.sort_unstable_by_key(|&(id, count)| (std::cmp::Reverse(count), id));
    totals.truncate(MAXIMUM_SEARCH_FACETS);
    totals
}
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mellow_protocol::{
        CreateChannelRequest, CreateGuildRequest, CreateInviteRequest, EditMessageRequest,
        HighlightRange, Message, OpenDirectMessageRequest, OverwriteKind, SendMessageRequest,
        SetPermissionOverwriteRequest,
    };

    use super::*;
    use crate::domain::account_rules::{GuildCreationPolicy, RegistrationMode};
    use crate::domain::accounts::Accounts;
    use crate::domain::accounts::tests::{NOW, migrated_database, register_request};
    use crate::domain::guilds::Guilds;
    use crate::domain::snowflake_generator::SnowflakeGenerator;
    use crate::events::EventHub;
    use crate::storage::Database;

    const DAY_MILLISECONDS: u64 = 86_400_000;
    /// 2050-01-01T00:00:00Z: later than any id the guild fixture issues with the real clock, so
    /// messages sent then get ids of that day.
    const JANUARY_FIRST_2050: u64 = 2_524_608_000_000;

    struct Fixture {
        _directory: tempfile::TempDir,
        database: Database,
        messages: Messages,
        guilds: Arc<Guilds>,
        owner: Snowflake,
        member: Snowflake,
        outsider: Snowflake,
        guild_id: Snowflake,
        general: Snowflake,
    }

    async fn fixture() -> Fixture {
        let (directory, database) = migrated_database().await;
        let ids = Arc::new(SnowflakeGenerator::default());
        let accounts = Accounts::new(
            database.clone(),
            RegistrationMode::Open,
            ids.clone(),
            Arc::default(),
        );
        let mut users = Vec::new();
        for name in ["owner", "member", "outsider"] {
            let session = accounts
                .register(register_request(name, &format!("{name}@example.org")), NOW)
                .await
                .unwrap();
            users.push(session.user.id);
        }
        let guilds = Arc::new(Guilds::new(
            database.clone(),
            ids.clone(),
            Arc::new(EventHub::default()),
            GuildCreationPolicy::Everyone,
        ));
        let guild = guilds
            .create_guild(
                users[0],
                false,
                CreateGuildRequest {
                    name: "Club".into(),
                    initial_categories: None,
                },
                NOW,
            )
            .await
            .unwrap();
        let invite = guilds
            .create_invite(users[0], guild.id, CreateInviteRequest::default(), NOW)
            .await
            .unwrap();
        guilds
            .join_with_invite(users[1], &invite.code, NOW)
            .await
            .unwrap();
        Fixture {
            _directory: directory,
            messages: Messages::new(database.clone(), ids, guilds.clone()),
            database,
            guilds,
            owner: users[0],
            member: users[1],
            outsider: users[2],
            guild_id: guild.id,
            general: guild.channels[1].id,
        }
    }

    impl Fixture {
        async fn send_at(
            &self,
            author: Snowflake,
            channel: Snowflake,
            content: &str,
            at: u64,
        ) -> Message {
            self.messages
                .send(
                    author,
                    channel,
                    SendMessageRequest {
                        content: content.into(),
                        reply_to_id: None,
                        nonce: None,
                        attachment_ids: Vec::new(),
                    },
                    at,
                )
                .await
                .unwrap()
        }

        async fn send(&self, author: Snowflake, channel: Snowflake, content: &str) -> Message {
            self.send_at(author, channel, content, NOW).await
        }

        async fn text_channel(&self, name: &str) -> Snowflake {
            self.guilds
                .create_channel(
                    self.owner,
                    self.guild_id,
                    CreateChannelRequest {
                        kind: mellow_protocol::ChannelKind::Text,
                        name: name.into(),
                        topic: String::new(),
                        parent_id: None,
                    },
                    NOW,
                )
                .await
                .unwrap()
                .id
        }

        async fn deny(
            &self,
            channel: Snowflake,
            kind: OverwriteKind,
            target: Snowflake,
            deny: Permissions,
        ) {
            self.guilds
                .set_permission_overwrite(
                    self.owner,
                    channel,
                    target,
                    SetPermissionOverwriteRequest {
                        kind,
                        allow: Permissions::empty(),
                        deny,
                    },
                )
                .await
                .unwrap();
        }

        async fn search_guild(
            &self,
            user: Snowflake,
            query: MessageSearchQuery,
        ) -> Result<MessageSearchResults, MessageError> {
            self.messages
                .search_guild_messages(user, self.guild_id, &query)
                .await
        }

        /// Ids of the hits of `q` in the guild, as the member, in the default order.
        async fn found(&self, q: &str) -> Vec<Snowflake> {
            hit_ids(&self.search_guild(self.member, query(q)).await.unwrap())
        }
    }

    fn query(q: &str) -> MessageSearchQuery {
        MessageSearchQuery {
            q: q.into(),
            ..MessageSearchQuery::default()
        }
    }

    fn hit_ids(results: &MessageSearchResults) -> Vec<Snowflake> {
        results.hits.iter().map(|hit| hit.message.id).collect()
    }

    #[tokio::test]
    async fn words_ignore_case_and_accents_match_prefixes_and_rank_exact_words_first() {
        let fixture = fixture().await;
        let (owner, general) = (fixture.owner, fixture.general);
        let cafeteria = fixture
            .send(owner, general, "The cafeteria is closed")
            .await;
        let coffee = fixture
            .send(owner, general, "Le CAFÉ est prêt, café!")
            .await;
        let tea = fixture.send(owner, general, "Tea is ready").await;
        let latte = fixture
            .send(owner, general, "café au lait, bien chaud")
            .await;

        let relevance = fixture
            .search_guild(fixture.member, query("cafe"))
            .await
            .unwrap();
        let newest = fixture
            .search_guild(
                fixture.member,
                MessageSearchQuery {
                    sort: Some(MessageSearchSort::Newest),
                    ..query("cafe")
                },
            )
            .await
            .unwrap();
        let phrase = fixture.found("\"au lait\"").await;
        let scattered_words = fixture.found("lait chaud").await;
        let excluded = fixture.found("cafe -lait").await;
        let only_excluded = fixture.found("-cafe").await;
        let explicit_prefix = fixture.found("rea*").await;
        let nothing = fixture.found("!!! 🎉").await;
        fixture.database.close().await;

        // Exact words first (the one with two of them on top), then the prefix match.
        assert_eq!(hit_ids(&relevance), [coffee.id, latte.id, cafeteria.id]);
        assert_eq!(relevance.total_count, 3);
        assert!(!relevance.total_count_is_lower_bound);
        assert_eq!(hit_ids(&newest), [latte.id, coffee.id, cafeteria.id]);
        assert_eq!(
            relevance.hits[0].highlights,
            [
                HighlightRange { start: 3, end: 7 },
                HighlightRange { start: 18, end: 22 }
            ]
        );
        assert_eq!(
            relevance.hits[2].highlights,
            [HighlightRange { start: 4, end: 13 }]
        );
        assert_eq!(phrase, [latte.id]);
        assert_eq!(scattered_words, [latte.id]);
        assert_eq!(excluded, [coffee.id, cafeteria.id]);
        // Excluded words match exactly: "cafeteria" stays.
        assert_eq!(only_excluded, [tea.id, cafeteria.id]);
        assert_eq!(explicit_prefix, [tea.id]);
        assert_eq!(nothing, []);
    }

    #[tokio::test]
    async fn hidden_channels_and_channels_without_history_never_show_even_in_counts() {
        let fixture = fixture().await;
        let (owner, member) = (fixture.owner, fixture.member);
        let secret = fixture.text_channel("secret").await;
        fixture
            .deny(
                secret,
                OverwriteKind::Role,
                fixture.guild_id,
                Permissions::VIEW_CHANNEL,
            )
            .await;
        let archive = fixture.text_channel("archive").await;
        fixture
            .deny(
                archive,
                OverwriteKind::Member,
                member,
                Permissions::READ_MESSAGE_HISTORY,
            )
            .await;
        let visible = fixture
            .send(owner, fixture.general, "pineapple pizza")
            .await;
        fixture.send(owner, secret, "pineapple plans").await;
        fixture.send(owner, archive, "pineapple archive").await;

        let as_member = fixture
            .search_guild(member, query("pineapple"))
            .await
            .unwrap();
        let as_owner = fixture
            .search_guild(owner, query("pineapple"))
            .await
            .unwrap();
        let naming_the_secret = fixture.found("pineapple in:secret").await;
        let naming_it_by_id = fixture.found(&format!("pineapple in:{secret}")).await;
        let outsider = fixture
            .search_guild(fixture.outsider, query("pineapple"))
            .await;
        fixture.database.close().await;

        assert_eq!(hit_ids(&as_member), [visible.id]);
        assert_eq!(as_member.total_count, 1);
        assert_eq!(
            as_member.channel_facets,
            [ChannelSearchFacet {
                channel_id: fixture.general,
                count: 1
            }]
        );
        assert_eq!(as_member.author_facets.len(), 1);
        assert_eq!(as_member.author_facets[0].author.username, "owner");
        assert_eq!(as_owner.total_count, 3);
        assert_eq!(naming_the_secret, []);
        assert_eq!(naming_it_by_id, []);
        assert!(
            matches!(outsider, Err(MessageError::Guild(GuildError::NotFound))),
            "{outsider:?}"
        );
    }

    #[tokio::test]
    async fn conversations_are_searched_by_their_participants_only() {
        let fixture = fixture().await;
        let conversation = fixture
            .messages
            .open_direct_message(
                fixture.owner,
                OpenDirectMessageRequest {
                    recipient_id: fixture.member,
                },
                NOW,
            )
            .await
            .unwrap();
        let private = fixture
            .send(fixture.owner, conversation.id, "mango smoothie")
            .await;
        let public = fixture
            .send(fixture.owner, fixture.general, "mango salad")
            .await;
        let search_conversations = |user: Snowflake, q: String| {
            let messages = &fixture.messages;
            async move {
                messages
                    .search_my_conversation_messages(user, &query(&q))
                    .await
                    .unwrap()
            }
        };

        let as_member = search_conversations(fixture.member, "mango".into()).await;
        let as_outsider = search_conversations(fixture.outsider, "mango".into()).await;
        let by_id =
            search_conversations(fixture.member, format!("mango in:{}", conversation.id)).await;
        let in_guild = fixture.found("mango").await;
        fixture.database.close().await;

        assert_eq!(hit_ids(&as_member), [private.id]);
        assert_eq!(hit_ids(&by_id), [private.id]);
        assert_eq!(as_outsider.total_count, 0);
        assert_eq!(hit_ids(&as_outsider), []);
        assert_eq!(in_guild, [public.id]);
    }

    #[tokio::test]
    async fn filters_narrow_by_author_mention_channel_contents_and_day() {
        let fixture = fixture().await;
        let (owner, member, general) = (fixture.owner, fixture.member, fixture.general);
        let random = fixture.text_channel("random").await;
        let by_owner = fixture
            .send_at(owner, general, "report draft", JANUARY_FIRST_2050)
            .await;
        let mentioning = fixture
            .send_at(member, general, "report for @owner", JANUARY_FIRST_2050)
            .await;
        let linked = fixture
            .send_at(
                owner,
                random,
                "report at https://example.org",
                JANUARY_FIRST_2050 + 2 * DAY_MILLISECONDS,
            )
            .await;
        fixture
            .messages
            .pin(owner, general, by_owner.id, NOW)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO attachments (id, channel_id, uploader_id, message_id, file_name, \
             content_type, size_bytes, created_at_unix_milliseconds) \
             VALUES (1, ?1, ?2, ?3, 'a.png', 'image/png', 1, 0), \
             (2, ?1, ?2, ?4, 'b.bin', 'application/octet-stream', 1, 0)",
        )
        .bind(i64::try_from(general.raw()).unwrap())
        .bind(i64::try_from(owner.raw()).unwrap())
        .bind(i64::try_from(by_owner.id.raw()).unwrap())
        .bind(i64::try_from(mentioning.id.raw()).unwrap())
        .execute(fixture.database.writer())
        .await
        .unwrap();

        let ids = |messages: &[&Message]| -> Vec<Snowflake> {
            messages.iter().map(|message| message.id).collect()
        };
        // Newest first: filters alone have no relevance.
        let cases = [
            ("from:owner", ids(&[&linked, &by_owner])),
            ("report FROM:@Member", ids(&[&mentioning])),
            ("from:nobody", Vec::new()),
            ("mentions:owner", ids(&[&mentioning])),
            ("in:#random", ids(&[&linked])),
            (
                "in:general in:random from:owner",
                ids(&[&linked, &by_owner]),
            ),
            ("has:pinned", ids(&[&by_owner])),
            ("has:file", ids(&[&mentioning, &by_owner])),
            ("has:image", ids(&[&by_owner])),
            ("has:link", ids(&[&linked])),
            ("during:2050-01-01", ids(&[&mentioning, &by_owner])),
            ("after:2050-01-01", ids(&[&linked])),
            ("before:2050-01-03", ids(&[&mentioning, &by_owner])),
        ];
        let mut found = Vec::new();
        for (q, _) in &cases {
            found.push(fixture.found(q).await);
        }
        fixture.database.close().await;

        for ((q, expected), found) in cases.iter().zip(found) {
            assert_eq!(&found, expected, "{q}");
        }
    }

    #[tokio::test]
    async fn pages_follow_the_order_and_the_total_counts_every_match() {
        let fixture = fixture().await;
        let mut sent = Vec::new();
        for index in 0..7 {
            let message = fixture
                .send_at(
                    fixture.owner,
                    fixture.general,
                    &format!("note {index}"),
                    NOW + index,
                )
                .await;
            sent.push(message.id);
        }
        let page = |offset| MessageSearchQuery {
            sort: Some(MessageSearchSort::Newest),
            offset: Some(offset),
            limit: Some(3),
            ..query("note")
        };
        let first = fixture.search_guild(fixture.member, page(0)).await.unwrap();
        let last = fixture.search_guild(fixture.member, page(6)).await.unwrap();
        let too_far = fixture
            .search_guild(
                fixture.member,
                MessageSearchQuery {
                    offset: Some(1_000),
                    ..query("note")
                },
            )
            .await;
        fixture.database.close().await;

        assert_eq!(hit_ids(&first), [sent[6], sent[5], sent[4]]);
        assert_eq!(hit_ids(&last), [sent[0]]);
        assert_eq!((first.total_count, last.total_count), (7, 7));
        assert!(
            matches!(
                too_far,
                Err(MessageError::InvalidInput(
                    MessageRuleViolation::SearchQuery(_)
                ))
            ),
            "{too_far:?}"
        );
    }

    #[tokio::test]
    async fn edits_and_deletions_reach_the_index() {
        let fixture = fixture().await;
        let (owner, general) = (fixture.owner, fixture.general);
        let message = fixture.send(owner, general, "orange juice").await;
        let doomed = fixture.send(owner, general, "orange peel").await;
        let random = fixture.text_channel("random").await;
        fixture.send(owner, random, "orange crate").await;
        fixture
            .messages
            .edit(
                owner,
                general,
                message.id,
                EditMessageRequest {
                    content: "lemon juice".into(),
                },
                NOW,
            )
            .await
            .unwrap();
        fixture
            .messages
            .delete(owner, general, doomed.id)
            .await
            .unwrap();
        fixture.guilds.delete_channel(owner, random).await.unwrap();

        let old_word = fixture.found("orange").await;
        let new_word = fixture.found("lemon").await;
        let index_integrity = sqlx::query(
            "INSERT INTO messages_fts (messages_fts, rank) VALUES ('integrity-check', 1)",
        )
        .execute(fixture.database.writer())
        .await;
        fixture.database.close().await;

        assert_eq!(old_word, []);
        assert_eq!(new_word, [message.id]);
        assert!(index_integrity.is_ok(), "{index_integrity:?}");
    }

    #[tokio::test]
    async fn a_user_runs_one_search_at_a_time() {
        let fixture = fixture().await;
        let running = fixture.messages.start_search_of(fixture.member).unwrap();
        let while_running = fixture.search_guild(fixture.member, query("word")).await;
        let someone_else = fixture.search_guild(fixture.owner, query("word")).await;
        drop(running);
        let afterwards = fixture.search_guild(fixture.member, query("word")).await;
        fixture.database.close().await;

        assert!(
            matches!(while_running, Err(MessageError::SearchAlreadyRunning)),
            "{while_running:?}"
        );
        assert!(someone_else.is_ok(), "{someone_else:?}");
        assert!(afterwards.is_ok(), "{afterwards:?}");
    }

    #[tokio::test]
    async fn counts_and_facets_never_pass_the_counted_matches() {
        let fixture = fixture().await;
        for index in 0..3 {
            fixture
                .send_at(fixture.owner, fixture.general, "same words", NOW + index)
                .await;
        }
        let criteria = SearchCriteria {
            channel_ids: vec![fixture.general],
            match_expression: Some("\"same\"".into()),
            ..SearchCriteria::default()
        };
        let two_counted = search_storage::count_matches(&fixture.database, &criteria, 2)
            .await
            .unwrap();
        let all_counted = search_storage::count_matches(&fixture.database, &criteria, 3)
            .await
            .unwrap();
        fixture.database.close().await;

        assert_eq!(channel_facets(&two_counted.counts)[0].count, 2);
        assert!(two_counted.has_more);
        assert_eq!(channel_facets(&all_counted.counts)[0].count, 3);
        assert!(!all_counted.has_more);
    }

    #[tokio::test]
    async fn deleting_a_guild_takes_its_messages_out_of_the_index() {
        let fixture = fixture().await;
        fixture.send(fixture.owner, fixture.general, "walnut").await;
        fixture
            .guilds
            .delete_guild(fixture.owner, fixture.guild_id)
            .await
            .unwrap();
        let remaining: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'walnut'",
        )
        .fetch_one(fixture.database.readers())
        .await
        .unwrap();
        let integrity = sqlx::query(
            "INSERT INTO messages_fts (messages_fts, rank) VALUES ('integrity-check', 1)",
        )
        .execute(fixture.database.writer())
        .await;
        fixture.database.close().await;

        assert_eq!(remaining, 0);
        assert!(integrity.is_ok(), "{integrity:?}");
    }

    #[tokio::test]
    async fn oversized_queries_are_refused_before_any_read() {
        let fixture = fixture().await;
        let refused = fixture
            .search_guild(fixture.outsider, query(&"word ".repeat(20)))
            .await;
        fixture.database.close().await;

        // Refused as invalid, before anything tells the outsider whether the guild exists.
        assert!(
            matches!(
                refused,
                Err(MessageError::InvalidInput(
                    MessageRuleViolation::SearchQuery(_)
                ))
            ),
            "{refused:?}"
        );
    }

    #[test]
    fn facets_keep_the_largest_totals_by_id() {
        let id = Snowflake::from_raw;
        let counts = [
            MatchCount {
                channel_id: id(1),
                author_id: id(10),
                count: 2,
            },
            MatchCount {
                channel_id: id(2),
                author_id: id(10),
                count: 3,
            },
            MatchCount {
                channel_id: id(1),
                author_id: id(11),
                count: 3,
            },
        ];
        assert_eq!(
            channel_facets(&counts),
            [
                ChannelSearchFacet {
                    channel_id: id(1),
                    count: 5
                },
                ChannelSearchFacet {
                    channel_id: id(2),
                    count: 3
                },
            ]
        );
        let many = (0..20u64).map(|index| (id(index), 1));
        assert_eq!(largest_totals(many).len(), MAXIMUM_SEARCH_FACETS);
    }
}
