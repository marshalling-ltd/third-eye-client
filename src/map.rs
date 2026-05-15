use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;
use std::sync::mpsc::{self, Receiver};
use std::thread;
#[cfg(target_os = "macos")]
use std::time::Duration;

use anyhow::{Context, Result};
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2_core_location::{CLAuthorizationStatus, CLLocationManager, kCLLocationAccuracyBest};
use reqwest::blocking::Client;
use serde_json::Value;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

pub const DEFAULT_ZOOM: u32 = 14;
pub const MIN_ZOOM: u32 = 3;
pub const MAX_ZOOM: u32 = 19;
pub const MAP_IMAGE_SIZE_PX: u32 = 768;
const MAP_TILE_SIZE_PX: isize = 256;
const MAP_TILE_CACHE_MARGIN: isize = 8;
#[cfg(target_os = "macos")]
const CORELOCATION_FIX_POLL_ATTEMPTS: u32 = 8;
#[cfg(target_os = "macos")]
const CORELOCATION_FIX_POLL_INTERVAL_MS: u64 = 250;
pub const DEFAULT_OSM_TILE_USER_AGENT: &str =
    "third-eye-client/0.1 (desktop map viewer; set contact URL/email for production use)";

// ---------------------------------------------------------------------------
// Shared frame type
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct RgbaFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn rgba_frame_to_slint_image(frame: &RgbaFrame) -> Image {
    let shared_buffer =
        SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&frame.rgba, frame.width, frame.height);
    Image::from_rgba8(shared_buffer)
}

// ---------------------------------------------------------------------------
// Map state
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct MapState {
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub zoom: u32,
    pub status: String,
    #[cfg(target_os = "macos")]
    pub(crate) corelocation_manager: Option<Retained<CLLocationManager>>,
    #[cfg(target_os = "macos")]
    pub(crate) corelocation_permission_requested: bool,
}

pub struct DetectedLocation {
    pub lat: f64,
    pub lon: f64,
    pub source: String,
}

#[cfg(target_os = "macos")]
enum CoreLocationDetectionOutcome {
    Located(f64, f64),
    PendingPermission(String),
    PendingFix(String),
}

// ---------------------------------------------------------------------------
// Tile coordinates & loading
// ---------------------------------------------------------------------------

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
struct TileCoordinate {
    z: u32,
    x: isize,
    y: isize,
}

struct TileLoadResult {
    coord: TileCoordinate,
    frame: Option<RgbaFrame>,
    error: Option<String>,
}

/// Data for a single visible tile, ready for the UI layer to convert into a
/// Slint model element.
pub struct TileRenderData {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub image: Image,
}

// ---------------------------------------------------------------------------
// MapTilesState
// ---------------------------------------------------------------------------

pub struct MapTilesState {
    client: Client,
    loaded_tiles: BTreeMap<TileCoordinate, Image>,
    loading_tiles: BTreeSet<TileCoordinate>,
    tile_cache: BTreeMap<TileCoordinate, Image>,
    pub fallback_zoom: Option<u32>,
    tile_result_tx: mpsc::Sender<TileLoadResult>,
    tile_result_rx: Receiver<TileLoadResult>,
    visible_width: f64,
    visible_height: f64,
    offset_x: f64,
    offset_y: f64,
}

