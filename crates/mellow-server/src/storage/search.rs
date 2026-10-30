//! Message search over the FTS5 index of migration 0021 (ADR-0029). Reads only, on the reader
//! pool. Every user value is a bound parameter; the `MATCH` expressions come from
//! `domain::search_query`, which quotes every word.

use mellow_protocol::Snowflake;

use super::Database;
use crate::storage::messages::{MessageStorageError, json_array_of_ids, lookup_integer, to_id};

/// What a message must hold (`has:` filters); each is independent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct RequiredContents {
    pub(crate) has_file: bool,
    pub(crate) has_image: bool,
    pub(crate) has_link: bool,
    pub(crate) is_pinned: bool,
}

/// What to search for, in which channels. Empty lists and `false` flags filter nothing, except
/// `channel_ids`: no channel, no result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SearchCriteria {
    pub(crate) channel_ids: Vec<Snowflake>,
    /// FTS5 expression the content must match; `None` for a search by filters only.
    pub(crate) match_expression: Option<String>,
    /// FTS5 expression the content must not match.
    pub(crate) exclusion_expression: Option<String>,
    /// Any of them wrote it.
    pub(crate) author_usernames: Vec<String>,
    /// It mentions any of them.
    pub(crate) mentioned_usernames: Vec<String>,
    pub(crate) required_contents: RequiredContents,
    /// Inclusive.
    pub(crate) lowest_id: Option<Snowflake>,
    /// Exclusive.
    pub(crate) id_below: Option<Snowflake>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchOrder {
    /// bm25, then newest first. Same as `Newest` without a match expression.
    Relevance,
    Newest,
}

/// How many counted matches one author wrote in one channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MatchCount {
    pub(crate) channel_id: Snowflake,
    pub(crate) author_id: Snowflake,
    pub(crate) count: u32,
}

/// The conditions shared by every search query, on parameters ?2 to ?10 (`bind_criteria!`).
macro_rules! search_conditions {
    () => {
        " AND messages.channel_id IN (SELECT value FROM json_each(?2)) \
         AND (?3 IS NULL OR messages.author_id IN (SELECT users.id FROM users \
             WHERE users.username IN (SELECT value FROM json_each(?3)))) \
         AND (?4 IS NULL OR EXISTS (SELECT 1 FROM message_mentions \
             JOIN users ON users.id = message_mentions.user_id \
             WHERE message_mentions.message_id = messages.id \
             AND users.username IN (SELECT value FROM json_each(?4)))) \
         AND (?5 = 0 OR EXISTS (SELECT 1 FROM attachments \
             WHERE attachments.message_id = messages.id)) \
         AND (?6 = 0 OR EXISTS (SELECT 1 FROM attachments \
             WHERE attachments.message_id = messages.id \
             AND attachments.content_type LIKE 'image/%')) \
         AND (?7 = 0 OR messages.content LIKE '%http://%' OR messages.content LIKE '%https://%') \
         AND (?8 = 0 OR messages.pinned_at_unix_milliseconds IS NOT NULL) \
         AND messages.id >= ?9 AND messages.id < ?10 "
    };
}

/// Messages matching ?1 with the shared conditions; ?11 is bound but unused, since the match
/// expression already subtracts excluded words.
macro_rules! matching_messages {
    () => {
        concat!(
            "FROM messages_fts JOIN messages ON messages.id = messages_fts.rowid \
             WHERE messages_fts MATCH ?1",
            search_conditions!()
        )
    };
}

/// Messages with the shared conditions, minus those matching ?11; ?1 is bound but unused.
/// `by_author` states the author condition plainly, so SQLite walks `messages_by_author_id`:
/// inside `?3 IS NULL OR …` it cannot use an index.
macro_rules! filtered_messages {
    () => {
        filtered_messages!("1")
    };
    (by_author) => {
        filtered_messages!(
            "messages.author_id IN (SELECT users.id FROM users \
             WHERE users.username IN (SELECT value FROM json_each(?3)))"
        )
    };
    ($first_condition:literal) => {
        concat!(
            "FROM messages WHERE ",
            $first_condition,
            search_conditions!(),
            // Correlated, so it runs only with excluded words: an uncorrelated `NOT IN` would be
            // materialized even for a NULL expression, scanning the whole index.
            "AND (?11 IS NULL OR NOT EXISTS (SELECT 1 FROM messages_fts \
             WHERE messages_fts MATCH ?11 AND messages_fts.rowid = messages.id)) "
        )
    };
}

