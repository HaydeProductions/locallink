use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=../assets/locallink-tray.ico.b64");
    embed_windows_app_icon();
}

#[cfg(windows)]
fn embed_windows_app_icon() {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let icon_path = Path::new(&out_dir).join("locallink-ui.ico");
    let icon_b64 = include_str!("../assets/locallink-tray.ico.b64").trim();
    let icon_bytes = STANDARD.decode(icon_b64).expect("decode LocalLink icon");
    fs::write(&icon_path, icon_bytes).expect("write LocalLink UI icon");

    winresource::WindowsResource::new()
        .set_icon(icon_path.to_str().expect("icon path is utf-8"))
        .compile()
        .expect("compile Windows icon resource");
}

#[cfg(not(windows))]
fn embed_windows_app_icon() {}
