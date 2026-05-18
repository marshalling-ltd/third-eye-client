use std::fmt::Write;
use anyhow::{Context, Result};
use reqwest::Url;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

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
    /// On macOS this uses `IP_BOUND_IF`; on Linux `SO_BINDTODEVICE`.
    /// On Windows, interface binding is not supported by reqwest and the
    /// parameter is ignored (OS routing is used).
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
        "<no msg>".to_owned()
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(parsed.status, Some(4325377));
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
        let client = CameraApiClient::new("http://192.168.1.88".to_owned());
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
            msg: "boom".to_owned(),
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
}
