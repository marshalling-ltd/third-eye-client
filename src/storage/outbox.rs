//! Durable outbound REST queue.
//!
//! Any call whose side effect must survive an app crash or restart should be
//! enqueued here rather than fired-and-forgotten. A single worker thread
//! drains the `rest_outbox` table with exponential backoff. The DB insert is
//! the commit boundary: anything the UI acknowledges is guaranteed to be
//! retried on the next launch.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use super::db::SharedDb;

/// Maximum number of seconds between retries.
const MAX_BACKOFF_SECS: i64 = 300;
/// Wait between polls when the queue is empty.
const IDLE_POLL: Duration = Duration::from_secs(1);

/// Minimal builder-style request description the worker needs to replay an
/// outbox row.
#[derive(Clone, Debug)]
pub struct OutboxRequest {
    pub method: String,
    pub endpoint: String,
    pub body_json: Option<String>,
    pub headers_json: Option<String>,
    pub idempotency_key: String,
}

impl OutboxRequest {
    #[must_use]
    pub fn new_with_random_key(method: &str, endpoint: &str) -> Self {
        Self {
            method: method.to_owned(),
            endpoint: endpoint.to_owned(),
            body_json: None,
            headers_json: None,
            idempotency_key: Uuid::new_v4().to_string(),
        }
    }

    #[must_use]
    pub fn with_json_body(mut self, body: &serde_json::Value) -> Self {
        self.body_json = Some(body.to_string());
        self
    }
}

/// Handle onto the outbox table. Cheap to clone; shares the DB mutex.
#[derive(Clone)]
pub struct OutboxStore {
    db: SharedDb,
}

impl OutboxStore {
    pub(crate) fn new(db: SharedDb) -> Self {
        Self { db }
    }

    /// Enqueues a request for retrying. The insert is what guarantees the
    /// request will be attempted after a restart.
    pub fn enqueue(&self, request: &OutboxRequest) -> Result<i64> {
        let now = now_ms();
        let conn = self.db.lock().expect("rest_outbox mutex poisoned");
        conn.execute(
            "INSERT INTO rest_outbox(method, endpoint, body_json, headers_json,
                created_at_ms, next_retry_at_ms, attempts, idempotency_key)
             VALUES(?1, ?2, ?3, ?4, ?5, ?5, 0, ?6)
             ON CONFLICT(idempotency_key) DO NOTHING",
            params![
                request.method,
                request.endpoint,
                request.body_json,
                request.headers_json,
                now,
                request.idempotency_key,
            ],
        )
        .context("enqueueing rest_outbox row")?;
        Ok(conn.last_insert_rowid())
    }

    /// Returns the count of items currently pending delivery.
    pub fn pending_count(&self) -> Result<i64> {
        let conn = self.db.lock().expect("rest_outbox mutex poisoned");
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM rest_outbox", [], |row| row.get(0))?;
        Ok(count)
    }

    /// Fetches the next-due row, if any.
    fn pop_due(&self, now_ms: i64) -> Result<Option<OutboxRow>> {
        let conn = self.db.lock().expect("rest_outbox mutex poisoned");
        conn.query_row(
            "SELECT id, method, endpoint, body_json, headers_json,
                    next_retry_at_ms, attempts, idempotency_key
             FROM rest_outbox
             WHERE next_retry_at_ms <= ?1
             ORDER BY next_retry_at_ms, id
             LIMIT 1",
            params![now_ms],
            |row| {
                Ok(OutboxRow {
                    id: row.get(0)?,
                    method: row.get(1)?,
                    endpoint: row.get(2)?,
                    body_json: row.get(3)?,
                    headers_json: row.get(4)?,
                    next_retry_at_ms: row.get(5)?,
                    attempts: row.get(6)?,
                    idempotency_key: row.get(7)?,
                })
            },
        )
        .optional()
        .context("selecting next due outbox row")
    }

    fn mark_succeeded(&self, id: i64) -> Result<()> {
        let conn = self.db.lock().expect("rest_outbox mutex poisoned");
        conn.execute("DELETE FROM rest_outbox WHERE id = ?1", params![id])
            .context("deleting completed rest_outbox row")?;
        Ok(())
    }

    fn mark_failed(&self, id: i64, attempts: i64, error: &str) -> Result<()> {
        let conn = self.db.lock().expect("rest_outbox mutex poisoned");
        let new_attempts = attempts + 1;
        let backoff_secs = backoff_for(new_attempts);
        let next = now_ms() + (backoff_secs * 1000);
        conn.execute(
            "UPDATE rest_outbox
                SET attempts = ?1, next_retry_at_ms = ?2, last_error = ?3
              WHERE id = ?4",
            params![new_attempts, next, error, id],
        )
        .context("updating failed rest_outbox row")?;
        Ok(())
    }
}

