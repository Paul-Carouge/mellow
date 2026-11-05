use std::sync::Arc;

use mellow_protocol::{ChangePasswordRequest, LoginRequest};

use super::*;
use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::tests::{NOW, PASSWORD, migrated_database, register_request};
use crate::domain::totp::{TOTP_STEP_SECONDS, code_at_time_step, time_step};

const STEP_MILLISECONDS: u64 = TOTP_STEP_SECONDS * 1_000;

struct Fixture {
    _directory: tempfile::TempDir,
    database: Database,
    accounts: Accounts,
    two_factor: TwoFactor,
    user: StoredUser,
    /// The session `register` opened.
    session_token: String,
}

async fn fixture() -> Fixture {
    let (directory, database) = migrated_database().await;
    let accounts = Accounts::new(
        database.clone(),
        RegistrationMode::Open,
        Arc::default(),
        Arc::default(),
    );
    let registered = accounts
        .register(register_request("aurore", "aurore@example.org"), NOW)
        .await
        .unwrap();
    let user = find_user_by_id(&database, registered.user.id)
        .await
        .unwrap()
        .unwrap();
    Fixture {
        _directory: directory,
        two_factor: TwoFactor::new(database.clone()),
        database,
        accounts,
        user,
        session_token: registered.token,
    }
}

fn code(secret: &str, now_unix_milliseconds: u64) -> String {
    let secret = base32_decode(secret);
    String::from_utf8(code_at_time_step(&secret, time_step(now_unix_milliseconds)).to_vec())
        .unwrap()
}

fn base32_decode(text: &str) -> Vec<u8> {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let (mut buffer, mut bits, mut bytes) = (0u32, 0, Vec::new());
    for character in text.bytes() {
        let value = alphabet.iter().position(|&a| a == character).unwrap();
        buffer = (buffer << 5) | u32::try_from(value).unwrap();
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            bytes.push(u8::try_from((buffer >> bits) & 0xFF).unwrap());
        }
    }
    bytes
}

/// Enrols the fixture's user and returns the secret and the recovery codes.
async fn enabled(fixture: &Fixture) -> (String, Vec<String>) {
    let enrolment = fixture
        .two_factor
        .start_enrolment(&fixture.user, PASSWORD.into(), NOW)
        .await
        .unwrap();
    let codes = fixture
        .two_factor
        .enable(
            &fixture.accounts,
            &fixture.session_token,
            fixture.user.id,
            &code(&enrolment.secret, NOW),
            NOW,
        )
        .await
        .unwrap();
    (enrolment.secret, codes.recovery_codes)
}

async fn challenge(fixture: &Fixture, now: Instant) -> String {
    let refusal = fixture
        .accounts
        .login(
            LoginRequest {
                login: "aurore".into(),
                password: PASSWORD.into(),
                device_name: "Phone".into(),
            },
            NOW,
        )
        .await;
    let Err(AccountError::SecondFactorRequired(login)) = refusal else {
        panic!("expected a second factor, got {refusal:?}");
    };
    fixture
        .two_factor
        .issue_challenge(*login, now)
        .unwrap()
        .two_factor_token
}

