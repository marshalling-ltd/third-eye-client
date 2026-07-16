//! Integration tests for the persistent `AppStore`.
//!
//! These tests target the on-disk behaviour: opening, closing, reopening, and
//! making sure everything roundtrips through `state.db`.

use std::path::PathBuf;

use third_eye_client::camera::{CameraApiClient, MediaInfo, MediaOrigin};
use third_eye_client::storage::AppStore;
use third_eye_client::storage::config::{ClientConfig, ClientConfigDefaults};
use third_eye_client::storage::media::download_to_local;
use third_eye_client::storage::outbox::OutboxRequest;

const DEFAULTS: ClientConfigDefaults<'static> = ClientConfigDefaults {
    rtsp_url: "rtsp://default",
    rov_http_base: "http://default",
    rov_udp_bind_host: "0.0.0.0",
    rov_udp_port: "8500",
    osm_tile_user_agent: "ua/0",
    server_base_url: "https://third-eye.marshalling.eu",
    rov_network_interface: "",
    nmea_gps_port: "11123",
    nmea_gps_mode: "0",
    nmea_serial_port: "",
    nmea_server_host: "",
    nmea_server_port: "11123",
    nmea_stale_timeout: "10",
    nmea_gps_protocol: "0",
    use_saved_map_tiles: "false",
    max_tile_storage_mb: "1024",
};

fn make_db_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "third-eye-client-test-{}-{}.db",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn config_and_outbox_survive_reopen() {
    let path = make_db_path("config-outbox");
    {
        let store = AppStore::open_at(&path).expect("open store");
        let cfg = ClientConfig {
            rtsp_url: "rtsp://persisted".into(),
            rov_http_base: "http://cam.test".into(),
            rov_udp_bind_host: "127.0.0.1".into(),
            rov_udp_port: "9000".into(),
            osm_tile_user_agent: "ua/persisted".into(),
            server_base_url: "https://api.test".into(),
            rov_network_interface: "en10".into(),
            nmea_gps_port: "11123".into(),
            nmea_gps_mode: "0".into(),
            nmea_serial_port: String::new(),
            nmea_server_host: String::new(),
            nmea_server_port: "11123".into(),
            nmea_stale_timeout: "10".into(),
            nmea_gps_protocol: "0".into(),
            use_saved_map_tiles: "false".into(),
            max_tile_storage_mb: "1024".into(),
        };
        store.config().save_client(&cfg).unwrap();

        let req = OutboxRequest::new_with_random_key("POST", "https://api.test/x");
        store.outbox().enqueue(&req).unwrap();
        assert_eq!(store.outbox().pending_count().unwrap(), 1);
        // Explicit shutdown is idempotent; `Drop` also calls it.
        store.shutdown();
    }

    let store = AppStore::open_at(&path).expect("reopen store");
    let loaded = store.config().load_client(&DEFAULTS).unwrap();
    assert_eq!(loaded.rtsp_url, "rtsp://persisted");
    assert_eq!(loaded.server_base_url, "https://api.test");
    assert_eq!(store.outbox().pending_count().unwrap(), 1);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn media_sync_is_idempotent_across_reopens() {
    use third_eye_client::camera::{MediaInfo, MediaOrigin};

    fn sample(name: &str, id: &str) -> MediaInfo {
        MediaInfo {
            name: name.into(),
            size: 100,
            canplayback: false,
            origin: MediaOrigin {
                width: 0,
                height: 0,
                duration: 0,
                fps: 0,
                br: 0,
                multi: 0,
                with_osd: false,
                id: id.into(),
                stat: 0,
            },
            play: None,
            osd: None,
        }
    }

    let path = make_db_path("media-sync");
    {
        let store = AppStore::open_at(&path).expect("open store");
        let report = store
            .media()
            .apply_rov_listing(&[sample("a.jpeg", "id-a")], None)
            .unwrap();
        assert_eq!(report.new_media, 1);
    }
    let store = AppStore::open_at(&path).expect("reopen store");
    let report = store
        .media()
        .apply_rov_listing(&[sample("a.jpeg", "id-a")], None)
        .unwrap();
    assert_eq!(report.new_media, 0, "reopen sees existing row");
    assert_eq!(report.updated_media, 1);
    assert_eq!(store.media().list_recent(10).unwrap().len(), 1);
    let _ = std::fs::remove_file(&path);
}

fn media_info(name: &str, id: &str, size: u64) -> MediaInfo {
    MediaInfo {
        name: name.into(),
        size,
        canplayback: false,
        origin: MediaOrigin {
            width: 0,
            height: 0,
            duration: 0,
            fps: 0,
            br: 0,
            multi: 0,
            with_osd: false,
            id: id.into(),
            stat: 0,
        },
        play: None,
        osd: None,
    }
}

#[test]
fn download_to_local_writes_file_and_updates_row() {
    let mut server = mockito::Server::new();
    let payload = b"fake-jpeg-bytes".to_vec();
    let download = server
        .mock("GET", "/v1/medias/a.jpeg/download")
        .with_status(200)
        .with_header("content-type", "image/jpeg")
        .with_body(payload.clone())
        .create();

    let db_path = make_db_path("download-to-local");
    let data_root = db_path.parent().unwrap().to_path_buf();
    let store = AppStore::open_at(&db_path).unwrap();
    // Seed the media_sync row that `download_to_local` will update.
    store
        .media()
        .apply_rov_listing(&[media_info("a.jpeg", "id-a", payload.len() as u64)], None)
        .unwrap();

    let camera = CameraApiClient::new(server.url());
    let local = download_to_local(store.media(), &camera, &data_root, "id-a", "a.jpeg")
        .expect("download succeeds");
    download.assert();

    // File exists with the exact bytes mockito served.
    assert_eq!(std::fs::read(&local).unwrap(), payload);
    let sep = std::path::MAIN_SEPARATOR;
    let expected = format!("{sep}media{sep}a.jpeg");
    assert!(local.to_string_lossy().contains(&expected));

    // media_sync row was updated with the local path and SHA.
    let rec = store
        .media()
        .list_recent(10)
        .unwrap()
        .into_iter()
        .find(|r| r.media_id == "id-a")
        .unwrap();
    assert_eq!(
        rec.local_path.as_deref(),
        Some(local.to_string_lossy().as_ref())
    );
    assert!(rec.local_sha256.as_ref().is_some_and(|h| h.len() == 64));

    // Cleanup.
    let _ = std::fs::remove_file(&local);
    let _ = std::fs::remove_dir(data_root.join("media"));
    let _ = std::fs::remove_file(&db_path);
}
