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
#[cfg(target_os = "macos")]
use map::corelocation_debug_status;
use map::{
    MapState, MapTilesState, RgbaFrame, ViewportAnimation, compute_scale_bar, detect_location,
    ease_out_cubic, lat_lon_to_world_px, rgba_frame_to_slint_image, DEFAULT_OSM_TILE_USER_AGENT,
    DEFAULT_ZOOM, MAX_ZOOM, MIN_ZOOM,
};
use reqwest::Url;
use slint::{ComponentHandle, ModelRc, VecModel};
use third_eye_client::camera::{CameraApiClient, MediaScene, PhotoFormat};
use third_eye_client::rov_status::{ROV_STATUS_UDP_PORT, UdpStatusState};

const DEFAULT_TEST_RTSP: &str = "rtsp://admin:admin@127.0.0.1:8554/stream";
const DEFAULT_ROV_RTSP: &str = "rtsp://admin:admin@192.168.1.88:8554/stream/0/0";
const DEFAULT_ROV_HTTP_BASE: &str = "http://192.168.1.88";

slint::slint! {
import { Button, HorizontalBox, LineEdit, ScrollView, VerticalBox } from "std-widgets.slint";

export struct MapTile {
    x: length,
    y: length,
    size: length,
    tile: image,
}

export component AppWindow inherits Window {
    title: "Third Eye Client";
    icon: @image-url("../assets/logo.png");
    preferred-width: 1520px;
    preferred-height: 960px;
    min-width: 680px;
    min-height: 400px;

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
    in-out property <string> pin_lat_lon_short;
    in property <length> scale_bar_width: 100px;
    in-out property <string> scale_bar_text;

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

            if root.active_screen != 1 : ScrollView {
                viewport-width: self.visible-width;

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

            // Full-bleed map screen
            if root.active_screen == 1 : Rectangle {
                width: parent.width;
                height: parent.height;

                map_canvas := Rectangle {
                    width: parent.width;
                    height: parent.height;
                    clip: true;
                    background: #ffffff;

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

                        // Coordinate label below pin
                        if root.map_has_pin : Rectangle {
                            width: 140px;
                            height: 20px;
                            x: root.map_pin_world_x - self.width / 2;
                            y: root.map_pin_world_y + 30px;
                            border-radius: 4px;
                            background: #000000aa;

                            Text {
                                text: root.pin_lat_lon_short;
                                font-size: 11px;
                                color: #ffffff;
                                horizontal-alignment: center;
                                vertical-alignment: center;
                            }
                        }

                        flicked => {
                            root.map_flicked(map_fli.viewport-x, map_fli.viewport-y, map_canvas.width, map_canvas.height);
                        }
                    }

                    if root.map_tiles.length == 0 : Text {
                        text: "Loading map tiles...";
                        color: #888888;
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
                                text: "\u{2212}";
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

                    // Scale legend \u{2013} bottom-right
                    Rectangle {
                        width: 148px;
                        height: 30px;
                        x: parent.width - self.width - 14px;
                        y: parent.height - self.height - 14px;
                        border-radius: 6px;
                        background: #0d1a2acc;
                        border-width: 1px;
                        border-color: #0a84ff44;

                        // Scale line
                        Rectangle {
                            width: root.scale_bar_width;
                            height: 2px;
                            x: (parent.width - self.width) / 2;
                            y: 6px;
                            background: #ffffff;
                        }
                        // Left tick
                        Rectangle {
                            width: 2px;
                            height: 8px;
                            x: (parent.width - root.scale_bar_width) / 2;
                            y: 3px;
                            background: #ffffff;
                        }
                        // Right tick
                        Rectangle {
                            width: 2px;
                            height: 8px;
                            x: (parent.width + root.scale_bar_width) / 2 - 1px;
                            y: 3px;
                            background: #ffffff;
                        }
                        // Scale label
                        Text {
                            text: root.scale_bar_text;
                            font-size: 11px;
                            color: #ffffff;
                            width: parent.width;
                            y: 12px;
                            horizontal-alignment: center;
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
    viewport_anim: Option<ViewportAnimation>,
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
            viewport_anim: None,
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
            state.viewport_anim = None;
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
            state.set_map_visible_size(viewport_width as f64, viewport_height as f64);
            state.set_map_viewport(viewport_x as f64, viewport_y as f64);
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
            state.set_map_visible_size(viewport_width as f64, viewport_height as f64);
            state.map_tiles.fallback_zoom = None;
            let (old_vp_x, old_vp_y, _, _) = state.map_tiles.viewport_for_slint(state.map.zoom);
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
            let (target_vp_x, target_vp_y, _, _) = state.map_tiles.viewport_for_slint(state.map.zoom);
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
        // Map fills the entire content panel
        let est_width = (content_width as f64).max(320.0);
        let est_height = (content_height as f64).max(320.0);
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
        let client = CameraApiClient::new(state.config.rov_http_base.clone());
        state.rov_info = match client.list_medias(None::<MediaScene>) {
            Ok(items) => {
                if items.is_empty() {
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
        let client = CameraApiClient::new(state.config.rov_http_base.clone());
        state.rov_info = match client.capture(PhotoFormat::Jpeg, 1) {
            Ok(resp) => {
                let msg = resp.msg.as_deref().unwrap_or("success");
                format!("Capture request sent successfully: {msg}")
            }
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
