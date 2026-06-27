use std::fs;
use std::path::Path;

pub fn run() {
    println!("cargo:rerun-if-changed=build_phase14_ui.rs");
    let path = Path::new("src/core_control_main.rs");
    let mut text = fs::read_to_string(path)
        .expect("read generated core-control UI source")
        .replace("\r\n", "\n");
    let original = text.clone();

    text = text.replace(
        "Owned spaces and joined spaces are separate. Pending invitations must be accepted before they can connect.",
        "Groups are persistent spaces. Direct device connections appear here temporarily while connected so you can choose local add-ons for that connection.",
    );

    text = text.replace("Create owned space", "Create group");
    text = text.replace(
        "Spaces created here are owned by this device. Only owned spaces can invite or remove members.",
        "Groups created here are persistent. The owner can invite and remove members; each device chooses its own local add-ons.",
    );
    text = text.replace("Create Owned Space", "Create Group");
    text = text.replace("Gaming PC space", "Gaming group");

    text = text.replace(
        r#"            ui.horizontal_wrapped(|ui| {
                ui.label("Kind");
                let mut direct = !self.space_kind_group;
                let mut group = self.space_kind_group;

                if ui.radio_value(&mut direct, true, "Direct").clicked() {
                    self.space_kind_group = false;
                }
                if ui.radio_value(&mut group, true, "Group").clicked() {
                    self.space_kind_group = true;
                }
            });

            ui.add_space(8.0);

"#,
        "",
    );

    text = text.replace(
        "let kind = if self.space_kind_group { \"group\" } else { \"direct\" }.to_string();",
        "let kind = \"group\".to_string();",
    );

    text = text.replace(
        "Create an owned space above, or wait for a space invite from another device.",
        "Create a group above, accept a group invite, or connect directly to a nearby device.",
    );

    text = text.replace(
        r#"                            let owner_summary = if space.role == "owner" {
                                "Owned by this device".to_string()
                            } else if space.owner_device_id.is_empty() {
                                "Joined foreign space".to_string()
                            } else {
                                format!("Owner: {}", ellipsize(&space.owner_device_id, 30))
                            };
"#,
        r#"                            let owner_summary = if space.local_state == "direct" {
                                "Temporary direct connection".to_string()
                            } else if space.role == "owner" {
                                "Owned by this device".to_string()
                            } else if space.owner_device_id.is_empty() {
                                "Joined group".to_string()
                            } else {
                                format!("Owner: {}", ellipsize(&space.owner_device_id, 30))
                            };
"#,
    );

    text = text.replace(
        r#"                                "owned" => color_accent(),
                                "joined" => color_success(),
                                "invite_pending" => color_warning(),
                                "removed" | "left" => color_error(),
"#,
        r#"                                "owned" => color_accent(),
                                "joined" => color_success(),
                                "direct" => color_success(),
                                "invite_pending" => color_warning(),
                                "removed" | "left" => color_error(),
"#,
    );

    text = text.replace(
        r#"                                "owned" => "Owned",
                                "joined" => "Joined",
                                "invite_pending" => "Invite pending",
"#,
        r#"                                "owned" => "Owned group",
                                "joined" => "Joined group",
                                "direct" => "Direct connection",
                                "invite_pending" => "Invite pending",
"#,
    );

    text = text.replace(
        r#"                            state_chip(ui, &space.kind, if space.kind == "group" { color_accent() } else { color_success() });
"#,
        r#"                            let context_label = if space.local_state == "direct" { "Connection" } else { "Group" };
                            state_chip(ui, context_label, if space.local_state == "direct" { color_success() } else { color_accent() });
"#,
    );

    text = text.replace(
        "if matches!(space.local_state.as_str(), \"owned\" | \"joined\") {",
        "if matches!(space.local_state.as_str(), \"owned\" | \"joined\" | \"direct\") {",
    );
    text = text.replace(
        "let can_manage_local_addons = matches!(space.local_state.as_str(), \"owned\" | \"joined\");",
        "let can_manage_local_addons = matches!(space.local_state.as_str(), \"owned\" | \"joined\" | \"direct\");",
    );

    text = text.replace(
        "Install or reload add-ons before assigning them to this local space setup.",
        "Install or reload add-ons before assigning them to this local connection setup.",
    );

    if text != original {
        fs::write(path, text).expect("write phase14 UI source");
    }
}
