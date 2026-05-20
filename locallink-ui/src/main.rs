use anyhow::{bail, Context, Result};
use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const LOCAL_API_ADDR: &str = "127.0.0.1:47900";

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LocalLink")
            .with_inner_size([470.0, 640.0])
            .with_min_inner_size([390.0, 520.0]),
        ..Default::default()
    };

    eframe::run_native(
        "LocalLink",
        options,
        Box::new(|_cc| Ok(Box::new(LocalLinkUi::new()))),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Discover,
    Devices,
    Addons,
    Activity,
}

#[derive(Debug, Clone)]
enum ApiJob {
    Status,
    Paths,
    Peers,
    Trusted,
    Connections,
    Addons,
    PollEvents {
        service: Option<String>,
    },
    AddTrusted {
        name: String,
        mac: String,
    },
    RemoveTrusted {
        mac: String,
    },
    Connect {
        mac: Option<String>,
        peer_id: Option<String>,
    },
    Disconnect {
        mac: Option<String>,
        peer_id: Option<String>,
    },
    ReloadAddons,
    Shutdown,
}

#[derive(Debug)]
enum UiMsg {
    ApiOk { job: String, value: Value },
    ApiErr { job: String, error: String },
}

#[derive(Debug, Clone, Default)]
struct CoreStatus {
    version: String,
    device_id: String,
    device_name: String,
    psk_configured: bool,
    api_addr: String,
    uptime_ms: u128,
}

#[derive(Debug, Clone, Default)]
struct Paths {
    app_dir: String,
    config_file: String,
    trusted_devices_file: String,
    addons_dir: String,
    logs_dir: String,
    runtime_dir: String,
    state_dir: String,
    lock_file: String,
}

#[derive(Debug, Clone, Default)]
struct PeerRow {
    device_id: String,
    device_name: String,
    addr: String,
    macs: Vec<String>,
    trusted: bool,
    trusted_name: Option<String>,
    connected: bool,
    last_seen_ms_ago: u128,
}

#[derive(Debug, Clone, Default)]
struct TrustedRow {
    name: String,
    macs: Vec<String>,
    device_id: Option<String>,
    blocked: bool,
}

#[derive(Debug, Clone, Default)]
struct ConnectionRow {
    device_id: String,
    device_name: String,
    addr: String,
    connected_ms_ago: u128,
    last_seen_ms_ago: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AddonManifest {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    executable: String,
    #[serde(default)]
    services: Vec<String>,
    #[serde(default)]
    enabled: bool,
}

#[derive(Debug, Clone, Default)]
struct AddonRow {
    id: String,
    name: String,
    version: String,
    description: String,
    executable: String,
    services: Vec<String>,
    enabled: bool,
    manifest_path: String,
    addon_dir: String,
}

#[derive(Debug, Clone, Default)]
struct EventRow {
    kind: String,
    peer_id: String,
    peer_name: String,
    service: String,
    channel_id: Option<String>,
    message_id: Option<String>,
    data_b64: Option<String>,
    reason: Option<String>,
    received_ms: u128,
}

struct LocalLinkUi {
    screen: Screen,

    tx: mpsc::Sender<ApiJob>,
    rx: mpsc::Receiver<UiMsg>,

    status: Option<CoreStatus>,
    paths: Option<Paths>,
    peers: Vec<PeerRow>,
    trusted: Vec<TrustedRow>,
    connections: Vec<ConnectionRow>,
    addons: Vec<AddonRow>,
    events: Vec<EventRow>,

    addon_processes: HashMap<String, Child>,

    log: Vec<String>,
    loading_count: usize,
    last_refresh: Option<Instant>,

    show_settings: bool,
    show_advanced: bool,

    add_name: String,
    add_mac: String,
    event_filter: String,
}

impl LocalLinkUi {
    fn new() -> Self {
        let (job_tx, job_rx) = mpsc::channel::<ApiJob>();
        let (msg_tx, msg_rx) = mpsc::channel::<UiMsg>();

        std::thread::spawn(move || api_worker(job_rx, msg_tx));

        let mut app = Self {
            screen: Screen::Discover,

            tx: job_tx,
            rx: msg_rx,

            status: None,
            paths: None,
            peers: Vec::new(),
            trusted: Vec::new(),
            connections: Vec::new(),
            addons: Vec::new(),
            events: Vec::new(),

            addon_processes: HashMap::new(),

            log: Vec::new(),
            loading_count: 0,
            last_refresh: None,

            show_settings: false,
            show_advanced: false,

            add_name: String::new(),
            add_mac: String::new(),
            event_filter: String::new(),
        };

        app.refresh_all();
        app
    }

    fn core_online(&self) -> bool {
        self.status.is_some()
    }

    fn refresh_all(&mut self) {
        self.send_job(ApiJob::Status);
        self.send_job(ApiJob::Paths);
        self.send_job(ApiJob::Peers);
        self.send_job(ApiJob::Trusted);
        self.send_job(ApiJob::Connections);
        self.send_job(ApiJob::Addons);
        self.last_refresh = Some(Instant::now());
    }

    fn refresh_visible(&mut self) {
        self.send_job(ApiJob::Status);

        match self.screen {
            Screen::Discover => {
                self.send_job(ApiJob::Peers);
                self.send_job(ApiJob::Trusted);
            }
            Screen::Devices => {
                self.send_job(ApiJob::Trusted);
                self.send_job(ApiJob::Peers);
                self.send_job(ApiJob::Connections);
            }
            Screen::Addons => {
                self.send_job(ApiJob::Addons);
            }
            Screen::Activity => {
                let service = if self.event_filter.trim().is_empty() {
                    None
                } else {
                    Some(self.event_filter.trim().to_string())
                };
                self.send_job(ApiJob::PollEvents { service });
            }
        }

        self.last_refresh = Some(Instant::now());
    }

    fn send_job(&mut self, job: ApiJob) {
        self.loading_count += 1;

        if let Err(err) = self.tx.send(job) {
            self.loading_count = self.loading_count.saturating_sub(1);
            self.log(format!("UI worker unavailable: {err}"));
        }
    }

