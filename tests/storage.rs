//! Integration tests for the persistent `AppStore`.
//!
//! These tests target the on-disk behaviour: opening, closing, reopening, and
//! making sure everything roundtrips through `state.db`.

use std::path::PathBuf;

use third_eye_client::storage::AppStore;
use third_eye_client::storage::config::{ClientConfig, ClientConfigDefaults};
use third_eye_client::storage::outbox::OutboxRequest;

const DEFAULTS: ClientConfigDefaults<'static> = ClientConfigDefaults {
    rtsp_url: "rtsp://default",
    rov_http_base: "http://default",
    rov_udp_bind_host: "0.0.0.0",
    rov_udp_port: "8500",
    osm_tile_user_agent: "ua/0",
    server_base_url: "https://third-eye.marshalling.eu",
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
