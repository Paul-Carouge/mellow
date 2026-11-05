# 0049. Two-factor sign-in with time-based one-time passwords

- Status: Accepted
- Date: 2026-11-05

## Context
ROADMAP item 19 ("Accounts and profiles", owner 2026-11-02) asks for two-factor sign-in. A password alone falls to reuse and phishing; Discord, GitHub and most self-hosted services offer an authenticator app as the second factor. Mellow's promise is trivial self-hosting: no SMS gateway, no email delivery to configure, no third party. Users who lose their phone must still get back in, and an instance administrator must be able to help someone who lost everything, without a database editor.

## Decision
- **Time-based one-time passwords (RFC 6238)** with the parameters every authenticator app reads by default: HMAC-SHA-1, 6 digits, 30-second steps, a 160-bit random secret. A code is accepted for its step and one step on each side (clock drift). The code is computed with the HMAC-SHA-1 of `aws-lc-rs`, already a dependency of the server (attachment sealing, `str0m`), and compared with its constant-time comparison; all three candidate steps are always computed, so the timing says nothing about which one matched. No new crate: the algorithm is 20 lines (`domain::totp`), checked against RFC 6238's test vectors.
- **Replay protection**: `two_factor_secrets.last_used_time_step` keeps the newest step whose code was accepted, and a code is accepted only for a later step, in the same `UPDATE` that records it (one writer, ADR-0024). A code therefore works once, even for two requests racing with it, as RFC 6238 §5.2 asks.
- **Secret at rest**: stored in the SQLCipher database as is. The database key is derived from the instance master key kept outside the data directory (ADR-0006), and backups copy the encrypted database; a second sealing with another key derived from the same master key would protect against no one who cannot already read the database. Secrets never appear in logs, `Debug` output or caches (`Cache-Control: no-store`).
- **Enrolment** (settings → Sécurité): `POST /api/v1/users/@me/two-factor/enrolment` `{ current_password }` returns `{ secret }` (base32) for a new or replaced unconfirmed enrolment; the web client builds the `otpauth://totp/Mellow%20(<host>):<username>?secret=…&issuer=Mellow%20(<host>)…` URI and draws its QR code itself, as SVG, with a small in-house encoder: the secret never goes to a third party. `POST …/two-factor/enable` `{ code }` turns it on once a code made from that very secret is confirmed, and returns ten recovery codes, shown once. `GET …/two-factor` gives `{ is_enabled, recovery_codes_remaining }`.
- **Recovery codes**: 10 single-use codes of 80 random bits each, written `abcd-efgh-ijkl-mnop` in lowercase base32; case, dashes and spaces do not count when typed. Only their SHA-256 is stored, as for session tokens: 80 random bits cannot be guessed online nor brute-forced from a stolen hash, so a slow password hash would add only cost (an Argon2 run per stored code on each attempt) and no protection. A used code is deleted.
- **Sign-in**: when the password is right and two-factor sign-in is on, `POST /api/v1/auth/login` answers `202 Accepted` with `{ two_factor_token, expires_in_seconds }` instead of a session. `POST /api/v1/auth/login/two-factor` `{ two_factor_token, code }` takes a TOTP code or a recovery code and answers like a login (`SessionCreated`, the cookie for the web client). The challenge lives 5 minutes in memory (at most 10 000 at once), takes 5 wrong codes, and is spent by the first right one; the session opens only if the account is still enabled and still has the password that was checked. A restart drops pending challenges: the user types the password again.
- **Rate limits**: the second step counts toward the address's authentication budget (20 per minute), and toward 10 codes per 15 minutes per account, whatever the address or the challenge. An attacker who has the password thus tries at most 960 codes a day, about a 0.3 % chance a day against three accepted codes in a million. The settings endpoints share the current-password checks' budget (10 per 15 minutes per session, 50 per user).
- **Turning it off and new codes** need the current password and a code (TOTP or recovery): `POST …/two-factor/disable`, `POST …/two-factor/recovery-codes` (returns ten new codes and invalidates the old ones). Enrolment needs the password too, so a stolen session cannot put the owner's sign-in behind its own authenticator.
- **Administrator reset**: `mellow-server admin reset-two-factor --username <name>` turns it off for someone who lost both their device and their codes. It is audited in `two_factor_resets` (who and when), written in the same transaction; the guild audit log is per guild and has no place for instance actions.
- **Errors**: a wrong, reused or stale code is the new code `INVALID_TWO_FACTOR_CODE` (401), so clients can tell it from a wrong password (`INVALID_CREDENTIALS`); an unknown, expired or spent challenge is `UNAUTHORIZED`; enabling twice or proving a disabled factor is `CONFLICT`. Everything is additive to API v1.

## Consequences
- Older clients sign into accounts without two-factor sign-in exactly as before. Against an account with it on, they cannot sign in (they do not know the `202` answer): `mellow-client-core` reports it as an unexpected response until a native client adds the second step.
- Turning two-factor sign-in on or off does not sign out other sessions; changing the password still does.
- Rate limits and challenges are in memory (ADR-0002): a restart forgives counts, as for logins.
- Account recovery without codes depends on an administrator with shell access, who must check who is asking.

## Alternatives considered
- **WebAuthn / passkeys**: stronger against phishing, but needs per-browser work, attestation choices and, for native clients, platform APIs; a later ADR can add it next to TOTP.
- **Email or SMS codes**: need delivery infrastructure the instance does not have, and SMS is weak.
- **A crate (`totp-rs`, `qrcode`)**: more code than the algorithm itself and new entries in `cargo deny`; the QR code is drawn by the browser anyway.
- **Argon2 for recovery codes**: up to ten slow hashes per attempt, for codes too random to need them.
- **Challenge in the database**: survives restarts, but adds writes to every sign-in for a five-minute token.