    fn log(&mut self, msg: impl Into<String>) {
        self.log.push(msg.into());

        if self.log.len() > 120 {
            self.log.remove(0);
        }
    }

    fn pump_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            self.loading_count = self.loading_count.saturating_sub(1);

            match msg {
                UiMsg::ApiOk { job, value } => self.handle_api_ok(&job, value),
                UiMsg::ApiErr { job, error } => {
                    if job == "status" {
                        self.status = None;
                    }
                    self.log(format!("{job}: {error}"));
                }
            }
        }
    }

    fn handle_api_ok(&mut self, job: &str, value: Value) {
        match job {
            "status" => self.apply_status(value),
            "paths" => self.apply_paths(value),
            "peers" => self.apply_peers(value),
            "trusted" => self.apply_trusted(value),
            "connections" => self.apply_connections(value),
            "addons" => self.apply_addons(value),
            "poll_events" => self.apply_events(value),
            "add_trusted" => {
                self.log("Device added.");
                self.add_name.clear();
                self.add_mac.clear();
                self.send_job(ApiJob::Peers);
                self.send_job(ApiJob::Trusted);
            }
            "remove_trusted" => {
                self.log("Device removed.");
                self.send_job(ApiJob::Peers);
                self.send_job(ApiJob::Trusted);
                self.send_job(ApiJob::Connections);
            }
            "connect" => {
                self.log("Connection requested.");
                self.send_job(ApiJob::Peers);
                self.send_job(ApiJob::Trusted);
                self.send_job(ApiJob::Connections);
            }
            "disconnect" => {
                self.log("Disconnected.");
                self.send_job(ApiJob::Peers);
                self.send_job(ApiJob::Trusted);
                self.send_job(ApiJob::Connections);
            }
            "reload_addons" => {
                self.log("Add-ons reloaded.");
                self.send_job(ApiJob::Addons);
            }
            "shutdown" => {
                self.status = None;
                self.log("Core shutdown requested.");
            }
            _ => {}
        }
    }

    fn apply_status(&mut self, v: Value) {
        if let Some(data) = v.get("data") {
            self.status = Some(CoreStatus {
                version: str_field(data, "version"),
                device_id: str_field(data, "device_id"),
                device_name: str_field(data, "device_name"),
                psk_configured: bool_field(data, "psk_configured"),
                api_addr: str_field(data, "api_addr"),
                uptime_ms: u128_field(data, "uptime_ms"),
            });
        }
    }

    fn apply_paths(&mut self, v: Value) {
        if let Some(data) = v.get("data") {
            self.paths = Some(Paths {
                app_dir: str_field(data, "app_dir"),
                config_file: str_field(data, "config_file"),
                trusted_devices_file: str_field(data, "trusted_devices_file"),
                addons_dir: str_field(data, "addons_dir"),
                logs_dir: str_field(data, "logs_dir"),
                runtime_dir: str_field(data, "runtime_dir"),
                state_dir: str_field(data, "state_dir"),
                lock_file: str_field(data, "lock_file"),
            });
        }
    }

    fn apply_peers(&mut self, v: Value) {
        self.peers.clear();

        if let Some(rows) = v.get("data").and_then(|x| x.as_array()) {
            for row in rows {
                self.peers.push(PeerRow {
                    device_id: str_field(row, "device_id"),
                    device_name: str_field(row, "device_name"),
                    addr: str_field(row, "addr"),
                    macs: string_array_field(row, "macs"),
                    trusted: bool_field(row, "trusted"),
                    trusted_name: optional_str_field(row, "trusted_name"),
                    connected: bool_field(row, "connected"),
                    last_seen_ms_ago: u128_field(row, "last_seen_ms_ago"),
                });
            }
        }
    }

    fn apply_trusted(&mut self, v: Value) {
        self.trusted.clear();

        if let Some(rows) = v.get("data").and_then(|x| x.as_array()) {
            for row in rows {
                self.trusted.push(TrustedRow {
                    name: str_field(row, "name"),
                    macs: string_array_field(row, "macs"),
                    device_id: optional_str_field(row, "device_id"),
                    blocked: bool_field(row, "blocked"),
                });
            }
        }
    }

    fn apply_connections(&mut self, v: Value) {
        self.connections.clear();

        if let Some(rows) = v.get("data").and_then(|x| x.as_array()) {
            for row in rows {
                self.connections.push(ConnectionRow {
                    device_id: str_field(row, "device_id"),
                    device_name: str_field(row, "device_name"),
                    addr: str_field(row, "addr"),
                    connected_ms_ago: u128_field(row, "connected_ms_ago"),
                    last_seen_ms_ago: u128_field(row, "last_seen_ms_ago"),
                });
            }
        }
    }

    fn apply_addons(&mut self, v: Value) {
        self.addons.clear();

        if let Some(rows) = v.get("data").and_then(|x| x.as_array()) {
            for row in rows {
                self.addons.push(AddonRow {
                    id: str_field(row, "id"),
                    name: str_field(row, "name"),
                    version: str_field(row, "version"),
                    description: str_field(row, "description"),
                    executable: str_field(row, "executable"),
                    services: string_array_field(row, "services"),
                    enabled: bool_field(row, "enabled"),
                    manifest_path: str_field(row, "manifest_path"),
                    addon_dir: str_field(row, "addon_dir"),
                });
            }
        }
    }

    fn apply_events(&mut self, v: Value) {
        let mut count = 0;

        if let Some(rows) = v.get("data").and_then(|x| x.as_array()) {
            for row in rows {
                count += 1;

                self.events.push(EventRow {
                    kind: str_field(row, "kind"),
                    peer_id: str_field(row, "peer_id"),
                    peer_name: str_field(row, "peer_name"),
                    service: str_field(row, "service"),
                    channel_id: optional_str_field(row, "channel_id"),
                    message_id: optional_str_field(row, "message_id"),
                    data_b64: optional_str_field(row, "data_b64"),
                    reason: optional_str_field(row, "reason"),
                    received_ms: u128_field(row, "received_ms"),
                });
            }
        }

        if count > 0 {
            self.log(format!("Loaded {count} activity item(s)."));
        }
    }

    fn start_core(&mut self) {
        match start_sibling_core() {
            Ok(()) => {
                self.log("Starting LocalLink Core...");
                std::thread::sleep(Duration::from_millis(250));
                self.refresh_all();
            }
            Err(e) => self.log(format!("Could not start core: {e}")),
        }
    }

    fn toggle_addon(&mut self, addon_id: String, enabled: bool) {
        let addon_snapshot = {
            let Some(addon) = self.addons.iter_mut().find(|a| a.id == addon_id) else {
                self.log(format!("Add-on not found: {addon_id}"));
                return;
            };

            addon.enabled = enabled;
            addon.clone()
        };

        if let Err(e) = set_manifest_enabled(&addon_snapshot.manifest_path, enabled) {
            self.log(format!("Could not update {}: {e}", addon_snapshot.name));
            return;
        }

        if enabled {
            match launch_addon(&addon_snapshot) {
                Ok(child) => {
                    self.addon_processes
                        .insert(addon_snapshot.id.clone(), child);
                    self.log(format!("Enabled {}", addon_snapshot.name));
                }
                Err(e) => self.log(format!(
                    "{} was enabled but could not be launched: {e}",
                    addon_snapshot.name
                )),
            }
        } else if let Some(mut child) = self.addon_processes.remove(&addon_snapshot.id) {
            let _ = child.kill();
            self.log(format!("Disabled {}", addon_snapshot.name));
        } else {
            self.log(format!("Disabled {}", addon_snapshot.name));
        }

        self.send_job(ApiJob::ReloadAddons);
    }
}

