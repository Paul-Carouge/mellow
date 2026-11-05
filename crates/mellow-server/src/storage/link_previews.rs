//! Link previews and the messages showing them (migration 0036, ADR-0041).

use std::collections::HashMap;

use mellow_protocol::{LinkEmbed, Snowflake};

use super::Database;
use crate::storage::messages::{MessageStorageError, json_array_of_ids};
use crate::storage::sqlite_integers::{
    convert_integer, lookup_integer, to_snowflake, to_sqlite_integer,
};

/// A preview's picture as stored: a sealed file in the shared file store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredPreviewImage {
    pub(crate) file_id: Snowflake,
    pub(crate) content_type: String,
    pub(crate) size_bytes: u64,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredLinkPreview {
    pub(crate) id: Snowflake,
    pub(crate) url: String,
    pub(crate) title: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) site_name: Option<String>,
    pub(crate) theme_color: Option<u32>,
    pub(crate) image: Option<StoredPreviewImage>,
    /// A known provider's facts (migration 0039, ADR-0044); never `LinkEmbed::Unknown`.
    pub(crate) embed: Option<LinkEmbed>,
    /// The fetch gave nothing to show: kept only so the link is not fetched again too soon.
    pub(crate) is_empty: bool,
}

/// A preview to record, with when it was fetched and until when it may be reused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewLinkPreview {
    pub(crate) preview: StoredLinkPreview,
    pub(crate) url_sha256: [u8; 32],
    pub(crate) fetched_at_unix_milliseconds: u64,
    pub(crate) expires_at_unix_milliseconds: u64,
}

const PREVIEW_COLUMNS: &str = "link_previews.id, link_previews.url, link_previews.title, \
    link_previews.description, link_previews.site_name, link_previews.theme_color, \
    link_previews.image_file_id, link_previews.image_content_type, link_previews.image_size_bytes, \
    link_previews.image_width, link_previews.image_height, link_previews.is_empty, \
    link_previews.embed";

type PreviewRow = (
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    bool,
    Option<String>,
);

/// A message's id, then a preview's columns.
type MessagePreviewRow = (
    i64,
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    bool,
    Option<String>,
);

/// The stored embed, or none when this binary cannot read it: a row written by a newer release
/// (after a rollback) then shows the plain card instead of failing the whole history page.
fn embed_from_column(embed: Option<String>) -> Option<LinkEmbed> {
    embed
        .and_then(|json| serde_json::from_str(&json).ok())
        .filter(|embed| *embed != LinkEmbed::Unknown)
}

/// The embed as stored.
fn embed_column(embed: Option<&LinkEmbed>) -> Result<Option<String>, MessageStorageError> {
    embed
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| MessageStorageError::UnwritableLinkEmbed)
}

fn preview_from_row(row: PreviewRow) -> Result<StoredLinkPreview, MessageStorageError> {
    let (
        id,
        url,
        title,
        description,
        site_name,
        theme_color,
        image_file_id,
        image_content_type,
        image_size_bytes,
        image_width,
        image_height,
        is_empty,
        embed,
    ) = row;
    let image = match (
        image_file_id,
        image_content_type,
        image_size_bytes,
        image_width,
        image_height,
    ) {
        (Some(file_id), Some(content_type), Some(size_bytes), Some(width), Some(height)) => {
            Some(StoredPreviewImage {
                file_id: to_snowflake(file_id)?,
                content_type,
                size_bytes: convert_integer(size_bytes)?,
                width: convert_integer(width)?,
                height: convert_integer(height)?,
            })
        }
        _ => None,
    };
    Ok(StoredLinkPreview {
        id: to_snowflake(id)?,
        url,
        title,
        description,
        site_name,
        theme_color: theme_color.map(convert_integer).transpose()?,
        image,
        embed: embed_from_column(embed),
        is_empty,
    })
}

/// The newest preview of the link that is still fresh at `now`.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn find_fresh_preview(
    database: &Database,
    url_sha256: &[u8; 32],
    now_unix_milliseconds: u64,
) -> Result<Option<StoredLinkPreview>, MessageStorageError> {
    let row: Option<PreviewRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {PREVIEW_COLUMNS} FROM link_previews \
         WHERE url_sha256 = ? AND expires_at_unix_milliseconds > ? \
         ORDER BY fetched_at_unix_milliseconds DESC LIMIT 1"
    )))
    .bind(url_sha256.as_slice())
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .fetch_optional(database.readers())
    .await?;
    row.map(preview_from_row).transpose()
}

