//! Needs `chasing-simulator` running (HTTP on 127.0.0.1:8080):
//! `cargo test --test simulator_media_e2e -- --ignored --nocapture`.

use third_eye_client::camera::{CameraApiClient, PhotoFormat};
use third_eye_client::storage::AppStore;
use third_eye_client::storage::media::download_to_local;

#[test]
#[ignore = "needs chasing-simulator running"]
fn capture_list_and_download_a_photo() {
    let dir = std::env::temp_dir().join(format!("te-media-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = AppStore::open_at(&dir.join("state.db")).unwrap();
    let camera = CameraApiClient::new("http://127.0.0.1:8080".to_owned());

    camera.capture(PhotoFormat::Jpeg, 1).expect("capture");
    let items = camera.list_medias(None).expect("list");
    let photo = items
        .iter()
        .filter(|m| {
            std::path::Path::new(&m.name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("jpeg"))
        })
        .max_by(|a, b| a.name.cmp(&b.name))
        .expect("a photo in the listing");
    println!(
        "listing: {:?}",
        items.iter().map(|m| &m.name).collect::<Vec<_>>()
    );
    store.media().apply_rov_listing(&items, None).unwrap();
    let local = download_to_local(store.media(), &camera, &dir, &photo.origin.id, &photo.name)
        .expect("download");
    println!(
        "saved {} ({} bytes)",
        local.display(),
        std::fs::metadata(&local).unwrap().len()
    );
    assert!(image::image_dimensions(&local).is_ok());
    let _ = std::fs::remove_dir_all(dir);
}

/// The app builds its capture client with the configured ROV interface.
#[test]
#[ignore = "needs chasing-simulator running"]
fn capture_through_an_interface_bound_client() {
    let iface = std::env::var("ROV_IFACE").unwrap_or_else(|_| "lo0".to_owned());
    let camera = CameraApiClient::new_bound("http://127.0.0.1:8080".to_owned(), Some(&iface));
    let result = camera.capture(PhotoFormat::Jpeg, 1);
    println!("bound to {iface}: {result:?}");
    result.expect("capture through bound client");
}
