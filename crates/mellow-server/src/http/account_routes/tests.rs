use axum::body::Body;
use mellow_protocol::{ApiErrorBody, ApiErrorCode, CurrentUser, SessionCreated};

use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::tests::{PASSWORD, migrated_database};
use crate::http::test_support::*;
use crate::http::*;

fn with_bearer(path: &str, method: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn register_me_logout_round_trip() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);

    let (status, body) = send(
        &state,
        post_json("/api/v1/auth/register", &registration("aurore")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session: SessionCreated = json(&body);

    let (status, body) = send(
        &state,
        with_bearer("/api/v1/users/@me", "GET", &session.token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json::<CurrentUser>(&body), session.user);

    let (status, _) = send(
        &state,
        with_bearer("/api/v1/auth/logout", "POST", &session.token),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = send(
        &state,
        with_bearer("/api/v1/users/@me", "GET", &session.token),
    )
    .await;
    database.close().await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json::<ApiErrorBody>(&body).code, ApiErrorCode::Unauthorized);
}

#[tokio::test]
async fn the_web_client_signs_in_with_a_cookie_usable_only_from_this_origin() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let mut register = post_json("/api/v1/auth/register", &registration("ada"));
    register.headers_mut().insert(
        "x-mellow-session-transport",
        HeaderValue::from_static("cookie"),
    );
    let ((status, body), headers) = send_from(&state, TEST_PEER, register).await;
    assert_eq!(status, StatusCode::CREATED);
    let session: SessionCreated = json(&body);
    assert_eq!(session.token, "", "the token stays out of page scripts");
    let set_cookie = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    assert!(set_cookie.contains("HttpOnly") && set_cookie.contains("SameSite=Strict"));
    let cookie = set_cookie.split(';').next().unwrap().to_owned();

    let request_from = |origin: &str| {
        Request::get("/api/v1/users/@me")
            .header(header::COOKIE, &cookie)
            .header(header::HOST, "chat.example.org")
            .header(header::ORIGIN, origin)
            .body(Body::empty())
            .unwrap()
    };
    let (same_origin, _) = send(&state, request_from("https://chat.example.org")).await;
    let (other_origin, _) = send(&state, request_from("https://evil.example")).await;
    // HTTP/2 names the host in `:authority` (the URI) and sends no `Host` header.
    let over_http2 = |host: Option<&str>| {
        let mut request = Request::get("https://chat.example.org/api/v1/users/@me")
            .header(header::COOKIE, &cookie)
            .header(header::ORIGIN, "https://chat.example.org");
        if let Some(host) = host {
            request = request.header(header::HOST, host);
        }
        request.body(Body::empty()).unwrap()
    };
    let (same_origin_over_http2, _) = send(&state, over_http2(None)).await;
    let (two_different_hosts, _) = send(&state, over_http2(Some("evil.example"))).await;
    let logout = Request::post("/api/v1/auth/logout")
        .header(header::COOKIE, &cookie)
        .header(header::HOST, "chat.example.org")
        .header(header::ORIGIN, "https://chat.example.org")
        .body(Body::empty())
        .unwrap();
    let ((logged_out, _), logout_headers) = send_from(&state, TEST_PEER, logout).await;
    let (after_logout, _) = send(&state, request_from("https://chat.example.org")).await;
    database.close().await;

    assert_eq!(same_origin, StatusCode::OK);
    assert_eq!(other_origin, StatusCode::UNAUTHORIZED);
    assert_eq!(same_origin_over_http2, StatusCode::OK);
    assert_eq!(two_different_hosts, StatusCode::BAD_REQUEST);
    assert_eq!(logged_out, StatusCode::NO_CONTENT);
    assert!(
        logout_headers
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    assert_eq!(after_logout, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn authentication_attempts_are_rate_limited_per_address() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Closed);
    let attempt = || post_json("/api/v1/auth/register", &registration("aurore"));
    for _ in 0..rate_limit::AUTHENTICATION_ATTEMPTS_PER_ADDRESS.maximum_events {
        let (status, _) = send(&state, attempt()).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "registration is closed");
    }
    let (status, body) = send(&state, attempt()).await;
    database.close().await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(json::<ApiErrorBody>(&body).code, ApiErrorCode::RateLimited);
}

#[tokio::test]
async fn one_address_cycling_login_names_cannot_fill_the_rate_limiter() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Closed);
    let attempts = rate_limit::AUTHENTICATION_ATTEMPTS_PER_ADDRESS.maximum_events * 3;
    for index in 0..attempts {
        send_from(
            &state,
            "198.51.100.1:1000",
            wrong_login(&format!("n{index}")),
        )
        .await;
    }
    database.close().await;
    // The address key, then two keys per attempt the address limit let through: refused
    // attempts add none, and signed-in users' limits live in their own table.
    let allowed = rate_limit::AUTHENTICATION_ATTEMPTS_PER_ADDRESS.maximum_events as usize;
    assert_eq!(
        state.unauthenticated_rate_limiter.tracked_key_count(),
        1 + 2 * allowed
    );
    assert_eq!(state.rate_limiter.tracked_key_count(), 0);
}