/// In-memory projection of a row handed to the worker.
#[derive(Debug, Clone)]
struct OutboxRow {
    id: i64,
    method: String,
    endpoint: String,
    body_json: Option<String>,
    #[allow(dead_code)]
    headers_json: Option<String>,
    #[allow(dead_code)]
    next_retry_at_ms: i64,
    attempts: i64,
    idempotency_key: String,
}

/// Returns the number of seconds before the next retry for the given attempt
/// count (1-indexed).
#[must_use]
pub fn backoff_for(attempts: i64) -> i64 {
    if attempts <= 0 {
        return 0;
    }
    let shift = attempts.min(16) as u32;
    let raw = 1_i64.checked_shl(shift).unwrap_or(MAX_BACKOFF_SECS);
    raw.min(MAX_BACKOFF_SECS)
}

/// Long-lived worker thread that drains the outbox.
pub struct OutboxWorker {
    stop_flag: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl OutboxWorker {
    #[must_use]
    pub fn spawn(store: OutboxStore) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let worker_flag = Arc::clone(&stop_flag);
        let handle = thread::Builder::new()
            .name("third-eye-outbox".into())
            .spawn(move || run_worker(store, worker_flag))
            .expect("failed to spawn outbox worker");
        Self {
            stop_flag,
            handle: Some(handle),
        }
    }

    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for OutboxWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_worker(store: OutboxStore, stop_flag: Arc<AtomicBool>) {
    let http = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            eprintln!("outbox: failed to build HTTP client: {err:#}");
            return;
        }
    };
    while !stop_flag.load(Ordering::Relaxed) {
        match store.pop_due(now_ms()) {
            Ok(Some(row)) => {
                if let Err(err) = attempt_send(&http, &store, row) {
                    eprintln!("outbox: send error: {err:#}");
                }
            }
            Ok(None) => thread::sleep(IDLE_POLL),
            Err(err) => {
                eprintln!("outbox: DB error: {err:#}");
                thread::sleep(IDLE_POLL);
            }
        }
    }
}

fn attempt_send(
    http: &reqwest::blocking::Client,
    store: &OutboxStore,
    row: OutboxRow,
) -> Result<()> {
    let method = match row.method.to_ascii_uppercase().as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "PATCH" => reqwest::Method::PATCH,
        "DELETE" => reqwest::Method::DELETE,
        other => {
            store.mark_failed(row.id, row.attempts, &format!("unsupported method {other}"))?;
            return Ok(());
        }
    };
    let mut request = http.request(method, &row.endpoint);
    if !row.idempotency_key.is_empty() {
        request = request.header("Idempotency-Key", row.idempotency_key.clone());
    }
    if let Some(body) = &row.body_json {
        request = request
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.clone());
    }
    match request.send() {
        Ok(response) if response.status().is_success() => {
            store.mark_succeeded(row.id)?;
            Ok(())
        }
        Ok(response) => {
            let msg = format!("HTTP {}", response.status());
            store.mark_failed(row.id, row.attempts, &msg)?;
            Ok(())
        }
        Err(err) => {
            store.mark_failed(row.id, row.attempts, &format!("{err:#}"))?;
            Ok(())
        }
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
    use crate::storage::db::open_in_memory;

    #[test]
    fn backoff_is_bounded_and_monotonic() {
        assert_eq!(backoff_for(0), 0);
        assert_eq!(backoff_for(1), 2);
        assert_eq!(backoff_for(2), 4);
        assert_eq!(backoff_for(3), 8);
        // Should never exceed the cap.
        assert_eq!(backoff_for(64), MAX_BACKOFF_SECS);
    }

    #[test]
    fn enqueue_then_pop() {
        let db = open_in_memory().unwrap();
        let store = OutboxStore::new(db);
        let req = OutboxRequest::new_with_random_key("POST", "https://example.test/x")
            .with_json_body(&serde_json::json!({"a": 1}));
        store.enqueue(&req).unwrap();
        assert_eq!(store.pending_count().unwrap(), 1);
        let row = store.pop_due(now_ms() + 1).unwrap().expect("should pop");
        assert_eq!(row.method, "POST");
        assert_eq!(row.endpoint, "https://example.test/x");
        assert!(row.body_json.as_deref().unwrap().contains("\"a\":1"));
    }

    #[test]
    fn duplicate_idempotency_key_is_ignored() {
        let db = open_in_memory().unwrap();
        let store = OutboxStore::new(db);
        let mut req = OutboxRequest::new_with_random_key("POST", "https://example.test/x");
        req.idempotency_key = "shared".into();
        store.enqueue(&req).unwrap();
        store.enqueue(&req).unwrap();
        assert_eq!(store.pending_count().unwrap(), 1);
    }
}
