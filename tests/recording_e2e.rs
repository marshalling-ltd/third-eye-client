//! Needs `chasing-simulator` running (RTSP on 127.0.0.1:8554) and ffmpeg on
//! `PATH`: `cargo test --test recording_e2e -- --ignored`.

use std::path::PathBuf;
use std::time::Duration;

use third_eye_client::recording::Recorder;
use third_eye_client::rov_status::{Imu, Status};

#[test]
#[ignore = "needs chasing-simulator running"]
fn records_video_with_a_telemetry_track() {
    let dir = std::env::temp_dir().join(format!("te-rec-{}", std::process::id()));
    let video = dir.join("dive.mp4");
    let mut rec = Recorder::start(
        PathBuf::from("ffmpeg"),
        "rtsp://admin:admin@127.0.0.1:8554/stream/0/0",
        video,
    )
    .expect("start");
    let status = Status {
        pitch: 0.0,
        roll: 0.0,
        yaw: 45.0,
        depth: 3.2,
        lat: 457_000_000,
        lon: 160_000_000,
        temperature: 20.0,
        batteries: vec![],
        imu: Imu::default(),
    };
    for _ in 0..12 {
        std::thread::sleep(Duration::from_millis(500));
        rec.push(&status);
    }
    let files = rec.stop().expect("stop");
    assert!(files.mux_error.is_none(), "{:?}", files.mux_error);
    println!(
        "size {} {:?}",
        std::fs::metadata(&files.video).unwrap().len(),
        files
    );
    assert!(std::fs::metadata(&files.video).unwrap().len() > 10_000);
    let probe = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,codec_name",
            "-of",
            "csv=p=0",
        ])
        .arg(&files.video)
        .output()
        .expect("ffprobe");
    let streams = String::from_utf8_lossy(&probe.stdout);
    println!("{streams}");
    assert!(streams.contains("video"), "{streams}");
    assert!(streams.contains("subtitle"), "{streams}");
    println!("{}", std::fs::read_to_string(&files.srt).unwrap());
    let _ = std::fs::remove_dir_all(dir);
}