#[tokio::test]
async fn a_flood_from_one_ipv6_site_does_not_lock_other_clients_out() {
    // Each attempt from another /64 of one /48: without the site limit, every one would add an
    // address key until the table is full (see the rate_limit unit tests). Registration is
    // closed, so the flood costs no password hashing and the test stays quick.
    let flood_attempts = 3 * rate_limit::AUTHENTICATING_ADDRESSES_PER_IPV6_SITE.maximum_events;
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Closed);
    for index in 0..flood_attempts {
        send_from(
            &state,
            &format!("[2001:db8:1:{index:x}::1]:1000"),
            post_json("/api/v1/auth/register", &registration(&format!("n{index}"))),
        )
        .await;
    }
    // One site key, then one address key per attempt the site limit let through.
    let allowed = rate_limit::AUTHENTICATING_ADDRESSES_PER_IPV6_SITE.maximum_events as usize;
    assert_eq!(
        state.unauthenticated_rate_limiter.tracked_key_count(),
        1 + allowed
    );
    let ((same_site_status, _), _) =
        send_from(&state, "[2001:db8:1:ffff::1]:1000", wrong_login("aurore")).await;
    let ((other_ipv4_client_status, _), _) =
        send_from(&state, "203.0.113.50:2000", wrong_login("aurore")).await;
    let ((other_ipv6_site_status, _), _) =
        send_from(&state, "[2001:db8:2::1]:1000", wrong_login("aurore")).await;
    database.close().await;
    assert_eq!(same_site_status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(other_ipv4_client_status, StatusCode::UNAUTHORIZED);
    assert_eq!(other_ipv6_site_status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn one_ipv6_client_over_its_limit_does_not_lock_its_site_neighbours_out() {
    // Many households share a carrier's or ISP's /48: one /64 hammering the login route must
    // only lock itself out, not use up the site's budget.
    let flood_attempts = 2 * rate_limit::AUTHENTICATING_ADDRESSES_PER_IPV6_SITE.maximum_events;
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Closed);
    for index in 0..flood_attempts {
        send_from(
            &state,
            "[2001:db8:1:1::1]:1000",
            post_json("/api/v1/auth/register", &registration(&format!("n{index}"))),
        )
        .await;
    }
    let ((flooder_status, _), _) =
        send_from(&state, "[2001:db8:1:1::1]:1000", wrong_login("aurore")).await;
    let ((neighbour_status, _), _) =
        send_from(&state, "[2001:db8:1:2::1]:1000", wrong_login("aurore")).await;
    database.close().await;
    assert_eq!(flooder_status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(neighbour_status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_attempts_are_limited_per_account_and_address_before_the_password_check() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    send(
        &state,
        post_json("/api/v1/auth/register", &registration("aurore")),
    )
    .await;
    let attacker = "198.51.100.1:1000";
    for _ in 0..rate_limit::LOGIN_ATTEMPTS_PER_ACCOUNT_AND_ADDRESS.maximum_events {
        let ((status, _), _) = send_from(&state, attacker, wrong_login("aurore")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    // Even the right password is refused from that address now: the attempt is counted first.
    let right_password = post_json(
        "/api/v1/auth/login",
        &serde_json::json!({"login": "Aurore", "password": PASSWORD, "device_name": "d"}),
    );
    let ((status, body), _) = send_from(&state, attacker, right_password).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(json::<ApiErrorBody>(&body).code, ApiErrorCode::RateLimited);

    // The real user, from another address, is not locked out by the attacker.
    let from_home = post_json(
        "/api/v1/auth/login",
        &serde_json::json!({"login": "aurore", "password": PASSWORD, "device_name": "d"}),
    );
    let ((status, _), headers) = send_from(&state, "203.0.113.50:2000", from_home).await;
    database.close().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
}

#[tokio::test]
async fn forwarded_for_from_the_loopback_proxy_separates_clients() {
    let (_directory, database) = migrated_database().await;
    let mut state = test_state(database.clone(), RegistrationMode::Closed);
    state.trusted_proxy_networks = client_address::build_trusted_proxy_networks(true, &[]).into();
    let via_proxy = |client: &str| {
        let mut request = wrong_login("nobody");
        request
            .headers_mut()
            .insert("x-forwarded-for", client.parse().unwrap());
        request
    };
    for _ in 0..rate_limit::AUTHENTICATION_ATTEMPTS_PER_ADDRESS.maximum_events {
        send_from(&state, "127.0.0.1:3000", via_proxy("203.0.113.1")).await;
    }
    let ((limited, _), _) = send_from(&state, "127.0.0.1:3000", via_proxy("203.0.113.1")).await;
    let ((other_client, _), _) =
        send_from(&state, "127.0.0.1:3000", via_proxy("203.0.113.2")).await;
    database.close().await;
    assert_eq!(limited, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(other_client, StatusCode::UNAUTHORIZED);
}

async fn logged_in_token(state: &AppState, username: &str, device_name: &str) -> String {
    let (_, body) = send(
        state,
        post_json(
            "/api/v1/auth/login",
            &serde_json::json!({"login": username, "password": PASSWORD, "device_name": device_name}),
        ),
    )
    .await;
    json::<SessionCreated>(&body).token
}

#[tokio::test]
async fn the_chosen_status_is_stored_and_validated() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let token = registered_token(&state, "aurore").await;
    let put = |body: serde_json::Value| {
        authorized("PUT", "/api/v1/users/@me/presence", &token, Some(body))
    };

    let (status, body) = send(
        &state,
        put(serde_json::json!({"status": "do_not_disturb", "custom_status": "  Focus  "})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let updated = json::<CurrentUser>(&body);
    assert_eq!(updated.status, mellow_protocol::ChosenStatus::DoNotDisturb);
    assert_eq!(updated.custom_status.as_deref(), Some("Focus"));
    let (status, body) = send(
        &state,
        put(serde_json::json!({"status": "online", "custom_status": "x".repeat(129)})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        json::<ApiErrorBody>(&body).code,
        ApiErrorCode::ValidationFailed
    );
    let (status, _) = send(&state, put(serde_json::json!({"status": "away"}))).await;
    assert!(status.is_client_error());
    let (_, body) = send(&state, authorized("GET", "/api/v1/users/@me", &token, None)).await;
    let stored = json::<CurrentUser>(&body);
    assert_eq!(stored.status, mellow_protocol::ChosenStatus::DoNotDisturb);
    assert_eq!(stored.custom_status.as_deref(), Some("Focus"));

    let (_, body) = send(&state, put(serde_json::json!({"status": "invisible"}))).await;
    let cleared = json::<CurrentUser>(&body);
    assert_eq!(cleared.status, mellow_protocol::ChosenStatus::Invisible);
    assert_eq!(cleared.custom_status, None);
    database.close().await;
}

/// A stolen session that uses up its password-check budget must not keep the owner's other
/// devices from changing the password (and so revoking it).
#[tokio::test]
async fn password_checks_and_profile_writes_are_rate_limited_per_session() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let stolen = registered_token(&state, "aurore").await;
    let own = logged_in_token(&state, "aurore", "own").await;
    let more_sessions = [
        logged_in_token(&state, "aurore", "third").await,
        logged_in_token(&state, "aurore", "fourth").await,
    ];
    let change = |token: &str, current: &str| {
        authorized(
            "POST",
            "/api/v1/users/@me/password",
            token,
            Some(serde_json::json!({
                "current_password": current,
                "new_password": "a brand new password",
            })),
        )
    };
    let rename = |token: &str, display_name: String| {
        authorized(
            "PATCH",
            "/api/v1/users/@me",
            token,
            Some(serde_json::json!({ "display_name": display_name })),
        )
    };

    // More than the user's budget too: refused checks must not count against it.
    let mut guesses = Vec::new();
    for _ in 0..=rate_limit::PASSWORD_CHECKS_PER_USER.maximum_events {
        guesses.push(send(&state, change(&stolen, "a wrong guess")).await.0);
    }
    let mut stolen_renames = Vec::new();
    for attempt in 0..=rate_limit::PROFILE_WRITES_PER_SESSION.maximum_events {
        stolen_renames.push(
            send(&state, rename(&stolen, format!("Thief {attempt}")))
                .await
                .0,
        );
    }
    let (stolen_with_right_password, _) = send(&state, change(&stolen, PASSWORD)).await;
    let mut renames = Vec::new();
    for attempt in 0..=rate_limit::PROFILE_WRITES_PER_SESSION.maximum_events {
        renames.push(
            send(&state, rename(&own, format!("Aurore {attempt}")))
                .await
                .0,
        );
    }
    // Sessions together hit the user's ceiling: 10 + 10 allowed above, 10 more here.
    let mut third_renames = Vec::new();
    for attempt in 0..rate_limit::PROFILE_WRITES_PER_SESSION.maximum_events {
        third_renames.push(
            send(
                &state,
                rename(&more_sessions[0], format!("Third {attempt}")),
            )
            .await
            .0,
        );
    }
    let (fourth_rename, _) = send(&state, rename(&more_sessions[1], "Fourth".into())).await;
    let (own_change, _) = send(&state, change(&own, PASSWORD)).await;
    let (stolen_after_change, _) =
        send(&state, with_bearer("/api/v1/users/@me", "GET", &stolen)).await;
    database.close().await;

    let session_budget = rate_limit::PASSWORD_CHECKS_PER_SESSION.maximum_events as usize;
    let (checked, refused) = guesses.split_at(session_budget);
    assert!(
        checked
            .iter()
            .all(|status| *status == StatusCode::UNAUTHORIZED)
    );
    assert!(
        refused
            .iter()
            .all(|status| *status == StatusCode::TOO_MANY_REQUESTS)
    );
    assert_eq!(stolen_renames.last(), Some(&StatusCode::TOO_MANY_REQUESTS));
    assert_eq!(stolen_with_right_password, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(own_change, StatusCode::NO_CONTENT);
    assert_eq!(stolen_after_change, StatusCode::UNAUTHORIZED);
    assert!(third_renames.iter().all(|status| *status == StatusCode::OK));
    assert_eq!(fourth_rename, StatusCode::TOO_MANY_REQUESTS);
    let (last, allowed) = renames.split_last().unwrap();
    assert!(allowed.iter().all(|status| *status == StatusCode::OK));
    assert_eq!(*last, StatusCode::TOO_MANY_REQUESTS);
}
