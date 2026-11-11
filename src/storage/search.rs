//! Nearby-resources search against the third-eye server's `POST
//! /api/v1/search` endpoint (`aoi`/`poi`/`intermagnet_analysis`), used to
//! surface situational context ("what's around you") on the Map
//! screen.
//!
//! This deliberately does **not** use the generated OpenAPI client
//! (`third_eye_openapi::apis::search_handler_api`): the generated
//! `SearchResourceModel` hardcodes its `data` field to `PoiSearchModel`
//! regardless of `resource_type`, because openapi-generator doesn't handle a
//! `oneOf` nested inside an `allOf` for Rust. That would hard-fail to
//! deserialize any response containing an `aoi` item (whose `data` has no
//! `latitude`/`longitude`) and silently mistype `intermagnet_analysis` items
//! as `PoiSearchModel`. Instead, this client builds the request and parses
//! the response by hand with `reqwest::blocking` + `serde_json::Value`.

use anyhow::Context;
use reqwest::Url;
use serde_json::{Value, json};

use third_eye_openapi::apis::firms_area_record_handler_api;
use third_eye_openapi::models::{
    PagedResponseFirmsAreaRecordItemsInner, SearchFirmsAreaRecordsSchema, SpatialFilterSchema,
};

use super::api::{ApiError, ApiSession, BACKEND_HTTP_TIMEOUT};

/// Smallest radius, in meters, ever searched around the map view. Keeps
/// tightly zoomed-in views from re-fetching on every small pan. The OpenAPI
/// schema doesn't document units for `r`; meters is an assumption.
pub const MIN_SEARCH_RADIUS_M: f64 = 5_000.0;

/// Largest radius, in meters, searched for AOI/POI/Magnetograph/fires. The
/// platform documents no maximum, so this is a client-side guard: zoomed out
/// further than this we only show what is already loaded.
pub const MAX_SEARCH_RADIUS_M: f64 = 200_000.0;

/// How far back, in hours, to look for fire detections.
pub const FIRE_LOOKBACK_HOURS: u64 = 24;

/// Maximum fire detections requested per refresh.
const FIRE_RESULT_LIMIT: i64 = 200;

/// Kind of resource returned by `/api/v1/search`, matching the
/// `SearchResourceType` enum in the OpenAPI schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NearbyKind {
    Poi,
    Aoi,
    IntermagnetAnalysis,
    /// Satellite fire detection from `/api/v1/firms/area-records/search`.
    Fire,
}

/// A single item from `/api/v1/search`, flattened to a plottable point.
/// `aoi` items have no coordinates in their `data`; `lat`/`lon` for those are
/// a best-effort centroid of the `area` geometry (see `geometry_centroid`).
#[derive(Clone, Debug)]
pub struct NearbyResource {
    pub id: String,
    pub kind: NearbyKind,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// AOI boundary as `(lat, lon)` vertices (first ring); empty for other kinds.
    pub outline: Vec<(f64, f64)>,
    /// Latest processed RGB image of an AOI (`None` for other kinds, and for
    /// AOIs whose RGB analysis hasn't succeeded yet). It covers exactly the
    /// bounding box of `outline`.
    pub rgb_image: Option<RgbImage>,
}

/// An AOI's RGB image as returned by the search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbImage {
    /// Id of the RGB analysis; changes when a newer image is processed.
    pub analysis_id: String,
    /// Encoded image bytes (PNG/JPEG). Empty when the server sent only the
    /// metadata (`include_rgb_images` unset).
    pub bytes: Vec<u8>,
}

/// Facade held by `AppStore`, mirroring `DevicesClient`. Cloneable (like
/// `MediaStore`) so it can be moved into a background fetch thread; the
/// [`ApiSession`] it carries is what keeps that background call authenticated.
#[derive(Clone)]
pub struct SearchClient {
    http: reqwest::blocking::Client,
    /// For the generated (async) FIRMS endpoint.
    async_http: reqwest::Client,
    api: ApiSession,
}

