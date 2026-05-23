fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => {
            println!("cargo:rustc-link-arg-bin=locallink-core=/SUBSYSTEM:WINDOWS");
            println!("cargo:rustc-link-arg-bin=locallink-core=/ENTRY:mainCRTStartup");
        }
        _ => println!("cargo:rustc-link-arg-bin=locallink-core=-mwindows"),
    }
}
