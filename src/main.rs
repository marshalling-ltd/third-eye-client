use std::f64::consts::PI;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};
use eframe::egui::{self, ColorImage, TextureHandle};
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2_core_location::{CLAuthorizationStatus, CLLocationManager, kCLLocationAccuracyBest};
use reqwest::blocking::Client;
use serde_json::Value;

const DEFAULT_TEST_RTSP: &str = "rtsp://wowzaec2demo.streamlock.net/vod/mp4:BigBuckBunny_115k.mov";
const DEFAULT_ROV_RTSP: &str = "rtsp://admin:admin@192.168.1.88:8554/stream/0/0";
const DEFAULT_ROV_HTTP_BASE: &str = "http://192.168.1.88";
const DEFAULT_ZOOM: u32 = 14;
const MIN_ZOOM: u32 = 1;
const MAX_ZOOM: u32 = 19;
const MAP_IMAGE_SIZE_PX: u32 = 768;
#[cfg(target_os = "macos")]
const CORELOCATION_FIX_POLL_ATTEMPTS: u32 = 8;
#[cfg(target_os = "macos")]
const CORELOCATION_FIX_POLL_INTERVAL_MS: u64 = 250;
const DEFAULT_OSM_TILE_USER_AGENT: &str =
    "third-eye-client/0.1 (desktop map viewer; set contact URL/email for production use)";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Configuration,
    Map,
    Stream,
}

#[derive(Clone)]
struct AppConfig {
    rtsp_url: String,
    rov_http_base: String,
    osm_tile_user_agent: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            rtsp_url: DEFAULT_TEST_RTSP.to_owned(),
            rov_http_base: DEFAULT_ROV_HTTP_BASE.to_owned(),
            osm_tile_user_agent: DEFAULT_OSM_TILE_USER_AGENT.to_owned(),
        }
    }
}

fn stream_stderr_loop(
    mut stderr: ChildStderr,
    stop_flag: Arc<AtomicBool>,
    tx: mpsc::Sender<StreamEvent>,
) {
    let mut read_buffer = [0_u8; 8 * 1024];
    let mut collected = Vec::new();
    while !stop_flag.load(Ordering::Relaxed) {
        match stderr.read(&mut read_buffer) {
            Ok(0) => break,
            Ok(n) => collected.extend_from_slice(&read_buffer[..n]),
            Err(_) => break,
        }
    }

    if stop_flag.load(Ordering::Relaxed) || collected.is_empty() {
        return;
    }

    if let Ok(stderr_text) = String::from_utf8(collected)
        && let Some(last_line) = stderr_text
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
    {
        let _ = tx.send(StreamEvent::Error(format!("ffmpeg: {last_line}")));
    }
}

#[derive(Default)]
struct MapState {
    lat: Option<f64>,
    lon: Option<f64>,
    zoom: u32,
    status: String,
    texture: Option<TextureHandle>,
    pending_texture_image: Option<ColorImage>,
    #[cfg(target_os = "macos")]
    corelocation_manager: Option<Retained<CLLocationManager>>,
    #[cfg(target_os = "macos")]
    corelocation_permission_requested: bool,
}
struct DetectedLocation {
    lat: f64,
    lon: f64,
    source: String,
}

#[cfg(target_os = "macos")]
enum CoreLocationDetectionOutcome {
    Located(f64, f64),
    PendingPermission(String),
    PendingFix(String),
}

struct ThirdEyeApp {
    active_screen: Screen,
    last_screen: Screen,
    config: AppConfig,
    map: MapState,
    rov_info: String,
    stream: StreamState,
}

impl ThirdEyeApp {
    fn new() -> Self {
        let mut app = Self {
            active_screen: Screen::Configuration,
            last_screen: Screen::Configuration,
            config: AppConfig::default(),
            map: MapState {
                zoom: DEFAULT_ZOOM,
                ..MapState::default()
            },
            rov_info: String::new(),
            stream: StreamState::default(),
        };
        app.initialize_location_on_startup();
        app
    }

    fn initialize_location_on_startup(&mut self) {
        match detect_location(&mut self.map) {
            Ok(location) => {
                self.map.lat = Some(location.lat);
                self.map.lon = Some(location.lon);
                let success_message = format!(
                    "Startup location via {}: lat={:.6}, lon={:.6}.",
                    location.source, location.lat, location.lon
                );
                self.load_map_tile_for_current_location(format!(
                    "{success_message} Map tile auto-loaded."
                ));
            }
            Err(err) => {
                self.map.status = format!("Startup location detection failed: {err:#}");
            }
        }
    }

