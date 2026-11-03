# 0039. Forward messages as snapshots

- Status: Accepted
- Date: 2026-11-03
- Amends: [ADR-0020](0020-attachments-and-avatars-encrypted-on-disk.md) (one sealed file may back several attachment rows)

## Context
ROADMAP item 18 asks for forwarding, as on Discord: a message, with its files, sent on to another channel or conversation, marked "Transféré" and pointing back at where it came from. The people who see the forward may not see the original's channel, so the forward cannot show the original live: it must carry what it shows. Attachments are sealed files read under the permission of their own channel (ADR-0020), so a forward cannot simply point at the original's files either.

## Decision
- **A forward is a new message** in the destination, sent with `POST /api/v1/channels/{channel_id}/messages` and `forwarded_from: {channel_id, message_id}` instead of content, attachments or a reply (those must be empty). The server copies the original's content and attachments into it at that moment: a snapshot. Later edits or the deletion of the original do not change the forward, as on Discord.
- **Permissions**: the forwarder needs `VIEW_CHANNEL` and `READ_MESSAGE_HISTORY` on the original's channel (a message they cannot read is `404`, like a hidden one), and `VIEW_CHANNEL`, `SEND_MESSAGES` and, when the original has files, `ATTACH_FILES` in the destination. Decided by the domain's permission function like every other send (ADR-0007).
- **The forward keeps its origin**: `Message.forwarded_from` holds the original's channel and message ids (two new columns on `messages`, migration 0034). Forwarding a forward keeps the first origin, so the link and the date still name the message written first. Clients show the origin as a link only when they know its channel, and derive its date from the message id.
- **No mentions, no edits**: a forward never mentions anyone (its text was written for another audience), and it cannot be edited (`400`): it is a copy of someone else's words. Its author may delete it; reactions, pins and replies work as on any message.
- **Files are shared, not copied**: each attachment of the forward is a new row pointing at the original's sealed file (`file_id`) and preview. Files are sealed with their file id as associated data, so a shared file opens the same way from either row. The only direct delete of a file a sent message may hold is the background replacement of a file by a smaller one, which now moves every row holding that file in the same statement (the other direct deletes concern unsent uploads and unrecorded previews, never shared); deleted messages leave their files to the orphan sweep, which keeps a file while any row points at it. A new index on `COALESCE(file_id, id)` keeps that statement and the preview update off a table scan.
- **One row per file counts in a quota**: a copied row is marked (`is_forwarded_copy`) and left out of its author's upload quota while the original holds the file; the quota index becomes partial on counted rows. When the counted row is deleted while copies keep the file, a trigger hands the quota to the oldest copy, with the original's compression state: otherwise uploading, forwarding and deleting the original would keep files on disk outside every quota (security audit). Copies are not queued for compression while the original is there: its compression moves them along; a copy that takes over a picture never tried is queued again.

## Consequences
- A forward takes one write transaction (the message and up to ten attachment rows), the same budget as a send with files, and no file I/O.
- Deleting the original does not delete its forwards, nor their files, as on Discord: a person who regrets a message must ask each forwarder too. The origin ids reach every reader of the forward, even one who cannot see the original's channel; they reveal that channel's id and the original's creation time, never its name or content.
- One file can outlive the message it was uploaded with, as long as a forward holds it; backups and the purge already count files by row, whatever the row. The first forwarder then carries the file in their quota.
- Older clients show a forward as an ordinary message from the forwarder, without the "Transféré" mark (they ignore `forwarded_from`).

## Alternatives considered
- **A live reference to the original** (no copy): the forward would change or vanish with the original, and readers of the destination would need the original channel's permission to see its files.
- **Copying each sealed file**: a full read, decrypt, seal and write per file, up to ten 25 MB files per forward, for bytes that never differ.
- **Counting copies in the quota**: forwarding would fill a person's quota without using disk.
- **Charging the original uploader for as long as any copy lives** (a `quota_owner_id` on each copy, the quota summed over distinct files): fairer to the forwarder, who inherits up to the file's size when the original goes, but every quota read becomes a grouping over files. The hand-over keeps the quota one indexed sum.