impl eframe::App for LocalLinkUi {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_style(ctx);
        self.pump_messages();

        if self.loading_count == 0 {
            let should_refresh = self
                .last_refresh
                .map(|t| t.elapsed() > Duration::from_secs(3))
                .unwrap_or(true);

            if should_refresh {
                self.refresh_visible();
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            paint_background(ui);

            egui::Frame::none()
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    self.header(ui);
                    ui.add_space(14.0);
                    self.tabs(ui);
                    ui.add_space(18.0);

                    match self.screen {
                        Screen::Discover => self.screen_discover(ui),
                        Screen::Devices => self.screen_devices(ui),
                        Screen::Addons => self.screen_addons(ui),
                        Screen::Activity => self.screen_activity(ui),
                    }
                });
        });

        self.settings_window(ctx);
    }
}

impl LocalLinkUi {
    fn apply_style(&self, ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.window_fill = color_bg();
        visuals.panel_fill = color_bg();
        visuals.extreme_bg_color = color_bg_dark();
        visuals.faint_bg_color = color_card();
        visuals.widgets.inactive.bg_fill = color_card();
        visuals.widgets.hovered.bg_fill = color_card_hover();
        visuals.widgets.active.bg_fill = color_accent_dark();
        visuals.selection.bg_fill = color_accent_dark();
        ctx.set_visuals(visuals);

        ctx.style_mut(|style| {
            style.spacing.item_spacing = egui::vec2(10.0, 10.0);
            style.spacing.button_padding = egui::vec2(16.0, 9.0);
            style.spacing.window_margin = egui::Margin::symmetric(18, 18);
        });
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        glass_panel(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.vertical(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        status_dot(
                            ui,
                            if self.core_online() {
                                color_success()
                            } else {
                                color_error()
                            },
                        );

                        ui.heading(
                            egui::RichText::new("LocalLink")
                                .size(26.0)
                                .color(color_text()),
                        );
                    });

                    ui.label(
                        egui::RichText::new("Secure local device bridge")
                            .color(color_muted())
                            .size(13.5),
                    );
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(icon_button("⚙")).clicked() {
                        self.show_settings = true;
                    }
                    if !self.core_online() && ui.add(primary_button("Start")).clicked() {
                        self.start_core();
                    }
                });
            });
        });
    }

    fn tabs(&mut self, ui: &mut egui::Ui) {
        // Drawn manually instead of using egui's normal button layout.
        // This guarantees the control is centered horizontally and does not drift vertically.
        let available_width = ui.available_width();

        let row_height: f32 = 54.0;
        let container_width: f32 = available_width.min(390.0).max(312.0);
        let container_height: f32 = 50.0;

        let (row_rect, _) = ui.allocate_exact_size(
            egui::vec2(available_width, row_height),
            egui::Sense::hover(),
        );

        let container_rect = egui::Rect::from_center_size(
            row_rect.center(),
            egui::vec2(container_width, container_height),
        );

        let painter = ui.painter();

        painter.rect(
            container_rect,
            25.0,
            color_panel(),
            egui::Stroke::new(1.0, color_border().linear_multiply(0.55)),
            egui::StrokeKind::Inside,
        );

        let inner = container_rect.shrink2(egui::vec2(7.0, 7.0));

        let gap: f32 = 5.0;
        let tab_count: f32 = 4.0;
        let tab_width = (inner.width() - gap * (tab_count - 1.0)) / tab_count;
        let tab_height = inner.height();

        let tabs = [
            (Screen::Discover, "Discover"),
            (Screen::Devices, "Devices"),
            (Screen::Addons, "Add-ons"),
            (Screen::Activity, "Activity"),
        ];

        for (i, (target, label)) in tabs.iter().enumerate() {
            let x = inner.left() + i as f32 * (tab_width + gap);

            let tab_rect = egui::Rect::from_min_size(
                egui::pos2(x, inner.top()),
                egui::vec2(tab_width, tab_height),
            );

            let id = ui.id().with(format!("tab-{}", label));
            let response = ui.interact(tab_rect, id, egui::Sense::click());

            if response.clicked() {
                self.screen = *target;
            }

            let selected = self.screen == *target;
            let hovered = response.hovered();

            let fill = if selected {
                color_accent_dark()
            } else if hovered {
                color_card_hover().linear_multiply(0.45)
            } else {
                egui::Color32::TRANSPARENT
            };

            let stroke = if selected {
                egui::Stroke::new(1.15, color_accent())
            } else if hovered {
                egui::Stroke::new(1.0, color_border().linear_multiply(0.85))
            } else {
                egui::Stroke::new(1.0, color_border().linear_multiply(0.38))
            };

            painter.rect(tab_rect, 18.0, fill, stroke, egui::StrokeKind::Inside);

            if selected {
                painter.rect_stroke(
                    tab_rect.shrink(2.0),
                    17.0,
                    egui::Stroke::new(1.0, color_accent().linear_multiply(0.30)),
                    egui::StrokeKind::Inside,
                );

                let underline = egui::Rect::from_center_size(
                    egui::pos2(tab_rect.center().x, tab_rect.bottom() - 4.5),
                    egui::vec2((tab_width * 0.36).max(20.0), 2.0),
                );

                painter.rect_filled(underline, 1.0, color_accent().linear_multiply(0.85));
            }

            let text_color = if selected {
                color_text()
            } else if hovered {
                color_text().linear_multiply(0.88)
            } else {
                color_muted()
            };

            painter.text(
                tab_rect.center(),
                egui::Align2::CENTER_CENTER,
                *label,
                egui::FontId::proportional(13.5),
                text_color,
            );
        }
    }

    fn screen_discover(&mut self, ui: &mut egui::Ui) {
        page_title(
            ui,
            "Discover",
            "Find nearby devices, then add them by MAC address.",
        );

        ui.add_space(14.0);

        if !self.core_online() {
            notice(
                ui,
                "Core is offline",
                "Start LocalLink Core to discover nearby devices.",
                color_error(),
            );
            return;
        }

        if self.peers.is_empty() {
            notice(
                ui,
                "Scanning local link",
                "No nearby devices are visible yet.",
                color_warning(),
            );
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            for peer in self.peers.clone() {
                let primary_mac = peer.macs.first().cloned().unwrap_or_default();

                device_card(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.vertical(|ui| {
                            ui.heading(egui::RichText::new(&peer.device_name).color(color_text()));
                            ui.label(
                                egui::RichText::new(if peer.trusted {
                                    "Known device"
                                } else {
                                    "Nearby unregistered device"
                                })
                                .color(color_muted()),
                            );
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if peer.connected {
                                state_chip(ui, "Connected", color_success());
                            } else if peer.trusted {
                                state_chip(ui, "Trusted", color_accent());
                            } else {
                                state_chip(ui, "New", color_warning());
                            }
                        });
                    });

                    ui.add_space(10.0);

                    if !primary_mac.is_empty() {
                        mono_line(ui, "MAC", &primary_mac);
                    }

                    ui.add_space(12.0);

                    ui.horizontal_wrapped(|ui| {
                        if !peer.trusted {
                            if ui.add(primary_button("Add Device")).clicked() {
                                self.send_job(ApiJob::AddTrusted {
                                    name: peer.device_name.clone(),
                                    mac: primary_mac.clone(),
                                });
                            }
                        } else if !peer.connected {
                            if ui.add(primary_button("Connect")).clicked() {
                                self.send_job(ApiJob::Connect {
                                    mac: Some(primary_mac.clone()),
                                    peer_id: Some(peer.device_id.clone()),
                                });
                            }
                        }

                        if self.show_advanced {
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new("Advanced enabled").color(color_muted()));
                        }
                    });

                    if self.show_advanced {
                        ui.separator();
                        mono_line(ui, "Device ID", &peer.device_id);
                        mono_line(ui, "Address", &peer.addr);
                        mono_line(ui, "All MACs", &peer.macs.join(", "));
                        if let Some(name) = &peer.trusted_name {
                            mono_line(ui, "Trusted as", name);
                        }
                    }
                });

                ui.add_space(12.0);
            }
        });

        ui.add_space(14.0);

        device_card(ui, |ui| {
            ui.heading(egui::RichText::new("Add manually").color(color_text()));
            ui.label(
                egui::RichText::new("Use this if the device is not currently visible.")
                    .color(color_muted()),
            );

            ui.add_space(8.0);

            ui.horizontal_wrapped(|ui| {
                ui.label("Name");
                ui.add(
                    egui::TextEdit::singleline(&mut self.add_name)
                        .desired_width(170.0)
                        .hint_text("Gaming PC"),
                );
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("MAC");
                ui.add(
                    egui::TextEdit::singleline(&mut self.add_mac)
                        .desired_width(170.0)
                        .hint_text("aa:bb:cc:dd:ee:ff"),
                );
            });

            ui.add_space(8.0);

            if ui.add(primary_button("Add Trusted Device")).clicked() {
                let name = if self.add_name.trim().is_empty() {
                    self.add_mac.trim().to_string()
                } else {
                    self.add_name.trim().to_string()
                };

                self.send_job(ApiJob::AddTrusted {
                    name,
                    mac: self.add_mac.trim().to_string(),
                });
            }
        });
    }

    fn screen_devices(&mut self, ui: &mut egui::Ui) {
        page_title(
            ui,
            "Devices",
            "Trusted devices stay here. Connect only when you want a secure link.",
        );

        ui.add_space(14.0);

        if self.trusted.is_empty() {
            notice(
                ui,
                "No trusted devices",
                "Use Discover to add a nearby device first.",
                color_warning(),
            );
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            for trusted in self.trusted.clone() {
                let connected = trusted
                    .device_id
                    .as_ref()
                    .and_then(|id| self.connections.iter().find(|c| &c.device_id == id))
                    .cloned();

                let nearby_peer = self
                    .peers
                    .iter()
                    .find(|peer| {
                        peer.macs.iter().any(|m| {
                            trusted
                                .macs
                                .iter()
                                .any(|tm| normalize_mac_ui(m) == normalize_mac_ui(tm))
                        })
                    })
                    .cloned();

                device_card(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.vertical(|ui| {
                            ui.heading(egui::RichText::new(&trusted.name).color(color_text()));
                            if let Some(mac) = trusted.macs.first() {
                                ui.label(egui::RichText::new(mac).color(color_muted()));
                            }
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if connected.is_some() {
                                state_chip(ui, "Connected", color_success());
                            } else if nearby_peer.is_some() {
                                state_chip(ui, "Available", color_accent());
                            } else {
                                state_chip(ui, "Offline", color_muted());
                            }
                        });
                    });

                    ui.add_space(12.0);

                    ui.horizontal_wrapped(|ui| {
                        if let Some(conn) = connected.clone() {
                            if ui.add(secondary_button("Disconnect")).clicked() {
                                self.send_job(ApiJob::Disconnect {
                                    mac: trusted.macs.first().cloned(),
                                    peer_id: Some(conn.device_id.clone()),
                                });
                            }
                        } else if let Some(peer) = nearby_peer.clone() {
                            if ui.add(primary_button("Connect")).clicked() {
                                self.send_job(ApiJob::Connect {
                                    mac: trusted.macs.first().cloned(),
                                    peer_id: Some(peer.device_id.clone()),
                                });
                            }
                        } else {
                            ui.add_enabled(false, primary_button("Connect"));
                        }

                        if ui.add(danger_button("Remove")).clicked() {
                            if let Some(mac) = trusted.macs.first() {
                                self.send_job(ApiJob::RemoveTrusted { mac: mac.clone() });
                            }
                        }
                    });

                    if let Some(conn) = connected {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "Secure session active for {}",
                                format_duration_ms(conn.connected_ms_ago)
                            ))
                            .color(color_muted()),
                        );
                    }

                    if trusted.blocked {
                        ui.label(egui::RichText::new("Blocked").color(color_error()));
                    }

                    if self.show_advanced {
                        ui.separator();
                        if let Some(device_id) = &trusted.device_id {
                            mono_line(ui, "Registered ID", device_id);
                        }
                        if let Some(peer) = nearby_peer {
                            mono_line(ui, "Nearby ID", &peer.device_id);
                            mono_line(ui, "Address", &peer.addr);
                        }
                    }
                });

                ui.add_space(12.0);
            }
        });
    }

    fn screen_addons(&mut self, ui: &mut egui::Ui) {
        page_title(
            ui,
            "Add-ons",
            "Switch features on when you want them active.",
        );

        ui.add_space(14.0);

        if self.addons.is_empty() {
            notice(
                ui,
                "No add-ons installed",
                "Installed add-ons will appear here.",
                color_warning(),
            );
            return;
        }

        let mut toggle_action: Option<(String, bool)> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            for addon in &mut self.addons {
                device_card(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.vertical(|ui| {
                            ui.heading(
                                egui::RichText::new(format!("{} {}", addon.name, addon.version))
                                    .color(color_text()),
                            );
                            ui.label(egui::RichText::new(&addon.description).color(color_muted()));
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let mut enabled = addon.enabled;

                            if toggle_switch(ui, &mut enabled).changed() {
                                toggle_action = Some((addon.id.clone(), enabled));
                            }

                            if self.addon_processes.contains_key(&addon.id) {
                                state_chip(ui, "Running", color_success());
                            } else if addon.enabled {
                                state_chip(ui, "Enabled", color_warning());
                            } else {
                                state_chip(ui, "Off", color_muted());
                            }
                        });
                    });

                    if self.show_advanced {
                        ui.separator();
                        mono_line(ui, "ID", &addon.id);
                        mono_line(ui, "Executable", &addon.executable);
                        mono_line(ui, "Services", &addon.services.join(", "));
                        mono_line(ui, "Folder", &addon.addon_dir);
                        mono_line(ui, "Manifest", &addon.manifest_path);
                    }
                });

                ui.add_space(12.0);
            }
        });

        if let Some((id, enabled)) = toggle_action {
            self.toggle_addon(id, enabled);
        }
    }

    fn screen_activity(&mut self, ui: &mut egui::Ui) {
        page_title(ui, "Activity", "Recent add-on and device activity.");

        ui.add_space(14.0);

        glass_panel(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Filter");
                ui.add(
                    egui::TextEdit::singleline(&mut self.event_filter)
                        .desired_width(170.0)
                        .hint_text("service"),
                );

                if ui.add(primary_button("Check")).clicked() {
                    let service = if self.event_filter.trim().is_empty() {
                        None
                    } else {
                        Some(self.event_filter.trim().to_string())
                    };

                    self.send_job(ApiJob::PollEvents { service });
                }

                if ui.add(secondary_button("Clear")).clicked() {
                    self.events.clear();
                }
            });
        });

        ui.add_space(14.0);

        if self.events.is_empty() {
            notice(
                ui,
                "No activity yet",
                "Messages and add-on events will appear here.",
                color_warning(),
            );
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            for event in self.events.iter().rev() {
                device_card(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.heading(human_event_title(event));
                        state_chip(ui, &event.service, color_accent());
                    });

                    ui.label(
                        egui::RichText::new(format!("From {}", event.peer_name))
                            .color(color_muted()),
                    );

                    if let Some(data_b64) = &event.data_b64 {
                        if let Ok(bytes) = base64_decode_simple(data_b64) {
                            let text = String::from_utf8_lossy(&bytes);
                            ui.label(format!("Message: {text}"));
                        }
                    }

                    if self.show_advanced {
                        ui.separator();
                        mono_line(ui, "Kind", &event.kind);
                        mono_line(ui, "Peer ID", &event.peer_id);
                        if let Some(channel_id) = &event.channel_id {
                            mono_line(ui, "Channel", channel_id);
                        }
                        if let Some(message_id) = &event.message_id {
                            mono_line(ui, "Message", message_id);
                        }
                        if let Some(reason) = &event.reason {
                            mono_line(ui, "Reason", reason);
                        }
                        ui.label(format!("Received ms: {}", event.received_ms));
                    }
                });

                ui.add_space(10.0);
            }
        });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }

        let mut open = self.show_settings;

        egui::Window::new("Settings")
            .open(&mut open)
            .default_width(460.0)
            .default_height(500.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("Settings");
                ui.label(
                    egui::RichText::new("Core controls and advanced details.").color(color_muted()),
                );

                ui.add_space(14.0);

                glass_panel(ui, |ui| {
                    ui.heading("Core");

                    ui.horizontal_wrapped(|ui| {
                        status_dot(
                            ui,
                            if self.core_online() {
                                color_success()
                            } else {
                                color_error()
                            },
                        );

                        ui.label(if self.core_online() {
                            "Online"
                        } else {
                            "Offline"
                        });

                        if ui.add(primary_button("Start")).clicked() {
                            self.start_core();
                        }

                        if ui.add(secondary_button("Shutdown")).clicked() {
                            self.send_job(ApiJob::Shutdown);
                        }

                        if ui.add(secondary_button("Refresh")).clicked() {
                            self.refresh_all();
                        }
                    });

                    if let Some(status) = &self.status {
                        ui.separator();
                        ui.label(format!("Device: {}", status.device_name));
                        ui.label(format!("Version: {}", status.version));
                        ui.label(format!("Uptime: {}", format_duration_ms(status.uptime_ms)));

                        if status.psk_configured {
                            ui.label(
                                egui::RichText::new("Security key configured.")
                                    .color(color_success()),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new("Security key missing.").color(color_warning()),
                            );
                        }
                    }
                });

                ui.add_space(12.0);

                glass_panel(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.heading("Advanced");
                        toggle_switch(ui, &mut self.show_advanced);
                    });

                    if self.show_advanced {
                        ui.separator();

                        if let Some(status) = &self.status {
                            mono_line(ui, "API", &status.api_addr);
                            mono_line(ui, "Device ID", &status.device_id);
                        }

                        if let Some(paths) = &self.paths {
                            ui.separator();
                            mono_line(ui, "AppData", &paths.app_dir);
                            mono_line(ui, "Config", &paths.config_file);
                            mono_line(ui, "Trusted devices", &paths.trusted_devices_file);
                            mono_line(ui, "Add-ons", &paths.addons_dir);
                            mono_line(ui, "Logs", &paths.logs_dir);
                            mono_line(ui, "Runtime", &paths.runtime_dir);
                            mono_line(ui, "State", &paths.state_dir);
                            mono_line(ui, "Lock", &paths.lock_file);
                        }
                    } else {
                        ui.label(
                            egui::RichText::new("Technical IDs and paths are hidden.")
                                .color(color_muted()),
                        );
                    }
                });

                ui.add_space(12.0);

                glass_panel(ui, |ui| {
                    ui.heading("Messages");

                    egui::ScrollArea::vertical()
                        .max_height(130.0)
                        .show(ui, |ui| {
                            for line in &self.log {
                                ui.label(line);
                            }
                        });

                    if ui.add(secondary_button("Clear")).clicked() {
                        self.log.clear();
                    }
                });
            });

        self.show_settings = open;
    }
}

