//! Shared plumbing for every authenticated call against the third-eye server.
//!
//! All server calls in this crate go through [`ApiSession`], which turns "a
//! bearer token" into "a session that keeps itself alive". Callers never pass
//! an access token around and never have to reason about expiry; each call:
//!
//!   1. acquires an access token, proactively refreshing it through the
//!      persisted `HttpOnly` refresh cookie when the recorded `exp` is at or
//!      near now (or is unknown), and
//!   2. if the server rejects it anyway with 401/403, refreshes once more and
//!      retries the call exactly once.
//!
//! The server rotates the refresh cookie on every successful refresh (see
//! `storage::auth`), so the practical effect is that a user who signed in
//! once stays signed in indefinitely — across restarts, across access-token
//! expiry, and across server-side access-token invalidation — for as long as
//! the app keeps refreshing before the *current* refresh token's own expiry,
//! not just the original one issued at login.
//!
//! Only a rejected *refresh* ends the session. That (and only that) clears the
//! local session and surfaces [`ApiError::SessionExpired`], which the UI maps
//! back to the sign-in form. Transport failures deliberately do **not** clear
//! the session: this app is expected to run offshore with no internet access,
//! where every server call fails but the user is still "signed in" as far as
//! their cached devices and active-device selection are concerned.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use reqwest::StatusCode;
use reqwest::Url;
use third_eye_openapi::apis::Error as GeneratedApiError;
use third_eye_openapi::apis::configuration as generated_configuration;

use super::auth::{AuthClient, AuthError};

/// Refresh an access token this long before its recorded expiry, so a call
/// that takes a moment to reach the server doesn't race the deadline.
const ACCESS_TOKEN_EXPIRY_SKEW_MS: i64 = 30_000;

/// Floor on how often a *proactive* refresh may fire for an access token whose
/// expiry is unknown (i.e. whose `exp` claim couldn't be decoded). Such a token
/// is treated as stale — we never assume a token we can't reason about is still
/// good — but without this floor that would cost an extra refresh round-trip on
/// every single call.
const UNKNOWN_EXPIRY_REFRESH_INTERVAL_MS: i64 = 5 * 60 * 1000;

/// Error surface shared by every server call. Replaces the per-domain
/// `DevicesError`/`SearchError` types, so all callers get the same
/// `SessionExpired` signal to react to.
#[derive(Debug)]
pub enum ApiError {
    Server {
        status: StatusCode,
        message: String,
    },
    Transport(anyhow::Error),
    /// No local session at all: the user has never signed in, or signed out.
    NotAuthenticated,
    /// The refresh token itself was rejected (expired, revoked, or the user was
    /// kicked out server-side), so the session is over and the local session
    /// has already been cleared. The user must sign in again.
    SessionExpired,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Server { status, message } => {
                write!(f, "server request failed (HTTP {status}): {message}")
            }
            ApiError::Transport(err) => write!(f, "network or decoding failure: {err:#}"),
            ApiError::NotAuthenticated => f.write_str("no active session; please sign in"),
            ApiError::SessionExpired => f.write_str("session expired; please sign in again"),
        }
    }
}

impl std::error::Error for ApiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // `anyhow::Error` does not implement `std::error::Error` directly.
        None
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        ApiError::Transport(err)
    }
}

impl From<AuthError> for ApiError {
    fn from(err: AuthError) -> Self {
        match err {
            AuthError::Server { status, message } => ApiError::Server { status, message },
            AuthError::Transport(err) => ApiError::Transport(err),
            AuthError::NotAuthenticated => ApiError::NotAuthenticated,
        }
    }
}

impl ApiError {
    /// True when this error means the user is no longer signed in, so the UI
    /// should fall back to the sign-in form. Distinguishes "the session is
    /// over" from "this particular call failed" (e.g. offline).
    pub const fn ends_session(&self) -> bool {
        matches!(self, ApiError::SessionExpired | ApiError::NotAuthenticated)
    }
}

