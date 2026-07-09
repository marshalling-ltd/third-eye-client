//! Authentication against the third-eye server.
//!
//! Endpoints (from `/api/v1/api-doc/openapi.json`):
//!   * `POST /api/v1/account/login`                     -> `{access_token, status}`
//!   * `POST /api/v1/account/refresh-access-token`      -> `{access_token, status}`
//!   * `GET  /api/v1/account/logout`                    -> `204`
//!
//! The refresh token is set by the server as an `HttpOnly` cookie. A persisted
//! cookie jar ([`PersistentCookieJar`]) mirrors those cookies to `SQLite` so the
//! session survives restarts without the user re-entering credentials.
use std::fmt::Write;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use base64::Engine;
use cookie::{Cookie, SameSite};
use reqwest::Client;
use reqwest::StatusCode;
use reqwest::Url;
use reqwest::cookie::{CookieStore, Jar};
use reqwest::header::HeaderValue;
use rusqlite::{OptionalExtension, params};
use third_eye_openapi::apis::Error as GeneratedApiError;
use third_eye_openapi::apis::account_handler_api;
use third_eye_openapi::apis::configuration as generated_configuration;
use third_eye_openapi::models::LoginSchema;

use super::db::SharedDb;

/// Currently-authenticated session row. Absence of a row / of an
/// `access_token` means the user is signed out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub server_base: String,
    pub email: Option<String>,
    pub user_id: Option<String>,
    pub access_token: String,
    pub access_exp_ms: Option<i64>,
}

/// Outcome of a successful login. Matches what `main.rs` needs to render.
#[derive(Clone, Debug)]
pub struct LoginOutcome {
    pub email: String,
    pub access_token: String,
    pub access_exp_ms: Option<i64>,
}

/// Error surface shared by auth calls so the UI can map a 401 to "bad
/// password" without parsing strings.
#[derive(Debug)]
pub enum AuthError {
    Server { status: StatusCode, message: String },
    Transport(anyhow::Error),
    NotAuthenticated,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Server { status, message } => {
                write!(f, "authentication failed (HTTP {status}): {message}")
            }
            AuthError::Transport(err) => write!(f, "network or decoding failure: {err:#}"),
            AuthError::NotAuthenticated => f.write_str("no active session; please sign in"),
        }
    }
}

impl std::error::Error for AuthError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // `anyhow::Error` does not implement `std::error::Error` directly.
        None
    }
}

impl From<anyhow::Error> for AuthError {
    fn from(err: anyhow::Error) -> Self {
        AuthError::Transport(err)
    }
}

impl AuthError {
    fn from_reqwest(err: reqwest::Error) -> Self {
        Self::Transport(anyhow::anyhow!(err))
    }

    fn from_rusqlite(err: rusqlite::Error) -> Self {
        Self::Transport(anyhow::anyhow!(err))
    }
}

/// Authentication facade held by `AppStore`.
pub struct AuthClient {
    db: SharedDb,
    jar: Arc<PersistentCookieJar>,
    http: Client,
}

impl AuthClient {
    pub(crate) fn new(db: SharedDb) -> Result<Self> {
        let jar = Arc::new(PersistentCookieJar::load(Arc::clone(&db))?);
        let http = Client::builder()
            .cookie_provider(Arc::clone(&jar))
            .build()
            .context("building authenticated reqwest client")?;
        Ok(Self { db, jar, http })
    }

    /// Reads the last-known session from the database, if any.
    pub fn current_session(&self) -> Result<Option<Session>> {
        let conn = self.db.lock().expect("auth_session mutex poisoned");
        conn.query_row(
            "SELECT server_base, email, user_id, access_token, access_exp_ms
             FROM auth_session WHERE id = 1",
            [],
            |row| {
                Ok(Session {
                    server_base: row.get(0)?,
                    email: row.get(1)?,
                    user_id: row.get(2)?,
                    access_token: row.get(3).unwrap_or_default(),
                    access_exp_ms: row.get(4)?,
                })
            },
        )
        .optional()
        .context("reading auth_session")?
        .filter(|s| !s.access_token.is_empty())
        .map(Ok)
        .transpose()
    }

    /// Returns the access token if currently authenticated.
    pub fn access_token(&self) -> Result<Option<String>> {
        Ok(self.current_session()?.map(|s| s.access_token))
    }

