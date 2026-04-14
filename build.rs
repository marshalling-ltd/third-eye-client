use std::env;
use std::path::PathBuf;

fn main() {
    // Load .env at compile time and re-export vars so env!() works in source.
    if let Ok(iter) = dotenvy::dotenv_iter() {
        for item in iter.flatten() {
            println!("cargo:rustc-env={}={}", item.0, item.1);
        }
    }
    println!("cargo:rerun-if-changed=.env");
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "macos" {
        return;
    }

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));
    let plist_path = manifest_dir.join("macos").join("Info.plist");

    println!("cargo:rerun-if-changed={}", plist_path.display());
    println!(
        "cargo:rustc-link-arg-bin=third-eye-client=-Wl,-sectcreate,__TEXT,__info_plist,{}",
        plist_path.display()
    );
}
