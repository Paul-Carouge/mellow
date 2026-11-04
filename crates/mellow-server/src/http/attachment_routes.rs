//! Attachment uploads and downloads (ADR-0020). File names never travel in URLs, so proxy access
//! logs and browser history never hold them (CLAUDE.md: logs never contain attachment names).

use std::time::Instant;

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use mellow_protocol::{Attachment, Snowflake};

use crate::domain::attachments::{AttachmentFileKind, ReadableAttachment};
use crate::domain::clock::now_unix_milliseconds;
use crate::domain::message_rules::{PLAYABLE_MEDIA_TYPES, PREVIEWABLE_IMAGE_TYPES};
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

/// `GET /api/v1/attachments/{id}`: the stored name only travels in `Content-Disposition`. A
/// matching `If-None-Match` gets 304 once the reader is authorised, without reading the file.
pub(crate) async fn download(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(attachment_id): ApiPath<Snowflake>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    serve(
        &state,
        user.id,
        attachment_id,
        &headers,
        AttachmentFileKind::Original,
    )
    .await
}

/// `GET /api/v1/attachments/{id}/preview`: the picture's smaller copy for the message list, or
/// the attachment itself when it has none (it already fits, or was uploaded before previews).
/// Same checks, headers and entity tags as `download`.
pub(crate) async fn download_preview(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(attachment_id): ApiPath<Snowflake>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    serve(
        &state,
        user.id,
        attachment_id,
        &headers,
        AttachmentFileKind::Preview,
    )
    .await
}

async fn serve(
    state: &AppState,
    user_id: Snowflake,
    attachment_id: Snowflake,
    headers: &HeaderMap,
    kind: AttachmentFileKind,
) -> Result<Response, ApiError> {
    if !state.rate_limiter.record_event(
        &format!("attachment-download:{user_id}"),
        ATTACHMENT_DOWNLOADS_PER_USER,
        Instant::now(),
    ) {
        return Err(ApiError::rate_limited());
    }
    let attachment = state
        .attachments
        .find_readable(user_id, attachment_id, kind)
        .await?;
    serve_readable(state, attachment, headers).await
}

/// `GET /api/v1/channels/{channel_id}/messages/{message_id}/link-previews/{preview_id}/image`:
/// a link preview's picture (ADR-0041), with the same checks, limits and headers as attachment
/// previews.
pub(crate) async fn download_link_preview_image(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((channel_id, message_id, preview_id)): ApiPath<(Snowflake, Snowflake, Snowflake)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if !state.rate_limiter.record_event(
        &format!("attachment-download:{}", user.id),
        ATTACHMENT_DOWNLOADS_PER_USER,
        Instant::now(),
    ) {
        return Err(ApiError::rate_limited());
    }
    let image = state
        .attachments
        .find_readable_link_preview_image(user.id, channel_id, message_id, preview_id)
        .await?;
    serve_readable(&state, image, &headers).await
}

async fn serve_readable(
    state: &AppState,
    attachment: ReadableAttachment,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let entity_tag = HeaderValue::from_str(&attachment.entity_tag())
        .map_err(|error| ApiError::internal("an attachment entity tag", &error))?;
    // Revalidated on every view: a shared computer keeps no conversation's files usable after
    // its user signed out.
    let cache_control = (
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    if headers
        .get(header::IF_NONE_MATCH)
        .is_some_and(|condition| is_entity_tag_matched(condition, &entity_tag))
    {
        return Ok((
            StatusCode::NOT_MODIFIED,
            [(header::ETAG, entity_tag), cache_control],
        )
            .into_response());
    }
    let download = state.attachments.download(attachment).await?;
    let is_inline = PREVIEWABLE_IMAGE_TYPES.contains(&download.content_type.as_str())
        || PLAYABLE_MEDIA_TYPES.contains(&download.content_type.as_str());
    let disposition = format!(
        "{}; filename*=UTF-8''{}",
        if is_inline { "inline" } else { "attachment" },
        percent_encode_file_name(&download.file_name)
    );
    let content_type = HeaderValue::from_str(&download.content_type)
        .map_err(|error| ApiError::internal("an attachment type", &error))?;
    let disposition = HeaderValue::from_str(&disposition)
        .map_err(|error| ApiError::internal("an attachment name", &error))?;
    let requested_range = requested_range(headers, &entity_tag, download.bytes.len());
    let (status, bytes, content_range) = match requested_range {
        None | Some(RequestedRange::Ignored) => (StatusCode::OK, download.bytes, None),
        Some(RequestedRange::Unsatisfiable) => {
            let total = download.bytes.len();
            return Ok((
                StatusCode::RANGE_NOT_SATISFIABLE,
                [
                    (
                        header::CONTENT_RANGE,
                        content_range_value(&format!("bytes */{total}"))?,
                    ),
                    (header::ETAG, entity_tag),
                    cache_control,
                ],
            )
                .into_response());
        }
        Some(RequestedRange::Satisfiable(range)) => {
            let (part, content_range) = part_of(&download.bytes, range)?;
            (StatusCode::PARTIAL_CONTENT, part, Some(content_range))
        }
    };
    let body_length = bytes.len();
    let mut response = (
        status,
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
            (header::ETAG, entity_tag),
            (header::ACCEPT_RANGES, HeaderValue::from_static("bytes")),
            cache_control,
        ],
        body_keeping_permit(bytes, download.permit),
    )
        .into_response();
    // A known length lets players size their buffer (a stream body sends none by itself).
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from(body_length));
    if let Some(content_range) = content_range {
        response
            .headers_mut()
            .insert(header::CONTENT_RANGE, content_range);
    }
    Ok(response)
}

