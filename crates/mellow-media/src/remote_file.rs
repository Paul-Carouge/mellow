//! A track read over HTTPS in byte ranges while it plays (ADR-0063). No cookie, no credential and
//! no Mellow header goes with a request, and a redirect is followed only to an allowed address.
//! A thread of its own downloads ahead of the reader, so the clip player's thread, which feeds
//! the sound output every few milliseconds, never waits on the network while the next bytes are
//! already here; a seek outside what is held starts a new range.

use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use crate::clip_decoder::ClipError;
use crate::remote_media::{RemoteTrack, is_allowed_stream_url};

/// A track longer than this is refused: about ten hours of 720p.
const MAXIMUM_TRACK_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// Each request asks for at most this much at first: YouTube slows down open-ended ranges, and
/// refuses ranges above a limit of each track's own (seen from about 1 MB on a Short's sound),
/// so a refused range is asked again at half the size, down to [`MINIMUM_RANGE_BYTES`].
const RANGE_BYTES: u64 = 8 * 1024 * 1024;
const MINIMUM_RANGE_BYTES: u64 = 128 * 1024;
/// Downloaded ahead of the reader before the download waits.
const MAXIMUM_BYTES_AHEAD: u64 = 16 * 1024 * 1024;
/// Kept behind the reader, for the small moves back a demuxer makes.
const KEPT_BYTES_BEHIND: u64 = 2 * 1024 * 1024;
/// A forward move within this much past what is held waits for the download instead of
/// starting a new range.
const FORWARD_WAIT_BYTES: u64 = 512 * 1024;
const READ_PIECE_BYTES: usize = 64 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Each read of an answer, not the whole answer.
const READ_TIMEOUT: Duration = Duration::from_secs(20);
const MAXIMUM_REDIRECTS: usize = 5;
/// A failed range is asked again this many times before the track fails.
const RANGE_ATTEMPTS: u32 = 3;

fn client() -> Option<&'static reqwest::blocking::Client> {
    static CLIENT: OnceLock<Option<reqwest::blocking::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::blocking::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(READ_TIMEOUT)
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() >= MAXIMUM_REDIRECTS
                        || !is_allowed_stream_url(attempt.url().as_str())
                    {
                        attempt.stop()
                    } else {
                        attempt.follow()
                    }
                }))
                .referer(false)
                .build()
                .map_err(|error| tracing::warn!(%error, "cannot build the stream client"))
                .ok()
        })
        .as_ref()
}

struct Window {
    /// Where `bytes` starts in the track.
    start: u64,
    bytes: VecDeque<u8>,
    /// Where the reader is.
    reader: u64,
    /// Bumped by each new range; a download of an older one stops.
    generation: u64,
    failed: bool,
    is_closed: bool,
}

impl Window {
    fn end(&self) -> u64 {
        self.start + self.bytes.len() as u64
    }
}

