//! Built-in TLS with ACME certificates over TLS-ALPN-01 (ADR-0013).
//!
//! Facing the internet directly, the server must also bound what a client can hold open: every
//! connection counts against a global and a per-address cap, the TLS handshake and the HTTP
//! headers have deadlines, and a connection that sends nothing for a while is closed.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use axum::http::HeaderMap;
use futures_util::StreamExt as _;
use rustls_acme::{AccountCache, AcmeConfig, CertCache, EventError, OrderError};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{Instant, Sleep};

use crate::config::TlsConfig;
use crate::http::client_address::client_address_key;
use crate::http::{AppState, make_service};
use crate::storage::create_private_directory;

const LETS_ENCRYPT_PRODUCTION_DIRECTORY: &str = "https://acme-v02.api.letsencrypt.org/directory";
const LETS_ENCRYPT_STAGING_DIRECTORY: &str =
    "https://acme-staging-v02.api.letsencrypt.org/directory";
/// Certificate authorities send TLS-ALPN-01 challenges to this port only.
const ACME_CHALLENGE_PORT: u16 = 443;
/// Open HTTP requests get this long to finish once shutdown starts; upgraded `WebSocket`
/// connections are not tracked by the HTTP server and end with the process.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Deadlines and caps for connections from the internet.
#[derive(Debug, Clone, Copy)]
struct ConnectionPolicy {
    /// From TCP accept to the end of the TLS handshake.
    handshake_timeout: Duration,
    /// For each request's headers, including the wait on an idle keep-alive connection.
    header_read_timeout: Duration,
    /// A connection that sends nothing for this long is closed. Above the gateway's 45 s
    /// heartbeat deadline, so live `WebSocket` connections are never cut.
    idle_read_timeout: Duration,
    maximum_connections: usize,
    /// Per IPv4 address or IPv6 /64, as for rate limits; generous for shared NAT addresses.
    maximum_connections_per_client_address: usize,
    /// Per IPv6 /48: one free tunnel-broker /48 holds 65 536 /64s.
    maximum_connections_per_ipv6_site: usize,
    /// A connection whose peer accepts no written byte for this long is closed (slow reads).
    write_stall_timeout: Duration,
}

const PRODUCTION_CONNECTION_POLICY: ConnectionPolicy = ConnectionPolicy {
    handshake_timeout: Duration::from_secs(10),
    header_read_timeout: Duration::from_secs(30),
    idle_read_timeout: Duration::from_secs(75),
    maximum_connections: 10_000,
    maximum_connections_per_client_address: 256,
    maximum_connections_per_ipv6_site: 1_024,
    write_stall_timeout: Duration::from_secs(30),
};