fn api_worker(rx: mpsc::Receiver<ApiJob>, tx: mpsc::Sender<UiMsg>) {
    while let Ok(job) = rx.recv() {
        let job_name = job_name(&job).to_string();

        let request = match job {
            ApiJob::Status => json!({ "cmd": "status" }),
            ApiJob::Paths => json!({ "cmd": "paths" }),
            ApiJob::Peers => json!({ "cmd": "list_peers" }),
            ApiJob::Trusted => json!({ "cmd": "list_trusted_devices" }),
            ApiJob::Connections => json!({ "cmd": "list_connections" }),
            ApiJob::Addons => json!({ "cmd": "list_addons" }),
            ApiJob::ReloadAddons => json!({ "cmd": "reload_addons" }),
            ApiJob::Shutdown => json!({ "cmd": "shutdown" }),
            ApiJob::PollEvents { service } => {
                let mut req = json!({
                    "cmd": "poll_events",
                    "max_events": 100
                });

                if let Some(service) = service {
                    req["service"] = json!(service);
                }

                req
            }
            ApiJob::AddTrusted { name, mac } => json!({
                "cmd": "add_trusted_device",
                "name": name,
                "mac": mac
            }),
            ApiJob::RemoveTrusted { mac } => json!({
                "cmd": "remove_trusted_device",
                "mac": mac
            }),
            ApiJob::Connect { mac, peer_id } => {
                let mut req = json!({ "cmd": "connect_device" });

                if let Some(mac) = mac {
                    req["mac"] = json!(mac);
                }

                if let Some(peer_id) = peer_id {
                    req["peer_id"] = json!(peer_id);
                }

                req
            }
            ApiJob::Disconnect { mac, peer_id } => {
                let mut req = json!({ "cmd": "disconnect_device" });

                if let Some(mac) = mac {
                    req["mac"] = json!(mac);
                }

                if let Some(peer_id) = peer_id {
                    req["peer_id"] = json!(peer_id);
                }

                req
            }
        };

        let result = api_request(request);

        let msg = match result {
            Ok(value) => UiMsg::ApiOk {
                job: job_name,
                value,
            },
            Err(error) => UiMsg::ApiErr {
                job: job_name,
                error: error.to_string(),
            },
        };

        let _ = tx.send(msg);
    }
}