/// Binds ?1 to ?11 of `search_conditions!` and `matching_messages!`, in order.
macro_rules! bind_criteria {
    ($query:expr, $bound:expr) => {
        $query
            .bind($bound.match_expression)
            .bind($bound.channel_ids)
            .bind($bound.author_usernames)
            .bind($bound.mentioned_usernames)
            .bind($bound.required_contents.has_file)
            .bind($bound.required_contents.has_image)
            .bind($bound.required_contents.has_link)
            .bind($bound.required_contents.is_pinned)
            .bind($bound.lowest_id)
            .bind($bound.id_below)
            .bind($bound.exclusion_expression)
    };
}

/// The criteria as SQLite values.
struct BoundCriteria {
    match_expression: Option<String>,
    channel_ids: String,
    author_usernames: Option<String>,
    mentioned_usernames: Option<String>,
    required_contents: RequiredContents,
    lowest_id: i64,
    id_below: i64,
    exclusion_expression: Option<String>,
}

impl BoundCriteria {
    fn new(criteria: &SearchCriteria) -> Self {
        let json_strings = |values: &[String]| {
            (!values.is_empty()).then(|| serde_json::Value::from(values).to_string())
        };
        Self {
            match_expression: criteria.match_expression.clone(),
            channel_ids: json_array_of_ids(&criteria.channel_ids),
            author_usernames: json_strings(&criteria.author_usernames),
            mentioned_usernames: json_strings(&criteria.mentioned_usernames),
            required_contents: criteria.required_contents,
            // An id beyond SQLite's range is past every stored one.
            lowest_id: criteria
                .lowest_id
                .map_or(0, |id| lookup_integer(id).unwrap_or(i64::MAX)),
            id_below: criteria
                .id_below
                .map_or(i64::MAX, |id| lookup_integer(id).unwrap_or(i64::MAX)),
            exclusion_expression: criteria.exclusion_expression.clone(),
        }
    }
}

/// The ids of one page of matches, in `order`.
///
/// # Errors
/// Fails on database errors, an FTS5 syntax error included (never expected from the parser).
pub(crate) async fn search_message_ids(
    database: &Database,
    criteria: &SearchCriteria,
    order: SearchOrder,
    offset: u32,
    limit: u32,
) -> Result<Vec<Snowflake>, MessageStorageError> {
    let bound = BoundCriteria::new(criteria);
    let ranks_by_relevance = order == SearchOrder::Relevance;
    let rows: Vec<(i64,)> = if bound.match_expression.is_some() {
        bind_criteria!(
            sqlx::query_as(concat!(
                "SELECT messages.id ",
                matching_messages!(),
                "ORDER BY CASE WHEN ?12 THEN messages_fts.rank END, messages.id DESC \
                 LIMIT ?13 OFFSET ?14"
            )),
            bound
        )
        .bind(ranks_by_relevance)
        .bind(i64::from(limit))
        .bind(i64::from(offset))
        .fetch_all(database.readers())
        .await?
    } else if bound.author_usernames.is_some() {
        bind_criteria!(
            sqlx::query_as(concat!(
                "SELECT messages.id ",
                filtered_messages!(by_author),
                "ORDER BY messages.id DESC LIMIT ?13 OFFSET ?14"
            )),
            bound
        )
        .bind(ranks_by_relevance)
        .bind(i64::from(limit))
        .bind(i64::from(offset))
        .fetch_all(database.readers())
        .await?
    } else {
        bind_criteria!(
            sqlx::query_as(concat!(
                "SELECT messages.id ",
                filtered_messages!(),
                "ORDER BY messages.id DESC LIMIT ?13 OFFSET ?14"
            )),
            bound
        )
        .bind(ranks_by_relevance)
        .bind(i64::from(limit))
        .bind(i64::from(offset))
        .fetch_all(database.readers())
        .await?
    };
    rows.into_iter().map(|(id,)| to_id(id)).collect()
}