/// Statuses that mean "this token isn't acceptable", i.e. the ones worth
/// retrying after a refresh.
fn is_auth_rejection(status: StatusCode) -> bool {
    status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN
}

/// Authenticated-call facade held by `AppStore` and handed to every domain
/// client (`DevicesClient`, `SearchClient`, ...). Cloneable and `Send` so it
/// can be moved into background worker threads.
#[derive(Clone)]
pub struct ApiSession {
    auth: Arc<AuthClient>,
    /// Unix-ms of the last refresh we performed. Used only to rate-limit
    /// proactive refreshes of tokens with an unknown expiry.
    last_refresh_ms: Arc<Mutex<i64>>,
}

impl ApiSession {
    pub(crate) fn new(auth: Arc<AuthClient>) -> Self {
        Self {
            auth,
            last_refresh_ms: Arc::new(Mutex::new(0)),
        }
    }

    /// True when a local session exists. Says nothing about whether its access
    /// token is currently fresh — that's [`ApiSession::access_token`]'s job.
    pub fn has_session(&self) -> bool {
        self.auth
            .current_session()
            .ok()
            .flatten()
            .is_some_and(|session| !session.access_token.trim().is_empty())
    }

    /// Returns an access token that is as fresh as we can make it, refreshing
    /// through the refresh cookie first when the current one is at/near expiry
    /// or has an expiry we can't read.
    pub fn access_token(&self, server_base: &str) -> Result<String, ApiError> {
        let session = self
            .auth
            .current_session()
            .map_err(ApiError::Transport)?
            .ok_or(ApiError::NotAuthenticated)?;
        let token = session.access_token;
        if token.trim().is_empty() {
            return Err(ApiError::NotAuthenticated);
        }
        let now_ms = now_ms();
        let needs_refresh = match session.access_exp_ms {
            Some(exp_ms) => exp_ms <= now_ms + ACCESS_TOKEN_EXPIRY_SKEW_MS,
            None => now_ms - self.last_refresh() >= UNKNOWN_EXPIRY_REFRESH_INTERVAL_MS,
        };
        if !needs_refresh {
            return Ok(token);
        }
        match self.refresh(server_base) {
            Ok(refreshed) => Ok(refreshed),
            // Couldn't reach the server to refresh. Hand back what we have and
            // let the call itself decide: it may still be accepted (clock skew,
            // server-side grace), and if it isn't, the 401 retry path gets
            // another chance at refreshing.
            Err(ApiError::Transport(err)) => {
                eprintln!(
                    "third-eye-client: proactive token refresh failed, using existing token: {err:#}"
                );
                Ok(token)
            }
            Err(err) => Err(err),
        }
    }

    /// Forces a refresh through the persisted refresh cookie, mapping a
    /// rejected cookie to [`ApiError::SessionExpired`] and clearing the local
    /// session in that case.
    pub fn refresh(&self, server_base: &str) -> Result<String, ApiError> {
        match self.auth.refresh(server_base) {
            Ok(token) => {
                self.record_refresh();
                Ok(token)
            }
            Err(AuthError::Server { status, message }) if is_auth_rejection(status) => {
                eprintln!(
                    "third-eye-client: refresh token rejected (HTTP {status}): {message}. Signing out locally."
                );
                // Clear locally rather than calling `logout`: the server has
                // already told us the cookie is no good, so the round-trip
                // would be pointless.
                self.auth.clear_local_session();
                Err(ApiError::SessionExpired)
            }
            Err(err) => Err(err.into()),
        }
    }

    /// Runs a generated-client call with a valid bearer token, transparently
    /// refreshing and retrying once if the server rejects that token.
    ///
    /// `operation` receives an owned [`generated_configuration::Configuration`]
    /// (rather than a reference) so the future it returns can own it. It may be
    /// invoked twice — once per attempt — hence the `Fn` bound: callers needing
    /// owned arguments should clone them inside the closure.
    pub fn call<T, E, F, Fut>(
        &self,
        server_base: &str,
        http: &reqwest::Client,
        operation: F,
    ) -> Result<T, ApiError>
    where
        F: Fn(generated_configuration::Configuration) -> Fut,
        Fut: std::future::Future<Output = Result<T, GeneratedApiError<E>>>,
    {
        self.call_with_token(server_base, |access_token| {
            let configuration = configuration_for(server_base, http, access_token)?;
            block_on(operation(configuration))?.map_err(map_generated_error)
        })
    }

