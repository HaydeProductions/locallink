mod build_phase14_ui;
mod build_phase17_ui;

#[allow(dead_code)]
mod generated_ui_build {
    include!("build.rs");

    pub fn run() {
        main();
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    generated_ui_build::run();
    build_phase14_ui::run();
    build_phase17_ui::run();
}