impl MapTilesState {
    pub fn new() -> Self {
        let (tile_result_tx, tile_result_rx) = mpsc::channel();
        Self {
            client: Client::new(),
            loaded_tiles: BTreeMap::new(),
            loading_tiles: BTreeSet::new(),
            tile_cache: BTreeMap::new(),
            fallback_zoom: None,
            tile_result_tx,
            tile_result_rx,
            visible_width: f64::from(MAP_IMAGE_SIZE_PX),
            visible_height: f64::from(MAP_IMAGE_SIZE_PX),
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }

    pub fn world_size_px(zoom_level: u32) -> f64 {
        (MAP_TILE_SIZE_PX as f64) * f64::exp2(f64::from(zoom_level))
    }

    fn clamp_offset_to_world(&mut self, zoom_level: u32) {
        let world_size = Self::world_size_px(zoom_level);
        let min_x = (world_size - self.visible_width).min(0.0);
        let max_x = (world_size - self.visible_width).max(0.0);
        let min_y = (world_size - self.visible_height).min(0.0);
        let max_y = (world_size - self.visible_height).max(0.0);
        self.offset_x = self.offset_x.clamp(min_x, max_x);
        self.offset_y = self.offset_y.clamp(min_y, max_y);
    }

    pub fn update_visible_size(&mut self, width: f64, height: f64, zoom_level: u32) -> bool {
        let width = width.clamp(32.0, 4096.0);
        let height = height.clamp(32.0, 4096.0);
        let changed = (self.visible_width - width).abs() > f64::EPSILON
            || (self.visible_height - height).abs() > f64::EPSILON;
        if changed {
            self.visible_width = width;
            self.visible_height = height;
            self.clamp_offset_to_world(zoom_level);
        }
        changed
    }

    pub fn center_on_location(&mut self, lat: f64, lon: f64, zoom_level: u32) {
        let world_size = Self::world_size_px(zoom_level);
        let x_world = ((lon + 180.0) / 360.0) * world_size;
        let lat_rad = lat.to_radians();
        let y_world =
            ((1.0 - (lat_rad.tan() + (1.0 / lat_rad.cos())).ln() / PI) / 2.0) * world_size;
        self.offset_x = x_world - (self.visible_width / 2.0);
        self.offset_y = y_world - (self.visible_height / 2.0);
        self.clamp_offset_to_world(zoom_level);
    }

    pub fn set_offset_from_viewport(&mut self, viewport_x: f64, viewport_y: f64, zoom_level: u32) {
        self.offset_x = -viewport_x;
        self.offset_y = -viewport_y;
        self.clamp_offset_to_world(zoom_level);
    }

    pub fn set_zoom_level(&mut self, current_zoom: u32, new_zoom: u32, focus_x: f64, focus_y: f64) {
        if current_zoom == new_zoom {
            return;
        }
        let old_world_size = Self::world_size_px(current_zoom);
        let new_world_size = Self::world_size_px(new_zoom);
        let focus_x = focus_x.clamp(0.0, self.visible_width);
        let focus_y = focus_y.clamp(0.0, self.visible_height);
        let old_anchor_x = (self.offset_x + focus_x).clamp(0.0, old_world_size);
        let old_anchor_y = (self.offset_y + focus_y).clamp(0.0, old_world_size);
        let anchor_u = if old_world_size > 0.0 {
            old_anchor_x / old_world_size
        } else {
            0.5
        };
        let anchor_v = if old_world_size > 0.0 {
            old_anchor_y / old_world_size
        } else {
            0.5
        };
        self.offset_x = anchor_u * new_world_size - focus_x;
        self.offset_y = anchor_v * new_world_size - focus_y;
        self.loading_tiles.clear();
        self.fallback_zoom = Some(current_zoom);
        self.clamp_offset_to_world(new_zoom);
    }

    pub fn zoom_focus_center(&self) -> (f64, f64) {
        (self.visible_width / 2.0, self.visible_height / 2.0)
    }

    pub fn center_lat_lon(&self, zoom_level: u32) -> Option<(f64, f64)> {
        let world_size = Self::world_size_px(zoom_level);
        if world_size <= 0.0 {
            return None;
        }
        let x_world = (self.offset_x + (self.visible_width / 2.0)).clamp(0.0, world_size);
        let y_world = (self.offset_y + (self.visible_height / 2.0)).clamp(0.0, world_size);
        let lon = (x_world / world_size) * 360.0 - 180.0;
        let n = PI - (2.0 * PI * y_world / world_size);
        let lat = n.sinh().atan().to_degrees();
        Some((lat, lon))
    }

    pub fn viewport_for_slint(&self, zoom_level: u32) -> (f32, f32, f32, f32) {
        let world_size = Self::world_size_px(zoom_level) as f32;
        (
            -(self.offset_x as f32),
            -(self.offset_y as f32),
            world_size,
            world_size,
        )
    }

    fn visible_tile_bounds(
        &self,
        current_zoom: u32,
        target_zoom: u32,
    ) -> (isize, isize, isize, isize, isize) {
        let scale = f64::exp2(f64::from(target_zoom as i32 - current_zoom as i32));
        let world_tiles = 1_isize << target_zoom;
        let offset_x = self.offset_x * scale;
        let offset_y = self.offset_y * scale;
        let visible_width = self.visible_width * scale;
        let visible_height = self.visible_height * scale;
        let min_x = (offset_x / MAP_TILE_SIZE_PX as f64).floor() as isize;
        let min_y = (offset_y / MAP_TILE_SIZE_PX as f64).floor() as isize;
        let max_x = (((offset_x + visible_width) / MAP_TILE_SIZE_PX as f64).ceil() as isize + 1)
            .min(world_tiles);
        let max_y = (((offset_y + visible_height) / MAP_TILE_SIZE_PX as f64).ceil() as isize + 1)
            .min(world_tiles);
        (min_x, min_y, max_x, max_y, world_tiles)
    }

    fn coord_in_bounds(
        coord: &TileCoordinate,
        min_x: isize,
        min_y: isize,
        max_x: isize,
        max_y: isize,
        world_tiles: isize,
    ) -> bool {
        coord.x >= 0
            && coord.x < world_tiles
            && coord.y >= 0
            && coord.y < world_tiles
            && coord.x > min_x - MAP_TILE_CACHE_MARGIN
            && coord.x < max_x + MAP_TILE_CACHE_MARGIN
            && coord.y > min_y - MAP_TILE_CACHE_MARGIN
            && coord.y < max_y + MAP_TILE_CACHE_MARGIN
    }

    pub fn request_visible_tiles(&mut self, zoom_level: u32, user_agent: &str) {
        const MAX_TILE_CACHE: usize = 500;
        if self.tile_cache.len() > MAX_TILE_CACHE {
            self.tile_cache
                .retain(|c, _| (c.z as i32 - zoom_level as i32).unsigned_abs() <= 2);
        }
        let (min_x, min_y, max_x, max_y, world_tiles) =
            self.visible_tile_bounds(zoom_level, zoom_level);
        let fallback_bounds = self.fallback_zoom.map(|fallback_zoom| {
            let (fmin_x, fmin_y, fmax_x, fmax_y, fworld_tiles) =
                self.visible_tile_bounds(zoom_level, fallback_zoom);
            (fallback_zoom, fmin_x, fmin_y, fmax_x, fmax_y, fworld_tiles)
        });
        let keep = |coord: &TileCoordinate| {
            if coord.z == zoom_level {
                Self::coord_in_bounds(coord, min_x, min_y, max_x, max_y, world_tiles)
            } else if let Some((fallback_zoom, fmin_x, fmin_y, fmax_x, fmax_y, fworld_tiles)) =
                fallback_bounds
            {
                coord.z == fallback_zoom
                    && Self::coord_in_bounds(coord, fmin_x, fmin_y, fmax_x, fmax_y, fworld_tiles)
            } else {
                false
            }
        };
        self.loaded_tiles.retain(|coord, _| keep(coord));
        self.loading_tiles.retain(keep);

        let user_agent = if user_agent.trim().is_empty() {
            DEFAULT_OSM_TILE_USER_AGENT.to_owned()
        } else {
            user_agent.trim().to_owned()
        };
        for x in min_x..max_x {
            for y in min_y..max_y {
                if !(0..world_tiles).contains(&x) || !(0..world_tiles).contains(&y) {
                    continue;
                }
                let coord = TileCoordinate {
                    z: zoom_level,
                    x,
                    y,
                };
                if self.loaded_tiles.contains_key(&coord) || self.loading_tiles.contains(&coord) {
                    continue;
                }
                if let Some(cached) = self.tile_cache.get(&coord).cloned() {
                    self.loaded_tiles.insert(coord, cached);
                    continue;
                }
                self.loading_tiles.insert(coord);
                let client = self.client.clone();
                let tx = self.tile_result_tx.clone();
                let user_agent = user_agent.clone();
                let tx_for_thread = tx.clone();
                let spawn_result = thread::Builder::new()
                    .name(format!("osm-tile-{}-{}-{}", coord.z, coord.x, coord.y))
                    .spawn(move || {
                        let outcome = load_osm_tile(client, coord, &user_agent).map_or_else(
                            |err| TileLoadResult {
                                coord,
                                frame: None,
                                error: Some(format!(
                                    "Failed loading tile z{} x{} y{}: {err:#}",
                                    coord.z, coord.x, coord.y
                                )),
                            },
                            |frame| TileLoadResult {
                                coord,
                                frame: Some(frame),
                                error: None,
                            },
                        );
                        let _ = tx_for_thread.send(outcome);
                    });
                if let Err(err) = spawn_result {
                    self.loading_tiles.remove(&coord);
                    let _ = tx.send(TileLoadResult {
                        coord,
                        frame: None,
                        error: Some(format!(
                            "Failed spawning tile loader z{} x{} y{}: {err}",
                            coord.z, coord.x, coord.y
                        )),
                    });
                }
            }
        }
    }

    pub fn poll_loaded_tiles(&mut self, zoom_level: u32) -> (bool, Option<String>) {
        let mut changed = false;
        let mut latest_error = None;
        while let Ok(result) = self.tile_result_rx.try_recv() {
            self.loading_tiles.remove(&result.coord);
            if let Some(frame) = result.frame {
                let image = rgba_frame_to_slint_image(&frame);
                self.tile_cache.insert(result.coord, image.clone());
                if result.coord.z == zoom_level {
                    self.loaded_tiles.insert(result.coord, image);
                    changed = true;
                }
            } else if result.coord.z == zoom_level
                && let Some(error) = result.error
            {
                latest_error = Some(error);
            }
        }
        if let Some(fallback_zoom) = self.fallback_zoom {
            if fallback_zoom == zoom_level {
                self.fallback_zoom = None;
            } else {
                let current_loaded_count = self
                    .loaded_tiles
                    .keys()
                    .filter(|coord| coord.z == zoom_level)
                    .count();
                if current_loaded_count >= 8 {
                    self.fallback_zoom = None;
                    self.loaded_tiles.retain(|coord, _| coord.z == zoom_level);
                    self.loading_tiles.retain(|coord| coord.z == zoom_level);
                }
            }
        }
        (changed, latest_error)
    }

    /// Returns visible tile data for UI rendering. The caller wraps this into
    /// the Slint `MapTile` model.
    pub fn visible_tiles(&self, render_zoom: u32) -> Vec<TileRenderData> {
        self.loaded_tiles
            .iter()
            .filter(|(coord, _)| {
                coord.z == render_zoom
                    || self
                        .fallback_zoom
                        .is_some_and(|fallback_zoom| coord.z == fallback_zoom)
            })
            .map(|(coord, image)| {
                let scale = 2.0_f32.powi(render_zoom as i32 - coord.z as i32);
                TileRenderData {
                    x: coord.x as f32 * MAP_TILE_SIZE_PX as f32 * scale,
                    y: coord.y as f32 * MAP_TILE_SIZE_PX as f32 * scale,
                    size: MAP_TILE_SIZE_PX as f32 * scale,
                    image: image.clone(),
                }
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Viewport animation
// ---------------------------------------------------------------------------

pub struct ViewportAnimation {
    pub start_vp_x: f32,
    pub start_vp_y: f32,
    pub target_vp_x: f32,
    pub target_vp_y: f32,
    pub elapsed_ms: f64,
    pub duration_ms: f64,
}

pub fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

// ---------------------------------------------------------------------------
// Scale bar & coordinate helpers
// ---------------------------------------------------------------------------

pub fn compute_scale_bar(zoom: u32, lat: f64) -> (f32, String) {
    const BAR_PX: f32 = 100.0;
    let lat_rad = lat.to_radians();
    let meters_per_pixel = 156543.03392 * lat_rad.cos() / f64::exp2(f64::from(zoom));
    let exact_meters = f64::from(BAR_PX) * meters_per_pixel;

    const NICE_DISTANCES: &[f64] = &[
        1.0,
        2.0,
        5.0,
        10.0,
        20.0,
        50.0,
        100.0,
        200.0,
        500.0,
        1_000.0,
        2_000.0,
        5_000.0,
        10_000.0,
        20_000.0,
        50_000.0,
        100_000.0,
        200_000.0,
        500_000.0,
        1_000_000.0,
        2_000_000.0,
    ];

    let scale_meters = NICE_DISTANCES
        .iter()
        .copied()
        .min_by(|a, b| {
            (a - exact_meters)
                .abs()
                .partial_cmp(&(b - exact_meters).abs())
                .unwrap()
        })
        .unwrap_or(100.0);

    let label = if scale_meters >= 1000.0 {
        format!("{} km", (scale_meters / 1000.0) as u32)
    } else {
        format!("{} m", scale_meters as u32)
    };
    (BAR_PX, label)
}

pub fn lat_lon_to_world_px(lat: f64, lon: f64, zoom_level: u32) -> (f32, f32) {
    let world_size = MapTilesState::world_size_px(zoom_level);
    let lon = lon.clamp(-180.0, 180.0);
    let lat = lat.clamp(-85.051_128_78, 85.051_128_78);
    let x_world = (((lon + 180.0) / 360.0) * world_size).clamp(0.0, world_size);
    let lat_rad = lat.to_radians();
    let y_world = ((1.0 - (lat_rad.tan() + (1.0 / lat_rad.cos())).ln() / PI) / 2.0) * world_size;
    let y_world = y_world.clamp(0.0, world_size);
    (x_world as f32, y_world as f32)
}

// ---------------------------------------------------------------------------
// Location detection
// ---------------------------------------------------------------------------

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

pub fn detect_location(
    map: &mut MapState,
    nmea_fix: Option<(f64, f64)>,
) -> Result<DetectedLocation> {
    // Highest priority: NMEA GPS fix from phone.
    if let Some((lat, lon)) = nmea_fix {
        return Ok(DetectedLocation {
            lat,
            lon,
            source: "Phone GPS (NMEA/TCP)".to_owned(),
        });
    }

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

    #[cfg(target_os = "windows")]
    {
        let _ = map;
        match detect_location_from_windows_location() {
            Ok((lat, lon)) => Ok(DetectedLocation {
                lat,
                lon,
                source: "Windows Location Services (native)".to_owned(),
            }),
            Err(native_err) => {
                let (lat, lon) = detect_location_from_ip().with_context(|| {
                    format!("Windows Location failed ({native_err:#}) and IP fallback also failed")
                })?;
                Ok(DetectedLocation {
                    lat,
                    lon,
                    source: format!("IP geolocation fallback ({native_err:#})"),
                })
            }
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
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

// ---------------------------------------------------------------------------
// Windows Location Services
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn detect_location_from_windows_location() -> Result<(f64, f64)> {
    use windows::Devices::Geolocation::{GeolocationAccessStatus, Geolocator};

    let locator = Geolocator::new().context("failed to create Windows Geolocator")?;

    let access = Geolocator::RequestAccessAsync()
        .context("RequestAccessAsync failed")?
        .get()
        .context("waiting for location access timed out")?;

    if access != GeolocationAccessStatus::Allowed {
        anyhow::bail!("Windows location access was not granted (status: {access:?})");
    }

    let position = locator
        .GetGeopositionAsync()
        .context("GetGeopositionAsync failed")?
        .get()
        .context("waiting for GPS position timed out")?;

    let coordinate = position.Coordinate().context("no coordinate in position")?;
    let point = coordinate.Point().context("no point in coordinate")?;
    let pos = point.Position().context("no position in point")?;

    let lat = pos.Latitude;
    let lon = pos.Longitude;
    if !lat.is_finite()
        || !lon.is_finite()
        || !(-90.0..=90.0).contains(&lat)
        || !(-180.0..=180.0).contains(&lon)
    {
        anyhow::bail!("Windows Location returned an invalid coordinate ({lat}, {lon})");
    }

    Ok((lat, lon))
}

// ---------------------------------------------------------------------------
// CoreLocation (macOS)
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn corelocation_status_label(status: CLAuthorizationStatus) -> &'static str {
    if status == CLAuthorizationStatus::NotDetermined {
        "NotDetermined"
    } else if status == CLAuthorizationStatus::Denied {
        "Denied"
    } else if status == CLAuthorizationStatus::Restricted {
        "Restricted"
    } else if status == CLAuthorizationStatus::AuthorizedWhenInUse {
        "AuthorizedWhenInUse"
    } else if status == CLAuthorizationStatus::AuthorizedAlways {
        "AuthorizedAlways"
    } else {
        "Unknown"
    }
}

#[cfg(target_os = "macos")]
pub fn corelocation_debug_status(map: &MapState) -> String {
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

        if status == CLAuthorizationStatus::NotDetermined {
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

        if status == CLAuthorizationStatus::Denied || status == CLAuthorizationStatus::Restricted {
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

        if status != CLAuthorizationStatus::AuthorizedAlways
            && status != CLAuthorizationStatus::AuthorizedWhenInUse
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

// ---------------------------------------------------------------------------
// OSM tile loading
// ---------------------------------------------------------------------------

fn load_osm_tile(client: Client, coord: TileCoordinate, user_agent: &str) -> Result<RgbaFrame> {
    let tile_base_url =
        std::env::var("OSM_TILES_URL").unwrap_or_else(|_| "https://tile.openstreetmap.org".into());
    let url = format!("{tile_base_url}/{}/{}/{}.png", coord.z, coord.x, coord.y);
    let response = client
        .get(&url)
        .header("User-Agent", user_agent)
        .send()
        .with_context(|| format!("tile request failed for {url}"))?
        .error_for_status()
        .with_context(|| format!("tile request returned non-success for {url}"))?;
    let bytes = response.bytes().context("tile bytes missing")?;
    let image = image::load_from_memory(&bytes)
        .with_context(|| format!("tile decode failed for {url}"))?
        .resize_exact(
            MAP_TILE_SIZE_PX as u32,
            MAP_TILE_SIZE_PX as u32,
            image::imageops::FilterType::Triangle,
        )
        .to_rgba8();
    Ok(RgbaFrame {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}