struct Shared {
    window: Mutex<Window>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Window> {
        self.window.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A track being read: [`Read`] and [`Seek`] over the network, for a demuxer.
pub(crate) struct RemoteFile {
    shared: Arc<Shared>,
    position: u64,
    length: u64,
}

impl RemoteFile {
    /// Asks for the first range (which says the track's length) and starts the download.
    pub(crate) fn open(track: &RemoteTrack) -> Result<Self, ClipError> {
        let client = client().ok_or(ClipError::Unsupported)?;
        let mut range_bytes = RANGE_BYTES;
        let first = request_range(client, track, 0, &mut range_bytes)?;
        let length = total_length(&first).ok_or(ClipError::Corrupt)?;
        if length == 0 || length > MAXIMUM_TRACK_BYTES {
            return Err(ClipError::Unsupported);
        }
        let shared = Arc::new(Shared {
            window: Mutex::new(Window {
                start: 0,
                bytes: VecDeque::new(),
                reader: 0,
                generation: 0,
                failed: false,
                is_closed: false,
            }),
            changed: Condvar::new(),
        });
        let thread_shared = Arc::clone(&shared);
        let thread_track = track.clone();
        std::thread::Builder::new()
            .name("mellow-stream".into())
            .spawn(move || {
                download(
                    client,
                    &thread_track,
                    length,
                    &thread_shared,
                    Some(first),
                    range_bytes,
                );
            })
            .map_err(|_| ClipError::Corrupt)?;
        Ok(Self {
            shared,
            position: 0,
            length,
        })
    }

    pub(crate) fn length(&self) -> u64 {
        self.length
    }
}

impl Drop for RemoteFile {
    fn drop(&mut self) {
        self.shared.lock().is_closed = true;
        self.shared.changed.notify_all();
    }
}

/// Asks for the range from `start`, `range_bytes` long, halving it while it is refused.
fn request_range(
    client: &reqwest::blocking::Client,
    track: &RemoteTrack,
    start: u64,
    range_bytes: &mut u64,
) -> Result<reqwest::blocking::Response, ClipError> {
    loop {
        let end = start + *range_bytes - 1;
        let mut request = client
            .get(&track.url)
            .header(reqwest::header::RANGE, format!("bytes={start}-{end}"));
        for (name, value) in &track.headers {
            request = request.header(name.as_str(), value.as_str());
        }
        let response = request.send().map_err(|error| {
            tracing::debug!(error = %error.without_url(), "a stream range failed");
            ClipError::Corrupt
        })?;
        if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            return Ok(response);
        }
        tracing::debug!(status = %response.status(), bytes = *range_bytes, "a stream range was refused");
        if response.status() != reqwest::StatusCode::FORBIDDEN
            || *range_bytes / 2 < MINIMUM_RANGE_BYTES
        {
            return Err(ClipError::Corrupt);
        }
        *range_bytes /= 2;
    }
}

/// The total of `Content-Range: bytes a-b/total`.
fn total_length(response: &reqwest::blocking::Response) -> Option<u64> {
    let range = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    range.rsplit_once('/')?.1.trim().parse().ok()
}

/// The download thread: fills the window ahead of the reader, range after range, until the
/// file is dropped.
fn download(
    client: &reqwest::blocking::Client,
    track: &RemoteTrack,
    length: u64,
    shared: &Shared,
    mut first: Option<reqwest::blocking::Response>,
    mut range_bytes: u64,
) {
    let mut piece = vec![0_u8; READ_PIECE_BYTES];
    let mut failures = 0;
    loop {
        // Wait until there is room ahead of the reader, or a new range is wanted.
        let (generation, from) = {
            let mut window = shared.lock();
            loop {
                if window.is_closed {
                    return;
                }
                let has_room = window.end() < length
                    && window.end().saturating_sub(window.reader) < MAXIMUM_BYTES_AHEAD;
                if has_room && !window.failed {
                    break;
                }
                window = shared
                    .changed
                    .wait(window)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            (window.generation, window.end())
        };
        let first_answer = first.take().filter(|_| from == 0 && generation == 0);
        let mut response = match first_answer {
            Some(response) => response,
            None => {
                if let Ok(response) = request_range(client, track, from, &mut range_bytes) {
                    response
                } else {
                    failures += 1;
                    if failures >= RANGE_ATTEMPTS {
                        shared.lock().failed = true;
                        shared.changed.notify_all();
                        return;
                    }
                    continue;
                }
            }
        };
        let mut received = 0_u64;
        loop {
            let count = match response.read(&mut piece) {
                Ok(count) if count > 0 => count,
                // The range's end, or a read that failed: the next range takes over.
                _ => break,
            };
            received += count as u64;
            let mut window = shared.lock();
            if window.is_closed {
                return;
            }
            if window.generation != generation {
                break;
            }
            window.bytes.extend(&piece[..count]);
            // What the reader left far behind is dropped.
            let kept_from = window.reader.saturating_sub(KEPT_BYTES_BEHIND);
            if kept_from > window.start {
                let drop_count = usize::try_from(kept_from - window.start)
                    .unwrap_or(usize::MAX)
                    .min(window.bytes.len());
                window.bytes.drain(..drop_count);
                window.start += drop_count as u64;
            }
            drop(window);
            shared.changed.notify_all();
            if received >= range_bytes {
                break;
            }
        }
        failures = if received > 0 { 0 } else { failures + 1 };
        if failures >= RANGE_ATTEMPTS {
            shared.lock().failed = true;
            shared.changed.notify_all();
            return;
        }
    }
}

impl Read for RemoteFile {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.position >= self.length || buffer.is_empty() {
            return Ok(0);
        }
        let mut window = self.shared.lock();
        window.reader = self.position;
        let is_held_or_coming =
            self.position >= window.start && self.position <= window.end() + FORWARD_WAIT_BYTES;
        if !is_held_or_coming {
            // A seek away: a new range from here.
            window.start = self.position;
            window.bytes.clear();
            window.generation += 1;
            window.failed = false;
        }
        self.shared.changed.notify_all();
        loop {
            if window.is_closed {
                return Ok(0);
            }
            if self.position < window.end() {
                let offset = usize::try_from(self.position - window.start).unwrap_or(0);
                let count = buffer.len().min(window.bytes.len() - offset);
                let (front, back) = window.bytes.as_slices();
                for (index, target) in buffer[..count].iter_mut().enumerate() {
                    let at = offset + index;
                    *target = if at < front.len() {
                        front[at]
                    } else {
                        back[at - front.len()]
                    };
                }
                self.position += count as u64;
                window.reader = self.position;
                drop(window);
                // Room was made ahead.
                self.shared.changed.notify_all();
                return Ok(count);
            }
            if window.failed {
                return Err(std::io::Error::other("the stream stopped"));
            }
            window = self
                .shared
                .changed
                .wait(window)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl Seek for RemoteFile {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let target = match to {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(offset) => self.length.checked_add_signed(offset),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
        }
        .filter(|target| *target <= self.length)
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        self.position = target;
        Ok(target)
    }
}
