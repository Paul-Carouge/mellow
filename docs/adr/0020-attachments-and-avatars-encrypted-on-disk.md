# 0020. Store attachments and avatars encrypted on disk, serve them as downloads

- Status: Accepted
- Date: 2026-10-21

## Context
Sharing files and pictures, and showing who is who with an avatar, are part of the Discord baseline (ROADMAP M2). ARCHITECTURE.md §4.3 already says attachments live encrypted under `attachments/` in the data directory, with their metadata in SQLite, and §7 that uploads are served with `Content-Disposition` and `nosniff`, never rendered as HTML or SVG. The web client (ADR-0019) is served from the same origin as the API and holds a session cookie: a file served inline as HTML or SVG from that origin would run with the user's session.

## Decision
- **Upload first, then send.** `POST /api/v1/channels/{id}/attachments` takes the raw file as the body (no multipart parser) and its name percent-encoded in `X-Mellow-File-Name` (never in a URL, so proxy logs and browser history hold no file names), and answers an attachment id. It needs `ATTACH_FILES` (participants of a conversation have it), and sending the message needs it again. A message refers to up to 10 uploaded attachments by id (`attachment_ids` in `SendMessageRequest`); an attachment belongs to the user who uploaded it and to its channel, and is linked to one message only. Uploads never linked are deleted after an hour.
- **Limits**: 25 MB per file by default (`maximum_attachment_bytes` in `mellow.toml`); rate limits, permissions, the name and the quota are checked before the body is read, which stops at the limit. 2 GiB of uploads per account (`maximum_attachment_storage_bytes_per_user`). At most six uploads or downloads hold a file in memory at once; beyond, the instance answers "busy". File names are cleaned (no path separators, no control characters, at most 255 bytes).
- **Encryption at rest**: each file is sealed with AES-256-GCM (`aws-lc-rs`, already in the tree through rustls: no new crate) under a key derived from the master key with HKDF (`mellow/attachments-aes-256-gcm/v1`), with a random 96-bit nonce per file, the attachment id as associated data. Files live at `attachments/<two id hex digits>/<id>`. A file is written to a temporary name and renamed once complete.
- **Downloads**: `GET /api/v1/attachments/{id}` (120 per minute per user) needs `VIEW_CHANNEL` and `READ_MESSAGE_HISTORY` on the message's channel (a hidden channel answers `NOT_FOUND`). Answers carry `X-Content-Type-Options: nosniff`, `Content-Security-Policy: default-src 'none'; sandbox`, `Cache-Control: private, no-cache`, and `Content-Disposition: attachment` — except PNG, JPEG, GIF and WebP whose magic bytes match their type, which are `inline` with that exact type so clients can show previews. Nothing else is ever inline, SVG included.
- **Avatars**: `PUT /api/v1/users/@me/avatar` (PNG, JPEG or WebP checked by magic bytes, at most 2 MB), `DELETE` to remove it; served at `GET /api/v1/avatars/{user_id}/{avatar_id}` to any signed-in user, same headers as inline images. `PublicUser` and `CurrentUser` gain `avatar_id` (absent when none); `USER_UPDATE` carries it.
- **Protocol**: `Message` gains `attachments` (id, file name, content type, size, and for images the pixel size when known), absent from older servers' payloads (`#[serde(default)]`).

## Consequences
- Backups must include `attachments/`; restoring needs the master key, as for the database.
- A whole file is sealed in memory (25 MB per upload at most). shortcut: streaming encryption in chunks when the limit grows or uploads get concurrent at scale.
- Deleting a message, a channel or a guild deletes the attachment rows at once (`ON DELETE CASCADE`); their files go at the next purge (every ten minutes) once older than an hour, the delay that keeps a file whose row is being written from being swept.
- Clients show images inline and other files as download cards; nothing is executed.

## Alternatives considered
- **Serving uploads from another origin** (a second domain): the strongest isolation for inline content, but a second domain and certificate contradict trivial self-hosting. `attachment` plus a sandbox CSP for everything but checked raster images is enough.
- **Multipart uploads**: one more parser to harden for no benefit; a raw body with the name in the query string is enough for one file per request.
- **No encryption at rest**: contradicts ADR-0006.
