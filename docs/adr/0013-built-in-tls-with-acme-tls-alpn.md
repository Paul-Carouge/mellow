# 0013. Serve TLS directly with ACME certificates over TLS-ALPN-01

- Status: Accepted
- Date: 2026-10-19

## Context
ADR-0006 promises TLS everywhere with built-in ACME certificates, and ADR-0002 promises one binary with no external pieces. Until now the server only spoke plain HTTP and refused public addresses unless a TLS reverse proxy fronted it. Most self-hosters have a VPS, a domain and nothing else: requiring Caddy or nginx breaks the "one binary" promise. The ACME challenge type decides what the host must expose: HTTP-01 needs port 80 as well as 443, DNS-01 needs DNS provider credentials, TLS-ALPN-01 needs only port 443.

## Decision
- `mellow.toml` gets an optional `[tls]` table: `domains`, optional `contact_email`, `certificate_directory`, and `use_staging_certificates` for tests against Let's Encrypt staging. Without it, the server keeps the current plain HTTP behaviour (loopback only, or behind a reverse proxy).
- With `[tls]`, the server listens with rustls and obtains and renews certificates itself through `rustls-acme`, using the **TLS-ALPN-01** challenge on the same port. Only port 443 has to be reachable; no port 80 listener, no redirect.
- `[tls]` and `behind_tls_reverse_proxy` are mutually exclusive. A public listen address is accepted only with one of them.
- The certificate directory holds the ACME account key and the certificate's private key. It must be outside the data directory (like the master key, so a copied data directory or backup does not carry it) and private to the service user (0700, checked at start-up; an existing directory open to others is refused).
- Facing the internet without a proxy, the server bounds what one client can hold: 10 000 connections in total and 256 per IPv4 address or IPv6 /64, 10 s for the TLS handshake, 30 s for each request's headers (including an idle keep-alive), and 75 s without any byte received (above the gateway's 45 s heartbeat deadline, so live WebSockets are never cut).
- Keys are stored by Mellow's own cache, not `rustls-acme`'s `DirCache` (which writes with the default umask and recreates a missing directory): owner-only files (0600) written atomically, into a directory that must exist, belong to the service user and be closed to others.
- `mellow-server init --domain chat.example.org` writes the `[tls]` table and listens on port 443 by default.

## Consequences
- An instance on a VPS needs one command, one open port and a DNS record. No reverse proxy, no certbot.
- The binary binds port 443: the service needs `CAP_NET_BIND_SERVICE` (systemd `AmbientCapabilities=`), documented in the README.
- Wildcard certificates are impossible (TLS-ALPN-01 cannot prove them); one certificate covers the listed names.
- The server pulls in `rustls-acme`, `axum-server` and their dependencies (`async-io`, `futures-rustls`, `rcgen`, `webpki-roots`). `webpki-roots` uses the CDLA-Permissive-2.0 data licence, already accepted for `webpki-root-certs`.
- The ACME path cannot be tested without an ACME server; tests cover configuration, the certificate directory and cache, connection caps, the handshake deadline and shutdown. Running Pebble in CI is a follow-up.
- Certificate and account keys are stored unencrypted, protected by file permissions only. Encrypting them with a key derived from the master key is possible later without changing the configuration.
- ACME errors are logged by category only, since the authority's messages can quote the contact email.

## Alternatives considered
- **HTTP-01 on port 80:** a second listener and port to open for no benefit over TLS-ALPN-01.
- **DNS-01:** needs DNS API credentials per provider; worth adding later for wildcard certificates or hosts without a public port.
- **`instant-acme` with our own renewal loop:** more code (challenge certificates, renewal scheduling, caching) for the same result.
- **User-supplied certificate files only:** keeps certbot or a reverse proxy in the setup, which this ADR exists to remove.
- **`rustls-acme`'s `DirCache`:** default file permissions, and inside the data directory backups would carry the TLS private key.
