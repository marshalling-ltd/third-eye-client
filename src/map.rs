use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use anyhow::{Context, Result};
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2_core_location::{CLAuthorizationStatus, CLLocationManager, kCLLocationAccuracyBest};
use reqwest::blocking::Client;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use third_eye_client::storage::tile_cache::TileCacheStore;

pub const DEFAULT_ZOOM: u32 = 14;
pub const MIN_ZOOM: u32 = 3;
pub const MAX_ZOOM: u32 = 19;
pub const MAP_IMAGE_SIZE_PX: u32 = 768;
const MAP_TILE_SIZE_PX: isize = 256;
const MAP_TILE_CACHE_MARGIN: isize = 8;
pub const DEFAULT_OSM_TILE_USER_AGENT: &str =
    "third-eye-client/0.1 (desktop map viewer; set contact URL/email for production use)";

// ---------------------------------------------------------------------------
// Shared frame type
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
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
    /// Persistent disk tile cache (SQLite-backed). `None` when tile saving is
    /// disabled.
    disk_cache: Option<TileCacheStore>,
    /// Maximum bytes allowed for the disk tile cache.
    disk_cache_max_bytes: u64,
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
            disk_cache: None,
            disk_cache_max_bytes: 0,
        }
    }

    /// Configures the persistent disk tile cache. Pass `None` to disable.
    pub fn set_disk_cache(&mut self, store: Option<TileCacheStore>, max_bytes: u64) {
        self.disk_cache = store;
        self.disk_cache_max_bytes = max_bytes;
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
                let disk_cache = self.disk_cache.clone();
                let disk_cache_max_bytes = self.disk_cache_max_bytes;
                let spawn_result = thread::Builder::new()
                    .name(format!("osm-tile-{}-{}-{}", coord.z, coord.x, coord.y))
                    .spawn(move || {
                        let outcome = load_osm_tile_cached(
                            client,
                            coord,
                            &user_agent,
                            disk_cache.as_ref(),
                            disk_cache_max_bytes,
                        )
                        .map_or_else(
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
// Windows Location Services
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
pub(crate) fn detect_location_from_windows_location_blocking() -> Result<(f64, f64)> {
    use windows::Devices::Geolocation::{GeolocationAccessStatus, Geolocator};

    let locator = Geolocator::new().context("failed to create Windows Geolocator")?;

    let access = Geolocator::RequestAccessAsync()
        .context("RequestAccessAsync failed")?
        .join()
        .context("waiting for location access")?;

    if access != GeolocationAccessStatus::Allowed {
        anyhow::bail!("Windows location access was not granted (status: {access:?})");
    }

    let position = locator
        .GetGeopositionAsync()
        .context("GetGeopositionAsync failed")?
        .join()
        .context("waiting for GPS position")?;

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
#[allow(dead_code)]
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

/// Initialises the CoreLocation manager
/// blocking. Safe to call before the Slint run loop starts; actual fixes are
/// delivered once the event loop is running. Call this at app startup so that
/// `check_corelocation_warmup_fix` can return a result quickly afterwards.
#[cfg(target_os = "macos")]
pub fn prime_corelocation_at_startup(map: &mut MapState) {
    unsafe {
        if !CLLocationManager::locationServicesEnabled_class() {
            return;
        }
        if map.corelocation_manager.is_none() {
            map.corelocation_manager = Some(CLLocationManager::new());
        }
        let Some(manager) = map.corelocation_manager.as_ref() else {
            return;
        };
        manager.setDesiredAccuracy(kCLLocationAccuracyBest);
        let status = manager.authorizationStatus();
        // Request permission on first launch (non-blocking — shows native prompt).
        if status == CLAuthorizationStatus::NotDetermined && !map.corelocation_permission_requested
        {
            manager.requestWhenInUseAuthorization();
            map.corelocation_permission_requested = true;
        }
        // If already authorised, start the continuous location stream so that
        // manager.location() gets populated as soon as the run loop starts.
        if status == CLAuthorizationStatus::AuthorizedAlways
            || status == CLAuthorizationStatus::AuthorizedWhenInUse
        {
            manager.startUpdatingLocation();
        }
    }
}

/// Checks whether the CoreLocation manager already has a valid cached fix,
/// without blocking. Also ensures location updates are running whenever the
/// manager is authorised (idempotent). Returns `Some((lat, lon))` on success
/// and stops the continuous update stream to save power.
#[cfg(target_os = "macos")]
pub fn check_corelocation_warmup_fix(map: &MapState) -> Option<(f64, f64)> {
    unsafe {
        let manager = map.corelocation_manager.as_ref()?;
        let status = manager.authorizationStatus();
        if status == CLAuthorizationStatus::AuthorizedAlways
            || status == CLAuthorizationStatus::AuthorizedWhenInUse
        {
            // Keep updates running until we have a fix (idempotent call).
            manager.startUpdatingLocation();
            if let Some(location) = manager.location() {
                let coordinate = location.coordinate();
                let lat = coordinate.latitude;
                let lon = coordinate.longitude;
                if lat.is_finite()
                    && lon.is_finite()
                    && (-90.0..=90.0).contains(&lat)
                    && (-180.0..=180.0).contains(&lon)
                {
                    // We have what we need — stop draining power.
                    manager.stopUpdatingLocation();
                    return Some((lat, lon));
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// OSM tile loading
// ---------------------------------------------------------------------------

/// Loads a tile, checking the persistent disk cache first when available.
/// On a cache miss the tile is fetched from the network and saved to disk.
fn load_osm_tile_cached(
    client: Client,
    coord: TileCoordinate,
    user_agent: &str,
    disk_cache: Option<&TileCacheStore>,
    disk_cache_max_bytes: u64,
) -> Result<RgbaFrame> {
    // --- Disk cache hit path ---
    if let Some(cache) = disk_cache
        && let Ok(Some(png_data)) = cache.get_tile(coord.z, coord.x as i64, coord.y as i64)
    {
        return decode_png_to_frame(&png_data);
    }

    // --- Network fetch ---
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

    // --- Persist to disk cache ---
    if let Some(cache) = disk_cache {
        // Save raw PNG bytes; errors are non-fatal.
        let _ = cache.put_tile(coord.z, coord.x as i64, coord.y as i64, &bytes);
        // Run LRU eviction in the background (cheap if under budget).
        let _ = cache.evict_lru(disk_cache_max_bytes);
    }

    decode_png_to_frame(&bytes)
}

fn decode_png_to_frame(bytes: &[u8]) -> Result<RgbaFrame> {
    let image = image::load_from_memory(bytes)
        .context("tile decode failed")?
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_size_zoom_0() {
        assert!((MapTilesState::world_size_px(0) - 256.0).abs() < f64::EPSILON);
    }

    #[test]
    fn world_size_zoom_1() {
        assert!((MapTilesState::world_size_px(1) - 512.0).abs() < f64::EPSILON);
    }

    #[test]
    fn world_size_zoom_14() {
        let expected = 256.0 * f64::exp2(14.0);
        assert!((MapTilesState::world_size_px(14) - expected).abs() < 1.0);
    }

    #[test]
    fn lat_lon_null_island() {
        let (x, y) = lat_lon_to_world_px(0.0, 0.0, 14);
        let world = MapTilesState::world_size_px(14) as f32;
        assert!((x - world / 2.0).abs() < 1.0);
        assert!((y - world / 2.0).abs() < 1.0);
    }

    #[test]
    fn lat_lon_london() {
        let (x, y) = lat_lon_to_world_px(51.5, -0.12, 14);
        let world = MapTilesState::world_size_px(14) as f32;
        assert!(x > 0.0 && x < world);
        assert!(y > 0.0 && y < world / 2.0);
    }

    #[test]
    fn lat_lon_sydney() {
        let (x, y) = lat_lon_to_world_px(-33.8, 151.2, 14);
        let world = MapTilesState::world_size_px(14) as f32;
        assert!(x > world / 2.0);
        assert!(y > world / 2.0);
    }

    #[test]
    fn lat_lon_clamping_at_poles() {
        let (_, y_north) = lat_lon_to_world_px(90.0, 0.0, 14);
        let (_, y_south) = lat_lon_to_world_px(-90.0, 0.0, 14);
        assert!(y_north >= 0.0);
        let world = MapTilesState::world_size_px(14) as f32;
        assert!(y_south <= world);
    }

    #[test]
    fn ease_out_cubic_boundaries() {
        assert!((ease_out_cubic(0.0) - 0.0).abs() < f64::EPSILON);
        assert!((ease_out_cubic(1.0) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn ease_out_cubic_midpoint() {
        let mid = ease_out_cubic(0.5);
        assert!(mid > 0.5);
        assert!(mid < 1.0);
    }

    #[test]
    fn compute_scale_bar_returns_known_unit() {
        let (_, label) = compute_scale_bar(14, 45.0);
        assert!(label.ends_with(" m") || label.ends_with(" km"));
    }

    #[test]
    fn compute_scale_bar_high_zoom() {
        let (_, label) = compute_scale_bar(19, 45.0);
        assert!(label.ends_with(" m"));
    }

    #[test]
    fn compute_scale_bar_low_zoom() {
        let (_, label) = compute_scale_bar(3, 45.0);
        assert!(label.ends_with(" km"));
    }

    #[test]
    fn center_lat_lon_round_trip() {
        let mut state = MapTilesState::new();
        state.update_visible_size(800.0, 600.0, 14);
        state.center_on_location(51.5, -0.12, 14);
        let (lat, lon) = state.center_lat_lon(14).expect("should have center");
        assert!((lat - 51.5).abs() < 0.01);
        assert!((lon - (-0.12)).abs() < 0.01);
    }

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    /// Encodes a solid-colour RGBA image as PNG bytes for decode/network tests.
    fn solid_png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(width, height, image::Rgba(color));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("encode test png");
        bytes
    }

    /// Builds a zero-filled `RgbaFrame` of the requested size.
    fn solid_frame(width: u32, height: u32) -> RgbaFrame {
        RgbaFrame {
            width,
            height,
            rgba: vec![0u8; width as usize * height as usize * 4],
        }
    }

    /// A 1x1 Slint image, handy for populating tile maps in tests.
    fn blank_image() -> Image {
        rgba_frame_to_slint_image(&solid_frame(1, 1))
    }

    /// Serialises tests that mutate the `OSM_TILES_URL` env var. Holding the
    /// guard for the whole test prevents background loader threads in other
    /// tests from observing a stale override (and never the real OSM host).
    static OSM_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_osm_env() -> std::sync::MutexGuard<'static, ()> {
        OSM_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    // -----------------------------------------------------------------------
    // PNG decode + Slint image conversion
    // -----------------------------------------------------------------------

    #[test]
    fn decode_png_to_frame_resizes_to_tile_size() {
        let png = solid_png(10, 10, [10, 20, 30, 255]);
        let frame = decode_png_to_frame(&png).expect("decode ok");
        assert_eq!(frame.width, MAP_TILE_SIZE_PX as u32);
        assert_eq!(frame.height, MAP_TILE_SIZE_PX as u32);
        assert_eq!(
            frame.rgba.len(),
            frame.width as usize * frame.height as usize * 4
        );
    }

    #[test]
    fn decode_png_to_frame_rejects_garbage() {
        let err = decode_png_to_frame(&[0, 1, 2, 3, 4]).expect_err("should fail");
        assert!(format!("{err:#}").contains("decode"));
    }

    #[test]
    fn rgba_frame_to_slint_image_preserves_dimensions() {
        let frame = RgbaFrame {
            width: 4,
            height: 6,
            rgba: vec![0u8; 4 * 6 * 4],
        };
        let image = rgba_frame_to_slint_image(&frame);
        assert_eq!(image.size().width, 4);
        assert_eq!(image.size().height, 6);
    }

    // -----------------------------------------------------------------------
    // Visible size / offset / viewport geometry
    // -----------------------------------------------------------------------

    #[test]
    fn update_visible_size_detects_change_and_noop() {
        let mut state = MapTilesState::new();
        assert!(state.update_visible_size(800.0, 600.0, 14));
        assert!(!state.update_visible_size(800.0, 600.0, 14));
    }

    #[test]
    fn update_visible_size_clamps_extremes() {
        let mut state = MapTilesState::new();
        state.update_visible_size(1.0, 100_000.0, 14);
        assert!((state.visible_width - 32.0).abs() < f64::EPSILON);
        assert!((state.visible_height - 4096.0).abs() < f64::EPSILON);
    }

    #[test]
    fn zoom_focus_center_is_half_visible() {
        let mut state = MapTilesState::new();
        state.update_visible_size(800.0, 600.0, 14);
        let (cx, cy) = state.zoom_focus_center();
        assert!((cx - 400.0).abs() < f64::EPSILON);
        assert!((cy - 300.0).abs() < f64::EPSILON);
    }

    #[test]
    fn viewport_round_trips_offset() {
        let mut state = MapTilesState::new();
        state.update_visible_size(800.0, 600.0, 5);
        // Negative viewport => positive offset, which survives clamping.
        state.set_offset_from_viewport(-500.0, -300.0, 5);
        let (vx, vy, w, h) = state.viewport_for_slint(5);
        assert!((vx - (-500.0)).abs() < 1e-3);
        assert!((vy - (-300.0)).abs() < 1e-3);
        let world = MapTilesState::world_size_px(5) as f32;
        assert!((w - world).abs() < 1.0);
        assert!((h - world).abs() < 1.0);
    }

    // -----------------------------------------------------------------------
    // Zooming
    // -----------------------------------------------------------------------

    #[test]
    fn set_zoom_level_same_zoom_is_noop() {
        let mut state = MapTilesState::new();
        state.update_visible_size(800.0, 600.0, 14);
        state.center_on_location(40.0, -70.0, 14);
        let before_x = state.offset_x;
        let before_y = state.offset_y;
        state.set_zoom_level(14, 14, 400.0, 300.0);
        assert!((state.offset_x - before_x).abs() < f64::EPSILON);
        assert!((state.offset_y - before_y).abs() < f64::EPSILON);
        assert!(state.fallback_zoom.is_none());
    }

    #[test]
    fn set_zoom_level_sets_fallback_and_clears_loading() {
        let mut state = MapTilesState::new();
        state.update_visible_size(800.0, 600.0, 14);
        state.center_on_location(40.0, -70.0, 14);
        state
            .loading_tiles
            .insert(TileCoordinate { z: 14, x: 1, y: 1 });
        state.set_zoom_level(14, 15, 400.0, 300.0);
        assert_eq!(state.fallback_zoom, Some(14));
        assert!(state.loading_tiles.is_empty());
    }

    #[test]
    fn set_zoom_level_keeps_center_anchored() {
        let mut state = MapTilesState::new();
        state.update_visible_size(800.0, 600.0, 14);
        state.center_on_location(48.0, 2.0, 14);
        let (cx, cy) = state.zoom_focus_center();
        state.set_zoom_level(14, 16, cx, cy);
        let (lat, lon) = state.center_lat_lon(16).expect("center");
        assert!((lat - 48.0).abs() < 0.05);
        assert!((lon - 2.0).abs() < 0.05);
    }

    #[test]
    fn center_lat_lon_round_trip_multiple_points() {
        let points = [(0.0, 0.0), (51.5, -0.12), (-33.8, 151.2), (35.68, 139.69)];
        for (lat, lon) in points {
            let mut state = MapTilesState::new();
            state.update_visible_size(800.0, 600.0, 12);
            state.center_on_location(lat, lon, 12);
            let (got_lat, got_lon) = state.center_lat_lon(12).expect("center");
            assert!((got_lat - lat).abs() < 0.05, "lat {lat} got {got_lat}");
            assert!((got_lon - lon).abs() < 0.05, "lon {lon} got {got_lon}");
        }
    }

    // -----------------------------------------------------------------------
    // Tile bounds
    // -----------------------------------------------------------------------

    #[test]
    fn coord_in_bounds_accepts_inside_and_rejects_outside() {
        let inside = TileCoordinate { z: 10, x: 5, y: 5 };
        assert!(MapTilesState::coord_in_bounds(&inside, 0, 0, 10, 10, 1024));
        let negative = TileCoordinate { z: 10, x: -1, y: 5 };
        assert!(!MapTilesState::coord_in_bounds(
            &negative, -5, 0, 10, 10, 1024
        ));
        let beyond = TileCoordinate {
            z: 10,
            x: 1024,
            y: 5,
        };
        assert!(!MapTilesState::coord_in_bounds(
            &beyond, 0, 0, 2000, 10, 1024
        ));
    }

    #[test]
    fn coord_in_bounds_respects_margin() {
        // max_x = 10, margin = 8: x = 14 is within (< 18) but x = 18 is not.
        let within = TileCoordinate { z: 10, x: 14, y: 5 };
        assert!(MapTilesState::coord_in_bounds(&within, 0, 0, 10, 10, 1024));
        let edge = TileCoordinate { z: 10, x: 18, y: 5 };
        assert!(!MapTilesState::coord_in_bounds(&edge, 0, 0, 10, 10, 1024));
    }

    // -----------------------------------------------------------------------
    // poll_loaded_tiles
    // -----------------------------------------------------------------------

    #[test]
    fn poll_loaded_tiles_applies_current_zoom_frame() {
        let mut state = MapTilesState::new();
        let coord = TileCoordinate { z: 14, x: 3, y: 4 };
        state.loading_tiles.insert(coord);
        state
            .tile_result_tx
            .send(TileLoadResult {
                coord,
                frame: Some(solid_frame(256, 256)),
                error: None,
            })
            .unwrap();
        let (changed, err) = state.poll_loaded_tiles(14);
        assert!(changed);
        assert!(err.is_none());
        assert!(state.loaded_tiles.contains_key(&coord));
        assert!(state.tile_cache.contains_key(&coord));
        assert!(!state.loading_tiles.contains(&coord));
    }

    #[test]
    fn poll_loaded_tiles_reports_error_for_current_zoom() {
        let mut state = MapTilesState::new();
        let coord = TileCoordinate { z: 14, x: 1, y: 1 };
        state.loading_tiles.insert(coord);
        state
            .tile_result_tx
            .send(TileLoadResult {
                coord,
                frame: None,
                error: Some("boom".to_string()),
            })
            .unwrap();
        let (changed, err) = state.poll_loaded_tiles(14);
        assert!(!changed);
        assert_eq!(err.as_deref(), Some("boom"));
    }

    #[test]
    fn poll_loaded_tiles_caches_other_zoom_without_changing() {
        let mut state = MapTilesState::new();
        let coord = TileCoordinate { z: 10, x: 1, y: 1 };
        state
            .tile_result_tx
            .send(TileLoadResult {
                coord,
                frame: Some(solid_frame(256, 256)),
                error: None,
            })
            .unwrap();
        let (changed, _err) = state.poll_loaded_tiles(14);
        assert!(!changed);
        assert!(state.tile_cache.contains_key(&coord));
        assert!(!state.loaded_tiles.contains_key(&coord));
    }

    #[test]
    fn poll_loaded_tiles_clears_fallback_equal_to_zoom() {
        let mut state = MapTilesState::new();
        state.fallback_zoom = Some(14);
        let _ = state.poll_loaded_tiles(14);
        assert!(state.fallback_zoom.is_none());
    }

    #[test]
    fn poll_loaded_tiles_clears_fallback_after_enough_tiles() {
        let mut state = MapTilesState::new();
        state.fallback_zoom = Some(13);
        for i in 0_isize..8 {
            state
                .loaded_tiles
                .insert(TileCoordinate { z: 14, x: i, y: 0 }, blank_image());
        }
        state
            .loaded_tiles
            .insert(TileCoordinate { z: 13, x: 0, y: 0 }, blank_image());
        let _ = state.poll_loaded_tiles(14);
        assert!(state.fallback_zoom.is_none());
        assert!(state.loaded_tiles.keys().all(|c| c.z == 14));
        assert_eq!(state.loaded_tiles.len(), 8);
    }

    #[test]
    fn poll_loaded_tiles_keeps_fallback_when_insufficient_tiles() {
        let mut state = MapTilesState::new();
        state.fallback_zoom = Some(13);
        for i in 0_isize..3 {
            state
                .loaded_tiles
                .insert(TileCoordinate { z: 14, x: i, y: 0 }, blank_image());
        }
        let _ = state.poll_loaded_tiles(14);
        assert_eq!(state.fallback_zoom, Some(13));
    }

    // -----------------------------------------------------------------------
    // visible_tiles
    // -----------------------------------------------------------------------

    #[test]
    fn visible_tiles_includes_current_and_fallback() {
        let mut state = MapTilesState::new();
        state.fallback_zoom = Some(13);
        state
            .loaded_tiles
            .insert(TileCoordinate { z: 14, x: 2, y: 3 }, blank_image());
        state
            .loaded_tiles
            .insert(TileCoordinate { z: 13, x: 1, y: 1 }, blank_image());
        state
            .loaded_tiles
            .insert(TileCoordinate { z: 9, x: 0, y: 0 }, blank_image());
        let tiles = state.visible_tiles(14);
        assert_eq!(tiles.len(), 2);
    }

    #[test]
    fn visible_tiles_scales_fallback_geometry() {
        let mut state = MapTilesState::new();
        state.fallback_zoom = Some(13);
        state
            .loaded_tiles
            .insert(TileCoordinate { z: 13, x: 2, y: 3 }, blank_image());
        let tiles = state.visible_tiles(14);
        assert_eq!(tiles.len(), 1);
        let tile = &tiles[0];
        // render_zoom (14) is one above tile zoom (13): scale = 2.
        let unit = MAP_TILE_SIZE_PX as f32 * 2.0;
        assert!((tile.size - unit).abs() < f32::EPSILON);
        assert!((tile.x - 2.0 * unit).abs() < f32::EPSILON);
        assert!((tile.y - 3.0 * unit).abs() < f32::EPSILON);
    }

    // -----------------------------------------------------------------------
    // request_visible_tiles (offline paths)
    // -----------------------------------------------------------------------

    #[test]
    fn request_visible_tiles_promotes_cached_tiles_without_spawning() {
        let _guard = lock_osm_env();
        // Any accidental spawn fails fast instead of hitting the real OSM host.
        // SAFETY: env access serialised by OSM_ENV_LOCK.
        unsafe { std::env::set_var("OSM_TILES_URL", "http://127.0.0.1:1") };
        let mut state = MapTilesState::new();
        state.update_visible_size(256.0, 256.0, 3);
        state.center_on_location(0.0, 0.0, 3);
        let (min_x, min_y, max_x, max_y, world_tiles) = state.visible_tile_bounds(3, 3);
        for x in min_x..max_x {
            for y in min_y..max_y {
                if (0..world_tiles).contains(&x) && (0..world_tiles).contains(&y) {
                    state
                        .tile_cache
                        .insert(TileCoordinate { z: 3, x, y }, blank_image());
                }
            }
        }
        state.request_visible_tiles(3, "agent");
        // SAFETY: env access serialised by OSM_ENV_LOCK.
        unsafe { std::env::remove_var("OSM_TILES_URL") };
        assert!(state.loading_tiles.is_empty());
        assert!(!state.loaded_tiles.is_empty());
        assert!(state.loaded_tiles.keys().all(|c| c.z == 3));
    }

    #[test]
    fn request_visible_tiles_evicts_distant_zoom_when_cache_large() {
        let _guard = lock_osm_env();
        // SAFETY: env access serialised by OSM_ENV_LOCK.
        unsafe { std::env::set_var("OSM_TILES_URL", "http://127.0.0.1:1") };
        let mut state = MapTilesState::new();
        state.update_visible_size(256.0, 256.0, 14);
        state.center_on_location(0.0, 0.0, 14);
        // >500 cached tiles at a far-away zoom (|z - 14| > 2) trigger eviction.
        for i in 0..600_isize {
            state
                .tile_cache
                .insert(TileCoordinate { z: 3, x: i, y: 0 }, blank_image());
        }
        // Pre-fill the actual visible coords so the request needs no network.
        let (min_x, min_y, max_x, max_y, world_tiles) = state.visible_tile_bounds(14, 14);
        for x in min_x..max_x {
            for y in min_y..max_y {
                if (0..world_tiles).contains(&x) && (0..world_tiles).contains(&y) {
                    state
                        .tile_cache
                        .insert(TileCoordinate { z: 14, x, y }, blank_image());
                }
            }
        }
        state.request_visible_tiles(14, "agent");
        // SAFETY: env access serialised by OSM_ENV_LOCK.
        unsafe { std::env::remove_var("OSM_TILES_URL") };
        assert!(state.tile_cache.keys().all(|c| c.z >= 12 && c.z <= 16));
        assert!(state.loading_tiles.is_empty());
    }

    // -----------------------------------------------------------------------
    // load_osm_tile_cached (network via mockito) + disk cache
    // -----------------------------------------------------------------------

    #[test]
    fn load_osm_tile_cached_fetches_from_network() {
        let _guard = lock_osm_env();
        let mut server = mockito::Server::new();
        let png = solid_png(8, 8, [1, 2, 3, 255]);
        let mock = server
            .mock("GET", "/3/1/2.png")
            .with_status(200)
            .with_header("content-type", "image/png")
            .with_body(png)
            .create();
        // SAFETY: env access serialised by OSM_ENV_LOCK.
        unsafe { std::env::set_var("OSM_TILES_URL", server.url()) };
        let coord = TileCoordinate { z: 3, x: 1, y: 2 };
        let frame =
            load_osm_tile_cached(Client::new(), coord, "agent", None, 0).expect("network ok");
        // SAFETY: env access serialised by OSM_ENV_LOCK.
        unsafe { std::env::remove_var("OSM_TILES_URL") };
        mock.assert();
        assert_eq!(frame.width, MAP_TILE_SIZE_PX as u32);
        assert_eq!(frame.height, MAP_TILE_SIZE_PX as u32);
    }

    #[test]
    fn load_osm_tile_cached_maps_http_error() {
        let _guard = lock_osm_env();
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/3/1/2.png")
            .with_status(500)
            .with_body("nope")
            .create();
        // SAFETY: env access serialised by OSM_ENV_LOCK.
        unsafe { std::env::set_var("OSM_TILES_URL", server.url()) };
        let coord = TileCoordinate { z: 3, x: 1, y: 2 };
        let err =
            load_osm_tile_cached(Client::new(), coord, "agent", None, 0).expect_err("should fail");
        // SAFETY: env access serialised by OSM_ENV_LOCK.
        unsafe { std::env::remove_var("OSM_TILES_URL") };
        mock.assert();
        assert!(format!("{err:#}").contains("non-success"));
    }

    #[test]
    fn load_osm_tile_cached_uses_disk_cache_hit() {
        let _guard = lock_osm_env();
        let app = third_eye_client::storage::AppStore::open_in_memory().unwrap();
        let store = app.tile_cache().clone();
        let png = solid_png(8, 8, [9, 9, 9, 255]);
        store.put_tile(3, 1, 2, &png).unwrap();
        // Unreachable URL proves the disk hit short-circuits the network.
        // SAFETY: env access serialised by OSM_ENV_LOCK.
        unsafe { std::env::set_var("OSM_TILES_URL", "http://127.0.0.1:1") };
        let coord = TileCoordinate { z: 3, x: 1, y: 2 };
        let frame = load_osm_tile_cached(Client::new(), coord, "agent", Some(&store), 1_000_000)
            .expect("disk hit");
        // SAFETY: env access serialised by OSM_ENV_LOCK.
        unsafe { std::env::remove_var("OSM_TILES_URL") };
        assert_eq!(frame.width, MAP_TILE_SIZE_PX as u32);
    }

    #[test]
    fn load_osm_tile_cached_persists_to_disk() {
        let _guard = lock_osm_env();
        let app = third_eye_client::storage::AppStore::open_in_memory().unwrap();
        let store = app.tile_cache().clone();
        let mut server = mockito::Server::new();
        let png = solid_png(8, 8, [4, 5, 6, 255]);
        let mock = server
            .mock("GET", "/5/3/7.png")
            .with_status(200)
            .with_body(png)
            .create();
        // SAFETY: env access serialised by OSM_ENV_LOCK.
        unsafe { std::env::set_var("OSM_TILES_URL", server.url()) };
        let coord = TileCoordinate { z: 5, x: 3, y: 7 };
        assert!(store.get_tile(5, 3, 7).unwrap().is_none());
        let _frame = load_osm_tile_cached(Client::new(), coord, "agent", Some(&store), 10_000_000)
            .expect("network ok");
        // SAFETY: env access serialised by OSM_ENV_LOCK.
        unsafe { std::env::remove_var("OSM_TILES_URL") };
        mock.assert();
        assert!(store.get_tile(5, 3, 7).unwrap().is_some());
    }

    #[test]
    fn request_visible_tiles_spawns_loaders_and_poll_applies() {
        let _guard = lock_osm_env();
        let mut server = mockito::Server::new();
        let png = solid_png(8, 8, [7, 7, 7, 255]);
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_body(png)
            .create();
        // SAFETY: env access serialised by OSM_ENV_LOCK.
        unsafe { std::env::set_var("OSM_TILES_URL", server.url()) };
        let mut state = MapTilesState::new();
        state.update_visible_size(256.0, 256.0, 3);
        state.center_on_location(0.0, 0.0, 3);
        state.request_visible_tiles(3, "agent");
        let mut changed = false;
        for _ in 0..200 {
            let (c, _err) = state.poll_loaded_tiles(3);
            changed |= c;
            if state.loading_tiles.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let drained_empty = state.loading_tiles.is_empty();
        // Only clear the override once every loader thread has finished, so no
        // straggler can fall back to the real OSM host.
        // SAFETY: env access serialised by OSM_ENV_LOCK.
        unsafe { std::env::remove_var("OSM_TILES_URL") };
        assert!(drained_empty, "loaders did not finish in time");
        assert!(changed);
        assert!(!state.loaded_tiles.is_empty());
    }

    // -----------------------------------------------------------------------
    // CoreLocation (macOS)
    // -----------------------------------------------------------------------

    #[cfg(target_os = "macos")]
    #[test]
    fn corelocation_status_label_covers_all_branches() {
        assert_eq!(
            corelocation_status_label(CLAuthorizationStatus::NotDetermined),
            "NotDetermined"
        );
        assert_eq!(
            corelocation_status_label(CLAuthorizationStatus::Denied),
            "Denied"
        );
        assert_eq!(
            corelocation_status_label(CLAuthorizationStatus::Restricted),
            "Restricted"
        );
        assert_eq!(
            corelocation_status_label(CLAuthorizationStatus::AuthorizedWhenInUse),
            "AuthorizedWhenInUse"
        );
        assert_eq!(
            corelocation_status_label(CLAuthorizationStatus::AuthorizedAlways),
            "AuthorizedAlways"
        );
        assert_eq!(
            corelocation_status_label(CLAuthorizationStatus(99)),
            "Unknown"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn corelocation_debug_status_handles_uninitialized_manager() {
        let map = MapState::default();
        let text = corelocation_debug_status(&map);
        assert!(text.contains("manager_initialized=false"));
        assert!(text.contains("ManagerNotInitialized"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn check_corelocation_warmup_fix_returns_none_without_manager() {
        let map = MapState::default();
        assert!(check_corelocation_warmup_fix(&map).is_none());
    }
}
