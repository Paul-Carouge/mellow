//! Deadlines on what an HTTP/2 connection's requests wait for from the client.
//!
//! HTTP/1.1 needs none of this: one request at a time, so the socket deadlines of
//! `IdleTimeoutStream` already bound it. HTTP/2 multiplexes, and its flow control lets a client
//! stall a response without ever blocking the socket, while any frame (a PING) keeps the
//! connection's reads alive. So each request is watched on its own: the connection is closed once
//! a response waits longer than `write_stall_timeout` for the client to accept its next chunk, a
//! request body waits longer than `idle_read_timeout` for its next bytes, or no request has been
//! open for `header_read_timeout` (the HTTP/2 counterpart of the HTTP/1.1 header deadline).
//! Closing the connection drops every response body, so the transfer permits they hold come back.

use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::http::{Request, Response, Version};
use http_body::{Frame, SizeHint};
use tokio::time::Instant;

/// Response data goes to the client in pieces this large, so a response makes visible progress
/// (and HTTP/2 buffers at most about two pieces of it) however large the body's own frames are.
const RESPONSE_PIECE_BYTES: usize = 64 * 1024;

/// The client-side waits of one connection's requests, shared by its service and its stream.
#[derive(Debug)]
pub(super) struct RequestProgress {
    header_read_timeout: Duration,
    idle_read_timeout: Duration,
    write_stall_timeout: Duration,
    state: Mutex<RequestProgressState>,
}

#[derive(Debug)]
struct RequestProgressState {
    /// HTTP/1.1 connections are left to the socket deadlines, and upgraded `WebSocket` ones must
    /// not be closed for having no open request.
    has_served_http1_request: bool,
    open_requests: usize,
    /// When the last request opened or ended (or the connection was accepted).
    last_request_change: Instant,
    /// By when each waiting body must hear from the client.
    client_wait_deadlines: HashMap<u64, Instant>,
    next_client_wait_id: u64,
}

