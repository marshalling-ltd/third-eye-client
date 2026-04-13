use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;
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
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2_core_location::{CLAuthorizationStatus, CLLocationManager, kCLLocationAccuracyBest};
use reqwest::Url;
use reqwest::blocking::Client;
use serde_json::Value;
use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use third_eye_client::rov_status::{ROV_STATUS_UDP_PORT, UdpStatusState};

const DEFAULT_TEST_RTSP: &str = "rtsp://admin:admin@127.0.0.1:8554/stream";
const DEFAULT_ROV_RTSP: &str = "rtsp://admin:admin@192.168.1.88:8554/stream/0/0";
const DEFAULT_ROV_HTTP_BASE: &str = "http://192.168.1.88";
const DEFAULT_ZOOM: u32 = 14;
const MIN_ZOOM: u32 = 3;
const MAX_ZOOM: u32 = 19;
const MAP_IMAGE_SIZE_PX: u32 = 768;
const MAP_TILE_SIZE_PX: isize = 256;
const MAP_TILE_CACHE_MARGIN: isize = 8;
#[cfg(target_os = "macos")]
const CORELOCATION_FIX_POLL_ATTEMPTS: u32 = 8;
#[cfg(target_os = "macos")]
const CORELOCATION_FIX_POLL_INTERVAL_MS: u64 = 250;
const DEFAULT_OSM_TILE_USER_AGENT: &str =
    "third-eye-client/0.1 (desktop map viewer; set contact URL/email for production use)";

