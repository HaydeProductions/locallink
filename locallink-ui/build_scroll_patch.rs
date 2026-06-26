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
    patch_space_purge_control();
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

fn patch_space_purge_control() {
    let path = Path::new("src/core_control_main.rs");
    let mut text = fs::read_to_string(path)
        .expect("read generated core-control UI source")
        .replace("\r\n", "\n");
    let original = text.clone();

    if !text.contains("PurgeSpace {") {
        text = text.replace(
            "    LeaveSpace {\n        space_id: String,\n    },\n    PollEvents {",
            "    LeaveSpace {\n        space_id: String,\n    },\n    PurgeSpace {\n        space_id: String,\n    },\n    PollEvents {",
        );
    }

    if !text.contains("ApiJob::PurgeSpace { space_id } => json!") {
        text = text.replace(
            "            ApiJob::LeaveSpace { space_id } => json!({\n                \"cmd\": \"leave_space\",\n                \"space_id\": space_id\n            }),",
            "            ApiJob::LeaveSpace { space_id } => json!({\n                \"cmd\": \"leave_space\",\n                \"space_id\": space_id\n            }),\n            ApiJob::PurgeSpace { space_id } => json!({\n                \"cmd\": \"purge_space\",\n                \"space_id\": space_id\n            }),",
        );
    }

    if !text.contains("ApiJob::PurgeSpace { .. } => \"purge_space\"") {
        text = text.replace(
            "        ApiJob::LeaveSpace { .. } => \"leave_space\",",
            "        ApiJob::LeaveSpace { .. } => \"leave_space\",\n        ApiJob::PurgeSpace { .. } => \"purge_space\",",
        );
    }

    if !text.contains("\"purge_space\" => {") {
        text = text.replace(
            "            \"leave_space\" => {\n                self.log(\"Left space.\");\n                self.send_job(ApiJob::Spaces);\n                self.send_job(ApiJob::Addons);\n            }",
            "            \"leave_space\" => {\n                self.log(\"Left space.\");\n                self.send_job(ApiJob::Spaces);\n                self.send_job(ApiJob::Addons);\n            }\n            \"purge_space\" => {\n                self.log(\"Space cleared.\");\n                self.send_job(ApiJob::Spaces);\n                self.send_job(ApiJob::Addons);\n            }",
        );
    }

    if !text.contains("can_clear_registered_space") {
        let icon = char::from_u32(0x1f5d1).unwrap();
        text = text.replace(
            "                        if space.can_leave && ui\n                            .add(danger_button(\"Leave Group\"))\n                            .on_hover_cursor(egui::CursorIcon::PointingHand)\n                            .clicked()\n                        {\n                            self.send_job(ApiJob::LeaveSpace { space_id: space.id.clone() });\n                        }",
            &format!("                        if space.can_leave && ui\n                            .add(danger_button(\"Leave Group\"))\n                            .on_hover_cursor(egui::CursorIcon::PointingHand)\n                            .clicked()\n                        {{\n                            self.send_job(ApiJob::LeaveSpace {{ space_id: space.id.clone() }});\n                        }}\n\n                        let can_clear_registered_space = matches!(space.local_state.as_str(), \"removed\" | \"left\");\n                        if (space.role == \"owner\" || can_clear_registered_space) && ui\n                            .add(danger_button(\"{}\"))\n                            .on_hover_cursor(egui::CursorIcon::PointingHand)\n                            .clicked()\n                        {{\n                            self.send_job(ApiJob::PurgeSpace {{ space_id: space.id.clone() }});\n                        }}", icon),
        );
    }

    text = text.replace(
        "                        ui.label(\n                            egui::RichText::new(\"Disconnect only affects local activity. Leave exits a foreign group.\")\n                                .color(color_muted())\n                                .size(12.5),\n                        );",
        "",
    );

    if text != original {
        fs::write(path, text).expect("write patched core-control UI source");
    }
}
