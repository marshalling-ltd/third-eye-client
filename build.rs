use std::env;
use std::path::PathBuf;

fn main() {
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent-dark".to_owned()),
    )
    .expect("Slint compilation failed");

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