/// Creates the certificate directory, or checks an existing one: it holds private keys, so it
/// must belong to the service user, be closed to everyone else, and be outside the data
/// directory, even through symbolic links.
///
/// # Errors
/// Fails if the directory cannot be created or resolved, belongs to another user, is open to
/// other users, or resolves inside the data directory.
pub(crate) fn prepare_certificate_directory(
    certificate_directory: &Path,
    data_directory: &Path,
) -> Result<()> {
    create_private_directory(certificate_directory).with_context(|| {
        format!(
            "cannot create the certificate directory {}",
            certificate_directory.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let metadata = std::fs::metadata(certificate_directory)
            .with_context(|| format!("cannot read {}", certificate_directory.display()))?;
        if metadata.uid() != rustix::process::geteuid().as_raw() {
            bail!(
                "the certificate directory {} belongs to another user; it must belong to the user running Mellow",
                certificate_directory.display()
            );
        }
        let mode = metadata.permissions().mode();
        if mode & 0o077 != 0 {
            bail!(
                "the certificate directory {} is open to other users (mode {:o}); run `chmod 700` on it",
                certificate_directory.display(),
                mode & 0o777
            );
        }
    }
    let resolve = |path: &Path| {
        std::fs::canonicalize(path).with_context(|| format!("cannot resolve {}", path.display()))
    };
    if resolve(certificate_directory)?.starts_with(resolve(data_directory)?) {
        bail!(
            "the certificate directory {} resolves inside the data directory; backups would carry the TLS private keys",
            certificate_directory.display()
        );
    }
    Ok(())
}

/// Serves HTTPS on `listener` until `shutdown` completes, obtaining and renewing the certificate
/// from Let's Encrypt in the background.
///
/// # Errors
/// Fails if the listener cannot be converted or stops with an I/O error.
pub(crate) async fn serve_tls_until_shutdown(
    listener: TcpListener,
    state: AppState,
    tls: &TlsConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let directory_url = if tls.use_staging_certificates {
        LETS_ENCRYPT_STAGING_DIRECTORY
    } else {
        LETS_ENCRYPT_PRODUCTION_DIRECTORY
    };
    serve_tls_with_policy(
        listener,
        state,
        tls,
        CertificateAuthority {
            directory_url,
            client_tls_config: None,
        },
        PRODUCTION_CONNECTION_POLICY,
        shutdown,
    )
    .await
}

struct CertificateAuthority<'a> {
    directory_url: &'a str,
    /// Trust for the authority's own HTTPS endpoint; `None` trusts the public web roots. Tests
    /// set it to reach a local authority whose certificate no public root signed.
    client_tls_config: Option<Arc<rustls_acme::rustls::ClientConfig>>,
}

async fn serve_tls_with_policy(
    listener: TcpListener,
    state: AppState,
    tls: &TlsConfig,
    certificate_authority: CertificateAuthority<'_>,
    policy: ConnectionPolicy,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let local_address = listener.local_addr().ok();
    let server = axum_server::from_tcp(listener.into_std()?)?;
    let mut acme_config = AcmeConfig::new(&tls.domains);
    if let Some(client_tls_config) = certificate_authority.client_tls_config {
        acme_config = acme_config.client_tls_config(client_tls_config);
    }
    let mut acme_state = acme_config
        .contact(
            tls.contact_email
                .iter()
                .map(|email| format!("mailto:{email}")),
        )
        .cache(PrivateDirectoryCache {
            directory: tls.certificate_directory.clone(),
        })
        .directory(certificate_authority.directory_url)
        .state();
    let acceptor = GuardedAcceptor {
        inner: acme_state.axum_acceptor(acme_state.default_rustls_config()),
        limiter: Arc::new(ConnectionLimiter::new(policy)),
        policy,
    };
    let mut server = server.acceptor(acceptor);
    // Without a timer hyper silently ignores its header read timeout.
    server
        .http_builder()
        .http1()
        .timer(hyper_util::rt::TokioTimer::new())
        .header_read_timeout(policy.header_read_timeout);
    // No HTTP/2 keep-alive pings: the client's automatic answers would count as activity and
    // keep an idle connection alive forever. Mellow clients speak HTTP/1.1 anyway.

    let renewal = tokio::spawn(async move {
        while let Some(event) = acme_state.next().await {
            match event {
                Ok(event) => tracing::info!(?event, "certificate"),
                // rustls-acme retries with backoff; the current certificate keeps serving.
                Err(error) => tracing::error!(
                    reason = certificate_error_category(&error),
                    "certificate request failed"
                ),
            }
        }
    });
    let handle = axum_server::Handle::new();
    tokio::spawn({
        let handle = handle.clone();
        async move {
            shutdown.await;
            handle.graceful_shutdown(Some(GRACEFUL_SHUTDOWN_TIMEOUT));
        }
    });
    if let Some(address) = local_address {
        tracing::info!(%address, domains = ?tls.domains, "listening with TLS");
        if address.port() != ACME_CHALLENGE_PORT {
            tracing::warn!(
                %address,
                "certificates can only be obtained if port 443 reaches this one (TLS-ALPN-01)"
            );
        }
    }
    let result = server
        .handle(handle)
        .serve(make_service(state))
        .await
        .context("HTTPS server stopped with an error");
    renewal.abort();
    result
}

/// Only the kind of failure: the certificate authority's messages can quote the contact email,
/// which logs must never contain.
fn certificate_error_category<CacheError: std::fmt::Debug>(
    error: &EventError<CacheError, CacheError>,
) -> &'static str {
    match error {
        EventError::CertCacheLoad(_) => "cannot read the cached certificate",
        EventError::AccountCacheLoad(_) => "cannot read the cached ACME account",
        EventError::CertCacheStore(_) => "cannot store the certificate",
        EventError::AccountCacheStore(_) => "cannot store the ACME account",
        EventError::CachedCertParse(_) => "the cached certificate is invalid",
        EventError::Order(OrderError::TooManyAttemptsAuth(_)) => {
            "the certificate authority could not validate the domain (DNS, port 443?)"
        }
        EventError::Order(_) => "the certificate order failed",
        EventError::NewCertParse(_) => "the new certificate is invalid",
    }
}

