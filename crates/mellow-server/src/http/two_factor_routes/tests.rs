use mellow_protocol::{
    ApiErrorBody, ApiErrorCode, SessionCreated, TwoFactorChallenge, TwoFactorEnrolment,
    TwoFactorRecoveryCodes, TwoFactorStatus,
};

use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::tests::{PASSWORD, migrated_database};
use crate::domain::clock::now_unix_milliseconds;
use crate::domain::totp::{code_at_time_step, time_step};
use crate::http::test_support::*;
use crate::http::*;

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

/// The code of the step `steps_ahead` after the current one: within the accepted drift.
fn code(secret: &str, steps_ahead: u64) -> String {
    let step = time_step(now_unix_milliseconds()) + steps_ahead;
    let digits = code_at_time_step(&base32_decode(secret), step);
    String::from_utf8(digits.to_vec()).unwrap()
}

fn login() -> serde_json::Value {
    serde_json::json!({"login": "aurore", "password": PASSWORD, "device_name": "Phone"})
}

#[tokio::test]
async fn sign_in_with_two_factor_takes_a_challenge_then_a_code() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let token = registered_token(&state, "aurore").await;
    let me = "/api/v1/users/@me/two-factor";

    let (status, body) = send(
        &state,
        authorized(
            "POST",
            &format!("{me}/enrolment"),
            &token,
            Some(serde_json::json!({"current_password": PASSWORD})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let secret = json::<TwoFactorEnrolment>(&body).secret;
    let (status, body) = send(
        &state,
        authorized(
            "POST",
            &format!("{me}/enable"),
            &token,
            Some(serde_json::json!({"code": code(&secret, 0)})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json::<TwoFactorRecoveryCodes>(&body).recovery_codes.len(),
        10
    );
    let (_, body) = send(&state, authorized("GET", me, &token, None)).await;
    assert!(json::<TwoFactorStatus>(&body).is_enabled);

    let ((status, body), headers) =
        send_from(&state, TEST_PEER, post_json("/api/v1/auth/login", &login())).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
    let challenge: TwoFactorChallenge = json(&body);
    assert_eq!(challenge.expires_in_seconds, 300);
    let second_step = |code: &str| {
        post_json(
            "/api/v1/auth/login/two-factor",
            &serde_json::json!({"two_factor_token": challenge.two_factor_token, "code": code}),
        )
    };
    let (status, body) = send(&state, second_step("000000")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        json::<ApiErrorBody>(&body).code,
        ApiErrorCode::InvalidTwoFactorCode
    );
    // The step of the enabling code is spent: the next one signs in.
    let (status, body) = send(&state, second_step(&code(&secret, 1))).await;
    assert_eq!(status, StatusCode::OK);
    let session: SessionCreated = json(&body);
    let (status, _) = send(
        &state,
        authorized("GET", "/api/v1/users/@me", &session.token, None),
    )
    .await;
    let (spent, _) = send(&state, second_step(&code(&secret, 1))).await;
    database.close().await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(spent, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn codes_are_limited_per_account_across_challenges() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let token = registered_token(&state, "aurore").await;
    let (_, body) = send(
        &state,
        authorized(
            "POST",
            "/api/v1/users/@me/two-factor/enrolment",
            &token,
            Some(serde_json::json!({"current_password": PASSWORD})),
        ),
    )
    .await;
    let secret = json::<TwoFactorEnrolment>(&body).secret;
    send(
        &state,
        authorized(
            "POST",
            "/api/v1/users/@me/two-factor/enable",
            &token,
            Some(serde_json::json!({"code": code(&secret, 0)})),
        ),
    )
    .await;

    let mut statuses = Vec::new();
    // Three challenges of five attempts: the eleventh code on the account is refused.
    for _ in 0..3 {
        let (_, body) = send(&state, post_json("/api/v1/auth/login", &login())).await;
        let challenge: TwoFactorChallenge = json(&body);
        for _ in 0..5 {
            let (status, _) = send(
                &state,
                post_json(
                    "/api/v1/auth/login/two-factor",
                    &serde_json::json!({
                        "two_factor_token": challenge.two_factor_token,
                        "code": "000000",
                    }),
                ),
            )
            .await;
            statuses.push(status);
        }
    }
    database.close().await;

    assert!(
        statuses[..10]
            .iter()
            .all(|status| *status == StatusCode::UNAUTHORIZED)
    );
    assert!(
        statuses[10..]
            .iter()
            .all(|status| *status == StatusCode::TOO_MANY_REQUESTS),
        "{statuses:?}"
    );
}

#[tokio::test]
async fn an_unknown_challenge_is_unauthorized_and_enabling_twice_conflicts() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let token = registered_token(&state, "aurore").await;

    let (unknown, body) = send(
        &state,
        post_json(
            "/api/v1/auth/login/two-factor",
            &serde_json::json!({"two_factor_token": "mlw_nothing", "code": "123456"}),
        ),
    )
    .await;
    assert_eq!(json::<ApiErrorBody>(&body).code, ApiErrorCode::Unauthorized);
    let (no_enrolment, _) = send(
        &state,
        authorized(
            "POST",
            "/api/v1/users/@me/two-factor/enable",
            &token,
            Some(serde_json::json!({"code": "123456"})),
        ),
    )
    .await;
    let (wrong_password, _) = send(
        &state,
        authorized(
            "POST",
            "/api/v1/users/@me/two-factor/enrolment",
            &token,
            Some(serde_json::json!({"current_password": "not the password"})),
        ),
    )
    .await;
    database.close().await;

    assert_eq!(unknown, StatusCode::UNAUTHORIZED);
    assert_eq!(no_enrolment, StatusCode::CONFLICT);
    assert_eq!(wrong_password, StatusCode::UNAUTHORIZED);
}