/// The newest `maximum_counted` matches, counted per channel and author.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CountedMatches {
    pub(crate) counts: Vec<MatchCount>,
    /// More matches exist than were counted.
    pub(crate) has_more: bool,
}

/// Counts the `maximum_counted` newest matches per channel and author, and tells whether more
/// exist; facets built from it never count more than the total says.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn count_matches(
    database: &Database,
    criteria: &SearchCriteria,
    maximum_counted: u32,
) -> Result<CountedMatches, MessageStorageError> {
    let bound = BoundCriteria::new(criteria);
    // One row more than counted, to tell "exactly the cap" from "more than the cap".
    let read_limit = i64::from(maximum_counted) + 1;
    let rows: Vec<(i64, i64)> = if bound.match_expression.is_some() {
        bind_criteria!(
            sqlx::query_as(concat!(
                "SELECT messages.channel_id, messages.author_id ",
                matching_messages!(),
                "ORDER BY messages_fts.rowid DESC LIMIT ?12"
            )),
            bound
        )
        .bind(read_limit)
        .fetch_all(database.readers())
        .await?
    } else if bound.author_usernames.is_some() {
        bind_criteria!(
            sqlx::query_as(concat!(
                "SELECT messages.channel_id, messages.author_id ",
                filtered_messages!(by_author),
                "ORDER BY messages.id DESC LIMIT ?12"
            )),
            bound
        )
        .bind(read_limit)
        .fetch_all(database.readers())
        .await?
    } else {
        bind_criteria!(
            sqlx::query_as(concat!(
                "SELECT messages.channel_id, messages.author_id ",
                filtered_messages!(),
                "ORDER BY messages.id DESC LIMIT ?12"
            )),
            bound
        )
        .bind(read_limit)
        .fetch_all(database.readers())
        .await?
    };
    let counted_rows = usize::try_from(maximum_counted).unwrap_or(usize::MAX);
    let has_more = rows.len() > counted_rows;
    let mut counts: Vec<MatchCount> = Vec::new();
    for (channel_id, author_id) in rows.into_iter().take(counted_rows) {
        let (channel_id, author_id) = (to_id(channel_id)?, to_id(author_id)?);
        match counts
            .iter_mut()
            .find(|count| count.channel_id == channel_id && count.author_id == author_id)
        {
            Some(count) => count.count += 1,
            None => counts.push(MatchCount {
                channel_id,
                author_id,
                count: 1,
            }),
        }
    }
    Ok(CountedMatches { counts, has_more })
}

/// The content of each of `message_ids` with `start_mark` and `end_mark` around every word
/// `match_expression` matches (FTS5 `highlight`).
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn highlighted_contents(
    database: &Database,
    match_expression: &str,
    message_ids: &[Snowflake],
    start_mark: char,
    end_mark: char,
) -> Result<Vec<(Snowflake, String)>, MessageStorageError> {
    // Only the page's rows: highlighting re-reads and re-tokenizes each content.
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT rowid, highlight(messages_fts, 0, ?3, ?4) FROM messages_fts \
         WHERE messages_fts MATCH ?1 AND rowid IN (SELECT value FROM json_each(?2))",
    )
    .bind(match_expression)
    .bind(json_array_of_ids(message_ids))
    .bind(start_mark.to_string())
    .bind(end_mark.to_string())
    .fetch_all(database.readers())
    .await?;
    rows.into_iter()
        .map(|(id, marked)| Ok((to_id(id)?, marked)))
        .collect()
}
/// Message histories that look like a real instance's, for migration tests and measurements.
#[cfg(test)]
pub(crate) mod realistic_messages {
    /// Common words first: picks are skewed towards the start, as word use is (Zipf-like).
    const VOCABULARY: &[&str] = &[
        "the",
        "de",
        "a",
        "le",
        "is",
        "la",
        "to",
        "et",
        "you",
        "je",
        "it",
        "pas",
        "that",
        "on",
        "est",
        "and",
        "les",
        "ok",
        "c'est",
        "lol",
        "in",
        "for",
        "pour",
        "this",
        "une",
        "what",
        "mais",
        "with",
        "avec",
        "just",
        "yes",
        "oui",
        "non",
        "game",
        "jeu",
        "ce",
        "soir",
        "tonight",
        "demain",
        "tomorrow",
        "server",
        "serveur",
        "message",
        "merci",
        "thanks",
        "café",
        "coffee",
        "réunion",
        "meeting",
        "build",
        "release",
        "bug",
        "fix",
        "deploy",
        "mellow",
        "voice",
        "écran",
        "screen",
        "photo",
        "image",
        "lien",
        "https://example.org/page",
        "@ada",
        "@bob",
        "<:party:501>",
        "😀",
        "été",
        "hiver",
        "garçon",
        "naïve",
        "crème",
        "brûlée",
        "pizza",
        "ananas",
        "pineapple",
        "documentation",
        "migration",
        "sqlite",
        "encryption",
        "chiffrement",
        "rendez-vous",
        "aujourd'hui",
        "weekend",
        "vacances",
        "holiday",
        "sprint",
        "roadmap",
        "design",
        "maquette",
        "mockup",
        "android",
        "desktop",
        "web",
        "navigateur",
        "browser",
        "notification",
        "recherche",
        "search",
        "elasticsearch",
        "index",
        "requête",
        "query",
        "performance",
        "latence",
        "latency",
        "kubernetes",
        "docker",
        "binary",
        "binaire",
        "backup",
        "sauvegarde",
        "restauration",
        "zeppelin",
        "quasar",
        "xylophone",
        "fjord",
    ];

