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
    clippy::struct_field_names,
    clippy::items_after_statements
)]

#[cfg(target_os = "linux")]
pub mod bluez;
pub mod bt_port;
pub mod camera;
pub mod formatting;
#[cfg(target_os = "macos")]
pub mod iobt;
pub mod ip_location;
pub mod network;
pub mod nmea;
pub mod rov_status;
pub mod rtsp_forwarder;
#[cfg(feature = "test-tools")]
pub mod simulator;
pub mod storage;
pub mod update_check;