/// Records a fetched preview.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn insert_preview(
    database: &Database,
    new: &NewLinkPreview,
) -> Result<(), MessageStorageError> {
    let preview = &new.preview;
    let image = preview.image.as_ref();
    sqlx::query(
        "INSERT INTO link_previews (id, url_sha256, url, title, description, site_name, \
         theme_color, image_file_id, image_content_type, image_size_bytes, image_width, \
         image_height, is_empty, fetched_at_unix_milliseconds, expires_at_unix_milliseconds, \
         embed) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(to_sqlite_integer(preview.id.raw())?)
    .bind(new.url_sha256.as_slice())
    .bind(&preview.url)
    .bind(&preview.title)
    .bind(&preview.description)
    .bind(&preview.site_name)
    .bind(preview.theme_color.map(i64::from))
    .bind(
        image
            .map(|image| to_sqlite_integer(image.file_id.raw()))
            .transpose()?,
    )
    .bind(image.map(|image| image.content_type.as_str()))
    .bind(
        image
            .map(|image| to_sqlite_integer(image.size_bytes))
            .transpose()?,
    )
    .bind(image.map(|image| i64::from(image.width)))
    .bind(image.map(|image| i64::from(image.height)))
    .bind(preview.is_empty)
    .bind(to_sqlite_integer(new.fetched_at_unix_milliseconds)?)
    .bind(to_sqlite_integer(new.expires_at_unix_milliseconds)?)
    .bind(embed_column(preview.embed.as_ref())?)
    .execute(database.writer())
    .await?;
    Ok(())
}

/// Adds the previews a job found to the message, if its content is still `expected_content` (an
/// edit meanwhile dropped what it removed and queued its own job). `links` are the message's
/// links in order, each with the preview the job found for it, if any: a link without one keeps
/// the preview it already shows, so a job cut short never takes a card away. Returns whether the
/// message's previews changed, or `None` when it wrote nothing.
///
/// # Errors
/// Fails on database errors. Nothing is written then.
pub(crate) async fn add_message_previews(
    database: &Database,
    message_id: Snowflake,
    expected_content: &str,
    links: &[(&str, Option<Snowflake>)],
) -> Result<Option<bool>, MessageStorageError> {
    let message_id = to_sqlite_integer(message_id.raw())?;
    let mut transaction = database.writer().begin_with("BEGIN IMMEDIATE").await?;
    let is_unchanged: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM messages WHERE id = ? AND content = ?)")
            .bind(message_id)
            .bind(expected_content)
            .fetch_one(&mut *transaction)
            .await?;
    if !is_unchanged {
        return Ok(None);
    }
    let shown = shown_previews(&mut transaction, message_id).await?;
    let mut wanted = Vec::with_capacity(links.len());
    for (url, found) in links {
        let found = found.map(|id| to_sqlite_integer(id.raw())).transpose()?;
        let already_shown = shown
            .iter()
            .find(|(_, shown_url)| shown_url == url)
            .map(|(id, _)| *id);
        wanted.extend(found.or(already_shown));
    }
    if shown.iter().map(|(id, _)| *id).eq(wanted.iter().copied()) {
        return Ok(Some(false));
    }
    sqlx::query("DELETE FROM message_link_previews WHERE message_id = ?")
        .bind(message_id)
        .execute(&mut *transaction)
        .await?;
    for (position, preview_id) in wanted.into_iter().enumerate() {
        sqlx::query(
            "INSERT INTO message_link_previews (message_id, position, link_preview_id) \
             VALUES (?, ?, ?)",
        )
        .bind(message_id)
        .bind(convert_integer::<_, i64>(position)?)
        .bind(preview_id)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(Some(true))
}

/// The message's previews, id and link, in order.
async fn shown_previews(
    connection: &mut sqlx::SqliteConnection,
    message_id: i64,
) -> Result<Vec<(i64, String)>, MessageStorageError> {
    Ok(sqlx::query_as(
        "SELECT link_previews.id, link_previews.url FROM message_link_previews \
         JOIN link_previews ON link_previews.id = message_link_previews.link_preview_id \
         WHERE message_link_previews.message_id = ? ORDER BY message_link_previews.position",
    )
    .bind(message_id)
    .fetch_all(connection)
    .await?)
}

/// Drops the message's previews of links no longer in `kept_links`, inside an edit's
/// transaction: the edit itself takes away what it removed, whatever happens to its job. Returns
/// the previews it kept, in order, read in the same transaction so the edit's event carries
/// exactly what it committed, even a card a job recorded while the edit waited for the writer.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn keep_previews_of_links(
    connection: &mut sqlx::SqliteConnection,
    message_id: Snowflake,
    kept_links: &[&str],
) -> Result<Vec<StoredLinkPreview>, MessageStorageError> {
    let message_id = to_sqlite_integer(message_id.raw())?;
    let rows: Vec<PreviewRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {PREVIEW_COLUMNS} FROM message_link_previews \
         JOIN link_previews ON link_previews.id = message_link_previews.link_preview_id \
         WHERE message_link_previews.message_id = ? ORDER BY message_link_previews.position"
    )))
    .bind(message_id)
    .fetch_all(&mut *connection)
    .await?;
    let mut kept = Vec::with_capacity(rows.len());
    for row in rows {
        let preview = preview_from_row(row)?;
        if kept_links.contains(&preview.url.as_str()) {
            kept.push(preview);
        } else {
            sqlx::query(
                "DELETE FROM message_link_previews WHERE message_id = ? AND link_preview_id = ?",
            )
            .bind(message_id)
            .bind(to_sqlite_integer(preview.id.raw())?)
            .execute(&mut *connection)
            .await?;
        }
    }
    Ok(kept)
}

