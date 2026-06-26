#[allow(dead_code)]
mod generated_ui_build {
    include!("build.rs");

    pub fn run() {
        main();
    }
}

use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    generated_ui_build::run();
    patch_spaces_page_scroll();
}

fn patch_spaces_page_scroll() {
    let path = Path::new("src/core_control_main.rs");
    let mut text = fs::read_to_string(path)
        .expect("read generated core-control UI source")
        .replace("\r\n", "\n");
    let original = text.clone();

    let start = text
        .find("    fn screen_spaces(&mut self, ui: &mut egui::Ui) {")
        .expect("find Spaces screen function");

    let open_pat = "        egui::ScrollArea::vertical().show(ui, |ui| {\n            for space in self.spaces.clone() {";
    let open_repl = "        for space in self.spaces.clone() {";
    let Some(open_at) = text[start..].find(open_pat).map(|offset| start + offset) else {
        // Already patched, or the Spaces UI was rewritten to avoid nested scrolls.
        return;
    };
    text = format!(
        "{}{}{}",
        &text[..open_at],
        open_repl,
        &text[open_at + open_pat.len()..]
    );

    let marker = "\nimpl LocalLinkUi {\n    fn network_requirements_panel";
    let search_end = text[open_at..]
        .find(marker)
        .map(|offset| open_at + offset)
        .or_else(|| text[open_at..].find("\nfn run_network_repair").map(|offset| open_at + offset))
        .expect("find end of Spaces UI block");

    let close_pat = "\n            }\n        });\n    }\n}";
    let close_repl = "\n            }\n    }\n}";
    let close_at = text[..search_end]
        .rfind(close_pat)
        .expect("find nested Spaces scroll closing block");
    text = format!(
        "{}{}{}",
        &text[..close_at],
        close_repl,
        &text[close_at + close_pat.len()..]
    );

    if text != original {
        fs::write(path, text).expect("write patched core-control UI source");
    }
}
