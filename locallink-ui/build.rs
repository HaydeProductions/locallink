use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src/main.rs");

    let source = fs::read_to_string("src/main.rs").expect("read src/main.rs");
    let mut generated = source;

    let process_use = "use std::process::{Child, Command, Stdio};\n";
    if !generated.contains("std::os::windows::process::CommandExt") {
        generated = generated.replace(
            process_use,
            &(process_use.to_owned()
                + "#[cfg(windows)]\n"
                + "use std::os::windows::process::CommandExt;\n"),
        );
    }

    let api_const = "const LOCAL_API_ADDR: &str = \"127.0.0.1:47900\";\n";
    if !generated.contains("CREATE_NO_WINDOW") {
        generated = generated.replace(
            api_const,
            &(api_const.to_owned()
                + "\n#[cfg(windows)]\n"
                + "const CREATE_NO_WINDOW: u32 = 0x08000000;\n"),
        );
    }

    generated = generated.replace("stdout(Stdio::inherit())", "stdout(Stdio::null())");
    generated = generated.replace("stderr(Stdio::inherit())", "stderr(Stdio::null())");

    if !generated.contains("fn prepare_hidden_child") {
        generated = generated.replace(
            "fn launch_addon(addon: &AddonRow) -> Result<Child> {\n",
            "fn prepare_hidden_child(cmd: &mut Command) -> &mut Command {\n    cmd.stdin(Stdio::null())\n        .stdout(Stdio::null())\n        .stderr(Stdio::null());\n\n    #[cfg(windows)]\n    cmd.creation_flags(CREATE_NO_WINDOW);\n\n    cmd\n}\n\nfn launch_addon(addon: &AddonRow) -> Result<Child> {\n",
        );
    }

    generated = generated.replace(
        "    let child = Command::new(&exe_path)\n        .current_dir(Path::new(&addon.addon_dir))\n        .stdin(Stdio::null())\n        .stdout(Stdio::null())\n        .stderr(Stdio::null())\n        .spawn()\n        .with_context(|| format!(\"launching {}\", exe_path.display()))?;\n",
        "    let mut cmd = Command::new(&exe_path);\n    let child = prepare_hidden_child(cmd.current_dir(Path::new(&addon.addon_dir)))\n        .spawn()\n        .with_context(|| format!(\"launching {}\", exe_path.display()))?;\n",
    );

    generated = generated.replace(
        "    Command::new(core)\n        .current_dir(dir)\n        .stdin(Stdio::null())\n        .stdout(Stdio::null())\n        .stderr(Stdio::null())\n        .spawn()?;\n",
        "    let mut cmd = Command::new(core);\n    prepare_hidden_child(cmd.current_dir(dir)).spawn()?;\n",
    );

    fs::write(Path::new("src/windowless_main.rs"), generated).expect("write generated UI main");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => {
            println!("cargo:rustc-link-arg-bin=locallink-ui=/SUBSYSTEM:WINDOWS");
            println!("cargo:rustc-link-arg-bin=locallink-ui=/ENTRY:mainCRTStartup");
        }
        _ => println!("cargo:rustc-link-arg-bin=locallink-ui=-mwindows"),
    }
}