/// The previews of each of `message_ids` that has some, in their order: one query.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn previews_of_messages(
    database: &Database,
    message_ids: &[Snowflake],
) -> Result<HashMap<Snowflake, Vec<StoredLinkPreview>>, MessageStorageError> {
    if message_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<MessagePreviewRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT message_link_previews.message_id, {PREVIEW_COLUMNS} \
             FROM message_link_previews \
             JOIN link_previews ON link_previews.id = message_link_previews.link_preview_id \
             WHERE message_link_previews.message_id IN (SELECT value FROM json_each(?)) \
             ORDER BY message_link_previews.message_id, message_link_previews.position"
    )))
    .bind(json_array_of_ids(message_ids))
    .fetch_all(database.readers())
    .await?;
    let mut previews: HashMap<Snowflake, Vec<StoredLinkPreview>> = HashMap::new();
    for row in rows {
        let (
            message_id,
            id,
            url,
            title,
            description,
            site_name,
            theme,
            file,
            kind,
            size,
            width,
            height,
            is_empty,
            embed,
        ) = row;
        previews
            .entry(to_snowflake(message_id)?)
            .or_default()
            .push(preview_from_row((
                id,
                url,
                title,
                description,
                site_name,
                theme,
                file,
                kind,
                size,
                width,
                height,
                is_empty,
                embed,
            ))?);
    }
    Ok(previews)
}

/// The picture of a preview the message shows, if the message is in `channel_id`.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn find_message_preview_image(
    database: &Database,
    channel_id: Snowflake,
    message_id: Snowflake,
    preview_id: Snowflake,
) -> Result<Option<StoredPreviewImage>, MessageStorageError> {
    let (Some(channel_id), Some(message_id), Some(preview_id)) = (
        lookup_integer(channel_id),
        lookup_integer(message_id),
        lookup_integer(preview_id),
    ) else {
        return Ok(None);
    };
    let row: Option<(i64, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT link_previews.image_file_id, link_previews.image_content_type, \
         link_previews.image_size_bytes, link_previews.image_width, link_previews.image_height \
         FROM message_link_previews \
         JOIN messages ON messages.id = message_link_previews.message_id \
         JOIN link_previews ON link_previews.id = message_link_previews.link_preview_id \
         WHERE message_link_previews.message_id = ? AND messages.channel_id = ? \
         AND message_link_previews.link_preview_id = ? \
         AND link_previews.image_file_id IS NOT NULL",
    )
    .bind(message_id)
    .bind(channel_id)
    .bind(preview_id)
    .fetch_optional(database.readers())
    .await?;
    row.map(|(file_id, content_type, size_bytes, width, height)| {
        Ok(StoredPreviewImage {
            file_id: to_snowflake(file_id)?,
            content_type,
            size_bytes: convert_integer(size_bytes)?,
            width: convert_integer(width)?,
            height: convert_integer(height)?,
        })
    })
    .transpose()
}

/// Deletes previews that no message shows and that expired at least `grace_milliseconds` before
/// `now` (longer than any fetch job, which may be about to link one it found fresh), and returns
/// their pictures' file ids.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn delete_expired_unshown_previews(
    database: &Database,
    now_unix_milliseconds: u64,
    grace_milliseconds: u64,
) -> Result<Vec<Snowflake>, MessageStorageError> {
    let file_ids: Vec<Option<i64>> = sqlx::query_scalar(
        "DELETE FROM link_previews WHERE expires_at_unix_milliseconds <= ? \
         AND NOT EXISTS (SELECT 1 FROM message_link_previews \
             WHERE message_link_previews.link_preview_id = link_previews.id) \
         RETURNING image_file_id",
    )
    .bind(to_sqlite_integer(
        now_unix_milliseconds.saturating_sub(grace_milliseconds),
    )?)
    .fetch_all(database.writer())
    .await?;
    file_ids
        .into_iter()
        .flatten()
        .map(|id| Ok(to_snowflake(id)?))
        .collect()
}

