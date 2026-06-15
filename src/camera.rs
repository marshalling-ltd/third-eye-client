use anyhow::{Context, Result};
use reqwest::Url;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use std::net::IpAddr;

/// Photo format accepted by the camera's `/v1/capture` endpoint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PhotoFormat {
    #[default]
    Jpeg,
    Dng,
    JpegDng,
}

impl PhotoFormat {
    #[must_use]
    pub fn as_api_str(self) -> &'static str {
        match self {
            PhotoFormat::Jpeg => "JPEG",
            PhotoFormat::Dng => "DNG",
            PhotoFormat::JpegDng => "JPEG+DNG",
        }
    }
}

/// Request body for `POST /v1/capture`.
#[derive(Clone, Debug, Serialize)]
pub struct CaptureRequest {
    pub format: String,
    pub burst: u8,
}

impl CaptureRequest {
    #[must_use]
    pub fn new(format: PhotoFormat, burst: u8) -> Self {
        Self {
            format: format.as_api_str().to_owned(),
            burst: burst.clamp(1, 5),
        }
    }
}

/// Success or failure envelope returned by the camera for `/v1/capture`.
///
/// Per the Chasing ROV camera `OpenAPI` spec (camera FW >= 7.10.0):
/// - Success (HTTP 201): `{"status":0,"msg":"success","data":null}`.
/// - Failure (HTTP 500): `{"code":...,"error":"...","status":...,"msg":"...","data":null,"errors":[...]}`
///   where `errors[*].meta` may contain an `{"ip": "..."}` object (multi-camera setups).
#[derive(Clone, Debug, Deserialize, Default)]
pub struct CaptureResponse {
    /// Legacy (compat) outer error code. Matches `status` when present.
    #[serde(default)]
    pub code: Option<i64>,
    /// Legacy (compat) error message.
    #[serde(default)]
    pub error: Option<String>,
    /// New-format status code. `0` indicates success.
    #[serde(default)]
    pub status: Option<i64>,
    /// Human-readable status / error message.
    #[serde(default)]
    pub msg: Option<String>,
    /// Per-camera error list (main camera aggregates sub-camera errors here).
    #[serde(default)]
    pub errors: Option<Vec<CaptureSubError>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CaptureSubError {
    pub code: i64,
    pub msg: String,
    #[serde(default)]
    pub meta: Option<CaptureErrorMeta>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CaptureErrorMeta {
    #[serde(default)]
    pub ip: Option<String>,
}

/// Capture scene filter for `GET /v1/medias`.
///
/// Mirrors the integer codes documented in the OPEN API spec.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MediaScene {
    #[default]
    Normal,
    VesselInspection,
    FishingNet,
}

impl MediaScene {
    #[must_use]
    pub fn as_query_int(self) -> i32 {
        match self {
            MediaScene::Normal => 0,
            MediaScene::VesselInspection => 1,
            MediaScene::FishingNet => 2,
        }
    }
}

/// File status reported by the camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaFileStat {
    Normal,
    NeedsRepair,
    Repairing,
    RepairFailed,
    Other(i32),
}

impl MediaFileStat {
    #[must_use]
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => MediaFileStat::Normal,
            1 => MediaFileStat::NeedsRepair,
            2 => MediaFileStat::Repairing,
            3 => MediaFileStat::RepairFailed,
            other => MediaFileStat::Other(other),
        }
    }
}

/// Entry in the array returned by `GET /v1/medias`.
#[derive(Clone, Debug, Deserialize)]
pub struct MediaInfo {
    pub name: String,
    pub size: u64,
    pub canplayback: bool,
    pub origin: MediaOrigin,
    #[serde(default)]
    pub play: Option<VideoStat>,
    #[serde(default)]
    pub osd: Option<MediaOsd>,
}

/// Origin file (image or video) metadata.
#[derive(Clone, Debug, Deserialize)]
pub struct MediaOrigin {
    pub width: i32,
    pub height: i32,
    pub duration: i32,
    pub fps: i32,
    pub br: i32,
    pub multi: i32,
    #[serde(rename = "withOsd")]
    pub with_osd: bool,
    pub id: String,
    pub stat: i32,
}

impl MediaOrigin {
    #[must_use]
    pub fn file_stat(&self) -> MediaFileStat {
        MediaFileStat::from_code(self.stat)
    }
}

/// Playback file metadata (only populated for video files).
#[derive(Clone, Debug, Deserialize)]
pub struct VideoStat {
    pub stat: i32,
}

impl VideoStat {
    #[must_use]
    pub fn file_stat(&self) -> MediaFileStat {
        MediaFileStat::from_code(self.stat)
    }
}

/// Deprecated OSD companion file metadata (see spec).
#[derive(Clone, Debug, Deserialize)]
pub struct MediaOsd {
    pub size: u64,
    pub stat: i32,
}

/// Path-mode selector shared by the single-file download/info endpoints.
///
/// Matches the `which` query parameter documented in the spec (empty or `play`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MediaWhich {
    #[default]
    Original,
    Play,
}

impl MediaWhich {
    #[must_use]
    pub fn as_query_value(self) -> Option<&'static str> {
        match self {
            MediaWhich::Original => None,
            MediaWhich::Play => Some("play"),
        }
    }
}

/// Selects the alternate info-file variant returned by `/v1/medias/{name}/info`.
///
/// Matches the `for` query parameter; `repair` requests the repair info file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MediaInfoFor {
    #[default]
    Default,
    Repair,
}

impl MediaInfoFor {
    #[must_use]
    pub fn as_query_value(self) -> Option<&'static str> {
        match self {
            MediaInfoFor::Default => None,
            MediaInfoFor::Repair => Some("repair"),
        }
    }
}

