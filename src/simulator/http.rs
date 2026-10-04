//! Fake Chasing camera HTTP API (`/v1/capture`, `/v1/medias`, `/v1/lamp`)
//! backed by the files of a samples directory.

use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tiny_http::{Header, Method, Request, Response, Server};

pub const DEFAULT_HTTP_PORT: u16 = 8080;

struct State {
    dir: PathBuf,
    brightness: Mutex<i64>,
    /// Names "deleted" via the API. Files on disk are never touched.
    deleted: Mutex<HashSet<String>>,
}

type Body = Box<dyn Read + Send>;

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header is valid")
}

fn json_response(code: u16, value: &Value) -> Response<Body> {
    let bytes = value.to_string().into_bytes();
    let len = bytes.len();
    Response::new(
        code.into(),
        vec![header("Content-Type", "application/json")],
        Box::new(std::io::Cursor::new(bytes)),
        Some(len),
        None,
    )
}

fn ok_envelope(data: &Value) -> Response<Body> {
    json_response(200, &json!({"status": 0, "msg": "success", "data": data}))
}

fn error(code: u16, msg: &str) -> Response<Body> {
    json_response(code, &json!({"code": code, "error": msg}))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_video(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("mp4") || x.eq_ignore_ascii_case("mov"))
}

fn list_files(state: &State) -> Vec<(String, u64)> {
    let deleted = state.deleted.lock().expect("lock");
    let mut files: Vec<(String, u64)> = std::fs::read_dir(&state.dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            let name = e.file_name().into_string().ok()?;
            (meta.is_file() && !name.starts_with('.') && !deleted.contains(&name))
                .then_some((name, meta.len()))
        })
        .collect();
    files.sort();
    files
}

fn media_origin(name: &str) -> Value {
    let video = is_video(name);
    json!({
        "width": 1920, "height": 1080,
        "duration": if video { 10 } else { 0 },
        "fps": if video { 30 } else { 0 },
        "br": if video { 8000 } else { 0 },
        "multi": 0, "withOsd": false, "id": name, "stat": 0
    })
}

fn media_json(name: &str, size: u64) -> Value {
    let mut v = json!({
        "name": name, "size": size,
        "canplayback": is_video(name),
        "origin": media_origin(name),
    });
    if is_video(name) {
        v["play"] = json!({"stat": 0});
    }
    v
}

fn content_type(name: &str) -> &'static str {
    match Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("dng") => "image/x-adobe-dng",
        _ => "application/octet-stream",
    }
}

/// Parses `bytes=start-end` into an inclusive range clamped to `len`.
fn parse_range(value: &str, len: u64) -> Option<(u64, u64)> {
    let spec = value.strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        (len.saturating_sub(n), len.checked_sub(1)?)
    } else {
        let start: u64 = a.parse().ok()?;
        let end = if b.is_empty() {
            len.checked_sub(1)?
        } else {
            b.parse::<u64>().ok()?.min(len.checked_sub(1)?)
        };
        (start, end)
    };
    (start <= end).then_some((start, end))
}

fn download(state: &State, name: &str, range: Option<&str>) -> Response<Body> {
    if !list_files(state).iter().any(|(n, _)| n == name) {
        return error(404, "media not found");
    }
    let Ok(mut file) = File::open(state.dir.join(name)) else {
        return error(500, "cannot open media");
    };
    let len = file.metadata().map_or(0, |m| m.len());
    let mut headers = vec![
        header("Content-Type", content_type(name)),
        header("Accept-Ranges", "bytes"),
    ];
    if let Some((start, end)) = range.and_then(|r| parse_range(r, len)) {
        let _ = file.seek(SeekFrom::Start(start));
        let n = end - start + 1;
        headers.push(header(
            "Content-Range",
            &format!("bytes {start}-{end}/{len}"),
        ));
        return Response::new(
            206.into(),
            headers,
            Box::new(file.take(n)),
            usize::try_from(n).ok(),
            None,
        );
    }
    Response::new(
        200.into(),
        headers,
        Box::new(file),
        usize::try_from(len).ok(),
        None,
    )
}

fn read_json(req: &mut Request) -> Value {
    let mut body = String::new();
    let _ = req.as_reader().read_to_string(&mut body);
    serde_json::from_str(&body).unwrap_or(Value::Null)
}

fn route(state: &State, req: &mut Request) -> Response<Body> {
    let url = req.url().to_owned();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let segs: Vec<String> = path
        .trim_matches('/')
        .split('/')
        .map(percent_decode)
        .collect();
    let segs: Vec<&str> = segs.iter().map(String::as_str).collect();
    let query_has = |kv: &str| query.split('&').any(|p| p == kv);
    let range = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Range"))
        .map(|h| h.value.as_str().to_owned());

    match (req.method().clone(), segs.as_slice()) {
        (Method::Post, ["v1", "capture"]) => {
            let _ = read_json(req);
            json_response(201, &json!({"status": 0, "msg": "success", "data": null}))
        }
        (Method::Get, ["v1", "lamp"]) => {
            let b = *state.brightness.lock().expect("lock");
            ok_envelope(&json!({"brightness": b}))
        }
        (Method::Post, ["v1", "lamp"]) => {
            let body = read_json(req);
            let Some(b) = body.get("brightness").and_then(Value::as_i64) else {
                return error(400, "missing brightness");
            };
            *state.brightness.lock().expect("lock") = b.clamp(0, 100);
            ok_envelope(&Value::Null)
        }
        (Method::Get, ["v1", "medias"]) => {
            let items: Vec<Value> = list_files(state)
                .iter()
                .map(|(n, s)| media_json(n, *s))
                .collect();
            json_response(200, &Value::Array(items))
        }
        (Method::Get, ["v1", "medias", name, "download"]) => {
            download(state, name, range.as_deref())
        }
        (Method::Get, ["v1", "medias", name, "info"]) => {
            let _ = query_has("for=repair");
            match list_files(state).into_iter().find(|(n, _)| n == name) {
                Some((n, size)) => {
                    let mut v = media_origin(&n);
                    v["name"] = json!(n);
                    v["size"] = json!(size);
                    json_response(200, &v)
                }
                None => error(404, "media not found"),
            }
        }
        (Method::Delete, ["v1", "medias", name]) => {
            if list_files(state).iter().any(|(n, _)| n == name) {
                state
                    .deleted
                    .lock()
                    .expect("lock")
                    .insert((*name).to_owned());
                json_response(200, &json!({"status": 0, "msg": "success", "data": null}))
            } else {
                error(404, "media not found")
            }
        }
        _ => error(404, "not found"),
    }
}

/// Serves the camera API on `bind:port` until `stop` is set.
pub fn run(bind_host: &str, port: u16, samples_dir: &Path, stop: &AtomicBool) -> Result<()> {
    let server = Arc::new(
        Server::http((bind_host, port))
            .map_err(|e| anyhow::anyhow!("{e}"))
            .with_context(|| format!("failed to bind HTTP server on {bind_host}:{port}"))?,
    );
    let state = Arc::new(State {
        dir: samples_dir.to_owned(),
        brightness: Mutex::new(50),
        deleted: Mutex::new(HashSet::new()),
    });
    while !stop.load(Ordering::Relaxed) {
        let Some(mut req) = server.recv_timeout(Duration::from_millis(200))? else {
            continue;
        };
        let state = Arc::clone(&state);
        // One thread per request: ffmpeg may hold a download open while the UI polls.
        std::thread::spawn(move || {
            let resp = route(&state, &mut req);
            let _ = req.respond(resp);
        });
    }
    Ok(())
}