    fn load_map_tile_for_current_location(&mut self, success_status: String) {
        match (self.map.lat, self.map.lon) {
            (Some(lat), Some(lon)) => {
                match fetch_map_tile(lat, lon, self.map.zoom, &self.config.osm_tile_user_agent) {
                    Ok(image) => {
                        self.map.pending_texture_image = Some(image);
                        self.map.status = success_status;
                    }
                    Err(err) => {
                        self.map.texture = None;
                        self.map.pending_texture_image = None;
                        self.map.status = format!(
                            "Failed to load OSM tile: {err:#}. Check OSM tile policy compliance (User-Agent, rate limits, caching) or use browser mode."
                        );
                    }
                }
            }
            _ => {
                self.map.status = "No location set. Use Detect location first.".to_owned();
            }
        }
    }

    fn auto_refresh_map_on_tab_enter(&mut self) {
        match detect_location(&mut self.map) {
            Ok(location) => {
                self.map.lat = Some(location.lat);
                self.map.lon = Some(location.lon);
                self.load_map_tile_for_current_location(format!(
                    "Auto-refreshed map on entering Device Map tab via {}: lat={:.6}, lon={:.6}.",
                    location.source, location.lat, location.lon
                ));
            }
            Err(err) => {
                if self.map.lat.is_some() && self.map.lon.is_some() {
                    self.load_map_tile_for_current_location(format!(
                        "Auto-refreshed map using last known location (new detection unavailable: {err:#})."
                    ));
                } else {
                    self.map.status = format!("Auto-refresh on tab enter failed: {err:#}");
                }
            }
        }
    }
    fn materialize_pending_map_texture(&mut self, ctx: &egui::Context) {
        if let Some(image) = self.map.pending_texture_image.take() {
            self.map.texture =
                Some(ctx.load_texture("osm_tile", image, egui::TextureOptions::LINEAR));
        }
    }