/// Response body of `GET /v1/medias/{name}/info`.
#[derive(Clone, Debug, Deserialize)]
pub struct SingleMediaInfo {
    pub name: String,
    pub size: u64,
    pub width: i32,
    pub height: i32,
    pub duration: i32,
    pub fps: i32,
    pub br: i32,
    pub multi: i32,
    #[serde(rename = "withOsd", default)]
    pub with_osd: bool,
    pub id: String,
}

/// Downloaded media payload plus metadata exposed by reqwest.
#[derive(Clone, Debug)]
pub struct DownloadedMedia {
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

/// Envelope returned by the lamp endpoints.
#[derive(Clone, Debug, Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
pub struct LampEnvelope<T> {
    #[serde(default)]
    pub status: i64,
    #[serde(default)]
    pub msg: String,
    #[serde(default = "Option::default")]
    pub data: Option<T>,
}

/// `data` payload of `GET /v1/lamp`.
#[derive(Clone, Debug, Deserialize)]
pub struct LampBrightnessData {
    pub brightness: i32,
}

/// Request body of `POST /v1/lamp`.
#[derive(Clone, Debug, Serialize)]
pub struct SetBrightnessRequest {
    pub brightness: i32,
}

/// `code`/`error` envelope used by legacy error responses.
#[derive(Clone, Debug, Deserialize, Default)]
struct LegacyErrorEnvelope {
    #[serde(default)]
    code: Option<i64>,
    #[serde(default)]
    error: Option<String>,
}

/// HTTP client for the Chasing ROV camera OPEN API.
pub struct CameraApiClient {
    base_url: String,
    http: Client,
}

impl CameraApiClient {
    #[must_use]
    pub fn new(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            http: Client::new(),
        }
    }

