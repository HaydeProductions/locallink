#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use eframe::egui;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LocalLink Debugger")
            .with_inner_size([980.0, 700.0])
            .with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };

    eframe::run_native(
        "LocalLink Debugger",
        options,
        Box::new(|_cc| Ok(Box::new(DebuggerApp::new()))),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SourceKind {
    Core,
    Ui,
    DevLaunch,
    SpaceProbe,
}

impl SourceKind {
    fn label(&self) -> &'static str {
        match self {
            Self::Core => "CORE",
            Self::Ui => "UI",
            Self::DevLaunch => "DEV",
            Self::SpaceProbe => "PROBE",
        }
    }
}

#[derive(Debug, Clone)]
struct LogFile {
    source: SourceKind,
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct LogLine {
    source: SourceKind,
    file_name: String,
    level: String,
    text: String,
}

struct DebuggerApp {
    log_root: PathBuf,
    last_refresh: Instant,
    paused: bool,
    raw: bool,
    auto_scroll: bool,
    show_core: bool,
    show_ui: bool,
    show_dev: bool,
    show_probe: bool,
    filter: String,
    tail_lines: usize,
    visible_lines: Vec<LogLine>,
    status: String,
}

impl DebuggerApp {
    fn new() -> Self {
        let mut app = Self {
            log_root: log_root(),
            last_refresh: Instant::now() - Duration::from_secs(10),
            paused: false,
            raw: false,
            auto_scroll: true,
            show_core: true,
            show_ui: true,
            show_dev: false,
            show_probe: true,
            filter: String::new(),
            tail_lines: 220,
            visible_lines: Vec::new(),
            status: String::new(),
        };
        app.refresh();
        app
    }

    fn refresh_if_needed(&mut self) {
        if self.paused {
            return;
        }

        if self.last_refresh.elapsed() >= Duration::from_millis(700) {
            self.refresh();
        }
    }

    fn refresh(&mut self) {
        self.last_refresh = Instant::now();
        self.visible_lines.clear();

        let files = self.discover_log_files();
        let file_count = files.len();

        for file in files {
            if !self.source_enabled(&file.source) {
                continue;
            }

            let Ok(text) = fs::read_to_string(&file.path) else {
                continue;
            };

            let file_name = file
                .path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| file.path.display().to_string());

            let mut lines: Vec<_> = text.lines().collect();
            if lines.len() > self.tail_lines {
                lines = lines.split_off(lines.len() - self.tail_lines);
            }

            for line in lines {
                if !self.line_matches(line) {
                    continue;
                }

                self.visible_lines.push(LogLine {
                    source: file.source.clone(),
                    file_name: file_name.clone(),
                    level: classify_line(line),
                    text: line.to_string(),
                });
            }
        }

        if self.visible_lines.len() > self.tail_lines {
            self.visible_lines = self.visible_lines.split_off(self.visible_lines.len() - self.tail_lines);
        }

        self.status = format!(
            "{} visible line(s) from {} log file(s)",
            self.visible_lines.len(),
            file_count
        );
    }

    fn discover_log_files(&self) -> Vec<LogFile> {
        let mut files = Vec::new();
        collect_matching(&mut files, &self.log_root, SourceKind::Core, |name| name == "diagnostics.log");
        collect_matching(&mut files, &self.log_root, SourceKind::Ui, |name| name.starts_with("ui-process-") && name.ends_with(".log"));
        collect_matching(&mut files, &self.log_root, SourceKind::DevLaunch, |name| name.starts_with("dev-launch-") && name.ends_with(".log"));
        collect_matching(&mut files, &self.log_root, SourceKind::SpaceProbe, |name| name.starts_with("space-probe-") && name.ends_with(".log"));

        let local_probe_root = local_probe_log_root();
        if local_probe_root != self.log_root {
            collect_matching(&mut files, &local_probe_root, SourceKind::SpaceProbe, |name| name.starts_with("space-probe-") && name.ends_with(".log"));
        }

        files.sort_by(|a, b| modified_age_ms(&a.path).cmp(&modified_age_ms(&b.path)));
        files.truncate(12);
        files
    }

    fn source_enabled(&self, source: &SourceKind) -> bool {
        match source {
            SourceKind::Core => self.show_core,
            SourceKind::Ui => self.show_ui,
            SourceKind::DevLaunch => self.show_dev,
            SourceKind::SpaceProbe => self.show_probe,
        }
    }

    fn line_matches(&self, line: &str) -> bool {
        let lower = line.to_ascii_lowercase();
        let needle = self.filter.trim().to_ascii_lowercase();

        if !needle.is_empty() && !lower.contains(&needle) {
            return false;
        }

        if self.raw {
            return true;
        }

        is_action_line(&lower)
    }
}

