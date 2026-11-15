//! A YouTube video's separate picture and sound tracks, streamed over HTTPS (ADR-0063). Their
//! addresses come from a third party: only `https://*.googlevideo.com` is ever requested or
//! followed ([`is_allowed_stream_url`]). The reading itself is `remote_file` (Linux only so far).

use crate::video_file::VideoContainer;

/// The only hosts a stream address may name: YouTube's media servers.
const ALLOWED_HOST_SUFFIX: &str = ".googlevideo.com";
/// The only header an extractor may ask to send with a track's requests: it says what the client
/// is, nothing of who reads (never a cookie or a credential). yt-dlp's `Accept` (a browser
/// page's) is left out: googlevideo refuses the ranges it comes with.
const ALLOWED_HEADERS: [&str; 1] = ["user-agent"];
const MAXIMUM_HEADER_VALUE_BYTES: usize = 512;

/// A track to stream: its address, checked by [`RemoteTrack::new`], and its container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteTrack {
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) url: String,
    container: VideoContainer,
    /// Sent with each range: only [`ALLOWED_HEADERS`], names in lower case.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) headers: Vec<(String, String)>,
}

impl RemoteTrack {
    /// `None` unless `url` is `https` on a `*.googlevideo.com` host, without credentials or a
    /// port of its own, and `mime_type` is a `WebM` or MP4 track.
    #[must_use]
    pub fn new(url: &str, mime_type: &str) -> Option<Self> {
        let container = match mime_type.split(';').next()?.trim() {
            "video/webm" | "audio/webm" => VideoContainer::WebM,
            "video/mp4" | "audio/mp4" => VideoContainer::Mp4,
            _ => return None,
        };
        is_allowed_stream_url(url).then(|| Self {
            url: url.to_owned(),
            container,
            headers: Vec::new(),
        })
    }

    /// With the headers an extractor names for the track, keeping only a user agent of plain
    /// visible text; anything else is dropped.
    #[must_use]
    pub fn with_headers<'a>(
        mut self,
        headers: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Self {
        self.headers = headers
            .into_iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value))
            .filter(|(name, value)| {
                ALLOWED_HEADERS.contains(&name.as_str())
                    && value.len() <= MAXIMUM_HEADER_VALUE_BYTES
                    && value
                        .bytes()
                        .all(|byte| byte == b' ' || byte.is_ascii_graphic())
            })
            .map(|(name, value)| (name, value.to_owned()))
            .collect();
        self
    }

    // Only Linux streams a track so far.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) fn container(&self) -> VideoContainer {
        self.container
    }
}

/// Whether a stream address (or a redirect) may be requested.
#[must_use]
pub fn is_allowed_stream_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.port().is_none()
        && parsed.host_str().is_some_and(|host| {
            host.len() > ALLOWED_HOST_SUFFIX.len()
                && host.to_ascii_lowercase().ends_with(ALLOWED_HOST_SUFFIX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_googlevideo_addresses_are_followed() {
        for allowed in [
            "https://rr1---sn-2o25g5-5h.googlevideo.com/videoplayback?id=1",
            "https://RR3---SN-AB.GOOGLEVIDEO.COM/videoplayback",
        ] {
            assert!(is_allowed_stream_url(allowed), "{allowed}");
        }
        for refused in [
            "http://rr1---sn-2o25g5-5h.googlevideo.com/videoplayback",
            "https://googlevideo.com/videoplayback",
            "https://.googlevideo.com/videoplayback",
            "https://evil.com/?.googlevideo.com",
            "https://rr1.googlevideo.com.evil.com/videoplayback",
            "https://user:pass@rr1.googlevideo.com/videoplayback",
            "https://rr1.googlevideo.com:8443/videoplayback",
            "file:///etc/passwd",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "not a url",
        ] {
            assert!(!is_allowed_stream_url(refused), "{refused}");
        }
    }

    #[test]
    fn a_track_names_its_container_from_its_type() {
        const URL: &str = "https://rr1.googlevideo.com/videoplayback";
        assert_eq!(
            RemoteTrack::new(URL, "video/webm; codecs=\"vp09.00.31.08\"")
                .map(|track| track.container()),
            Some(VideoContainer::WebM)
        );
        assert_eq!(
            RemoteTrack::new(URL, "audio/mp4; codecs=\"mp4a.40.2\"").map(|track| track.container()),
            Some(VideoContainer::Mp4)
        );
        assert_eq!(RemoteTrack::new(URL, "video/3gpp"), None);
        let with_headers = RemoteTrack::new(URL, "audio/webm")
            .map(|track| {
                track.with_headers([
                    ("User-Agent", "Mozilla/5.0 (X11; Linux x86_64)"),
                    ("Accept", "*/*"),
                    ("Cookie", "SID=secret"),
                    ("Authorization", "Bearer secret"),
                    ("Accept", "bad\r\nX-Injected: 1"),
                ])
            })
            .map(|track| track.headers);
        assert_eq!(
            with_headers,
            Some(vec![(
                "user-agent".to_owned(),
                "Mozilla/5.0 (X11; Linux x86_64)".to_owned()
            ),])
        );
        assert_eq!(RemoteTrack::new("https://example.com/", "video/webm"), None);
    }
}
