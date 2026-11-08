//! Files: authenticated downloads (pictures, attachments, sounds) with entity tags and a byte cap,
//! and uploads sent as the raw body with progress reports and a size-based deadline.
//!
//! An upload or a download is cancelled by dropping its future (or aborting the task running it):
//! the connection is closed mid-transfer and an unfinished upload is never stored.

use std::convert::Infallible;
use std::time::Duration;

use futures_util::{Stream, StreamExt, stream};
use mellow_protocol::{
    Attachment, BannerCropQuery, Channel, CreateEmojiQuery, CreateSoundQuery, CurrentUser,
    CustomEmoji, Guild, IconCropQuery, Role, Snowflake, SoundboardSound,
};
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, IF_NONE_MATCH, RANGE};
use reqwest::{Method, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;

use super::{RestClient, percent_encode};
use crate::error::ClientError;

/// What any transfer gets, for the instance to answer (pictures are re-encoded before it does).
const TRANSFER_BASE_TIMEOUT: Duration = Duration::from_secs(60);
/// About 256 kbit/s: a transfer slower than this on average is taken as stalled. The same rule as
/// the web client's `uploadTimeoutMilliseconds`.
const SLOWEST_TRANSFER_BYTES_PER_MILLISECOND: u64 = 32;
/// A download whose answer or next byte takes longer than this is given up: its deadline comes
/// from the caller's cap, not the file's size, so it alone would let a dead connection hang for
/// close to an hour.
const STALLED_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);
/// Small enough for smooth progress reports, large enough to keep the per-chunk cost negligible.
const UPLOAD_CHUNK_BYTES: usize = 64 * 1024;
const GENERIC_UPLOAD_CONTENT_TYPE: &str = "application/octet-stream";
/// Soundboard sounds are sent as the canonical WAV (ADR-0032).
const SOUND_UPLOAD_CONTENT_TYPE: &str = "audio/wav";

/// How long a transfer of `bytes` may take before it is given up as stalled. The deadline grows
/// with the size, so a large file on a slow link is not cut off by the 20 s call timeout.
fn transfer_timeout(bytes: u64) -> Duration {
    TRANSFER_BASE_TIMEOUT
        + Duration::from_millis(bytes.div_ceil(SLOWEST_TRANSFER_BYTES_PER_MILLISECOND))
}

/// A file the instance serves to signed-in users who may see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DownloadableFile {
    /// An avatar or banner, as stored (possibly animated). Immutable: cache it for good.
    ProfileImage(Snowflake),
    /// The first frame of an animated avatar or banner.
    ProfileImageStill(Snowflake),
    /// A guild, channel or role icon, or a custom emoji. Immutable.
    Icon(Snowflake),
    /// The first frame of an animated icon or emoji.
    IconStill(Snowflake),
    /// An attachment as uploaded (pictures as processed by ADR-0033). Revalidate with its entity
    /// tag on each view.
    Attachment(Snowflake),
    /// The small preview of a picture or video attachment.
    AttachmentPreview(Snowflake),
    /// The picture of a link preview.
    LinkPreviewImage {
        channel_id: Snowflake,
        message_id: Snowflake,
        preview_id: Snowflake,
    },
    /// A soundboard sound, as a canonical WAV.
    Sound(Snowflake),
    /// The site icon of a profile link (ADR-0050).
    LinkIcon(Snowflake),
}

impl DownloadableFile {
    fn path(self) -> String {
        match self {
            Self::ProfileImage(id) => format!("api/v1/profile-images/{id}"),
            Self::ProfileImageStill(id) => format!("api/v1/profile-images/{id}/still"),
            Self::Icon(id) => format!("api/v1/icons/{id}"),
            Self::IconStill(id) => format!("api/v1/icons/{id}/still"),
            Self::Attachment(id) => format!("api/v1/attachments/{id}"),
            Self::AttachmentPreview(id) => format!("api/v1/attachments/{id}/preview"),
            Self::LinkPreviewImage {
                channel_id,
                message_id,
                preview_id,
            } => format!(
                "api/v1/channels/{channel_id}/messages/{message_id}/link-previews/{preview_id}/image"
            ),
            Self::Sound(id) => format!("api/v1/sounds/{id}"),
            Self::LinkIcon(id) => format!("api/v1/link-icons/{id}"),
        }
    }
}

/// The answer to a download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Download {
    /// The cached copy whose entity tag was given is still current.
    NotModified,
    Fetched(DownloadedFile),
}

