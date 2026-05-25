use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src/main.rs");

    let source = fs::read_to_string("src/main.rs")
        .expect("read src/main.rs")
        .replace("\r\n", "\n");
    let mut generated = source;

    generated = must_replace(
        generated,
        "    fn start_core(&mut self) {\n        match start_sibling_core() {\n            Ok(()) => {\n                self.log(\"Starting LocalLink Core...\");\n                std::thread::sleep(Duration::from_millis(250));\n                self.refresh_all();\n            }\n            Err(e) => self.log(format!(\"Could not start core: {e}\")),\n        }\n    }\n\n",
        "    fn start_core(&mut self) {\n        match start_sibling_core() {\n            Ok(()) => {\n                self.log(\"Starting LocalLink Core...\");\n                std::thread::sleep(Duration::from_millis(250));\n                self.refresh_all();\n            }\n            Err(e) => self.log(format!(\"Could not start core: {e}\")),\n        }\n    }\n\n    fn stop_core(&mut self) {\n        for (_, mut child) in self.addon_processes.drain() {\n            let _ = child.kill();\n            let _ = child.wait();\n        }\n\n        self.send_job(ApiJob::Shutdown);\n\n        self.status = None;\n        self.peers.clear();\n        self.connections.clear();\n        self.addons.clear();\n\n        self.log(\"Stopping LocalLink Core...\");\n    }\n\n",
    );

    generated = must_replace(
        generated,
        "                    if !self.core_online()\n                        && ui\n                            .add(primary_button(\"Start\"))\n                            .on_hover_cursor(egui::CursorIcon::PointingHand)\n                            .clicked()\n                    {\n                        self.start_core();\n                    }\n",
        "                    if self.core_online() {\n                        if ui\n                            .add(danger_button(\"Stop Core\"))\n                            .on_hover_cursor(egui::CursorIcon::PointingHand)\n                            .clicked()\n                        {\n                            self.stop_core();\n                        }\n                    } else if ui\n                        .add(primary_button(\"Start\"))\n                        .on_hover_cursor(egui::CursorIcon::PointingHand)\n                        .clicked()\n                    {\n                        self.start_core();\n                    }\n",
    );

    generated = generated.replace("secondary_button(\"Shutdown\")", "danger_button(\"Stop Core\")");

    generated = must_replace(
        generated,
        "                ui.add_space(12.0);\n\n                glass_panel(ui, |ui| {\n                    ui.horizontal_wrapped(|ui| {\n                        ui.heading(\"Advanced\");",
        "                ui.add_space(12.0);\n\n                self.network_requirements_panel(ui);\n\n                ui.add_space(12.0);\n\n                glass_panel(ui, |ui| {\n                    ui.horizontal_wrapped(|ui| {\n                        ui.heading(\"Advanced\");",
    );

    generated.push_str(NETWORK_REQUIREMENTS_CODE);

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
