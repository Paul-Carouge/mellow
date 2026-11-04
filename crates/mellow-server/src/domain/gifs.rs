//! GIFs from Klipy (ADR-0042): the settings a member's browser needs, and their starred GIFs.

use hkdf::Hkdf;
use mellow_protocol::{GifSettings, MAXIMUM_GIF_FAVOURITES, Snowflake};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::domain::messages::MessageError;
use crate::storage::Database;
use crate::storage::gif_favourites as storage;

/// Klipy slugs are letters, digits, `-` and `_`.
const MAXIMUM_SLUG_LENGTH: usize = 64;
const CUSTOMER_ID_BYTES: usize = 16;

#[derive(Debug, thiserror::Error)]
pub(crate) enum GifError {
    #[error("not a Klipy slug")]
    InvalidSlug,
    #[error("at most {MAXIMUM_GIF_FAVOURITES} starred GIFs")]
    TooManyFavourites,
    #[error(transparent)]
    Message(#[from] MessageError),
}

/// The instance's Klipy key and the key turning user ids into opaque customer ids.
pub(crate) struct Gifs {
    klipy_app_key: String,
    customer_key: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for Gifs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Gifs(<keys redacted>)")
    }
}

impl Gifs {
    pub(crate) fn new(klipy_app_key: String, customer_key: Zeroizing<[u8; 32]>) -> Self {
        Self {
            klipy_app_key,
            customer_key,
        }
    }

    /// What `user_id`'s browser needs to search Klipy. The customer id is a keyed hash of the
    /// user id: stable, and telling Klipy nothing about the account.
    pub(crate) fn settings_for(&self, user_id: Snowflake) -> GifSettings {
        let derivation = Hkdf::<Sha256>::new(None, self.customer_key.as_ref());
        let mut customer_id = [0_u8; CUSTOMER_ID_BYTES];
        // Invariant: HKDF-SHA256 can produce up to 255 × 32 bytes, and we ask for 16.
        #[allow(clippy::expect_used)]
        derivation
            .expand(&user_id.raw().to_be_bytes(), &mut customer_id)
            .expect("16 bytes is within HKDF-SHA256's output limit");
        GifSettings {
            klipy_app_key: self.klipy_app_key.clone(),
            customer_id: customer_id.iter().fold(
                String::with_capacity(2 * CUSTOMER_ID_BYTES),
                |mut text, byte| {
                    use std::fmt::Write as _;
                    // Writing to a String never fails.
                    let _ = write!(text, "{byte:02x}");
                    text
                },
            ),
        }
    }
}

fn validate_slug(slug: &str) -> Result<(), GifError> {
    let is_valid = !slug.is_empty()
        && slug.len() <= MAXIMUM_SLUG_LENGTH
        && slug
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
    if is_valid {
        Ok(())
    } else {
        Err(GifError::InvalidSlug)
    }
}

/// The member's starred GIFs, newest first.
pub(crate) async fn favourites(
    database: &Database,
    user_id: Snowflake,
) -> Result<Vec<String>, GifError> {
    Ok(storage::favourite_slugs(database, user_id)
        .await
        .map_err(MessageError::from)?)
}

pub(crate) async fn add_favourite(
    database: &Database,
    user_id: Snowflake,
    slug: &str,
    now_unix_milliseconds: u64,
) -> Result<(), GifError> {
    validate_slug(slug)?;
    let is_added = storage::add_favourite(
        database,
        user_id,
        slug,
        now_unix_milliseconds,
        MAXIMUM_GIF_FAVOURITES,
    )
    .await
    .map_err(MessageError::from)?;
    if is_added {
        Ok(())
    } else {
        Err(GifError::TooManyFavourites)
    }
}

pub(crate) async fn remove_favourite(
    database: &Database,
    user_id: Snowflake,
    slug: &str,
) -> Result<(), GifError> {
    validate_slug(slug)?;
    Ok(storage::remove_favourite(database, user_id, slug)
        .await
        .map_err(MessageError::from)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::tests::{NOW, migrated_database};

    #[test]
    fn customer_ids_are_stable_per_user_and_reveal_nothing_of_the_id() {
        let gifs = Gifs::new("key".into(), Zeroizing::new([7; 32]));
        let first = gifs.settings_for(Snowflake::from_raw(1)).customer_id;
        assert_eq!(first, gifs.settings_for(Snowflake::from_raw(1)).customer_id);
        assert_ne!(first, gifs.settings_for(Snowflake::from_raw(2)).customer_id);
        assert_eq!(first.len(), 2 * CUSTOMER_ID_BYTES);
        let other_instance = Gifs::new("key".into(), Zeroizing::new([8; 32]));
        assert_ne!(
            first,
            other_instance
                .settings_for(Snowflake::from_raw(1))
                .customer_id
        );
    }

    #[tokio::test]
    async fn favourites_are_valid_slugs_newest_first_and_capped() {
        let (_directory, database) = migrated_database().await;
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0);",
        )
        .execute(database.writer())
        .await
        .unwrap();
        let ada = Snowflake::from_raw(1);
        add_favourite(&database, ada, "cat-1", NOW).await.unwrap();
        add_favourite(&database, ada, "dog_2", NOW + 1)
            .await
            .unwrap();
        add_favourite(&database, ada, "cat-1", NOW + 2)
            .await
            .unwrap();
        let invalid = add_favourite(&database, ada, "../x", NOW).await;
        let listed = favourites(&database, ada).await.unwrap();
        remove_favourite(&database, ada, "dog_2").await.unwrap();
        let after_removal = favourites(&database, ada).await.unwrap();
        for index in 0..MAXIMUM_GIF_FAVOURITES {
            add_favourite(&database, ada, &format!("fill-{index}"), NOW)
                .await
                .unwrap_or(());
        }
        let over_the_cap = add_favourite(&database, ada, "one-more", NOW).await;
        let restarred = add_favourite(&database, ada, "cat-1", NOW + 9).await;
        database.close().await;

        assert!(matches!(invalid, Err(GifError::InvalidSlug)));
        assert_eq!(listed, ["cat-1", "dog_2"], "starring again moves it first");
        assert_eq!(after_removal, ["cat-1"]);
        assert!(matches!(over_the_cap, Err(GifError::TooManyFavourites)));
        assert!(restarred.is_ok(), "one already starred still moves first");
    }
}
