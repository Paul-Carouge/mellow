//! GIFs from Klipy (ADR-0042): the browser's settings and the member's starred GIFs.

use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

use mellow_protocol::GifSettings;

use crate::domain::clock::now_unix_milliseconds;
use crate::domain::gifs;
use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiPath};
use crate::http::authentication::AuthenticatedUser;
use crate::http::rate_limit::GIF_FAVOURITE_CHANGES_PER_USER;

/// `GET /api/v1/gifs/settings`: `404` when the instance has no Klipy key.
pub(crate) async fn settings(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<GifSettings>, ApiError> {
    Ok(Json(require_gifs_on(&state)?.settings_for(user.id)))
}

/// `GET /api/v1/users/@me/gif-favourites`: Klipy slugs, newest first.
pub(crate) async fn favourites(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<String>>, ApiError> {
    require_gifs_on(&state)?;
    Ok(Json(gifs::favourites(&state.database, user.id).await?))
}

/// Every GIF route answers `404` while the instance has no Klipy key (ADR-0042).
fn require_gifs_on(state: &AppState) -> Result<&crate::domain::gifs::Gifs, ApiError> {
    state.gifs.as_deref().ok_or_else(ApiError::not_found)
}

fn require_change_allowed(
    state: &AppState,
    user_id: mellow_protocol::Snowflake,
) -> Result<(), ApiError> {
    if state.rate_limiter.record_event(
        &format!("gif-favourite:{user_id}"),
        GIF_FAVOURITE_CHANGES_PER_USER,
        Instant::now(),
    ) {
        Ok(())
    } else {
        Err(ApiError::rate_limited())
    }
}

/// `PUT /api/v1/users/@me/gif-favourites/{slug}`: `204`, or `409` past the cap.
pub(crate) async fn add_favourite(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(slug): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    require_gifs_on(&state)?;
    require_change_allowed(&state, user.id)?;
    gifs::add_favourite(&state.database, user.id, &slug, now_unix_milliseconds()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/users/@me/gif-favourites/{slug}`: `204`, starred or not.
pub(crate) async fn remove_favourite(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(slug): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    require_gifs_on(&state)?;
    require_change_allowed(&state, user.id)?;
    gifs::remove_favourite(&state.database, user.id, &slug).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use mellow_protocol::GifSettings;
    use zeroize::Zeroizing;

    use crate::domain::account_rules::RegistrationMode;
    use crate::domain::accounts::tests::migrated_database;
    use crate::domain::gifs::Gifs;
    use crate::http::test_support::*;

    #[tokio::test]
    async fn gif_settings_reach_signed_in_members_only_when_a_key_is_set() {
        let (_directory, database) = migrated_database().await;
        let without_key = test_state(database.clone(), RegistrationMode::Open);
        let token = registered_token(&without_key, "ada").await;
        let (status_off, _) = send(
            &without_key,
            authorized("GET", "/api/v1/gifs/settings", &token, None),
        )
        .await;
        let with_key = without_key
            .clone()
            .with_gifs(Gifs::new("test-key".into(), Zeroizing::new([3; 32])));
        let (status_on, body) = send(
            &with_key,
            authorized("GET", "/api/v1/gifs/settings", &token, None),
        )
        .await;
        let (status_anonymous, _) = send(
            &with_key,
            axum::http::Request::get("/api/v1/gifs/settings")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
        database.close().await;

        assert_eq!(status_off, axum::http::StatusCode::NOT_FOUND);
        assert_eq!(status_on, axum::http::StatusCode::OK);
        let settings: GifSettings = json(&body);
        assert_eq!(settings.klipy_app_key, "test-key");
        assert_eq!(settings.customer_id.len(), 32);
        assert_eq!(status_anonymous, axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn favourites_are_kept_per_member_by_slug() {
        let (_directory, database) = migrated_database().await;
        let without_key = test_state(database.clone(), RegistrationMode::Open);
        let ada = registered_token(&without_key, "ada").await;
        let bob = registered_token(&without_key, "bob").await;
        let path = "/api/v1/users/@me/gif-favourites";
        let (off, _) = send(
            &without_key,
            authorized("PUT", &format!("{path}/happy-cat"), &ada, None),
        )
        .await;
        let state = without_key.with_gifs(Gifs::new("test-key".into(), Zeroizing::new([3; 32])));
        let (added, _) = send(
            &state,
            authorized("PUT", &format!("{path}/happy-cat"), &ada, None),
        )
        .await;
        let (invalid, _) = send(
            &state,
            authorized("PUT", &format!("{path}/a.b"), &ada, None),
        )
        .await;
        let (_, ada_list) = send(&state, authorized("GET", path, &ada, None)).await;
        let (_, bob_list) = send(&state, authorized("GET", path, &bob, None)).await;
        let (removed, _) = send(
            &state,
            authorized("DELETE", &format!("{path}/happy-cat"), &ada, None),
        )
        .await;
        let (_, after_removal) = send(&state, authorized("GET", path, &ada, None)).await;
        database.close().await;

        assert_eq!(off, axum::http::StatusCode::NOT_FOUND, "no key, no GIFs");
        assert_eq!(added, axum::http::StatusCode::NO_CONTENT);
        assert_eq!(invalid, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(json::<Vec<String>>(&ada_list), ["happy-cat"]);
        assert_eq!(json::<Vec<String>>(&bob_list), Vec::<String>::new());
        assert_eq!(removed, axum::http::StatusCode::NO_CONTENT);
        assert_eq!(json::<Vec<String>>(&after_removal), Vec::<String>::new());
    }
}