#[derive(Clone, PartialEq, Eq)]
pub struct DownloadedFile {
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
    /// Send it back with the next download of the same file to skip an unchanged body.
    pub entity_tag: Option<String>,
}

/// The bytes are a conversation's file: never printed, only counted.
impl std::fmt::Debug for DownloadedFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DownloadedFile")
            .field("byte_count", &self.bytes.len())
            .field("content_type", &self.content_type)
            .field("entity_tag", &self.entity_tag)
            .finish()
    }
}

/// How much of an upload has been handed to the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UploadProgress {
    pub sent_bytes: u64,
    pub total_bytes: u64,
}

/// The body in chunks, reporting each one as it is handed to the connection.
fn chunks_reporting_progress(
    bytes: Vec<u8>,
    on_progress: impl Fn(UploadProgress) + Send + 'static,
) -> impl Stream<Item = Result<Vec<u8>, Infallible>> + Send + 'static {
    let total_bytes = bytes.len() as u64;
    stream::iter((0..bytes.len()).step_by(UPLOAD_CHUNK_BYTES)).map(move |start| {
        let end = bytes.len().min(start + UPLOAD_CHUNK_BYTES);
        let chunk = bytes.get(start..end).unwrap_or_default().to_vec();
        on_progress(UploadProgress {
            sent_bytes: end as u64,
            total_bytes,
        });
        Ok(chunk)
    })
}

impl RestClient {
    /// Downloads a file, or answers `NotModified` when `cached_entity_tag` is still current. The
    /// body is read up to `maximum_bytes` only, so a hostile instance cannot make the client
    /// buffer more than the caller allows.
    ///
    /// # Errors
    /// Network or API errors (`NotFound` also when the user may not see the file, `RateLimited`
    /// past the instance's file request budget); `FileTooLarge` past `maximum_bytes`;
    /// `TransferStalled` when the answer or its body stops arriving.
    pub async fn download(
        &self,
        file: DownloadableFile,
        cached_entity_tag: Option<&str>,
        maximum_bytes: usize,
    ) -> Result<Download, ClientError> {
        let mut builder = self
            .request(Method::GET, &file.path(), true)?
            .timeout(transfer_timeout(maximum_bytes as u64));
        if let Some(entity_tag) = cached_entity_tag {
            builder = builder.header(IF_NONE_MATCH, entity_tag);
        }
        let response = tokio::time::timeout(STALLED_DOWNLOAD_TIMEOUT, builder.send())
            .await
            .map_err(|_| ClientError::TransferStalled)?
            .map_err(ClientError::Network)?;
        if response.status() == StatusCode::NOT_MODIFIED {
            return Ok(Download::NotModified);
        }
        let response = Self::api_error_unless_success(response).await?;
        let header_text = |name| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let content_type = header_text(CONTENT_TYPE);
        let entity_tag = header_text(ETAG);
        let bytes = Self::body_within(response, maximum_bytes, STALLED_DOWNLOAD_TIMEOUT)
            .await?
            .ok_or(ClientError::FileTooLarge)?;
        Ok(Download::Fetched(DownloadedFile {
            bytes,
            content_type,
            entity_tag,
        }))
    }