/// Every picture file a preview holds, for the orphan sweep.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn all_preview_image_file_ids(
    database: &Database,
) -> Result<Vec<Snowflake>, MessageStorageError> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT image_file_id FROM link_previews WHERE image_file_id IS NOT NULL",
    )
    .fetch_all(database.readers())
    .await?;
    ids.into_iter().map(|id| Ok(to_snowflake(id)?)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::tests::migrated_database;

    const HOUR: u64 = 60 * 60 * 1_000;

    fn preview(id: u64, url: &str, file_id: Option<u64>) -> StoredLinkPreview {
        StoredLinkPreview {
            id: Snowflake::from_raw(id),
            url: url.into(),
            title: Some("Title".into()),
            description: None,
            site_name: None,
            theme_color: None,
            image: file_id.map(|file_id| StoredPreviewImage {
                file_id: Snowflake::from_raw(file_id),
                content_type: "image/webp".into(),
                size_bytes: 10,
                width: 4,
                height: 3,
            }),
            embed: None,
            is_empty: false,
        }
    }

    async fn record(database: &Database, preview: StoredLinkPreview, expires_at: u64) {
        insert_preview(
            database,
            &NewLinkPreview {
                url_sha256: [u8::try_from(preview.id.raw() % 256).unwrap(); 32],
                preview,
                fetched_at_unix_milliseconds: 0,
                expires_at_unix_milliseconds: expires_at,
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn previews_are_added_only_on_the_text_the_job_saw_and_kept_where_a_job_found_none() {
        const TEXT: &str = "see https://a.example/ and https://b.example/";
        let (_directory, database) = migrated_database().await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0); \
             INSERT INTO channels (id, guild_id, kind, name, position) \
             VALUES (20, 10, 'text', 'general', 0); \
             INSERT INTO messages (id, channel_id, author_id, content) VALUES (30, 20, 1, '{TEXT}');"
        )))
        .execute(database.writer())
        .await
        .unwrap();
        let (a, b) = ("https://a.example/", "https://b.example/");
        record(&database, preview(40, a, None), HOUR).await;
        record(&database, preview(41, b, None), HOUR).await;
        let message = Snowflake::from_raw(30);
        let (preview_a, preview_b) = (Some(Snowflake::from_raw(40)), Some(Snowflake::from_raw(41)));
        let shown_ids = async || -> Vec<u64> {
            previews_of_messages(&database, &[message])
                .await
                .unwrap()
                .remove(&message)
                .unwrap_or_default()
                .iter()
                .map(|preview| preview.id.raw())
                .collect()
        };

        let stale = add_message_previews(&database, message, "older text", &[(a, preview_a)])
            .await
            .unwrap();
        let first_only =
            add_message_previews(&database, message, TEXT, &[(a, preview_a), (b, None)])
                .await
                .unwrap();
        let after_first = shown_ids().await;
        let second_only =
            add_message_previews(&database, message, TEXT, &[(a, None), (b, preview_b)])
                .await
                .unwrap();
        let after_second = shown_ids().await;
        let again = add_message_previews(&database, message, TEXT, &[(a, None), (b, preview_b)])
            .await
            .unwrap();
        database.close().await;

        assert_eq!(stale, None, "an edit happened since the job read the text");
        assert_eq!(first_only, Some(true));
        assert_eq!(after_first, [40]);
        assert_eq!(second_only, Some(true));
        assert_eq!(after_second, [40, 41], "in link order, the first one kept");
        assert_eq!(again, Some(false), "nothing new to publish");
    }

    #[tokio::test]
    async fn an_edit_drops_only_the_previews_of_the_links_it_removed_and_returns_the_rest() {
        let (_directory, database) = migrated_database().await;
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0); \
             INSERT INTO channels (id, guild_id, kind, name, position) \
             VALUES (20, 10, 'text', 'general', 0); \
             INSERT INTO messages (id, channel_id, author_id, content) VALUES (30, 20, 1, 'x');",
        )
        .execute(database.writer())
        .await
        .unwrap();
        let (a, b) = ("https://a.example/", "https://b.example/");
        record(&database, preview(40, a, None), HOUR).await;
        record(&database, preview(41, b, None), HOUR).await;
        let message = Snowflake::from_raw(30);
        add_message_previews(
            &database,
            message,
            "x",
            &[
                (a, Some(Snowflake::from_raw(40))),
                (b, Some(Snowflake::from_raw(41))),
            ],
        )
        .await
        .unwrap();

        let mut connection = database.writer().acquire().await.unwrap();
        let kept = keep_previews_of_links(&mut connection, message, &[b, "https://c.example/"])
            .await
            .unwrap();
        drop(connection);
        let shown = previews_of_messages(&database, &[message]).await.unwrap();
        database.close().await;

        let urls: Vec<&str> = shown[&message]
            .iter()
            .map(|preview| preview.url.as_str())
            .collect();
        assert_eq!(urls, [b]);
        let kept_urls: Vec<&str> = kept.iter().map(|preview| preview.url.as_str()).collect();
        assert_eq!(kept_urls, [b], "what it kept is what the message now shows");
    }

    #[tokio::test]
    async fn the_purge_keeps_shown_previews_and_fresh_ones_and_frees_the_others_pictures() {
        let (_directory, database) = migrated_database().await;
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0); \
             INSERT INTO channels (id, guild_id, kind, name, position) \
             VALUES (20, 10, 'text', 'general', 0); \
             INSERT INTO messages (id, channel_id, author_id, content) VALUES (30, 20, 1, 'x');",
        )
        .execute(database.writer())
        .await
        .unwrap();
        record(
            &database,
            preview(40, "https://shown.example/", Some(400)),
            HOUR,
        )
        .await;
        record(
            &database,
            preview(41, "https://gone.example/", Some(410)),
            HOUR,
        )
        .await;
        record(
            &database,
            preview(42, "https://fresh.example/", Some(420)),
            10 * HOUR,
        )
        .await;
        record(
            &database,
            preview(43, "https://recent.example/", None),
            2 * HOUR,
        )
        .await;
        add_message_previews(
            &database,
            Snowflake::from_raw(30),
            "x",
            &[("https://shown.example/", Some(Snowflake::from_raw(40)))],
        )
        .await
        .unwrap();

        let deleted = delete_expired_unshown_previews(&database, 3 * HOUR, HOUR / 2)
            .await
            .unwrap();
        let pictures = all_preview_image_file_ids(&database).await.unwrap();
        database.close().await;

        assert_eq!(deleted, [Snowflake::from_raw(410)]);
        let mut pictures: Vec<u64> = pictures.into_iter().map(Snowflake::raw).collect();
        pictures.sort_unstable();
        assert_eq!(pictures, [400, 420], "shown and fresh pictures stay");
    }

    #[tokio::test]
    async fn embeds_are_stored_and_read_back_and_unreadable_ones_show_the_plain_card() {
        let (_directory, database) = migrated_database().await;
        let embed = LinkEmbed::Twitch(mellow_protocol::TwitchEmbed {
            item: mellow_protocol::TwitchItem::Clip,
            id: "AwkwardHelplessSalamander".into(),
        });
        record(
            &database,
            StoredLinkPreview {
                embed: Some(embed.clone()),
                ..preview(40, "https://clips.twitch.tv/a", None)
            },
            HOUR,
        )
        .await;
        let stored = find_fresh_preview(&database, &[40; 32], 0).await.unwrap();
        // A row this release cannot read (a field missing), and a newer release's kind.
        sqlx::query("UPDATE link_previews SET embed = ? WHERE id = 40")
            .bind(r#"{"kind":"twitch","item":"clip"}"#)
            .execute(database.writer())
            .await
            .unwrap();
        let missing_id = find_fresh_preview(&database, &[40; 32], 0).await.unwrap();
        sqlx::query("UPDATE link_previews SET embed = ? WHERE id = 40")
            .bind(r#"{"kind":"hologram"}"#)
            .execute(database.writer())
            .await
            .unwrap();
        let unknown = find_fresh_preview(&database, &[40; 32], 0).await.unwrap();
        let not_an_object = sqlx::query("UPDATE link_previews SET embed = '[1]' WHERE id = 40")
            .execute(database.writer())
            .await;
        database.close().await;

        assert_eq!(stored.unwrap().embed, Some(embed));
        assert_eq!(missing_id.unwrap().embed, None);
        assert_eq!(unknown.unwrap().embed, None, "clients never get `unknown`");
        assert!(
            not_an_object.is_err(),
            "the column holds an object with a kind"
        );
    }
}
