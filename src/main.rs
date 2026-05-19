#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::module_name_repetitions,
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    clippy::struct_excessive_bools,
    clippy::struct_field_names,
    clippy::items_after_statements,
    clippy::unreadable_literal
)]

mod map;

use std::cell::RefCell;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};
use map::{
    DEFAULT_OSM_TILE_USER_AGENT, DEFAULT_ZOOM, MAX_ZOOM, MIN_ZOOM, MapState, MapTilesState,
    RgbaFrame, ViewportAnimation, compute_scale_bar, ease_out_cubic, lat_lon_to_world_px,
    rgba_frame_to_slint_image,
};
#[cfg(target_os = "macos")]
use map::{
    check_corelocation_warmup_fix, corelocation_debug_status, prime_corelocation_at_startup,
};
use reqwest::Url;
use slint::{ComponentHandle, ModelRc, VecModel};
use third_eye_client::camera::{CameraApiClient, MediaInfo, MediaScene, PhotoFormat};
use third_eye_client::nmea::{DEFAULT_NMEA_GPS_PORT, NmeaGpsState};
use third_eye_client::rov_status::{ROV_STATUS_UDP_PORT, Status as RovUdpStatus, UdpStatusState};
use third_eye_client::storage::AppStore;
use third_eye_client::storage::config::{ClientConfig, ClientConfigDefaults};
use third_eye_client::storage::media::{
    CaptureMetadata as StoredCaptureMetadata, LocalMediaRecord, MediaStore, download_to_local,
};

const DEFAULT_TEST_RTSP: &str = "rtsp://admin:admin@127.0.0.1:8554/stream";
const DEFAULT_ROV_RTSP: &str = "rtsp://admin:admin@192.168.1.88:8554/stream/0/0";
const DEFAULT_ROV_HTTP_BASE: &str = "http://192.168.1.88";
const DEFAULT_SERVER_BASE_URL: &str = "https://third-eye.marshalling.eu";
const DEFAULT_ROV_UDP_BIND_HOST: &str = "0.0.0.0";

slint::include_modules!();

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Configuration,
    Map,
    Stream,
    Media,
    Nmea,
}

impl Screen {
    const fn index(self) -> i32 {
        match self {
            Self::Configuration => 0,
            Self::Map => 1,
            Self::Stream => 2,
            Self::Media => 3,
            Self::Nmea => 4,
        }
    }
}

#[derive(Clone)]
struct AppConfig {
    rtsp_url: String,
    rov_http_base: String,
    rov_status_udp_bind_host: String,
    rov_status_udp_port: String,
    osm_tile_user_agent: String,
    server_base_url: String,
    rov_network_interface: String,
    nmea_gps_port: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            rtsp_url: DEFAULT_TEST_RTSP.to_owned(),
            rov_http_base: DEFAULT_ROV_HTTP_BASE.to_owned(),
            rov_status_udp_bind_host: default_rov_udp_bind_host(),
            rov_status_udp_port: ROV_STATUS_UDP_PORT.to_string(),
            osm_tile_user_agent: DEFAULT_OSM_TILE_USER_AGENT.to_owned(),
            server_base_url: DEFAULT_SERVER_BASE_URL.to_owned(),
            rov_network_interface: String::new(),
            nmea_gps_port: DEFAULT_NMEA_GPS_PORT.to_string(),
        }
    }
}

impl AppConfig {
    fn parse_rov_status_udp_port(&self) -> Result<u16> {
        let port_text = self.rov_status_udp_port.trim();
        let port = port_text
            .parse::<u16>()
            .context("ROV telemetry UDP port must be a number between 1 and 65535")?;
        if port == 0 {
            anyhow::bail!("ROV telemetry UDP port must be between 1 and 65535");
        }
        Ok(port)
    }

    fn to_client_config(&self) -> ClientConfig {
        ClientConfig {
            rtsp_url: self.rtsp_url.clone(),
            rov_http_base: self.rov_http_base.clone(),
            rov_udp_bind_host: self.rov_status_udp_bind_host.clone(),
            rov_udp_port: self.rov_status_udp_port.clone(),
            osm_tile_user_agent: self.osm_tile_user_agent.clone(),
            server_base_url: self.server_base_url.clone(),
            rov_network_interface: self.rov_network_interface.clone(),
            nmea_gps_port: self.nmea_gps_port.clone(),
        }
    }

    fn from_client_config(config: ClientConfig) -> Self {
        Self {
            rtsp_url: config.rtsp_url,
            rov_http_base: config.rov_http_base,
            rov_status_udp_bind_host: config.rov_udp_bind_host,
            rov_status_udp_port: config.rov_udp_port,
            osm_tile_user_agent: config.osm_tile_user_agent,
            server_base_url: config.server_base_url,
            rov_network_interface: config.rov_network_interface,
            nmea_gps_port: config.nmea_gps_port,
        }
    }

    fn parse_nmea_gps_port(&self) -> Result<u16> {
        let port_text = self.nmea_gps_port.trim();
        let port = port_text
            .parse::<u16>()
            .context("NMEA GPS port must be a number between 1 and 65535")?;
        if port == 0 {
            anyhow::bail!("NMEA GPS port must be between 1 and 65535");
        }
        Ok(port)
    }

    /// Returns the configured interface name if non-empty, or `None` to let
    /// the OS decide routing.
    fn rov_interface(&self) -> Option<&str> {
        let trimmed = self.rov_network_interface.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }
}

fn client_config_defaults() -> (String, ClientConfigDefaults<'static>) {
    let udp_bind_host = default_rov_udp_bind_host();
    // Leak the default bind host so we can hand out a `&'static str` into
    // `ClientConfigDefaults`. This is called once at startup.
    let udp_bind_static: &'static str = Box::leak(udp_bind_host.into_boxed_str());
    let defaults = ClientConfigDefaults {
        rtsp_url: DEFAULT_TEST_RTSP,
        rov_http_base: DEFAULT_ROV_HTTP_BASE,
        rov_udp_bind_host: udp_bind_static,
        rov_udp_port: UDP_PORT_DEFAULT_STR,
        osm_tile_user_agent: DEFAULT_OSM_TILE_USER_AGENT,
        server_base_url: DEFAULT_SERVER_BASE_URL,
        rov_network_interface: "",
        nmea_gps_port: NMEA_GPS_PORT_DEFAULT_STR,
    };
    (udp_bind_static.to_owned(), defaults)
}

// String form of `ROV_STATUS_UDP_PORT` known at compile time for use with
// `ClientConfigDefaults` (which stores `&'static str`).
const UDP_PORT_DEFAULT_STR: &str = "8500";
const _: () = {
    // Compile-time check that the string matches the real constant. If the
    // constant ever changes, this will prevent a silent drift.
    assert!(ROV_STATUS_UDP_PORT == 8500);
};

const NMEA_GPS_PORT_DEFAULT_STR: &str = "11123";
const _: () = {
    assert!(DEFAULT_NMEA_GPS_PORT == 11123);
};

fn parse_host_from_http_base(base: &str) -> Option<String> {
    let normalized = if base.contains("://") {
        base.trim().to_owned()
    } else {
        format!("http://{}", base.trim())
    };
    Url::parse(&normalized)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
}

fn default_rov_udp_bind_host() -> String {
    DEFAULT_ROV_UDP_BIND_HOST.to_owned()
}

#[derive(Default)]
struct AuthUiState {
    email: String,
    password: String,
    status_text: String,
    signed_in_as: String,
    is_signed_in: bool,
}

/// View-model backing the Media screen. Lives in `ThirdEyeState`.
struct MediaUiState {
    rows: Vec<LocalMediaRecord>,
    status_text: String,
    /// `(media_id, name)` of the currently-selected row, if any.
    selected: Option<(String, String)>,
    /// Pre-rendered detail strings for the right-hand panel.
    details_text: String,
    capture_text: String,
    has_capture_meta: bool,
    local_path: String,
    /// True while a background download is in flight.
    download_in_progress: bool,
    /// True while a ROV refresh HTTP request is in flight.
    refresh_in_progress: bool,
    /// True while a capture + metadata-attach is in flight.
    capture_in_progress: bool,
    /// Sender half of the persistent media-event channel.  Cloned into
    /// background threads so they can post results back to the UI loop.
    event_tx: mpsc::Sender<MediaEvent>,
    /// Receiver polled every frame by the timer callback.
    event_rx: mpsc::Receiver<MediaEvent>,
    /// Loaded preview image for the selected media (images only).
    preview_image: Option<slint::Image>,
    /// Cache of thumbnail images keyed by media name.
    thumbnail_cache: std::collections::HashMap<String, slint::Image>,
    /// Active media playback stream (ffmpeg decoding an MP4 from the ROV).
    media_stream_controller: Option<StreamController>,
    media_stream_event_rx: Option<Receiver<StreamEvent>>,
    media_stream_active: bool,
    media_stream_frames: u64,
    /// Structured capture-overlay short strings.
    capture_depth: String,
    capture_temp: String,
    capture_heading: String,
    capture_attitude: String,
    capture_coords: String,
    capture_battery: String,
    /// Compact subtitle: "793 KB \u{2022} image/jpeg \u{2022} 1920\u{00d7}1080"
    info_subtitle: String,
}