    /// Downloads a whole file through byte ranges (ADR-0038), as browsers fetch a played sound:
    /// the instance answers at most 8 MiB per range, so each answer holds a download slot
    /// briefly, and the next range is asked for until the file is read. Never more than
    /// `maximum_bytes` is read, nor asked for once the file's announced size passes it.
    ///
    /// # Errors
    /// As [`Self::download`]; `FileTooLarge` past `maximum_bytes`; `UnexpectedResponse` for a
    /// range answer that is not the part asked for.
    pub async fn download_in_ranges(
        &self,
        file: DownloadableFile,
        maximum_bytes: usize,
    ) -> Result<Vec<u8>, ClientError> {
        let mut bytes: Vec<u8> = Vec::new();
        loop {
            let first_byte = bytes.len() as u64;
            let room = maximum_bytes.saturating_sub(bytes.len());
            let builder = self
                .request(Method::GET, &file.path(), true)?
                .timeout(transfer_timeout(room as u64))
                .header(RANGE, format!("bytes={first_byte}-"));
            let response = tokio::time::timeout(STALLED_DOWNLOAD_TIMEOUT, builder.send())
                .await
                .map_err(|_| ClientError::TransferStalled)?
                .map_err(ClientError::Network)?;
            let response = Self::api_error_unless_success(response).await?;
            if response.status() != StatusCode::PARTIAL_CONTENT {
                // The whole file (an instance without ranges, or an `If-Range` mismatch): only
                // a first request may take it.
                if first_byte != 0 {
                    return Err(unexpected_range("the whole file after a range"));
                }
                return Self::body_within(response, maximum_bytes, STALLED_DOWNLOAD_TIMEOUT)
                    .await?
                    .ok_or(ClientError::FileTooLarge);
            }
            let content_range = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_content_range)
                .filter(|range| range.first_byte == first_byte)
                .ok_or_else(|| unexpected_range("a range other than the one asked for"))?;
            let total_bytes = content_range.total_bytes;
            if total_bytes > maximum_bytes as u64 {
                return Err(ClientError::FileTooLarge);
            }
            let body = Self::body_within(response, room, STALLED_DOWNLOAD_TIMEOUT)
                .await?
                .ok_or(ClientError::FileTooLarge)?;
            if body.len() as u64 != content_range.length() {
                return Err(unexpected_range("a part of another length than announced"));
            }
            bytes.extend_from_slice(&body);
            let read = bytes.len() as u64;
            // Every part holds at least one byte (`parse_content_range`), so the loop advances.
            if read >= total_bytes {
                return Ok(bytes);
            }
        }
    }

    /// Sends `bytes` as the raw body, with a deadline that grows with its size.
    async fn send_file<T: DeserializeOwned>(
        builder: RequestBuilder,
        bytes: Vec<u8>,
        content_type: &'static str,
        on_progress: impl Fn(UploadProgress) + Send + 'static,
    ) -> Result<T, ClientError> {
        let total_bytes = bytes.len() as u64;
        let builder = builder
            .timeout(transfer_timeout(total_bytes))
            .header(CONTENT_TYPE, content_type)
            // Known up front, so the instance refuses an oversized file before it is sent.
            .header(CONTENT_LENGTH, total_bytes)
            .body(reqwest::Body::wrap_stream(chunks_reporting_progress(
                bytes,
                on_progress,
            )));
        let response = builder.send().await.map_err(ClientError::Network)?;
        // The instance's body limit answers without the usual error body.
        if response.status() == StatusCode::PAYLOAD_TOO_LARGE {
            return Err(ClientError::FileTooLarge);
        }
        let response = Self::api_error_unless_success(response).await?;
        Self::decode_json(response).await
    }

    /// Uploads one file to a channel; send its id in a message within the hour (ADR-0020).
    ///
    /// # Errors
    /// Network or API errors; `FileTooLarge` over the instance's size limit.
    pub async fn upload_attachment(
        &self,
        channel_id: Snowflake,
        file_name: &str,
        bytes: Vec<u8>,
    ) -> Result<Attachment, ClientError> {
        self.upload_attachment_reporting_progress(channel_id, file_name, bytes, |_| {})
            .await
    }

    /// [`Self::upload_attachment`], calling `on_progress` as the file goes out. Drop the future to
    /// cancel the upload.
    ///
    /// # Errors
    /// Network or API errors; `FileTooLarge` over the instance's size limit; `Network` when the
    /// upload outlives its size-based deadline.
    pub async fn upload_attachment_reporting_progress(
        &self,
        channel_id: Snowflake,
        file_name: &str,
        bytes: Vec<u8>,
        on_progress: impl Fn(UploadProgress) + Send + 'static,
    ) -> Result<Attachment, ClientError> {
        let builder = self
            .request(
                Method::POST,
                &format!("api/v1/channels/{channel_id}/attachments"),
                true,
            )?
            // In a header, never in the URL: proxy logs must not hold file names.
            .header("x-mellow-file-name", percent_encode(file_name));
        Self::send_file(builder, bytes, GENERIC_UPLOAD_CONTENT_TYPE, on_progress).await
    }

    /// The picture as stored by the user; the instance crops (`crop`, or the centred square),
    /// resizes and compresses it before answering (ADR-0021, ADR-0033).
    ///
    /// # Errors
    /// Network or API errors; `ValidationFailed` for an unreadable picture or a partial crop;
    /// `FileTooLarge`.
    pub async fn set_avatar(
        &self,
        picture: Vec<u8>,
        crop: &IconCropQuery,
    ) -> Result<CurrentUser, ClientError> {
        let builder = self
            .request(Method::PUT, "api/v1/users/@me/avatar", true)?
            .query(crop);
        Self::send_file(builder, picture, GENERIC_UPLOAD_CONTENT_TYPE, |_| {}).await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn remove_avatar(&self) -> Result<CurrentUser, ClientError> {
        self.call(Method::DELETE, "api/v1/users/@me/avatar").await
    }

    /// Like [`Self::set_avatar`], for the profile card's 5:2 band.
    ///
    /// # Errors
    /// Network or API errors; `ValidationFailed` for an unreadable picture or a partial crop;
    /// `FileTooLarge`.
    pub async fn set_banner(
        &self,
        picture: Vec<u8>,
        crop: &BannerCropQuery,
    ) -> Result<CurrentUser, ClientError> {
        let builder = self
            .request(Method::PUT, "api/v1/users/@me/banner", true)?
            .query(crop);
        Self::send_file(builder, picture, GENERIC_UPLOAD_CONTENT_TYPE, |_| {}).await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn remove_banner(&self) -> Result<CurrentUser, ClientError> {
        self.call(Method::DELETE, "api/v1/users/@me/banner").await
    }

    /// Animated pictures stay animated.
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_GUILD`; `FileTooLarge`.
    pub async fn set_guild_icon(
        &self,
        guild_id: Snowflake,
        picture: Vec<u8>,
        crop: &IconCropQuery,
    ) -> Result<Guild, ClientError> {
        let builder = self
            .request(Method::PUT, &format!("api/v1/guilds/{guild_id}/icon"), true)?
            .query(crop);
        Self::send_file(builder, picture, GENERIC_UPLOAD_CONTENT_TYPE, |_| {}).await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_GUILD`.
    pub async fn remove_guild_icon(&self, guild_id: Snowflake) -> Result<Guild, ClientError> {
        self.call(Method::DELETE, &format!("api/v1/guilds/{guild_id}/icon"))
            .await
    }

    /// Text channels only; always stored still.
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_CHANNELS`; `FileTooLarge`.
    pub async fn set_channel_icon(
        &self,
        channel_id: Snowflake,
        picture: Vec<u8>,
        crop: &IconCropQuery,
    ) -> Result<Channel, ClientError> {
        let builder = self
            .request(
                Method::PUT,
                &format!("api/v1/channels/{channel_id}/icon"),
                true,
            )?
            .query(crop);
        Self::send_file(builder, picture, GENERIC_UPLOAD_CONTENT_TYPE, |_| {}).await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_CHANNELS`.
    pub async fn remove_channel_icon(&self, channel_id: Snowflake) -> Result<Channel, ClientError> {
        self.call(
            Method::DELETE,
            &format!("api/v1/channels/{channel_id}/icon"),
        )
        .await
    }

    /// The instance keeps the picture's centred square (ADR-0028).
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_EMOJIS`; `LimitReached` past
    /// the guild's emoji count; `FileTooLarge` past `MAXIMUM_EMOJI_UPLOAD_BYTES`.
    pub async fn create_emoji(
        &self,
        guild_id: Snowflake,
        name: &str,
        picture: Vec<u8>,
    ) -> Result<CustomEmoji, ClientError> {
        let builder = self
            .request(
                Method::POST,
                &format!("api/v1/guilds/{guild_id}/emojis"),
                true,
            )?
            .query(&CreateEmojiQuery {
                name: name.to_owned(),
            });
        Self::send_file(builder, picture, GENERIC_UPLOAD_CONTENT_TYPE, |_| {}).await
    }

    /// `wav` is the canonical WAV (mono, 48 kHz, 16-bit, ADR-0032).
    ///
    /// # Errors
    /// Network or API errors; `ValidationFailed` for another format or a sound too long;
    /// `FileTooLarge`.
    pub async fn create_sound(
        &self,
        guild_id: Snowflake,
        details: &CreateSoundQuery,
        wav: Vec<u8>,
    ) -> Result<SoundboardSound, ClientError> {
        let builder = self
            .request(
                Method::POST,
                &format!("api/v1/guilds/{guild_id}/sounds"),
                true,
            )?
            .query(details);
        Self::send_file(builder, wav, SOUND_UPLOAD_CONTENT_TYPE, |_| {}).await
    }

    /// The picture replaces any emoji icon (ADR-0050); the instance keeps its centred square
    /// unless `crop` says otherwise.
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` for a role out of the caller's reach;
    /// `FileTooLarge`.
    pub async fn set_role_icon_picture(
        &self,
        guild_id: Snowflake,
        role_id: Snowflake,
        picture: Vec<u8>,
        crop: &IconCropQuery,
    ) -> Result<Role, ClientError> {
        let builder = self
            .request(
                Method::PUT,
                &format!("api/v1/guilds/{guild_id}/roles/{role_id}/icon"),
                true,
            )?
            .query(crop);
        Self::send_file(builder, picture, GENERIC_UPLOAD_CONTENT_TYPE, |_| {}).await
    }
}

/// A `206` answer's `Content-Range: bytes first-last/total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContentRange {
    first_byte: u64,
    last_byte: u64,
    total_bytes: u64,
}

