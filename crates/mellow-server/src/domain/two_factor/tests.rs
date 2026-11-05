use std::sync::Arc;

use mellow_protocol::LoginRequest;

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
        .enable(fixture.user.id, &code(&enrolment.secret, NOW), NOW)
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
    assert_eq!(
        fixture.two_factor.challenge_owner(&token, now),
        Some(fixture.user.id)
    );
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
    assert_eq!(fixture.two_factor.challenge_owner(&token, now), None);
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
        .enable(fixture.user.id, &code(&first.secret, NOW), NOW)
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
        .enable(fixture.user.id, &code(&second.secret, NOW), NOW)
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
            recovery_codes_remaining: 0
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