fn job_name(job: &ApiJob) -> &'static str {
    match job {
        ApiJob::Status => "status",
        ApiJob::Paths => "paths",
        ApiJob::Peers => "peers",
        ApiJob::Trusted => "trusted",
        ApiJob::Connections => "connections",
        ApiJob::Addons => "addons",
        ApiJob::PollEvents { .. } => "poll_events",
        ApiJob::AddTrusted { .. } => "add_trusted",
        ApiJob::RemoveTrusted { .. } => "remove_trusted",
        ApiJob::Connect { .. } => "connect",
        ApiJob::Disconnect { .. } => "disconnect",
        ApiJob::ReloadAddons => "reload_addons",
        ApiJob::Shutdown => "shutdown",
    }
}

fn paint_background(ui: &mut egui::Ui) {
    let rect = ui.max_rect();
    let painter = ui.painter();

    painter.rect_filled(rect, 0.0, color_bg());

    let glow_1 = egui::Rect::from_min_size(
        rect.left_top() + egui::vec2(-60.0, 40.0),
        egui::vec2(260.0, 220.0),
    );
    painter.rect_filled(glow_1, 140.0, color_accent_dark().linear_multiply(0.23));

    let glow_2 = egui::Rect::from_min_size(
        rect.right_bottom() - egui::vec2(260.0, 240.0),
        egui::vec2(310.0, 260.0),
    );
    painter.rect_filled(glow_2, 160.0, color_accent().linear_multiply(0.10));
}