    fn top_menu(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(
                &mut self.active_screen,
                Screen::Configuration,
                "1) Configuration",
            );
            ui.selectable_value(&mut self.active_screen, Screen::Map, "2) Device Map");
            ui.selectable_value(&mut self.active_screen, Screen::Stream, "3) Live Stream");
        });
        ui.separator();
    }

    fn show_configuration_screen(&mut self, ui: &mut egui::Ui) {
        ui.heading("RTSP + ROV Configuration");
        ui.label("Set RTSP URLs and ROV HTTP endpoint. These values are used by the Stream and API actions.");
        ui.separator();

        ui.label("RTSP URL:");
        ui.text_edit_singleline(&mut self.config.rtsp_url);
        if ui.button("Use default test RTSP URL").clicked() {
            self.config.rtsp_url = DEFAULT_TEST_RTSP.to_owned();
        }
        if ui.button("Use default ROV RTSP URL").clicked() {
            self.config.rtsp_url = DEFAULT_ROV_RTSP.to_owned();
        }

        ui.separator();
        ui.label("ROV HTTP API Base URL:");
        ui.text_edit_singleline(&mut self.config.rov_http_base);
        if ui.button("Use default ROV HTTP API URL").clicked() {
            self.config.rov_http_base = DEFAULT_ROV_HTTP_BASE.to_owned();
        }
        ui.separator();
        ui.label("OpenStreetMap tile User-Agent:");
        ui.text_edit_singleline(&mut self.config.osm_tile_user_agent);
        if ui.button("Use default OSM tile User-Agent").clicked() {
            self.config.osm_tile_user_agent = DEFAULT_OSM_TILE_USER_AGENT.to_owned();
        }
        ui.label("Include an app identifier and contact URL/email for OSM tile policy compliance.");

        ui.separator();
        ui.collapsing("ROV API notes", |ui| {
            ui.label("• RTSP stream example: rtsp://admin:admin@192.168.1.88:8554/stream/0/0");
            ui.label("• HTTP API server example: http://192.168.1.88:80");
            ui.label("• Use HTTP status 2xx for success checks (avoid relying on error codes).");
            ui.label("• Capture endpoint: POST /v1/capture");
            ui.label("• Media list endpoint: GET /v1/medias");
            ui.label("• Download endpoint: GET /v1/medias/{name}/download");
        });

        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui.button("List medias (GET /v1/medias)").clicked() {
                let client = RovApiClient::new(self.config.rov_http_base.clone());
                match client.list_medias() {
                    Ok(names) => {
                        self.rov_info = if names.is_empty() {
                            "No media names detected in response.".to_owned()
                        } else {
                            format!("Media files:\n{}", names.join("\n"))
                        };
                    }
                    Err(err) => {
                        self.rov_info = format!("List medias failed: {err:#}");
                    }
                }
            }

            if ui.button("Capture photo (POST /v1/capture)").clicked() {
                let client = RovApiClient::new(self.config.rov_http_base.clone());
                match client.capture() {
                    Ok(()) => {
                        self.rov_info = "Capture request sent successfully (HTTP 2xx).".to_owned();
                    }
                    Err(err) => {
                        self.rov_info = format!("Capture failed: {err:#}");
                    }
                }
            }
        });

        if !self.rov_info.is_empty() {
            ui.separator();
            ui.label(&self.rov_info);
        }
    }

    fn show_map_screen(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Device Location on OpenStreetMap");
        ui.label(
            "This desktop app uses native location when available, with IP geolocation fallback.",
        );
        ui.separator();

        ui.horizontal_wrapped(|ui| {
            if ui.button("Detect location").clicked() {
                match detect_location(&mut self.map) {
                    Ok(location) => {
                        self.map.lat = Some(location.lat);
                        self.map.lon = Some(location.lon);
                        let success_message = format!(
                            "Detected location via {}: lat={:.6}, lon={:.6}",
                            location.source, location.lat, location.lon
                        );
                        self.load_map_tile_for_current_location(format!(
                            "{success_message}. Map auto-refreshed."
                        ));
                    }
                    Err(err) => {
                        self.map.status = format!("Failed to detect location: {err:#}");
                    }
                }
                ctx.request_repaint();
            }

            if ui.button("Load OSM map tile").clicked() {
                self.load_map_tile_for_current_location(
                    "Loaded OpenStreetMap tile for detected location.".to_owned(),
                );
            }
            ui.separator();
            ui.label(format!("Zoom: {}", self.map.zoom));
            if ui.button("Zoom out").clicked() {
                let next_zoom = self.map.zoom.saturating_sub(1).max(MIN_ZOOM);
                if next_zoom != self.map.zoom {
                    self.map.zoom = next_zoom;
                    self.load_map_tile_for_current_location(format!(
                        "Zoomed out to {} and refreshed map tile.",
                        self.map.zoom
                    ));
                }
            }
            if ui.button("Zoom in").clicked() {
                let next_zoom = self.map.zoom.saturating_add(1).min(MAX_ZOOM);
                if next_zoom != self.map.zoom {
                    self.map.zoom = next_zoom;
                    self.load_map_tile_for_current_location(format!(
                        "Zoomed in to {} and refreshed map tile.",
                        self.map.zoom
                    ));
                }
            }

            if ui.button("Open interactive map in browser").clicked() {
                match (self.map.lat, self.map.lon) {
                    (Some(lat), Some(lon)) => {
                        let url = format!(
                            "https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map={}/{lat}/{lon}",
                            self.map.zoom
                        );
                        match webbrowser::open(&url) {
                            Ok(()) => {
                                self.map.status = "Opened map in browser.".to_owned();
                            }
                            Err(err) => {
                                self.map.status =
                                    format!("Failed to open browser map: {err:#}");
                            }
                        }
                    }
                    _ => {
                        self.map.status =
                            "No location set. Use Detect location first.".to_owned();
                    }
                }
            }
        });

        #[cfg(target_os = "macos")]
        ui.monospace(corelocation_debug_status(&self.map));

        if !self.map.status.is_empty() {
            ui.separator();
            ui.label(&self.map.status);
        }

        if let Some(texture) = &self.map.texture {
            ui.separator();
            let available = ui.available_size();
            let size = egui::vec2(available.x.max(100.0), available.y.max(100.0));
            ui.horizontal_centered(|ui| {
                let response = ui.add(
                    egui::Image::new((texture.id(), texture.size_vec2())).fit_to_exact_size(size),
                );
                let center = response.rect.center();
                ui.painter()
                    .circle_filled(center, 7.0, egui::Color32::from_rgb(220, 20, 60));
                ui.painter().circle_stroke(
                    center,
                    13.0,
                    egui::Stroke::new(2.0, egui::Color32::WHITE),
                );
            });
        }
    }

    fn show_stream_screen(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("RTSP Live Stream");
        ui.label("Current stream URL (shared from configuration screen):");
        ui.monospace(&self.config.rtsp_url);
        ui.separator();
        ui.label("Embedded stream is decoded with ffmpeg and rendered directly in this window.");

        ui.horizontal_wrapped(|ui| {
            if ui.button("Start embedded stream").clicked() {
                self.stream.stop();
                self.stream.status = match self.stream.start(self.config.rtsp_url.clone()) {
                    Ok(msg) => msg,
                    Err(err) => format!("Failed to start stream: {err:#}"),
                };
            }

            if ui.button("Stop stream").clicked() {
                self.stream.stop();
            }
        });

        ui.separator();
        ui.label(&self.stream.status);
        ui.label(format!("Frames received: {}", self.stream.frames_received));

        if self.stream.is_running() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }

        if let Some(texture) = &self.stream.texture {
            ui.separator();
            let available = ui.available_size();
            let mut size = texture.size_vec2();
            if size.x > available.x || size.y > available.y {
                let scale = (available.x / size.x).min(available.y / size.y).max(0.1);
                size *= scale;
            }
            ui.image((texture.id(), size));
        } else {
            ui.separator();
            ui.label("No frames rendered yet.");
        }
    }
}

