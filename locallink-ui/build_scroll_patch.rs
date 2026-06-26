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
    patch_spaces_delete_controls();
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

fn patch_spaces_delete_controls() {
    let path = Path::new("src/core_control_main.rs");
    let mut text = fs::read_to_string(path)
        .expect("read generated core-control UI source")
        .replace("\r\n", "\n");
    let original = text.clone();

    if !text.contains("DeleteSpace {") {
        text = text.replace(
            "    LeaveSpace {\n        space_id: String,\n    },\n    PollEvents {",
            "    LeaveSpace {\n        space_id: String,\n    },\n    DeleteSpace {\n        space_id: String,\n    },\n    PollEvents {",
        );
    }

    if !text.contains("ApiJob::DeleteSpace { space_id } => json!") {
        text = text.replace(
            "            ApiJob::LeaveSpace { space_id } => json!({\n                \"cmd\": \"leave_space\",\n                \"space_id\": space_id\n            }),",
            "            ApiJob::LeaveSpace { space_id } => json!({\n                \"cmd\": \"leave_space\",\n                \"space_id\": space_id\n            }),\n            ApiJob::DeleteSpace { space_id } => json!({\n                \"cmd\": \"delete_space\",\n                \"space_id\": space_id\n            }),",
        );
    }

    if !text.contains("ApiJob::DeleteSpace { .. } => \"delete_space\"") {
        text = text.replace(
            "        ApiJob::LeaveSpace { .. } => \"leave_space\",",
            "        ApiJob::LeaveSpace { .. } => \"leave_space\",\n        ApiJob::DeleteSpace { .. } => \"delete_space\",",
        );
    }

    if !text.contains("\"delete_space\" => {") {
        text = text.replace(
            "            \"leave_space\" => {\n                self.log(\"Left space.\");\n                self.send_job(ApiJob::Spaces);\n                self.send_job(ApiJob::Addons);\n            }",
            "            \"leave_space\" => {\n                self.log(\"Left space.\");\n                self.send_job(ApiJob::Spaces);\n                self.send_job(ApiJob::Addons);\n            }\n            \"delete_space\" => {\n                self.log(\"Space deleted.\");\n                self.send_job(ApiJob::Spaces);\n                self.send_job(ApiJob::Addons);\n            }",
        );
    }

    if !text.contains("Delete Local Copy") {
        text = text.replace(
            "                        if space.can_leave && ui\n                            .add(danger_button(\"Leave Group\"))\n                            .on_hover_cursor(egui::CursorIcon::PointingHand)\n                            .clicked()\n                        {\n                            self.send_job(ApiJob::LeaveSpace { space_id: space.id.clone() });\n                        }",
            "                        if space.can_leave && ui\n                            .add(danger_button(\"Leave Group\"))\n                            .on_hover_cursor(egui::CursorIcon::PointingHand)\n                            .clicked()\n                        {\n                            self.send_job(ApiJob::LeaveSpace { space_id: space.id.clone() });\n                        }\n\n                        let can_delete_local_copy = space.local_state == \"removed\" || space.local_state == \"left\";\n                        if (space.role == \"owner\" || can_delete_local_copy) && ui\n                            .add(danger_button(if space.role == \"owner\" { \"Delete Space\" } else { \"Delete Local Copy\" }))\n                            .on_hover_cursor(egui::CursorIcon::PointingHand)\n                            .clicked()\n                        {\n                            self.send_job(ApiJob::DeleteSpace { space_id: space.id.clone() });\n                        }",
        );
    }

    text = text.replace(
        "Disconnect only affects local activity. Leave exits a foreign group.",
        "Disconnect only affects local activity. Leave exits a foreign group. Deleted/removed foreign spaces can be cleared locally.",
    );
    text = text.replace(
        "Delete removes an owned space for everyone.",
        "Delete removes an owned space for everyone. Delete Local Copy only clears a removed foreign space from this device.",
    );

    if text != original {
        fs::write(path, text).expect("write patched core-control UI source");
    }
}
