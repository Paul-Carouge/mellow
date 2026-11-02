# 0034. Shard attachment files by a mixed id

- Status: Accepted
- Date: 2026-11-02
- Amends [ADR-0020](0020-attachments-and-avatars-encrypted-on-disk.md) on where a file lives under `attachments/`. Everything else it decided still holds.

## Context
ADR-0020 stores each sealed file at `attachments/<two id hex digits>/<id>`, the two digits being the id's low byte (`id % 256`). Ids are Snowflakes (ADR-0004): 42 bits of timestamp, 10 of sequence and 12 reserved bits that are always zero. The low byte of every generated id is therefore zero, and every attachment, avatar, banner, icon, emoji, preview and sound of an instance lands in `attachments/00/`. The 256 directories were meant to keep each one small; one directory holding every file makes listings slow on some filesystems and backups harder to inspect.

## Decision
- **Shard rule**: the directory of a file is the two lowercase hex digits of the top byte of SplitMix64's finaliser applied to the id (`x ^= x >> 30; x *= 0xbf58476d1ce4e5b9; x ^= x >> 27; x *= 0x94d049bb133111eb; x ^= x >> 31`, wrapping). Every bit of the id moves every bit of the result, so ids that differ only in their timestamp spread evenly over the 256 directories (a test checks it on 25 600 generated ids). The lookup stays one path computed from the id, with no directory scan. The rule places files on disk and never changes without a move like the one below.
- **Files stored before**: at each start, `serve` moves every file whose directory is not the one the rule gives, after the database migrations and before answering any request. Each move is one `rename` inside `attachments/`, atomic on one filesystem: a crash leaves each file in its old or its new place, never neither, and the next start goes on. Temporary `.partial` files are left to the sweep. A file that cannot be moved is counted, logged without its id, and stays where it is. Only the counts are logged. With nothing to move, the pass is one listing of the directories (17 to 55 ms for 10 000 files on the development Mac, APFS, debug build); moving 10 000 small files out of `00/` took 1.0 to 2.1 s there.
- **Reads and deletes** look in the new place first, then in the old one (`id % 256`), so commands run before `serve` has moved anything (`backup`, `admin recompress-images`) and a file whose move failed still work. The fallback costs one more `stat` only for a file missing from its new place.
- The purge and `backup` list every directory, whatever their name, so they see files in either place; the backup copies the tree as it is and checks each file the database points at in either place.

## Consequences
- An instance upgraded to this version reorganises `attachments/` once, at its first start; a backup made before restores fine, its files moved at the next start.
- The fallback to `id % 256` stays in the code as long as instances from before may exist.

## Alternatives considered
- **The timestamp's low byte** (`(id >> 22) % 256`): spreads as well for generated ids, but any id built another way (imports, tests) would cluster; the mix costs a few nanoseconds.
- **Moving files lazily on first access**: no start-up pass, but files never read again would stay in `00/` forever and every read would keep the fallback.
- **A marker file recording the move as done**: saves the listing at each start, but adds state that a restored backup can contradict; the listing is cheap and makes every start self-healing.