slint::slint! {
import { Button, HorizontalBox, LineEdit, VerticalBox } from "std-widgets.slint";

export struct MapTile {
    x: length,
    y: length,
    size: length,
    tile: image,
}

export component AppWindow inherits Window {
    title: "Third Eye Client";
    icon: @image-url("../assets/logo.png");
    width: 1520px;
    height: 960px;

    in-out property <int> active_screen: 0;

    in-out property <string> rtsp_url;
    in-out property <string> rov_http_base;
    in-out property <string> rov_status_udp_bind_host;
    in-out property <string> rov_status_udp_port;
    in-out property <string> osm_tile_user_agent;
    in-out property <string> rov_info;

    in-out property <string> map_status;
    in-out property <string> corelocation_debug;
    in-out property <string> lat_lon_text;
    in-out property <string> zoom_text;
    in property <[MapTile]> map_tiles;
    in-out property <length> map_pin_world_x: 0px;
    in-out property <length> map_pin_world_y: 0px;
    in-out property <bool> map_has_pin: false;
    in-out property <length> map_viewport_x: 0px;
    in-out property <length> map_viewport_y: 0px;
    in-out property <length> map_viewport_width: 0px;
    in-out property <length> map_viewport_height: 0px;

    in-out property <string> stream_status;
    in-out property <string> frames_received_text;
    in-out property <image> stream_image;
    in-out property <bool> has_stream_image: false;

    in-out property <string> rov_status_text;
    in-out property <string> rov_packets_received_text;
    in-out property <bool> has_rov_status: false;
    in-out property <string> rov_attitude_text;
    in-out property <string> rov_depth_temp_text;
    in-out property <string> rov_coordinates_text;
    in-out property <string> rov_imu_text;
    in-out property <string> rov_batteries_text;

    callback navigate_configuration();
    callback navigate_map(length, length);
    callback navigate_stream();

    callback use_default_test_rtsp();
    callback use_default_rov_rtsp();
    callback use_default_rov_http_base();
    callback use_host_from_rov_http_base();
    callback use_default_rov_status_udp_port();
    callback use_default_osm_tile_user_agent();

    callback list_medias();
    callback capture_photo();

    callback detect_location(length, length);
    callback load_map_tile(length, length);
    callback open_interactive_map();
    callback map_flicked(length, length, length, length);
    callback map_zoom_in(length, length, length, length);
    callback map_zoom_out(length, length, length, length);
    callback center_map_on_pin(length, length, length, length);

    callback start_stream();
    callback stop_stream();
    callback start_rov_status_listener();
    callback stop_rov_status_listener();

    public function set_map_viewport(ox: length, oy: length, width: length, height: length) {
        root.map_viewport_x = ox;
        root.map_viewport_y = oy;
        root.map_viewport_width = width;
        root.map_viewport_height = height;
    }
    HorizontalBox {
        padding: 10px;
        spacing: 10px;

        Rectangle {
            min-width: 240px;
            max-width: 240px;
            border-width: 1px;
            border-color: #3f4148;
            background: #1f2127;

            VerticalBox {
                padding: 12px;
                spacing: 8px;
                Image {
                    width: 90px;
                    height: 70px;
                    source: @image-url("../assets/logo.png");
                    image-fit: contain;
                }

                Text {
                    text: "Third Eye Client";
                    font-size: 26px;
                }
                Text {
                    text: "Navigation";
                    color: #8f96a3;
                }
                Rectangle {
                    height: 1px;
                    background: #3f4148;
                }

                Button {
                    text: "Configuration";
                    clicked => { root.navigate_configuration(); }
                }
                Button {
                    text: "Device Map";
                    clicked => { root.navigate_map(content_panel.width, content_panel.height); }
                }
                Button {
                    text: "Live Stream";
                    clicked => { root.navigate_stream(); }
                }
                Rectangle {
                    vertical-stretch: 1;
                }
            }
        }

        content_panel := Rectangle {
            horizontal-stretch: 1;
            vertical-stretch: 1;
            border-width: 1px;
            border-color: #3f4148;
            background: #202328;

            VerticalBox {
                padding: 14px;
                spacing: 10px;

                if root.active_screen == 0 : VerticalBox {
                    spacing: 8px;
                    Text {
                        text: "RTSP + ROV Configuration";
                        font-size: 24px;
                    }
                    Text {
                        text: "Set RTSP URLs and ROV HTTP endpoint. These values are used by the Stream and API actions.";
                        wrap: word-wrap;
                    }

                    Text { text: "RTSP URL:"; }
                    LineEdit { text <=> root.rtsp_url; }
                    HorizontalBox {
                        spacing: 8px;
                        Button {
                            horizontal-stretch: 1;
                            text: "Use default test RTSP URL";
                            clicked => { root.use_default_test_rtsp(); }
                        }
                        Button {
                            horizontal-stretch: 1;
                            text: "Use default ROV RTSP URL";
                            clicked => { root.use_default_rov_rtsp(); }
                        }
                    }

                    Text { text: "ROV HTTP API Base URL:"; }
                    LineEdit { text <=> root.rov_http_base; }
                    HorizontalBox {
                        spacing: 8px;
                        Button {
                            horizontal-stretch: 1;
                            text: "Use default ROV HTTP API URL";
                            clicked => { root.use_default_rov_http_base(); }
                        }
                        Button {
                            horizontal-stretch: 1;
                            text: "Use host from ROV HTTP API URL for telemetry UDP bind";
                            clicked => { root.use_host_from_rov_http_base(); }
                        }
                    }

                    Text { text: "ROV telemetry UDP bind host:"; }
                    LineEdit { text <=> root.rov_status_udp_bind_host; }
                    Text { text: "ROV telemetry UDP port:"; }
                    LineEdit { text <=> root.rov_status_udp_port; }
                    Button {
                        text: "Use default ROV telemetry UDP port (8500)";
                        clicked => { root.use_default_rov_status_udp_port(); }
                    }

                    Text { text: "OpenStreetMap tile User-Agent:"; }
                    LineEdit { text <=> root.osm_tile_user_agent; }
                    Button {
                        text: "Use default OSM tile User-Agent";
                        clicked => { root.use_default_osm_tile_user_agent(); }
                    }
                    Text {
                        text: "Include an app identifier and contact URL/email for OSM tile policy compliance.";
                        wrap: word-wrap;
                    }

                    Text { text: "ROV API notes:"; }
                    Text {
                        text: "• RTSP stream example: rtsp://admin:admin@192.168.1.88:8554/stream/0/0";
                        wrap: word-wrap;
                    }
                    Text {
                        text: "• HTTP API server example: http://192.168.1.88:80";
                        wrap: word-wrap;
                    }
                    Text {
                        text: "• Capture endpoint: POST /v1/capture";
                        wrap: word-wrap;
                    }
                    Text {
                        text: "• Media list endpoint: GET /v1/medias";
                        wrap: word-wrap;
                    }

                    HorizontalBox {
                        spacing: 8px;
                        Button {
                            horizontal-stretch: 1;
                            text: "List medias (GET /v1/medias)";
                            clicked => { root.list_medias(); }
                        }
                        Button {
                            horizontal-stretch: 1;
                            text: "Capture photo (POST /v1/capture)";
                            clicked => { root.capture_photo(); }
                        }
                    }

                    Text { text: root.rov_info; wrap: word-wrap; }
                }

                if root.active_screen == 1 : VerticalBox {
                    spacing: 8px;
                    Text {
                        text: "Device Location on OpenStreetMap";
                        font-size: 24px;
                    }
                    Text {
                        text: "This desktop app uses native location when available, with IP geolocation fallback.";
                        wrap: word-wrap;
                    }

                    HorizontalBox {
                        spacing: 8px;
                        Button {
                            horizontal-stretch: 1;
                            text: "Detect location";
                            clicked => { root.detect_location(map_canvas.width, map_canvas.height); }
                        }
                        Button {
                            horizontal-stretch: 1;
                            text: "Refresh visible tiles";
                            clicked => { root.load_map_tile(map_canvas.width, map_canvas.height); }
                        }
                        Button {
                            horizontal-stretch: 1;
                            text: "Open interactive map in browser";
                            clicked => { root.open_interactive_map(); }
                        }
                    }

                    Text { text: "Coordinates: " + root.lat_lon_text; }
                    Text { text: "Zoom: " + root.zoom_text; }
                    Text { text: root.corelocation_debug; wrap: word-wrap; }
                    Text { text: root.map_status; wrap: word-wrap; }

                    map_canvas := Rectangle {
                        border-width: 1px;
                        border-color: #5f5f5f;
                        min-height: 320px;
                        horizontal-stretch: 1;
                        vertical-stretch: 1;
                        clip: true;
                        map_fli := Flickable {
                            viewport-x <=> root.map_viewport_x;
                            viewport-y <=> root.map_viewport_y;
                            viewport-width: root.map_viewport_width;
                            viewport-height: root.map_viewport_height;

                            for tile in root.map_tiles : Image {
                                x: tile.x;
                                y: tile.y;
                                width: tile.size;
                                height: tile.size;
                                source: tile.tile;
                                image-fit: fill;
                            }
                            if root.map_has_pin : Rectangle {
                                width: 52px;
                                height: 52px;
                                x: root.map_pin_world_x - self.width / 2;
                                y: root.map_pin_world_y - self.height / 2;
                                background: #00000000;

                                Rectangle {
                                    width: 52px;
                                    height: 52px;
                                    border-radius: 26px;
                                    background: #0a84ff15;
                                }
                                Rectangle {
                                    width: 42px;
                                    height: 42px;
                                    x: (parent.width - self.width) / 2;
                                    y: (parent.height - self.height) / 2;
                                    border-radius: 21px;
                                    background: #0a84ff28;
                                }
                                Rectangle {
                                    width: 34px;
                                    height: 34px;
                                    x: (parent.width - self.width) / 2;
                                    y: (parent.height - self.height) / 2;
                                    border-radius: 17px;
                                    background: #0a84ff40;
                                }
                                Image {
                                    width: 26px;
                                    height: 26px;
                                    x: (parent.width - self.width) / 2;
                                    y: (parent.height - self.height) / 2;
                                    source: @image-url("../assets/macbook_pin.png");
                                    image-fit: contain;
                                }
                            }


                            flicked => {
                                root.map_flicked(map_fli.viewport-x, map_fli.viewport-y, map_canvas.width, map_canvas.height);
                            }
                        }


                        if root.map_tiles.length == 0 : Text {
                            text: "Loading map tiles...";
                            horizontal-alignment: center;
                            vertical-alignment: center;
                        }

                        // Map control button group – top-right
                        Rectangle {
                            width: 46px;
                            height: 132px;
                            x: parent.width - self.width - 10px;
                            y: 10px;
                            border-radius: 12px;
                            background: #0d1a2acc;
                            border-width: 1px;
                            border-color: #0a84ff44;

                            // Zoom-in button
                            Rectangle {
                                width: 40px;
                                height: 40px;
                                x: 3px;
                                y: 3px;
                                border-radius: 10px;
                                background: btn-plus-ta.pressed ? #0a84ff77 : btn-plus-ta.has-hover ? #0a84ff44 : #0a84ff18;
                                animate background { duration: 120ms; }
                                Text {
                                    text: "+";
                                    font-size: 26px;
                                    color: #ffffff;
                                    horizontal-alignment: center;
                                    vertical-alignment: center;
                                }
                                btn-plus-ta := TouchArea {
                                    clicked => {
                                        root.map_zoom_in(
                                            map_fli.viewport-x,
                                            map_fli.viewport-y,
                                            map_canvas.width,
                                            map_canvas.height
                                        );
                                    }
                                }
                            }

                            // Separator
                            Rectangle {
                                width: 28px;
                                height: 1px;
                                x: (parent.width - self.width) / 2;
                                y: 44px;
                                background: #0a84ff33;
                            }

                            // Zoom-out button
                            Rectangle {
                                width: 40px;
                                height: 40px;
                                x: 3px;
                                y: 46px;
                                border-radius: 10px;
                                background: btn-minus-ta.pressed ? #0a84ff77 : btn-minus-ta.has-hover ? #0a84ff44 : #0a84ff18;
                                animate background { duration: 120ms; }
                                Text {
                                    text: "−";
                                    font-size: 26px;
                                    color: #ffffff;
                                    horizontal-alignment: center;
                                    vertical-alignment: center;
                                }
                                btn-minus-ta := TouchArea {
                                    clicked => {
                                        root.map_zoom_out(
                                            map_fli.viewport-x,
                                            map_fli.viewport-y,
                                            map_canvas.width,
                                            map_canvas.height
                                        );
                                    }
                                }
                            }

                            // Separator
                            Rectangle {
                                width: 28px;
                                height: 1px;
                                x: (parent.width - self.width) / 2;
                                y: 87px;
                                background: #0a84ff33;
                            }

                            // Center / locate button
                            Rectangle {
                                width: 40px;
                                height: 40px;
                                x: 3px;
                                y: 89px;
                                border-radius: 10px;
                                background: btn-center-ta.pressed ? #0a84ff77 : btn-center-ta.has-hover ? #0a84ff44 : #0a84ff18;
                                animate background { duration: 120ms; }

                                // Crosshair ring
                                Rectangle {
                                    width: 16px;
                                    height: 16px;
                                    x: (parent.width - self.width) / 2;
                                    y: (parent.height - self.height) / 2;
                                    border-width: 2px;
                                    border-color: #ffffff;
                                    border-radius: 8px;
                                    background: #00000000;
                                }
                                // Center dot
                                Rectangle {
                                    width: 4px;
                                    height: 4px;
                                    x: (parent.width - self.width) / 2;
                                    y: (parent.height - self.height) / 2;
                                    border-radius: 2px;
                                    background: #ffffff;
                                }
                                // Crosshair top
                                Rectangle {
                                    width: 2px;
                                    height: 5px;
                                    x: (parent.width - self.width) / 2;
                                    y: (parent.height - 16px) / 2 - self.height;
                                    background: #ffffff;
                                }
                                // Crosshair bottom
                                Rectangle {
                                    width: 2px;
                                    height: 5px;
                                    x: (parent.width - self.width) / 2;
                                    y: (parent.height + 16px) / 2;
                                    background: #ffffff;
                                }
                                // Crosshair left
                                Rectangle {
                                    width: 5px;
                                    height: 2px;
                                    x: (parent.width - 16px) / 2 - self.width;
                                    y: (parent.height - self.height) / 2;
                                    background: #ffffff;
                                }
                                // Crosshair right
                                Rectangle {
                                    width: 5px;
                                    height: 2px;
                                    x: (parent.width + 16px) / 2;
                                    y: (parent.height - self.height) / 2;
                                    background: #ffffff;
                                }

                                btn-center-ta := TouchArea {
                                    clicked => {
                                        root.center_map_on_pin(
                                            map_fli.viewport-x,
                                            map_fli.viewport-y,
                                            map_canvas.width,
                                            map_canvas.height
                                        );
                                    }
                                }
                            }
                        }

                    }

                }

                if root.active_screen == 2 : VerticalBox {
                    spacing: 8px;
                    Text {
                        text: "RTSP Live Stream";
                        font-size: 24px;
                    }
                    Text {
                        text: "Current stream URL (shared from configuration screen): " + root.rtsp_url;
                        wrap: word-wrap;
                    }
                    Text {
                        text: "ROV telemetry bind target: " + root.rov_status_udp_bind_host + ":" + root.rov_status_udp_port;
                        wrap: word-wrap;
                    }

                    HorizontalBox {
                        spacing: 8px;
                        Button {
                            horizontal-stretch: 1;
                            text: "Start embedded stream";
                            clicked => { root.start_stream(); }
                        }
                        Button {
                            horizontal-stretch: 1;
                            text: "Stop stream";
                            clicked => { root.stop_stream(); }
                        }
                    }
                    HorizontalBox {
                        spacing: 8px;
                        Button {
                            horizontal-stretch: 1;
                            text: "Start ROV status listener";
                            clicked => { root.start_rov_status_listener(); }
                        }
                        Button {
                            horizontal-stretch: 1;
                            text: "Stop ROV status listener";
                            clicked => { root.stop_rov_status_listener(); }
                        }
                    }

                    Text { text: root.stream_status; wrap: word-wrap; }
                    Text { text: "Frames received: " + root.frames_received_text; }
                    Text { text: root.rov_status_text; wrap: word-wrap; }
                    Text { text: "Status packets received: " + root.rov_packets_received_text; }

                    if root.has_rov_status : VerticalBox {
                        spacing: 4px;
                        Text { text: "Latest ROV status"; font-size: 18px; }
                        Text { text: root.rov_attitude_text; wrap: word-wrap; }
                        Text { text: root.rov_depth_temp_text; wrap: word-wrap; }
                        Text { text: root.rov_coordinates_text; wrap: word-wrap; }
                        Text { text: root.rov_imu_text; wrap: word-wrap; }
                        Text { text: root.rov_batteries_text; wrap: word-wrap; }
                    }

                    Rectangle {
                        border-width: 1px;
                        border-color: #5f5f5f;
                        min-height: 320px;
                        horizontal-stretch: 1;
                        vertical-stretch: 1;
                        clip: true;

                        if root.has_stream_image : Image {
                            width: parent.width;
                            height: parent.height;
                            source: root.stream_image;
                            image-fit: contain;
                        }
                        if !root.has_stream_image : Text {
                            text: "No frames rendered yet.";
                            horizontal-alignment: center;
                            vertical-alignment: center;
                        }
                    }
                }
            }
        }
    }
}
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Configuration,
    Map,
    Stream,
}

