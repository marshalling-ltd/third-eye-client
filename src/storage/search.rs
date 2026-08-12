//! Nearby-resources search against the third-eye server's `POST
//! /api/v1/search` endpoint (`aoi`/`poi`/`intermagnet_analysis`), used to
//! surface situational context ("what's around you") on the Device Map
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

use super::api::{ApiError, ApiSession, BACKEND_HTTP_TIMEOUT};

/// Default search radius, in meters, used when pulling nearby resources for
/// the Device Map screen. The OpenAPI schema doesn't document units for `r`;
/// this is an assumption, easy to tune once confirmed against the web app.
pub const DEFAULT_SEARCH_RADIUS_M: f64 = 5000.0;

/// Kind of resource returned by `/api/v1/search`, matching the
/// `SearchResourceType` enum in the OpenAPI schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NearbyKind {
    Poi,
    Aoi,
    IntermagnetAnalysis,
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
}

/// Facade held by `AppStore`, mirroring `DevicesClient`. Cloneable (like
/// `MediaStore`) so it can be moved into a background fetch thread; the
/// [`ApiSession`] it carries is what keeps that background call authenticated.
#[derive(Clone)]
pub struct SearchClient {
    http: reqwest::blocking::Client,
    api: ApiSession,
}

impl SearchClient {
    pub(crate) fn new(api: ApiSession) -> Self {
        Self {
            http: reqwest::blocking::Client::builder()
                .timeout(BACKEND_HTTP_TIMEOUT)
                .build()
                .unwrap_or_default(),
            api,
        }
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
            })
        }
        "aoi" => {
            let (lat, lon) = geometry_centroid(data.get("area")?)?;
            Some(NearbyResource {
                id,
                kind: NearbyKind::Aoi,
                name,
                lat,
                lon,
            })
        }
        _ => None,
    }
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
    use super::{NearbyKind, parse_items};
    use serde_json::json;

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
}
