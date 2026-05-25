use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src/main.rs");

    let source = fs::read_to_string("src/main.rs")
        .expect("read src/main.rs")
        .replace("\r\n", "\n");
    let mut generated = format!(
        "#![cfg_attr(target_os = \"windows\", windows_subsystem = \"windows\")]\n\n{}",
        source
    );

    generated = generated.replace(
        "use std::process::{Child, Command, Stdio};\n",
        "use std::process::{Child, Command, Stdio};\n#[cfg(target_os = \"windows\")]\nuse std::os::windows::process::CommandExt;\n",
    );

    generated = must_replace(
        generated,
        "    fn start_core(&mut self) {\n        match start_sibling_core() {\n            Ok(()) => {\n                self.log(\"Starting LocalLink Core...\");\n                std::thread::sleep(Duration::from_millis(250));\n                self.refresh_all();\n            }\n            Err(e) => self.log(format!(\"Could not start core: {e}\")),\n        }\n    }\n\n",
        "    fn start_core(&mut self) {\n        force_stop_core_processes();\n        std::thread::sleep(Duration::from_millis(200));\n\n        match start_sibling_core() {\n            Ok(()) => {\n                self.log(\"Starting LocalLink Core...\");\n                std::thread::sleep(Duration::from_millis(250));\n                self.refresh_all();\n            }\n            Err(e) => self.log(format!(\"Could not start core: {e}\")),\n        }\n    }\n\n    fn stop_core(&mut self) {\n        self.send_job(ApiJob::Shutdown);\n        force_stop_core_processes();\n\n        self.status = None;\n        self.peers.clear();\n        self.connections.clear();\n        self.addons.clear();\n\n        self.log(\"Stopped LocalLink Core.\");\n    }\n\n",
    );

    generated = must_replace(
        generated,
        "                    if !self.core_online()\n                        && ui\n                            .add(primary_button(\"Start\"))\n                            .on_hover_cursor(egui::CursorIcon::PointingHand)\n                            .clicked()\n                    {\n                        self.start_core();\n                    }\n",
        "                    if self.core_online() {\n                        if ui\n                            .add(danger_button(\"Stop Core\"))\n                            .on_hover_cursor(egui::CursorIcon::PointingHand)\n                            .clicked()\n                        {\n                            self.stop_core();\n                        }\n                    } else if ui\n                        .add(primary_button(\"Start\"))\n                        .on_hover_cursor(egui::CursorIcon::PointingHand)\n                        .clicked()\n                    {\n                        self.start_core();\n                    }\n",
    );

    generated = generated.replace("secondary_button(\"Shutdown\")", "danger_button(\"Stop Core\")");

    generated = must_replace(
        generated,
        "            .show(ctx, |ui| {\n                ui.heading(\"Settings\");",
        "            .show(ctx, |ui| {\n                egui::ScrollArea::vertical()\n                    .auto_shrink([false, false])\n                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)\n                    .show(ui, |ui| {\n                        ui.heading(\"Settings\");",
    );

    generated = must_replace(
        generated,
        "                glass_panel(ui, |ui| {\n                    ui.heading(\"Messages\");\n\n                    egui::ScrollArea::vertical()\n                        .max_height(130.0)\n                        .show(ui, |ui| {\n                            for line in &self.log {\n                                ui.label(line);\n                            }\n                        });\n\n                    if ui\n                        .add(secondary_button(\"Clear\"))\n                        .on_hover_cursor(egui::CursorIcon::PointingHand)\n                        .clicked()\n                    {\n                        self.log.clear();\n                    }\n                });\n            });\n\n        self.show_settings = open;",
        "                glass_panel(ui, |ui| {\n                    ui.heading(\"Messages\");\n\n                    egui::ScrollArea::vertical()\n                        .max_height(130.0)\n                        .show(ui, |ui| {\n                            for line in &self.log {\n                                ui.label(line);\n                            }\n                        });\n\n                    if ui\n                        .add(secondary_button(\"Clear\"))\n                        .on_hover_cursor(egui::CursorIcon::PointingHand)\n                        .clicked()\n                    {\n                        self.log.clear();\n                    }\n                });\n                    });\n            });\n\n        self.show_settings = open;",
    );

    generated = must_replace(
        generated,
        "                ui.add_space(12.0);\n\n                glass_panel(ui, |ui| {\n                    ui.horizontal_wrapped(|ui| {\n                        ui.heading(\"Advanced\");",
        "                ui.add_space(12.0);\n\n                self.network_requirements_panel(ui);\n\n                ui.add_space(12.0);\n\n                glass_panel(ui, |ui| {\n                    ui.horizontal_wrapped(|ui| {\n                        ui.heading(\"Advanced\");",
    );

    generated = must_replace(
        generated,
        "    Command::new(core)\n        .current_dir(dir)\n        .stdin(Stdio::null())\n        .stdout(Stdio::inherit())\n        .stderr(Stdio::inherit())\n        .spawn()?;",
        "    let mut command = Command::new(core);\n    command\n        .current_dir(dir)\n        .stdin(Stdio::null())\n        .stdout(Stdio::null())\n        .stderr(Stdio::null());\n\n    #[cfg(target_os = \"windows\")]\n    command.creation_flags(0x08000000); // CREATE_NO_WINDOW\n\n    command.spawn()?;",
    );

    generated = must_replace(
        generated,
        "        self.reconcile_addon_processes();\n",
        "        // The UI only displays add-on state. Add-on processes are owned by the core/connection layer, not the UI.\n",
    );

    generated = must_replace(
        generated,
        "        if enabled {\n            match launch_addon(&addon_snapshot) {\n                Ok(child) => {\n                    self.addon_processes\n                        .insert(addon_snapshot.id.clone(), child);\n                    self.log(format!(\"Enabled {}\", addon_snapshot.name));\n                }\n                Err(e) => self.log(format!(\n                    \"{} was enabled but could not be launched: {e}\",\n                    addon_snapshot.name\n                )),\n            }\n        } else if let Some(mut child) = self.addon_processes.remove(&addon_snapshot.id) {\n            let _ = child.kill();\n            self.log(format!(\"Disabled {}\", addon_snapshot.name));\n        } else {\n            self.log(format!(\"Disabled {}\", addon_snapshot.name));\n        }\n\n        self.send_job(ApiJob::ReloadAddons);",
        "        if enabled {\n            self.log(format!(\"Enabled {}\", addon_snapshot.name));\n        } else {\n            self.log(format!(\"Disabled {}\", addon_snapshot.name));\n        }\n\n        self.send_job(ApiJob::ReloadAddons);",
    );

    generated.push_str(NETWORK_REQUIREMENTS_CODE);
    generated.push_str(PROCESS_CONTROL_CODE);

    fs::write(Path::new("src/core_control_main.rs"), generated)
        .expect("write generated UI entry point");
}

