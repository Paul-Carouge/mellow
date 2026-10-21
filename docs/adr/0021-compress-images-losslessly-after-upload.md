# 0021. Compress images losslessly; animated avatars and banners for everyone

- Status: Accepted
- Date: 2026-10-21

## Context
The project owner wants what Discord sells as Nitro, free: animated avatars and animated profile banners. Animated GIFs are large, and every picture uploaded (avatars, banners, attachments) takes disk space on a self-hosted machine, often small. Pictures must not lose visible quality, and uploading must stay as fast as it is today ("almost seamless"). ADR-0020 stores files sealed on disk; avatars were planned there, without banners or animation.

## Decision
- **Attachments: accept first, compress after.** An attachment is stored as received (sealed, ADR-0020) and answered at once, so sending stays instant; a background worker then produces the compressed version and replaces the stored file when it is smaller, updating the size and type. Clients see the new file on their next download. A bounded queue and one worker cap the CPU used; a job that fails, finds the queue full or runs past its time budget keeps the original.
- **Avatars and banners: compressed while uploading.** They are small after resizing (a still avatar takes about 50 ms; a large animated GIF one or two seconds, shown as progress in settings), and processing them before answering gives each version its own id, so clients can cache them forever.
- **Decoding is pure Rust** (`image` crate: PNG, JPEG, GIF, WebP decoders, all memory-safe), with limits checked before decoding: at most 8 192 × 8 192 pixels, 64 megapixels in total across frames, 600 frames. Anything over them, or undecodable, is kept as uploaded and never shown inline.
- **Avatars and banners** (`PUT /api/v1/users/@me/avatar` and `/banner`, PNG, JPEG, GIF or WebP, 8 MB at most, within the transfer slots of ADR-0020): resized to their largest display size (avatars 512 × 512, banners 1 920 px wide, never enlarged) with a Lanczos filter, then encoded as lossless WebP, animated when the source is (`libwebp` through `webp-animation`, statically linked: no system library). Animated avatars and banners are free for everyone. An animated one also gets a still version (its first frame), which clients show by default; they animate it on hover or focus and in the profile popout, as Discord does, and never under reduced motion.
- **Attachments keep their format and their pixels**: PNG is recompressed losslessly (`oxipng`, pure Rust, a fast preset); JPEG is not re-encoded (any re-encoding loses quality) but its metadata is stripped, which also removes GPS positions (privacy); animated GIF becomes animated lossless WebP, the file name's extension changing to `.webp`; other files are untouched.
- **Protocol**: `PublicUser` and `CurrentUser` gain `avatar` and `banner` (`{ id, is_animated }`, absent when none, ignored by older clients); `USER_UPDATE` carries them. They are served at `GET /api/v1/profile-images/{id}` and `/profile-images/{id}/still` to any signed-in user, immutable (`Cache-Control: private, max-age=31536000, immutable`), with the other headers of inline images (ADR-0020).

## Consequences
- Two new crates with their dependencies (`image` with only the four formats enabled, `oxipng` without its default parallelism features, `webp-animation` with `libwebp-sys` built from source by `cc`): they need a reason in the PR and `cargo deny`.
- `libwebp` is C code, used to encode only: untrusted bytes are decoded by Rust, and libwebp receives decoded pixels.
- A stored file can change size after upload; quotas count the stored size, so compression frees quota.
- CPU use after uploads, bounded by the queue; measured in release, a 480 × 480 GIF of 60 frames takes about 2 s, decoding included.

## Alternatives considered
- **Compress before answering the upload**: simplest, but a large animated GIF would block the upload for seconds, against the "seamless" requirement.
- **Lossy WebP or AVIF**: much smaller, but visibly changes pictures; the owner asked for no quality loss.
- **ffmpeg or gifsicle as external programs**: better tools, but a second thing to install contradicts the single-binary promise.
- **Converting every attachment to WebP**: smaller PNGs too, but people expect to download the file format they sent; only animated GIFs, which browsers handle the same as animated WebP, are converted.