impl SearchClient {
    pub(crate) fn new(api: ApiSession) -> Self {
        Self {
            http: reqwest::blocking::Client::builder()
                .timeout(BACKEND_HTTP_TIMEOUT)
                .build()
                .unwrap_or_default(),
            async_http: reqwest::Client::builder()
                .timeout(BACKEND_HTTP_TIMEOUT)
                .build()
                .unwrap_or_default(),
            api,
        }
    }

    /// Active-fire detections (NASA FIRMS) within `radius_m` meters of
    /// `(lat, lon)` over the last [`FIRE_LOOKBACK_HOURS`], via the generated
    /// `firms_area_record_handler_api`.
    pub fn fires(
        &self,
        server_base: &str,
        lat: f64,
        lon: f64,
        radius_m: f64,
    ) -> Result<Vec<NearbyResource>, ApiError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let from = now.saturating_sub(FIRE_LOOKBACK_HOURS * 3600);
        let (acquired_from, acquired_to) = (rfc3339_utc(from), rfc3339_utc(now));
        let page = self
            .api
            .call(server_base, &self.async_http, |configuration| {
                let schema = SearchFirmsAreaRecordsSchema::new(
                    acquired_from.clone(),
                    acquired_to.clone(),
                    SpatialFilterSchema::new(lat, lon, radius_m),
                );
                async move {
                    firms_area_record_handler_api::search_firms_area_records_handler(
                        &configuration,
                        schema,
                        None,
                        Some(FIRE_RESULT_LIMIT),
                        Some("acquired_at"),
                        Some(true),
                    )
                    .await
                }
            })?;
        Ok(page
            .items
            .into_iter()
            .map(|record| fire_resource(&record))
            .collect())
    }

    /// `POST /api/v1/search` for `aoi`/`poi`/`intermagnet_analysis` resources
    /// within `radius_m` meters of `(lat, lon)`.
    ///
    /// Hand-rolled rather than generated (see the module docs), so it opts into
    /// the same refresh-and-retry-on-401 behaviour as the generated calls via
    /// [`ApiSession::call_with_token`] instead of `ApiSession::call`.
    pub fn nearby(
        &self,
        server_base: &str,
        lat: f64,
        lon: f64,
        radius_m: f64,
    ) -> Result<Vec<NearbyResource>, ApiError> {
        let base_url = Url::parse(server_base.trim())
            .with_context(|| format!("invalid server URL {}", server_base.trim()))
            .map_err(ApiError::Transport)?;
        let url = base_url
            .join("/api/v1/search")
            .context("building search URL")
            .map_err(ApiError::Transport)?;
        let body = json!({
            "location": { "lat": lat, "lon": lon, "r": radius_m },
            "resource_types": ["aoi", "poi", "intermagnet_analysis"],
            // AOIs come back with their processed RGB image when there is one.
            "include_rgb_images": true,
        });
        self.api.call_with_token(server_base, |access_token| {
            let response = self
                .http
                .post(url.clone())
                .bearer_auth(access_token)
                .json(&body)
                .send()
                .context("sending search request")
                .map_err(ApiError::Transport)?;
            let status = response.status();
            if !status.is_success() {
                // Reported as `ApiError::Server` so a 401/403 triggers the
                // refresh-and-retry path rather than surfacing to the user.
                let message = response.text().unwrap_or_default();
                return Err(ApiError::Server { status, message });
            }
            let value: Value = response
                .json()
                .context("decoding search response")
                .map_err(ApiError::Transport)?;
            Ok(parse_items(&value))
        })
    }
}

/// Decodes a base64 image payload, tolerating a `data:image/...;base64,` prefix.
fn decode_base64_image(encoded: String) -> Option<Vec<u8>> {
    use base64::Engine;
    let payload = encoded.split_once(',').map_or(encoded.as_str(), |(_, b)| b);
    base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .ok()
        .filter(|bytes| !bytes.is_empty())
}

