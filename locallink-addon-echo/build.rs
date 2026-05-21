fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => {
            println!("cargo:rustc-link-arg-bin=locallink-addon-echo=/SUBSYSTEM:WINDOWS");
            println!("cargo:rustc-link-arg-bin=locallink-addon-echo=/ENTRY:mainCRTStartup");
        }
        _ => println!("cargo:rustc-link-arg-bin=locallink-addon-echo=-mwindows"),
    }
}