    /// Same refresh-and-retry contract as [`ApiSession::call`], for calls that
    /// don't go through the generated client (see `storage::search`, which
    /// hand-rolls `/api/v1/search`). `operation` is handed a valid access token
    /// and must report HTTP failures as [`ApiError::Server`] for the retry to
    /// kick in.
    pub fn call_with_token<T, F>(&self, server_base: &str, operation: F) -> Result<T, ApiError>
    where
        F: Fn(&str) -> Result<T, ApiError>,
    {
        let token = self.access_token(server_base)?;
        match operation(&token) {
            Err(ApiError::Server { status, message }) if is_auth_rejection(status) => {
                // The access token was rejected mid-flight: either our expiry
                // bookkeeping was wrong, or the server invalidated it early
                // (restart, key rotation, forced re-auth). Refresh and retry
                // exactly once — a second rejection is a genuine error.
                eprintln!(
                    "third-eye-client: access token rejected (HTTP {status}): {message}. Refreshing and retrying once."
                );
                let refreshed = self.refresh(server_base)?;
                operation(&refreshed)
            }
            other => other,
        }
    }

    fn last_refresh(&self) -> i64 {
        *self
            .last_refresh_ms
            .lock()
            .expect("api session refresh clock mutex poisoned")
    }

    fn record_refresh(&self) {
        *self
            .last_refresh_ms
            .lock()
            .expect("api session refresh clock mutex poisoned") = now_ms();
    }
}

/// Builds a generated-client `Configuration` pointing at `server_base` and
/// authenticated with `access_token`.
pub fn configuration_for(
    server_base: &str,
    http: &reqwest::Client,
    access_token: &str,
) -> Result<generated_configuration::Configuration, ApiError> {
    let mut configuration = generated_configuration::Configuration::new();
    let base_url = Url::parse(server_base.trim())
        .with_context(|| format!("invalid server URL {}", server_base.trim()))
        .map_err(ApiError::Transport)?;
    base_url
        .as_str()
        .trim_end_matches('/')
        .clone_into(&mut configuration.base_path);
    configuration.user_agent = None;
    configuration.client = http.clone();
    configuration.bearer_access_token = Some(access_token.to_owned());
    Ok(configuration)
}

/// Drives a single generated-client future to completion on a throwaway
/// current-thread runtime. Every call site is already synchronous (the Slint UI
/// thread, or a dedicated worker thread), so there's no ambient runtime to
/// reuse. Note that refreshes always happen *outside* this call, so we never
/// try to start a runtime from within one.
fn block_on<T>(future: impl std::future::Future<Output = T>) -> Result<T, ApiError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("creating runtime for third-eye server call")
        .map_err(ApiError::Transport)?;
    Ok(runtime.block_on(future))
}