    /// Constructor that binds outgoing connections to a specific network
    /// interface (e.g. `"en10"` for a USB ethernet adapter).
    ///
    /// On macOS this uses `IP_BOUND_IF`; on Linux `SO_BINDTODEVICE`. On
    /// Windows reqwest cannot bind by interface name, so the chosen NIC's
    /// local IPv4 is resolved and used as the client's bind address instead.
    /// Pass `None` for default OS routing (equivalent to [`Self::new`]).
    #[must_use]
    #[allow(unused_variables)]
    pub fn new_bound(base_url: String, interface: Option<&str>) -> Self {
        #[allow(unused_mut)]
        let mut builder = Client::builder();
        #[cfg(unix)]
        if let Some(iface) = interface {
            builder = builder.interface(iface);
        }
        // reqwest cannot bind to an interface by name on Windows, but binding
        // the client's local address to that NIC's IPv4 selects the same
        // source interface for outgoing ROV connections.
        #[cfg(windows)]
        if let Some(iface) = interface
            && let Some(ip) = local_ipv4_for_interface(iface)
        {
            builder = builder.local_address(ip);
        }
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            http: builder.build().unwrap_or_else(|_| Client::new()),
        }
    }

    /// Convenience constructor that accepts a pre-built `reqwest` blocking client.
    #[must_use]
    pub fn with_http(base_url: String, http: Client) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            http,
        }
    }

    fn build_url(&self, segments: &[&str]) -> Result<Url> {
        let mut url = Url::parse(&self.base_url)
            .with_context(|| format!("invalid base URL: {}", self.base_url))?;
        {
            let mut path_segments = url
                .path_segments_mut()
                .map_err(|()| anyhow::anyhow!("base URL {} cannot be a base", self.base_url))?;
            for segment in segments {
                path_segments.push(segment);
            }
        }
        Ok(url)
    }

    /// POST `/v1/capture` with the given photo format and burst count.
    ///
    /// Returns the parsed response envelope on success (HTTP 2xx and
    /// `status == 0`). On any transport failure, non-success HTTP status, or
    /// non-zero camera status, returns an `Err` with a human-readable message
    /// that includes the sub-camera error list when available.
    pub fn capture(&self, format: PhotoFormat, burst: u8) -> Result<CaptureResponse> {
        let url = self.build_url(&["v1", "capture"])?;
        let body = CaptureRequest::new(format, burst);
        let response = self
            .http
            .post(url)
            .json(&body)
            .send()
            .context("capture request failed")?;

        let http_status = response.status();
        let payload: CaptureResponse = response
            .json()
            .context("invalid capture JSON payload")
            .unwrap_or_default();

        let camera_status = payload.status.or(payload.code).unwrap_or(0);
        if http_status.is_success() && camera_status == 0 {
            return Ok(payload);
        }

        let base_msg = payload
            .msg
            .clone()
            .or_else(|| payload.error.clone())
            .unwrap_or_else(|| format!("HTTP {http_status}"));
        let mut detail = format!("capture failed: {base_msg} (status={camera_status})");
        if let Some(errors) = &payload.errors
            && !errors.is_empty()
        {
            detail.push_str("; errors=[");
            for (i, sub) in errors.iter().enumerate() {
                if i > 0 {
                    detail.push_str(", ");
                }
                let _ = write!(detail, "code={} msg={:?}", sub.code, sub.msg);
                if let Some(meta) = &sub.meta
                    && let Some(ip) = &meta.ip
                {
                    let _ = write!(detail, " ip={ip}");
                }
            }
            detail.push(']');
        }
        anyhow::bail!(detail)
    }

    /// GET `/v1/medias`, optionally filtering by scene.
    ///
    /// The deprecated `camera` query parameter is intentionally not exposed.
    pub fn list_medias(&self, scene: Option<MediaScene>) -> Result<Vec<MediaInfo>> {
        let mut url = self.build_url(&["v1", "medias"])?;
        if let Some(scene) = scene {
            url.query_pairs_mut()
                .append_pair("scene", &scene.as_query_int().to_string());
        }
        let response = self
            .http
            .get(url)
            .send()
            .context("list medias request failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            anyhow::bail!(format_legacy_error("list medias", status, &body));
        }
        response
            .json::<Vec<MediaInfo>>()
            .context("invalid medias JSON payload")
    }

    /// GET `/v1/medias/{name}/download`.
    ///
    /// Set `which` to `MediaWhich::Play` to download the small video variant.
    pub fn download_media(&self, name: &str, which: MediaWhich) -> Result<DownloadedMedia> {
        if name.is_empty() {
            anyhow::bail!("download_media: name must not be empty");
        }
        let mut url = self.build_url(&["v1", "medias", name, "download"])?;
        if let Some(value) = which.as_query_value() {
            url.query_pairs_mut().append_pair("which", value);
        }
        let response = self
            .http
            .get(url)
            .send()
            .context("download media request failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            anyhow::bail!(format_legacy_error("download media", status, &body));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = response
            .bytes()
            .context("failed to read download media body")?
            .to_vec();
        Ok(DownloadedMedia {
            content_type,
            bytes,
        })
    }

    /// DELETE `/v1/medias/{name}`.
    pub fn delete_media(&self, name: &str) -> Result<()> {
        if name.is_empty() {
            anyhow::bail!("delete_media: name must not be empty");
        }
        let url = self.build_url(&["v1", "medias", name])?;
        let response = self
            .http
            .delete(url)
            .send()
            .context("delete media request failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            anyhow::bail!(format_legacy_error("delete media", status, &body));
        }
        Ok(())
    }

    /// GET `/v1/medias/{name}/info`.
    ///
    /// `for_info` selects the standard info file (`Default`) or the repair info
    /// file (`Repair`). `which` is only meaningful when `for_info == Repair`.
    pub fn media_info(
        &self,
        name: &str,
        for_info: MediaInfoFor,
        which: MediaWhich,
    ) -> Result<SingleMediaInfo> {
        if name.is_empty() {
            anyhow::bail!("media_info: name must not be empty");
        }
        let mut url = self.build_url(&["v1", "medias", name, "info"])?;
        {
            let mut pairs = url.query_pairs_mut();
            if let Some(value) = for_info.as_query_value() {
                pairs.append_pair("for", value);
            }
            if let Some(value) = which.as_query_value() {
                pairs.append_pair("which", value);
            }
        }
        let response = self
            .http
            .get(url)
            .send()
            .context("media info request failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            anyhow::bail!(format_legacy_error("media info", status, &body));
        }
        response
            .json::<SingleMediaInfo>()
            .context("invalid media info JSON payload")
    }

    /// GET `/v1/lamp` returning the current LED light brightness.
    pub fn get_led_brightness(&self) -> Result<i32> {
        let url = self.build_url(&["v1", "lamp"])?;
        let response = self
            .http
            .get(url)
            .send()
            .context("get lamp brightness request failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            anyhow::bail!(format_legacy_error("get lamp brightness", status, &body));
        }
        let envelope: LampEnvelope<LampBrightnessData> = response
            .json()
            .context("invalid lamp brightness JSON payload")?;
        check_lamp_status("get lamp brightness", &envelope)?;
        let data = envelope
            .data
            .ok_or_else(|| anyhow::anyhow!("lamp brightness response missing data field"))?;
        Ok(data.brightness)
    }

    /// POST `/v1/lamp` to set the LED light brightness.
    pub fn set_led_brightness(&self, brightness: i32) -> Result<()> {
        let url = self.build_url(&["v1", "lamp"])?;
        let body = SetBrightnessRequest { brightness };
        let response = self
            .http
            .post(url)
            .json(&body)
            .send()
            .context("set lamp brightness request failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            anyhow::bail!(format_legacy_error("set lamp brightness", status, &body));
        }
        let envelope: LampEnvelope<serde_json::Value> =
            response.json().context("invalid lamp JSON payload")?;
        check_lamp_status("set lamp brightness", &envelope)?;
        Ok(())
    }
}

fn check_lamp_status<T>(op: &str, envelope: &LampEnvelope<T>) -> Result<()> {
    if envelope.status == 0 {
        return Ok(());
    }
    let msg = if envelope.msg.is_empty() {
        "<no msg>".to_string()
    } else {
        envelope.msg.clone()
    };
    anyhow::bail!("{op} failed: status={} msg={msg}", envelope.status);
}

fn format_legacy_error(op: &str, status: reqwest::StatusCode, body: &str) -> String {
    if body.is_empty() {
        return format!("{op} failed with HTTP {status}");
    }
    if let Ok(envelope) = serde_json::from_str::<LegacyErrorEnvelope>(body) {
        let code = envelope.code.map(|c| c.to_string()).unwrap_or_default();
        let err = envelope.error.unwrap_or_default();
        if !code.is_empty() || !err.is_empty() {
            return format!("{op} failed (HTTP {status}, code={code}, error={err:?})");
        }
    }
    format!("{op} failed (HTTP {status}): {body}")
}

/// Resolves the first non-loopback IPv4 address of the named interface.
///
/// Used on Windows, where reqwest cannot bind to an interface by name, to
/// translate the chosen interface into a `local_address` bind target. Pure
/// over the supplied interface list so it can be unit-tested on any platform;
/// loopback interfaces, name mismatches, and IPv6-only interfaces are skipped.
#[must_use]
pub fn local_ipv4_for_interface_from(ifaces: &[if_addrs::Interface], name: &str) -> Option<IpAddr> {
    ifaces.iter().find_map(|iface| {
        if iface.name != name || iface.is_loopback() {
            return None;
        }
        match &iface.addr {
            if_addrs::IfAddr::V4(v4) => Some(IpAddr::V4(v4.ip)),
            if_addrs::IfAddr::V6(_) => None,
        }
    })
}

