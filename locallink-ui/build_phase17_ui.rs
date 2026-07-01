use std::fs;
use std::path::Path;

pub fn run() {
    println!("cargo:rerun-if-changed=build_phase17_ui.rs");
    let path = Path::new("src/core_control_main.rs");
    let mut text = fs::read_to_string(path)
        .expect("read generated core-control UI source")
        .replace("\r\n", "\n");
    let original = text.clone();

    patch_spaces_page_scroll(&mut text);
    patch_addon_count(&mut text);
    patch_jobs(&mut text);
    patch_space_purge_button(&mut text);
    patch_space_addon_selection(&mut text);
    patch_command_diagnostics(&mut text);

    if text != original {
        fs::write(path, text).expect("write phase17 UI source");
    }
}

fn patch_spaces_page_scroll(text: &mut String) {
    let Some(start) = text.find("    fn screen_spaces(&mut self, ui: &mut egui::Ui) {") else {
        return;
    };

    let open_pat = "        egui::ScrollArea::vertical().show(ui, |ui| {\n            for space in self.spaces.clone() {";
    let open_repl = "        for space in self.spaces.clone() {";
    let Some(open_at) = text[start..].find(open_pat).map(|offset| start + offset) else {
        return;
    };

    *text = format!(
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
        .unwrap_or(text.len());

    let close_pat = "\n            }\n        });\n    }\n}";
    let close_repl = "\n            }\n    }\n}";
    if let Some(close_at) = text[..search_end].rfind(close_pat) {
        *text = format!(
            "{}{}{}",
            &text[..close_at],
            close_repl,
            &text[close_at + close_pat.len()..]
        );
    }
}

fn patch_addon_count(text: &mut String) {
    *text = text.replace(
        "                let addon_count = row\n                    .get(\"addons\")\n                    .and_then(|x| x.as_object())\n                    .map(|addons| addons.len())\n                    .unwrap_or(0);",
        "                let addon_count = row\n                    .get(\"addon_count\")\n                    .and_then(|x| x.as_u64())\n                    .map(|count| count as usize)\n                    .or_else(|| {\n                        row.get(\"addons\")\n                            .and_then(|x| x.as_object())\n                            .map(|addons| addons.len())\n                    })\n                    .unwrap_or(0);",
    );
}

fn patch_jobs(text: &mut String) {
    if !text.contains("SetSpaceAddonEnabled {") {
        *text = text.replace(
            "    LeaveSpace {\n        space_id: String,\n    },\n    PollEvents {",
            "    LeaveSpace {\n        space_id: String,\n    },\n    PurgeSpace {\n        space_id: String,\n    },\n    SetSpaceAddonEnabled {\n        space_id: String,\n        addon_id: String,\n        enabled: bool,\n    },\n    PollEvents {",
        );
    } else if !text.contains("PurgeSpace {") {
        *text = text.replace(
            "    LeaveSpace {\n        space_id: String,\n    },\n    SetSpaceAddonEnabled {",
            "    LeaveSpace {\n        space_id: String,\n    },\n    PurgeSpace {\n        space_id: String,\n    },\n    SetSpaceAddonEnabled {",
        );
    }

    if !text.contains("ApiJob::PurgeSpace { space_id } => json!") {
        *text = text.replace(
            "            ApiJob::LeaveSpace { space_id } => json!({\n                \"cmd\": \"leave_space\",\n                \"space_id\": space_id\n            }),",
            "            ApiJob::LeaveSpace { space_id } => json!({\n                \"cmd\": \"leave_space\",\n                \"space_id\": space_id\n            }),\n            ApiJob::PurgeSpace { space_id } => json!({\n                \"cmd\": \"purge_space\",\n                \"space_id\": space_id\n            }),",
        );
    }

    if !text.contains("ApiJob::SetSpaceAddonEnabled { space_id, addon_id, enabled } => json!") {
        *text = text.replace(
            "            ApiJob::PurgeSpace { space_id } => json!({\n                \"cmd\": \"purge_space\",\n                \"space_id\": space_id\n            }),",
            "            ApiJob::PurgeSpace { space_id } => json!({\n                \"cmd\": \"purge_space\",\n                \"space_id\": space_id\n            }),\n            ApiJob::SetSpaceAddonEnabled { space_id, addon_id, enabled } => json!({\n                \"cmd\": \"set_space_addon_enabled\",\n                \"space_id\": space_id,\n                \"addon_id\": addon_id,\n                \"enabled\": enabled\n            }),",
        );
    }

    if !text.contains("ApiJob::PurgeSpace { .. } => \"purge_space\"") {
        *text = text.replace(
            "        ApiJob::LeaveSpace { .. } => \"leave_space\",",
            "        ApiJob::LeaveSpace { .. } => \"leave_space\",\n        ApiJob::PurgeSpace { .. } => \"purge_space\",",
        );
    }

    if !text.contains("ApiJob::SetSpaceAddonEnabled { .. } => \"set_space_addon_enabled\"") {
        *text = text.replace(
            "        ApiJob::PurgeSpace { .. } => \"purge_space\",",
            "        ApiJob::PurgeSpace { .. } => \"purge_space\",\n        ApiJob::SetSpaceAddonEnabled { .. } => \"set_space_addon_enabled\",",
        );
    }

    if !text.contains("\"purge_space\" => {") {
        *text = text.replace(
            "            \"leave_space\" => {\n                self.log(\"Left space.\");\n                self.send_job(ApiJob::Spaces);\n                self.send_job(ApiJob::Addons);\n            }",
            "            \"leave_space\" => {\n                self.log(\"Left space.\");\n                self.send_job(ApiJob::Spaces);\n                self.send_job(ApiJob::Addons);\n            }\n            \"purge_space\" => {\n                self.log(\"Space removed.\");\n                self.send_job(ApiJob::Spaces);\n                self.send_job(ApiJob::Addons);\n            }",
        );
    }

    if !text.contains("\"set_space_addon_enabled\" => {") {
        *text = text.replace(
            "            \"purge_space\" => {\n                self.log(\"Space removed.\");\n                self.send_job(ApiJob::Spaces);\n                self.send_job(ApiJob::Addons);\n            }",
            "            \"purge_space\" => {\n                self.log(\"Space removed.\");\n                self.send_job(ApiJob::Spaces);\n                self.send_job(ApiJob::Addons);\n            }\n            \"set_space_addon_enabled\" => {\n                self.log(\"Space add-on setting updated.\");\n                self.send_job(ApiJob::Spaces);\n                self.send_job(ApiJob::Addons);\n            }",
        );
    }
}

fn patch_space_purge_button(text: &mut String) {
    if text.contains("Delete Group") {
        return;
    }

    *text = text.replace(
        "                        if space.can_leave && ui\n                            .add(danger_button(\"Leave Group\"))\n                            .on_hover_cursor(egui::CursorIcon::PointingHand)\n                            .clicked()\n                        {\n                            self.send_job(ApiJob::LeaveSpace { space_id: space.id.clone() });\n                        }",
        "                        if space.can_leave && ui\n                            .add(danger_button(\"Leave Group\"))\n                            .on_hover_cursor(egui::CursorIcon::PointingHand)\n                            .clicked()\n                        {\n                            self.send_job(ApiJob::LeaveSpace { space_id: space.id.clone() });\n                        }\n\n                        let can_delete_owned_group = space.role == \"owner\";\n                        let can_forget_local_space = matches!(space.local_state.as_str(), \"removed\" | \"left\" | \"invite_declined\");\n                        if (can_delete_owned_group || can_forget_local_space) && ui\n                            .add(danger_button(if can_delete_owned_group { \"Delete Group\" } else { \"Forget Space\" }))\n                            .on_hover_cursor(egui::CursorIcon::PointingHand)\n                            .clicked()\n                        {\n                            self.send_job(ApiJob::PurgeSpace { space_id: space.id.clone() });\n                        }",
    );

    *text = text.replace(
        "                        ui.label(\n                            egui::RichText::new(\"Disconnect only affects local activity. Leave exits a foreign group.\")\n                                .color(color_muted())\n                                .size(12.5),\n                        );",
        "                        ui.label(\n                            egui::RichText::new(\"Disconnect only pauses local activity. Leave exits a joined group. Delete removes an owned group.\")\n                                .color(color_muted())\n                                .size(12.5),\n                        );",
    );
}

fn patch_space_addon_selection(text: &mut String) {
    if text.contains("Phase17 local add-on selection") {
        return;
    }

    let marker = "                    ui.add_space(10.0);\n\n                    glass_panel(ui, |ui| {\n                        ui.heading(egui::RichText::new(\"Members\").color(color_text()).size(16.0));";
    let replacement = "                    ui.add_space(10.0);\n\n                    // Phase17 local add-on selection\n                    glass_panel(ui, |ui| {\n                        ui.heading(egui::RichText::new(\"Add-on selection\").color(color_text()).size(16.0));\n                        ui.label(\n                            egui::RichText::new(\"Choose which installed add-ons run for this local space or direct connection context.\")\n                                .color(color_muted())\n                                .size(12.5),\n                        );\n\n                        if self.addons.is_empty() {\n                            ui.add_space(6.0);\n                            ui.label(\n                                egui::RichText::new(\"No add-ons installed. Build/install add-ons, then reload or restart Core.\")\n                                    .color(color_muted())\n                                    .size(12.5),\n                            );\n                        } else {\n                            let can_manage_local_addons = matches!(space.local_state.as_str(), \"owned\" | \"joined\" | \"direct\");\n\n                            for addon in self.addons.clone() {\n                                ui.separator();\n                                ui.horizontal_wrapped(|ui| {\n                                    ui.vertical(|ui| {\n                                        ui.label(egui::RichText::new(&addon.name).color(color_text()).strong());\n                                        ui.label(\n                                            egui::RichText::new(format!(\"{} · {}\", addon.id, addon.services.join(\", \")))\n                                                .color(color_muted())\n                                                .size(12.0),\n                                        );\n                                    });\n\n                                    if can_manage_local_addons {\n                                        if ui\n                                            .add(primary_button(\"Enable\"))\n                                            .on_hover_cursor(egui::CursorIcon::PointingHand)\n                                            .clicked()\n                                        {\n                                            self.send_job(ApiJob::SetSpaceAddonEnabled {\n                                                space_id: space.id.clone(),\n                                                addon_id: addon.id.clone(),\n                                                enabled: true,\n                                            });\n                                        }\n\n                                        if ui\n                                            .add(danger_button(\"Disable\"))\n                                            .on_hover_cursor(egui::CursorIcon::PointingHand)\n                                            .clicked()\n                                        {\n                                            self.send_job(ApiJob::SetSpaceAddonEnabled {\n                                                space_id: space.id.clone(),\n                                                addon_id: addon.id.clone(),\n                                                enabled: false,\n                                            });\n                                        }\n                                    } else {\n                                        state_chip(ui, \"Unavailable\", color_muted());\n                                    }\n                                });\n                            }\n                        }\n                    });\n\n                    ui.add_space(10.0);\n\n                    glass_panel(ui, |ui| {\n                        ui.heading(egui::RichText::new(\"Members\").color(color_text()).size(16.0));";

    *text = text.replace(marker, replacement);
}

fn patch_command_diagnostics(text: &mut String) {
    if text.contains("action ApiJob for diagnostics") {
        return;
    }

    *text = text.replace(
        "    fn send_job(&mut self, job: ApiJob) {\n        self.loading_count += 1;",
        "    fn send_job(&mut self, job: ApiJob) {\n        self.loading_count += 1;\n        if matches!(job, ApiJob::CreateSpace { .. } | ApiJob::ActivateSpace { .. } | ApiJob::DeactivateSpace { .. } | ApiJob::LeaveSpace { .. } | ApiJob::PurgeSpace { .. } | ApiJob::SetSpaceAddonEnabled { .. }) {\n            eprintln!(\"[ui] action ApiJob for diagnostics: {:?}\", job);\n        }",
    );

    *text = text.replace(
        "        let result = api_request(request);",
        "        let log_api_job = matches!(job_name.as_str(), \"create_space\" | \"activate_space\" | \"deactivate_space\" | \"leave_space\" | \"purge_space\" | \"set_space_addon_enabled\");\n        if log_api_job {\n            eprintln!(\"[ui-api] request job={}\", job_name);\n        }\n        let result = api_request(request);",
    );

    *text = text.replace(
        "            Ok(value) => UiMsg::ApiOk {\n                job: job_name,\n                value,\n            },",
        "            Ok(value) => {\n                if log_api_job {\n                    eprintln!(\"[ui-api] response job={} ok=true\", job_name);\n                }\n                UiMsg::ApiOk {\n                    job: job_name,\n                    value,\n                }\n            },",
    );

    *text = text.replace(
        "            Err(error) => UiMsg::ApiErr {\n                job: job_name,\n                error: error.to_string(),\n            },",
        "            Err(error) => {\n                eprintln!(\"[ui-api] response job={} ok=false error={}\", job_name, error);\n                UiMsg::ApiErr {\n                    job: job_name,\n                    error: error.to_string(),\n                }\n            },",
    );
}
