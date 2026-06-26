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
    patch_space_service_controls();
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

fn patch_space_service_controls() {
    let path = Path::new("src/core_control_main.rs");
    let mut text = fs::read_to_string(path)
        .expect("read generated core-control UI source")
        .replace("\r\n", "\n");
    let original = text.clone();

    text = text.replace(
        "                let addon_count = row\n                    .get(\"addons\")\n                    .and_then(|x| x.as_object())\n                    .map(|addons| addons.len())\n                    .unwrap_or(0);",
        "                let addon_count = row\n                    .get(\"addon_count\")\n                    .and_then(|x| x.as_u64())\n                    .map(|count| count as usize)\n                    .or_else(|| {\n                        row.get(\"addons\")\n                            .and_then(|x| x.as_object())\n                            .map(|addons| addons.len())\n                    })\n                    .unwrap_or(0);",
    );

    if !text.contains("SetSpaceAddonEnabled {") {
        text = text.replace(
            "    LeaveSpace {\n        space_id: String,\n    },\n    PollEvents {",
            "    LeaveSpace {\n        space_id: String,\n    },\n    SetSpaceAddonEnabled {\n        space_id: String,\n        addon_id: String,\n        enabled: bool,\n    },\n    PollEvents {",
        );
    }

    if !text.contains("ApiJob::SetSpaceAddonEnabled { space_id, addon_id, enabled } => json!") {
        text = text.replace(
            "            ApiJob::LeaveSpace { space_id } => json!({\n                \"cmd\": \"leave_space\",\n                \"space_id\": space_id\n            }),",
            "            ApiJob::LeaveSpace { space_id } => json!({\n                \"cmd\": \"leave_space\",\n                \"space_id\": space_id\n            }),\n            ApiJob::SetSpaceAddonEnabled { space_id, addon_id, enabled } => json!({\n                \"cmd\": \"set_space_addon_enabled\",\n                \"space_id\": space_id,\n                \"addon_id\": addon_id,\n                \"enabled\": enabled\n            }),",
        );
    }

    if !text.contains("ApiJob::SetSpaceAddonEnabled { .. } => \"set_space_addon_enabled\"") {
        text = text.replace(
            "        ApiJob::LeaveSpace { .. } => \"leave_space\",",
            "        ApiJob::LeaveSpace { .. } => \"leave_space\",\n        ApiJob::SetSpaceAddonEnabled { .. } => \"set_space_addon_enabled\",",
        );
    }

    if !text.contains("\"set_space_addon_enabled\" => {") {
        text = text.replace(
            "            \"leave_space\" => {\n                self.log(\"Left space.\");\n                self.send_job(ApiJob::Spaces);\n                self.send_job(ApiJob::Addons);\n            }",
            "            \"leave_space\" => {\n                self.log(\"Left space.\");\n                self.send_job(ApiJob::Spaces);\n                self.send_job(ApiJob::Addons);\n            }\n            \"set_space_addon_enabled\" => {\n                self.log(\"Space add-on setting updated.\");\n                self.send_job(ApiJob::Spaces);\n                self.send_job(ApiJob::Addons);\n            }",
        );
    }

    if !text.contains("for addon in self.addons.clone() {") {
        text = text.replace(
            "                                ui.with_layout(\n                                    egui::Layout::right_to_left(egui::Align::Center),\n                                    |ui| {\n                                        if space.can_manage_addons {\n                                            state_chip(ui, \"Owner managed\", color_success());\n                                        } else {\n                                            state_chip(ui, \"Owner controlled\", color_muted());\n                                        }\n                                    },\n                                );\n                            });",
            "                                ui.with_layout(\n                                    egui::Layout::right_to_left(egui::Align::Center),\n                                    |ui| {\n                                        if space.can_manage_addons {\n                                            state_chip(ui, \"Owner managed\", color_success());\n                                        } else {\n                                            state_chip(ui, \"Owner controlled\", color_muted());\n                                        }\n                                    },\n                                );\n                            });\n\n                            ui.separator();\n\n                            if self.addons.is_empty() {\n                                ui.label(\n                                    egui::RichText::new(\"Install or reload add-ons before assigning them to this space.\")\n                                        .color(color_muted())\n                                        .size(12.5),\n                                );\n                            } else {\n                                for addon in self.addons.clone() {\n                                    ui.horizontal_wrapped(|ui| {\n                                        ui.vertical(|ui| {\n                                            ui.label(egui::RichText::new(&addon.name).color(color_text()).strong());\n                                            ui.label(\n                                                egui::RichText::new(format!(\"{} · {}\", addon.id, addon.services.join(\", \")))\n                                                    .color(color_muted())\n                                                    .size(12.0),\n                                            );\n                                        });\n\n                                        if space.can_manage_addons && !matches!(space.local_state.as_str(), \"removed\" | \"left\") {\n                                            if ui\n                                                .add(primary_button(\"Enable\"))\n                                                .on_hover_cursor(egui::CursorIcon::PointingHand)\n                                                .clicked()\n                                            {\n                                                self.send_job(ApiJob::SetSpaceAddonEnabled {\n                                                    space_id: space.id.clone(),\n                                                    addon_id: addon.id.clone(),\n                                                    enabled: true,\n                                                });\n                                            }\n\n                                            if ui\n                                                .add(danger_button(\"Disable\"))\n                                                .on_hover_cursor(egui::CursorIcon::PointingHand)\n                                                .clicked()\n                                            {\n                                                self.send_job(ApiJob::SetSpaceAddonEnabled {\n                                                    space_id: space.id.clone(),\n                                                    addon_id: addon.id.clone(),\n                                                    enabled: false,\n                                                });\n                                            }\n                                        } else {\n                                            state_chip(ui, \"Owner controlled\", color_muted());\n                                        }\n                                    });\n                                }\n                            }",
        );
    }

    if text != original {
        fs::write(path, text).expect("write patched core-control UI source");
    }
}