fn nav_tab(ui: &mut egui::Ui, current: &mut Screen, tab: Screen, label: &str, width: f32) {
    let selected = *current == tab;
    let desired_size = egui::vec2(width, 34.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());

    if response.clicked() {
        *current = tab;
    }

    let hovered = response.hovered();

    let fill = if selected {
        color_accent_dark()
    } else if hovered {
        color_card_hover().linear_multiply(0.45)
    } else {
        egui::Color32::TRANSPARENT
    };

    let stroke = if selected {
        egui::Stroke::new(1.15, color_accent())
    } else if hovered {
        egui::Stroke::new(1.0, color_border().linear_multiply(0.85))
    } else {
        egui::Stroke::new(1.0, color_border().linear_multiply(0.38))
    };

    ui.painter()
        .rect(rect, 17.0, fill, stroke, egui::StrokeKind::Inside);

    if selected {
        let inner = rect.shrink(2.0);
        ui.painter().rect_stroke(
            inner,
            16.0,
            egui::Stroke::new(1.0, color_accent().linear_multiply(0.28)),
            egui::StrokeKind::Inside,
        );

        let underline = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, rect.bottom() - 4.5),
            egui::vec2((width * 0.34).max(18.0), 2.0),
        );

        ui.painter()
            .rect_filled(underline, 1.0, color_accent().linear_multiply(0.85));
    }

    let text_color = if selected {
        color_text()
    } else if hovered {
        color_text().linear_multiply(0.88)
    } else {
        color_muted()
    };

    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(13.5),
        text_color,
    );
}

