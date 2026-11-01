//! Attachment uploads and downloads (ADR-0020). File names never travel in URLs, so proxy access
//! logs and browser history never hold them (CLAUDE.md: logs never contain attachment names).

use std::time::Instant;

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use mellow_protocol::{Attachment, Snowflake};

use crate::domain::clock::now_unix_milliseconds;
use crate::domain::message_rules::PREVIEWABLE_IMAGE_TYPES;
use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiPath};
use crate::http::authentication::AuthenticatedUser;
use crate::http::rate_limit::{
    ATTACHMENT_DOWNLOADS_PER_USER, ATTACHMENT_UPLOADS_PER_USER,
    ATTACHMENT_UPLOADS_PER_USER_PER_HOUR,
};

/// The upload's file name, percent-encoded UTF-8 (RFC 3986), as `encodeURIComponent` writes it.
pub(crate) const FILE_NAME_HEADER: HeaderName = HeaderName::from_static("x-mellow-file-name");
/// Nothing in a served file may run or load anything, even if a browser renders it.
const ATTACHMENT_CONTENT_SECURITY_POLICY: &str = "default-src 'none'; sandbox";

/// `POST /api/v1/channels/{channel_id}/attachments`, the raw file as the body and its name in
/// `X-Mellow-File-Name`. Rate limits, permissions, the name, the quota and a transfer slot are
/// checked before the body is read; `DefaultBodyLimit` on the route answers 413 on a declared
/// length over the limit, and reading stops at the limit otherwise.
pub(crate) async fn upload(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(channel_id): ApiPath<Snowflake>,
    headers: HeaderMap,
    body: Body,
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
    let file_name = headers
        .get(FILE_NAME_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(percent_decode)
        .ok_or_else(|| ApiError::validation("send the file name in X-Mellow-File-Name".into()))?;
    let slot = state
        .attachments
        .start_upload(user.id, channel_id, &file_name)
        .await?;
    let limit = usize::try_from(state.maximum_attachment_bytes).unwrap_or(usize::MAX);
    // The slot is held while the body arrives: a client trickling bytes gets a deadline.
    let bytes = read_upload_body(body, limit).await?;
    if bytes.is_empty() {
        return Err(ApiError::validation("an attachment cannot be empty".into()));
    }
    let attachment = state
        .attachments
        .finish_upload(slot, Vec::from(bytes), now_unix_milliseconds())
        .await?;
    Ok((StatusCode::CREATED, Json(attachment)))
}

/// `GET /api/v1/attachments/{id}`: the stored name only travels in `Content-Disposition`.
pub(crate) async fn download(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(attachment_id): ApiPath<Snowflake>,
) -> Result<Response, ApiError> {
    if !state.rate_limiter.record_event(
        &format!("attachment-download:{}", user.id),
        ATTACHMENT_DOWNLOADS_PER_USER,
        Instant::now(),
    ) {
        return Err(ApiError::rate_limited());
    }
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
            // Revalidated on every view: a shared computer keeps no conversation's files usable
            // after its user signed out.
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, no-cache"),
            ),
        ],
        body_keeping_permit(download.bytes, download.permit),
    )
        .into_response())
}

/// The file as a response body that holds its transfer slot until the body is dropped, after its
/// last byte is sent: a slow reader keeps a slot, not an unbounded number of buffers.
pub(crate) fn body_keeping_permit(
    bytes: Vec<u8>,
    permit: tokio::sync::OwnedSemaphorePermit,
) -> axum::body::Body {
    use futures_util::StreamExt as _;
    let chunks = futures_util::stream::iter([Ok::<_, std::convert::Infallible>(
        axum::body::Bytes::from(bytes),
    )])
    .map(move |chunk| {
        let _held_until_the_body_is_dropped = &permit;
        chunk
    });
    axum::body::Body::from_stream(chunks)
}

/// Slowest upload speed accepted for the largest file, and the least time any upload gets.
const MINIMUM_UPLOAD_BYTES_PER_SECOND: u64 = 64 * 1024;
const MINIMUM_UPLOAD_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// Time allowed to receive an upload body of at most `maximum_bytes`.
fn upload_deadline(maximum_bytes: u64) -> std::time::Duration {
    std::time::Duration::from_secs(maximum_bytes / MINIMUM_UPLOAD_BYTES_PER_SECOND)
        .max(MINIMUM_UPLOAD_DEADLINE)
}

/// Reads an upload body of at most `maximum_bytes`; a client trickling bytes gets
/// `server_busy` once the deadline for that size passes, so it cannot hold a request open.
pub(crate) async fn read_upload_body(
    body: Body,
    maximum_bytes: usize,
) -> Result<axum::body::Bytes, ApiError> {
    let deadline = upload_deadline(u64::try_from(maximum_bytes).unwrap_or(u64::MAX));
    tokio::time::timeout(deadline, axum::body::to_bytes(body, maximum_bytes))
        .await
        .map_err(|_| ApiError::server_busy())?
        .map_err(|_| ApiError::payload_too_large())
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

/// `None` for a malformed escape or bytes that are not UTF-8.
fn percent_decode(encoded: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut rest = encoded.bytes();
    while let Some(byte) = rest.next() {
        if byte == b'%' {
            let high = char::from(rest.next()?).to_digit(16)?;
            let low = char::from(rest.next()?).to_digit(16)?;
            bytes.push(u8::try_from(high * 16 + low).ok()?);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_uploads_get_time_at_a_slow_speed_and_small_ones_a_floor() {
        assert_eq!(upload_deadline(25 * 1024 * 1024).as_secs(), 400);
        assert_eq!(upload_deadline(1024), MINIMUM_UPLOAD_DEADLINE);
    }

    #[test]
    fn file_names_cannot_break_out_of_the_disposition_header() {
        assert_eq!(percent_encode_file_name("photo.png"), "photo.png");
        assert_eq!(
            percent_encode_file_name("a\"b;c=d é.txt"),
            "a%22b%3Bc%3Dd%20%C3%A9.txt"
        );
    }

    #[test]
    fn header_file_names_decode_as_encode_uri_component_writes_them() {
        assert_eq!(
            percent_decode("vacances%20d%27%C3%A9t%C3%A9.png").as_deref(),
            Some("vacances d'été.png")
        );
        assert_eq!(percent_decode("%ZZ"), None);
        assert_eq!(percent_decode("%C3"), None, "not UTF-8");
        assert_eq!(percent_decode("cut%4"), None);
    }
}