#[tokio::test]
async fn sign_in_needs_a_code_once_enabled_and_each_code_works_once() {
    let fixture = fixture().await;
    let before = fixture
        .accounts
        .login(
            LoginRequest {
                login: "aurore".into(),
                password: PASSWORD.into(),
                device_name: "Phone".into(),
            },
            NOW,
        )
        .await;
    assert!(before.is_ok(), "no second factor before enrolment");
    let (secret, _) = enabled(&fixture).await;
    let status = fixture.two_factor.status(fixture.user.id).await.unwrap();
    assert!(status.is_enabled);
    assert_eq!(status.recovery_codes_remaining, 10);

    let now = Instant::now();
    let later = NOW + STEP_MILLISECONDS;
    let token = challenge(&fixture, now).await;
    let wrong = fixture
        .two_factor
        .complete_sign_in(&fixture.accounts, &token, "000000", now, later)
        .await;
    assert!(
        matches!(wrong, Err(TwoFactorError::InvalidCode)),
        "{wrong:?}"
    );
    let session = fixture
        .two_factor
        .complete_sign_in(&fixture.accounts, &token, &code(&secret, later), now, later)
        .await
        .unwrap();
    assert_eq!(session.user.id, fixture.user.id);
    assert!(
        fixture
            .accounts
            .authenticate(&session.token, later)
            .await
            .is_ok()
    );

    let spent = fixture
        .two_factor
        .complete_sign_in(&fixture.accounts, &token, &code(&secret, later), now, later)
        .await;
    assert!(
        matches!(
            spent,
            Err(TwoFactorError::Account(AccountError::Unauthorized))
        ),
        "a challenge signs in once: {spent:?}"
    );
    let replay_token = challenge(&fixture, now).await;
    let replayed = fixture
        .two_factor
        .complete_sign_in(
            &fixture.accounts,
            &replay_token,
            &code(&secret, later),
            now,
            later,
        )
        .await;
    assert!(
        matches!(replayed, Err(TwoFactorError::InvalidCode)),
        "a code works once: {replayed:?}"
    );
    // The enrolment's own code (step of NOW) is older than the one used: refused too.
    let older = fixture
        .two_factor
        .complete_sign_in(
            &fixture.accounts,
            &replay_token,
            &code(&secret, NOW),
            now,
            later,
        )
        .await;
    assert!(
        matches!(older, Err(TwoFactorError::InvalidCode)),
        "{older:?}"
    );
    fixture.database.close().await;
}

#[tokio::test]
async fn a_recovery_code_signs_in_once_however_it_is_typed() {
    let fixture = fixture().await;
    let (_, recovery_codes) = enabled(&fixture).await;
    let now = Instant::now();
    let typed = recovery_codes[3].to_uppercase().replace('-', " ");

    let token = challenge(&fixture, now).await;
    fixture
        .two_factor
        .complete_sign_in(&fixture.accounts, &token, &typed, now, NOW)
        .await
        .unwrap();
    let token = challenge(&fixture, now).await;
    let again = fixture
        .two_factor
        .complete_sign_in(&fixture.accounts, &token, &recovery_codes[3], now, NOW)
        .await;

    assert!(
        matches!(again, Err(TwoFactorError::InvalidCode)),
        "{again:?}"
    );
    let status = fixture.two_factor.status(fixture.user.id).await.unwrap();
    assert_eq!(status.recovery_codes_remaining, 9);
    fixture.database.close().await;
}

#[tokio::test]
async fn a_challenge_expires_and_dies_after_five_wrong_codes() {
    let fixture = fixture().await;
    let (secret, _) = enabled(&fixture).await;
    let now = Instant::now();
    let later = NOW + 2 * STEP_MILLISECONDS;

    let expiring = challenge(&fixture, now).await;
    let expired = fixture
        .two_factor
        .complete_sign_in(
            &fixture.accounts,
            &expiring,
            &code(&secret, later),
            now + SIGN_IN_CHALLENGE_LIFETIME,
            later,
        )
        .await;
    assert!(
        matches!(
            expired,
            Err(TwoFactorError::Account(AccountError::Unauthorized))
        ),
        "{expired:?}"
    );

    let token = challenge(&fixture, now).await;
    for _ in 0..CODE_ATTEMPTS_PER_CHALLENGE {
        let wrong = fixture
            .two_factor
            .complete_sign_in(&fixture.accounts, &token, "not a code", now, later)
            .await;
        assert!(
            matches!(wrong, Err(TwoFactorError::InvalidCode)),
            "{wrong:?}"
        );
    }
    let right_but_too_late = fixture
        .two_factor
        .complete_sign_in(&fixture.accounts, &token, &code(&secret, later), now, later)
        .await;
    assert!(
        matches!(
            right_but_too_late,
            Err(TwoFactorError::Account(AccountError::Unauthorized))
        ),
        "{right_but_too_late:?}"
    );
    fixture.database.close().await;
}