    /// POST `/api/v1/account/login`.
    ///
    /// On success, stores the JWT access token; cookies are persisted by the
    /// shared `PersistentCookieJar`.
    pub fn login(
        &self,
        server_base: &str,
        email: &str,
        password: &str,
    ) -> Result<LoginOutcome, AuthError> {
        let configuration = self.generated_configuration(server_base)?;
        let payload = Self::run_generated(account_handler_api::login_user_handler(
            &configuration,
            LoginSchema::new(email.to_owned(), password.to_owned()),
        ))?;
        let exp_ms = decode_jwt_exp_ms(&payload.access_token);
        update_session_row(
            &self.db,
            server_base,
            Some(email),
            None,
            &payload.access_token,
            exp_ms,
        )
        .map_err(AuthError::from_rusqlite)?;

        Ok(LoginOutcome {
            email: email.to_owned(),
            access_token: payload.access_token,
            access_exp_ms: exp_ms,
        })
    }

    /// POST `/api/v1/account/refresh-access-token`. Requires a valid refresh
    /// cookie to already be in the jar.
    pub fn refresh(&self, server_base: &str) -> Result<String, AuthError> {
        let configuration = self.generated_configuration(server_base)?;
        let payload = Self::run_generated(account_handler_api::refresh_access_token_handler(
            &configuration,
        ))?;
        let exp_ms = decode_jwt_exp_ms(&payload.access_token);
        update_session_token(&self.db, server_base, &payload.access_token, exp_ms)
            .map_err(AuthError::from_rusqlite)?;
        Ok(payload.access_token)
    }

    /// GET `/api/v1/account/logout`.
    ///
    /// Always clears the locally-persisted session and cookie jar, even when
    /// the server responds with an error - the user clearly wants to be
    /// signed out.
    pub fn logout(&self, server_base: &str) -> Result<(), AuthError> {
        let configuration = self.generated_configuration(server_base)?;
        let result = Self::run_generated(account_handler_api::logout_handler(&configuration));
        // Clear local state regardless of response.
        let _ = clear_session(&self.db);
        let _ = self.jar.clear_all();
        result
    }

    fn generated_configuration(
        &self,
        server_base: &str,
    ) -> Result<generated_configuration::Configuration, AuthError> {
        let mut configuration = generated_configuration::Configuration::new();
        let base_url = Url::parse(server_base.trim())
            .with_context(|| format!("invalid server URL {}", server_base.trim()))
            .map_err(AuthError::Transport)?;
        base_url
            .as_str()
            .trim_end_matches('/')
            .clone_into(&mut configuration.base_path);
        configuration.user_agent = None;
        configuration.client = self.http.clone();
        Ok(configuration)
    }
    fn run_generated<T, E, F>(future: F) -> Result<T, AuthError>
    where
        F: std::future::Future<Output = Result<T, GeneratedApiError<E>>>,
    {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("creating runtime for generated auth API")
            .map_err(AuthError::Transport)?;
        runtime.block_on(future).map_err(map_generated_error)
    }
}

fn map_generated_error<E>(error: GeneratedApiError<E>) -> AuthError {
    match error {
        GeneratedApiError::ResponseError(content) => AuthError::Server {
            status: content.status,
            message: content.content,
        },
        GeneratedApiError::Reqwest(error) => AuthError::from_reqwest(error),
        GeneratedApiError::Serde(error) => AuthError::Transport(anyhow::anyhow!(error)),
        GeneratedApiError::Io(error) => AuthError::Transport(anyhow::anyhow!(error)),
    }
}

