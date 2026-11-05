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
- Facing the internet without a proxy, the server bounds what one client can hold: 10 000 connections in total, 256 per IPv4 address or IPv6 /64 and 1 024 per IPv6 /48, 10 s for the TLS handshake, 30 s for each request's headers (including an idle keep-alive), 75 s without any byte received (above the gateway's 45 s heartbeat deadline, so live WebSockets are never cut), and 30 s for a peer that stops accepting written bytes. HTTP/2 keep-alive pings are off, since their automatic answers would keep idle connections alive.
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

## Amendment (2026-11-02): HTTP/2 over built-in TLS
The owner accepted HTTP/2 on built-in TLS (2026-11-01), with a security audit. An avatar-heavy guild page sends dozens of small image requests; over HTTP/1.1 a browser queues them on six connections.

- ALPN offers `h2`, then `http/1.1`. TLS-ALPN-01 challenges are unaffected: rustls-acme's acceptor recognises a client hello offering only `acme-tls/1` and answers it with its own configuration before Mellow's applies.
- HTTP/2 is bounded explicitly: 100 concurrent streams per connection (the RFC 9113 minimum advice), 20 streams reset before being accepted (Rapid Reset, CVE-2023-44487; hyper also caps locally caused resets at 1 024) and a 16 KiB decoded header list. Keep-alive pings stay off. The same settings apply to HTTP/2 with prior knowledge on the plain HTTP listener, which hyper already accepted before this amendment.
- The socket deadlines do not bound HTTP/2. Any frame from the client, a PING included, resets the 75 s idle read deadline, and flow control lets a client stall a response without ever blocking the socket, so the write stall deadline never starts; hyper has no header deadline for HTTP/2 either. Left alone, one connection could hold up to 100 requests forever, each with its response in memory and its transfer permit (8 attachment downloads and 16 profile image reads for the whole instance). So Mellow watches each HTTP/2 request (`tls/request_progress.rs`) and closes the connection, which drops every response and frees its permits, once a response has waited 30 s (the write stall timeout) for the client to accept its next 64 KiB piece, a request body has waited 75 s (the idle read timeout) for its next bytes, or no request has been open for 30 s (the header deadline). Responses are handed to hyper in 64 KiB pieces, so a large one shows progress piece by piece and HTTP/2 buffers about two pieces of it, not the whole body. Waits on the server (a handler still producing data) do not count. HTTP/1.1 connections keep the socket deadlines only, so an upgraded `WebSocket` is never closed for having no open request. *Amended 2026-11-05 ([ADR-0038](0038-play-audio-and-video-attachments-inline.md), throughput floor):* on both versions, a response of known length must also be handed over whole within 30 s plus 1 s per 32 KiB, counted from its first piece; a client reading slower is closed, so a trickle that keeps the write stall deadline from firing no longer keeps its permit. A response with no data (an upgrade) never starts that deadline.
- What one stalled HTTP/2 connection can still hold: up to 100 in-flight requests, each with its response and permits, for at most 30 s after the client last accepted a piece of it. A client that keeps accepting a piece of every response every 30 s can keep them, as an HTTP/1.1 client reading slowly already could, one response per connection.
- HTTP/2 names the host in `:authority`, not `Host`. A middleware copies it into `Host`, which the origin check (CSRF, cross-site WebSocket hijacking) reads, and refuses a request whose `Host` names another host.
- Extended CONNECT (RFC 8441) is not enabled, so browsers keep opening the gateway `WebSocket` over a separate HTTP/1.1 connection.
- Consequence: one address can now run up to 256 connections × 100 streams of requests at once instead of 256. Request rate limits are unchanged and still apply per address.