impl ContentRange {
    fn length(self) -> u64 {
        self.last_byte - self.first_byte + 1
    }
}

/// `bytes first-last/total` with `first <= last < total`; anything else is `None`.
fn parse_content_range(header: &str) -> Option<ContentRange> {
    let (range, total) = header.strip_prefix("bytes ")?.split_once('/')?;
    let (first, last) = range.split_once('-')?;
    let range = ContentRange {
        first_byte: first.parse().ok()?,
        last_byte: last.parse().ok()?,
        total_bytes: total.parse().ok()?,
    };
    (range.first_byte <= range.last_byte && range.last_byte < range.total_bytes).then_some(range)
}

fn unexpected_range(what: &str) -> ClientError {
    ClientError::UnexpectedResponse(format!("the instance answered {what}"))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    #[test]
    fn a_content_range_is_read_only_when_it_is_a_part_of_its_file() {
        assert_eq!(
            parse_content_range("bytes 0-8388607/20000000"),
            Some(ContentRange {
                first_byte: 0,
                last_byte: 8_388_607,
                total_bytes: 20_000_000
            })
        );
        assert_eq!(
            parse_content_range("bytes 5-5/6").map(ContentRange::length),
            Some(1)
        );
        for malformed in [
            "bytes */20",
            "bytes 6-5/10",
            "bytes 0-10/10",
            "items 0-1/2",
            "bytes 0-1",
            "bytes -1/2",
        ] {
            assert_eq!(parse_content_range(malformed), None, "{malformed}");
        }
    }

    use super::*;

    #[test]
    fn the_transfer_deadline_grows_with_the_size_like_the_web_clients() {
        assert_eq!(transfer_timeout(0), Duration::from_secs(60));
        // 32 bytes per millisecond: 32 MB take 1 000 s more.
        assert_eq!(
            transfer_timeout(32_000_000),
            Duration::from_secs(60 + 1_000)
        );
        assert_eq!(transfer_timeout(1), Duration::from_millis(60_001));
    }

    #[tokio::test]
    async fn a_body_that_stops_arriving_is_given_up_as_stalled() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let (stop_server, server_stopped) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            let (mut connection, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = connection.read(&mut request).await.unwrap();
            // Promises 100 bytes, sends 10, then keeps the connection open without a word.
            connection
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n0123456789")
                .await
                .unwrap();
            let _ = server_stopped.await;
        });
        let response = reqwest::Client::new().get(address).send().await.unwrap();
        let outcome = RestClient::body_within(response, 1_000, Duration::from_millis(50)).await;
        assert!(
            matches!(outcome, Err(ClientError::TransferStalled)),
            "{outcome:?}"
        );
        drop(stop_server);
    }

    #[tokio::test]
    async fn chunks_cover_the_body_once_and_report_rising_progress() {
        let body: Vec<u8> = (0..=u8::MAX)
            .cycle()
            .take(UPLOAD_CHUNK_BYTES * 2 + 5)
            .collect();
        let reports = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&reports);
        let chunks: Vec<_> = chunks_reporting_progress(body.clone(), move |progress| {
            recorder.lock().unwrap().push(progress.sent_bytes);
        })
        .collect()
        .await;
        let joined: Vec<u8> = chunks.into_iter().flat_map(Result::unwrap).collect();
        assert_eq!(joined, body);
        let total = body.len() as u64;
        assert_eq!(
            *reports.lock().unwrap(),
            [
                UPLOAD_CHUNK_BYTES as u64,
                2 * UPLOAD_CHUNK_BYTES as u64,
                total
            ]
        );
    }

    #[test]
    fn a_downloaded_file_never_prints_its_bytes() {
        let file = DownloadedFile {
            bytes: b"secret holiday plans".to_vec(),
            content_type: None,
            entity_tag: None,
        };
        let printed = format!("{file:?}");
        assert!(!printed.contains("115"), "{printed}");
        assert!(printed.contains("byte_count: 20"), "{printed}");
    }
}
