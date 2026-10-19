//! Built-in TLS with ACME certificates over TLS-ALPN-01 (ADR-0013).

use std::future::Future;
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use futures_util::StreamExt as _;
use rustls_acme::AcmeConfig;
use rustls_acme::caches::DirCache;
use tokio::net::TcpListener;

use crate::config::TlsConfig;
use crate::http::{AppState, make_service};
use crate::storage::create_private_directory;

/// A client that has not finished the handshake by then is dropped, so slow or idle connections
/// cannot pile up before HTTP starts.
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Open requests and `WebSocket` connections get this long to finish once shutdown starts.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Creates the certificate directory, or checks an existing one: it holds private keys, so it
/// must be private to the service user and outside the data directory, even through symbolic links.
///
/// # Errors
/// Fails if the directory cannot be created or resolved, is open to other users, or resolves
/// inside the data directory.
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
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(certificate_directory)
            .with_context(|| format!("cannot read {}", certificate_directory.display()))?
            .permissions()
            .mode();
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
/// in the background.
///
/// # Errors
/// Fails if the listener stops with an I/O error.
pub(crate) async fn serve_tls_until_shutdown(
    listener: TcpListener,
    state: AppState,
    tls: &TlsConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let mut certificates = AcmeConfig::new(&tls.domains)
        .contact(
            tls.contact_email
                .iter()
                .map(|email| format!("mailto:{email}")),
        )
        .cache(DirCache::new(tls.certificate_directory.clone()))
        .directory_lets_encrypt(!tls.use_staging_certificates)
        .state();
    let acceptor = HandshakeTimeoutAcceptor {
        inner: certificates.axum_acceptor(certificates.default_rustls_config()),
    };
    let renewal = tokio::spawn(async move {
        while let Some(event) = certificates.next().await {
            match event {
                Ok(event) => tracing::info!(?event, "certificate"),
                // Renewal is retried by rustls-acme; the current certificate keeps serving.
                Err(error) => tracing::error!(?error, "certificate request failed"),
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
    if let Ok(address) = listener.local_addr() {
        tracing::info!(%address, domains = ?tls.domains, "listening with TLS");
    }
    let result = axum_server::from_tcp(listener.into_std()?)?
        .acceptor(acceptor)
        .handle(handle)
        .serve(make_service(state))
        .await
        .context("HTTPS server stopped with an error");
    renewal.abort();
    result
}

#[derive(Clone)]
struct HandshakeTimeoutAcceptor<Inner> {
    inner: Inner,
}

impl<Stream, Service, Inner> axum_server::accept::Accept<Stream, Service>
    for HandshakeTimeoutAcceptor<Inner>
where
    Inner: axum_server::accept::Accept<Stream, Service>,
    Inner::Future: Send + 'static,
{
    type Stream = Inner::Stream;
    type Service = Inner::Service;
    type Future = Pin<Box<dyn Future<Output = io::Result<(Self::Stream, Self::Service)>> + Send>>;

    fn accept(&self, stream: Stream, service: Service) -> Self::Future {
        let accepting = self.inner.accept(stream, service);
        Box::pin(async move {
            tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, accepting)
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TLS handshake timed out"))?
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    #[test]
    fn the_certificate_directory_must_be_private_and_outside_the_data_directory() {
        let root = tempfile::tempdir().unwrap();
        let data_directory = root.path().join("data");
        std::fs::create_dir(&data_directory).unwrap();

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
        assert!(prepare_certificate_directory(&shared, &data_directory).is_err());

        let link = root.path().join("looks-separate");
        std::os::unix::fs::symlink(&data_directory, &link).unwrap();
        std::fs::set_permissions(&data_directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(prepare_certificate_directory(&link, &data_directory).is_err());
    }
}