impl Screen {
    const fn index(self) -> i32 {
        match self {
            Self::Configuration => 0,
            Self::Map => 1,
            Self::Stream => 2,
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
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            rtsp_url: DEFAULT_TEST_RTSP.to_owned(),
            rov_http_base: DEFAULT_ROV_HTTP_BASE.to_owned(),
            rov_status_udp_bind_host: default_rov_udp_bind_host(),
            rov_status_udp_port: ROV_STATUS_UDP_PORT.to_string(),
            osm_tile_user_agent: DEFAULT_OSM_TILE_USER_AGENT.to_owned(),
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
}

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
    parse_host_from_http_base(DEFAULT_ROV_HTTP_BASE).unwrap_or_else(|| "0.0.0.0".to_owned())
}

#[derive(Default)]
struct MapState {
    lat: Option<f64>,
    lon: Option<f64>,
    zoom: u32,
    status: String,
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

struct MapTilesState {
    client: Client,
    loaded_tiles: BTreeMap<TileCoordinate, Image>,
    loading_tiles: BTreeSet<TileCoordinate>,
    tile_cache: BTreeMap<TileCoordinate, Image>,
    fallback_zoom: Option<u32>,
    tile_result_tx: mpsc::Sender<TileLoadResult>,
    tile_result_rx: Receiver<TileLoadResult>,
    visible_width: f64,
    visible_height: f64,
    offset_x: f64,
    offset_y: f64,
}

impl MapTilesState {
    fn new() -> Self {
        let (tile_result_tx, tile_result_rx) = mpsc::channel();
        Self {
            client: Client::new(),
            loaded_tiles: BTreeMap::new(),
            loading_tiles: BTreeSet::new(),
            tile_cache: BTreeMap::new(),
            fallback_zoom: None,
            tile_result_tx,
            tile_result_rx,
            visible_width: MAP_IMAGE_SIZE_PX as f64,
            visible_height: MAP_IMAGE_SIZE_PX as f64,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }

    fn world_size_px(zoom_level: u32) -> f64 {
        (MAP_TILE_SIZE_PX as f64) * f64::exp2(zoom_level as f64)
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

    fn update_visible_size(&mut self, width: f64, height: f64, zoom_level: u32) -> bool {
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

    fn center_on_location(&mut self, lat: f64, lon: f64, zoom_level: u32) {
        let world_size = Self::world_size_px(zoom_level);
        let x_world = ((lon + 180.0) / 360.0) * world_size;
        let lat_rad = lat.to_radians();
        let y_world =
            ((1.0 - (lat_rad.tan() + (1.0 / lat_rad.cos())).ln() / PI) / 2.0) * world_size;
        self.offset_x = x_world - (self.visible_width / 2.0);
        self.offset_y = y_world - (self.visible_height / 2.0);
        self.clamp_offset_to_world(zoom_level);
    }

    fn set_offset_from_viewport(&mut self, viewport_x: f64, viewport_y: f64, zoom_level: u32) {
        self.offset_x = -viewport_x;
        self.offset_y = -viewport_y;
        self.clamp_offset_to_world(zoom_level);
    }

    fn set_zoom_level(&mut self, current_zoom: u32, new_zoom: u32, focus_x: f64, focus_y: f64) {
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

    fn zoom_focus_center(&self) -> (f64, f64) {
        (self.visible_width / 2.0, self.visible_height / 2.0)
    }

    fn center_lat_lon(&self, zoom_level: u32) -> Option<(f64, f64)> {
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

    fn viewport_for_slint(&self, zoom_level: u32) -> (f32, f32, f32, f32) {
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
        let scale = f64::exp2((target_zoom as i32 - current_zoom as i32) as f64);
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

    fn request_visible_tiles(&mut self, zoom_level: u32, user_agent: &str) {
        const MAX_TILE_CACHE: usize = 500;
        if self.tile_cache.len() > MAX_TILE_CACHE {
            self.tile_cache.retain(|c, _| {
                (c.z as i32 - zoom_level as i32).unsigned_abs() <= 2
            });
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
                        let outcome = load_osm_tile(client, coord, &user_agent)
                            .map(|frame| TileLoadResult {
                                coord,
                                frame: Some(frame),
                                error: None,
                            })
                            .unwrap_or_else(|err| TileLoadResult {
                                coord,
                                frame: None,
                                error: Some(format!(
                                    "Failed loading tile z{} x{} y{}: {err:#}",
                                    coord.z, coord.x, coord.y
                                )),
                            });
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

    fn poll_loaded_tiles(&mut self, zoom_level: u32) -> (bool, Option<String>) {
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

    fn tile_model(&self, render_zoom: u32) -> ModelRc<MapTile> {
        let model = VecModel::from(
            self.loaded_tiles
                .iter()
                .filter(|(coord, _)| {
                    coord.z == render_zoom
                        || self
                            .fallback_zoom
                            .is_some_and(|fallback_zoom| coord.z == fallback_zoom)
                })
                .map(|(coord, image)| MapTile {
                    x: (coord.x as f32)
                        * (MAP_TILE_SIZE_PX as f32)
                        * 2.0_f32.powi(render_zoom as i32 - coord.z as i32),
                    y: (coord.y as f32)
                        * (MAP_TILE_SIZE_PX as f32)
                        * 2.0_f32.powi(render_zoom as i32 - coord.z as i32),
                    size: (MAP_TILE_SIZE_PX as f32)
                        * 2.0_f32.powi(render_zoom as i32 - coord.z as i32),
                    tile: image.clone(),
                })
                .collect::<Vec<_>>(),
        );
        ModelRc::new(model)
    }
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
}

impl ThirdEyeState {
    fn new() -> Self {
        Self {
            active_screen: Screen::Configuration,
            last_screen: Screen::Configuration,
            suppress_next_map_flick: false,
            config: AppConfig::default(),
            map: MapState {
                zoom: DEFAULT_ZOOM,
                ..MapState::default()
            },
            map_tiles: MapTilesState::new(),
            rov_info: String::new(),
            stream: StreamState::default(),
            rov_status: UdpStatusState::default(),
        }
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
                    "{success_message} Map tiles are loading."
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

#[derive(Clone)]
struct RgbaFrame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Default)]
struct StreamState {
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
    ui.set_rov_status_udp_bind_host(state.config.rov_status_udp_bind_host.clone().into());
    ui.set_rov_status_udp_port(state.config.rov_status_udp_port.clone().into());
    ui.set_osm_tile_user_agent(state.config.osm_tile_user_agent.clone().into());
    ui.set_rov_info(state.rov_info.clone().into());
    apply_map_runtime_to_ui(ui, state);
    apply_stream_and_rov_runtime_to_ui(ui, state);
}

fn apply_map_runtime_to_ui(ui: &AppWindow, state: &ThirdEyeState) {
    ui.set_map_status(state.map.status.clone().into());
    ui.set_zoom_text(state.map.zoom.to_string().into());
    let lat_lon = match (state.map.lat, state.map.lon) {
        (Some(lat), Some(lon)) => format!("{lat:.6}, {lon:.6}"),
        _ => "n/a".to_owned(),
    };
    ui.set_lat_lon_text(lat_lon.into());
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
    let (viewport_x, viewport_y, viewport_width, viewport_height) =
        state.map_tiles.viewport_for_slint(state.map.zoom);
    ui.invoke_set_map_viewport(viewport_x, viewport_y, viewport_width, viewport_height);
    ui.set_map_tiles(state.map_tiles.tile_model(state.map.zoom));
    apply_stream_and_rov_runtime_to_ui(ui, state);
}

fn lat_lon_to_world_px(lat: f64, lon: f64, zoom_level: u32) -> (f32, f32) {
    let world_size = MapTilesState::world_size_px(zoom_level);
    let lon = lon.clamp(-180.0, 180.0);
    let lat = lat.clamp(-85.051_128_78, 85.051_128_78);
    let x_world = (((lon + 180.0) / 360.0) * world_size).clamp(0.0, world_size);
    let lat_rad = lat.to_radians();
    let y_world = ((1.0 - (lat_rad.tan() + (1.0 / lat_rad.cos())).ln() / PI) / 2.0) * world_size;
    let y_world = y_world.clamp(0.0, world_size);
    (x_world as f32, y_world as f32)
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
    } else {
        ui.set_has_rov_status(false);
        ui.set_rov_attitude_text("".into());
        ui.set_rov_depth_temp_text("".into());
        ui.set_rov_coordinates_text("".into());
        ui.set_rov_imu_text("".into());
        ui.set_rov_batteries_text("".into());
    }
}

fn pull_configuration_from_ui(ui: &AppWindow, state: &mut ThirdEyeState) {
    state.config.rtsp_url = ui.get_rtsp_url().to_string();
    state.config.rov_http_base = ui.get_rov_http_base().to_string();
    state.config.rov_status_udp_bind_host = ui.get_rov_status_udp_bind_host().to_string();
    state.config.rov_status_udp_port = ui.get_rov_status_udp_port().to_string();
    state.config.osm_tile_user_agent = ui.get_osm_tile_user_agent().to_string();
}

fn rgba_frame_to_slint_image(frame: &RgbaFrame) -> Image {
    let shared_buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        frame.rgba.as_slice(),
        frame.width,
        frame.height,
    );
    Image::from_rgba8(shared_buffer)
}

fn register_callbacks(ui: &AppWindow, state: Rc<RefCell<ThirdEyeState>>) {
    let ui_weak = ui.as_weak();
    let state_for_configuration = Rc::clone(&state);
    ui.on_navigate_configuration(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_configuration.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state);
        state.active_screen = Screen::Configuration;
        state.last_screen = Screen::Configuration;
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
            state.set_map_visible_size(viewport_width as f64, viewport_height as f64);
            state.set_map_viewport(viewport_x as f64, viewport_y as f64);
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
            state.set_map_visible_size(viewport_width as f64, viewport_height as f64);
            state.set_map_viewport(viewport_x as f64, viewport_y as f64);
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
            state.set_map_visible_size(viewport_width as f64, viewport_height as f64);
            state.set_map_viewport(viewport_x as f64, viewport_y as f64);
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
            state.set_map_visible_size(viewport_width as f64, viewport_height as f64);
            state.map_tiles.fallback_zoom = None;
            match detect_location(&mut state.map) {
                Ok(location) => {
                    state.map.lat = Some(location.lat);
                    state.map.lon = Some(location.lon);
                    state.load_map_tile_for_current_location(format!(
                        "Centered on device location via {}: lat={:.6}, lon={:.6}.",
                        location.source, location.lat, location.lon
                    ));
                }
                Err(err) => {
                    if state.map.lat.is_some() && state.map.lon.is_some() {
                        state.load_map_tile_for_current_location(format!(
                            "Centered on last known location (detection unavailable: {err:#})."
                        ));
                    } else {
                        state.map.status =
                            format!("Cannot center: no location available ({err:#}).");
                    }
                }
            }
            state.suppress_next_map_flick = true;
            apply_map_runtime_to_ui(&ui, &state);
        },
    );

    let ui_weak = ui.as_weak();
    let state_for_map_navigation = Rc::clone(&state);
    ui.on_navigate_map(move |content_width, content_height| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_map_navigation.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state);
        state.active_screen = Screen::Map;
        // Estimate map canvas size from content panel (minus padding/header)
        let est_width = (content_width as f64 - 30.0).max(320.0);
        let est_height = (content_height as f64 - 180.0).max(320.0);
        state.set_map_visible_size(est_width, est_height);
        state.map_tiles.fallback_zoom = None;
        state.auto_refresh_map_on_tab_enter();
        state.last_screen = Screen::Map;
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_stream_navigation = Rc::clone(&state);
    ui.on_navigate_stream(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_stream_navigation.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state);
        state.active_screen = Screen::Stream;
        state.last_screen = Screen::Stream;
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_default_test_rtsp = Rc::clone(&state);
    ui.on_use_default_test_rtsp(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_default_test_rtsp.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        state.config.rtsp_url = DEFAULT_TEST_RTSP.to_owned();
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_default_rov_rtsp = Rc::clone(&state);
    ui.on_use_default_rov_rtsp(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_default_rov_rtsp.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        state.config.rtsp_url = DEFAULT_ROV_RTSP.to_owned();
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_default_rov_http = Rc::clone(&state);
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
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_use_host_from_base = Rc::clone(&state);
    ui.on_use_host_from_rov_http_base(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_use_host_from_base.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state);
        if let Some(host) = parse_host_from_http_base(&state.config.rov_http_base) {
            state.config.rov_status_udp_bind_host = host;
        } else {
            state.rov_info = "Could not extract host from ROV HTTP API URL.".to_owned();
        }
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_default_rov_udp_port = Rc::clone(&state);
    ui.on_use_default_rov_status_udp_port(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_default_rov_udp_port.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        state.config.rov_status_udp_port = ROV_STATUS_UDP_PORT.to_string();
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_default_osm_ua = Rc::clone(&state);
    ui.on_use_default_osm_tile_user_agent(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_default_osm_ua.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        state.config.osm_tile_user_agent = DEFAULT_OSM_TILE_USER_AGENT.to_owned();
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_list_medias = Rc::clone(&state);
    ui.on_list_medias(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_list_medias.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state);
        let client = RovApiClient::new(state.config.rov_http_base.clone());
        state.rov_info = match client.list_medias() {
            Ok(names) => {
                if names.is_empty() {
                    "No media names detected in response.".to_owned()
                } else {
                    format!("Media files:\n{}", names.join("\n"))
                }
            }
            Err(err) => format!("List medias failed: {err:#}"),
        };
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_capture = Rc::clone(&state);
    ui.on_capture_photo(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_capture.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state);
        let client = RovApiClient::new(state.config.rov_http_base.clone());
        state.rov_info = match client.capture() {
            Ok(()) => "Capture request sent successfully (HTTP 2xx).".to_owned(),
            Err(err) => format!("Capture failed: {err:#}"),
        };
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_detect_location = Rc::clone(&state);
    ui.on_detect_location(move |viewport_width, viewport_height| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_detect_location.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state);
        state.set_map_visible_size(viewport_width as f64, viewport_height as f64);
        match detect_location(&mut state.map) {
            Ok(location) => {
                state.map.lat = Some(location.lat);
                state.map.lon = Some(location.lon);
                let success_message = format!(
                    "Detected location via {}: lat={:.6}, lon={:.6}",
                    location.source, location.lat, location.lon
                );
                state.load_map_tile_for_current_location(format!(
                    "{success_message}. Map auto-refreshed."
                ));
            }
            Err(err) => {
                state.map.status = format!("Failed to detect location: {err:#}");
            }
        }
        apply_state_to_ui(&ui, &state);
    });

    let ui_weak = ui.as_weak();
    let state_for_load_map_tile = Rc::clone(&state);
    ui.on_load_map_tile(move |viewport_width, viewport_height| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_load_map_tile.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state);
        state.set_map_visible_size(viewport_width as f64, viewport_height as f64);
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
    ui.on_start_stream(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_start_stream.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state);
        state.stream.stop();
        let rtsp_url = state.config.rtsp_url.clone();
        state.stream.status = match state.stream.start(rtsp_url) {
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
    ui.on_start_rov_status_listener(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let mut state = match state_for_start_rov_listener.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return,
        };
        pull_configuration_from_ui(&ui, &mut state);
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
        if let Err(err) = state.rov_status.start(&bind_host, port) {
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

fn configure_slint_style() {
    if std::env::var_os("SLINT_STYLE").is_none() {
        // SAFETY: Called in main before UI initialization or background threads.
        unsafe {
            std::env::set_var("SLINT_STYLE", "cupertino");
        }
    }
}

fn main() -> Result<()> {
    configure_slint_style();
    let ui = AppWindow::new().context("failed to initialize Slint window")?;
    let state = Rc::new(RefCell::new(ThirdEyeState::new()));
    state.borrow_mut().initialize_location_on_startup();

    {
        let state = state.borrow();
        apply_state_to_ui(&ui, &state);
    }

    register_callbacks(&ui, Rc::clone(&state));

    let ui_weak = ui.as_weak();
    let poll_state = Rc::clone(&state);
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
            if let Some(frame) = state.stream.poll_events() {
                ui.set_stream_image(rgba_frame_to_slint_image(&frame));
                ui.set_has_stream_image(true);
            }
            let current_zoom = state.map.zoom;
            let (map_changed, map_error) = state.map_tiles.poll_loaded_tiles(current_zoom);
            let has_map_error = map_error.is_some();
            if let Some(error) = map_error {
                state.map.status = error;
                state.request_visible_map_tiles();
            }
            if map_changed || has_map_error {
                apply_map_runtime_to_ui(&ui, &state);
            }
            state.rov_status.poll_events();
            apply_stream_and_rov_runtime_to_ui(&ui, &state);
        },
    );

    ui.run()
        .map_err(|err| anyhow::anyhow!("failed to run GUI app: {err}"))?;

    if let Ok(mut state) = state.try_borrow_mut() {
        state.stream.stop();
        state.rov_status.stop();
    }

    Ok(())
}