fn must_replace(input: String, from: &str, to: &str) -> String {
    let output = input.replace(from, to);

    if output == input {
        panic!("expected UI source pattern was not found while generating core-control entry point");
    }

    output
}

const NETWORK_REQUIREMENTS_CODE: &str = r#"

impl LocalLinkUi {
    fn network_requirements_panel(&mut self, ui: &mut egui::Ui) {
        glass_panel(ui, |ui| {
            ui.heading("Network setup");
            ui.label(
                egui::RichText::new(
                    "Checks Windows networking requirements and applies fixes when needed. Windows may ask you to allow administrator changes.",
                )
                .color(color_muted()),
            );

            ui.add_space(10.0);

            if ui
                .add(primary_button("Check and fix requirements"))
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                match run_network_repair() {
                    Ok(()) => self.log("Network requirements check started."),
                    Err(err) => self.log(format!("Could not start network requirements check: {err}")),
                }
            }
        });
    }
}

fn run_network_repair() -> Result<()> {
    let script = network_repair_script()?;

    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("starting network requirements check")?;

    Ok(())
}

fn network_repair_script() -> Result<std::path::PathBuf> {
    let current = std::env::current_exe()?;
    let exe_dir = current
        .parent()
        .ok_or_else(|| anyhow::anyhow!("could not determine LocalLink executable folder"))?;

    let packaged = exe_dir.join("scripts").join("windows-network-repair.ps1");
    if packaged.exists() {
        return Ok(packaged);
    }

    let dev = exe_dir
        .join("..")
        .join("..")
        .join("scripts")
        .join("windows-network-repair.ps1");
    if dev.exists() {
        return Ok(dev);
    }

    anyhow::bail!("windows-network-repair.ps1 was not found")
}
"#;

const PROCESS_CONTROL_CODE: &str = r#"

fn force_stop_core_processes() {
    #[cfg(target_os = "windows")]
    {
        for image in [
            "locallink-addon-clipboard.exe",
            "locallink-addon-echo.exe",
            "locallink-core.exe",
        ] {
            let mut command = Command::new("taskkill.exe");
            command
                .args(["/F", "/T", "/IM", image])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(0x08000000); // CREATE_NO_WINDOW

            let _ = command.spawn().and_then(|mut child| child.wait());
        }

        let mut command = Command::new("powershell.exe");
        command
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "Get-Process | Where-Object { $_.ProcessName -like 'locallink-addon-*' } | Stop-Process -Force",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000); // CREATE_NO_WINDOW

        let _ = command.spawn().and_then(|mut child| child.wait());
    }
}
"#;