/// Keys and certificates as owner-only files (0600), written atomically into the certificate
/// directory, which is never recreated here: if it disappears, storing fails instead of creating
/// it with default permissions.
struct PrivateDirectoryCache {
    directory: PathBuf,
}

impl PrivateDirectoryCache {
    /// Stable per domains (or contacts) and certificate authority, like rustls-acme's own cache.
    fn file_name(kind: &str, names: &[String], directory_url: &str) -> String {
        let mut hasher = Sha256::new();
        for name in names {
            hasher.update(name.as_bytes());
            hasher.update([0]);
        }
        hasher.update(directory_url.as_bytes());
        let hash = hasher
            .finalize()
            .iter()
            .fold(String::new(), |mut hex, byte| {
                // Writing into a String cannot fail.
                let _ = write!(hex, "{byte:02x}");
                hex
            });
        format!("{kind}-{hash}")
    }

    async fn read(&self, file_name: String) -> io::Result<Option<Vec<u8>>> {
        match tokio::fs::read(self.directory.join(file_name)).await {
            Ok(contents) => Ok(Some(contents)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn write(&self, file_name: String, contents: &[u8]) -> io::Result<()> {
        let directory = self.directory.clone();
        let contents = contents.to_vec();
        tokio::task::spawn_blocking(move || {
            write_private_file_atomically(&directory, &file_name, &contents)
        })
        .await
        .map_err(io::Error::other)?
    }
}

fn write_private_file_atomically(
    directory: &Path,
    file_name: &str,
    contents: &[u8],
) -> io::Result<()> {
    use std::io::Write as _;
    let temporary_path = directory.join(format!(".{file_name}.partial"));
    // A leftover from a crash may have other permissions: start from a fresh file.
    match std::fs::remove_file(&temporary_path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&temporary_path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    std::fs::rename(&temporary_path, directory.join(file_name))
}

#[async_trait::async_trait]
impl CertCache for PrivateDirectoryCache {
    type EC = io::Error;

    async fn load_cert(
        &self,
        domains: &[String],
        directory_url: &str,
    ) -> io::Result<Option<Vec<u8>>> {
        self.read(Self::file_name("certificate", domains, directory_url))
            .await
    }

    async fn store_cert(
        &self,
        domains: &[String],
        directory_url: &str,
        cert: &[u8],
    ) -> io::Result<()> {
        self.write(Self::file_name("certificate", domains, directory_url), cert)
            .await
    }
}

#[async_trait::async_trait]
impl AccountCache for PrivateDirectoryCache {
    type EA = io::Error;

    async fn load_account(
        &self,
        contact: &[String],
        directory_url: &str,
    ) -> io::Result<Option<Vec<u8>>> {
        self.read(Self::file_name("account", contact, directory_url))
            .await
    }

    async fn store_account(
        &self,
        contact: &[String],
        directory_url: &str,
        account: &[u8],
    ) -> io::Result<()> {
        self.write(Self::file_name("account", contact, directory_url), account)
            .await
    }
}

/// Counts open connections, globally and per client address (and IPv6 site).
#[derive(Debug)]
struct ConnectionLimiter {
    total: Arc<Semaphore>,
    per_address_group: Mutex<HashMap<String, usize>>,
    maximum_per_client_address: usize,
    maximum_per_ipv6_site: usize,
}

impl ConnectionLimiter {
    fn new(policy: ConnectionPolicy) -> Self {
        Self {
            total: Arc::new(Semaphore::new(policy.maximum_connections)),
            per_address_group: Mutex::default(),
            maximum_per_client_address: policy.maximum_connections_per_client_address,
            maximum_per_ipv6_site: policy.maximum_connections_per_ipv6_site,
        }
    }

    /// The groups a connection counts against, with their caps.
    fn address_groups(&self, peer: SocketAddr) -> Vec<(String, usize)> {
        let mut groups = vec![(
            client_address_key(peer, &HeaderMap::new(), &[]),
            self.maximum_per_client_address,
        )];
        if let IpAddr::V6(address) = peer.ip()
            && address.to_ipv4_mapped().is_none()
        {
            let segments = address.segments();
            groups.push((
                format!("{:x}:{:x}:{:x}::/48", segments[0], segments[1], segments[2]),
                self.maximum_per_ipv6_site,
            ));
        }
        groups
    }

    /// `None` when any cap is reached; the connection is then dropped at once.
    fn try_reserve(self: &Arc<Self>, peer: SocketAddr) -> Option<ConnectionSlot> {
        let permit = self.total.clone().try_acquire_owned().ok()?;
        let groups = self.address_groups(peer);
        let mut counts = self
            .per_address_group
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let is_any_group_full = groups
            .iter()
            .any(|(group, maximum)| counts.get(group).copied().unwrap_or(0) >= *maximum);
        if is_any_group_full {
            return None;
        }
        for (group, _) in &groups {
            *counts.entry(group.clone()).or_insert(0) += 1;
        }
        Some(ConnectionSlot {
            _permit: permit,
            limiter: self.clone(),
            address_groups: groups.into_iter().map(|(group, _)| group).collect(),
        })
    }
}

/// Held for the connection's lifetime; frees its place in every cap when dropped.
#[derive(Debug)]
struct ConnectionSlot {
    _permit: OwnedSemaphorePermit,
    limiter: Arc<ConnectionLimiter>,
    address_groups: Vec<String>,
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        let mut counts = self
            .limiter
            .per_address_group
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for group in &self.address_groups {
            if let Some(count) = counts.get_mut(group) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    counts.remove(group);
                }
            }
        }
    }
}

/// Applies the connection policy around the TLS acceptor.
#[derive(Clone)]
struct GuardedAcceptor<Inner> {
    inner: Inner,
    limiter: Arc<ConnectionLimiter>,
    policy: ConnectionPolicy,
}

impl<Service, Inner> axum_server::accept::Accept<TcpStream, Service> for GuardedAcceptor<Inner>
where
    Inner: axum_server::accept::Accept<TcpStream, Service>,
    Inner::Future: Send + 'static,
    Inner::Stream: Send + 'static,
    Inner::Service: Send + 'static,
{
    type Stream = IdleTimeoutStream<Inner::Stream>;
    type Service = Inner::Service;
    type Future = Pin<Box<dyn Future<Output = io::Result<(Self::Stream, Self::Service)>> + Send>>;

    fn accept(&self, stream: TcpStream, service: Service) -> Self::Future {
        let slot = stream
            .peer_addr()
            .ok()
            .and_then(|peer| self.limiter.try_reserve(peer));
        let Some(slot) = slot else {
            return Box::pin(async {
                Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    "connection limit reached",
                ))
            });
        };
        let accepting = self.inner.accept(stream, service);
        let policy = self.policy;
        Box::pin(async move {
            let (stream, service) = tokio::time::timeout(policy.handshake_timeout, accepting)
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "TLS handshake timed out")
                })??;
            Ok((
                IdleTimeoutStream {
                    inner: stream,
                    idle_read_timeout: policy.idle_read_timeout,
                    idle_deadline: Box::pin(tokio::time::sleep(policy.idle_read_timeout)),
                    write_stall_timeout: policy.write_stall_timeout,
                    write_stall_deadline: None,
                    _slot: slot,
                },
                service,
            ))
        })
    }
}

