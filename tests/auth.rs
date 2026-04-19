//! End-to-end tests for the `AuthClient` against an in-process mock server.

use base64::Engine;
use third_eye_client::storage::AppStore;

/// Builds a minimal unsigned JWT whose payload sets `exp` to `exp_secs`.
fn make_jwt(exp_secs: i64) -> String {
    let header =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
    let payload = serde_json::json!({"exp": exp_secs, "sub": "user"});
    let payload_b64 =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
    format!("{header}.{payload_b64}.")
}

#[test]
fn login_persists_token_and_cookie() {
    let mut server = mockito::Server::new();
    let token = make_jwt(2_000_000_000);
    let body = serde_json::json!({"access_token": token, "status": "success"}).to_string();

    let login = server
        .mock("POST", "/api/v1/account/login")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_header(
            "set-cookie",
            "refresh_token=abc; Path=/; HttpOnly; Max-Age=3600",
        )
        .with_body(&body)
        .create();

    let store = AppStore::open_in_memory().unwrap();
    let outcome = store
        .auth()
        .login(&server.url(), "me@example.test", "secret")
        .expect("login succeeds");
    login.assert();
    assert_eq!(outcome.email, "me@example.test");
    assert!(outcome.access_exp_ms.is_some());

    // Reading back the session from the DB should return the same token.
    let session = store
        .auth()
        .current_session()
        .unwrap()
        .expect("session stored");
    assert_eq!(session.access_token, outcome.access_token);
    assert_eq!(session.email.as_deref(), Some("me@example.test"));
}

#[test]
fn login_surfaces_server_error() {
    let mut server = mockito::Server::new();
    let login = server
        .mock("POST", "/api/v1/account/login")
        .with_status(401)
        .with_header("content-type", "application/json")
        .with_body(r#"{"code":"Unauthorized","message":"Invalid username or password"}"#)
        .create();

    let store = AppStore::open_in_memory().unwrap();
    let err = store
        .auth()
        .login(&server.url(), "nobody@example.test", "wrong")
        .expect_err("should fail");
    login.assert();
    let msg = format!("{err}");
    assert!(msg.contains("401"), "error should mention 401: {msg}");
    assert!(store.auth().current_session().unwrap().is_none());
}

#[test]
fn logout_clears_local_session_even_on_server_error() {
    let mut login_server = mockito::Server::new();
    let token = make_jwt(2_000_000_000);
    let login_body = serde_json::json!({"access_token": token, "status": "success"}).to_string();
    login_server
        .mock("POST", "/api/v1/account/login")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(login_body)
        .create();
    login_server
        .mock("GET", "/api/v1/account/logout")
        .with_status(500)
        .create();

    let store = AppStore::open_in_memory().unwrap();
    store
        .auth()
        .login(&login_server.url(), "me@example.test", "pw")
        .unwrap();
    assert!(store.auth().current_session().unwrap().is_some());

    let result = store.auth().logout(&login_server.url());
    assert!(result.is_err(), "server returned 500");
    // Local session cleared regardless of server outcome.
    assert!(store.auth().current_session().unwrap().is_none());
}