fn primary_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(
        egui::RichText::new(text)
            .color(color_text())
            .size(14.0)
            .strong(),
    )
    .fill(color_accent_dark())
    .stroke(egui::Stroke::new(1.0, color_accent()))
    .min_size(egui::vec2(96.0, 36.0))
}

fn secondary_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(egui::RichText::new(text).color(color_text()).size(14.0))
        .fill(color_panel())
        .stroke(egui::Stroke::new(1.0, color_border()))
        .min_size(egui::vec2(94.0, 36.0))
}

fn danger_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(egui::RichText::new(text).color(color_error()).size(14.0))
        .fill(color_panel())
        .stroke(egui::Stroke::new(1.0, color_error().linear_multiply(0.55)))
        .min_size(egui::vec2(82.0, 36.0))
}

fn icon_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(egui::RichText::new(text).color(color_text()).size(18.0))
        .fill(color_panel())
        .stroke(egui::Stroke::new(1.0, color_border()))
        .min_size(egui::vec2(44.0, 38.0))
}

fn page_title(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.heading(egui::RichText::new(title).color(color_text()).size(24.0));
    ui.label(
        egui::RichText::new(subtitle)
            .color(color_muted())
            .size(13.5),
    );
}

fn glass_panel<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(color_panel())
        .stroke(egui::Stroke::new(1.0, color_border()))
        .rounding(egui::Rounding::same(22))
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, add)
        .inner
}

fn device_card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(color_card())
        .stroke(egui::Stroke::new(1.0, color_border()))
        .rounding(egui::Rounding::same(18))
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, add)
        .inner
}

fn notice(ui: &mut egui::Ui, title: &str, body: &str, color: egui::Color32) {
    glass_panel(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            status_dot(ui, color);
            ui.heading(egui::RichText::new(title).color(color));
        });

        ui.add_space(4.0);

        ui.label(egui::RichText::new(body).color(color_muted()).size(14.0));
    });
}

fn state_chip(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    egui::Frame::none()
        .fill(color.linear_multiply(0.13))
        .stroke(egui::Stroke::new(1.0, color.linear_multiply(0.7)))
        .rounding(egui::Rounding::same(255))
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).color(color).size(13.0).strong());
        });
}

fn status_dot(ui: &mut egui::Ui, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(13.0, 13.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, color);
    ui.painter().circle_stroke(
        rect.center(),
        6.0,
        egui::Stroke::new(1.0, color.linear_multiply(0.55)),
    );
}

fn toggle_switch(ui: &mut egui::Ui, value: &mut bool) -> egui::Response {
    let desired_size = egui::vec2(58.0, 32.0);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, egui::Sense::click());

    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }

    let t = if *value { 1.0 } else { 0.0 };
    let bg = if *value {
        color_accent_dark()
    } else {
        color_bg_dark()
    };
    let knob = if *value {
        color_accent()
    } else {
        color_muted()
    };

    ui.painter().rect(
        rect,
        16.0,
        bg,
        egui::Stroke::new(
            1.0,
            if *value {
                color_accent()
            } else {
                color_border()
            },
        ),
        egui::StrokeKind::Inside,
    );

    let x = egui::lerp((rect.left() + 16.0)..=(rect.right() - 16.0), t);
    let center = egui::pos2(x, rect.center().y);

    ui.painter().circle_filled(center, 11.5, knob);
    ui.painter().circle_stroke(
        center,
        11.5,
        egui::Stroke::new(1.0, color_text().linear_multiply(0.35)),
    );

    response
}