#[tokio::test]
async fn enrolment_needs_the_password_and_a_code_from_its_own_secret() {
    let fixture = fixture().await;
    let wrong_password = fixture
        .two_factor
        .start_enrolment(&fixture.user, "not the password".into(), NOW)
        .await;
    assert!(
        matches!(
            wrong_password,
            Err(TwoFactorError::Account(AccountError::InvalidCredentials))
        ),
        "{wrong_password:?}"
    );
    let first = fixture
        .two_factor
        .start_enrolment(&fixture.user, PASSWORD.into(), NOW)
        .await
        .unwrap();
    assert_eq!(first.secret.len(), 32);
    // Restarting replaces the secret: a code from the first one no longer turns it on.
    let second = fixture
        .two_factor
        .start_enrolment(&fixture.user, PASSWORD.into(), NOW)
        .await
        .unwrap();
    let stale = fixture
        .two_factor
        .enable(
            &fixture.accounts,
            &fixture.session_token,
            fixture.user.id,
            &code(&first.secret, NOW),
            NOW,
        )
        .await;
    assert!(
        matches!(stale, Err(TwoFactorError::InvalidCode)),
        "{stale:?}"
    );
    assert!(
        !fixture
            .two_factor
            .status(fixture.user.id)
            .await
            .unwrap()
            .is_enabled
    );

    fixture
        .two_factor
        .enable(
            &fixture.accounts,
            &fixture.session_token,
            fixture.user.id,
            &code(&second.secret, NOW),
            NOW,
        )
        .await
        .unwrap();
    let again = fixture
        .two_factor
        .start_enrolment(&fixture.user, PASSWORD.into(), NOW)
        .await;
    assert!(
        matches!(again, Err(TwoFactorError::AlreadyEnabled)),
        "{again:?}"
    );
    fixture.database.close().await;
}

#[tokio::test]
async fn disabling_and_new_recovery_codes_need_the_password_and_a_code() {
    let fixture = fixture().await;
    let (secret, old_codes) = enabled(&fixture).await;
    let later = NOW + STEP_MILLISECONDS;

    let without_password = fixture
        .two_factor
        .disable(
            &fixture.user,
            "wrong password".into(),
            &code(&secret, later),
            later,
        )
        .await;
    assert!(
        matches!(
            without_password,
            Err(TwoFactorError::Account(AccountError::InvalidCredentials))
        ),
        "{without_password:?}"
    );
    let without_code = fixture
        .two_factor
        .regenerate_recovery_codes(&fixture.user, PASSWORD.into(), "123", later)
        .await;
    assert!(
        matches!(without_code, Err(TwoFactorError::InvalidCode)),
        "{without_code:?}"
    );

    let new_codes = fixture
        .two_factor
        .regenerate_recovery_codes(&fixture.user, PASSWORD.into(), &code(&secret, later), later)
        .await
        .unwrap();
    assert!(
        new_codes
            .recovery_codes
            .iter()
            .all(|code| !old_codes.contains(code))
    );
    let old_code = fixture
        .two_factor
        .disable(&fixture.user, PASSWORD.into(), &old_codes[0], later)
        .await;
    assert!(
        matches!(old_code, Err(TwoFactorError::InvalidCode)),
        "{old_code:?}"
    );

    fixture
        .two_factor
        .disable(
            &fixture.user,
            PASSWORD.into(),
            &new_codes.recovery_codes[0],
            later,
        )
        .await
        .unwrap();
    assert_eq!(
        fixture.two_factor.status(fixture.user.id).await.unwrap(),
        TwoFactorStatus {
            is_enabled: false,
            recovery_codes_remaining: 0,
            last_failed_code_at_unix_milliseconds: None,
        }
    );
    let session = fixture
        .accounts
        .login(
            LoginRequest {
                login: "aurore".into(),
                password: PASSWORD.into(),
                device_name: "Phone".into(),
            },
            later,
        )
        .await;
    assert!(session.is_ok(), "{session:?}");
    fixture.database.close().await;
}

