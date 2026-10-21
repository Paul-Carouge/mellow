//! Attachment uploads and downloads (ADR-0020).

use std::time::Instant;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use mellow_protocol::{Attachment, Snowflake};
use serde::Deserialize;

use crate::domain::clock::now_unix_milliseconds;
use crate::domain::message_rules::PREVIEWABLE_IMAGE_TYPES;
use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiPath, ApiQuery};
use crate::http::authentication::AuthenticatedUser;
use crate::http::rate_limit::{ATTACHMENT_UPLOADS_PER_USER, ATTACHMENT_UPLOADS_PER_USER_PER_HOUR};

/// Nothing in a served file may run or load anything, even if a browser renders it.
const ATTACHMENT_CONTENT_SECURITY_POLICY: &str = "default-src 'none'; sandbox";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UploadQuery {
    file_name: String,
}

/// `POST /api/v1/channels/{channel_id}/attachments?file_name=…`, the raw file as the body. The
/// size limit is enforced on the body before it is read whole (`DefaultBodyLimit` on the route).
pub(crate) async fn upload(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(channel_id): ApiPath<Snowflake>,
    ApiQuery(query): ApiQuery<UploadQuery>,
    body: Bytes,
) -> Result<(StatusCode, Json<Attachment>), ApiError> {
    let now = Instant::now();
    let within_burst = state.rate_limiter.record_event(
        &format!("attachment-upload:{}", user.id),
        ATTACHMENT_UPLOADS_PER_USER,
        now,
    );
    let within_hour = state.rate_limiter.record_event(
        &format!("attachment-upload-hour:{}", user.id),
        ATTACHMENT_UPLOADS_PER_USER_PER_HOUR,
        now,
    );
    if !(within_burst && within_hour) {
        return Err(ApiError::rate_limited());
    }
    if body.is_empty() {
        return Err(ApiError::validation("an attachment cannot be empty".into()));
    }
    let attachment = state
        .attachments
        .upload(
            user.id,
            channel_id,
            &query.file_name,
            body.to_vec(),
            now_unix_milliseconds(),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(attachment)))
}

/// `GET /api/v1/attachments/{id}/{file_name}`: the name in the path is only for the browser's
/// "save as"; the stored one is what `Content-Disposition` says.
pub(crate) async fn download(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((attachment_id, _file_name)): ApiPath<(Snowflake, String)>,
) -> Result<Response, ApiError> {
    let download = state.attachments.download(user.id, attachment_id).await?;
    let is_previewable = PREVIEWABLE_IMAGE_TYPES.contains(&download.content_type.as_str());
    let disposition = format!(
        "{}; filename*=UTF-8''{}",
        if is_previewable {
            "inline"
        } else {
            "attachment"
        },
        percent_encode_file_name(&download.file_name)
    );
    let content_type = HeaderValue::from_str(&download.content_type)
        .map_err(|error| ApiError::internal("an attachment type", &error))?;
    let disposition = HeaderValue::from_str(&disposition)
        .map_err(|error| ApiError::internal("an attachment name", &error))?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CONTENT_DISPOSITION, disposition),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(ATTACHMENT_CONTENT_SECURITY_POLICY),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=86400"),
            ),
        ],
        download.bytes,
    )
        .into_response())
}

/// RFC 5987 `filename*`: everything but unreserved ASCII is percent-encoded, so no quote, comma
/// or line break can end the header value early.
fn percent_encode_file_name(name: &str) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            // Writing to a String cannot fail.
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_cannot_break_out_of_the_disposition_header() {
        assert_eq!(percent_encode_file_name("photo.png"), "photo.png");
        assert_eq!(
            percent_encode_file_name("a\"b;c=d é.txt"),
            "a%22b%3Bc%3Dd%20%C3%A9.txt"
        );
    }
}