/// The range a media player asks for (Safari plays nothing otherwise, others cannot seek). An
/// `If-Range` naming another version gets the whole file, as RFC 9110 §13.1.5 asks.
fn requested_range(
    headers: &HeaderMap,
    entity_tag: &HeaderValue,
    total: usize,
) -> Option<RequestedRange> {
    let is_range_applicable = headers
        .get(header::IF_RANGE)
        .is_none_or(|condition| condition.as_bytes() == entity_tag.as_bytes());
    headers
        .get(header::RANGE)
        .filter(|_| is_range_applicable)
        .and_then(|value| value.to_str().ok())
        .map(|value| byte_range(value, total))
}

/// A copy of the bytes of `range` and their `Content-Range`; the whole file is dropped once the
/// response is built (a part kept in place would hold the whole buffer until a slow player reads it).
fn part_of(
    bytes: &[u8],
    range: std::ops::RangeInclusive<usize>,
) -> Result<(Vec<u8>, HeaderValue), ApiError> {
    let total = bytes.len();
    let content_range =
        content_range_value(&format!("bytes {}-{}/{total}", range.start(), range.end()))?;
    let part = bytes
        .get(range)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| ApiError::internal("a byte range", &"outside its file"))?;
    Ok((part, content_range))
}

fn content_range_value(value: &str) -> Result<HeaderValue, ApiError> {
    HeaderValue::from_str(value).map_err(|error| ApiError::internal("a content range", &error))
}

/// The longest part one range answer carries: a player that stops reading (paused, or only
/// after the metadata) then holds a download slot for a short answer, not a whole file.
const MAXIMUM_RANGE_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// What a `Range` header asks of a file of `total` bytes.
#[derive(Debug, PartialEq, Eq)]
enum RequestedRange {
    /// Served whole with 200: not a single byte range this route understands (other units,
    /// several ranges, malformed), as RFC 9110 §14.2 allows.
    Ignored,
    /// Answered 416: the range starts past the end, or the file is empty.
    Unsatisfiable,
    Satisfiable(std::ops::RangeInclusive<usize>),
}

/// `bytes=first-last`, `bytes=first-` or `bytes=-suffix_length` (RFC 9110 §14.1.2), one range
/// only; a last position past the end is clamped to it, and an answer to at most
/// `MAXIMUM_RANGE_RESPONSE_BYTES` (the player asks for the next part itself).
fn byte_range(header_value: &str, total: usize) -> RequestedRange {
    let header_value = header_value.trim();
    let Some(spec) = header_value
        .get(..6)
        .filter(|unit| unit.eq_ignore_ascii_case("bytes="))
        .and_then(|_| header_value.get(6..))
    else {
        return RequestedRange::Ignored;
    };
    let Some((first, last)) = spec.trim().split_once('-') else {
        return RequestedRange::Ignored;
    };
    let parse = |digits: &str| {
        (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| digits.parse::<usize>().ok())
            .flatten()
    };
    let (first, last) = match (first.is_empty(), last.is_empty()) {
        (true, false) => {
            let Some(suffix_length) = parse(last) else {
                return RequestedRange::Ignored;
            };
            if suffix_length == 0 || total == 0 {
                return RequestedRange::Unsatisfiable;
            }
            (total.saturating_sub(suffix_length), total - 1)
        }
        (false, _) => {
            let Some(first) = parse(first) else {
                return RequestedRange::Ignored;
            };
            let last = if last.is_empty() {
                usize::MAX
            } else {
                match parse(last) {
                    Some(last) if last >= first => last,
                    _ => return RequestedRange::Ignored,
                }
            };
            if first >= total {
                return RequestedRange::Unsatisfiable;
            }
            (first, last.min(total - 1))
        }
        (true, true) => return RequestedRange::Ignored,
    };
    let longest_last = first.saturating_add(MAXIMUM_RANGE_RESPONSE_BYTES - 1);
    RequestedRange::Satisfiable(first..=last.min(longest_last))
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

/// `If-None-Match` (RFC 9110 §13.1.2): `*`, or a list holding the tag; weak comparison, so a
/// `W/` prefix a proxy added still matches.
fn is_entity_tag_matched(condition: &HeaderValue, entity_tag: &HeaderValue) -> bool {
    let Ok(condition) = condition.to_str() else {
        return false;
    };
    let entity_tag = entity_tag.as_bytes();
    condition.split(',').map(str::trim).any(|candidate| {
        candidate == "*" || candidate.trim_start_matches("W/").as_bytes() == entity_tag
    })
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
mod tests;
