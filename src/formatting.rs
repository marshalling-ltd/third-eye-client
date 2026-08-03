//! Pure formatting and parsing helpers extracted from the UI layer for
//! testability.

use anyhow::{Context, Result};
use reqwest::Url;

/// Parses the stale-timeout config string (minutes) into milliseconds.
/// Falls back to 10 minutes (600 000 ms) on invalid input.
pub fn parse_stale_timeout_ms(value: &str) -> i64 {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|&v| v > 0.0)
        .map_or(600_000, |mins| (mins * 60_000.0) as i64)
}

/// Formats a byte count as a human-readable string (e.g. "1.5 MB").
pub fn format_bytes(bytes: i64) -> String {
    let bytes = bytes.max(0) as f64;
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes as i64, units[unit])
    } else {
        format!("{:.1} {}", value, units[unit])
    }
}

/// Formats an epoch-millis timestamp as a `YYYY-MM-DD HH:MM:SS` string.
///
/// On Unix platforms this uses `localtime_r` to produce local time; on all
/// other platforms (and as a fallback) it computes UTC using Hinnant's
/// civil-from-days algorithm.
#[allow(clippy::many_single_char_names)]
pub fn format_epoch_ms_datetime(ts_ms: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    let st = UNIX_EPOCH + Duration::from_millis(ts_ms as u64);
    #[cfg(unix)]
    {
        let secs = st.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        // SAFETY: localtime_r is thread-safe and writes into our stack `tm`.
        let res = unsafe { libc::localtime_r(&raw const secs, &raw mut tm) };
        if !res.is_null() {
            return format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                tm.tm_year + 1900,
                tm.tm_mon + 1,
                tm.tm_mday,
                tm.tm_hour,
                tm.tm_min,
                tm.tm_sec,
            );
        }
    }
    format_epoch_ms_utc(st.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs())
}

/// Pure UTC formatter — no platform dependencies, always deterministic.
/// Exposed for testing.
#[allow(clippy::many_single_char_names)]
pub fn format_epoch_ms_utc(secs: u64) -> String {
    let days = secs / 86_400;
    let day_secs = secs % 86_400;
    let h = day_secs / 3600;
    let m = (day_secs % 3600) / 60;
    let s = day_secs % 60;
    // Civil date from day count (algorithm from Howard Hinnant).
    let z = days as i64 + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let yr = if mo <= 2 { y + 1 } else { y };
    format!("{yr:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02} UTC")
}

/// Formats how long ago `ts_ms` was relative to `now_ms` (e.g. "5m ago").
#[must_use]
pub fn format_relative_age(now_ms: i64, ts_ms: i64) -> String {
    let diff_secs = ((now_ms - ts_ms).max(0) / 1000) as u64;
    if diff_secs < 10 {
        "just now".to_string()
    } else if diff_secs < 60 {
        format!("{diff_secs}s ago")
    } else if diff_secs < 3600 {
        format!("{}m ago", diff_secs / 60)
    } else if diff_secs < 86_400 {
        format!("{}h ago", diff_secs / 3600)
    } else {
        format!("{}d ago", diff_secs / 86_400)
    }
}

/// True if `name`'s extension is a still-image format the app can preview.
#[must_use]
pub fn is_image_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    std::path::Path::new(&lower)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jpg"))
        || std::path::Path::new(&lower)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jpeg"))
        || std::path::Path::new(&lower)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
        || std::path::Path::new(&lower)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("dng"))
}

/// True if `name`'s extension is a video format the app can play back.
#[must_use]
pub fn is_video_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    std::path::Path::new(&lower)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
        || std::path::Path::new(&lower)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mov"))
}