    /// A deterministic generator (64-bit LCG), so every run builds the same history.
    #[derive(Debug)]
    pub(crate) struct WordPicker(u64);

    impl WordPicker {
        pub(crate) const fn new(seed: u64) -> Self {
            Self(seed)
        }

        #[allow(clippy::cast_precision_loss)]
        fn next_fraction(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 11) as f64 / (1_u64 << 53) as f64
        }

        /// A number in `0..bound`.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        pub(crate) fn below(&mut self, bound: usize) -> usize {
            ((self.next_fraction() * bound as f64) as usize).min(bound - 1)
        }

        /// A chat message of 3 to 24 words, common words more often.
        pub(crate) fn message(&mut self) -> String {
            let word_count = 3 + self.below(22);
            (0..word_count)
                .map(|_| {
                    let skewed = self.next_fraction().powi(3);
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        clippy::cast_precision_loss
                    )]
                    let index = (skewed * VOCABULARY.len() as f64) as usize;
                    VOCABULARY[index.min(VOCABULARY.len() - 1)]
                })
                .collect::<Vec<_>>()
                .join(" ")
        }
    }

    /// Inserts `count` messages with consecutive ids from `first_id`, spread over `channel_ids`
    /// and `author_ids`, in one transaction.
    pub(crate) async fn insert_messages(
        writer: &sqlx::SqlitePool,
        first_id: i64,
        count: usize,
        channel_ids: &[i64],
        author_ids: &[i64],
    ) {
        let mut picker = WordPicker::new(7);
        let mut transaction = writer.begin().await.unwrap();
        for offset in 0..count {
            let id = first_id + i64::try_from(offset).unwrap();
            sqlx::query(
                "INSERT INTO messages (id, channel_id, author_id, content) VALUES (?, ?, ?, ?)",
            )
            .bind(id)
            .bind(channel_ids[picker.below(channel_ids.len())])
            .bind(author_ids[picker.below(author_ids.len())])
            .bind(picker.message())
            .execute(&mut *transaction)
            .await
            .unwrap();
        }
        transaction.commit().await.unwrap();
    }
}
#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::domain::search_query::{
        HIGHLIGHT_END_MARK, HIGHLIGHT_START_MARK, parse_search_query,
    };
    use crate::storage::messages::find_messages_by_ids;
    use crate::storage::{DatabaseKey, migrate_with_backup, open_encrypted_database};

    const GUILD_COUNT: i64 = 5;
    const CHANNELS_PER_GUILD: i64 = 20;
    const USER_COUNT: i64 = 50;
    const RUNS_PER_QUERY: usize = 30;
    /// Inserted after the backfill, to time inserts with the index.
    const EXTRA_MESSAGES: usize = 10_000;
    const MEASURED_QUERIES: [&str; 11] = [
        "the",
        "cafe",
        "xylophone",
        "release deploy",
        "\"rendez vous\"",
        "mell*",
        "pizza -ananas",
        "zeppelin from:user3",
        "from:user3",
        "has:link",
        "the in:channel-0-3",
    ];

    fn milliseconds(duration: Duration) -> f64 {
        duration.as_secs_f64() * 1_000.0
    }

    fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
        sorted[index]
    }

    async fn index_bytes(database: &Database, table_pattern: &str) -> i64 {
        sqlx::query_scalar("SELECT COALESCE(SUM(pgsize), 0) FROM dbstat WHERE name LIKE ?")
            .bind(table_pattern)
            .fetch_one(database.writer())
            .await
            .unwrap()
    }

    /// One search as the domain runs it: page ids, counts, the page's messages and highlights.
    async fn search_once(database: &Database, criteria: &SearchCriteria, order: SearchOrder) {
        let ids = search_message_ids(database, criteria, order, 0, 25)
            .await
            .unwrap();
        count_matches(
            database,
            criteria,
            mellow_protocol::MAXIMUM_COUNTED_SEARCH_MATCHES,
        )
        .await
        .unwrap();
        find_messages_by_ids(database, &ids).await.unwrap();
        if let Some(expression) = &criteria.match_expression {
            highlighted_contents(
                database,
                expression,
                &ids,
                HIGHLIGHT_START_MARK,
                HIGHLIGHT_END_MARK,
            )
            .await
            .unwrap();
        }
    }

    /// Measures backfill time, index size, write overhead and query latency on a realistic
    /// history. `MELLOW_SEARCH_MEASURE_MESSAGES` sets its size (default 100 000). Run with
    /// `cargo test --release -p mellow-server --lib measure_search -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "measurement, minutes long; run by hand"]
    #[allow(clippy::print_stdout, clippy::too_many_lines)]
    async fn measure_search_backfill_and_latency() {
        let message_count: usize = std::env::var("MELLOW_SEARCH_MEASURE_MESSAGES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(100_000);
        let directory = tempfile::tempdir().unwrap();
        let database = open_encrypted_database(
            &directory.path().join("mellow.db"),
            &DatabaseKey::from_bytes([5; 32]),
        )
        .await
        .unwrap();
        migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        let writer = database.writer();
        // Back to the schema before migration 0021, to time its backfill alone.
        sqlx::raw_sql(
            "DROP TRIGGER messages_fts_after_insert; DROP TRIGGER messages_fts_after_delete; \
             DROP TRIGGER messages_fts_after_content_update; DROP TABLE messages_fts; \
             DROP INDEX messages_by_author_id;",
        )
        .execute(writer)
        .await
        .unwrap();
        for user in 1..=USER_COUNT {
            sqlx::query(
                "INSERT INTO users (id, username, display_name, email, password_hash, \
                 created_at_unix_milliseconds) VALUES (?1, 'user' || ?1, 'User', \
                 'user' || ?1 || '@example.org', 'x', 0)",
            )
            .bind(user)
            .execute(writer)
            .await
            .unwrap();
        }
        let mut channel_ids = Vec::new();
        for guild in 0..GUILD_COUNT {
            let guild_id = 1_000 + guild;
            sqlx::query(
                "INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
                 VALUES (?, 'Guild', 1, 0)",
            )
            .bind(guild_id)
            .execute(writer)
            .await
            .unwrap();
            for channel in 0..CHANNELS_PER_GUILD {
                let channel_id = 10_000 + guild * 100 + channel;
                sqlx::query(
                    "INSERT INTO channels (id, guild_id, kind, name, position) \
                     VALUES (?, ?, 'text', ?, ?)",
                )
                .bind(channel_id)
                .bind(guild_id)
                .bind(format!("channel-{guild}-{channel}"))
                .bind(channel)
                .execute(writer)
                .await
                .unwrap();
                channel_ids.push(channel_id);
            }
        }
        let author_ids: Vec<i64> = (1..=USER_COUNT).collect();
        let first_message_id = 1 << 40;
        let started = Instant::now();
        realistic_messages::insert_messages(
            writer,
            first_message_id,
            message_count,
            &channel_ids,
            &author_ids,
        )
        .await;
        let insert_without_index = started.elapsed();

        let started = Instant::now();
        sqlx::raw_sql(include_str!(
            "../../migrations/0021_create_message_search_index.sql"
        ))
        .execute(writer)
        .await
        .unwrap();
        let backfill = started.elapsed();

        let started = Instant::now();
        realistic_messages::insert_messages(
            writer,
            first_message_id + i64::try_from(message_count).unwrap(),
            EXTRA_MESSAGES,
            &channel_ids,
            &author_ids,
        )
        .await;
        let insert_with_index = started.elapsed();
        let messages_bytes = index_bytes(&database, "messages").await;
        let search_index_bytes = index_bytes(&database, "messages_fts%").await;

        #[allow(clippy::cast_precision_loss)]
        let per_message =
            |duration: Duration, count: usize| milliseconds(duration) * 1_000.0 / count as f64;
        println!("messages: {message_count}");
        println!(
            "insert without index: {:.1} µs/message; with index: {:.1} µs/message",
            per_message(insert_without_index, message_count),
            per_message(insert_with_index, EXTRA_MESSAGES)
        );
        println!(
            "backfill (migration 0021): {:.0} ms",
            milliseconds(backfill)
        );
        println!(
            "messages table: {:.1} MiB; search index: {:.1} MiB",
            f64::from(u32::try_from(messages_bytes / 1024).unwrap()) / 1024.0,
            f64::from(u32::try_from(search_index_bytes / 1024).unwrap()) / 1024.0
        );

        // One guild's channels: a fifth of the messages, as a member of that guild searches.
        let guild_channels: Vec<Snowflake> = channel_ids
            .iter()
            .take(usize::try_from(CHANNELS_PER_GUILD).unwrap())
            .map(|id| Snowflake::from_raw(u64::try_from(*id).unwrap()))
            .collect();
        let mut all = Vec::new();
        for order in [SearchOrder::Relevance, SearchOrder::Newest] {
            for text in MEASURED_QUERIES {
                let parsed = parse_search_query(text).unwrap();
                let named_channel = parsed.channel_references.first().map(|name| {
                    let index: i64 = name.rsplit('-').next().unwrap().parse().unwrap();
                    Snowflake::from_raw(u64::try_from(10_000 + index).unwrap())
                });
                let criteria = SearchCriteria {
                    channel_ids: named_channel
                        .map_or_else(|| guild_channels.clone(), |id| vec![id]),
                    match_expression: parsed.match_expression(),
                    exclusion_expression: parsed.standalone_exclusion_expression(),
                    author_usernames: parsed.author_usernames.clone(),
                    mentioned_usernames: parsed.mentioned_usernames.clone(),
                    required_contents: parsed.required_contents,
                    ..SearchCriteria::default()
                };
                let matches = count_matches(
                    &database,
                    &criteria,
                    mellow_protocol::MAXIMUM_COUNTED_SEARCH_MATCHES,
                )
                .await
                .unwrap()
                .counts
                .iter()
                .map(|count| count.count)
                .sum::<u32>();
                let mut durations = Vec::new();
                for _ in 0..RUNS_PER_QUERY {
                    let started = Instant::now();
                    search_once(&database, &criteria, order).await;
                    durations.push(started.elapsed());
                }
                durations.sort_unstable();
                println!(
                    "{order:?} {text:?} ({matches} counted): p50 {:.1} ms, p95 {:.1} ms",
                    milliseconds(percentile(&durations, 0.5)),
                    milliseconds(percentile(&durations, 0.95))
                );
                all.extend(durations);
            }
        }
        all.sort_unstable();
        println!(
            "all queries: p50 {:.1} ms, p95 {:.1} ms",
            milliseconds(percentile(&all, 0.5)),
            milliseconds(percentile(&all, 0.95))
        );
        database.close().await;
    }

    fn channel_deletion_audit(channel_id: Snowflake) -> crate::storage::audit_log::NewAuditEntry {
        crate::storage::audit_log::NewAuditEntry {
            id: Snowflake::from_raw(1 << 50),
            guild_id: Snowflake::from_raw(1),
            actor_id: Snowflake::from_raw(1),
            action: mellow_protocol::AuditAction::ChannelDelete,
            target_id: Some(channel_id),
            context_id: None,
            channel_id: None,
            reason: String::new(),
            changes: Vec::new(),
        }
    }

    /// Measures how long the writer is held when deleting a channel holding a fifth of the
    /// messages and a guild holding a tenth, with the search triggers in place. Run with
    /// `cargo test --release -p mellow-server --lib measure_deletion -- --ignored --nocapture`;
    /// `MELLOW_SEARCH_MEASURE_MESSAGES` sets the total (default 1 000 000).
    #[tokio::test]
    #[ignore = "measurement, minutes long; run by hand"]
    #[allow(clippy::print_stdout)]
    async fn measure_deletion_of_large_channels_and_guilds() {
        let message_count: usize = std::env::var("MELLOW_SEARCH_MEASURE_MESSAGES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1_000_000);
        let directory = tempfile::tempdir().unwrap();
        let database = open_encrypted_database(
            &directory.path().join("mellow.db"),
            &DatabaseKey::from_bytes([6; 32]),
        )
        .await
        .unwrap();
        migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        let writer = database.writer();
        sqlx::raw_sql(
            "DROP TRIGGER messages_fts_after_insert; DROP TRIGGER messages_fts_after_delete; \
             DROP TRIGGER messages_fts_after_content_update; DROP TABLE messages_fts; \
             DROP INDEX messages_by_author_id; \
             INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (1, 'Big', 1, 0), (2, 'Other', 1, 0), (3, 'Rest', 1, 0); \
             INSERT INTO channels (id, guild_id, kind, name, position) VALUES \
             (10, 1, 'text', 'big', 0), (20, 2, 'text', 'a', 0), (21, 2, 'text', 'b', 1), \
             (30, 3, 'text', 'c', 0), (31, 3, 'text', 'd', 1);",
        )
        .execute(writer)
        .await
        .unwrap();
        let in_big_channel = message_count / 5;
        let in_other_guild = message_count / 10;
        let mut first_id = 1 << 40;
        for (count, channels) in [
            (in_big_channel, &[10][..]),
            (in_other_guild, &[20, 21][..]),
            (
                message_count - in_big_channel - in_other_guild,
                &[30, 31][..],
            ),
        ] {
            realistic_messages::insert_messages(writer, first_id, count, channels, &[1]).await;
            first_id += i64::try_from(count).unwrap();
        }
        sqlx::raw_sql(include_str!(
            "../../migrations/0021_create_message_search_index.sql"
        ))
        .execute(writer)
        .await
        .unwrap();

        // A small write every 50 ms meanwhile: its longest wait is how long others queue.
        let is_deleting = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let probe = tokio::spawn({
            let writer = writer.clone();
            let is_deleting = is_deleting.clone();
            async move {
                let mut longest_wait = Duration::ZERO;
                while is_deleting.load(std::sync::atomic::Ordering::Relaxed) {
                    let started = Instant::now();
                    sqlx::query("UPDATE users SET display_name = 'Ada' WHERE id = 1")
                        .execute(&writer)
                        .await
                        .unwrap();
                    longest_wait = longest_wait.max(started.elapsed());
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                longest_wait
            }
        });
        let audit = channel_deletion_audit(Snowflake::from_raw(10));
        let started = Instant::now();
        crate::storage::guilds::delete_channel(&database, Snowflake::from_raw(10), &audit)
            .await
            .unwrap();
        let channel_deletion = started.elapsed();
        let started = Instant::now();
        crate::storage::guilds::delete_guild(
            &database,
            Snowflake::from_raw(2),
            Snowflake::from_raw(1),
        )
        .await
        .unwrap();
        let guild_deletion = started.elapsed();
        is_deleting.store(false, std::sync::atomic::Ordering::Relaxed);
        let longest_wait = probe.await.unwrap();
        let integrity = sqlx::query(
            "INSERT INTO messages_fts (messages_fts, rank) VALUES ('integrity-check', 1)",
        )
        .execute(writer)
        .await;
        println!(
            "in batches: channel of {in_big_channel} messages {:.0} ms; guild of {in_other_guild} \
             messages {:.0} ms; longest wait of another write {:.0} ms",
            milliseconds(channel_deletion),
            milliseconds(guild_deletion),
            milliseconds(longest_wait)
        );
        database.close().await;
        assert!(integrity.is_ok(), "{integrity:?}");
    }
}