/// Flattens a FIRMS record to a plottable pin. FIRMS records have no id, so
/// one is derived from source/time/position.
fn fire_resource(record: &PagedResponseFirmsAreaRecordItemsInner) -> NearbyResource {
    let frp = record
        .frp
        .flatten()
        .map_or_default(|frp| format!(", {frp:.1} MW"));
    NearbyResource {
        id: format!(
            "fire:{}:{}:{:.5}:{:.5}",
            record.source, record.acquired_at, record.latitude, record.longitude
        ),
        kind: NearbyKind::Fire,
        name: format!(
            "Fire ({}, {} confidence{frp})",
            record.satellite, record.confidence
        ),
        lat: record.latitude,
        lon: record.longitude,
        outline: Vec::new(),
        rgb_image: None,
    }
}

/// Unix seconds -> `YYYY-MM-DDTHH:MM:SSZ` (UTC), no date crate needed.
fn rfc3339_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn parse_items(value: &Value) -> Vec<NearbyResource> {
    let Some(items) = value.get("items").and_then(Value::as_array) else {
        return Vec::new();
    };
    items.iter().filter_map(parse_item).collect()
}

fn parse_item(item: &Value) -> Option<NearbyResource> {
    let id = item.get("id")?.as_str()?.to_owned();
    let resource_type = item.get("resource_type")?.as_str()?;
    let data = item.get("data")?;
    let name = data
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    match resource_type {
        "poi" => {
            let lat = data.get("latitude")?.as_f64()?;
            let lon = data.get("longitude")?.as_f64()?;
            Some(NearbyResource {
                id,
                kind: NearbyKind::Poi,
                name,
                lat,
                lon,
                outline: Vec::new(),
                rgb_image: None,
            })
        }
        "intermagnet_analysis" => {
            let lat = data.get("latitude")?.as_f64()?;
            let lon = data.get("longitude")?.as_f64()?;
            Some(NearbyResource {
                id,
                kind: NearbyKind::IntermagnetAnalysis,
                name,
                lat,
                lon,
                outline: Vec::new(),
                rgb_image: None,
            })
        }
        "aoi" => {
            let area = data.get("area")?;
            let (lat, lon) = geometry_centroid(area)?;
            Some(NearbyResource {
                id,
                kind: NearbyKind::Aoi,
                name,
                lat,
                lon,
                outline: first_ring(area).unwrap_or_default(),
                rgb_image: parse_rgb_image(data.get("rgb_image")),
            })
        }
        _ => None,
    }
}

/// Reads an AOI item's `rgb_image` object. `None` when absent (no processed
/// RGB image) or when the image data can't be decoded.
fn parse_rgb_image(value: Option<&Value>) -> Option<RgbImage> {
    let value = value?;
    let analysis_id = value.get("analysis_id")?.as_str()?.to_owned();
    let bytes = match value.get("file_base64").and_then(Value::as_str) {
        Some(encoded) => decode_base64_image(encoded.to_owned())?,
        None => Vec::new(),
    };
    Some(RgbImage { analysis_id, bytes })
}

/// Approximates a GeoJSON-style geometry's centroid by averaging every
/// `[lon, lat]` coordinate pair found anywhere within it (recursing through
/// arbitrarily nested arrays/objects, and parsing string-encoded JSON if
/// `area` turns out to be double-encoded). This is a simple average of
/// vertices, not a true area-weighted centroid, but is good enough to place
/// a single representative map pin, matching how the reference web app
/// renders AOIs as single dots rather than outlined polygons.
fn geometry_centroid(area: &Value) -> Option<(f64, f64)> {
    let mut sum_lat = 0.0;
    let mut sum_lon = 0.0;
    let mut count: u32 = 0;
    collect_coordinate_pairs(area, &mut sum_lat, &mut sum_lon, &mut count);
    if count == 0 {
        return None;
    }
    Some((sum_lat / f64::from(count), sum_lon / f64::from(count)))
}