/// Fails reads once nothing has arrived for `idle_read_timeout` (covers the bytes hyper reads
/// before its own timers start, and request bodies sent a byte at a time), and writes once the
/// peer has accepted nothing for `write_stall_timeout` (a client that stops reading).
struct IdleTimeoutStream<Inner> {
    inner: Inner,
    idle_read_timeout: Duration,
    idle_deadline: Pin<Box<Sleep>>,
    write_stall_timeout: Duration,
    /// Armed while a write waits, cleared as soon as one makes progress.
    write_stall_deadline: Option<Pin<Box<Sleep>>>,
    _slot: ConnectionSlot,
}

impl<Inner> IdleTimeoutStream<Inner> {
    fn track_write<Output>(
        &mut self,
        context: &mut Context<'_>,
        poll: Poll<io::Result<Output>>,
    ) -> Poll<io::Result<Output>> {
        if poll.is_ready() {
            self.write_stall_deadline = None;
            return poll;
        }
        let timeout = self.write_stall_timeout;
        let deadline = self
            .write_stall_deadline
            .get_or_insert_with(|| Box::pin(tokio::time::sleep(timeout)));
        match deadline.as_mut().poll(context) {
            Poll::Ready(()) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the client stopped reading",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<Inner: AsyncRead + Unpin> AsyncRead for IdleTimeoutStream<Inner> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_read(context, buffer) {
            Poll::Ready(result) => {
                let next_deadline = Instant::now() + this.idle_read_timeout;
                this.idle_deadline.as_mut().reset(next_deadline);
                Poll::Ready(result)
            }
            Poll::Pending => match this.idle_deadline.as_mut().poll(context) {
                Poll::Ready(()) => Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "connection idle for too long",
                ))),
                Poll::Pending => Poll::Pending,
            },
        }
    }
}

