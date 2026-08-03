//! Pure parsing/selection helpers for the GitHub-releases update checker.
//!
//! The actual HTTP call lives in the UI layer (`main.rs`); this module holds
//! the logic that's testable without a network connection.

use serde::Deserialize;

/// A single downloadable asset attached to a GitHub release.
#[derive(Deserialize)]
pub struct GithubReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}

/// Strips a leading `v`/`V` from a release tag (e.g. `"v1.2.3"` -> `"1.2.3"`).
/// Returns `None` if nothing remains after stripping.
#[must_use]
pub fn normalize_release_tag(tag: &str) -> Option<String> {
    let stripped = tag.trim().trim_start_matches(['v', 'V']);
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_owned())
    }
}

/// Parses a `major.minor.patch` version string, ignoring any
/// pre-release/build metadata suffix (`-` or `+`). Returns `None` for
/// anything that isn't exactly three numeric components.
#[must_use]
pub fn parse_version_triplet(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.split(['-', '+']).next()?;
    let mut pieces = core.split('.');
    let major = pieces.next()?.parse::<u64>().ok()?;
    let minor = pieces.next()?.parse::<u64>().ok()?;
    let patch = pieces.next()?.parse::<u64>().ok()?;
    if pieces.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Picks the first release asset matching this platform's installer format.
#[must_use]
pub fn pick_download_url_for_platform(assets: &[GithubReleaseAsset]) -> Option<String> {
    assets
        .iter()
        .find(|asset| is_platform_release_asset(&asset.name))
        .map(|asset| asset.browser_download_url.clone())
}

/// Returns whether `name` looks like this platform's installer (`.dmg` on
/// macOS, `.exe` on Windows, `.AppImage` on Linux).
#[must_use]
pub fn is_platform_release_asset(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    #[cfg(target_os = "macos")]
    {
        return std::path::Path::new(&lowered)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("dmg"));
    }
    #[cfg(target_os = "windows")]
    {
        return std::path::Path::new(&lowered)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"));
    }
    #[cfg(target_os = "linux")]
    {
        return std::path::Path::new(&lowered)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("appimage"));
    }
    #[allow(unreachable_code)]
    {
        let _ = lowered;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- normalize_release_tag ---------------------------------------------

    #[test]
    fn normalize_strips_v_prefix() {
        assert_eq!(normalize_release_tag("v1.2.3"), Some("1.2.3".to_string()));
        assert_eq!(normalize_release_tag("V1.2.3"), Some("1.2.3".to_string()));
    }

    #[test]
    fn normalize_handles_no_prefix() {
        assert_eq!(normalize_release_tag("1.2.3"), Some("1.2.3".to_string()));
    }

    #[test]
    fn normalize_trims_whitespace() {
        assert_eq!(
            normalize_release_tag("  v1.2.3  "),
            Some("1.2.3".to_string())
        );
    }

    #[test]
    fn normalize_rejects_empty_after_strip() {
        assert_eq!(normalize_release_tag("v"), None);
        assert_eq!(normalize_release_tag(""), None);
    }

    // ---- parse_version_triplet ---------------------------------------------

    #[test]
    fn version_triplet_parses_plain() {
        assert_eq!(parse_version_triplet("1.2.3"), Some((1, 2, 3)));
    }

    #[test]
    fn version_triplet_ignores_prerelease_suffix() {
        assert_eq!(parse_version_triplet("1.2.3-beta.1"), Some((1, 2, 3)));
    }

    #[test]
    fn version_triplet_ignores_build_metadata() {
        assert_eq!(parse_version_triplet("1.2.3+build42"), Some((1, 2, 3)));
    }

    #[test]
    fn version_triplet_rejects_too_few_components() {
        assert_eq!(parse_version_triplet("1.2"), None);
    }

    #[test]
    fn version_triplet_rejects_too_many_components() {
        assert_eq!(parse_version_triplet("1.2.3.4"), None);
    }

    #[test]
    fn version_triplet_rejects_non_numeric() {
        assert_eq!(parse_version_triplet("a.b.c"), None);
    }

    #[test]
    fn version_triplet_orders_correctly() {
        assert!(parse_version_triplet("1.2.3") < parse_version_triplet("1.3.0"));
        assert!(parse_version_triplet("2.0.0") > parse_version_triplet("1.99.99"));
    }

    // ---- pick_download_url_for_platform / is_platform_release_asset -------

    fn asset(name: &str) -> GithubReleaseAsset {
        GithubReleaseAsset {
            name: name.to_string(),
            browser_download_url: format!("https://example.test/{name}"),
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn platform_asset_matches_dmg_on_macos() {
        assert!(is_platform_release_asset("third-eye-client.dmg"));
        assert!(is_platform_release_asset("Third-Eye-Client.DMG"));
        assert!(!is_platform_release_asset("third-eye-client.exe"));
        assert!(!is_platform_release_asset("third-eye-client.AppImage"));
    }

    #[test]
    fn pick_download_url_finds_matching_asset() {
        let assets = vec![
            asset("third-eye-client.exe"),
            asset("third-eye-client.AppImage"),
            #[cfg(target_os = "macos")]
            asset("third-eye-client.dmg"),
        ];
        let picked = pick_download_url_for_platform(&assets);
        #[cfg(target_os = "macos")]
        assert_eq!(
            picked,
            Some("https://example.test/third-eye-client.dmg".to_string())
        );
        #[cfg(not(target_os = "macos"))]
        let _ = picked;
    }

    #[test]
    fn pick_download_url_returns_none_when_no_match() {
        let assets = vec![asset("readme.txt"), asset("checksums.sha256")];
        assert_eq!(pick_download_url_for_platform(&assets), None);
    }
}