fn map_generated_error<E>(error: GeneratedApiError<E>) -> ApiError {
    match error {
        GeneratedApiError::ResponseError(content) => ApiError::Server {
            status: content.status,
            message: content.content,
        },
        GeneratedApiError::Reqwest(error) => ApiError::Transport(anyhow::anyhow!(error)),
        GeneratedApiError::Serde(error) => ApiError::Transport(anyhow::anyhow!(error)),
        GeneratedApiError::Io(error) => ApiError::Transport(anyhow::anyhow!(error)),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_rejections_are_the_refreshable_statuses() {
        assert!(is_auth_rejection(StatusCode::UNAUTHORIZED));
        assert!(is_auth_rejection(StatusCode::FORBIDDEN));
        assert!(!is_auth_rejection(StatusCode::NOT_FOUND));
        assert!(!is_auth_rejection(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(!is_auth_rejection(StatusCode::OK));
    }

    #[test]
    fn only_session_ending_errors_sign_the_user_out() {
        assert!(ApiError::SessionExpired.ends_session());
        assert!(ApiError::NotAuthenticated.ends_session());
        // A failed call must never sign the user out on its own: the app has to
        // keep working offshore with no route to the server.
        assert!(
            !ApiError::Transport(anyhow::anyhow!("offline")).ends_session(),
            "transport failures must not end the session"
        );
        assert!(
            !ApiError::Server {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: String::new(),
            }
            .ends_session()
        );
    }

    #[test]
    fn configuration_normalises_base_path_and_sets_bearer_token() {
        let configuration =
            configuration_for("  https://example.test/  ", &reqwest::Client::new(), "tok").unwrap();
        assert_eq!(configuration.base_path, "https://example.test");
        assert_eq!(configuration.bearer_access_token.as_deref(), Some("tok"));
    }

    // ---- ApiSession::call_with_token retry-on-401 --------------------------

    use base64::Engine;
    use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

    fn make_jwt(exp_secs: i64) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"alg":"none","typ":"JWT"}"#);
        let payload = serde_json::json!({"exp": exp_secs, "sub": "user"});
        let payload_b64 =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
        format!("{header}.{payload_b64}.")
    }

    #[test]
    fn call_with_token_refreshes_and_retries_once_after_401() {
        let mut server = mockito::Server::new();
        let store = crate::storage::AppStore::open_in_memory().unwrap();

        // Sign in with a token that's still fresh, so `access_token()` hands
        // it straight to the operation without a proactive refresh — the
        // only refresh in this test should be the reactive one after 401.
        let login_token = make_jwt(2_000_000_000);
        let login_body = serde_json::json!({
            "access_token": login_token,
            "refresh_token": "abc",
            "status": "success"
        })
        .to_string();
        server
            .mock("POST", "/api/v1/account/login")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(login_body)
            .create();
        store
            .auth()
            .login(&server.url(), "me@example.test", "pw")
            .unwrap();

        let fresh_token = make_jwt(2_000_000_000);
        let refresh_body = serde_json::json!({
            "access_token": fresh_token,
            "refresh_token": "def",
            "status": "success"
        })
        .to_string();
        let refresh_mock = server
            .mock("POST", "/api/v1/account/refresh-access-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(refresh_body.clone())
            .create();

        let attempts = AtomicU32::new(0);
        let result = store.api().call_with_token(&server.url(), |token| {
            let n = attempts.fetch_add(1, AtomicOrdering::SeqCst);
            if n == 0 {
                // First attempt: simulate the server rejecting this token.
                Err(ApiError::Server {
                    status: StatusCode::UNAUTHORIZED,
                    message: "expired".to_string(),
                })
            } else {
                Ok(token.to_string())
            }
        });

        assert_eq!(attempts.load(AtomicOrdering::SeqCst), 2);
        assert_eq!(result.unwrap(), fresh_token);
        refresh_mock.assert();
    }

    #[test]
    fn call_with_token_does_not_retry_non_auth_errors() {
        let mut server = mockito::Server::new();
        let store = crate::storage::AppStore::open_in_memory().unwrap();

        let token = make_jwt(2_000_000_000);
        let login_body = serde_json::json!({
            "access_token": token,
            "refresh_token": "abc",
            "status": "success"
        })
        .to_string();
        server
            .mock("POST", "/api/v1/account/login")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(login_body)
            .create();
        store
            .auth()
            .login(&server.url(), "me@example.test", "pw")
            .unwrap();

        let attempts = AtomicU32::new(0);
        let result: Result<(), ApiError> = store.api().call_with_token(&server.url(), |_token| {
            attempts.fetch_add(1, AtomicOrdering::SeqCst);
            Err(ApiError::Server {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: "boom".to_string(),
            })
        });

        assert_eq!(
            attempts.load(AtomicOrdering::SeqCst),
            1,
            "a non-401/403 error must not trigger a refresh-and-retry"
        );
        assert!(result.is_err());
    }
}