/// First polygon ring found in a GeoJSON-style geometry (arbitrarily nested,
/// possibly string-encoded), as `(lat, lon)` vertices. A ring is an array of at
/// least three `[lon, lat]` pairs.
fn first_ring(value: &Value) -> Option<Vec<(f64, f64)>> {
    match value {
        Value::Array(items) => {
            let pairs: Option<Vec<(f64, f64)>> = items
                .iter()
                .map(|item| match item.as_array()?.as_slice() {
                    [Value::Number(lon), Value::Number(lat), ..] => {
                        Some((lat.as_f64()?, lon.as_f64()?))
                    }
                    _ => None,
                })
                .collect();
            match pairs {
                Some(ring) if ring.len() >= 3 => Some(ring),
                _ => items.iter().find_map(first_ring),
            }
        }
        Value::Object(map) => map
            .get("coordinates")
            .and_then(first_ring)
            .or_else(|| map.values().find_map(first_ring)),
        Value::String(text) => first_ring(&serde_json::from_str::<Value>(text).ok()?),
        _ => None,
    }
}

/// Recursively walks a JSON value looking for GeoJSON-style `[lon, lat]`
/// coordinate pairs (a 2-element array of numbers) and accumulates them.
fn collect_coordinate_pairs(value: &Value, sum_lat: &mut f64, sum_lon: &mut f64, count: &mut u32) {
    match value {
        Value::Array(items) => {
            if let [Value::Number(lon), Value::Number(lat)] = items.as_slice()
                && let (Some(lon), Some(lat)) = (lon.as_f64(), lat.as_f64())
            {
                *sum_lon += lon;
                *sum_lat += lat;
                *count += 1;
                return;
            }
            for item in items {
                collect_coordinate_pairs(item, sum_lat, sum_lon, count);
            }
        }
        Value::Object(map) => {
            for nested in map.values() {
                collect_coordinate_pairs(nested, sum_lat, sum_lon, count);
            }
        }
        Value::String(text) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                collect_coordinate_pairs(&parsed, sum_lat, sum_lon, count);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiError, MAX_SEARCH_RADIUS_M, NearbyKind, parse_items, rfc3339_utc};
    use serde_json::json;

    #[test]
    fn rfc3339_formats_known_instants() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(1_709_210_096), "2024-02-29T12:34:56Z");
    }

    #[test]
    fn parses_poi_and_intermagnet_analysis_items() {
        let value = json!({
            "center": null,
            "items": [
                {
                    "id": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
                    "data": {"latitude": 45.1, "longitude": 15.2, "name": "Osijek Magnetograph"},
                    "resource_type": "intermagnet_analysis"
                },
                {
                    "id": "3fa85f64-5717-4562-b3fc-2c963f66afa7",
                    "data": {"latitude": 42.6, "longitude": 18.1, "name": "Dubrovnik POI"},
                    "resource_type": "poi"
                }
            ]
        });
        let items = parse_items(&value);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, NearbyKind::IntermagnetAnalysis);
        assert_eq!(items[0].name, "Osijek Magnetograph");
        assert!((items[0].lat - 45.1).abs() < f64::EPSILON);
        assert!((items[0].lon - 15.2).abs() < f64::EPSILON);
        assert_eq!(items[1].kind, NearbyKind::Poi);
    }

    #[test]
    fn aoi_keeps_its_polygon_outline() {
        let value = json!({"items": [{"id": "a1", "resource_type": "aoi", "data": {"name": "Zone", "area": {"type": "Polygon", "coordinates": [[[18.0, 42.0], [18.2, 42.0], [18.2, 42.2], [18.0, 42.0]]]}}}]});
        let items = parse_items(&value);
        assert_eq!(items[0].outline.len(), 4);
        assert_eq!(items[0].outline[1], (42.0, 18.2));
    }

    #[test]
    fn parses_aoi_polygon_area_as_centroid() {
        let value = json!({
            "center": null,
            "items": [
                {
                    "id": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
                    "data": {
                        "area": {
                            "type": "Polygon",
                            "coordinates": [[[10.0, 40.0], [12.0, 40.0], [12.0, 42.0], [10.0, 42.0]]]
                        },
                        "name": "Adriatic AOI"
                    },
                    "resource_type": "aoi"
                }
            ]
        });
        let items = parse_items(&value);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, NearbyKind::Aoi);
        assert_eq!(items[0].name, "Adriatic AOI");
        // Average of the 4 corners: lon (10+12+12+10)/4 = 11, lat (40+40+42+42)/4 = 41.
        assert!((items[0].lon - 11.0).abs() < 1e-9);
        assert!((items[0].lat - 41.0).abs() < 1e-9);
    }

    #[test]
    fn skips_unknown_resource_types_and_malformed_items() {
        let value = json!({
            "center": null,
            "items": [
                {"id": "x", "data": {"name": "no coords"}, "resource_type": "poi"},
                {"id": "y", "data": {"name": "unknown"}, "resource_type": "something_else"}
            ]
        });
        assert!(parse_items(&value).is_empty());
    }

    #[test]
    fn missing_items_array_yields_nothing() {
        assert!(parse_items(&json!({"center": null})).is_empty());
        assert!(parse_items(&json!({"items": "nope"})).is_empty());
    }

    #[test]
    fn skips_aoi_without_usable_geometry() {
        let value = json!({
            "items": [
                {"id": "a", "data": {"name": "no area"}, "resource_type": "aoi"},
                {"id": "b", "data": {"area": {"type": "Polygon", "coordinates": []}}, "resource_type": "aoi"}
            ]
        });
        assert!(parse_items(&value).is_empty());
    }

    #[test]
    fn parses_aoi_area_that_is_double_encoded_json() {
        let area = json!({"type": "Point", "coordinates": [16.0, 45.0]}).to_string();
        let value = json!({
            "items": [{"id": "a", "data": {"area": area, "name": "Encoded"}, "resource_type": "aoi"}]
        });
        let items = parse_items(&value);
        assert_eq!(items.len(), 1);
        assert!((items[0].lon - 16.0).abs() < 1e-9);
        assert!((items[0].lat - 45.0).abs() < 1e-9);
    }

    #[test]
    fn ignores_area_strings_that_are_not_json() {
        let value = json!({
            "items": [{"id": "a", "data": {"area": "not json"}, "resource_type": "aoi"}]
        });
        assert!(parse_items(&value).is_empty());
    }

    // ---- SearchClient::nearby (mocked server) ------------------------------

    fn signed_in_store(server: &mut mockito::Server) -> crate::storage::AppStore {
        use base64::Engine;
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header = engine.encode(br#"{"alg":"none","typ":"JWT"}"#);
        let payload = engine.encode(json!({"exp": 2_000_000_000_i64, "sub": "user"}).to_string());
        let token = format!("{header}.{payload}.");
        server
            .mock("POST", "/api/v1/account/login")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({"access_token": token, "refresh_token": "abc", "status": "success"})
                    .to_string(),
            )
            .create();
        let store = crate::storage::AppStore::open_in_memory().unwrap();
        store
            .auth()
            .login(&server.url(), "me@example.test", "secret")
            .expect("login succeeds");
        store
    }

    #[test]
    fn aoi_items_carry_rgb_image_when_processed() {
        let area = json!({"type": "Polygon", "coordinates": [[[1.0, 2.0], [1.0, 3.0], [2.0, 3.0], [1.0, 2.0]]]});
        let value = json!({"items": [
            {"id": "a1", "resource_type": "aoi", "data": {"name": "With", "area": area,
                "rgb_image": {"analysis_id": "an-1", "image_datetime": "2026-11-09T10:00:00Z",
                              "file_base64": "data:image/png;base64,aGVsbG8="}}},
            {"id": "a2", "resource_type": "aoi", "data": {"name": "Without", "area": area}},
            {"id": "a3", "resource_type": "aoi", "data": {"name": "MetaOnly", "area": area,
                "rgb_image": {"analysis_id": "an-3", "image_datetime": "2026-11-09T10:00:00Z"}}}
        ]});
        let items = parse_items(&value);
        assert_eq!(items.len(), 3);
        let rgb = items[0].rgb_image.as_ref().expect("rgb present");
        assert_eq!(rgb.analysis_id, "an-1");
        assert_eq!(rgb.bytes, b"hello");
        assert!(items[1].rgb_image.is_none());
        assert_eq!(items[2].rgb_image.as_ref().unwrap().bytes, Vec::<u8>::new());
    }

    #[test]
    fn fires_returns_pins_from_firms_records() {
        let mut server = mockito::Server::new();
        let store = signed_in_store(&mut server);
        let mock = server
            .mock("POST", "/api/v1/firms/area-records/search")
            .match_query(mockito::Matcher::Any)
            .match_body(mockito::Matcher::PartialJson(
                json!({"spatial": {"latitude": 42.6, "longitude": 18.1, "radius_meters": 200_000.0}}),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({"items": [{
                    "acquired_at": "2026-11-10T01:00:00Z", "confidence": "high",
                    "created_at": "2026-11-10T01:05:00Z", "daynight": "N", "frp": 12.5,
                    "latitude": 42.7, "longitude": 18.2, "satellite": "N20",
                    "scan": 0.4, "source": "VIIRS_NOAA20_NRT", "track": 0.4
                }], "limit": 200, "page": 1, "total": 1})
                .to_string(),
            )
            .create();

        let fires = store
            .search()
            .fires(&server.url(), 42.6, 18.1, MAX_SEARCH_RADIUS_M)
            .expect("fires succeed");
        mock.assert();
        assert_eq!(fires.len(), 1);
        assert_eq!(fires[0].kind, NearbyKind::Fire);
        assert!(fires[0].name.contains("12.5 MW"));
        assert_eq!((fires[0].lat, fires[0].lon), (42.7, 18.2));
    }

    #[test]
    fn nearby_returns_parsed_items() {
        let mut server = mockito::Server::new();
        let store = signed_in_store(&mut server);
        let mock = server
            .mock("POST", "/api/v1/search")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({"items": [
                    {"id": "p1", "data": {"latitude": 42.6, "longitude": 18.1, "name": "Dubrovnik"}, "resource_type": "poi"}
                ]})
                .to_string(),
            )
            .create();

        let items = store
            .search()
            .nearby(&server.url(), 42.6, 18.1, 5000.0)
            .expect("search succeeds");
        mock.assert();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Dubrovnik");
        assert_eq!(items[0].kind, NearbyKind::Poi);
    }

    #[test]
    fn nearby_surfaces_server_error() {
        let mut server = mockito::Server::new();
        let store = signed_in_store(&mut server);
        server
            .mock("POST", "/api/v1/search")
            .with_status(500)
            .with_body("boom")
            .create();

        let err = store
            .search()
            .nearby(&server.url(), 0.0, 0.0, 1.0)
            .unwrap_err();
        assert!(
            matches!(&err, ApiError::Server { status, message } if status.as_u16() == 500 && message == "boom"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn nearby_reports_undecodable_response() {
        let mut server = mockito::Server::new();
        let store = signed_in_store(&mut server);
        server
            .mock("POST", "/api/v1/search")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("not json")
            .create();

        let err = store
            .search()
            .nearby(&server.url(), 0.0, 0.0, 1.0)
            .unwrap_err();
        assert!(matches!(err, ApiError::Transport(_)), "unexpected: {err}");
    }

    #[test]
    fn nearby_rejects_an_invalid_server_url() {
        let store = crate::storage::AppStore::open_in_memory().unwrap();
        let err = store
            .search()
            .nearby("not a url", 0.0, 0.0, 1.0)
            .unwrap_err();
        assert!(matches!(err, ApiError::Transport(_)), "unexpected: {err}");
    }
}