fn login_request() -> LoginRequest {
    LoginRequest {
        login: "aurore".into(),
        password: PASSWORD.into(),
        device_name: "Laptop".into(),
    }
}

async fn session_count(database: &Database) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(database.readers())
        .await
        .unwrap()
}

#[tokio::test]
async fn a_challenge_dies_with_a_password_change_or_a_disabled_account() {
    let fixture = fixture().await;
    let (secret, _) = enabled(&fixture).await;
    let now = Instant::now();
    let later = NOW + STEP_MILLISECONDS;
    let before_password_change = challenge(&fixture, now).await;
    fixture
        .accounts
        .change_password(
            &fixture.session_token,
            ChangePasswordRequest {
                current_password: PASSWORD.into(),
                new_password: "another long password".into(),
            },
            NOW,
        )
        .await
        .unwrap();
    let sessions_before = session_count(&fixture.database).await;

    let after_password_change = fixture
        .two_factor
        .complete_sign_in(
            &fixture.accounts,
            &before_password_change,
            &code(&secret, later),
            now,
            later,
        )
        .await;

    assert!(
        matches!(
            after_password_change,
            Err(TwoFactorError::Account(AccountError::InvalidCredentials))
        ),
        "{after_password_change:?}"
    );
    assert_eq!(session_count(&fixture.database).await, sessions_before);
    fixture.database.close().await;
}

#[tokio::test]
async fn a_challenge_dies_with_a_disabled_account() {
    let fixture = fixture().await;
    let (secret, _) = enabled(&fixture).await;
    let now = Instant::now();
    let later = NOW + STEP_MILLISECONDS;
    let token = challenge(&fixture, now).await;
    sqlx::query("UPDATE users SET disabled_at_unix_milliseconds = ? WHERE id = ?")
        .bind(i64::try_from(NOW).unwrap())
        .bind(i64::try_from(fixture.user.id.raw()).unwrap())
        .execute(fixture.database.writer())
        .await
        .unwrap();
    let sessions_before = session_count(&fixture.database).await;

    let refused = fixture
        .two_factor
        .complete_sign_in(&fixture.accounts, &token, &code(&secret, later), now, later)
        .await;

    assert!(
        matches!(
            refused,
            Err(TwoFactorError::Account(AccountError::InvalidCredentials))
        ),
        "{refused:?}"
    );
    assert_eq!(session_count(&fixture.database).await, sessions_before);
    fixture.database.close().await;
}

#[tokio::test]
async fn one_account_holds_only_a_few_pending_sign_ins() {
    let fixture = fixture().await;
    let (secret, _) = enabled(&fixture).await;
    let now = Instant::now();
    let later = NOW + STEP_MILLISECONDS;
    let mut tokens = Vec::new();
    for second in 0..=MAXIMUM_PENDING_SIGN_INS_PER_USER {
        let issued_at = now + Duration::from_secs(u64::try_from(second).unwrap());
        tokens.push(challenge(&fixture, issued_at).await);
    }

    assert_eq!(
        fixture.two_factor.lock_pending_sign_ins().len(),
        MAXIMUM_PENDING_SIGN_INS_PER_USER
    );
    let oldest = fixture
        .two_factor
        .complete_sign_in(
            &fixture.accounts,
            &tokens[0],
            &code(&secret, later),
            now,
            later,
        )
        .await;
    assert!(
        matches!(
            oldest,
            Err(TwoFactorError::Account(AccountError::Unauthorized))
        ),
        "the oldest challenge made room: {oldest:?}"
    );
    let newest = fixture
        .two_factor
        .complete_sign_in(
            &fixture.accounts,
            tokens.last().unwrap(),
            &code(&secret, later),
            now,
            later,
        )
        .await;
    assert!(newest.is_ok(), "{newest:?}");
    fixture.database.close().await;
}

