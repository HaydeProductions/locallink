use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src/main.rs");
    println!("cargo:rerun-if-changed=../assets/locallink-tray.ico.b64");

    embed_windows_app_icon();

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

    generated = generated.replace("use std::sync::mpsc;\n", "use std::sync::{mpsc, Arc};\n");

    generated = must_replace(
        generated,
        "            .with_title(\"LocalLink\")\n            .with_inner_size([470.0, 640.0])\n            .with_min_inner_size([390.0, 520.0]),",
        "            .with_title(\"LocalLink\")\n            .with_inner_size([470.0, 640.0])\n            .with_min_inner_size([390.0, 520.0])\n            .with_icon(local_link_window_icon()),",
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

    generated = generated.replace(
        "secondary_button(\"Shutdown\")",
        "danger_button(\"Stop Core\")",
    );

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
    generated.push_str(WINDOW_ICON_CODE);

    fs::write(Path::new("src/core_control_main.rs"), generated)
        .expect("write generated UI entry point");
}

#[cfg(windows)]
fn embed_windows_app_icon() {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let icon_path = Path::new(&out_dir).join("locallink-ui.ico");
    let icon_b64 = include_str!("../assets/locallink-tray.ico.b64").trim();
    let icon_bytes = STANDARD.decode(icon_b64).expect("decode LocalLink icon");
    fs::write(&icon_path, icon_bytes).expect("write LocalLink UI icon");

    winresource::WindowsResource::new()
        .set_icon(icon_path.to_str().expect("icon path is utf-8"))
        .compile()
        .expect("compile Windows icon resource");
}

#[cfg(not(windows))]
fn embed_windows_app_icon() {}

fn must_replace(input: String, from: &str, to: &str) -> String {
    let output = input.replace(from, to);

    if output == input {
        panic!(
            "expected UI source pattern was not found while generating core-control entry point"
        );
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

const WINDOW_ICON_CODE: &str = r#"

fn local_link_window_icon() -> Arc<egui::IconData> {
    let size = 64usize;
    let mut rgba = vec![0u8; size * size * 4];

    for y in 0..size {
        for x in 0..size {
            let dx = x.min(size - 1 - x) as f32;
            let dy = y.min(size - 1 - y) as f32;
            let radius = 13.0;
            let corner = if dx < radius && dy < radius {
                let cx = radius - dx;
                let cy = radius - dy;
                (cx * cx + cy * cy).sqrt() <= radius
            } else {
                true
            };

            if corner {
                let i = (y * size + x) * 4;
                let t = y as f32 / (size - 1) as f32;
                rgba[i] = (12.0 + 8.0 * t) as u8;
                rgba[i + 1] = (22.0 + 12.0 * t) as u8;
                rgba[i + 2] = (44.0 + 26.0 * t) as u8;
                rgba[i + 3] = 255;
            }
        }
    }

    draw_line(&mut rgba, size, 19.0, 33.0, 32.0, 20.0, [87, 232, 255, 255], 5.0);
    draw_line(&mut rgba, size, 32.0, 20.0, 45.0, 33.0, [87, 232, 255, 255], 5.0);
    draw_line(&mut rgba, size, 19.0, 33.0, 32.0, 44.0, [98, 255, 173, 255], 5.0);
    draw_line(&mut rgba, size, 32.0, 44.0, 45.0, 33.0, [98, 255, 173, 255], 5.0);

    draw_circle(&mut rgba, size, 19.0, 33.0, 7.0, [87, 232, 255, 255]);
    draw_circle(&mut rgba, size, 45.0, 33.0, 7.0, [98, 255, 173, 255]);
    draw_circle(&mut rgba, size, 32.0, 20.0, 5.5, [230, 255, 255, 255]);
    draw_circle(&mut rgba, size, 32.0, 44.0, 5.5, [230, 255, 255, 255]);
    draw_circle(&mut rgba, size, 19.0, 33.0, 3.0, [230, 255, 255, 255]);
    draw_circle(&mut rgba, size, 45.0, 33.0, 3.0, [230, 255, 255, 255]);
    draw_circle(&mut rgba, size, 32.0, 20.0, 2.2, [87, 232, 255, 255]);
    draw_circle(&mut rgba, size, 32.0, 44.0, 2.2, [98, 255, 173, 255]);

    Arc::new(egui::IconData {
        rgba,
        width: size as u32,
        height: size as u32,
    })
}

fn draw_circle(rgba: &mut [u8], size: usize, cx: f32, cy: f32, r: f32, color: [u8; 4]) {
    let min_x = (cx - r - 1.0).floor().max(0.0) as usize;
    let max_x = (cx + r + 1.0).ceil().min((size - 1) as f32) as usize;
    let min_y = (cy - r - 1.0).floor().max(0.0) as usize;
    let max_y = (cy + r + 1.0).ceil().min((size - 1) as f32) as usize;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            if dx * dx + dy * dy <= r * r {
                blend_pixel(rgba, size, x, y, color);
            }
        }
    }
}

fn draw_line(rgba: &mut [u8], size: usize, x0: f32, y0: f32, x1: f32, y1: f32, color: [u8; 4], width: f32) {
    let min_x = (x0.min(x1) - width).floor().max(0.0) as usize;
    let max_x = (x0.max(x1) + width).ceil().min((size - 1) as f32) as usize;
    let min_y = (y0.min(y1) - width).floor().max(0.0) as usize;
    let max_y = (y0.max(y1) + width).ceil().min((size - 1) as f32) as usize;
    let vx = x1 - x0;
    let vy = y1 - y0;
    let len2 = vx * vx + vy * vy;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let px = x as f32;
            let py = y as f32;
            let t = (((px - x0) * vx + (py - y0) * vy) / len2).clamp(0.0, 1.0);
            let cx = x0 + t * vx;
            let cy = y0 + t * vy;
            let dx = px - cx;
            let dy = py - cy;
            if dx * dx + dy * dy <= width * width {
                blend_pixel(rgba, size, x, y, color);
            }
        }
    }
}

fn blend_pixel(rgba: &mut [u8], size: usize, x: usize, y: usize, color: [u8; 4]) {
    let i = (y * size + x) * 4;
    let a = color[3] as f32 / 255.0;
    let inv = 1.0 - a;
    rgba[i] = (color[0] as f32 * a + rgba[i] as f32 * inv) as u8;
    rgba[i + 1] = (color[1] as f32 * a + rgba[i + 1] as f32 * inv) as u8;
    rgba[i + 2] = (color[2] as f32 * a + rgba[i + 2] as f32 * inv) as u8;
    rgba[i + 3] = 255;
}
"#;