impl eframe::App for ThirdEyeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.stream.poll_events(ctx);
        self.materialize_pending_map_texture(ctx);
        egui::TopBottomPanel::top("top_menu").show(ctx, |ui| {
            self.top_menu(ui);
        });
        if self.active_screen == Screen::Map && self.last_screen != Screen::Map {
            self.auto_refresh_map_on_tab_enter();
            self.materialize_pending_map_texture(ctx);
        }
        self.last_screen = self.active_screen;

        egui::CentralPanel::default().show(ctx, |ui| match self.active_screen {
            Screen::Configuration => self.show_configuration_screen(ui),
            Screen::Map => self.show_map_screen(ui, ctx),
            Screen::Stream => self.show_stream_screen(ui, ctx),
        });
    }
}

impl Drop for ThirdEyeApp {
    fn drop(&mut self) {
        self.stream.stop();
    }
}

#[derive(Default)]
struct StreamState {
    texture: Option<TextureHandle>,
    event_rx: Option<Receiver<StreamEvent>>,
    controller: Option<StreamController>,
    status: String,
    frames_received: u64,
}

impl StreamState {
    fn start(&mut self, rtsp_url: String) -> Result<String> {
        let ffmpeg_bin = locate_ffmpeg_binary().context(
            "ffmpeg binary not found. Bundle it as ./bin/ffmpeg beside the app executable.",
        )?;
        let ffmpeg_label = ffmpeg_bin.display().to_string();
        let (controller, rx) = spawn_stream_pipeline(ffmpeg_bin, rtsp_url)?;
        self.event_rx = Some(rx);
        self.controller = Some(controller);
        self.frames_received = 0;
        self.texture = None;
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

    fn is_running(&self) -> bool {
        self.controller.is_some()
    }

    fn poll_events(&mut self, ctx: &egui::Context) {
        let mut disconnected = false;
        if let Some(rx) = &self.event_rx {
            loop {
                match rx.try_recv() {
                    Ok(StreamEvent::Frame(frame)) => {
                        let image = ColorImage::from_rgba_unmultiplied(
                            [frame.width, frame.height],
                            &frame.rgba,
                        );
                        if let Some(texture) = &mut self.texture {
                            texture.set(image, egui::TextureOptions::LINEAR);
                        } else {
                            self.texture = Some(ctx.load_texture(
                                "embedded_rtsp",
                                image,
                                egui::TextureOptions::LINEAR,
                            ));
                        }
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
    }
}

struct StreamController {
    stop_flag: Arc<AtomicBool>,
    ffmpeg_child: Child,
    workers: Vec<JoinHandle<()>>,
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

struct FrameMessage {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

enum StreamEvent {
    Frame(FrameMessage),
    Status(String),
    Error(String),
    Ended,
}

fn spawn_stream_pipeline(
    ffmpeg_bin: PathBuf,
    rtsp_url: String,
) -> Result<(StreamController, Receiver<StreamEvent>)> {
    let mut ffmpeg_child = Command::new(ffmpeg_bin)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-rtsp_transport")
        .arg("tcp")
        .arg("-fflags")
        .arg("nobuffer")
        .arg("-flags")
        .arg("low_delay")
        .arg("-rw_timeout")
        .arg("10000000")
        .arg("-i")
        .arg(rtsp_url)
        .arg("-vf")
        .arg("fps=15,scale=960:-1")
        .arg("-f")
        .arg("mjpeg")
        .arg("-q:v")
        .arg("6")
        .arg("pipe:1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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

fn decode_jpeg_to_frame(jpeg: &[u8]) -> Result<FrameMessage> {
    let image = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
        .context("invalid jpeg frame")?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(FrameMessage {
        width: width as usize,
        height: height as usize,
        rgba: rgba.into_raw(),
    })
}

fn locate_ffmpeg_binary() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("bin/ffmpeg"));
        candidates.push(dir.join("ffmpeg"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("bin/ffmpeg"));
        candidates.push(cwd.join("ffmpeg"));
    }

    candidates
        .into_iter()
        .find(|path| path.exists())
        .or_else(|| Some(PathBuf::from("ffmpeg")))
}

struct RovApiClient {
    base_url: String,
    http: Client,
}

impl RovApiClient {
    fn new(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            http: Client::new(),
        }
    }

    fn capture(&self) -> Result<()> {
        let url = format!("{}/v1/capture", self.base_url);
        let response = self
            .http
            .post(url)
            .send()
            .context("capture request failed")?;
        if response.status().is_success() {
            Ok(())
        } else {
            anyhow::bail!("capture failed with HTTP {}", response.status())
        }
    }

    fn list_medias(&self) -> Result<Vec<String>> {
        let url = format!("{}/v1/medias", self.base_url);
        let response = self
            .http
            .get(url)
            .send()
            .context("list medias request failed")?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("list medias failed with HTTP {status}");
        }
        let payload: Value = response.json().context("invalid medias JSON payload")?;
        Ok(extract_media_names(&payload))
    }
}

fn extract_media_names(payload: &Value) -> Vec<String> {
    fn from_obj(value: &Value) -> Option<String> {
        value
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                value
                    .get("file_name")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .or_else(|| {
                value
                    .get("filename")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
    }

    match payload {
        Value::Array(items) => items.iter().filter_map(from_obj).collect(),
        Value::Object(obj) => {
            if let Some(Value::Array(items)) = obj.get("data") {
                items.iter().filter_map(from_obj).collect()
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn detect_location_from_ip() -> Result<(f64, f64)> {
    let response = Client::new()
        .get("http://ip-api.com/json")
        .send()
        .context("IP geolocation request failed")?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("IP geolocation failed with HTTP {status}");
    }
    let payload: Value = response.json().context("invalid location payload")?;
    let lat = payload
        .get("lat")
        .and_then(Value::as_f64)
        .context("missing lat in location payload")?;
    let lon = payload
        .get("lon")
        .and_then(Value::as_f64)
        .context("missing lon in location payload")?;
    Ok((lat, lon))
}

fn detect_location(map: &mut MapState) -> Result<DetectedLocation> {
    #[cfg(target_os = "macos")]
    {
        match detect_location_from_corelocation(map) {
            Ok(CoreLocationDetectionOutcome::Located(lat, lon)) => Ok(DetectedLocation {
                lat,
                lon,
                source: "macOS CoreLocation (native)".to_owned(),
            }),
            Ok(CoreLocationDetectionOutcome::PendingPermission(message)) => {
                let (lat, lon) = detect_location_from_ip().with_context(|| {
                    format!("CoreLocation permission is pending ({message}) and IP fallback failed")
                })?;
                Ok(DetectedLocation {
                    lat,
                    lon,
                    source: format!("IP geolocation fallback ({message})"),
                })
            }
            Ok(CoreLocationDetectionOutcome::PendingFix(message)) => anyhow::bail!(
                "Native CoreLocation is authorized but still acquiring a fix ({message}). Try Detect location again in a moment."
            ),
            Err(native_err) => {
                let (lat, lon) = detect_location_from_ip().with_context(|| {
                    format!("CoreLocation failed ({native_err:#}) and IP fallback also failed")
                })?;
                Ok(DetectedLocation {
                    lat,
                    lon,
                    source: format!("IP geolocation fallback ({native_err:#})"),
                })
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = map;
        let (lat, lon) = detect_location_from_ip()?;
        Ok(DetectedLocation {
            lat,
            lon,
            source: "IP geolocation".to_owned(),
        })
    }
}

#[cfg(target_os = "macos")]
fn corelocation_status_label(status: CLAuthorizationStatus) -> &'static str {
    if status == CLAuthorizationStatus::kCLAuthorizationStatusNotDetermined {
        "NotDetermined"
    } else if status == CLAuthorizationStatus::kCLAuthorizationStatusDenied {
        "Denied"
    } else if status == CLAuthorizationStatus::kCLAuthorizationStatusRestricted {
        "Restricted"
    } else if status == CLAuthorizationStatus::kCLAuthorizationStatusAuthorizedWhenInUse {
        "AuthorizedWhenInUse"
    } else if status == CLAuthorizationStatus::kCLAuthorizationStatusAuthorizedAlways {
        "AuthorizedAlways"
    } else {
        "Unknown"
    }
}

#[cfg(target_os = "macos")]
fn corelocation_debug_status(map: &MapState) -> String {
    unsafe {
        let services_enabled = CLLocationManager::locationServicesEnabled_class();
        let (status_raw, status_label) = if let Some(manager) = map.corelocation_manager.as_ref() {
            let status = manager.authorizationStatus();
            (status.0, corelocation_status_label(status))
        } else {
            (-1, "ManagerNotInitialized")
        };
        format!(
            "CoreLocation debug: services_enabled={services_enabled}, manager_initialized={}, permission_requested={}, auth_status={status_label} ({status_raw})",
            map.corelocation_manager.is_some(),
            map.corelocation_permission_requested
        )
    }
}

#[cfg(target_os = "macos")]
fn detect_location_from_corelocation(map: &mut MapState) -> Result<CoreLocationDetectionOutcome> {
    fn valid_coordinate(lat: f64, lon: f64) -> bool {
        lat.is_finite()
            && lon.is_finite()
            && (-90.0..=90.0).contains(&lat)
            && (-180.0..=180.0).contains(&lon)
    }

    unsafe {
        if !CLLocationManager::locationServicesEnabled_class() {
            anyhow::bail!("CoreLocation services are disabled");
        }
        if map.corelocation_manager.is_none() {
            map.corelocation_manager = Some(CLLocationManager::new());
        }
        let manager = map
            .corelocation_manager
            .as_ref()
            .context("failed to initialize CoreLocation manager")?;
        manager.setDesiredAccuracy(kCLLocationAccuracyBest);
        let status = manager.authorizationStatus();

        if status == CLAuthorizationStatus::kCLAuthorizationStatusNotDetermined {
            if !map.corelocation_permission_requested {
                manager.requestWhenInUseAuthorization();
                map.corelocation_permission_requested = true;
                return Ok(CoreLocationDetectionOutcome::PendingPermission(
                    "Requested native location permission. Approve the macOS prompt, then click Detect location again.".to_owned(),
                ));
            }
            return Ok(CoreLocationDetectionOutcome::PendingPermission(
                "Waiting for native location permission response. If no prompt appears, focus the app and click Detect location again.".to_owned(),
            ));
        }
        map.corelocation_permission_requested = false;

        if status == CLAuthorizationStatus::kCLAuthorizationStatusDenied
            || status == CLAuthorizationStatus::kCLAuthorizationStatusRestricted
        {
            anyhow::bail!("CoreLocation permission is denied or restricted");
        }

        if let Some(location) = manager.location() {
            let coordinate = location.coordinate();
            if valid_coordinate(coordinate.latitude, coordinate.longitude) {
                manager.stopUpdatingLocation();
                return Ok(CoreLocationDetectionOutcome::Located(
                    coordinate.latitude,
                    coordinate.longitude,
                ));
            }
        }

        if status != CLAuthorizationStatus::kCLAuthorizationStatusAuthorizedAlways
            && status != CLAuthorizationStatus::kCLAuthorizationStatusAuthorizedWhenInUse
        {
            anyhow::bail!("CoreLocation is not authorized for this app");
        }

        manager.startUpdatingLocation();
        manager.requestLocation();
        for _ in 0..CORELOCATION_FIX_POLL_ATTEMPTS {
            if let Some(location) = manager.location() {
                let coordinate = location.coordinate();
                if valid_coordinate(coordinate.latitude, coordinate.longitude) {
                    manager.stopUpdatingLocation();
                    return Ok(CoreLocationDetectionOutcome::Located(
                        coordinate.latitude,
                        coordinate.longitude,
                    ));
                }
            }
            thread::sleep(Duration::from_millis(CORELOCATION_FIX_POLL_INTERVAL_MS));
        }
    }

    Ok(CoreLocationDetectionOutcome::PendingFix(
        "Waiting for native location fix. Click Detect location again in a moment.".to_owned(),
    ))
}

fn fetch_map_tile(lat: f64, lon: f64, zoom: u32, user_agent: &str) -> Result<ColorImage> {
    let image_size_px = MAP_IMAGE_SIZE_PX;
    let lat_rad = lat.to_radians();
    let world_tiles = 1_i64 << zoom;
    let world_px = world_tiles * 256;
    let x_world = (((lon + 180.0) / 360.0) * world_px as f64).round() as i64;
    let y_world = (((1.0 - (lat_rad.tan() + (1.0 / lat_rad.cos())).ln() / PI) / 2.0)
        * world_px as f64)
        .round() as i64;
    let half = (image_size_px as i64) / 2;
    let start_world_x = x_world - half;
    let start_world_y = y_world - half;
    let end_world_x = start_world_x + image_size_px as i64;
    let end_world_y = start_world_y + image_size_px as i64;
    let min_tile_x = start_world_x.div_euclid(256);
    let max_tile_x = (end_world_x - 1).div_euclid(256);
    let min_tile_y = start_world_y.div_euclid(256);
    let max_tile_y = (end_world_y - 1).div_euclid(256);
    let user_agent = if user_agent.trim().is_empty() {
        DEFAULT_OSM_TILE_USER_AGENT
    } else {
        user_agent.trim()
    };
    let client = Client::builder()
        .user_agent(user_agent)
        .build()
        .context("failed to build tile HTTP client")?;

    let mut canvas = image::RgbaImage::new(image_size_px, image_size_px);

    for tile_y in min_tile_y..=max_tile_y {
        if tile_y < 0 || tile_y >= world_tiles {
            continue;
        }
        for tile_x in min_tile_x..=max_tile_x {
            let wrapped_tile_x = tile_x.rem_euclid(world_tiles) as u32;
            let tile_y_u32 = tile_y as u32;
            let url =
                format!("https://tile.openstreetmap.org/{zoom}/{wrapped_tile_x}/{tile_y_u32}.png");
            let response = client
                .get(&url)
                .send()
                .with_context(|| format!("tile request failed for {url}"))?
                .error_for_status()
                .with_context(|| {
                    format!(
                        "OpenStreetMap tile request was rejected for {url} (HTTP error status returned)"
                    )
                })?;
            let bytes = response.bytes().context("tile bytes missing")?;
            let tile = image::load_from_memory(&bytes)
                .context("tile decoding failed")?
                .to_rgba8();

            let tile_start_x = tile_x * 256;
            let tile_start_y = tile_y * 256;
            let tile_end_x = tile_start_x + 256;
            let tile_end_y = tile_start_y + 256;

            let overlap_start_x = start_world_x.max(tile_start_x);
            let overlap_start_y = start_world_y.max(tile_start_y);
            let overlap_end_x = end_world_x.min(tile_end_x);
            let overlap_end_y = end_world_y.min(tile_end_y);
            if overlap_start_x >= overlap_end_x || overlap_start_y >= overlap_end_y {
                continue;
            }

            let copy_w = (overlap_end_x - overlap_start_x) as u32;
            let copy_h = (overlap_end_y - overlap_start_y) as u32;
            let src_x = (overlap_start_x - tile_start_x) as u32;
            let src_y = (overlap_start_y - tile_start_y) as u32;
            let dst_x = (overlap_start_x - start_world_x) as u32;
            let dst_y = (overlap_start_y - start_world_y) as u32;

            for dy in 0..copy_h {
                for dx in 0..copy_w {
                    let pixel = tile.get_pixel(src_x + dx, src_y + dy);
                    canvas.put_pixel(dst_x + dx, dst_y + dy, *pixel);
                }
            }
        }
    }

    let (w, h) = canvas.dimensions();
    Ok(ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        canvas.as_raw(),
    ))
}

fn main() -> Result<()> {
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "Third Eye Client",
        native_options,
        Box::new(|_cc| Ok(Box::new(ThirdEyeApp::new()))),
    )
    .map_err(|err| anyhow::anyhow!("failed to run GUI app: {err}"))
}