fn mono_line(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(format!("{label}:")).color(color_muted()));
        ui.label(
            egui::RichText::new(value)
                .monospace()
                .color(color_text())
                .size(12.5),
        );
    });
}

fn readonly_line(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(format!("{label}:")).color(color_muted()));
        let mut s = value.to_string();
        ui.add(
            egui::TextEdit::singleline(&mut s)
                .desired_width(340.0)
                .interactive(false),
        );
    });
}

fn human_event_title(event: &EventRow) -> String {
    match event.kind.as_str() {
        "service_data" => "Message received".to_string(),
        "channel_open" => "Channel opened".to_string(),
        "channel_data" => "Channel data".to_string(),
        "channel_close" => "Channel closed".to_string(),
        other => other.to_string(),
    }
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

fn optional_str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(|x| x.to_string())
}

fn bool_field(v: &Value, key: &str) -> bool {
    v.get(key).and_then(|x| x.as_bool()).unwrap_or(false)
}

fn u128_field(v: &Value, key: &str) -> u128 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0) as u128
}

fn string_array_field(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn api_request(req: Value) -> Result<Value> {
    let mut stream = TcpStream::connect(LOCAL_API_ADDR)
        .with_context(|| format!("could not connect to LocalLink Core API at {LOCAL_API_ADDR}"))?;

    let line = serde_json::to_string(&req)?;
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;

    if response.trim().is_empty() {
        bail!("empty response from LocalLink Core API");
    }

    let value: Value = serde_json::from_str(&response)?;

    if value.get("ok").and_then(|x| x.as_bool()) == Some(false) {
        let error = value
            .get("error")
            .and_then(|x| x.as_str())
            .unwrap_or("unknown API error");
        bail!("{error}");
    }

    Ok(value)
}

fn set_manifest_enabled(manifest_path: &str, enabled: bool) -> Result<()> {
    let text = fs::read_to_string(manifest_path)
        .with_context(|| format!("reading manifest {manifest_path}"))?;

    let mut manifest: AddonManifest =
        serde_json::from_str(&text).with_context(|| format!("parsing manifest {manifest_path}"))?;

    manifest.enabled = enabled;

    fs::write(manifest_path, serde_json::to_string_pretty(&manifest)?)
        .with_context(|| format!("writing manifest {manifest_path}"))?;

    Ok(())
}

fn launch_addon(addon: &AddonRow) -> Result<Child> {
    let exe_path = Path::new(&addon.addon_dir).join(&addon.executable);

    if !exe_path.exists() {
        bail!("add-on executable not found: {}", exe_path.display());
    }

    let child = Command::new(&exe_path)
        .current_dir(Path::new(&addon.addon_dir))
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("launching {}", exe_path.display()))?;

    Ok(child)
}

fn start_sibling_core() -> Result<()> {
    let current = std::env::current_exe()?;
    let dir = current
        .parent()
        .ok_or_else(|| anyhow::anyhow!("could not determine UI executable folder"))?;

    let core = dir.join("locallink-core.exe");

    if !core.exists() {
        bail!("sibling locallink-core.exe not found at {}", core.display());
    }

    Command::new(core)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;

    Ok(())
}

fn normalize_mac_ui(mac: &str) -> String {
    let hex: String = mac
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_lowercase())
        .collect();

    if hex.len() != 12 {
        return String::new();
    }

    hex.as_bytes()
        .chunks(2)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or(""))
        .collect::<Vec<_>>()
        .join(":")
}

fn format_duration_ms(ms: u128) -> String {
    if ms < 1_000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1000.0)
    } else {
        format!("{:.1} min", ms as f64 / 60_000.0)
    }
}

fn color_bg() -> egui::Color32 {
    egui::Color32::from_rgb(4, 10, 22)
}

fn color_bg_dark() -> egui::Color32 {
    egui::Color32::from_rgb(3, 7, 16)
}

fn color_panel() -> egui::Color32 {
    egui::Color32::from_rgb(8, 22, 42)
}

fn color_card() -> egui::Color32 {
    egui::Color32::from_rgb(12, 32, 60)
}

fn color_card_hover() -> egui::Color32 {
    egui::Color32::from_rgb(18, 48, 86)
}

fn color_border() -> egui::Color32 {
    egui::Color32::from_rgb(38, 86, 138)
}

fn color_accent() -> egui::Color32 {
    egui::Color32::from_rgb(88, 202, 255)
}

fn color_accent_dark() -> egui::Color32 {
    egui::Color32::from_rgb(20, 92, 160)
}

fn color_success() -> egui::Color32 {
    egui::Color32::from_rgb(96, 235, 178)
}

fn color_warning() -> egui::Color32 {
    egui::Color32::from_rgb(255, 196, 92)
}

fn color_error() -> egui::Color32 {
    egui::Color32::from_rgb(255, 100, 126)
}

fn color_text() -> egui::Color32 {
    egui::Color32::from_rgb(235, 246, 255)
}

fn color_muted() -> egui::Color32 {
    egui::Color32::from_rgb(145, 174, 205)
}

fn base64_decode_simple(s: &str) -> Result<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;

    while i + 3 < bytes.len() {
        let c0 = val(bytes[i]).ok_or_else(|| anyhow::anyhow!("bad base64"))?;
        let c1 = val(bytes[i + 1]).ok_or_else(|| anyhow::anyhow!("bad base64"))?;
        let c2 = if bytes[i + 2] == b'=' {
            None
        } else {
            val(bytes[i + 2])
        };
        let c3 = if bytes[i + 3] == b'=' {
            None
        } else {
            val(bytes[i + 3])
        };

        out.push((c0 << 2) | (c1 >> 4));

        if let Some(c2) = c2 {
            out.push(((c1 & 0b00001111) << 4) | (c2 >> 2));

            if let Some(c3) = c3 {
                out.push(((c2 & 0b00000011) << 6) | c3);
            }
        }

        i += 4;
    }

    Ok(out)
}