impl eframe::App for DebuggerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(250));
        self.refresh_if_needed();

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("LocalLink Debugger");
                ui.separator();

                if ui.button(if self.paused { "Resume" } else { "Pause" }).clicked() {
                    self.paused = !self.paused;
                }

                if ui.button("Refresh").clicked() {
                    self.refresh();
                }

                if ui.button("Clear view").clicked() {
                    self.visible_lines.clear();
                    self.status = "View cleared. Refresh to reload from files.".to_string();
                }
            });

            ui.add_space(8.0);

            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.raw, "Raw mode");
                ui.checkbox(&mut self.auto_scroll, "Auto-scroll");
                ui.separator();
                ui.checkbox(&mut self.show_core, "Core");
                ui.checkbox(&mut self.show_ui, "UI");
                ui.checkbox(&mut self.show_probe, "Space probe");
                ui.checkbox(&mut self.show_dev, "Build/run");
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("Filter");
                let changed = ui
                    .add(egui::TextEdit::singleline(&mut self.filter).hint_text("clipboard-sync, space id, addon id, error..."))
                    .changed();

                ui.label("Tail");
                let tail_changed = ui
                    .add(egui::DragValue::new(&mut self.tail_lines).range(40..=2000).speed(20))
                    .changed();

                if changed || tail_changed {
                    self.refresh();
                }
            });

            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(&self.status).monospace());
                ui.separator();
                ui.label(egui::RichText::new(self.log_root.display().to_string()).monospace().small());
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::Frame::none()
                .fill(egui::Color32::from_rgb(12, 14, 18))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(55, 65, 80)))
                .inner_margin(egui::Margin::symmetric(10, 10))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(self.auto_scroll && !self.paused)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if self.visible_lines.is_empty() {
                                ui.label(
                                    egui::RichText::new("No matching diagnostic lines yet. Click Enable/Connect or switch on Raw mode.")
                                        .monospace()
                                        .color(egui::Color32::LIGHT_GRAY),
                                );
                            }

                            for line in &self.visible_lines {
                                let color = match line.level.as_str() {
                                    "ERROR" => egui::Color32::from_rgb(255, 115, 115),
                                    "ACTION" => egui::Color32::from_rgb(130, 205, 255),
                                    "LAUNCH" => egui::Color32::from_rgb(150, 240, 180),
                                    "STATE" => egui::Color32::from_rgb(235, 220, 140),
                                    _ => egui::Color32::from_rgb(210, 215, 225),
                                };

                                ui.horizontal_wrapped(|ui| {
                                    ui.label(egui::RichText::new(line.source.label()).monospace().color(egui::Color32::from_rgb(150, 160, 175)));
                                    ui.label(egui::RichText::new(&line.level).monospace().color(color));
                                    ui.label(egui::RichText::new(&line.file_name).monospace().color(egui::Color32::from_rgb(120, 130, 145)).small());
                                    ui.label(egui::RichText::new(&line.text).monospace().color(color));
                                });
                            }
                        });
                });
        });
    }
}

fn is_action_line(lower: &str) -> bool {
    lower.contains("action apijob")
        || lower.contains("set_space_addon_enabled")
        || lower.contains("set_addon_enabled")
        || lower.contains("set_space_active")
        || lower.contains("activate_space")
        || lower.contains("deactivate_space")
        || lower.contains("ok=false")
        || lower.contains("error=")
        || lower.contains("failed")
        || lower.contains("could not")
        || lower.contains("panic")
        || lower.contains("addon-manager")
        || lower.contains("addon-launch")
        || lower.contains("starting add-on")
        || lower.contains("started add-on")
        || lower.contains("exited")
        || lower.contains("suppressed")
        || lower.contains("space-probe")
        || lower.contains("probe_ping")
        || lower.contains("probe_pong")
        || lower.contains("send_space_message")
        || lower.contains("space_service_data")
}

fn classify_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();

    if lower.contains("ok=false") || lower.contains("error=") || lower.contains("failed") || lower.contains("could not") || lower.contains("panic") {
        "ERROR".to_string()
    } else if lower.contains("action apijob") || lower.contains("set_space_addon_enabled") {
        "ACTION".to_string()
    } else if lower.contains("set_addon_enabled") || lower.contains("set_space_active") {
        "STATE".to_string()
    } else if lower.contains("addon-launch") || lower.contains("starting add-on") || lower.contains("started add-on") {
        "LAUNCH".to_string()
    } else if lower.contains("addon-manager") {
        "PLAN".to_string()
    } else if lower.contains("space-probe") || lower.contains("probe_ping") || lower.contains("probe_pong") {
        "PROBE".to_string()
    } else {
        "DEBUG".to_string()
    }
}

fn collect_matching(files: &mut Vec<LogFile>, dir: &Path, source: SourceKind, matches: impl Fn(&str) -> bool) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().map(|name| name.to_string_lossy().to_string()) else {
            continue;
        };

        if matches(&name) {
            files.push(LogFile { source: source.clone(), path });
        }
    }
}

fn log_root() -> PathBuf {
    std::env::var("APPDATA")
        .or_else(|_| std::env::var("LOCALAPPDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("LocalLink")
        .join("logs")
}

fn local_probe_log_root() -> PathBuf {
    std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| log_root())
        .join("LocalLink")
        .join("logs")
}

fn modified_age_ms(path: &Path) -> u128 {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(u128::MAX)
}