impl RequestProgress {
    pub(super) fn new(
        header_read_timeout: Duration,
        idle_read_timeout: Duration,
        write_stall_timeout: Duration,
    ) -> Self {
        Self {
            header_read_timeout,
            idle_read_timeout,
            write_stall_timeout,
            state: Mutex::new(RequestProgressState {
                has_served_http1_request: false,
                open_requests: 0,
                last_request_change: Instant::now(),
                client_wait_deadlines: HashMap::new(),
                next_client_wait_id: 0,
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, RequestProgressState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// When the connection must be closed if nothing changes; `None` for HTTP/1.1.
    pub(super) fn closing_deadline(&self) -> Option<Instant> {
        let state = self.state();
        if state.has_served_http1_request {
            return None;
        }
        let earliest_client_wait = state.client_wait_deadlines.values().min().copied();
        let no_open_request = (state.open_requests == 0)
            .then(|| state.last_request_change + self.header_read_timeout);
        earliest_client_wait
            .into_iter()
            .chain(no_open_request)
            .min()
    }

    /// Every deadline set from now on falls at least this far ahead, so a stream that checks
    /// `closing_deadline` again within this time never misses one set in between.
    pub(super) fn shortest_timeout(&self) -> Duration {
        self.header_read_timeout
            .min(self.idle_read_timeout)
            .min(self.write_stall_timeout)
    }

    fn open_request(self: &Arc<Self>, version: Version) -> OpenRequest {
        let mut state = self.state();
        if version < Version::HTTP_2 {
            state.has_served_http1_request = true;
        }
        state.open_requests += 1;
        state.last_request_change = Instant::now();
        OpenRequest {
            progress: self.clone(),
        }
    }
}

/// Counts as an open request until dropped (with the response body, or the handler's future).
#[derive(Debug)]
struct OpenRequest {
    progress: Arc<RequestProgress>,
}

impl Drop for OpenRequest {
    fn drop(&mut self) {
        let mut state = self.progress.state();
        state.open_requests = state.open_requests.saturating_sub(1);
        state.last_request_change = Instant::now();
    }
}

/// One body's wait on the client, if it is waiting.
#[derive(Debug)]
struct ClientWait {
    progress: Arc<RequestProgress>,
    id: Option<u64>,
}

impl ClientWait {
    fn new(progress: Arc<RequestProgress>) -> Self {
        Self { progress, id: None }
    }

    fn start_or_restart(&mut self, timeout: Duration) {
        let mut state = self.progress.state();
        let id = *self.id.get_or_insert_with(|| {
            state.next_client_wait_id += 1;
            state.next_client_wait_id
        });
        state
            .client_wait_deadlines
            .insert(id, Instant::now() + timeout);
    }

    fn start_unless_waiting(&mut self, timeout: Duration) {
        if self.id.is_none() {
            self.start_or_restart(timeout);
        }
    }

    fn stop(&mut self) {
        if let Some(id) = self.id.take() {
            self.progress.state().client_wait_deadlines.remove(&id);
        }
    }
}

impl Drop for ClientWait {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The request body: waits on the client while the handler waits for bytes that have not come.
struct WatchedRequestBody<Inner> {
    inner: Inner,
    wait: ClientWait,
}

impl<Inner: http_body::Body + Unpin> http_body::Body for WatchedRequestBody<Inner> {
    type Data = Inner::Data;
    type Error = Inner::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Inner::Data>, Inner::Error>>> {
        let this = &mut *self;
        let poll = Pin::new(&mut this.inner).poll_frame(context);
        if poll.is_pending() {
            let timeout = this.wait.progress.idle_read_timeout;
            this.wait.start_unless_waiting(timeout);
        } else {
            this.wait.stop();
        }
        poll
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

/// The response body, cut into pieces: waits on the client from each piece handed over until
/// the next is asked for, but not while the handler is still producing data.
struct WatchedResponseBody {
    inner: Body,
    unsent_piece_remainder: Bytes,
    wait: ClientWait,
    _request: OpenRequest,
}

impl WatchedResponseBody {
    fn next_piece(&mut self) -> Frame<Bytes> {
        let length = self.unsent_piece_remainder.len().min(RESPONSE_PIECE_BYTES);
        let timeout = self.wait.progress.write_stall_timeout;
        self.wait.start_or_restart(timeout);
        Frame::data(self.unsent_piece_remainder.split_to(length))
    }
}

impl http_body::Body for WatchedResponseBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        let this = &mut *self;
        if !this.unsent_piece_remainder.is_empty() {
            return Poll::Ready(Some(Ok(this.next_piece())));
        }
        match Pin::new(&mut this.inner).poll_frame(context) {
            Poll::Ready(Some(Ok(frame))) => match frame.into_data() {
                Ok(data) => {
                    this.unsent_piece_remainder = data;
                    Poll::Ready(Some(Ok(this.next_piece())))
                }
                Err(trailers) => {
                    this.wait.stop();
                    Poll::Ready(Some(Ok(trailers)))
                }
            },
            other => {
                this.wait.stop();
                other
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.unsent_piece_remainder.is_empty() && self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        let unsent = u64::try_from(self.unsent_piece_remainder.len()).unwrap_or(u64::MAX);
        let inner = self.inner.size_hint();
        let mut hint = SizeHint::new();
        hint.set_lower(inner.lower().saturating_add(unsent));
        if let Some(upper) = inner.upper() {
            hint.set_upper(upper.saturating_add(unsent));
        }
        hint
    }
}

/// The connection's service with its requests and response bodies watched.
#[derive(Debug, Clone)]
pub(super) struct WatchedRequestService<Inner> {
    pub(super) inner: Inner,
    pub(super) progress: Arc<RequestProgress>,
}

impl<Inner, RequestBody> tower::Service<Request<RequestBody>> for WatchedRequestService<Inner>
where
    Inner: tower::Service<Request<Body>, Response = Response<Body>, Error = Infallible>,
    Inner::Future: Send + 'static,
    RequestBody: http_body::Body<Data = Bytes> + Unpin + Send + 'static,
    RequestBody::Error: Into<axum::BoxError>,
{
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response<Body>, Infallible>> + Send>>;

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        self.inner.poll_ready(context)
    }

    fn call(&mut self, request: Request<RequestBody>) -> Self::Future {
        let open_request = self.progress.open_request(request.version());
        let wait = ClientWait::new(self.progress.clone());
        let request = request.map(|inner| Body::new(WatchedRequestBody { inner, wait }));
        let response = self.inner.call(request);
        let progress = self.progress.clone();
        Box::pin(async move {
            let response = response.await?;
            let mut wait = ClientWait::new(progress);
            let timeout = wait.progress.write_stall_timeout;
            wait.start_or_restart(timeout);
            Ok(response.map(|inner| {
                Body::new(WatchedResponseBody {
                    inner,
                    unsent_piece_remainder: Bytes::new(),
                    wait,
                    _request: open_request,
                })
            }))
        })
    }
}

#[cfg(test)]
mod tests {
    use http_body::Body as _;

    use super::*;

    const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(30);
    const IDLE_READ_TIMEOUT: Duration = Duration::from_secs(75);
    const WRITE_STALL_TIMEOUT: Duration = Duration::from_secs(20);

    fn progress() -> Arc<RequestProgress> {
        Arc::new(RequestProgress::new(
            HEADER_READ_TIMEOUT,
            IDLE_READ_TIMEOUT,
            WRITE_STALL_TIMEOUT,
        ))
    }

    #[tokio::test(start_paused = true)]
    async fn the_closing_deadline_follows_the_earliest_wait_on_the_client() {
        let progress = progress();
        let accepted = Instant::now();
        assert_eq!(
            progress.closing_deadline(),
            Some(accepted + HEADER_READ_TIMEOUT)
        );

        let request = progress.open_request(Version::HTTP_2);
        assert_eq!(progress.closing_deadline(), None, "a handler is working");
        let mut request_body = ClientWait::new(progress.clone());
        request_body.start_unless_waiting(IDLE_READ_TIMEOUT);
        let mut response_body = ClientWait::new(progress.clone());
        response_body.start_or_restart(WRITE_STALL_TIMEOUT);
        assert_eq!(
            progress.closing_deadline(),
            Some(accepted + WRITE_STALL_TIMEOUT)
        );
        response_body.stop();
        assert_eq!(
            progress.closing_deadline(),
            Some(accepted + IDLE_READ_TIMEOUT)
        );

        tokio::time::advance(Duration::from_secs(5)).await;
        drop(request_body);
        drop(request);
        assert_eq!(
            progress.closing_deadline(),
            Some(accepted + Duration::from_secs(5) + HEADER_READ_TIMEOUT),
            "counted from the end of the last request"
        );
        assert_eq!(progress.shortest_timeout(), WRITE_STALL_TIMEOUT);
    }

    #[tokio::test(start_paused = true)]
    async fn http1_connections_have_no_closing_deadline() {
        let progress = progress();
        let request = progress.open_request(Version::HTTP_11);
        drop(request);
        assert_eq!(progress.closing_deadline(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn large_response_frames_are_handed_over_in_pieces_of_the_same_bytes() {
        let progress = progress();
        let bytes: Vec<u8> = (0..=u8::MAX)
            .cycle()
            .take(RESPONSE_PIECE_BYTES * 2 + 1)
            .collect();
        let mut body = Box::pin(WatchedResponseBody {
            inner: Body::from(bytes.clone()),
            unsent_piece_remainder: Bytes::new(),
            wait: ClientWait::new(progress.clone()),
            _request: progress.open_request(Version::HTTP_2),
        });
        let mut pieces: Vec<Bytes> = Vec::new();
        let mut unsent_bytes = bytes.len();
        while let Some(frame) =
            std::future::poll_fn(|context| body.as_mut().poll_frame(context)).await
        {
            let piece = frame.unwrap().into_data().unwrap();
            unsent_bytes -= piece.len();
            pieces.push(piece);
            assert_eq!(body.size_hint().exact(), u64::try_from(unsent_bytes).ok());
        }
        assert_eq!(
            pieces.iter().map(Bytes::len).collect::<Vec<_>>(),
            [RESPONSE_PIECE_BYTES, RESPONSE_PIECE_BYTES, 1]
        );
        assert_eq!(pieces.concat(), bytes);
        drop(body);
        assert_eq!(
            progress.closing_deadline(),
            Some(Instant::now() + HEADER_READ_TIMEOUT),
            "the finished body neither waits nor keeps its request open"
        );
    }
}