impl MediaUiState {
    fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        Self {
            rows: Vec::new(),
            status_text: String::new(),
            selected: None,
            details_text: String::new(),
            capture_text: String::new(),
            has_capture_meta: false,
            local_path: String::new(),
            download_in_progress: false,
            refresh_in_progress: false,
            capture_in_progress: false,
            event_tx,
            event_rx,
            preview_image: None,
            thumbnail_cache: std::collections::HashMap::new(),
            media_stream_controller: None,
            media_stream_event_rx: None,
            media_stream_active: false,
            media_stream_frames: 0,
            capture_depth: String::new(),
            capture_temp: String::new(),
            capture_heading: String::new(),
            capture_attitude: String::new(),
            capture_coords: String::new(),
            capture_battery: String::new(),
            info_subtitle: String::new(),
        }
    }

    fn poll_media_stream(&mut self) -> Option<RgbaFrame> {
        let mut disconnected = false;
        let mut latest_frame = None;
        if let Some(rx) = &self.media_stream_event_rx {
            loop {
                match rx.try_recv() {
                    Ok(StreamEvent::Frame(frame)) => {
                        latest_frame = Some(frame);
                        self.media_stream_frames = self.media_stream_frames.saturating_add(1);
                    }
                    Ok(StreamEvent::Status(_) | StreamEvent::Error(_)) => {}
                    Ok(StreamEvent::Ended) => {
                        disconnected = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if disconnected {
            if let Some(mut controller) = self.media_stream_controller.take() {
                controller.stop();
            }
            self.media_stream_event_rx = None;
            self.media_stream_active = false;
        }
        latest_frame
    }

    fn stop_media_stream(&mut self) {
        if let Some(mut controller) = self.media_stream_controller.take() {
            controller.stop();
        }
        self.media_stream_event_rx = None;
        self.media_stream_active = false;
        self.media_stream_frames = 0;
    }
}

/// Messages sent from background worker threads back to the UI loop.
enum MediaEvent {
    Download {
        name: String,
        result: Result<std::path::PathBuf, String>,
    },
    Refresh {
        status_text: String,
    },
    Capture {
        capture_msg: String,
        attached_text: String,
    },
    Delete {
        status_text: String,
    },
    ListMedias {
        rov_info: String,
    },
}

struct ThirdEyeState {
    active_screen: Screen,
    last_screen: Screen,
    suppress_next_map_flick: bool,
    config: AppConfig,
    map: MapState,
    map_tiles: MapTilesState,
    rov_info: String,
    stream: StreamState,
    rov_status: UdpStatusState,
    nmea_gps: NmeaGpsState,
    viewport_anim: Option<ViewportAnimation>,
    auth: AuthUiState,
    attached_metadata_text: String,
    media: MediaUiState,
    /// Unix-ms timestamp of the last successful location fix.
    location_detected_at_ms: i64,
    /// Unix-ms timestamp when the user left the stream screen.
    /// `0` means we are on the stream screen (or never were).
    stream_left_at_ms: i64,
    /// Background startup location warmup (Windows only). A background thread
    /// calls the blocking GPS API and sends the result here; the timer loop
    /// picks it up and applies it without blocking the UI.
    #[cfg(target_os = "windows")]
    startup_location_rx: Option<mpsc::Receiver<Result<(f64, f64), String>>>,
}

impl ThirdEyeState {
    fn new(store: &AppStore) -> Self {
        let (_bind_owned, defaults) = client_config_defaults();
        let client_config = store.config().load_client(&defaults).unwrap_or_else(|err| {
            eprintln!("failed to load persisted config, falling back to defaults: {err:#}");
            ClientConfig {
                rtsp_url: defaults.rtsp_url.to_owned(),
                rov_http_base: defaults.rov_http_base.to_owned(),
                rov_udp_bind_host: defaults.rov_udp_bind_host.to_owned(),
                rov_udp_port: defaults.rov_udp_port.to_owned(),
                osm_tile_user_agent: defaults.osm_tile_user_agent.to_owned(),
                server_base_url: defaults.server_base_url.to_owned(),
                rov_network_interface: defaults.rov_network_interface.to_owned(),
                nmea_gps_port: defaults.nmea_gps_port.to_owned(),
            }
        });

        let mut auth = AuthUiState::default();
        match store.auth().current_session() {
            Ok(Some(session)) => {
                auth.is_signed_in = true;
                auth.signed_in_as = session.email.unwrap_or_default();
                auth.email = auth.signed_in_as.clone();
                auth.status_text = "Signed in. Session restored from storage.".to_owned();
            }
            Ok(None) => {
                auth.status_text = "Not signed in. Enter credentials to authenticate.".to_owned();
            }
            Err(err) => {
                auth.status_text = format!("Failed to read auth session: {err:#}");
            }
        }

        let mut media = MediaUiState::new();
        // Hydrate the Media screen with whatever we already know about ROV
        // media (previous sessions may have populated the table already).
        match store.media().list_all() {
            Ok(rows) => {
                media.rows = rows;
                if media.rows.is_empty() {
                    media.status_text =
                        "No media recorded yet. Click \"Refresh from ROV\" to populate.".to_owned();
                } else {
                    media.status_text =
                        format!("{} media record(s) in local library.", media.rows.len());
                }
            }
            Err(err) => {
                media.status_text = format!("Failed to load local media registry: {err:#}");
            }
        }

        Self {
            active_screen: Screen::Configuration,
            last_screen: Screen::Configuration,
            suppress_next_map_flick: false,
            config: AppConfig::from_client_config(client_config),
            map: MapState {
                zoom: DEFAULT_ZOOM,
                ..MapState::default()
            },
            map_tiles: MapTilesState::new(),
            rov_info: String::new(),
            stream: StreamState::default(),
            rov_status: UdpStatusState::default(),
            nmea_gps: NmeaGpsState::default(),
            viewport_anim: None,
            auth,
            attached_metadata_text: String::new(),
            media,
            location_detected_at_ms: 0,
            stream_left_at_ms: 0,
            #[cfg(target_os = "windows")]
            startup_location_rx: None,
        }
    }

    fn load_map_tile_for_current_location(&mut self, success_status: String) {
        match (self.map.lat, self.map.lon) {
            (Some(lat), Some(lon)) => {
                self.map_tiles.center_on_location(lat, lon, self.map.zoom);
                self.request_visible_map_tiles();
                self.map.status = success_status;
            }
            _ => {
                self.map.status = "No location set. Use Detect location first.".to_owned();
            }
        }
    }

    fn auto_refresh_map_on_tab_enter(&mut self) {
        // Always show the map immediately without blocking. If we have a
        // recent location, center on it. If not, leave the map at its
        // current position and show a hint. The user can press
        // "Detect Location" to get a fresh fix — that call is explicit
        // and the user expects it to take a moment.
        if self.map.lat.is_some() && self.map.lon.is_some() {
            self.load_map_tile_for_current_location("Centered on last known location.".to_owned());
        } else {
            // No location yet — load tiles at the current viewport
            // so at least the map renders, and prompt the user.
            self.request_visible_map_tiles();
            self.map.status =
                "No location set. Use Detect location button to find your position.".to_owned();
        }
    }

    fn request_visible_map_tiles(&mut self) {
        self.map_tiles
            .request_visible_tiles(self.map.zoom, &self.config.osm_tile_user_agent);
    }

    fn set_map_visible_size(&mut self, width: f64, height: f64) {
        let center_before_resize = self
            .map_tiles
            .center_lat_lon(self.map.zoom)
            .or(self.map.lat.zip(self.map.lon));
        if self
            .map_tiles
            .update_visible_size(width, height, self.map.zoom)
        {
            if let Some((lat, lon)) = center_before_resize {
                self.map_tiles.center_on_location(lat, lon, self.map.zoom);
            }
            self.request_visible_map_tiles();
        }
    }

    fn set_map_viewport(&mut self, viewport_x: f64, viewport_y: f64) {
        self.map_tiles
            .set_offset_from_viewport(viewport_x, viewport_y, self.map.zoom);
        self.request_visible_map_tiles();
    }

    fn set_map_zoom(&mut self, next_zoom: u32, focus_x: f64, focus_y: f64) {
        if next_zoom == self.map.zoom {
            return;
        }
        let bounded_zoom = next_zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let current_zoom = self.map.zoom;
        self.map_tiles
            .set_zoom_level(current_zoom, bounded_zoom, focus_x, focus_y);
        self.map.zoom = bounded_zoom;
        self.request_visible_map_tiles();
    }
}

#[derive(Default)]
struct StreamState {
    event_rx: Option<Receiver<StreamEvent>>,
    controller: Option<StreamController>,
    status: String,
    frames_received: u64,
}

impl StreamState {
    /// Start the RTSP stream for `rtsp_url`.
    ///
    /// `rov_http_base` and `rov_interface` are used on Windows to pre-populate
    /// the ARP cache before launching ffmpeg: without this, Windows may not
    /// have resolved the ROV's MAC address and ffmpeg's TCP CONNECT will fail.
    #[allow(unused_variables)]
    fn start(
        &mut self,
        rtsp_url: String,
        rov_http_base: Option<&str>,
        rov_interface: Option<&str>,
    ) -> Result<String> {
        let ffmpeg_bin = locate_ffmpeg_binary().context(
            "ffmpeg binary not found. Bundle it as ./bin/ffmpeg beside the app executable.",
        )?;
        let ffmpeg_label = ffmpeg_bin.display().to_string();

        // On Windows, make a quick HTTP request to the ROV before launching
        // ffmpeg. This forces Windows to resolve the ROV's MAC address and
        // populate the ARP cache so that ffmpeg's subsequent TCP connection
        // goes to the right adapter instead of getting "connection refused".
        #[cfg(target_os = "windows")]
        if let (Some(base), Some(iface)) = (rov_http_base, rov_interface) {
            let client = CameraApiClient::new_bound(base.to_owned(), Some(iface));
            let _ = client.list_medias(None::<MediaScene>);
        }

        let (controller, rx) = spawn_stream_pipeline(ffmpeg_bin, rtsp_url)?;
        self.event_rx = Some(rx);
        self.controller = Some(controller);
        self.frames_received = 0;
        Ok(format!(
            "Embedded stream started via ffmpeg at {ffmpeg_label}."
        ))
    }

    fn stop(&mut self) {
        if let Some(mut controller) = self.controller.take() {
            controller.stop();
            self.status = "Stream stopped.".to_owned();
        }
        self.event_rx = None;
    }

    fn poll_events(&mut self) -> Option<RgbaFrame> {
        let mut disconnected = false;
        let mut latest_frame = None;

        if let Some(rx) = &self.event_rx {
            loop {
                match rx.try_recv() {
                    Ok(StreamEvent::Frame(frame)) => {
                        latest_frame = Some(frame);
                        self.frames_received = self.frames_received.saturating_add(1);
                    }
                    Ok(StreamEvent::Status(text)) => {
                        self.status = text;
                    }
                    Ok(StreamEvent::Error(text)) => {
                        self.status = text;
                    }
                    Ok(StreamEvent::Ended) => {
                        if self.status.trim().is_empty()
                            || self.status == "Streaming started. Waiting for frames..."
                        {
                            self.status = "Stream ended.".to_owned();
                        }
                        disconnected = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }

        if disconnected {
            self.controller = None;
            self.event_rx = None;
        }

        latest_frame
    }
}

struct StreamController {
    stop_flag: Arc<AtomicBool>,
    ffmpeg_child: Child,
    workers: Vec<JoinHandle<()>>,
    /// Keeps the RTSP TCP proxy alive for the lifetime of the stream.
    _proxy_guard: Option<TcpProxyGuard>,
}

impl StreamController {
    fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        let _ = self.ffmpeg_child.kill();
        let _ = self.ffmpeg_child.wait();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

impl Drop for StreamController {
    fn drop(&mut self) {
        self.stop();
    }
}

enum StreamEvent {
    Frame(RgbaFrame),
    Status(String),
    Error(String),
    Ended,
}

fn apply_state_to_ui(ui: &AppWindow, state: &ThirdEyeState) {
    ui.set_active_screen(state.active_screen.index());

    ui.set_rtsp_url(state.config.rtsp_url.clone().into());
    ui.set_rov_http_base(state.config.rov_http_base.clone().into());
    ui.set_rov_status_udp_port(state.config.rov_status_udp_port.clone().into());
    ui.set_osm_tile_user_agent(state.config.osm_tile_user_agent.clone().into());
    ui.set_server_base_url(state.config.server_base_url.clone().into());
    ui.set_rov_info(state.rov_info.clone().into());
    ui.set_nmea_gps_port(state.config.nmea_gps_port.clone().into());
    ui.set_nmea_gps_status(state.nmea_gps.status_text().to_owned().into());
    ui.set_nmea_gps_running(state.nmea_gps.is_running());
    ui.set_nmea_has_fix(state.nmea_gps.fixes_received() > 0);
    // Only populate the IP field if the user hasn't typed anything yet.
    if ui.get_nmea_local_ip().is_empty() {
        ui.set_nmea_local_ip(detect_local_ip().unwrap_or_default().into());
    }
    ui.set_auth_email(state.auth.email.clone().into());
    ui.set_auth_password(state.auth.password.clone().into());
    ui.set_auth_status_text(state.auth.status_text.clone().into());
    ui.set_auth_signed_in_as(state.auth.signed_in_as.clone().into());
    ui.set_auth_is_signed_in(state.auth.is_signed_in);
    ui.set_attached_metadata_text(state.attached_metadata_text.clone().into());
    apply_map_runtime_to_ui(ui, state);
    apply_stream_and_rov_runtime_to_ui(ui, state);
    apply_media_runtime_to_ui(ui, state);
}

fn apply_map_runtime_to_ui(ui: &AppWindow, state: &ThirdEyeState) {
    ui.set_map_status(state.map.status.clone().into());
    ui.set_zoom_text(state.map.zoom.to_string().into());
    let lat_lon = match (state.map.lat, state.map.lon) {
        (Some(lat), Some(lon)) => format!("{lat:.6}, {lon:.6}"),
        _ => "n/a".to_owned(),
    };
    ui.set_lat_lon_text(lat_lon.into());
    let pin_short = match (state.map.lat, state.map.lon) {
        (Some(lat), Some(lon)) => format!("{lat:.4}, {lon:.4}"),
        _ => String::new(),
    };
    ui.set_pin_lat_lon_short(pin_short.into());
    match (state.map.lat, state.map.lon) {
        (Some(lat), Some(lon)) => {
            let (pin_x, pin_y) = lat_lon_to_world_px(lat, lon, state.map.zoom);
            ui.set_map_pin_world_x(pin_x);
            ui.set_map_pin_world_y(pin_y);
            ui.set_map_has_pin(true);
        }
        _ => {
            ui.set_map_has_pin(false);
        }
    }
    #[cfg(target_os = "macos")]
    ui.set_corelocation_debug(corelocation_debug_status(&state.map).into());
    #[cfg(not(target_os = "macos"))]
    ui.set_corelocation_debug("CoreLocation debug: not available on this platform.".into());
    let (target_vp_x, target_vp_y, viewport_width, viewport_height) =
        state.map_tiles.viewport_for_slint(state.map.zoom);
    let (display_vp_x, display_vp_y) = if let Some(anim) = &state.viewport_anim {
        let t = ease_out_cubic((anim.elapsed_ms / anim.duration_ms).clamp(0.0, 1.0)) as f32;
        (
            anim.start_vp_x + (anim.target_vp_x - anim.start_vp_x) * t,
            anim.start_vp_y + (anim.target_vp_y - anim.start_vp_y) * t,
        )
    } else {
        (target_vp_x, target_vp_y)
    };
    ui.invoke_set_map_viewport(display_vp_x, display_vp_y, viewport_width, viewport_height);
    let tiles = state.map_tiles.visible_tiles(state.map.zoom);
    let tile_model = VecModel::from(
        tiles
            .into_iter()
            .map(|t| MapTile {
                x: t.x,
                y: t.y,
                size: t.size,
                tile: t.image,
            })
            .collect::<Vec<_>>(),
    );
    ui.set_map_tiles(ModelRc::new(tile_model));
    let scale_lat = state.map.lat.unwrap_or(45.0);
    let (bar_px, bar_text) = compute_scale_bar(state.map.zoom, scale_lat);
    ui.set_scale_bar_width(bar_px);
    ui.set_scale_bar_text(bar_text.into());
    apply_stream_and_rov_runtime_to_ui(ui, state);
}

fn apply_stream_and_rov_runtime_to_ui(ui: &AppWindow, state: &ThirdEyeState) {
    ui.set_stream_status(state.stream.status.clone().into());
    ui.set_frames_received_text(state.stream.frames_received.to_string().into());

    ui.set_rov_status_text(state.rov_status.status_text().to_owned().into());
    ui.set_rov_packets_received_text(state.rov_status.packets_received().to_string().into());

    if let Some(status) = state.rov_status.latest_status() {
        ui.set_has_rov_status(true);
        ui.set_rov_attitude_text(
            format!(
                "Attitude [rad]: pitch={:.3}, roll={:.3}, yaw={:.3}",
                status.pitch, status.roll, status.yaw
            )
            .into(),
        );
        ui.set_rov_depth_temp_text(
            format!(
                "Depth: {:.2} m | Temperature: {:.1} °C",
                status.depth, status.temperature
            )
            .into(),
        );
        ui.set_rov_coordinates_text(
            format!(
                "Coordinates: lat_degE7={}, lon_degE7={}",
                status.lat, status.lon
            )
            .into(),
        );
        ui.set_rov_imu_text(
            format!(
                "IMU gyro [0.1°/s]: x={}, y={}, z={}",
                status.imu.gyro_x, status.imu.gyro_y, status.imu.gyro_z
            )
            .into(),
        );
        let batteries_text = if status.batteries.is_empty() {
            "Batteries: no battery data in payload.".to_owned()
        } else {
            let mut lines = vec!["Batteries:".to_owned()];
            for battery in &status.batteries {
                lines.push(format!(
                    "ID {}: {} mV, {} (10mA), {}%",
                    battery.id, battery.voltage, battery.current, battery.remaining
                ));
            }
            lines.join("\n")
        };
        ui.set_rov_batteries_text(batteries_text.into());

        // Compact overlay values for the full-bleed stream screen.
        ui.set_rov_depth_short(format!("{:.1} m", status.depth).into());
        ui.set_rov_temp_short(format!("{:.1} \u{00b0}C", status.temperature).into());
        let heading_deg = status.yaw.to_degrees().rem_euclid(360.0);
        ui.set_rov_heading_short(format!("{heading_deg:.0}\u{00b0}").into());
        ui.set_rov_attitude_short(
            format!(
                "P {:.1}\u{00b0}  R {:.1}\u{00b0}",
                status.pitch.to_degrees(),
                status.roll.to_degrees()
            )
            .into(),
        );
        // POS: use device CoreLocation, not ROV UDP (which sends 0,0).
        let location_age_ms = current_unix_ms() - state.location_detected_at_ms;
        let pos_text = if let (Some(lat), Some(lon)) = (state.map.lat, state.map.lon) {
            if state.location_detected_at_ms > 0 && location_age_ms < 600_000 {
                format!("{lat:.4}, {lon:.4}")
            } else {
                "stale".to_owned()
            }
        } else {
            "\u{2014}".to_owned()
        };
        ui.set_rov_coords_short(pos_text.into());
        let battery_short = if status.batteries.is_empty() {
            "\u{2014}".to_owned()
        } else {
            status
                .batteries
                .iter()
                .map(|b| format!("{}%", b.remaining))
                .collect::<Vec<_>>()
                .join(" / ")
        };
        ui.set_rov_battery_short(battery_short.into());
    } else {
        ui.set_has_rov_status(false);
        ui.set_rov_attitude_text("".into());
        ui.set_rov_depth_temp_text("".into());
        ui.set_rov_coordinates_text("".into());
        ui.set_rov_imu_text("".into());
        ui.set_rov_batteries_text("".into());
        ui.set_rov_depth_short("".into());
        ui.set_rov_temp_short("".into());
        ui.set_rov_heading_short("".into());
        ui.set_rov_attitude_short("".into());
        ui.set_rov_coords_short("".into());
        ui.set_rov_battery_short("".into());
    }
}

fn pull_configuration_from_ui(ui: &AppWindow, state: &mut ThirdEyeState, store: &AppStore) {
    state.config.rtsp_url = ui.get_rtsp_url().to_string();
    state.config.rov_http_base = ui.get_rov_http_base().to_string();
    state.config.rov_status_udp_port = ui.get_rov_status_udp_port().to_string();
    state.config.osm_tile_user_agent = ui.get_osm_tile_user_agent().to_string();
    state.config.server_base_url = ui.get_server_base_url().to_string();
    state.config.nmea_gps_port = ui.get_nmea_gps_port().to_string();
    state.auth.email = ui.get_auth_email().to_string();
    state.auth.password = ui.get_auth_password().to_string();
    if let Err(err) = store.config().save_client(&state.config.to_client_config()) {
        eprintln!("failed to persist configuration: {err:#}");
    }
}

fn persist_config(state: &ThirdEyeState, store: &AppStore) {
    if let Err(err) = store.config().save_client(&state.config.to_client_config()) {
        eprintln!("failed to persist configuration: {err:#}");
    }
}

/// Finds the network interface that is on the same subnet as `rov_host`.
///
/// Uses `if-addrs` for cross-platform interface enumeration. On macOS the
/// WiFi adapter (`en0`) is excluded so that wired USB-ethernet adapters are
/// preferred; on other platforms the first matching non-loopback interface
/// is returned.
/// Returns the local IPv4 address that the OS would use to reach the
/// internet (i.e. the adapter with the default gateway). Works cross-
/// platform by connecting a UDP socket to a public IP — no data is sent,
/// the OS just resolves which local address it would route through.
fn detect_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    // Connect to a well-known public IP. No packet is actually sent.
    socket.connect("8.8.8.8:80").ok()?;
    let local_addr = socket.local_addr().ok()?;
    Some(local_addr.ip().to_string())
}

fn detect_rov_interface(rov_host: &str) -> Option<String> {
    let rov_ip = rov_host.parse::<std::net::Ipv4Addr>().ok()?;
    let interfaces = if_addrs::get_if_addrs().ok()?;

    let candidates: Vec<String> = interfaces
        .into_iter()
        .filter(|iface| !iface.is_loopback())
        .filter_map(|iface| {
            if let if_addrs::IfAddr::V4(v4) = iface.addr
                && v4.ip != rov_ip
            {
                let mask = u32::from(v4.netmask);
                if (u32::from(v4.ip) & mask) == (u32::from(rov_ip) & mask) {
                    return Some(iface.name);
                }
            }
            None
        })
        .collect();

    // On macOS prefer any interface over en0 (en0 is typically WiFi;
    // wired USB-ethernet adapters appear as en5, en6, etc.).
    #[cfg(target_os = "macos")]
    {
        candidates
            .iter()
            .find(|name| name.as_str() != "en0")
            .cloned()
    }

    #[cfg(not(target_os = "macos"))]
    candidates.into_iter().next()
}

fn refresh_rov_network(state: &mut ThirdEyeState, setup_external_route: bool) {
    state.config.rov_status_udp_bind_host = default_rov_udp_bind_host();
    let Some(rov_host) = parse_host_from_http_base(&state.config.rov_http_base) else {
        state.config.rov_network_interface.clear();
        state.rov_info = "Could not extract host from ROV HTTP API URL.".to_owned();
        return;
    };

    if let Some(interface) = detect_rov_interface(&rov_host) {
        state.config.rov_network_interface = interface.clone();
        let mut summary = format!("Detected wired ROV interface {interface} for {rov_host}.");
        if setup_external_route {
            match ensure_rov_external_route(&state.config.rov_http_base, &interface) {
                Ok(()) => {
                    summary.push_str(" External stream route is ready.");
                }
                Err(err) => {
                    summary.push_str(&format!(" External stream route is not ready yet: {err:#}"));
                }
            }
        }
        state.rov_info = summary;
    } else {
        state.config.rov_network_interface.clear();
        // Remove any stale host route from a previous cable session so
        // ffmpeg falls back to the default OS routing (e.g. ROV WiFi).
        cleanup_stale_rov_route(&rov_host);
        state.rov_info =
            format!("No dedicated wired ROV interface detected for {rov_host}. Using OS routing.");
    }
}

// -------------------------------------------------------------------------
// Media screen helpers
// -------------------------------------------------------------------------

fn format_bytes(bytes: i64) -> String {
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

fn format_relative_age(ts_ms: i64) -> String {
    let now = current_unix_ms();
    let diff_secs = ((now - ts_ms).max(0) / 1000) as u64;
    if diff_secs < 10 {
        "just now".to_owned()
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

fn origin_label(record: &LocalMediaRecord) -> &'static str {
    match record.mime.as_deref() {
        Some(mime) if mime.starts_with("image/") => "image",
        Some(mime) if mime.starts_with("video/") => "video",
        _ => "other",
    }
}

fn state_label(record: &LocalMediaRecord) -> &'static str {
    if record.deleted_on_rov {
        "deleted on ROV"
    } else if record.local_path.is_some() {
        "local"
    } else {
        "remote only"
    }
}

fn scene_label(scene: Option<i32>) -> &'static str {
    match scene {
        Some(0) => "Normal",
        Some(1) => "Vessel inspection",
        Some(2) => "Fishing net",
        Some(_) => "Other",
        None => "-",
    }
}

fn rov_stat_label(code: Option<i32>) -> &'static str {
    match code {
        Some(0) => "Normal",
        Some(1) => "Needs repair",
        Some(2) => "Repairing",
        Some(3) => "Repair failed",
        Some(_) => "Other",
        None => "-",
    }
}

fn build_details_text(record: &LocalMediaRecord) -> String {
    let mut lines = Vec::<String>::new();
    lines.push(format!("Size: {}", format_bytes(record.size_bytes)));
    if let (Some(w), Some(h)) = (record.width, record.height)
        && w > 0
        && h > 0
    {
        lines.push(format!("Dimensions: {w} \u{00d7} {h}"));
    }
    if let Some(dur) = record.duration_s
        && dur > 0
    {
        lines.push(format!("Duration: {dur} s"));
    }
    if let Some(mime) = &record.mime {
        lines.push(format!("Type: {mime}"));
    }
    lines.push(format!("Scene: {}", scene_label(record.scene)));
    lines.push(format!(
        "ROV file status: {}",
        rov_stat_label(record.rov_stat)
    ));
    lines.push(format!(
        "First seen: {}",
        format_relative_age(record.first_seen_ms)
    ));
    lines.push(format!(
        "Last seen: {}",
        format_relative_age(record.last_seen_ms)
    ));
    if record.deleted_on_rov {
        lines.push("Flagged as deleted on the ROV since last refresh.".to_owned());
    }
    if let Some(hash) = &record.local_sha256 {
        lines.push(format!("Local SHA-256: {hash}"));
    }
    lines.join("\n")
}

fn build_capture_text(meta: &StoredCaptureMetadata) -> String {
    fn opt_num<T: std::fmt::Display>(
        prefix: &str,
        value: Option<T>,
        suffix: &str,
    ) -> Option<String> {
        value.map(|v| format!("{prefix}{v}{suffix}"))
    }
    let mut lines = Vec::<String>::new();
    lines.push(format!(
        "Captured at: {} ({})",
        format_relative_age(meta.captured_at_ms),
        meta.captured_at_ms
    ));
    if let (Some(pitch), Some(roll), Some(yaw)) = (meta.pitch, meta.roll, meta.yaw) {
        lines.push(format!(
            "Attitude [rad]: pitch={pitch:.3}, roll={roll:.3}, yaw={yaw:.3}"
        ));
    }
    if let Some(depth) = meta.depth_m {
        lines.push(format!("Depth: {depth:.2} m"));
    }
    if let Some(temp) = meta.temperature_c {
        lines.push(format!("Temperature: {temp:.1} \u{00b0}C"));
    }
    if let (Some(lat), Some(lon)) = (meta.lat_e7, meta.lon_e7) {
        let lat_deg = lat as f64 / 1e7;
        let lon_deg = lon as f64 / 1e7;
        lines.push(format!(
            "Coordinates: {lat_deg:.6}, {lon_deg:.6} (lat_e7={lat}, lon_e7={lon})"
        ));
    } else {
        if let Some(line) = opt_num("lat_e7=", meta.lat_e7, "") {
            lines.push(line);
        }
        if let Some(line) = opt_num("lon_e7=", meta.lon_e7, "") {
            lines.push(line);
        }
    }
    if let Some(imu) = &meta.imu_json {
        lines.push(format!("IMU: {imu}"));
    }
    if let Some(batts) = &meta.batteries_json
        && batts != "[]"
        && !batts.is_empty()
    {
        lines.push(format!("Batteries: {batts}"));
    }
    if let Some(note) = &meta.note
        && !note.is_empty()
    {
        lines.push(format!("Note: {note}"));
    }
    if let Some(tags) = &meta.tags_json
        && tags != "[]"
        && !tags.is_empty()
    {
        lines.push(format!("Tags: {tags}"));
    }
    lines.join("\n")
}

fn refresh_media_rows(state: &mut ThirdEyeState, store: &AppStore) {
    match store.media().list_all() {
        Ok(rows) => {
            state.media.rows = rows;
        }
        Err(err) => {
            state.media.status_text = format!("Failed to list local media: {err:#}");
        }
    }
    // Build thumbnails for newly-downloaded images.
    for row in &state.media.rows {
        if state.media.thumbnail_cache.contains_key(&row.name) {
            continue;
        }
        if is_image_name(&row.name)
            && let Some(path) = &row.local_path
            && let Some(img) = load_image_preview(path, 192)
        {
            state.media.thumbnail_cache.insert(row.name.clone(), img);
        }
    }
    // Refresh the detail panel too, so any background update is reflected.
    recompute_media_selection_details(state, store);
}

fn is_image_name(name: &str) -> bool {
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

fn is_video_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    std::path::Path::new(&lower)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
        || std::path::Path::new(&lower)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mov"))
}

fn build_media_download_url(rov_http_base: &str, name: &str) -> Result<String> {
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

fn load_image_preview(path: &str, max_dim: u32) -> Option<slint::Image> {
    let img = image::open(path).ok()?;
    let img = img.resize(max_dim, max_dim, image::imageops::FilterType::Triangle);
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let frame = RgbaFrame {
        width: w,
        height: h,
        rgba: rgba.into_raw(),
    };
    Some(rgba_frame_to_slint_image(&frame))
}

fn build_info_subtitle(record: &LocalMediaRecord) -> String {
    let mut parts = Vec::new();
    parts.push(format_bytes(record.size_bytes));
    if let Some(mime) = &record.mime {
        parts.push(mime.clone());
    }
    if let (Some(w), Some(h)) = (record.width, record.height)
        && w > 0
        && h > 0
    {
        parts.push(format!("{w}\u{00d7}{h}"));
    }
    if let Some(dur) = record.duration_s
        && dur > 0
    {
        parts.push(format!("{dur}s"));
    }
    parts.join(" \u{2022} ")
}

fn populate_capture_overlay(state: &mut ThirdEyeState, meta: &StoredCaptureMetadata) {
    state.media.capture_depth = meta
        .depth_m
        .map(|d| format!("{d:.1} m"))
        .unwrap_or_default();
    state.media.capture_temp = meta
        .temperature_c
        .map(|t| format!("{t:.1} \u{00b0}C"))
        .unwrap_or_default();
    state.media.capture_heading = meta
        .yaw
        .map(|y| format!("{:.0}\u{00b0}", y.to_degrees().rem_euclid(360.0)))
        .unwrap_or_default();
    state.media.capture_attitude = match (meta.pitch, meta.roll) {
        (Some(p), Some(r)) => format!(
            "P {:.1}\u{00b0}  R {:.1}\u{00b0}",
            p.to_degrees(),
            r.to_degrees()
        ),
        _ => String::new(),
    };
    state.media.capture_coords = match (meta.lat_e7, meta.lon_e7) {
        (Some(lat), Some(lon)) => {
            let lat_deg = lat as f64 / 1e7;
            let lon_deg = lon as f64 / 1e7;
            format!("{lat_deg:.4}, {lon_deg:.4}")
        }
        _ => String::new(),
    };
    state.media.capture_battery = meta
        .batteries_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<Vec<serde_json::Value>>(json).ok())
        .map(|batts| {
            batts
                .iter()
                .filter_map(|b| b.get("remain").and_then(serde_json::Value::as_i64))
                .map(|r| format!("{r}%"))
                .collect::<Vec<_>>()
                .join(" / ")
        })
        .unwrap_or_default();
}

fn clear_capture_overlay(state: &mut ThirdEyeState) {
    state.media.capture_depth.clear();
    state.media.capture_temp.clear();
    state.media.capture_heading.clear();
    state.media.capture_attitude.clear();
    state.media.capture_coords.clear();
    state.media.capture_battery.clear();
}

fn recompute_media_selection_details(state: &mut ThirdEyeState, store: &AppStore) {
    let Some((media_id, name)) = state.media.selected.clone() else {
        state.media.details_text.clear();
        state.media.capture_text.clear();
        state.media.has_capture_meta = false;
        state.media.local_path.clear();
        state.media.preview_image = None;
        state.media.info_subtitle.clear();
        clear_capture_overlay(state);
        return;
    };
    let record = state
        .media
        .rows
        .iter()
        .find(|r| r.media_id == media_id && r.name == name);
    if let Some(record) = record {
        state.media.details_text = build_details_text(record);
        state.media.info_subtitle = build_info_subtitle(record);
        state.media.local_path = record.local_path.clone().unwrap_or_default();
        // Load preview from local file if it's an image.
        if is_image_name(&name) && !state.media.local_path.is_empty() {
            state.media.preview_image = load_image_preview(&state.media.local_path, 800);
        } else if !state.media.media_stream_active {
            state.media.preview_image = None;
        }
    } else {
        // Row was pruned (e.g. DB reset); clear selection.
        state.media.selected = None;
        state.media.details_text.clear();
        state.media.info_subtitle.clear();
        state.media.local_path.clear();
        state.media.preview_image = None;
    }
    match store.media().get_capture_metadata(&media_id, &name) {
        Ok(Some(meta)) => {
            state.media.capture_text = build_capture_text(&meta);
            state.media.has_capture_meta = true;
            populate_capture_overlay(state, &meta);
        }
        Ok(None) => {
            state.media.capture_text.clear();
            state.media.has_capture_meta = false;
            clear_capture_overlay(state);
        }
        Err(err) => {
            state.media.capture_text = format!("Failed to load capture metadata: {err:#}");
            state.media.has_capture_meta = true;
            clear_capture_overlay(state);
        }
    }
}

fn apply_media_runtime_to_ui(ui: &AppWindow, state: &ThirdEyeState) {
    let selected = state.media.selected.clone();
    let empty_img = slint::Image::default();
    let rows: Vec<MediaRow> = state
        .media
        .rows
        .iter()
        .map(|r| {
            let thumb = state.media.thumbnail_cache.get(&r.name);
            MediaRow {
                media_id: r.media_id.clone().into(),
                name: r.name.clone().into(),
                size_text: format_bytes(r.size_bytes).into(),
                seen_text: format!("seen {}", format_relative_age(r.last_seen_ms)).into(),
                state_text: state_label(r).into(),
                origin_text: origin_label(r).into(),
                has_local: r.local_path.is_some(),
                deleted_on_rov: r.deleted_on_rov,
                selected: matches!(
                    &selected,
                    Some((id, name)) if id == &r.media_id && name == &r.name
                ),
                thumbnail: thumb.cloned().unwrap_or_else(|| empty_img.clone()),
                has_thumbnail: thumb.is_some(),
            }
        })
        .collect();
    ui.set_media_rows(ModelRc::new(VecModel::from(rows)));
    ui.set_media_status(state.media.status_text.clone().into());
    let (sel_id, sel_name) = selected.clone().unwrap_or_default();
    ui.set_media_selected_id(sel_id.into());
    ui.set_media_selected_name(sel_name.into());
    ui.set_media_selected_details(state.media.details_text.clone().into());
    ui.set_media_selected_capture_text(state.media.capture_text.clone().into());
    ui.set_media_selected_local_path(state.media.local_path.clone().into());
    ui.set_media_selected_has_capture_meta(state.media.has_capture_meta);
    ui.set_media_download_in_progress(state.media.download_in_progress);
    let selected_is_video = state
        .media
        .selected
        .as_ref()
        .is_some_and(|(_, name)| is_video_name(name));
    ui.set_media_selected_is_video(selected_is_video);
    ui.set_media_stream_active(state.media.media_stream_active);
    ui.set_media_info_subtitle(state.media.info_subtitle.clone().into());
    ui.set_media_capture_depth(state.media.capture_depth.clone().into());
    ui.set_media_capture_temp(state.media.capture_temp.clone().into());
    ui.set_media_capture_heading(state.media.capture_heading.clone().into());
    ui.set_media_capture_attitude(state.media.capture_attitude.clone().into());
    ui.set_media_capture_coords(state.media.capture_coords.clone().into());
    ui.set_media_capture_battery(state.media.capture_battery.clone().into());
    if let Some(img) = &state.media.preview_image {
        ui.set_media_preview_image(img.clone());
        ui.set_has_media_preview(true);
    } else {
        ui.set_has_media_preview(false);
    }
}

fn current_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Reconciles the ROV media list and writes a `capture_metadata` row for the
/// file that was most recently seen. Returns the summary text the UI should
/// render, or `None` if nothing could be attached.
fn attach_capture_metadata_to_latest(
    client: &CameraApiClient,
    media_store: &MediaStore,
    status: Option<&RovUdpStatus>,
    captured_at_ms: i64,
) -> Result<Option<String>> {
    // Snapshot existing media names so we can detect the newly captured file
    // after the ROV listing is applied (apply_rov_listing sets all rows'
    // last_seen_ms to the same value, breaking list_recent ordering).
    let known_names: std::collections::HashSet<String> = media_store
        .list_all()?
        .into_iter()
        .map(|r| r.name)
        .collect();

    let items = client.list_medias(None::<MediaScene>)?;
    media_store.apply_rov_listing(&items, None)?;

    // Identify the new item(s) that appeared on the ROV since our last sync.
    let mut new_items: Vec<&MediaInfo> = items
        .iter()
        .filter(|item| !known_names.contains(&item.name))
        .collect();
    // Sort by name descending: timestamp-based names sort newest-first.
    new_items.sort_by(|a, b| b.name.cmp(&a.name));

    let target = if let Some(newest) = new_items.first() {
        Some((newest.origin.id.clone(), newest.name.clone()))
    } else {
        // No new items — fall back to the item with the newest name
        // (timestamp-based names, so alphabetically last = most recent).
        items
            .iter()
            .max_by(|a, b| a.name.cmp(&b.name))
            .map(|item| (item.origin.id.clone(), item.name.clone()))
    };

    let Some((media_id, name)) = target else {
        return Ok(None);
    };

    media_store.attach_capture_metadata(&media_id, &name, captured_at_ms, status, None)?;
    let mut line = format!("Attached capture metadata to {name}.");
    if let Some(status) = status {
        line.push_str(&format!(
            " depth {:.2} m, yaw {:.2} rad, lat_e7={}, lon_e7={}",
            status.depth, status.yaw, status.lat, status.lon
        ));
    } else {
        line.push_str(" (no ROV telemetry snapshot was available - start the UDP listener to capture depth/yaw/coords)");
    }
    Ok(Some(line))
}

fn register_callbacks(ui: &AppWindow, state: Rc<RefCell<ThirdEyeState>>, store: Rc<AppStore>) {
    let ui_weak = ui.as_weak();
    let state_for_configuration = Rc::clone(&state);
    let store_for_configuration = Rc::clone(&store);
    ui.on_navigate_configuration(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_configuration.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state, &store_for_configuration);
        if state.last_screen == Screen::Stream {
            state.stream_left_at_ms = current_unix_ms();
        }
        state.media.stop_media_stream();
        state.active_screen = Screen::Configuration;
        state.last_screen = Screen::Configuration;
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_nmea_nav = Rc::clone(&state);
    let store_for_nmea_nav = Rc::clone(&store);
    ui.on_navigate_nmea(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_nmea_nav.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state, &store_for_nmea_nav);
        if state.last_screen == Screen::Stream {
            state.stream_left_at_ms = current_unix_ms();
        }
        state.media.stop_media_stream();
        // Refresh the BT port list when entering the Phone GPS screen.
        let bt_ports = third_eye_client::nmea::list_bluetooth_ports();
        let ports_text = if bt_ports.is_empty() {
            String::new()
        } else {
            bt_ports.join(", ")
        };
        ui.set_nmea_serial_ports(ports_text.into());
        state.active_screen = Screen::Nmea;
        state.last_screen = Screen::Nmea;
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_map_flicked = Rc::clone(&state);
    ui.on_map_flicked(
        move |viewport_x, viewport_y, viewport_width, viewport_height| {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let mut state = match state_for_map_flicked.try_borrow_mut() {
                Ok(state) => state,
                Err(_) => return,
            };
            if state.suppress_next_map_flick {
                state.suppress_next_map_flick = false;
                return;
            }
            state.viewport_anim = None;
            state.set_map_visible_size(f64::from(viewport_width), f64::from(viewport_height));
            state.set_map_viewport(f64::from(viewport_x), f64::from(viewport_y));
            apply_map_runtime_to_ui(&ui, &state);
        },
    );

    let ui_weak = ui.as_weak();
    let state_for_map_zoom_in = Rc::clone(&state);
    ui.on_map_zoom_in(
        move |viewport_x, viewport_y, viewport_width, viewport_height| {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let mut state = match state_for_map_zoom_in.try_borrow_mut() {
                Ok(state) => state,
                Err(_) => return,
            };
            state.set_map_visible_size(f64::from(viewport_width), f64::from(viewport_height));
            state.set_map_viewport(f64::from(viewport_x), f64::from(viewport_y));
            state.viewport_anim = None;
            let next_zoom = state.map.zoom.saturating_add(1).min(MAX_ZOOM);
            let (focus_x, focus_y) = state.map_tiles.zoom_focus_center();
            state.set_map_zoom(next_zoom, focus_x, focus_y);
            state.suppress_next_map_flick = true;
            state.map.status = format!("Zoomed in to {}.", state.map.zoom);
            apply_map_runtime_to_ui(&ui, &state);
        },
    );

    let ui_weak = ui.as_weak();
    let state_for_map_zoom_out = Rc::clone(&state);
    ui.on_map_zoom_out(
        move |viewport_x, viewport_y, viewport_width, viewport_height| {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let mut state = match state_for_map_zoom_out.try_borrow_mut() {
                Ok(state) => state,
                Err(_) => return,
            };
            state.set_map_visible_size(f64::from(viewport_width), f64::from(viewport_height));
            state.set_map_viewport(f64::from(viewport_x), f64::from(viewport_y));
            state.viewport_anim = None;
            let next_zoom = state.map.zoom.saturating_sub(1).max(MIN_ZOOM);
            let (focus_x, focus_y) = state.map_tiles.zoom_focus_center();
            state.set_map_zoom(next_zoom, focus_x, focus_y);
            state.suppress_next_map_flick = true;
            state.map.status = format!("Zoomed out to {}.", state.map.zoom);
            apply_map_runtime_to_ui(&ui, &state);
        },
    );

    let ui_weak = ui.as_weak();
    let state_for_map_center_on_pin = Rc::clone(&state);
    ui.on_center_map_on_pin(
        move |_viewport_x, _viewport_y, viewport_width, viewport_height| {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let mut state = match state_for_map_center_on_pin.try_borrow_mut() {
                Ok(state) => state,
                Err(_) => return,
            };
            state.set_map_visible_size(f64::from(viewport_width), f64::from(viewport_height));
            state.map_tiles.fallback_zoom = None;
            let (old_vp_x, old_vp_y, _, _) = state.map_tiles.viewport_for_slint(state.map.zoom);
            // Non-blocking: try NMEA GPS, then CoreLocation cached fix.
            // Never call the blocking detect_location() from an event handler.
            let fresh = if let Some((lat, lon)) = state.nmea_gps.latest_location() {
                Some((lat, lon, "Phone GPS (NMEA/TCP)".to_owned()))
            } else {
                #[cfg(target_os = "macos")]
                {
                    check_corelocation_warmup_fix(&state.map)
                        .map(|(lat, lon)| (lat, lon, "macOS CoreLocation (native)".to_owned()))
                }
                #[cfg(not(target_os = "macos"))]
                {
                    None
                }
            };
            if let Some((lat, lon, source)) = fresh {
                state.map.lat = Some(lat);
                state.map.lon = Some(lon);
                state.location_detected_at_ms = current_unix_ms();
                state.load_map_tile_for_current_location(format!(
                    "Centered on device location via {source}: lat={lat:.6}, lon={lon:.6}."
                ));
            } else if state.map.lat.is_some() && state.map.lon.is_some() {
                state.load_map_tile_for_current_location(
                    "Centered on last known location.".to_owned(),
                );
            } else {
                state.map.status =
                    "No location available. Use Detect Location button first.".to_owned();
            }
            let (target_vp_x, target_vp_y, _, _) =
                state.map_tiles.viewport_for_slint(state.map.zoom);
            if (old_vp_x - target_vp_x).abs() > 1.0 || (old_vp_y - target_vp_y).abs() > 1.0 {
                state.viewport_anim = Some(ViewportAnimation {
                    start_vp_x: old_vp_x,
                    start_vp_y: old_vp_y,
                    target_vp_x,
                    target_vp_y,
                    elapsed_ms: 0.0,
                    duration_ms: 300.0,
                });
            }
            state.suppress_next_map_flick = true;
            apply_map_runtime_to_ui(&ui, &state);
        },
    );

    let ui_weak = ui.as_weak();
    let state_for_map_navigation = Rc::clone(&state);
    let store_for_map_navigation = Rc::clone(&store);
    ui.on_navigate_map(move |content_width, content_height| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_map_navigation.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state, &store_for_map_navigation);
        if state.last_screen == Screen::Stream {
            state.stream_left_at_ms = current_unix_ms();
        }
        state.media.stop_media_stream();
        state.active_screen = Screen::Map;
        // Map fills the entire content panel
        let est_width = f64::from(content_width).max(320.0);
        let est_height = f64::from(content_height).max(320.0);
        state.set_map_visible_size(est_width, est_height);
        state.map_tiles.fallback_zoom = None;
        state.auto_refresh_map_on_tab_enter();
        state.last_screen = Screen::Map;
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_stream_navigation = Rc::clone(&state);
    let store_for_stream_navigation = Rc::clone(&store);
    ui.on_navigate_stream(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_stream_navigation.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state, &store_for_stream_navigation);

        // Note: location is NOT refreshed here via detect_location() because
        // that call blocks the main thread (CoreLocation polling on macOS,
        // Windows GPS warmup) from inside an ObjC/winit event handler, which
        // causes panic_cannot_unwind. Location is kept up-to-date by:
        //   • the background warmup timer (macOS CoreLocation / Windows GPS)
        //   • NMEA GPS polling
        //   • explicit "Detect Location" button clicks
        // Use whatever location is already in state; the POS overlay will
        // show "stale" or "—" if the fix is missing or outdated.

        // Auto-detect ROV interface before starting stream.
        refresh_rov_network(&mut state, false);
        persist_config(&state, &store_for_stream_navigation);

        // Always restart stream+telemetry: the underlying network may have
        // changed (WiFi ↔ hotspot ↔ cable) even if the interface name didn't.
        state.stream_left_at_ms = 0;
        state.stream.stop();
        state.rov_status.stop();
        {
            // Set up external route for ffmpeg now that we know the interface.
            if let Some(iface) = state.config.rov_interface()
                && let Err(err) = ensure_rov_external_route(&state.config.rov_http_base, iface)
            {
                state.rov_info = format!(
                    "Detected interface {iface} but route setup failed: {err:#}. RTSP may not work."
                );
            }
            state.stream.stop();
            let rtsp_url = state.config.rtsp_url.clone();
            let rov_http_base = state.config.rov_http_base.clone();
            let rov_interface = state.config.rov_interface().map(str::to_owned);
            state.stream.status =
                match state
                    .stream
                    .start(rtsp_url, Some(&rov_http_base), rov_interface.as_deref())
                {
                    Ok(msg) => msg,
                    Err(err) => format!("Failed to start stream: {err:#}"),
                };
            ui.set_has_stream_image(false);
        }

        // Auto-start telemetry listener on 0.0.0.0.
        if !state.rov_status.is_running() {
            let port = state.config.parse_rov_status_udp_port();
            match port {
                Ok(port) => {
                    let bind_host = DEFAULT_ROV_UDP_BIND_HOST.to_owned();
                    let iface = state.config.rov_interface().map(str::to_owned);
                    if let Err(err) = state.rov_status.start(&bind_host, port, iface.as_deref()) {
                        state
                            .rov_status
                            .set_status_text(format!("Failed to start UDP listener: {err:#}"));
                    }
                }
                Err(err) => {
                    state
                        .rov_status
                        .set_status_text(format!("Invalid telemetry UDP port: {err:#}"));
                }
            }
        }

        state.media.stop_media_stream();
        state.active_screen = Screen::Stream;
        state.last_screen = Screen::Stream;
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_default_test_rtsp = Rc::clone(&state);
    let store_for_default_test_rtsp = Rc::clone(&store);
    ui.on_use_default_test_rtsp(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_default_test_rtsp.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        state.config.rtsp_url = DEFAULT_TEST_RTSP.to_owned();
        persist_config(&state, &store_for_default_test_rtsp);
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_default_rov_rtsp = Rc::clone(&state);
    let store_for_default_rov_rtsp = Rc::clone(&store);
    ui.on_use_default_rov_rtsp(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_default_rov_rtsp.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        state.config.rtsp_url = DEFAULT_ROV_RTSP.to_owned();
        persist_config(&state, &store_for_default_rov_rtsp);
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_default_rov_http = Rc::clone(&state);
    let store_for_default_rov_http = Rc::clone(&store);
    ui.on_use_default_rov_http_base(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_default_rov_http.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        state.config.rov_http_base = DEFAULT_ROV_HTTP_BASE.to_owned();
        state.config.rov_status_udp_bind_host = default_rov_udp_bind_host();
        persist_config(&state, &store_for_default_rov_http);
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_use_host_from_base = Rc::clone(&state);
    let store_for_use_host_from_base = Rc::clone(&store);
    ui.on_use_host_from_rov_http_base(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_use_host_from_base.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state, &store_for_use_host_from_base);
        state.config.rov_status_udp_bind_host = default_rov_udp_bind_host();
        persist_config(&state, &store_for_use_host_from_base);
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_default_rov_udp_port = Rc::clone(&state);
    let store_for_default_rov_udp_port = Rc::clone(&store);
    ui.on_use_default_rov_status_udp_port(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_default_rov_udp_port.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        state.config.rov_status_udp_port = ROV_STATUS_UDP_PORT.to_string();
        persist_config(&state, &store_for_default_rov_udp_port);
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_default_osm_ua = Rc::clone(&state);
    let store_for_default_osm_ua = Rc::clone(&store);
    ui.on_use_default_osm_tile_user_agent(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_default_osm_ua.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        state.config.osm_tile_user_agent = DEFAULT_OSM_TILE_USER_AGENT.to_owned();
        persist_config(&state, &store_for_default_osm_ua);
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_recalibrate = Rc::clone(&state);
    let store_for_recalibrate = Rc::clone(&store);
    ui.on_recalibrate_rov_network(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_recalibrate.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state, &store_for_recalibrate);
        refresh_rov_network(&mut state, true);
        persist_config(&state, &store_for_recalibrate);
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_default_server_url = Rc::clone(&state);
    let store_for_default_server_url = Rc::clone(&store);
    ui.on_use_default_server_base_url(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_default_server_url.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        state.config.server_base_url = DEFAULT_SERVER_BASE_URL.to_owned();
        persist_config(&state, &store_for_default_server_url);
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_list_medias = Rc::clone(&state);
    let store_for_list_medias = Rc::clone(&store);
    ui.on_list_medias(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_list_medias.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state, &store_for_list_medias);
        let client = CameraApiClient::new_bound(
            state.config.rov_http_base.clone(),
            state.config.rov_interface(),
        );
        let media_store = store_for_list_medias.media().clone();
        let tx = state.media.event_tx.clone();
        state.rov_info = "Listing media on ROV...".to_owned();
        thread::spawn(move || {
            let rov_info = match client.list_medias(None::<MediaScene>) {
                Ok(items) => {
                    let rendered = if items.is_empty() {
                        "No media files on camera.".to_owned()
                    } else {
                        let mut lines = vec![format!("Media files ({}):", items.len())];
                        for item in &items {
                            lines.push(format!(
                                "- {} ({} bytes){}",
                                item.name,
                                item.size,
                                if item.canplayback { " [video]" } else { "" }
                            ));
                        }
                        lines.join("\n")
                    };
                    match media_store.apply_rov_listing(&items, None) {
                        Ok(report) => format!(
                            "{rendered}\n[sync] new={}, updated={}, disappeared_now={}",
                            report.new_media, report.updated_media, report.disappeared_media
                        ),
                        Err(err) => {
                            format!("{rendered}\n[sync] failed to update local registry: {err:#}")
                        }
                    }
                }
                Err(err) => format!("List medias failed: {err:#}"),
            };
            let _ = tx.send(MediaEvent::ListMedias { rov_info });
        });
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_capture = Rc::clone(&state);
    let store_for_capture = Rc::clone(&store);
    ui.on_capture_photo(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_capture.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        if state.media.capture_in_progress {
            return;
        }
        pull_configuration_from_ui(&ui, &mut state, &store_for_capture);

        // Refresh location from the best non-blocking source before capture so
        // the freshest possible coordinates are attached to the photo metadata.
        {
            let fresh_fix: Option<(f64, f64)> = if let Some(fix) = state.nmea_gps.latest_location()
            {
                Some(fix)
            } else {
                #[cfg(target_os = "macos")]
                {
                    check_corelocation_warmup_fix(&state.map)
                }
                #[cfg(not(target_os = "macos"))]
                {
                    None
                }
            };
            if let Some((lat, lon)) = fresh_fix {
                state.map.lat = Some(lat);
                state.map.lon = Some(lon);
                state.location_detected_at_ms = current_unix_ms();
            }
        }
        // Snapshot the latest ROV telemetry *before* the capture call so we
        // attribute the correct depth/attitude/coords to the image.
        let mut status_snapshot: Option<RovUdpStatus> = state.rov_status.latest_status().cloned();
        // The ROV UDP always sends 0,0 for lat/lon — override with the
        // device's native GPS position (same source as the POS overlay).
        if let Some(ref mut status) = status_snapshot {
            let location_age_ms = current_unix_ms() - state.location_detected_at_ms;
            if let (Some(lat), Some(lon)) = (state.map.lat, state.map.lon)
                && state.location_detected_at_ms > 0
                && location_age_ms < 600_000
            {
                status.lat = (lat * 1e7) as i32;
                status.lon = (lon * 1e7) as i32;
            }
        }
        let captured_at_ms = current_unix_ms();

        let client = CameraApiClient::new_bound(
            state.config.rov_http_base.clone(),
            state.config.rov_interface(),
        );
        let media_store = store_for_capture.media().clone();
        let tx = state.media.event_tx.clone();
        state.media.capture_in_progress = true;
        state.rov_info = "Capturing photo...".to_owned();
        if state.active_screen == Screen::Stream {
            state.stream.status = "Capturing photo...".to_owned();
        }
        thread::spawn(move || {
            match client.capture(PhotoFormat::Jpeg, 1) {
                Ok(resp) => {
                    let msg = resp.msg.as_deref().unwrap_or("success");
                    let capture_msg = format!("Capture OK: {msg}");
                    // Give the camera a brief moment to materialise the file.
                    std::thread::sleep(Duration::from_millis(400));
                    let attached_text = match attach_capture_metadata_to_latest(
                        &client,
                        &media_store,
                        status_snapshot.as_ref(),
                        captured_at_ms,
                    ) {
                        Ok(Some(line)) => line,
                        Ok(None) => String::new(),
                        Err(err) => format!("Capture metadata attach failed: {err:#}"),
                    };
                    let _ = tx.send(MediaEvent::Capture {
                        capture_msg,
                        attached_text,
                    });
                }
                Err(err) => {
                    let _ = tx.send(MediaEvent::Capture {
                        capture_msg: format!("Capture failed: {err:#}"),
                        attached_text: String::new(),
                    });
                }
            }
        });
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_sign_in = Rc::clone(&state);
    let store_for_sign_in = Rc::clone(&store);
    ui.on_sign_in(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_sign_in.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state, &store_for_sign_in);
        let email = state.auth.email.trim().to_owned();
        let password = state.auth.password.clone();
        let server_base = state.config.server_base_url.trim().to_owned();
        if email.is_empty() || password.is_empty() {
            state.auth.status_text = "Email and password are required to sign in.".to_owned();
            apply_state_to_ui(&ui, &state);
            return;
        }
        match store_for_sign_in
            .auth()
            .login(&server_base, &email, &password)
        {
            Ok(outcome) => {
                state.auth.is_signed_in = true;
                state.auth.signed_in_as = outcome.email.clone();
                // The "Signed in as <email>" line is rendered from
                // `auth_signed_in_as`; keep the status line complementary so
                // the UI doesn't print the email twice.
                state.auth.status_text = "Signed in successfully.".to_owned();
                // Do NOT keep the plaintext password in the state or UI.
                state.auth.password.clear();
            }
            Err(err) => {
                state.auth.is_signed_in = false;
                state.auth.status_text = format!("Sign in failed: {err}");
            }
        }
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_sign_out = Rc::clone(&state);
    let store_for_sign_out = Rc::clone(&store);
    ui.on_sign_out(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_sign_out.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state, &store_for_sign_out);
        let server_base = state.config.server_base_url.trim().to_owned();
        match store_for_sign_out.auth().logout(&server_base) {
            Ok(()) => {
                state.auth.is_signed_in = false;
                state.auth.signed_in_as.clear();
                state.auth.status_text = "Signed out.".to_owned();
            }
            Err(err) => {
                // Local session is cleared inside `logout` even on error.
                state.auth.is_signed_in = false;
                state.auth.signed_in_as.clear();
                state.auth.status_text = format!("Signed out locally (server: {err}).");
            }
        }
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_detect_location = Rc::clone(&state);
    let store_for_detect_location = Rc::clone(&store);
    ui.on_detect_location(move |viewport_width, viewport_height| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_detect_location.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state, &store_for_detect_location);
        state.set_map_visible_size(f64::from(viewport_width), f64::from(viewport_height));
        // Non-blocking: try NMEA GPS then CoreLocation cached fix.
        // Restart CoreLocation updates so the background timer delivers a
        // fresh fix within the next polling cycle.
        #[cfg(target_os = "macos")]
        prime_corelocation_at_startup(&mut state.map);
        let fresh = if let Some((lat, lon)) = state.nmea_gps.latest_location() {
            Some((lat, lon, "Phone GPS (NMEA/TCP)".to_owned()))
        } else {
            #[cfg(target_os = "macos")]
            {
                check_corelocation_warmup_fix(&state.map)
                    .map(|(lat, lon)| (lat, lon, "macOS CoreLocation (native)".to_owned()))
            }
            #[cfg(not(target_os = "macos"))]
            {
                None
            }
        };
        if let Some((lat, lon, source)) = fresh {
            state.map.lat = Some(lat);
            state.map.lon = Some(lon);
            state.location_detected_at_ms = current_unix_ms();
            state.load_map_tile_for_current_location(format!(
                "Detected location via {source}: lat={lat:.6}, lon={lon:.6}. Map auto-refreshed."
            ));
        } else {
            // No cached fix yet — reset the warmup flag so the 16 ms timer
            // resumes polling and will apply the fix as soon as it arrives.
            state.location_detected_at_ms = 0;
            state.map.status =
                "Detecting location in background. The map will update automatically.".to_owned();
        }
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_load_map_tile = Rc::clone(&state);
    let store_for_load_map_tile = Rc::clone(&store);
    ui.on_load_map_tile(move |viewport_width, viewport_height| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_load_map_tile.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state, &store_for_load_map_tile);
        state.set_map_visible_size(f64::from(viewport_width), f64::from(viewport_height));
        state.load_map_tile_for_current_location(
            "Loaded OpenStreetMap tile for detected location.".to_owned(),
        );
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_open_map = Rc::clone(&state);
    ui.on_open_interactive_map(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_open_map.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        state.map.status = match (state.map.lat, state.map.lon) {
            (Some(lat), Some(lon)) => {
                let url = format!(
                    "https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map={}/{lat}/{lon}",
                    state.map.zoom
                );
                match webbrowser::open(&url) {
                    Ok(()) => "Opened map in browser.".to_owned(),
                    Err(err) => format!("Failed to open browser map: {err:#}"),
                }
            }
            _ => "No location set. Use Detect location first.".to_owned(),
        };
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_start_stream = Rc::clone(&state);
    let store_for_start_stream = Rc::clone(&store);
    ui.on_start_stream(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_start_stream.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state, &store_for_start_stream);
        state.stream.stop();
        let rtsp_url = state.config.rtsp_url.clone();
        let rov_http_base = state.config.rov_http_base.clone();
        let rov_interface = state.config.rov_interface().map(str::to_owned);
        state.stream.status =
            match state
                .stream
                .start(rtsp_url, Some(&rov_http_base), rov_interface.as_deref())
            {
                Ok(msg) => msg,
                Err(err) => format!("Failed to start stream: {err:#}"),
            };
        ui.set_has_stream_image(false);
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_stop_stream = Rc::clone(&state);
    ui.on_stop_stream(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_stop_stream.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        state.stream.stop();
        ui.set_has_stream_image(false);
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_start_rov_listener = Rc::clone(&state);
    let store_for_start_rov_listener = Rc::clone(&store);
    ui.on_start_rov_status_listener(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_start_rov_listener.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state, &store_for_start_rov_listener);
        state.rov_status.stop();
        let port = match state.config.parse_rov_status_udp_port() {
            Ok(port) => port,
            Err(err) => {
                state
                    .rov_status
                    .set_status_text(format!("Invalid telemetry UDP port: {err:#}"));
                apply_state_to_ui(&ui, &state);
                return;
            }
        };
        let bind_host = state.config.rov_status_udp_bind_host.clone();
        let iface = state.config.rov_interface().map(str::to_owned);
        if let Err(err) = state.rov_status.start(&bind_host, port, iface.as_deref()) {
            state
                .rov_status
                .set_status_text(format!("Failed to start UDP listener: {err:#}"));
        }
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_stop_rov_listener = Rc::clone(&state);
    ui.on_stop_rov_status_listener(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_stop_rov_listener.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        state.rov_status.stop();
        apply_state_to_ui(&ui, &state);
    });

    // --- NMEA GPS callback (auto-detects Bluetooth serial vs TCP) ---

    let ui_weak = ui.as_weak();
    let state_for_start_nmea = Rc::clone(&state);
    let store_for_start_nmea = Rc::clone(&store);
    ui.on_start_nmea_gps(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_start_nmea.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state, &store_for_start_nmea);
        state.nmea_gps.stop();

        // Try Bluetooth first: look for BT SPP ports specifically.
        let bt_ports = third_eye_client::nmea::list_bluetooth_ports();
        if !bt_ports.is_empty() {
            let port_path = &bt_ports[0];
            if bt_ports.len() > 1 {
                state.nmea_gps.set_status(format!(
                    "Found {} Bluetooth ports: {}. Using {}.",
                    bt_ports.len(),
                    bt_ports.join(", "),
                    port_path
                ));
            }
            match state.nmea_gps.start_bluetooth(port_path) {
                Ok(_msg) => {}
                Err(err) => {
                    ui.set_nmea_gps_status(
                        format!("Failed to start Bluetooth GPS on {port_path}: {err:#}").into(),
                    );
                }
            }
            apply_state_to_ui(&ui, &state);
            return;
        }

        // Fall back to TCP: use the IP from the UI field (user-editable).
        let port = match state.config.parse_nmea_gps_port() {
            Ok(port) => port,
            Err(err) => {
                state.nmea_gps = NmeaGpsState::default();
                apply_state_to_ui(&ui, &state);
                ui.set_nmea_gps_status(format!("Invalid NMEA GPS port: {err:#}").into());
                return;
            }
        };
        let host = ui.get_nmea_local_ip().to_string();
        let host = if host.trim().is_empty() {
            detect_local_ip().unwrap_or_default()
        } else {
            host
        };
        match state.nmea_gps.start(&host, port) {
            Ok(_msg) => {}
            Err(err) => {
                ui.set_nmea_gps_status(format!("Failed to start NMEA GPS: {err:#}").into());
            }
        }
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_stop_nmea = Rc::clone(&state);
    ui.on_stop_nmea_gps(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_stop_nmea.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        state.nmea_gps.stop();
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_default_nmea_port = Rc::clone(&state);
    let store_for_default_nmea_port = Rc::clone(&store);
    ui.on_use_default_nmea_gps_port(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_default_nmea_port.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        state.config.nmea_gps_port = DEFAULT_NMEA_GPS_PORT.to_string();
        persist_config(&state, &store_for_default_nmea_port);
        apply_state_to_ui(&ui, &state);
    });

    // --- Media screen callbacks ---

    let ui_weak = ui.as_weak();
    let state_for_nav_media = Rc::clone(&state);
    let store_for_nav_media = Rc::clone(&store);
    ui.on_navigate_media(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_nav_media.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state, &store_for_nav_media);
        if state.last_screen == Screen::Stream {
            state.stream_left_at_ms = current_unix_ms();
        }
        refresh_media_rows(&mut state, &store_for_nav_media);
        if state.media.status_text.is_empty() {
            state.media.status_text = if state.media.rows.is_empty() {
                "No media recorded yet. Click \"Refresh from ROV\" to populate.".to_owned()
            } else {
                format!(
                    "{} media record(s) in local library.",
                    state.media.rows.len()
                )
            };
        }
        state.active_screen = Screen::Media;
        state.last_screen = Screen::Media;
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_refresh_media = Rc::clone(&state);
    let store_for_refresh_media = Rc::clone(&store);
    ui.on_refresh_media(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_refresh_media.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        if state.media.refresh_in_progress {
            return;
        }
        pull_configuration_from_ui(&ui, &mut state, &store_for_refresh_media);
        let client = CameraApiClient::new_bound(
            state.config.rov_http_base.clone(),
            state.config.rov_interface(),
        );
        let media_store = store_for_refresh_media.media().clone();
        let tx = state.media.event_tx.clone();
        state.media.refresh_in_progress = true;
        state.media.status_text = "Refreshing media from ROV...".to_owned();
        thread::spawn(move || {
            let status_text = match client.list_medias(None::<MediaScene>) {
                Ok(items) => match media_store.apply_rov_listing(&items, None) {
                    Ok(report) => format!(
                        "Refreshed. {} on ROV (new {}, updated {}, newly vanished {}).",
                        report.total_on_rov,
                        report.new_media,
                        report.updated_media,
                        report.disappeared_media
                    ),
                    Err(err) => format!("Refresh succeeded but local update failed: {err:#}"),
                },
                Err(err) => format!("Refresh failed: {err:#}"),
            };
            let _ = tx.send(MediaEvent::Refresh { status_text });
        });
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_select_media = Rc::clone(&state);
    let store_for_select_media = Rc::clone(&store);
    ui.on_select_media(move |media_id, name| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_select_media.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        let media_id_str = media_id.to_string();
        let name_str = name.to_string();
        state.media.stop_media_stream();
        state.media.selected = Some((media_id_str.clone(), name_str.clone()));
        recompute_media_selection_details(&mut state, &store_for_select_media);

        // Auto-download images that don't have a local copy yet.
        if is_image_name(&name_str)
            && state.media.local_path.is_empty()
            && !state.media.download_in_progress
        {
            let data_root = match store_for_select_media.data_path().and_then(|p| p.parent()) {
                Some(dir) => dir.to_path_buf(),
                None => std::env::temp_dir().join("third-eye-client"),
            };
            let camera = CameraApiClient::new_bound(
                state.config.rov_http_base.clone(),
                state.config.rov_interface(),
            );
            let tx = state.media.event_tx.clone();
            state.media.download_in_progress = true;
            state.media.status_text = format!("Fetching preview for {name_str}...");
            let media_store = store_for_select_media.media().clone();
            let mid = media_id_str.clone();
            let nm = name_str.clone();
            thread::spawn(move || {
                let result = download_to_local(&media_store, &camera, &data_root, &mid, &nm)
                    .map_err(|err| format!("{err:#}"));
                let _ = tx.send(MediaEvent::Download { name: nm, result });
            });
        }

        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_download_media = Rc::clone(&state);
    let store_for_download_media = Rc::clone(&store);
    ui.on_download_selected_media(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_download_media.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        if state.media.download_in_progress {
            return;
        }
        let Some((media_id, name)) = state.media.selected.clone() else {
            state.media.status_text = "Select a media entry first.".to_owned();
            apply_state_to_ui(&ui, &state);
            return;
        };
        let data_root = match store_for_download_media
            .data_path()
            .and_then(|p| p.parent())
        {
            Some(dir) => dir.to_path_buf(),
            None => std::env::temp_dir().join("third-eye-client"),
        };
        let camera = CameraApiClient::new_bound(
            state.config.rov_http_base.clone(),
            state.config.rov_interface(),
        );
        let tx = state.media.event_tx.clone();
        state.media.download_in_progress = true;
        state.media.status_text = format!("Downloading {name} from ROV...");
        let media_store = store_for_download_media.media().clone();
        let media_id_thread = media_id.clone();
        let name_thread = name.clone();
        thread::spawn(move || {
            let result = download_to_local(
                &media_store,
                &camera,
                &data_root,
                &media_id_thread,
                &name_thread,
            )
            .map_err(|err| format!("{err:#}"));
            let _ = tx.send(MediaEvent::Download {
                name: name_thread,
                result,
            });
        });
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_open_media = Rc::clone(&state);
    ui.on_open_selected_local_media(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_open_media.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        if state.media.local_path.is_empty() {
            state.media.status_text = "No local copy for this media yet.".to_owned();
        } else {
            match webbrowser::open(&state.media.local_path) {
                Ok(()) => {
                    state.media.status_text = format!("Opened {}", state.media.local_path);
                }
                Err(err) => {
                    state.media.status_text = format!("Failed to open local file: {err:#}");
                }
            }
        }
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_delete_media = Rc::clone(&state);
    let store_for_delete_media = Rc::clone(&store);
    ui.on_delete_selected_media_from_rov(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_delete_media.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        let Some((_, name)) = state.media.selected.clone() else {
            state.media.status_text = "Select a media entry first.".to_owned();
            apply_state_to_ui(&ui, &state);
            return;
        };
        // Immediate local cleanup (fast).
        if !state.media.local_path.is_empty() {
            let _ = std::fs::remove_file(&state.media.local_path);
        }
        let _ = store_for_delete_media.media().remove_by_name(&name);
        state.media.thumbnail_cache.remove(&name);
        state.media.selected = None;
        state.media.preview_image = None;
        state.media.status_text = format!("Deleting {name} from ROV...");
        refresh_media_rows(&mut state, &store_for_delete_media);
        // ROV HTTP delete in background.
        let client = CameraApiClient::new_bound(
            state.config.rov_http_base.clone(),
            state.config.rov_interface(),
        );
        let tx = state.media.event_tx.clone();
        let name_thread = name.clone();
        thread::spawn(move || {
            let status_text = match client.delete_media(&name_thread) {
                Ok(()) => format!("Deleted {name_thread}."),
                Err(err) => {
                    format!("Deleted {name_thread} locally (ROV delete failed: {err:#}).")
                }
            };
            let _ = tx.send(MediaEvent::Delete { status_text });
        });
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_stream_media = Rc::clone(&state);
    ui.on_stream_selected_media(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_stream_media.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        let Some((_, name)) = state.media.selected.clone() else {
            state.media.status_text = "Select a media entry first.".to_owned();
            apply_state_to_ui(&ui, &state);
            return;
        };
        if state.media.media_stream_active {
            return;
        }
        let download_url = match build_media_download_url(&state.config.rov_http_base, &name) {
            Ok(url) => url,
            Err(err) => {
                state.media.status_text = format!("Cannot build stream URL: {err:#}");
                apply_state_to_ui(&ui, &state);
                return;
            }
        };
        let ffmpeg_bin = if let Some(bin) = locate_ffmpeg_binary() {
            bin
        } else {
            state.media.status_text = "ffmpeg not found. Bundle it as ./bin/ffmpeg.".to_owned();
            apply_state_to_ui(&ui, &state);
            return;
        };
        match spawn_media_stream_pipeline(ffmpeg_bin, download_url) {
            Ok((controller, rx)) => {
                state.media.media_stream_controller = Some(controller);
                state.media.media_stream_event_rx = Some(rx);
                state.media.media_stream_active = true;
                state.media.media_stream_frames = 0;
                state.media.status_text = format!("Streaming {name} from ROV...");
            }
            Err(err) => {
                state.media.status_text = format!("Failed to start media stream: {err:#}");
            }
        }
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_stop_media_stream = Rc::clone(&state);
    ui.on_stop_media_stream(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_stop_media_stream.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        state.media.stop_media_stream();
        state.media.preview_image = None;
        state.media.status_text = "Playback stopped.".to_owned();
        apply_state_to_ui(&ui, &state);
    });
}

/// Polls background media events and updates state accordingly.
/// Returns `true` if the UI needs a refresh.
fn poll_media_events(state: &mut ThirdEyeState, store: &AppStore) -> bool {
    let mut changed = false;
    while let Ok(event) = state.media.event_rx.try_recv() {
        changed = true;
        match event {
            MediaEvent::Download { name, result } => {
                state.media.download_in_progress = false;
                match result {
                    Ok(path) => {
                        state.media.status_text =
                            format!("Downloaded {name} to {}.", path.display());
                    }
                    Err(err) => {
                        state.media.status_text = format!("Download of {name} failed: {err}");
                    }
                }
                refresh_media_rows(state, store);
            }
            MediaEvent::Refresh { status_text } => {
                state.media.refresh_in_progress = false;
                state.media.status_text = status_text;
                refresh_media_rows(state, store);
            }
            MediaEvent::Capture {
                capture_msg,
                attached_text,
            } => {
                state.media.capture_in_progress = false;
                state.rov_info = capture_msg.clone();
                if state.active_screen == Screen::Stream {
                    state.stream.status = capture_msg;
                }
                state.attached_metadata_text = attached_text;
                refresh_media_rows(state, store);
            }
            MediaEvent::Delete { status_text } => {
                state.media.status_text = status_text;
            }
            MediaEvent::ListMedias { rov_info } => {
                state.rov_info = rov_info;
                refresh_media_rows(state, store);
            }
        }
    }
    changed
}

fn stream_stderr_loop(
    mut stderr: ChildStderr,
    stop_flag: Arc<AtomicBool>,
    tx: mpsc::Sender<StreamEvent>,
) {
    let mut read_buffer = [0_u8; 8 * 1024];
    let mut line_buffer = Vec::new();
    while !stop_flag.load(Ordering::Relaxed) {
        match stderr.read(&mut read_buffer) {
            Ok(0) => break,
            Ok(n) => {
                line_buffer.extend_from_slice(&read_buffer[..n]);
                while let Some(pos) = line_buffer.iter().position(|&b| b == b'\n') {
                    let line_bytes = line_buffer.drain(..=pos).collect::<Vec<_>>();
                    if let Ok(line) = String::from_utf8(line_bytes) {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            let _ = tx.send(StreamEvent::Error(format!("ffmpeg: {trimmed}")));
                        }
                    }
                }
            }
            Err(_) => break,
        }
    }
    if !line_buffer.is_empty()
        && let Ok(line) = String::from_utf8(line_buffer)
    {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            let _ = tx.send(StreamEvent::Error(format!("ffmpeg: {trimmed}")));
        }
    }
}

fn spawn_media_stream_pipeline(
    ffmpeg_bin: PathBuf,
    http_url: String,
) -> Result<(StreamController, Receiver<StreamEvent>)> {
    let mut cmd = Command::new(&ffmpeg_bin);
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(&http_url)
        .arg("-vf")
        .arg("fps=15,scale=960:-1")
        .arg("-f")
        .arg("mjpeg")
        .arg("-q:v")
        .arg("6")
        .arg("pipe:1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut ffmpeg_child = cmd
        .spawn()
        .context("failed to spawn ffmpeg for media streaming")?;

    let stdout = ffmpeg_child
        .stdout
        .take()
        .context("failed to capture ffmpeg stdout")?;
    let stderr = ffmpeg_child
        .stderr
        .take()
        .context("failed to capture ffmpeg stderr")?;

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stdout_stop_flag = Arc::clone(&stop_flag);
    let stderr_stop_flag = Arc::clone(&stop_flag);
    let (tx, rx) = mpsc::channel();
    let stdout_tx = tx.clone();
    let stdout_worker = thread::spawn(move || {
        let _ = tx.send(StreamEvent::Status(
            "Media stream started. Waiting for frames...".to_owned(),
        ));
        stream_worker_loop(stdout, stdout_stop_flag, tx);
    });
    let stderr_worker = thread::spawn(move || {
        stream_stderr_loop(stderr, stderr_stop_flag, stdout_tx);
    });

    Ok((
        StreamController {
            stop_flag,
            ffmpeg_child,
            workers: vec![stdout_worker, stderr_worker],
            _proxy_guard: None,
        },
        rx,
    ))
}

fn spawn_stream_pipeline(
    ffmpeg_bin: PathBuf,
    rtsp_url: String,
) -> Result<(StreamController, Receiver<StreamEvent>)> {
    let mut command = Command::new(ffmpeg_bin);
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-rtsp_transport")
        .arg("tcp")
        .arg("-fflags")
        .arg("nobuffer")
        .arg("-flags")
        .arg("low_delay");
    // Note: -localaddr is NOT a valid option for the RTSP demuxer (only for
    // SDP/RTP). On macOS the osascript route+ARP handles interface binding;
    // on Windows the HTTP ARP-ping before this call populates the ARP cache
    // so OS routing directs ffmpeg to the correct adapter automatically.
    command
        .arg("-i")
        .arg(&rtsp_url)
        .arg("-vf")
        .arg("fps=15,scale=960:-1")
        .arg("-f")
        .arg("mjpeg")
        .arg("-q:v")
        .arg("6")
        .arg("pipe:1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut ffmpeg_child = command
        .spawn()
        .context("failed to spawn ffmpeg for embedded stream")?;

    let stdout = ffmpeg_child
        .stdout
        .take()
        .context("failed to capture ffmpeg stdout")?;
    let stderr = ffmpeg_child
        .stderr
        .take()
        .context("failed to capture ffmpeg stderr")?;

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stdout_stop_flag = Arc::clone(&stop_flag);
    let stderr_stop_flag = Arc::clone(&stop_flag);
    let (tx, rx) = mpsc::channel();
    let stdout_tx = tx.clone();
    let stdout_worker = thread::spawn(move || {
        let _ = tx.send(StreamEvent::Status(
            "Streaming started. Waiting for frames...".to_owned(),
        ));
        stream_worker_loop(stdout, stdout_stop_flag, tx);
    });
    let stderr_worker = thread::spawn(move || {
        stream_stderr_loop(stderr, stderr_stop_flag, stdout_tx);
    });

    Ok((
        StreamController {
            stop_flag,
            ffmpeg_child,
            workers: vec![stdout_worker, stderr_worker],
            _proxy_guard: None,
        },
        rx,
    ))
}

fn stream_worker_loop(
    mut stdout: ChildStdout,
    stop_flag: Arc<AtomicBool>,
    tx: mpsc::Sender<StreamEvent>,
) {
    let mut read_buffer = [0_u8; 16 * 1024];
    let mut packet_buffer = Vec::new();
    while !stop_flag.load(Ordering::Relaxed) {
        match stdout.read(&mut read_buffer) {
            Ok(0) => {
                let _ = tx.send(StreamEvent::Ended);
                break;
            }
            Ok(n) => {
                packet_buffer.extend_from_slice(&read_buffer[..n]);
                while let Some(jpeg) = extract_jpeg_frame(&mut packet_buffer) {
                    match decode_jpeg_to_frame(&jpeg) {
                        Ok(frame) => {
                            if tx.send(StreamEvent::Frame(frame)).is_err() {
                                return;
                            }
                        }
                        Err(err) => {
                            let _ =
                                tx.send(StreamEvent::Error(format!("JPEG decode failed: {err:#}")));
                        }
                    }
                }
            }
            Err(err) => {
                let _ = tx.send(StreamEvent::Error(format!(
                    "Failed while reading ffmpeg output: {err}"
                )));
                break;
            }
        }
    }
}

fn extract_jpeg_frame(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let start = buffer.windows(2).position(|pair| pair == [0xFF, 0xD8])?;
    if start > 0 {
        buffer.drain(..start);
    }
    let end_rel = buffer[2..]
        .windows(2)
        .position(|pair| pair == [0xFF, 0xD9])?;
    let end = end_rel + 3;
    let frame = buffer[..=end].to_vec();
    buffer.drain(..=end);
    Some(frame)
}

fn decode_jpeg_to_frame(jpeg: &[u8]) -> Result<RgbaFrame> {
    let image = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
        .context("invalid jpeg frame")?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(RgbaFrame {
        width,
        height,
        rgba: rgba.into_raw(),
    })
}

// -------------------------------------------------------------------------
// OS-level route for external processes (ffmpeg can't use IP_BOUND_IF)
// -------------------------------------------------------------------------

/// Placeholder for the proxy guard; kept for `StreamController` layout.
type TcpProxyGuard = ();

/// Probes the ROV via HTTP (to populate ARP) then ensures the OS-level
/// route exists. Called by `refresh_rov_network` when a wired interface
/// is detected. Returns `Ok(())` when the route is ready, or an error
/// if ARP/route setup failed (e.g. ROV is off).
fn ensure_rov_external_route(rov_http_base: &str, interface: &str) -> Result<()> {
    let client = CameraApiClient::new_bound(rov_http_base.to_owned(), Some(interface));
    let _ = client.list_medias(None::<MediaScene>);
    let host =
        parse_host_from_http_base(rov_http_base).unwrap_or_else(|| "192.168.1.88".to_owned());
    let dummy_rtsp = format!("rtsp://x@{host}:8554/");
    ensure_rov_route_for_rtsp(&dummy_rtsp, interface)
}

/// Removes a stale host route for `rov_host` that may have been created by a
/// previous cable session. Without this, switching from cable to ROV WiFi
/// leaves ffmpeg trying to reach the ROV through the disconnected wired
/// interface.
#[cfg(target_os = "macos")]
fn cleanup_stale_rov_route(rov_host: &str) {
    // Check if there's actually a static host route before prompting for admin.
    let has_route = Command::new("netstat")
        .args(["-rn", "-f", "inet"])
        .output()
        .ok()
        .is_some_and(|output| {
            let text = String::from_utf8_lossy(&output.stdout);
            text.lines().any(|line| {
                line.contains(rov_host)
                    && line
                        .split_whitespace()
                        .nth(2)
                        .is_some_and(|flags| flags.contains('H') && flags.contains('S'))
            })
        });
    if !has_route {
        return;
    }
    let script = format!(
        r#"do shell script "/sbin/route delete -host {rov_host} 2>/dev/null; /usr/sbin/arp -d {rov_host} 2>/dev/null" with administrator privileges"#
    );
    let _ = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(target_os = "macos"))]
fn cleanup_stale_rov_route(_rov_host: &str) {}

/// Sets up an OS-level host route + ARP entry so that ffmpeg's TCP connections
/// to the ROV go through the correct network interface.
///
/// This is needed because ffmpeg is an external process and we can't set
/// `IP_BOUND_IF` on its sockets. On macOS this uses `osascript` to request
/// admin privileges with a native password dialog.
#[cfg(target_os = "macos")]
fn run_rov_route_osascript(rov_host: &str, interface: &str, rov_mac: &str) -> Result<()> {
    let script = format!(
        r#"do shell script "
/sbin/route delete -host {rov_host} 2>/dev/null; 
/sbin/route add -host {rov_host} -interface {interface}; 
/usr/sbin/arp -d {rov_host} 2>/dev/null; 
/usr/sbin/arp -s {rov_host} {rov_mac} ifscope {interface}
" with administrator privileges"#
    );

    let status = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .context("failed to run osascript for route setup")?;

    if !status.success() {
        anyhow::bail!("route setup via osascript failed (status {status})");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn resolve_rov_host_and_mac(rtsp_url: &str, interface: &str) -> Result<(String, String)> {
    let parsed = Url::parse(rtsp_url).context("invalid RTSP URL")?;
    let rov_host = parsed
        .host_str()
        .context("RTSP URL has no host")?
        .to_owned();
    let rov_mac = read_arp_mac_on_interface(&rov_host, interface)
        .context("ROV MAC not found in ARP table. Make an HTTP request first so the app populates the ARP entry via IP_BOUND_IF.")?;
    Ok((rov_host, rov_mac))
}

#[cfg(target_os = "macos")]
fn ensure_rov_route_for_rtsp(rtsp_url: &str, interface: &str) -> Result<()> {
    let (rov_host, rov_mac) = resolve_rov_host_and_mac(rtsp_url, interface)?;
    if has_valid_rov_route(&rov_host, interface) {
        return Ok(());
    }
    run_rov_route_osascript(&rov_host, interface, &rov_mac)
}

#[cfg(not(target_os = "macos"))]
fn ensure_rov_route_for_rtsp(_rtsp_url: &str, _interface: &str) -> Result<()> {
    Ok(())
}

/// Checks whether a valid host route
/// on the specified interface.
fn has_valid_rov_route(host: &str, interface: &str) -> bool {
    // Check ARP: must have a real MAC (not incomplete, not adapter's own MAC)
    // on the correct interface.
    let adapter_mac = get_interface_mac(interface).unwrap_or_default();
    if let Some(mac) = read_arp_mac_on_interface(host, interface) {
        // Also check the route table has a host entry on our interface.
        if mac != adapter_mac && has_host_route(host, interface) {
            return true;
        }
    }
    false
}

/// Checks if a **non-scoped** host route for `host` exists on `interface`.
///
/// ARP-cache entries show up as `UHLSI` (scoped) and don't override the subnet
/// route for processes that don't use `IP_BOUND_IF`. We need `UHLS` (no `I`)
/// created by `route add -host -interface`.
fn has_host_route(host: &str, interface: &str) -> bool {
    let output = Command::new("netstat")
        .args(["-rn", "-f", "inet"])
        .output()
        .ok();
    let Some(output) = output else { return false };
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().any(|line| {
        if !line.contains(host) || !line.contains(interface) {
            return false;
        }
        // Extract the flags column (typically the 3rd whitespace-delimited field).
        let flags = line.split_whitespace().nth(2).unwrap_or("");
        // Must be a host route (H), static (S), and NOT interface-scoped (no I).
        flags.contains('H') && flags.contains('S') && !flags.contains('I')
    })
}

/// Returns the MAC address of a network interface (e.g. en10's own MAC).
fn get_interface_mac(interface: &str) -> Option<String> {
    let output = Command::new("ifconfig").arg(interface).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("ether ") {
            return trimmed.strip_prefix("ether ").map(|s| s.trim().to_owned());
        }
    }
    None
}

/// Reads the MAC address for `host` from the ARP table, filtered to entries
/// on the specified interface.
fn read_arp_mac_on_interface(host: &str, interface: &str) -> Option<String> {
    let output = Command::new("arp").arg("-an").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let host_pattern = format!("({host})");
    for line in text.lines() {
        if !line.contains(&host_pattern) || !line.contains(interface) {
            continue;
        }
        if let Some(at_pos) = line.find(" at ") {
            let after_at = &line[at_pos + 4..];
            if let Some(mac) = after_at.split_whitespace().next()
                && mac.contains(':')
                && mac != "(incomplete)"
            {
                return Some(mac.to_owned());
            }
        }
    }
    None
}

fn locate_ffmpeg_binary() -> Option<PathBuf> {
    let exe_name = if cfg!(target_os = "windows") {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("bin").join(exe_name));
        candidates.push(dir.join(exe_name));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("bin").join(exe_name));
        candidates.push(cwd.join(exe_name));
    }

    candidates
        .into_iter()
        .find(|path| path.exists())
        .or_else(|| Some(PathBuf::from(exe_name)))
}

fn main() -> Result<()> {
    let ui = AppWindow::new().context("failed to initialize Slint window")?;
    ui.window().set_maximized(true);
    let store = Rc::new(match AppStore::open() {
        Ok(store) => store,
        Err(err) => {
            eprintln!(
                "third-eye-client: failed to open persistent storage ({err:#}); falling back to in-memory store"
            );
            AppStore::open_in_memory().context("opening in-memory fallback AppStore")?
        }
    });
    let state = Rc::new(RefCell::new(ThirdEyeState::new(&store)));
    // Warm up location services in the background so the map can auto-centre
    // without blocking the UI or requiring an explicit user action.
    //
    // macOS  – CoreLocation must be initialised on the main thread (framework
    //           requirement). Permission is requested here (non-blocking native
    //           dialog); the fix is delivered via the run loop and picked up by
    //           the 16 ms poll timer once ui.run() starts.
    //
    // Windows – the blocking GPS call runs in a background thread; the result
    //            is forwarded to the UI timer via an mpsc channel.
    //
    // Linux / others – no native GPS source; nothing to warm up.
    #[cfg(target_os = "macos")]
    {
        let mut s = state.borrow_mut();
        prime_corelocation_at_startup(&mut s.map);
    }
    #[cfg(target_os = "windows")]
    {
        let (loc_tx, loc_rx) = mpsc::channel::<Result<(f64, f64), String>>();
        thread::spawn(move || {
            // Two-thread wrapper so we can cap the total wait and avoid an
            // ever-running thread if the GPS hardware never delivers a fix.
            let (inner_tx, inner_rx) = mpsc::channel();
            thread::spawn(move || {
                let r = map::detect_location_from_windows_location_blocking()
                    .map_err(|e| format!("{e:#}"));
                let _ = inner_tx.send(r);
            });
            let result = inner_rx
                .recv_timeout(Duration::from_secs(30))
                .unwrap_or_else(|_| Err("GPS warmup timed out after 30 s".to_owned()));
            let _ = loc_tx.send(result);
        });
        state.borrow_mut().startup_location_rx = Some(loc_rx);
    }
    // Auto-detect ROV network interface at startup (passive ifconfig scan).
    {
        let mut s = state.borrow_mut();
        refresh_rov_network(&mut s, false);
        persist_config(&s, &store);
    }

    {
        let state = state.borrow();
        apply_state_to_ui(&ui, &state);
    }

    register_callbacks(&ui, Rc::clone(&state), Rc::clone(&store));

    let ui_weak = ui.as_weak();
    let poll_state = Rc::clone(&state);
    let poll_store = Rc::clone(&store);
    let stream_poll_timer = slint::Timer::default();
    stream_poll_timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(16),
        move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let mut state = match poll_state.try_borrow_mut() {
                Ok(state) => state,
                Err(_) => return,
            };
            // Tear down stream + telemetry after 10 min away from the stream screen.
            if state.stream_left_at_ms > 0 && current_unix_ms() - state.stream_left_at_ms > 600_000
            {
                state.stream.stop();
                state.rov_status.stop();
                state.stream_left_at_ms = 0;
            }
            if let Some(frame) = state.stream.poll_events() {
                ui.set_stream_image(rgba_frame_to_slint_image(&frame));
                ui.set_has_stream_image(true);
            }
            // Poll media playback stream (MP4 streaming from ROV).
            if let Some(frame) = state.media.poll_media_stream() {
                let img = rgba_frame_to_slint_image(&frame);
                state.media.preview_image = Some(img.clone());
                ui.set_media_preview_image(img);
                ui.set_has_media_preview(true);
            }
            ui.set_media_stream_active(state.media.media_stream_active);
            let current_zoom = state.map.zoom;
            let (map_changed, map_error) = state.map_tiles.poll_loaded_tiles(current_zoom);
            let has_map_error = map_error.is_some();
            if let Some(error) = map_error {
                state.map.status = error;
                state.request_visible_map_tiles();
            }
            let anim_active = state.viewport_anim.is_some();
            if let Some(anim) = &mut state.viewport_anim {
                anim.elapsed_ms += 16.0;
                if anim.elapsed_ms >= anim.duration_ms {
                    state.viewport_anim = None;
                }
            }
            if map_changed || has_map_error || anim_active {
                apply_map_runtime_to_ui(&ui, &state);
            }
            state.rov_status.poll_events();
            // Poll NMEA GPS: update map location when a fix arrives.
            if state.nmea_gps.poll_events()
                && let Some((lat, lon)) = state.nmea_gps.latest_location()
            {
                state.map.lat = Some(lat);
                state.map.lon = Some(lon);
                state.location_detected_at_ms = current_unix_ms();
            }
            // Apply background location warmup result.
            // Only applied if no location has been set yet (user may have
            // already detected one manually or via NMEA GPS).
            //
            // macOS: poll CoreLocation's cached property which is updated by
            //        the run loop after startUpdatingLocation() was called at
            //        startup.
            // Windows: drain the background-thread channel.
            #[cfg(target_os = "macos")]
            if state.location_detected_at_ms == 0 {
                let fix = check_corelocation_warmup_fix(&state.map);
                if let Some((lat, lon)) = fix {
                    state.map.lat = Some(lat);
                    state.map.lon = Some(lon);
                    state.location_detected_at_ms = current_unix_ms();
                    if state.active_screen == Screen::Map {
                        state.load_map_tile_for_current_location(
                            "Location detected (CoreLocation).".to_owned(),
                        );
                        apply_map_runtime_to_ui(&ui, &state);
                    }
                }
            }
            #[cfg(target_os = "windows")]
            {
                let warmup_fix = if let Some(rx) = &state.startup_location_rx {
                    rx.try_recv().ok()
                } else {
                    None
                };
                if let Some(result) = warmup_fix {
                    state.startup_location_rx = None;
                    if let Ok((lat, lon)) = result {
                        if state.location_detected_at_ms == 0 {
                            state.map.lat = Some(lat);
                            state.map.lon = Some(lon);
                            state.location_detected_at_ms = current_unix_ms();
                            if state.active_screen == Screen::Map {
                                state.load_map_tile_for_current_location(
                                    "Location detected (Windows GPS).".to_owned(),
                                );
                                apply_map_runtime_to_ui(&ui, &state);
                            }
                        }
                    }
                }
            }
            ui.set_nmea_gps_status(state.nmea_gps.status_text().to_owned().into());
            ui.set_nmea_gps_running(state.nmea_gps.is_running());
            ui.set_nmea_has_fix(state.nmea_gps.fixes_received() > 0);
            apply_stream_and_rov_runtime_to_ui(&ui, &state);
            if poll_media_events(&mut state, &poll_store) {
                apply_state_to_ui(&ui, &state);
            }
        },
    );

    ui.run()
        .map_err(|err| anyhow::anyhow!("failed to run GUI app: {err}"))?;

    if let Ok(mut state) = state.try_borrow_mut() {
        state.stream.stop();
        state.media.stop_media_stream();
        state.rov_status.stop();
        state.nmea_gps.stop();
    }
    store.shutdown();

    Ok(())
}