impl<Inner: AsyncWrite + Unpin> AsyncWrite for IdleTimeoutStream<Inner> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        let poll = Pin::new(&mut self.inner).poll_write(context, buffer);
        self.track_write(context, poll)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffers: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let poll = Pin::new(&mut self.inner).poll_write_vectored(context, buffers);
        self.track_write(context, poll)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        let poll = Pin::new(&mut self.inner).poll_flush(context);
        self.track_write(context, poll)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(context)
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncReadExt as _;

    use super::*;
    use crate::domain::account_rules::RegistrationMode;
    use crate::domain::accounts::tests::migrated_pool;
    use crate::http::tests::test_state;

    /// Nothing listens there: certificate requests fail at once, without touching the network.
    const UNREACHABLE_DIRECTORY_URL: &str = "https://127.0.0.1:9/directory";

    const FAST_POLICY: ConnectionPolicy = ConnectionPolicy {
        handshake_timeout: Duration::from_millis(300),
        header_read_timeout: Duration::from_millis(300),
        idle_read_timeout: Duration::from_millis(600),
        maximum_connections: 3,
        maximum_connections_per_client_address: 2,
        maximum_connections_per_ipv6_site: 3,
        write_stall_timeout: Duration::from_millis(600),
    };

    fn tls_config(certificate_directory: &Path) -> TlsConfig {
        TlsConfig {
            domains: vec!["chat.example.org".into()],
            contact_email: None,
            certificate_directory: certificate_directory.to_path_buf(),
            use_staging_certificates: true,
        }
    }

    /// True once the server closed the connection (end of stream or reset).
    async fn is_closed_within(stream: &mut TcpStream, limit: Duration) -> bool {
        let mut byte = [0_u8; 1];
        matches!(
            tokio::time::timeout(limit, stream.read(&mut byte)).await,
            Ok(Ok(0) | Err(_))
        )
    }

    #[tokio::test]
    async fn silent_and_excess_connections_are_closed_and_shutdown_is_clean() {
        let (directory, pool) = migrated_pool().await;
        let certificate_directory = directory.path().join("certificates");
        create_private_directory(&certificate_directory).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn({
            let state = test_state(pool.clone(), RegistrationMode::Open);
            let tls = tls_config(&certificate_directory);
            async move {
                serve_tls_with_policy(
                    listener,
                    state,
                    &tls,
                    CertificateAuthority {
                        directory_url: UNREACHABLE_DIRECTORY_URL,
                        client_tls_config: None,
                    },
                    FAST_POLICY,
                    async {
                        let _ = stopped.await;
                    },
                )
                .await
            }
        });

        // Two connections from one address fit; the third is over the per-address cap.
        let mut first = TcpStream::connect(address).await.unwrap();
        let mut second = TcpStream::connect(address).await.unwrap();
        let mut third = TcpStream::connect(address).await.unwrap();
        assert!(is_closed_within(&mut third, Duration::from_secs(2)).await);
        // The silent ones never finish a handshake and are dropped after the deadline.
        assert!(is_closed_within(&mut first, Duration::from_secs(2)).await);
        assert!(is_closed_within(&mut second, Duration::from_secs(2)).await);
        // Their slots are free again.
        let mut fourth = TcpStream::connect(address).await.unwrap();
        assert!(!is_closed_within(&mut fourth, Duration::from_millis(100)).await);

        stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(15), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        pool.close().await;
    }

    fn guarded_stream(
        inner: tokio::io::DuplexStream,
    ) -> IdleTimeoutStream<tokio::io::DuplexStream> {
        let limiter = Arc::new(ConnectionLimiter::new(FAST_POLICY));
        IdleTimeoutStream {
            inner,
            idle_read_timeout: FAST_POLICY.idle_read_timeout,
            idle_deadline: Box::pin(tokio::time::sleep(FAST_POLICY.idle_read_timeout)),
            write_stall_timeout: FAST_POLICY.write_stall_timeout,
            write_stall_deadline: None,
            _slot: limiter.try_reserve("192.0.2.1:1".parse().unwrap()).unwrap(),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_peer_or_one_that_stops_reading_times_out() {
        use tokio::io::AsyncWriteExt as _;

        let (inner, mut peer) = tokio::io::duplex(64);
        let mut stream = guarded_stream(inner);
        peer.write_all(b"x").await.unwrap();
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).await.unwrap();
        let error = stream.read(&mut byte).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);

        // The peer never reads: the 64-byte pipe fills, then writing stalls until the deadline.
        let (inner, _peer_that_never_reads) = tokio::io::duplex(64);
        let mut stream = guarded_stream(inner);
        let error = stream.write_all(&[0; 1_024]).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn ipv6_connections_also_count_per_slash_48() {
        let limiter = Arc::new(ConnectionLimiter::new(FAST_POLICY));
        let peer = |text: &str| text.parse::<SocketAddr>().unwrap();
        let slots: Vec<_> = [
            "[2001:db8:1:1::1]:1",
            "[2001:db8:1:2::1]:1",
            "[2001:db8:1:3::1]:1",
        ]
        .into_iter()
        .map(|address| limiter.try_reserve(peer(address)).unwrap())
        .collect();
        assert!(
            limiter.try_reserve(peer("[2001:db8:1:4::1]:1")).is_none(),
            "fourth /64 of the same /48"
        );
        drop(slots);
        let fifth = limiter.try_reserve(peer("[2001:db8:1:4::1]:1"));
        assert!(fifth.is_some());
        let group_count =
            |limiter: &ConnectionLimiter| limiter.per_address_group.lock().unwrap().len();
        assert_eq!(group_count(&limiter), 2, "one /64 and one /48");
        drop(fifth);
        assert_eq!(group_count(&limiter), 0, "freed groups are removed");
    }

    #[test]
    fn cached_keys_are_owner_only_files_replaced_atomically() {
        let directory = tempfile::tempdir().unwrap();
        write_private_file_atomically(directory.path(), "account-x", b"first").unwrap();
        write_private_file_atomically(directory.path(), "account-x", b"second").unwrap();
        let path = directory.path().join("account-x");
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let missing = directory.path().join("gone");
        assert!(write_private_file_atomically(&missing, "account-x", b"x").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn the_certificate_directory_must_be_private_and_outside_the_data_directory() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().unwrap();
        let data_directory = root.path().join("data");
        std::fs::create_dir(&data_directory).unwrap();
        std::fs::set_permissions(&data_directory, std::fs::Permissions::from_mode(0o700)).unwrap();

        let certificates = root.path().join("certificates");
        prepare_certificate_directory(&certificates, &data_directory).unwrap();
        let mode = std::fs::metadata(&certificates)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);

        let shared = root.path().join("shared");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = prepare_certificate_directory(&shared, &data_directory).unwrap_err();
        assert!(error.to_string().contains("open to other users"), "{error}");

        let link = root.path().join("looks-separate");
        std::os::unix::fs::symlink(&data_directory, &link).unwrap();
        let error = prepare_certificate_directory(&link, &data_directory).unwrap_err();
        assert!(error.to_string().contains("resolves inside"), "{error}");
    }

    /// Path of a Pebble binary (<https://github.com/letsencrypt/pebble>), the ACME test authority.
    const PEBBLE_BINARY_VARIABLE: &str = "MELLOW_PEBBLE_BINARY";
    /// Pebble validates TLS-ALPN-01 by connecting to this name, which resolves to loopback.
    const PEBBLE_TEST_DOMAIN: &str = "localhost";
    const PEBBLE_STARTUP_LIMIT: Duration = Duration::from_secs(10);
    const CERTIFICATE_ISSUANCE_LIMIT: Duration = Duration::from_secs(60);
    const RETRY_INTERVAL: Duration = Duration::from_millis(200);

    /// rustls-acme answers TLS-ALPN-01 only, so Pebble never connects to its HTTP-01 port.
    const UNUSED_HTTP_CHALLENGE_PORT: u16 = 5002;

    struct RunningPebble {
        /// Killed on drop.
        process: tokio::process::Child,
        directory_url: String,
        management_url: String,
        /// Certificate and key of Pebble's own HTTPS endpoints.
        https_identity: rcgen::CertifiedKey,
    }

    /// Starts Pebble with a fresh HTTPS certificate; it validates TLS-ALPN-01 on `challenge_port`.
    async fn start_pebble(work_directory: &Path, challenge_port: u16) -> RunningPebble {
        let binary = std::env::var_os(PEBBLE_BINARY_VARIABLE).unwrap_or_else(|| {
            panic!("set {PEBBLE_BINARY_VARIABLE} to a Pebble binary to run this test")
        });
        let https_identity =
            rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).unwrap();
        let certificate_path = work_directory.join("pebble-https.pem");
        let key_path = work_directory.join("pebble-https.key");
        std::fs::write(&certificate_path, https_identity.cert.pem()).unwrap();
        std::fs::write(&key_path, https_identity.key_pair.serialize_pem()).unwrap();
        // Both held until Pebble starts, so the system cannot hand out the same free port twice.
        let directory_port_probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let management_port_probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let directory_port = directory_port_probe.local_addr().unwrap().port();
        let management_port = management_port_probe.local_addr().unwrap().port();
        let config = serde_json::json!({ "pebble": {
            "listenAddress": format!("127.0.0.1:{directory_port}"),
            "managementListenAddress": format!("127.0.0.1:{management_port}"),
            "certificate": certificate_path,
            "privateKey": key_path,
            "httpPort": UNUSED_HTTP_CHALLENGE_PORT,
            "tlsPort": challenge_port,
            "ocspResponderURL": "",
            "externalAccountBindingRequired": false,
        }});
        let config_path = work_directory.join("pebble-config.json");
        std::fs::write(&config_path, config.to_string()).unwrap();
        let log_path = work_directory.join("pebble.log");
        drop((directory_port_probe, management_port_probe));
        let mut process = tokio::process::Command::new(binary)
            .arg("-config")
            .arg(&config_path)
            // No artificial validation delay, and no random nonce rejections: rustls-acme does
            // not retry a request refused for a bad nonce, Let's Encrypt never sends those.
            .env("PEBBLE_VA_NOSLEEP", "1")
            .env("PEBBLE_WFE_NONCEREJECT", "0")
            // Would approve every authorization without connecting to the server under test.
            .env_remove("PEBBLE_VA_ALWAYS_VALID")
            .stdout(std::fs::File::create(&log_path).unwrap())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let started = Instant::now();
        while TcpStream::connect(("127.0.0.1", management_port))
            .await
            .is_err()
        {
            let log = || std::fs::read_to_string(&log_path).unwrap_or_default();
            if let Some(status) = process.try_wait().unwrap() {
                panic!("Pebble exited at startup ({status}):\n{}", log());
            }
            assert!(
                started.elapsed() < PEBBLE_STARTUP_LIMIT,
                "Pebble did not start:\n{}",
                log()
            );
            tokio::time::sleep(RETRY_INTERVAL).await;
        }
        RunningPebble {
            process,
            directory_url: format!("https://127.0.0.1:{directory_port}/dir"),
            management_url: format!("https://127.0.0.1:{management_port}"),
            https_identity,
        }
    }

    fn spawn_server(
        listener: TcpListener,
        pool: sqlx::SqlitePool,
        tls: TlsConfig,
        directory_url: String,
        acme_client_tls_config: Arc<rustls_acme::rustls::ClientConfig>,
    ) -> (
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<Result<()>>,
    ) {
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            serve_tls_with_policy(
                listener,
                test_state(pool, RegistrationMode::Open),
                &tls,
                CertificateAuthority {
                    directory_url: &directory_url,
                    client_tls_config: Some(acme_client_tls_config),
                },
                PRODUCTION_CONNECTION_POLICY,
                async {
                    let _ = stopped.await;
                },
            )
            .await
        });
        (stop, server)
    }

    /// Health over HTTPS, trusting only `root`, retried until a certificate is served.
    async fn wait_for_https_health(port: u16, root: &reqwest::Certificate) {
        let client = reqwest::Client::builder()
            .tls_certs_only([root.clone()])
            .build()
            .unwrap();
        let url = format!("https://{PEBBLE_TEST_DOMAIN}:{port}/api/v1/health");
        let started = Instant::now();
        loop {
            match client.get(&url).send().await {
                Ok(response) if response.status().is_success() => return,
                outcome => {
                    assert!(
                        started.elapsed() < CERTIFICATE_ISSUANCE_LIMIT,
                        "no trusted certificate served in time: {outcome:?}"
                    );
                    tokio::time::sleep(RETRY_INTERVAL).await;
                }
            }
        }
    }

    #[tokio::test]
    #[ignore = "needs Pebble: set MELLOW_PEBBLE_BINARY"]
    async fn a_certificate_is_obtained_over_tls_alpn_and_served_from_the_cache_after_restart() {
        let (directory, pool) = migrated_pool().await;
        let certificate_directory = directory.path().join("certificates");
        create_private_directory(&certificate_directory).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let RunningPebble {
            process: pebble,
            directory_url,
            management_url,
            https_identity: pebble_https,
        } = start_pebble(directory.path(), port).await;
        let pebble_https_root =
            reqwest::Certificate::from_pem(pebble_https.cert.pem().as_bytes()).unwrap();
        let mut acme_roots = rustls_acme::rustls::RootCertStore::empty();
        acme_roots.add(pebble_https.cert.der().clone()).unwrap();
        let acme_client_tls_config = Arc::new(
            rustls_acme::rustls::ClientConfig::builder()
                .with_root_certificates(acme_roots)
                .with_no_client_auth(),
        );
        let tls = TlsConfig {
            domains: vec![PEBBLE_TEST_DOMAIN.into()],
            ..tls_config(&certificate_directory)
        };
        let (stop, server) = spawn_server(
            listener,
            pool.clone(),
            tls.clone(),
            directory_url.clone(),
            acme_client_tls_config.clone(),
        );
        // Pebble signs with a root generated at startup, published on its management endpoint.
        let issuing_root = reqwest::Client::builder()
            .tls_certs_only([pebble_https_root])
            .build()
            .unwrap()
            .get(format!("{management_url}/roots/0"))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let issuing_root = reqwest::Certificate::from_pem(&issuing_root).unwrap();
        wait_for_https_health(port, &issuing_root).await;
        stop.send(()).unwrap();
        server.await.unwrap().unwrap();

        // Without the authority, a restart still serves the certificate it cached.
        drop(pebble);
        let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        let (stop, server) = spawn_server(
            listener,
            pool.clone(),
            tls,
            directory_url,
            acme_client_tls_config,
        );
        wait_for_https_health(port, &issuing_root).await;
        stop.send(()).unwrap();
        server.await.unwrap().unwrap();
        pool.close().await;
    }
}