#[test]
fn every_tenth_wrong_code_in_a_row_locks_app_codes_longer() {
    let minutes = |count: u64| Some(Duration::from_mins(count));
    assert_eq!(code_lock_after(0), None);
    assert_eq!(code_lock_after(9), None);
    assert_eq!(code_lock_after(10), minutes(15));
    assert_eq!(code_lock_after(11), None);
    assert_eq!(code_lock_after(20), minutes(30));
    assert_eq!(code_lock_after(30), minutes(60));
    assert_eq!(code_lock_after(80), minutes(24 * 60));
    assert_eq!(code_lock_after(u32::MAX / 10 * 10), minutes(24 * 60));
}

/// Types `count` wrong codes at `at` (never a right one by chance), all refused as wrong.
async fn wrong_codes(fixture: &Fixture, count: u32, at: u64) {
    for _ in 0..count {
        let wrong = fixture
            .two_factor
            .check_code(fixture.user.id, "not a code", at)
            .await;
        assert!(
            matches!(wrong, Err(TwoFactorError::InvalidCode)),
            "{wrong:?}"
        );
    }
}

#[tokio::test]
async fn wrong_codes_lock_app_codes_across_windows_but_not_recovery_codes() {
    let fixture = fixture().await;
    let (secret, recovery_codes) = enabled(&fixture).await;
    let first_lock = 15 * 60 * 1_000;
    let mut now = NOW + STEP_MILLISECONDS;

    wrong_codes(&fixture, 10, now).await;
    let locked = fixture
        .two_factor
        .check_code(fixture.user.id, &code(&secret, now), now)
        .await;
    assert!(
        matches!(locked, Err(TwoFactorError::CodesLocked)),
        "{locked:?}"
    );
    // The lock is in the database: it holds for a new process too.
    let restarted = TwoFactor::new(fixture.database.clone());
    let still_locked = restarted
        .check_code(fixture.user.id, &code(&secret, now), now)
        .await;
    assert!(
        matches!(still_locked, Err(TwoFactorError::CodesLocked)),
        "{still_locked:?}"
    );
    let status = fixture.two_factor.status(fixture.user.id).await.unwrap();
    assert_eq!(status.last_failed_code_at_unix_milliseconds, Some(now));

    // Ten more wrong codes once it ends: twice as long, a window does not forgive them.
    now += first_lock;
    wrong_codes(&fixture, 10, now).await;
    let locked_longer = fixture
        .two_factor
        .check_code(
            fixture.user.id,
            &code(&secret, now + first_lock),
            now + first_lock,
        )
        .await;
    assert!(
        matches!(locked_longer, Err(TwoFactorError::CodesLocked)),
        "{locked_longer:?}"
    );
    // The owner's recovery code still works and lifts the lock.
    fixture
        .two_factor
        .check_code(fixture.user.id, &recovery_codes[0], now)
        .await
        .unwrap();
    fixture
        .two_factor
        .check_code(fixture.user.id, &code(&secret, now), now)
        .await
        .unwrap();
    // A right code forgives the streak: nine wrong codes do not lock.
    wrong_codes(&fixture, 9, now).await;
    let later = now + STEP_MILLISECONDS;
    fixture
        .two_factor
        .check_code(fixture.user.id, &code(&secret, later), later)
        .await
        .unwrap();
    fixture.database.close().await;
}

#[tokio::test]
async fn turning_two_factor_on_signs_out_the_other_sessions() {
    let fixture = fixture().await;
    let other = fixture.accounts.login(login_request(), NOW).await.unwrap();

    enabled(&fixture).await;

    assert!(
        fixture
            .accounts
            .authenticate(&other.token, NOW)
            .await
            .is_err(),
        "the other session ended"
    );
    assert!(
        fixture
            .accounts
            .authenticate(&fixture.session_token, NOW)
            .await
            .is_ok(),
        "the session that turned it on stays"
    );
    fixture.database.close().await;
}