/// Live Windows wrapper around [`local_ipv4_for_interface_from`] that
/// enumerates the host's interfaces.
#[cfg(windows)]
fn local_ipv4_for_interface(name: &str) -> Option<IpAddr> {
    let ifaces = if_addrs::get_if_addrs().ok()?;
    local_ipv4_for_interface_from(&ifaces, name)
}

#[cfg(test)]
mod tests {
    use reqwest::StatusCode;
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    fn make_iface(name: &str, addr: if_addrs::IfAddr) -> if_addrs::Interface {
        if_addrs::Interface {
            name: name.to_string(),
            addr,
            index: None,
            oper_status: if_addrs::IfOperStatus::Up,
            is_p2p: false,
            #[cfg(windows)]
            adapter_name: String::new(),
        }
    }

    fn v4_iface(name: &str, ip: [u8; 4]) -> if_addrs::Interface {
        make_iface(
            name,
            if_addrs::IfAddr::V4(if_addrs::Ifv4Addr {
                ip: Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]),
                netmask: Ipv4Addr::new(255, 255, 255, 0),
                prefixlen: 24,
                broadcast: None,
            }),
        )
    }

    fn v6_iface(name: &str) -> if_addrs::Interface {
        make_iface(
            name,
            if_addrs::IfAddr::V6(if_addrs::Ifv6Addr {
                ip: Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1),
                netmask: Ipv6Addr::new(0xffff, 0xffff, 0xffff, 0xffff, 0, 0, 0, 0),
                prefixlen: 64,
                broadcast: None,
            }),
        )
    }

    fn loopback_iface() -> if_addrs::Interface {
        make_iface(
            "lo0",
            if_addrs::IfAddr::V4(if_addrs::Ifv4Addr {
                ip: Ipv4Addr::LOCALHOST,
                netmask: Ipv4Addr::new(255, 0, 0, 0),
                prefixlen: 8,
                broadcast: None,
            }),
        )
    }

    #[test]
    fn local_ipv4_picks_named_interface() {
        let ifaces = vec![
            v4_iface("en5", [192, 168, 1, 9]),
            v4_iface("en0", [10, 0, 0, 2]),
        ];
        assert_eq!(
            local_ipv4_for_interface_from(&ifaces, "en0"),
            Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)))
        );
    }

    #[test]
    fn local_ipv4_skips_loopback_and_mismatch() {
        let ifaces = vec![loopback_iface(), v4_iface("en5", [192, 168, 1, 9])];
        assert_eq!(local_ipv4_for_interface_from(&ifaces, "en9"), None);
        assert_eq!(local_ipv4_for_interface_from(&ifaces, "lo0"), None);
    }

    #[test]
    fn local_ipv4_scans_past_ipv6_entry() {
        // The IPv6 entry for `en5` precedes its IPv4 entry; the scan must skip
        // past V6 and still find the V4 address.
        let ifaces = vec![v6_iface("en5"), v4_iface("en5", [192, 168, 1, 50])];
        assert_eq!(
            local_ipv4_for_interface_from(&ifaces, "en5"),
            Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)))
        );
    }

    #[test]
    fn local_ipv4_none_when_only_ipv6() {
        let ifaces = vec![v6_iface("en5")];
        assert_eq!(local_ipv4_for_interface_from(&ifaces, "en5"), None);
    }

    #[test]
    fn photo_format_api_strings() {
        assert_eq!(PhotoFormat::Jpeg.as_api_str(), "JPEG");
        assert_eq!(PhotoFormat::Dng.as_api_str(), "DNG");
        assert_eq!(PhotoFormat::JpegDng.as_api_str(), "JPEG+DNG");
    }

    #[test]
    fn capture_request_clamps_burst() {
        assert_eq!(CaptureRequest::new(PhotoFormat::Jpeg, 0).burst, 1);
        assert_eq!(CaptureRequest::new(PhotoFormat::Jpeg, 3).burst, 3);
        assert_eq!(CaptureRequest::new(PhotoFormat::Jpeg, 9).burst, 5);
    }

    #[test]
    fn capture_request_serializes_expected_fields() {
        let body = CaptureRequest::new(PhotoFormat::Jpeg, 1);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["format"], "JPEG");
        assert_eq!(json["burst"], 1);
    }

    #[test]
    fn parse_success_envelope() {
        let payload = r#"{"status":0,"msg":"success","data":null}"#;
        let parsed: CaptureResponse = serde_json::from_str(payload).unwrap();
        assert_eq!(parsed.status, Some(0));
        assert_eq!(parsed.msg.as_deref(), Some("success"));
    }

    #[test]
    fn parse_multi_camera_error_envelope() {
        let payload = r#"{
            "code": 4325377,
            "error": "capture parallel error",
            "status": 4325377,
            "msg": "capture parallel error",
            "data": null,
            "errors": [
                {"code": 1002, "msg": "format xx not supported", "meta": null},
                {"code": 4390913, "msg": "timeout", "meta": {"ip": "192.168.1.104"}}
            ]
        }"#;
        let parsed: CaptureResponse = serde_json::from_str(payload).unwrap();
        assert_eq!(parsed.status, Some(4_325_377));
        let errors = parsed.errors.expect("errors present");
        assert_eq!(errors.len(), 2);
        assert_eq!(
            errors[1].meta.as_ref().and_then(|m| m.ip.as_deref()),
            Some("192.168.1.104")
        );
    }

    #[test]
    fn media_scene_query_ints() {
        assert_eq!(MediaScene::Normal.as_query_int(), 0);
        assert_eq!(MediaScene::VesselInspection.as_query_int(), 1);
        assert_eq!(MediaScene::FishingNet.as_query_int(), 2);
    }

    #[test]
    fn media_which_and_info_for_query_values() {
        assert_eq!(MediaWhich::Original.as_query_value(), None);
        assert_eq!(MediaWhich::Play.as_query_value(), Some("play"));
        assert_eq!(MediaInfoFor::Default.as_query_value(), None);
        assert_eq!(MediaInfoFor::Repair.as_query_value(), Some("repair"));
    }

    #[test]
    fn media_file_stat_mapping() {
        assert_eq!(MediaFileStat::from_code(0), MediaFileStat::Normal);
        assert_eq!(MediaFileStat::from_code(1), MediaFileStat::NeedsRepair);
        assert_eq!(MediaFileStat::from_code(2), MediaFileStat::Repairing);
        assert_eq!(MediaFileStat::from_code(3), MediaFileStat::RepairFailed);
        assert_eq!(MediaFileStat::from_code(42), MediaFileStat::Other(42));
    }

    #[test]
    fn parse_media_list_example() {
        let payload = r#"[
            {
                "name": "GLDS0329_144128968.jpeg",
                "size": 1064293,
                "canplayback": false,
                "origin": {
                    "width": 0, "height": 0, "duration": 0,
                    "fps": 0, "br": 0, "multi": 0,
                    "withOsd": false, "id": "GLDS0329_144128968.jpeg", "stat": 0
                },
                "play": null,
                "osd": null
            },
            {
                "name": "GLDS0410_183055351.mp4",
                "size": 63801500,
                "canplayback": true,
                "origin": {
                    "width": 3840, "height": 2160, "duration": 27,
                    "fps": 30, "br": 20480, "multi": 0,
                    "withOsd": false, "id": "GLDS0410_183055351.mp4", "stat": 0
                },
                "play": {"stat": 0},
                "osd": null
            }
        ]"#;
        let parsed: Vec<MediaInfo> = serde_json::from_str(payload).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "GLDS0329_144128968.jpeg");
        assert!(!parsed[0].canplayback);
        assert!(parsed[0].play.is_none());
        assert!(parsed[1].canplayback);
        let play = parsed[1].play.as_ref().unwrap();
        assert_eq!(play.file_stat(), MediaFileStat::Normal);
        assert_eq!(parsed[1].origin.file_stat(), MediaFileStat::Normal);
        assert_eq!(parsed[1].origin.width, 3840);
    }

    #[test]
    fn parse_single_media_info_example() {
        let payload = r#"{
            "name": "GLDS0507_200835616.mp4",
            "size": 10282174,
            "width": 1920,
            "height": 1080,
            "duration": 17,
            "fps": 120,
            "br": 20480,
            "multi": -4,
            "withOsd": false,
            "id": "GLDS0507_200835616.mp4"
        }"#;
        let parsed: SingleMediaInfo = serde_json::from_str(payload).unwrap();
        assert_eq!(parsed.name, "GLDS0507_200835616.mp4");
        assert_eq!(parsed.multi, -4);
        assert_eq!(parsed.width, 1920);
        assert!(!parsed.with_osd);
    }

    #[test]
    fn parse_lamp_get_envelope() {
        let payload = r#"{"status":0,"msg":"success","data":{"brightness":10}}"#;
        let parsed: LampEnvelope<LampBrightnessData> = serde_json::from_str(payload).unwrap();
        assert_eq!(parsed.status, 0);
        assert_eq!(parsed.data.unwrap().brightness, 10);
    }

    #[test]
    fn set_brightness_request_serializes() {
        let body = SetBrightnessRequest { brightness: 70 };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["brightness"], 70);
    }

    #[test]
    fn build_url_encodes_filename_segments() {
        let client = CameraApiClient::new("http://192.168.1.88".to_string());
        let url = client
            .build_url(&["v1", "medias", "a b/c.mp4", "download"])
            .unwrap();
        // `push` percent-encodes the filename segment so that the slash does
        // not escape the intended path segment.
        assert_eq!(
            url.as_str(),
            "http://192.168.1.88/v1/medias/a%20b%2Fc.mp4/download"
        );
    }

    #[test]
    fn check_lamp_status_errors_on_non_zero() {
        let envelope = LampEnvelope::<serde_json::Value> {
            status: 1234,
            msg: "boom".to_string(),
            data: None,
        };
        let err = check_lamp_status("set lamp brightness", &envelope).unwrap_err();
        let text = format!("{err}");
        assert!(text.contains("1234"));
        assert!(text.contains("boom"));
    }

    #[test]
    fn format_legacy_error_extracts_code_and_message() {
        let body = r#"{"code":1101,"error":"file not found"}"#;
        let msg = format_legacy_error("media info", reqwest::StatusCode::NOT_FOUND, body);
        assert!(msg.contains("404"));
        assert!(msg.contains("1101"));
        assert!(msg.contains("file not found"));
    }

    #[test]
    fn photo_format_as_api_str() {
        assert_eq!(PhotoFormat::Jpeg.as_api_str(), "JPEG");
        assert_eq!(PhotoFormat::Dng.as_api_str(), "DNG");
        assert_eq!(PhotoFormat::JpegDng.as_api_str(), "JPEG+DNG");
    }

    #[test]
    fn media_file_stat_handles_unknown_code() {
        assert_eq!(MediaFileStat::from_code(999), MediaFileStat::Other(999));
    }

    #[test]
    fn capture_response_handles_missing_fields() {
        let payload = r#"{"status":0}"#;
        let parsed: CaptureResponse = serde_json::from_str(payload).unwrap();
        assert_eq!(parsed.status, Some(0));
        assert!(parsed.msg.is_none());
        assert!(parsed.errors.is_none());
    }

    #[test]
    fn capture_response_handles_empty_errors() {
        let payload = r#"{"status":1,"errors":[]}"#;
        let parsed: CaptureResponse = serde_json::from_str(payload).unwrap();
        assert_eq!(parsed.status, Some(1));
        assert!(parsed.errors.unwrap().is_empty());
    }

    #[test]
    fn lamp_envelope_handles_missing_data() {
        let payload = r#"{"status":0,"msg":"success"}"#;
        let parsed: LampEnvelope<LampBrightnessData> = serde_json::from_str(payload).unwrap();
        assert_eq!(parsed.status, 0);
        assert_eq!(parsed.msg, "success");
        assert!(parsed.data.is_none());
    }

    #[test]
    fn lamp_envelope_handles_invalid_data() {
        let payload = r#"{"status":0,"msg":"success","data":{"invalid_field":10}}"#;
        let parsed: Result<LampEnvelope<LampBrightnessData>, _> = serde_json::from_str(payload);
        assert!(parsed.is_err());
    }

    #[test]
    fn camera_api_client_handles_invalid_url() {
        let client = CameraApiClient::new("invalid_url".to_string());
        let result = client.build_url(&["v1", "capture"]);
        assert!(result.is_err());
    }

    #[test]
    fn camera_api_client_handles_empty_base_url() {
        let client = CameraApiClient::new(String::new());
        let result = client.build_url(&["v1", "capture"]);
        assert!(result.is_err());
    }

    #[test]
    fn camera_api_client_handles_empty_segments() {
        let client = CameraApiClient::new("http://example.com".to_string());
        let result = client.build_url(&[]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().as_str(), "http://example.com/");
    }

    #[test]
    fn media_info_handles_missing_fields() {
        let payload = r#"{
            "name": "test.mp4",
            "size": 12345,
            "canplayback": true,
            "origin": {
                "width": 1920,
                "height": 1080,
                "duration": 60,
                "fps": 30,
                "br": 10000,
                "multi": 0,
                "withOsd": false,
                "id": "test.mp4",
                "stat": 0
            }
        }"#;
        let parsed: MediaInfo = serde_json::from_str(payload).unwrap();
        assert_eq!(parsed.name, "test.mp4");
        assert_eq!(parsed.size, 12345);
        assert!(parsed.canplayback);
        assert!(parsed.play.is_none());
        assert!(parsed.osd.is_none());
    }

    #[test]
    fn media_origin_handles_invalid_stat() {
        let origin = MediaOrigin {
            width: 1920,
            height: 1080,
            duration: 60,
            fps: 30,
            br: 10000,
            multi: 0,
            with_osd: false,
            id: "test.mp4".to_string(),
            stat: 999,
        };
        assert_eq!(origin.file_stat(), MediaFileStat::Other(999));
    }

    #[test]
    fn media_which_handles_query_value() {
        assert_eq!(MediaWhich::Original.as_query_value(), None);
        assert_eq!(MediaWhich::Play.as_query_value(), Some("play"));
    }

    #[test]
    fn media_info_for_handles_query_value() {
        assert_eq!(MediaInfoFor::Default.as_query_value(), None);
        assert_eq!(MediaInfoFor::Repair.as_query_value(), Some("repair"));
    }

    #[test]
    fn downloaded_media_handles_empty_content_type() {
        let media = DownloadedMedia {
            content_type: None,
            bytes: vec![1, 2, 3],
        };
        assert!(media.content_type.is_none());
        assert_eq!(media.bytes, vec![1, 2, 3]);
    }

    #[test]
    fn format_legacy_error_handles_empty_body() {
        let msg = format_legacy_error("test operation", StatusCode::NOT_FOUND, "");
        assert!(msg.contains("test operation failed with HTTP 404"));
    }

    #[test]
    fn format_legacy_error_handles_invalid_json() {
        let body = "invalid json";
        let msg = format_legacy_error("test operation", StatusCode::BAD_REQUEST, body);
        assert!(msg.contains("test operation failed (HTTP 400 Bad Request): invalid json"));
    }

    #[test]
    fn format_legacy_error_falls_back_for_empty_envelope() {
        // Valid JSON object with neither `code` nor `error` set.
        let msg = format_legacy_error("op", StatusCode::BAD_GATEWAY, "{}");
        assert!(msg.contains("op failed (HTTP 502"));
        assert!(msg.contains("{}"));
    }

    #[test]
    fn check_lamp_status_accepts_zero_and_reports_empty_msg() {
        let ok = LampEnvelope::<serde_json::Value> {
            status: 0,
            msg: String::new(),
            data: None,
        };
        assert!(check_lamp_status("op", &ok).is_ok());
        let bad = LampEnvelope::<serde_json::Value> {
            status: 9,
            msg: String::new(),
            data: None,
        };
        let err = check_lamp_status("op", &bad).unwrap_err();
        assert!(format!("{err}").contains("<no msg>"));
    }

    // ---- constructors -----------------------------------------------------

    #[test]
    fn new_trims_trailing_slash() {
        let client = CameraApiClient::new("http://192.168.1.88/".to_string());
        let url = client.build_url(&["v1", "capture"]).unwrap();
        assert_eq!(url.as_str(), "http://192.168.1.88/v1/capture");
    }

    #[test]
    fn with_http_builds_expected_url() {
        let client = CameraApiClient::with_http("http://cam.test".to_string(), Client::new());
        let url = client.build_url(&["v1", "lamp"]).unwrap();
        assert_eq!(url.as_str(), "http://cam.test/v1/lamp");
    }

    #[test]
    fn new_bound_constructs_client() {
        let bound = CameraApiClient::new_bound("http://cam.test".to_string(), Some("lo0"));
        assert!(bound.build_url(&["v1", "capture"]).is_ok());
        let unbound = CameraApiClient::new_bound("http://cam.test".to_string(), None);
        assert!(unbound.build_url(&["v1", "capture"]).is_ok());
    }

    // ---- capture ----------------------------------------------------------

    #[test]
    fn capture_success_returns_response() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", "/v1/capture")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":0,"msg":"success","data":null}"#)
            .create();
        let client = CameraApiClient::new(server.url());
        let resp = client.capture(PhotoFormat::Jpeg, 1).expect("capture ok");
        mock.assert();
        assert_eq!(resp.status, Some(0));
    }

    #[test]
    fn capture_surfaces_sub_camera_errors() {
        let mut server = mockito::Server::new();
        let body = r#"{
            "code": 4325377,
            "status": 4325377,
            "msg": "capture parallel error",
            "errors": [
                {"code": 4390913, "msg": "timeout", "meta": {"ip": "192.168.1.104"}}
            ]
        }"#;
        let mock = server
            .mock("POST", "/v1/capture")
            .with_status(500)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create();
        let client = CameraApiClient::new(server.url());
        let err = client
            .capture(PhotoFormat::JpegDng, 2)
            .expect_err("should fail");
        mock.assert();
        let msg = format!("{err}");
        assert!(msg.contains("capture failed"));
        assert!(msg.contains("4390913"));
        assert!(msg.contains("192.168.1.104"));
    }

    #[test]
    fn capture_treats_unparsable_success_as_ok() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", "/v1/capture")
            .with_status(200)
            .with_body("not json")
            .create();
        let client = CameraApiClient::new(server.url());
        let resp = client.capture(PhotoFormat::Dng, 1).expect("defaults to ok");
        mock.assert();
        assert!(resp.status.is_none());
    }

    // ---- list_medias ------------------------------------------------------

    #[test]
    fn list_medias_returns_parsed_entries() {
        let mut server = mockito::Server::new();
        let body = r#"[
            {"name":"a.jpeg","size":100,"canplayback":false,
             "origin":{"width":0,"height":0,"duration":0,"fps":0,"br":0,"multi":0,"withOsd":false,"id":"a.jpeg","stat":0},
             "play":null,"osd":null}
        ]"#;
        let mock = server
            .mock("GET", "/v1/medias")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create();
        let client = CameraApiClient::new(server.url());
        let medias = client.list_medias(None).expect("list ok");
        mock.assert();
        assert_eq!(medias.len(), 1);
        assert_eq!(medias[0].name, "a.jpeg");
    }

    #[test]
    fn list_medias_with_scene_sends_query() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v1/medias")
            .match_query(mockito::Matcher::UrlEncoded("scene".into(), "1".into()))
            .with_status(200)
            .with_body("[]")
            .create();
        let client = CameraApiClient::new(server.url());
        let medias = client
            .list_medias(Some(MediaScene::VesselInspection))
            .expect("list ok");
        mock.assert();
        assert!(medias.is_empty());
    }

    #[test]
    fn list_medias_maps_http_error() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v1/medias")
            .with_status(500)
            .with_body(r#"{"code":1,"error":"boom"}"#)
            .create();
        let client = CameraApiClient::new(server.url());
        let err = client.list_medias(None).expect_err("should fail");
        mock.assert();
        assert!(format!("{err}").contains("list medias"));
    }

    // ---- download_media ---------------------------------------------------

    #[test]
    fn download_media_rejects_empty_name() {
        let client = CameraApiClient::new("http://cam.test".to_string());
        assert!(client.download_media("", MediaWhich::Original).is_err());
    }

    #[test]
    fn download_media_returns_bytes_and_content_type() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v1/medias/a.jpeg/download")
            .with_status(200)
            .with_header("content-type", "image/jpeg")
            .with_body(b"jpeg-bytes")
            .create();
        let client = CameraApiClient::new(server.url());
        let media = client
            .download_media("a.jpeg", MediaWhich::Original)
            .expect("download ok");
        mock.assert();
        assert_eq!(media.bytes, b"jpeg-bytes".to_vec());
        assert_eq!(media.content_type.as_deref(), Some("image/jpeg"));
    }

    #[test]
    fn download_media_play_variant_sends_query() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v1/medias/clip.mp4/download")
            .match_query(mockito::Matcher::UrlEncoded("which".into(), "play".into()))
            .with_status(200)
            .with_body(b"mp4")
            .create();
        let client = CameraApiClient::new(server.url());
        let media = client
            .download_media("clip.mp4", MediaWhich::Play)
            .expect("download ok");
        mock.assert();
        assert_eq!(media.bytes, b"mp4".to_vec());
    }

    #[test]
    fn download_media_maps_http_error() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v1/medias/missing.jpeg/download")
            .with_status(404)
            .with_body("")
            .create();
        let client = CameraApiClient::new(server.url());
        let err = client
            .download_media("missing.jpeg", MediaWhich::Original)
            .expect_err("should fail");
        mock.assert();
        assert!(format!("{err}").contains("download media"));
    }

    // ---- delete_media -----------------------------------------------------

    #[test]
    fn delete_media_rejects_empty_name() {
        let client = CameraApiClient::new("http://cam.test".to_string());
        assert!(client.delete_media("").is_err());
    }

    #[test]
    fn delete_media_success() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("DELETE", "/v1/medias/a.jpeg")
            .with_status(200)
            .create();
        let client = CameraApiClient::new(server.url());
        client.delete_media("a.jpeg").expect("delete ok");
        mock.assert();
    }

    #[test]
    fn delete_media_maps_http_error() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("DELETE", "/v1/medias/a.jpeg")
            .with_status(500)
            .with_body("nope")
            .create();
        let client = CameraApiClient::new(server.url());
        let err = client.delete_media("a.jpeg").expect_err("should fail");
        mock.assert();
        assert!(format!("{err}").contains("delete media"));
    }

    // ---- media_info -------------------------------------------------------

    #[test]
    fn media_info_rejects_empty_name() {
        let client = CameraApiClient::new("http://cam.test".to_string());
        assert!(
            client
                .media_info("", MediaInfoFor::Default, MediaWhich::Original)
                .is_err()
        );
    }

    #[test]
    fn media_info_success() {
        let mut server = mockito::Server::new();
        let body = r#"{
            "name":"clip.mp4","size":10,"width":1920,"height":1080,
            "duration":17,"fps":30,"br":20480,"multi":0,"withOsd":false,"id":"clip.mp4"
        }"#;
        let mock = server
            .mock("GET", "/v1/medias/clip.mp4/info")
            // media_info always opens a query string, so the request path ends
            // in a bare `?`; accept any (including empty) query here.
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create();
        let client = CameraApiClient::new(server.url());
        let info = client
            .media_info("clip.mp4", MediaInfoFor::Default, MediaWhich::Original)
            .expect("info ok");
        mock.assert();
        assert_eq!(info.name, "clip.mp4");
        assert_eq!(info.width, 1920);
    }

    #[test]
    fn media_info_repair_sends_queries() {
        let mut server = mockito::Server::new();
        let body = r#"{
            "name":"clip.mp4","size":10,"width":1,"height":1,
            "duration":1,"fps":1,"br":1,"multi":0,"id":"clip.mp4"
        }"#;
        let mock = server
            .mock("GET", "/v1/medias/clip.mp4/info")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("for".into(), "repair".into()),
                mockito::Matcher::UrlEncoded("which".into(), "play".into()),
            ]))
            .with_status(200)
            .with_body(body)
            .create();
        let client = CameraApiClient::new(server.url());
        let info = client
            .media_info("clip.mp4", MediaInfoFor::Repair, MediaWhich::Play)
            .expect("info ok");
        mock.assert();
        assert_eq!(info.name, "clip.mp4");
    }

    #[test]
    fn media_info_maps_http_error() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v1/medias/x/info")
            .match_query(mockito::Matcher::Any)
            .with_status(404)
            .with_body(r#"{"code":1101,"error":"file not found"}"#)
            .create();
        let client = CameraApiClient::new(server.url());
        let err = client
            .media_info("x", MediaInfoFor::Default, MediaWhich::Original)
            .expect_err("should fail");
        mock.assert();
        assert!(format!("{err}").contains("media info"));
    }

    // ---- lamp -------------------------------------------------------------

    #[test]
    fn get_led_brightness_success() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v1/lamp")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":0,"msg":"success","data":{"brightness":42}}"#)
            .create();
        let client = CameraApiClient::new(server.url());
        let brightness = client.get_led_brightness().expect("ok");
        mock.assert();
        assert_eq!(brightness, 42);
    }

    #[test]
    fn get_led_brightness_status_nonzero_errors() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v1/lamp")
            .with_status(200)
            .with_body(r#"{"status":7,"msg":"lamp busy"}"#)
            .create();
        let client = CameraApiClient::new(server.url());
        let err = client.get_led_brightness().expect_err("should fail");
        mock.assert();
        assert!(format!("{err}").contains("lamp busy"));
    }

    #[test]
    fn get_led_brightness_missing_data_errors() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v1/lamp")
            .with_status(200)
            .with_body(r#"{"status":0,"msg":"success"}"#)
            .create();
        let client = CameraApiClient::new(server.url());
        let err = client.get_led_brightness().expect_err("should fail");
        mock.assert();
        assert!(format!("{err}").contains("missing data"));
    }

    #[test]
    fn get_led_brightness_http_error() {
        let mut server = mockito::Server::new();
        let mock = server.mock("GET", "/v1/lamp").with_status(500).create();
        let client = CameraApiClient::new(server.url());
        assert!(client.get_led_brightness().is_err());
        mock.assert();
    }

    #[test]
    fn set_led_brightness_success() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", "/v1/lamp")
            .with_status(200)
            .with_body(r#"{"status":0,"msg":"success"}"#)
            .create();
        let client = CameraApiClient::new(server.url());
        client.set_led_brightness(70).expect("ok");
        mock.assert();
    }

    #[test]
    fn set_led_brightness_status_nonzero_errors() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", "/v1/lamp")
            .with_status(200)
            .with_body(r#"{"status":3,"msg":"rejected"}"#)
            .create();
        let client = CameraApiClient::new(server.url());
        let err = client.set_led_brightness(10).expect_err("should fail");
        mock.assert();
        assert!(format!("{err}").contains("rejected"));
    }

    #[test]
    fn set_led_brightness_http_error() {
        let mut server = mockito::Server::new();
        let mock = server.mock("POST", "/v1/lamp").with_status(500).create();
        let client = CameraApiClient::new(server.url());
        assert!(client.set_led_brightness(10).is_err());
        mock.assert();
    }
}
