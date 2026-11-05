# 0048. Unique account names and permanent friend codes

- Status: Accepted
- Date: 2026-11-05

## Context
ROADMAP item 19 (owner, 2026-11-02): "one display name per account, unique whatever the case ("Paul" is found by paul, pAul…), capitals kept as typed, no separate @handle, renamed later after an availability check; plus a permanent friend code per account (for example LDOA-ODA8, never changes, points to the person whatever their name)". Until now an account had a `username` (the sign-in handle: lowercase ASCII, unique, changed only with the password) and a free `display_name` anyone could copy. Two people could both show as "Paul", and the handle was a second name to remember. `/api/v1` is released and must not break.

## Decision
- **One name per account.** It is 2 to 32 characters once composed (NFC), trimmed and with every run of white space made one space: letters of any script, digits, spaces, `_`, `.` and `-`; combining marks (accents, vowel signs) only after a letter; no control, invisible or reordering character (`is_invisible_format_character`); never the word "everyone" (ADR-0040). Capitals and accents are stored as typed.
- **Unique by a folded key**, stored in `users.name_key` with a unique index: NFKC, then each character upper- then lowercased (so "ß" meets "ss"), then NFKD with every combining mark removed. "Paul", "paul" and "pAul" are one name, and so are "Élodie", "elodie" and "ELODIE", so an accent cannot impersonate someone. Look-alike letters of different scripts (Cyrillic "а" and Latin "a") are not folded: known limit.
- **No separate handle.** On the wire `username` stays, as a deprecated alias always equal to `display_name`, so clients written for `/api/v1` keep working; the column `users.username` keeps the name too. In requests, `display_name` wins and `username` is read when it is absent (`RegisterRequest`, `UpdateCurrentUserRequest`).
- **Sign-in** accepts the name in any case and accents, or the email address: `users.name_key = key(login) OR users.email = login`, both indexed. Names never hold "@" and emails always do, so at most one account answers.
- **Renaming** (`PATCH /api/v1/users/@me`) needs no password any more (the friend code, not the name, is what others keep); it answers `USERNAME_TAKEN` when another account holds the key, and the account's own name may change case or accents. Old names are not reserved: anyone may take one at once. `GET /api/v1/users/name-availability?name=` answers `{"is_available": bool}` for the signed-in user, never who holds a name, and is limited to 30 checks a minute per user (`NAME_AVAILABILITY_CHECKS_PER_USER`); a name breaking the rules answers `VALIDATION_FAILED`.
- **Friend code**: 40 random bits from the operating system's generator (`getrandom`), stored in `users.friend_code` (unique index), shown as 8 symbols of Crockford's base 32 `XXXX-XXXX` (no I, L, O, U), and read in any case, with or without the dash or spaces, I and L as 1 and O as 0. It never changes. It is sent only to its owner (`CurrentUser.friend_code`), never in `PublicUser`. A new account draws again on the rare collision (up to four draws).
- **Friend requests**: `POST /api/v1/users/@me/relationships` takes `friend_code` (optional, wins) or `username` (now the name, found by its key). An unknown code, oneself or a block answer `NOT_FOUND`, as for names (ADR-0025), within the same friend-request rate limits.
- **Mentions** follow: a name with spaces or dashes is mentioned as written ("@Paul Carouge"); the longest name written after the "@" wins, on the server (`domain/mentions.rs`) and in the web client alike.
- **Search filters** `from:` and `mentions:` compare folded keys (`from:elodie` finds Élodie) and take a quoted name holding spaces (`from:"Paul Carouge"`); the web client quotes such names when it writes the filter.
- **Sign-in rate limits** per account are keyed on the folded key of the login, the same identity sign-in looks the account up by, so respelling a name (accents, full-width letters, spaces) shares one budget.
- **No "@name" line**: the web client no longer shows the name a second time under itself (profile card, user panel, member and ban lists, composer and people suggestions).
- **Migration 0041** adds `name_key` and `friend_code` (nullable, unique indexes). SQLite cannot fold Unicode, so the server fills both right after the migrations run, in one write transaction (`storage::account_names::give_names_and_friend_codes_to_older_accounts`), for every account still without them; afterwards it finds none and changes nothing. Oldest account first (`created_at`, then id), each keeps its display name, without the characters names may no longer hold ("Aurore 🌸" becomes "Aurore", "Jean-Paul (admin)" becomes "Jean-Paul admin"), or its old username when too little is left; when an older account already took that key, " 2", " 3"… is added (shortened to stay within 32 characters). Old usernames step aside first so a name never meets another account's old username.
- **Logs** never hold names or codes: the migration logs only how many accounts it named.

## Consequences
- Existing accounts sign in with their display name (any case) or their email, no longer with their old handle; renamed duplicates ("Paul 2") can rename themselves at once. The owner should tell members before updating.
- Messages written before the update that mention someone by their old handle ("@paul" for "Paul Carouge") no longer show a chip; who was notified is kept.
- Every account has an email (required since migration 0001), so a member whose name changed in the migration ("Aurore 🌸" to "Aurore", a duplicate to "Paul 2") can always sign in with it; the old handle is not kept as a sign-in alias.
- Mentions are still read from the text: someone named "Paul Merci" is the one mentioned by "@Paul merci", since the longest name wins. The composer sending the chosen mention ids would remove this; it is a protocol change left for later.
- The deprecated `username` will be removed only in a future API version.
- `mellow-server admin create-user --username` sets the name (`--display-name` wins when given); `admin set-administrator --username` finds the account by its folded name.

## Alternatives considered
- **Keep the handle and make display names unique**: two names per account, which the owner refused ("no separate @handle").
- **Case-insensitive only (`COLLATE NOCASE`)**: SQLite folds ASCII only, and "Élodie" against "elodie" would stay two accounts that look alike.
- **Reserve old names for a while**: the owner asked for none; the friend code is the stable pointer.
- **Friend codes from SQLite's `random()` in the migration**: not the OS generator the owner asked for, and collisions could not be retried in SQL.
- **Friend codes in `PublicUser`**: anyone seeing a member could add them by code; the code is for its owner to share.
