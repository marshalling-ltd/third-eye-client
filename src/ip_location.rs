//! Approximate location from the machine's public IP address.
//!
//! Used on platforms without a native location service (Linux). The accuracy
//! is city level at best, and wrong behind a VPN or corporate NAT, so callers
//! should present the result as approximate.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

/// Default provider (free, no API key, HTTPS). Returns `latitude`/`longitude`.
pub const DEFAULT_IP_GEOLOCATION_URL: &str = "https://ipwho.is/";

/// Environment variable overriding the provider URL (self-hosted GeoIP, tests).
pub const IP_GEOLOCATION_URL_ENV: &str = "THIRD_EYE_IP_GEOLOCATION_URL";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

fn valid_coordinate(lat: f64, lon: f64) -> bool {
    lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
        // Providers sometimes answer 0,0 ("null island") when they have no data.
        && !(lat == 0.0 && lon == 0.0)
}

fn as_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Parses a geolocation response body.
///
/// Accepts `{"latitude": .., "longitude": ..}` (ipwho.is, ipapi.co),
/// `{"lat": .., "lon": ..}` (ip-api.com) and `{"loc": "lat,lon"}` (ipinfo.io).
/// Responses flagged as failures (`"success": false`, `"error": true`,
/// `"status": "fail"`) are rejected.
pub fn parse_ip_location(body: &str) -> Result<(f64, f64)> {
    let json: Value = serde_json::from_str(body).context("IP geolocation response is not JSON")?;

    if json.get("success").and_then(Value::as_bool) == Some(false)
        || json
            .get("error")
            .is_some_and(|e| e.as_bool() != Some(false))
        || json.get("status").and_then(Value::as_str) == Some("fail")
    {
        let reason = ["message", "reason"]
            .iter()
            .find_map(|k| json.get(*k).and_then(Value::as_str))
            .unwrap_or("provider reported an error");
        bail!("IP geolocation failed: {reason}");
    }

    let pair = [("latitude", "longitude"), ("lat", "lon")]
        .iter()
        .find_map(|(la, lo)| Some((as_f64(json.get(*la)?)?, as_f64(json.get(*lo)?)?)))
        .or_else(|| {
            let (la, lo) = json.get("loc")?.as_str()?.split_once(',')?;
            Some((la.trim().parse().ok()?, lo.trim().parse().ok()?))
        });

    let (lat, lon) = pair.context("IP geolocation response has no coordinates")?;
    if !valid_coordinate(lat, lon) {
        bail!("IP geolocation returned an invalid coordinate ({lat}, {lon})");
    }
    Ok((lat, lon))
}

/// Resolves the provider URL: env override if set and non-empty, else default.
#[must_use]
pub fn ip_geolocation_url() -> String {
    std::env::var(IP_GEOLOCATION_URL_ENV)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_IP_GEOLOCATION_URL.to_owned())
}

/// Blocking lookup against `url`. Never call from the UI thread.
pub fn detect_location_from_ip_blocking(url: &str) -> Result<(f64, f64)> {
    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(concat!("third-eye-client/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to build HTTP client")?;
    let response = client
        .get(url)
        .send()
        .context("IP geolocation request failed")?;
    let status = response.status();
    if !status.is_success() {
        bail!("IP geolocation provider returned HTTP {status}");
    }
    let body = response.text().context("reading IP geolocation response")?;
    parse_ip_location(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ipwho_is() {
        let body = r#"{"ip":"1.2.3.4","success":true,"latitude":45.815,"longitude":15.9819}"#;
        assert_eq!(parse_ip_location(body).unwrap(), (45.815, 15.9819));
    }

    #[test]
    fn parses_ip_api_com() {
        let body = r#"{"status":"success","lat":48.85,"lon":2.35}"#;
        assert_eq!(parse_ip_location(body).unwrap(), (48.85, 2.35));
    }

    #[test]
    fn parses_ipinfo_loc_string() {
        let body = r#"{"ip":"1.2.3.4","loc":"37.3860,-122.0838"}"#;
        assert_eq!(parse_ip_location(body).unwrap(), (37.386, -122.0838));
    }

    #[test]
    fn parses_numeric_strings() {
        let body = r#"{"latitude":"10.5","longitude":"-20.25"}"#;
        assert_eq!(parse_ip_location(body).unwrap(), (10.5, -20.25));
    }

    #[test]
    fn ipapi_co_error_false_is_not_an_error() {
        let body = r#"{"error":false,"latitude":1.5,"longitude":2.5}"#;
        assert_eq!(parse_ip_location(body).unwrap(), (1.5, 2.5));
    }

    #[test]
    fn rejects_provider_failures() {
        let e = parse_ip_location(r#"{"success":false,"message":"Invalid IP"}"#).unwrap_err();
        assert!(format!("{e}").contains("Invalid IP"));
        assert!(parse_ip_location(r#"{"error":true,"reason":"RateLimited"}"#).is_err());
        assert!(parse_ip_location(r#"{"status":"fail","message":"reserved range"}"#).is_err());
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse_ip_location("not json").is_err());
        assert!(parse_ip_location("{}").is_err());
        assert!(parse_ip_location(r#"{"latitude":1.0}"#).is_err());
        assert!(parse_ip_location(r#"{"loc":"abc"}"#).is_err());
    }

    #[test]
    fn rejects_out_of_range_and_null_island() {
        assert!(parse_ip_location(r#"{"latitude":91.0,"longitude":0.5}"#).is_err());
        assert!(parse_ip_location(r#"{"latitude":10.0,"longitude":181.0}"#).is_err());
        assert!(parse_ip_location(r#"{"latitude":0.0,"longitude":0.0}"#).is_err());
    }

    #[test]
    fn blocking_lookup_success() {
        let mut server = mockito::Server::new();
        let m = server
            .mock("GET", "/")
            .with_status(200)
            .with_body(r#"{"success":true,"latitude":45.0,"longitude":16.0}"#)
            .create();
        let got = detect_location_from_ip_blocking(&server.url()).unwrap();
        assert_eq!(got, (45.0, 16.0));
        m.assert();
    }

    #[test]
    fn blocking_lookup_http_error() {
        let mut server = mockito::Server::new();
        let _m = server.mock("GET", "/").with_status(429).create();
        let e = detect_location_from_ip_blocking(&server.url()).unwrap_err();
        assert!(format!("{e}").contains("429"));
    }

    #[test]
    fn blocking_lookup_provider_error_body() {
        let mut server = mockito::Server::new();
        let _m = server
            .mock("GET", "/")
            .with_status(200)
            .with_body(r#"{"success":false,"message":"quota"}"#)
            .create();
        assert!(detect_location_from_ip_blocking(&server.url()).is_err());
    }

    #[test]
    fn blocking_lookup_unreachable() {
        assert!(detect_location_from_ip_blocking("http://127.0.0.1:1").is_err());
    }

    #[test]
    fn url_env_override() {
        // Single test touching the env var to avoid races between tests.
        unsafe { std::env::remove_var(IP_GEOLOCATION_URL_ENV) };
        assert_eq!(ip_geolocation_url(), DEFAULT_IP_GEOLOCATION_URL);
        unsafe { std::env::set_var(IP_GEOLOCATION_URL_ENV, "  ") };
        assert_eq!(ip_geolocation_url(), DEFAULT_IP_GEOLOCATION_URL);
        unsafe { std::env::set_var(IP_GEOLOCATION_URL_ENV, "http://example.test/geo") };
        assert_eq!(ip_geolocation_url(), "http://example.test/geo");
        unsafe { std::env::remove_var(IP_GEOLOCATION_URL_ENV) };
    }
}