/// Builds the `GET /v1/medias/{name}/download` URL against `rov_http_base`.
pub fn build_media_download_url(rov_http_base: &str, name: &str) -> Result<String> {
    let base = rov_http_base.trim_end_matches('/');
    let mut url = Url::parse(base).with_context(|| format!("invalid ROV HTTP base URL: {base}"))?;
    {
        let mut segs = url
            .path_segments_mut()
            .map_err(|()| anyhow::anyhow!("URL cannot be a base: {base}"))?;
        segs.push("v1").push("medias").push(name).push("download");
    }
    Ok(url.to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- parse_stale_timeout_ms ------------------------------------------

    #[test]
    fn stale_timeout_valid_minutes() {
        assert_eq!(parse_stale_timeout_ms("5"), 300_000);
        assert_eq!(parse_stale_timeout_ms("10"), 600_000);
        assert_eq!(parse_stale_timeout_ms("0.5"), 30_000);
    }

    #[test]
    fn stale_timeout_trims_whitespace() {
        assert_eq!(parse_stale_timeout_ms("  10  "), 600_000);
    }

    #[test]
    fn stale_timeout_zero_falls_back() {
        assert_eq!(parse_stale_timeout_ms("0"), 600_000);
    }

    #[test]
    fn stale_timeout_negative_falls_back() {
        assert_eq!(parse_stale_timeout_ms("-5"), 600_000);
    }

    #[test]
    fn stale_timeout_empty_falls_back() {
        assert_eq!(parse_stale_timeout_ms(""), 600_000);
    }

    #[test]
    fn stale_timeout_non_numeric_falls_back() {
        assert_eq!(parse_stale_timeout_ms("abc"), 600_000);
    }

    // ---- format_bytes ----------------------------------------------------

    #[test]
    fn format_bytes_zero() {
        assert_eq!(format_bytes(0), "0 B");
    }

    #[test]
    fn format_bytes_small() {
        assert_eq!(format_bytes(500), "500 B");
    }

    #[test]
    fn format_bytes_exactly_1kb() {
        assert_eq!(format_bytes(1024), "1.0 KB");
    }

    #[test]
    fn format_bytes_megabyte_range() {
        // 1.5 MB = 1_572_864 bytes
        assert_eq!(format_bytes(1_572_864), "1.5 MB");
    }

    #[test]
    fn format_bytes_gigabyte_range() {
        // 2 GB = 2_147_483_648 bytes
        assert_eq!(format_bytes(2_147_483_648), "2.0 GB");
    }

    #[test]
    fn format_bytes_negative_clamps_to_zero() {
        assert_eq!(format_bytes(-100), "0 B");
    }

    // ---- format_epoch_ms_utc ---------------------------------------------

    #[test]
    fn utc_unix_epoch() {
        assert_eq!(format_epoch_ms_utc(0), "1970-01-01 00:00:00 UTC");
    }

    #[test]
    fn utc_known_timestamp() {
        // 2024-01-15 12:30:45 UTC = 1705321845
        assert_eq!(
            format_epoch_ms_utc(1_705_321_845),
            "2024-01-15 12:30:45 UTC"
        );
    }

    #[test]
    fn utc_year_2000() {
        // 2000-01-01 00:00:00 UTC = 946684800
        assert_eq!(format_epoch_ms_utc(946_684_800), "2000-01-01 00:00:00 UTC");
    }

    #[test]
    fn utc_leap_year_feb_29() {
        // 2024-02-29 23:59:59 UTC = 1709251199
        assert_eq!(
            format_epoch_ms_utc(1_709_251_199),
            "2024-02-29 23:59:59 UTC"
        );
    }

    #[test]
    fn format_epoch_ms_datetime_returns_string() {
        // Just verify it doesn't panic and returns a non-empty string.
        let result = format_epoch_ms_datetime(1_705_321_845_000);
        assert!(!result.is_empty());
        // Should contain a date-like pattern.
        assert!(result.contains("2024"));
    }

    // ---- format_relative_age ----------------------------------------------

    #[test]
    fn relative_age_just_now() {
        assert_eq!(format_relative_age(10_000, 9_500), "just now");
    }

    #[test]
    fn relative_age_seconds() {
        assert_eq!(format_relative_age(45_000, 10_000), "35s ago");
    }

    #[test]
    fn relative_age_minutes() {
        assert_eq!(format_relative_age(600_000, 0), "10m ago");
    }

    #[test]
    fn relative_age_hours() {
        assert_eq!(format_relative_age(3 * 3_600_000, 0), "3h ago");
    }

    #[test]
    fn relative_age_days() {
        assert_eq!(format_relative_age(2 * 86_400_000, 0), "2d ago");
    }

    #[test]
    fn relative_age_future_timestamp_clamps_to_zero() {
        // ts_ms in the future relative to now_ms should not go negative.
        assert_eq!(format_relative_age(0, 10_000), "just now");
    }

    // ---- is_image_name / is_video_name -------------------------------------

    #[test]
    fn image_name_recognizes_supported_extensions() {
        for name in ["a.jpg", "a.JPG", "a.jpeg", "a.png", "a.dng"] {
            assert!(is_image_name(name), "{name} should be an image");
        }
    }

    #[test]
    fn image_name_rejects_other_extensions() {
        assert!(!is_image_name("a.mp4"));
        assert!(!is_image_name("a"));
        assert!(!is_image_name(""));
    }

    #[test]
    fn video_name_recognizes_supported_extensions() {
        for name in ["a.mp4", "a.MOV", "a.mov"] {
            assert!(is_video_name(name), "{name} should be a video");
        }
    }

    #[test]
    fn video_name_rejects_other_extensions() {
        assert!(!is_video_name("a.jpg"));
        assert!(!is_video_name(""));
    }

    // ---- build_media_download_url ------------------------------------------

    #[test]
    fn media_download_url_builds_expected_path() {
        let url = build_media_download_url("http://192.168.1.88", "a.jpeg").unwrap();
        assert_eq!(url, "http://192.168.1.88/v1/medias/a.jpeg/download");
    }

    #[test]
    fn media_download_url_strips_trailing_slash() {
        let url = build_media_download_url("http://192.168.1.88/", "a.jpeg").unwrap();
        assert_eq!(url, "http://192.168.1.88/v1/medias/a.jpeg/download");
    }

    #[test]
    fn media_download_url_rejects_invalid_base() {
        assert!(build_media_download_url("not a url", "a.jpeg").is_err());
    }
}
