# 0038. Play audio and video attachments inline

- Status: Accepted
- Date: 2026-11-03
- Amends: [ADR-0020](0020-attachments-and-avatars-encrypted-on-disk.md)

## Context
ROADMAP item 18 (owner, #roadmap): audio files played by clicking them whatever their common format, and videos watched inside the app. ADR-0020 serves everything but checked raster pictures with `Content-Disposition: attachment` and `application/octet-stream`, so a browser can only download them. Media elements also need byte ranges: Safari plays nothing without them, and other browsers cannot seek.

## Decision
- **Sniffed types.** The upload's first bytes decide its type, as for pictures; what a client claims is never trusted. Played inline (`PLAYABLE_MEDIA_TYPES`): `video/mp4` (ISO-BMFF with an ordinary video major brand: `isom`, `iso2`, `iso4`–`iso6`, `mp41`, `mp42`, `avc1`, `M4V `, `dash`, `mmp4`), `video/quicktime` (`qt  `), `video/webm` (EBML with the `webm` doc type), `audio/mp4` (`M4A `, `M4B `), `audio/mpeg` (`ID3` or an MPEG layer III frame), `audio/aac` (ADTS), `audio/ogg`, `audio/wav` (`RIFF…WAVE`), `audio/flac`. Matroska, AVI and anything else stay downloads.
- **Served inline** with that exact type, under the same headers as pictures (`nosniff`, `default-src 'none'; sandbox`, `private, no-cache`, the entity tag). None of these types is a document a browser renders with scripts.
- **Byte ranges** on `GET /api/v1/attachments/{id}` (and `/preview`): `Accept-Ranges: bytes`, one range per request (`first-last`, `first-`, `-suffix`; a last position past the end is clamped, and one answer carries at most 8 MiB: players ask for the next part themselves, and one that stops reading, paused or after the metadata, holds a download slot for a short answer instead of a whole file), answered `206` with `Content-Length` and `Content-Range`; a range starting past the end gets `416` with `bytes */total`; several ranges or a malformed header get the whole file (`200`), as RFC 9110 allows; an `If-Range` that is not the current entity tag gets the whole file. Permission check, rate limit and download slot are the same as for a whole download.
- **Clients** show playable attachments with a player (video with its poster frame loaded on demand, audio as a compact player) and keep the download card for the rest.

## Consequences
- Every range request decrypts the whole file (at most 25 MB by default) and takes one of the eight download slots, because files are sealed in one piece (ADR-0020). shortcut: one range request decrypts the whole file; seal in chunks (ADR-0020's own shortcut) if seeking in large videos shows on the server.
- The cap trades slots for work: a 25 MB video watched to the end takes about four range requests, each decrypting the whole file (about 100 MB decrypted per viewing).
- Seeking issues more requests than one download: the 120 downloads per minute per user still apply; a player seeking wildly can meet the limit and stall for a moment; the web client keeps its player on a network error (pressing play again resumes) and shows the download card only when the browser cannot decode the file.
- Audio and video keep their metadata, location included (phones write it into MP4 and MOV files); pictures are the only uploads stripped (ADR-0033). Readers of the channel could already download them. shortcut: metadata kept; strip ISO-BMFF `udta`/`meta` location boxes at upload when media sharing grows.
- Audio and video uploaded before this change keep `application/octet-stream` and stay downloads (their type was stored at upload); no migration rewrites them.
- No transcoding: a file plays when the reader's browser can decode its codec (H.264, VP8/VP9, AV1, AAC, MP3, Opus, Vorbis, FLAC and PCM cover what phones and recorders produce). HEVC in MP4 plays in Safari and some Chrome builds only.

## Alternatives considered
- **Sniffing at download time** so old uploads play too: clients choose the player from the type in the message, which would still say `application/octet-stream`; a migration reading and decrypting every file costs more than re-uploading the few concerned.
- **Transcoding to one format on the server**: an FFmpeg dependency or a codec stack in the binary, heavy CPU on small hosts; against trivial self-hosting.
- **Streaming decryption per range** (chunked sealing): the right fix at scale, but a storage format change for all attachments; kept as the shortcut's upgrade path.