fn update_session_row(
    db: &SharedDb,
    server_base: &str,
    email: Option<&str>,
    user_id: Option<&str>,
    access_token: &str,
    access_exp_ms: Option<i64>,
) -> Result<(), rusqlite::Error> {
    let conn = db.lock().expect("auth_session mutex poisoned");
    let now = now_ms();
    conn.execute(
        "INSERT INTO auth_session(id, server_base, email, user_id, access_token, access_exp_ms, updated_at_ms)
         VALUES(1, ?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
             server_base   = excluded.server_base,
             email         = COALESCE(excluded.email, auth_session.email),
             user_id       = COALESCE(excluded.user_id, auth_session.user_id),
             access_token  = excluded.access_token,
             access_exp_ms = excluded.access_exp_ms,
             updated_at_ms = excluded.updated_at_ms",
        params![server_base, email, user_id, access_token, access_exp_ms, now],
    )?;
    Ok(())
}

fn update_session_token(
    db: &SharedDb,
    server_base: &str,
    access_token: &str,
    access_exp_ms: Option<i64>,
) -> Result<(), rusqlite::Error> {
    let conn = db.lock().expect("auth_session mutex poisoned");
    let now = now_ms();
    conn.execute(
        "INSERT INTO auth_session(id, server_base, access_token, access_exp_ms, updated_at_ms)
         VALUES(1, ?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
             server_base   = excluded.server_base,
             access_token  = excluded.access_token,
             access_exp_ms = excluded.access_exp_ms,
             updated_at_ms = excluded.updated_at_ms",
        params![server_base, access_token, access_exp_ms, now],
    )?;
    Ok(())
}

fn clear_session(db: &SharedDb) -> Result<()> {
    let conn = db.lock().expect("auth_session mutex poisoned");
    conn.execute("DELETE FROM auth_session WHERE id = 1", [])
        .context("clearing auth_session")?;
    Ok(())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Decodes the `exp` claim of a JWT without verifying the signature. We only
/// use this for scheduling proactive refresh.
#[must_use]
pub fn decode_jwt_exp_ms(token: &str) -> Option<i64> {
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload_b64 = parts.next()?;
    let payload_bytes = b64_url_decode(payload_b64)?;
    let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
    let exp_seconds = payload.get("exp")?.as_i64()?;
    Some(exp_seconds.saturating_mul(1000))
}

fn b64_url_decode(input: &str) -> Option<Vec<u8>> {
    let trimmed = input.trim_end_matches('=');
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(trimmed)
        .ok()
}

// =======================================================================
// Persistent cookie jar
// =======================================================================

/// Cookie jar that mirrors every inserted cookie into the `http_cookies`
/// table so `HttpOnly` refresh tokens survive restarts.
pub(crate) struct PersistentCookieJar {
    inner: Jar,
    db: SharedDb,
    // Guards concurrent writers of the DB mirror. The `Jar` itself is
    // already internally synchronised.
    write_lock: Mutex<()>,
}

impl PersistentCookieJar {
    fn load(db: SharedDb) -> Result<Self> {
        let jar = Jar::default();
        let conn = db.lock().expect("http_cookies mutex poisoned");
        let mut stmt = conn
            .prepare(
                "SELECT domain, path, name, value, expires_ms, secure, http_only, same_site
                 FROM http_cookies",
            )
            .context("preparing http_cookies load")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(StoredCookie {
                    domain: row.get(0)?,
                    path: row.get(1)?,
                    name: row.get(2)?,
                    value: row.get(3)?,
                    expires_ms: row.get(4)?,
                    secure: row.get::<_, i64>(5)? != 0,
                    http_only: row.get::<_, i64>(6)? != 0,
                    same_site: row.get(7)?,
                })
            })
            .context("scanning http_cookies")?;
        for row in rows {
            let cookie = row.context("reading http_cookies row")?;
            let Some((cookie_str, url)) = cookie.to_cookie_header() else {
                continue;
            };
            jar.add_cookie_str(&cookie_str, &url);
        }
        drop(stmt);
        drop(conn);
        Ok(Self {
            inner: jar,
            db,
            write_lock: Mutex::new(()),
        })
    }

    fn persist_set_cookies(&self, headers: &[HeaderValue], url: &Url) -> Result<(), AuthError> {
        if headers.is_empty() {
            return Ok(());
        }
        let _guard = self.write_lock.lock().expect("cookie write lock poisoned");
        let conn = self.db.lock().expect("http_cookies mutex poisoned");
        let tx = conn
            .unchecked_transaction()
            .map_err(AuthError::from_rusqlite)?;
        for header in headers {
            let Ok(header_str) = header.to_str() else {
                continue;
            };
            let Ok(cookie) = Cookie::parse_encoded(header_str.to_owned()) else {
                continue;
            };
            let stored = StoredCookie::from_parsed(&cookie, url);
            // Max-Age / expiry of 0 or in the past == deletion request.
            if stored.expires_ms.is_some_and(|ms| ms <= now_ms()) {
                tx.execute(
                    "DELETE FROM http_cookies WHERE domain = ?1 AND path = ?2 AND name = ?3",
                    params![stored.domain, stored.path, stored.name],
                )
                .map_err(AuthError::from_rusqlite)?;
                continue;
            }
            tx.execute(
                "INSERT INTO http_cookies(domain, path, name, value, expires_ms, secure, http_only, same_site)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(domain, path, name) DO UPDATE SET
                     value      = excluded.value,
                     expires_ms = excluded.expires_ms,
                     secure     = excluded.secure,
                     http_only  = excluded.http_only,
                     same_site  = excluded.same_site",
                params![
                    stored.domain,
                    stored.path,
                    stored.name,
                    stored.value,
                    stored.expires_ms,
                    i64::from(stored.secure),
                    i64::from(stored.http_only),
                    stored.same_site,
                ],
            )
            .map_err(AuthError::from_rusqlite)?;
        }
        tx.commit().map_err(AuthError::from_rusqlite)?;
        Ok(())
    }

    fn clear_all(&self) -> Result<()> {
        let _guard = self.write_lock.lock().expect("cookie write lock poisoned");
        let conn = self.db.lock().expect("http_cookies mutex poisoned");
        conn.execute("DELETE FROM http_cookies", [])
            .context("clearing http_cookies")?;
        Ok(())
    }
}

impl CookieStore for PersistentCookieJar {
    fn set_cookies(&self, cookie_headers: &mut dyn Iterator<Item = &HeaderValue>, url: &Url) {
        let headers: Vec<HeaderValue> = cookie_headers.cloned().collect();
        let mut iter = headers.iter();
        self.inner.set_cookies(&mut iter, url);
        if let Err(err) = self.persist_set_cookies(&headers, url) {
            eprintln!("third-eye-client: warning: failed to persist auth cookies: {err}");
        }
    }

    fn cookies(&self, url: &Url) -> Option<HeaderValue> {
        self.inner.cookies(url)
    }
}

/// Representation of a single persisted cookie. Used both when writing to the
/// DB and when restoring cookies back into an empty `Jar` on startup.
struct StoredCookie {
    domain: String,
    path: String,
    name: String,
    value: String,
    expires_ms: Option<i64>,
    secure: bool,
    http_only: bool,
    same_site: Option<String>,
}

impl StoredCookie {
    fn from_parsed(cookie: &Cookie<'_>, url: &Url) -> Self {
        let domain = cookie
            .domain()
            .map(|d| d.trim_start_matches('.').to_owned())
            .or_else(|| url.host_str().map(str::to_owned))
            .unwrap_or_default();
        let path = cookie.path().unwrap_or("/").to_owned();
        let expires_ms = cookie.expires_datetime().map(|dt| {
            let ts = dt.unix_timestamp();
            ts.saturating_mul(1000)
        });
        let same_site = cookie.same_site().map(|s| match s {
            SameSite::Strict => "Strict".to_string(),
            SameSite::Lax => "Lax".to_string(),
            SameSite::None => "None".to_string(),
        });
        Self {
            domain,
            path,
            name: cookie.name().to_owned(),
            value: cookie.value().to_owned(),
            expires_ms,
            secure: cookie.secure().unwrap_or(false),
            http_only: cookie.http_only().unwrap_or(false),
            same_site,
        }
    }

    /// Serialises back to a `Set-Cookie` string + effective URL so
    /// [`Jar::add_cookie_str`] can consume it.
    fn to_cookie_header(&self) -> Option<(String, Url)> {
        if self.domain.is_empty() {
            return None;
        }
        let mut header = format!("{}={}", self.name, self.value);
        header.push_str("; Path=");
        header.push_str(&self.path);
        header.push_str("; Domain=");
        header.push_str(&self.domain);
        if let Some(expires_ms) = self.expires_ms {
            // `add_cookie_str` only understands the Max-Age attribute reliably
            // across platforms; convert an absolute expiry back to a relative
            // one in seconds (rounded down, clamped to 0).
            let now = now_ms();
            let max_age_secs = ((expires_ms - now) / 1000).max(0);
            let _ = write!(header, "; Max-Age={max_age_secs}");
        }
        if self.secure {
            header.push_str("; Secure");
        }
        if self.http_only {
            header.push_str("; HttpOnly");
        }
        if let Some(same_site) = &self.same_site {
            header.push_str("; SameSite=");
            header.push_str(same_site);
        }
        let scheme = if self.secure { "https" } else { "http" };
        let url = Url::parse(&format!("{scheme}://{}{}", self.domain, self.path)).ok()?;
        Some((header, url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::open_in_memory;

    #[test]
    fn jwt_exp_roundtrip() {
        // Header: {"alg":"none","typ":"JWT"}
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"alg":"none","typ":"JWT"}"#);
        let payload =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"exp":1700000000}"#);
        let token = format!("{header}.{payload}.sig");
        assert_eq!(decode_jwt_exp_ms(&token), Some(1_700_000_000_000));
    }

    #[test]
    fn jwt_without_exp_returns_none() {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"sub":"u"}"#);
        let token = format!("{header}.{payload}.");
        assert_eq!(decode_jwt_exp_ms(&token), None);
    }

    #[test]
    fn persistent_cookie_jar_roundtrips() {
        let db = open_in_memory().unwrap();
        let url = Url::parse("https://example.test/api/v1/account/login").unwrap();
        let headers = vec![HeaderValue::from_static(
            "refresh_token=abc; Path=/; Domain=example.test; HttpOnly; Secure; Max-Age=86400",
        )];
        {
            let jar = PersistentCookieJar::load(Arc::clone(&db)).unwrap();
            jar.persist_set_cookies(&headers, &url).unwrap();
        }
        // Reopen - should rehydrate the cookie.
        let jar = PersistentCookieJar::load(Arc::clone(&db)).unwrap();
        let cookies = jar
            .cookies(&url)
            .expect("cookie jar should replay persisted cookie");
        let cookies_str = cookies.to_str().unwrap();
        assert!(
            cookies_str.contains("refresh_token=abc"),
            "got {cookies_str}"
        );
    }
}
