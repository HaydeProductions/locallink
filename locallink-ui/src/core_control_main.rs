#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use anyhow::{bail, Context, Result};
use eframe::egui;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const LOCAL_API_ADDR: &str = "127.0.0.1:47900";

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LocalLink")
            .with_inner_size([470.0, 640.0])
            .with_min_inner_size([390.0, 520.0])
            .with_icon(local_link_window_icon()),
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
    Spaces,
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
    Spaces,
    CreateSpace {
        name: String,
        kind: String,
    },
    ActivateSpace {
        space_id: String,
    },
    DeactivateSpace {
        space_id: String,
    },
    AddSpaceMember {
        space_id: String,
        peer_id: String,
    },
    RemoveSpaceMember {
        space_id: String,
        peer_id: String,
    },
    AcceptSpaceInvite {
        space_id: String,
    },
    DeclineSpaceInvite {
        space_id: String,
    },
    LeaveSpace {
        space_id: String,
    },
    DeleteSpace {
        space_id: String,
    },
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
struct SpaceRow {
    id: String,
    name: String,
    kind: String,
    active: bool,
    members: Vec<String>,
    addon_count: usize,
    role: String,
    owner_device_id: String,
    local_state: String,
    can_accept_invite: bool,
    can_decline_invite: bool,
    can_connect: bool,
    can_disconnect: bool,
    can_leave: bool,
    can_invite_members: bool,
    can_remove_members: bool,
    can_manage_addons: bool,
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
    spaces: Vec<SpaceRow>,
    events: Vec<EventRow>,

    log: Vec<String>,
    loading_count: usize,
    last_refresh: Option<Instant>,

    show_settings: bool,
    show_advanced: bool,

    add_name: String,
    add_mac: String,
    event_filter: String,
    space_name: String,
    space_kind_group: bool,
    space_member_peer_id: String,
}

impl LocalLinkUi {
    fn new() -> Self {
        let (job_tx, job_rx) = mpsc::channel::<ApiJob>();
        let (msg_tx, msg_rx) = mpsc::channel::<UiMsg>();

        std::thread::spawn(move || api_worker(job_rx, msg_tx));

        // Background live-refresh loop.
        // This keeps the UI feeling live without blocking the UI thread.
        // A true push/subscription system can replace this later.
        let refresh_tx = job_tx.clone();
        std::thread::spawn(move || {
            let mut tick: u64 = 0;

            loop {
                std::thread::sleep(Duration::from_millis(1500));
                tick += 1;

                let _ = refresh_tx.send(ApiJob::Status);
                let _ = refresh_tx.send(ApiJob::Peers);
                let _ = refresh_tx.send(ApiJob::Trusted);
                let _ = refresh_tx.send(ApiJob::Connections);

                if tick % 3 == 0 {
                    let _ = refresh_tx.send(ApiJob::Spaces);
                    let _ = refresh_tx.send(ApiJob::Addons);
                }

                if tick % 2 == 0 {
                    let _ = refresh_tx.send(ApiJob::PollEvents { service: None });
                }
            }
        });

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
            spaces: Vec::new(),
            events: Vec::new(),

            log: Vec::new(),
            loading_count: 0,
            last_refresh: None,

            show_settings: false,
            show_advanced: false,

            add_name: String::new(),
            add_mac: String::new(),
            event_filter: String::new(),
            space_name: String::new(),
            space_kind_group: false,
            space_member_peer_id: String::new(),
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
        self.send_job(ApiJob::Spaces);
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
            Screen::Spaces => {
                self.send_job(ApiJob::Spaces);
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
            "spaces" => self.apply_spaces(value),
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
                self.send_job(ApiJob::Spaces);
                self.send_job(ApiJob::Addons);
            }
            "disconnect" => {
                self.log("Disconnected.");
                self.send_job(ApiJob::Peers);
                self.send_job(ApiJob::Trusted);
                self.send_job(ApiJob::Connections);
                self.send_job(ApiJob::Spaces);
                self.send_job(ApiJob::Addons);
            }
            "shutdown" => {
                self.status = None;
                self.log("Core shutdown requested.");
            }
            "create_space" => {
                self.log("Space created.");
                self.space_name.clear();
                self.send_job(ApiJob::Spaces);
            }
            "activate_space" => {
                self.log("Space connected.");
                self.send_job(ApiJob::Spaces);
                self.send_job(ApiJob::Addons);
            }
            "deactivate_space" => {
                self.log("Space disconnected.");
                self.send_job(ApiJob::Spaces);
                self.send_job(ApiJob::Addons);
            }
            "add_space_member" => {
                self.log("Space invite sent.");
                self.space_member_peer_id.clear();
                self.send_job(ApiJob::Spaces);
            }
            "remove_space_member" => {
                self.log("Space member removed.");
                self.send_job(ApiJob::Spaces);
            }
            "accept_space_invite" => {
                self.log("Space invite accepted.");
                self.send_job(ApiJob::Spaces);
                self.send_job(ApiJob::Addons);
            }
            "decline_space_invite" => {
                self.log("Space invite declined.");
                self.send_job(ApiJob::Spaces);
            }
            "leave_space" => {
                self.log("Left space.");
                self.send_job(ApiJob::Spaces);
                self.send_job(ApiJob::Addons);
            }
            "delete_space" => {
                self.log("Space deleted.");
                self.send_job(ApiJob::Spaces);
                self.send_job(ApiJob::Addons);
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

    fn apply_spaces(&mut self, v: Value) {
        self.spaces.clear();

        if let Some(rows) = v.get("data").and_then(|x| x.as_array()) {
            for row in rows {
                let addon_count = row
                    .get("addons")
                    .and_then(|x| x.as_object())
                    .map(|addons| addons.len())
                    .unwrap_or(0);

                self.spaces.push(SpaceRow {
                    id: str_field(row, "space_id"),
                    name: str_field(row, "name"),
                    kind: str_field(row, "kind"),
                    active: bool_field(row, "active"),
                    members: string_array_field(row, "members"),
                    addon_count,
                    role: str_field(row, "role"),
                    owner_device_id: str_field(row, "owner_device_id"),
                    local_state: str_field(row, "local_state"),
                    can_accept_invite: bool_field(row, "can_accept_invite"),
                    can_decline_invite: bool_field(row, "can_decline_invite"),
                    can_connect: bool_field(row, "can_connect"),
                    can_disconnect: bool_field(row, "can_disconnect"),
                    can_leave: bool_field(row, "can_leave"),
                    can_invite_members: bool_field(row, "can_invite_members"),
                    can_remove_members: bool_field(row, "can_remove_members"),
                    can_manage_addons: bool_field(row, "can_manage_addons"),
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
        force_stop_core_processes();
        std::thread::sleep(Duration::from_millis(200));

        match start_sibling_core() {
            Ok(()) => {
                self.log("Starting LocalLink Core...");
                std::thread::sleep(Duration::from_millis(250));
                self.refresh_all();
            }
            Err(e) => self.log(format!("Could not start core: {e}")),
        }
    }

    fn stop_core(&mut self) {
        self.send_job(ApiJob::Shutdown);
        force_stop_core_processes();

        self.status = None;
        self.peers.clear();
        self.connections.clear();
        self.spaces.clear();
        self.addons.clear();

        self.log("Stopped LocalLink Core.");
    }


}

impl eframe::App for LocalLinkUi {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Keep the widget live even when the user is not moving the mouse.
        // Without this, egui may not repaint immediately after background API updates.
        ctx.request_repaint_after(Duration::from_millis(150));

        self.apply_style(ctx);
        self.pump_messages();

        egui::CentralPanel::default().show(ctx, |ui| {
            paint_background(ui);

            egui::Frame::none()
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    self.header(ui);
                    ui.add_space(14.0);
                    self.tabs(ui);
                    ui.add_space(18.0);

                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            // Subtle right-side breathing room so the scrollbar
                            // does not visually touch the cards. This does not
                            // introduce a separate gutter or change the outer layout.
                            let content_width = (ui.available_width() - 10.0).max(260.0);
                            ui.set_width(content_width);

                            match self.screen {
                                Screen::Discover => self.screen_discover(ui),
                                Screen::Devices => self.screen_devices(ui),
                                Screen::Spaces => self.screen_spaces(ui),
                                Screen::Addons => self.screen_addons(ui),
                                Screen::Activity => self.screen_activity(ui),
                            }

                            ui.add_space(28.0);
                        });
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
                    if ui
                        .add(icon_button("⚙"))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        self.show_settings = true;
                    }
                    if self.core_online() {
                        if ui
                            .add(danger_button("Stop Core"))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.stop_core();
                        }
                    } else if ui
                        .add(primary_button("Start"))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
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
        let container_width: f32 = available_width.min(430.0).max(350.0);
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
        let tab_count: f32 = 5.0;
        let tab_width = (inner.width() - gap * (tab_count - 1.0)) / tab_count;
        let tab_height = inner.height();

        let tabs = [
            (Screen::Discover, "Discover"),
            (Screen::Devices, "Devices"),
            (Screen::Spaces, "Spaces"),
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

            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }

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
                            if ui
                                .add(primary_button("Add Device"))
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                self.send_job(ApiJob::AddTrusted {
                                    name: peer.device_name.clone(),
                                    mac: primary_mac.clone(),
                                });
                            }
                        } else if !peer.connected {
                            if ui
                                .add(primary_button("Connect"))
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
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

            if ui
                .add(primary_button("Add Trusted Device"))
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
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
                            if ui
                                .add(secondary_button("Disconnect"))
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                self.send_job(ApiJob::Disconnect {
                                    mac: trusted.macs.first().cloned(),
                                    peer_id: Some(conn.device_id.clone()),
                                });
                            }
                        } else if let Some(peer) = nearby_peer.clone() {
                            if ui
                                .add(primary_button("Connect"))
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                self.send_job(ApiJob::Connect {
                                    mac: trusted.macs.first().cloned(),
                                    peer_id: Some(peer.device_id.clone()),
                                });
                            }
                        } else {
                            ui.add_enabled(false, primary_button("Connect"));
                        }

                        if ui
                            .add(danger_button("Remove"))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
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
            "Installed add-ons are owned by Core and enabled per connection space.",
        );

        ui.add_space(14.0);

        notice(
            ui,
            "Core-owned runtime",
            "The UI no longer starts or stops add-ons directly. Per-space controls will wire into the Core API next.",
            color_accent(),
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

        egui::ScrollArea::vertical().show(ui, |ui| {
            for addon in &self.addons {
                let title = ellipsize(&format!("{} {}", addon.name, addon.version), 30);
                let description = ellipsize(&addon.description, 82);
                let executable = ellipsize(&addon.executable, 42);
                let services = ellipsize(&addon.services.join(", "), 54);
                let folder = ellipsize(&addon.addon_dir, 54);
                let manifest = ellipsize(&addon.manifest_path, 54);

                device_card(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_min_width(0.0);
                            ui.set_max_width((ui.available_width() - 110.0).max(180.0));

                            ui.label(
                                egui::RichText::new(title)
                                    .color(color_text())
                                    .size(21.0)
                                    .strong(),
                            );

                            ui.add_space(4.0);

                            ui.label(
                                egui::RichText::new(description)
                                    .color(color_muted())
                                    .size(14.0),
                            );
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            state_chip(ui, "Core-owned", color_accent());
                        });
                    });

                    ui.add_space(16.0);

                    egui::Frame::none()
                        .fill(color_panel().linear_multiply(0.82))
                        .stroke(egui::Stroke::new(1.0, color_border().linear_multiply(0.45)))
                        .rounding(egui::Rounding::same(18))
                        .inner_margin(egui::Margin::symmetric(16, 13))
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new("Available to spaces")
                                            .color(color_text())
                                            .size(15.0)
                                            .strong(),
                                    );

                                    ui.add_space(2.0);

                                    ui.label(
                                        egui::RichText::new(
                                            "Desired state is configured per connection space.",
                                        )
                                        .color(color_muted())
                                        .size(12.5),
                                    );
                                });

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        state_chip(ui, "Per-space", color_muted());
                                    },
                                );
                            });
                        });

                    if self.show_advanced {
                        ui.separator();
                        mono_line(ui, "ID", &ellipsize(&addon.id, 44));
                        mono_line(ui, "Executable", &executable);
                        mono_line(ui, "Services", &services);
                        mono_line(ui, "Folder", &folder);
                        mono_line(ui, "Manifest", &manifest);
                        mono_line(
                            ui,
                            "Legacy enabled flag",
                            if addon.enabled { "true" } else { "false" },
                        );
                    }
                });

                ui.add_space(12.0);
            }
        });
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

                if ui
                    .add(primary_button("Check"))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    let service = if self.event_filter.trim().is_empty() {
                        None
                    } else {
                        Some(self.event_filter.trim().to_string())
                    };

                    self.send_job(ApiJob::PollEvents { service });
                }

                if ui
                    .add(secondary_button("Clear"))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
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
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
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

                        if ui
                            .add(primary_button("Start"))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.start_core();
                        }

                        if ui
                            .add(danger_button("Stop Core"))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.send_job(ApiJob::Shutdown);
                        }

                        if ui
                            .add(secondary_button("Refresh"))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
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

                self.network_requirements_panel(ui);

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

                    if ui
                        .add(secondary_button("Clear"))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        self.log.clear();
                    }
                });
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
            ApiJob::Spaces => json!({ "cmd": "list_spaces" }),
            ApiJob::CreateSpace { name, kind } => json!({
                "cmd": "create_space",
                "name": name,
                "kind": kind
            }),
            ApiJob::ActivateSpace { space_id } => json!({
                "cmd": "activate_space",
                "space_id": space_id
            }),
            ApiJob::DeactivateSpace { space_id } => json!({
                "cmd": "deactivate_space",
                "space_id": space_id
            }),
            ApiJob::AddSpaceMember { space_id, peer_id } => json!({
                "cmd": "add_space_member",
                "space_id": space_id,
                "peer_id": peer_id
            }),
            ApiJob::RemoveSpaceMember { space_id, peer_id } => json!({
                "cmd": "remove_space_member",
                "space_id": space_id,
                "peer_id": peer_id
            }),
            ApiJob::AcceptSpaceInvite { space_id } => json!({
                "cmd": "accept_space_invite",
                "space_id": space_id
            }),
            ApiJob::DeclineSpaceInvite { space_id } => json!({
                "cmd": "decline_space_invite",
                "space_id": space_id
            }),
            ApiJob::LeaveSpace { space_id } => json!({
                "cmd": "leave_space",
                "space_id": space_id
            }),
            ApiJob::DeleteSpace { space_id } => json!({
                "cmd": "delete_space",
                "space_id": space_id
            }),
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
        ApiJob::Spaces => "spaces",
        ApiJob::CreateSpace { .. } => "create_space",
        ApiJob::ActivateSpace { .. } => "activate_space",
        ApiJob::DeactivateSpace { .. } => "deactivate_space",
        ApiJob::AddSpaceMember { .. } => "add_space_member",
        ApiJob::RemoveSpaceMember { .. } => "remove_space_member",
        ApiJob::AcceptSpaceInvite { .. } => "accept_space_invite",
        ApiJob::DeclineSpaceInvite { .. } => "decline_space_invite",
        ApiJob::LeaveSpace { .. } => "leave_space",
        ApiJob::DeleteSpace { .. } => "delete_space",
        ApiJob::PollEvents { .. } => "poll_events",
        ApiJob::AddTrusted { .. } => "add_trusted",
        ApiJob::RemoveTrusted { .. } => "remove_trusted",
        ApiJob::Connect { .. } => "connect",
        ApiJob::Disconnect { .. } => "disconnect",
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

    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

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

    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

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

fn ellipsize(input: &str, max_chars: usize) -> String {
    let trimmed = input.trim();

    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }

    if max_chars <= 3 {
        return "...".to_string();
    }

    let keep = max_chars - 3;
    let mut out: String = trimmed.chars().take(keep).collect();
    out.push_str("...");
    out
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

fn start_sibling_core() -> Result<()> {
    let current = std::env::current_exe()?;
    let dir = current
        .parent()
        .ok_or_else(|| anyhow::anyhow!("could not determine UI executable folder"))?;

    let core = dir.join("locallink-core.exe");

    if !core.exists() {
        bail!("sibling locallink-core.exe not found at {}", core.display());
    }

    let mut command = Command::new(core);
    command
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW

    command.spawn()?;

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


impl LocalLinkUi {
    fn space_member_candidates(&self) -> Vec<(String, String, String)> {
        let mut candidates = Vec::<(String, String, String)>::new();

        for connection in &self.connections {
            let id = connection.device_id.clone();
            if !id.trim().is_empty() && !candidates.iter().any(|(existing, _, _)| existing == &id) {
                candidates.push((id, connection.device_name.clone(), "Connected".to_string()));
            }
        }

        for peer in &self.peers {
            let id = peer.device_id.clone();
            if !id.trim().is_empty() && !candidates.iter().any(|(existing, _, _)| existing == &id) {
                let source = if peer.trusted { "Nearby trusted" } else { "Nearby" };
                candidates.push((id, peer.device_name.clone(), source.to_string()));
            }
        }

        for trusted in &self.trusted {
            if let Some(device_id) = &trusted.device_id {
                let id = device_id.clone();
                if !id.trim().is_empty() && !candidates.iter().any(|(existing, _, _)| existing == &id) {
                    candidates.push((id, trusted.name.clone(), "Trusted".to_string()));
                }
            }
        }

        candidates
    }

    fn screen_spaces(&mut self, ui: &mut egui::Ui) {
        page_title(
            ui,
            "Spaces",
            "Owned spaces and joined spaces are separate. Pending invitations must be accepted before they can connect.",
        );

        ui.add_space(14.0);

        if !self.core_online() {
            notice(
                ui,
                "Core is offline",
                "Start LocalLink Core to load connection spaces.",
                color_error(),
            );
            return;
        }

        glass_panel(ui, |ui| {
            ui.heading(egui::RichText::new("Create owned space").color(color_text()));
            ui.label(
                egui::RichText::new("Spaces created here are owned by this device. Only owned spaces can invite or remove members.")
                    .color(color_muted()),
            );

            ui.add_space(8.0);

            ui.horizontal_wrapped(|ui| {
                ui.label("Name");
                ui.add(
                    egui::TextEdit::singleline(&mut self.space_name)
                        .desired_width(170.0)
                        .hint_text("Gaming PC space"),
                );
            });

            ui.horizontal_wrapped(|ui| {
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

            if ui
                .add(primary_button("Create Owned Space"))
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                let name = self.space_name.trim().to_string();
                if name.is_empty() {
                    self.log("Space name is required.");
                } else {
                    let kind = if self.space_kind_group { "group" } else { "direct" }.to_string();
                    self.send_job(ApiJob::CreateSpace { name, kind });
                }
            }
        });

        ui.add_space(14.0);

        if self.spaces.is_empty() {
            notice(
                ui,
                "No spaces yet",
                "Create an owned space above, or wait for a space invite from another device.",
                color_warning(),
            );
            return;
        }

        let device_candidates = self.space_member_candidates();

        for space in self.spaces.clone() {
                device_card(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_min_width(0.0);
                            ui.set_max_width((ui.available_width() - 110.0).max(180.0));

                            ui.label(
                                egui::RichText::new(&space.name)
                                    .color(color_text())
                                    .size(21.0)
                                    .strong(),
                            );

                            ui.add_space(4.0);

                            let member_summary = if space.members.is_empty() {
                                "No members".to_string()
                            } else if space.members.len() == 1 {
                                "1 member".to_string()
                            } else {
                                format!("{} members", space.members.len())
                            };

                            let owner_summary = if space.role == "owner" {
                                "Owned by this device".to_string()
                            } else if space.owner_device_id.is_empty() {
                                "Joined foreign space".to_string()
                            } else {
                                format!("Owner: {}", ellipsize(&space.owner_device_id, 30))
                            };

                            ui.label(
                                egui::RichText::new(format!("{} · {}", member_summary, owner_summary))
                                    .color(color_muted())
                                    .size(14.0),
                            );
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            let state_color = match space.local_state.as_str() {
                                "owned" => color_accent(),
                                "joined" => color_success(),
                                "invite_pending" => color_warning(),
                                "removed" | "left" => color_error(),
                                _ => color_muted(),
                            };
                            let state_label = match space.local_state.as_str() {
                                "owned" => "Owned",
                                "joined" => "Joined",
                                "invite_pending" => "Invite pending",
                                "invite_declined" => "Invite declined",
                                "removed" => "Removed",
                                "left" => "Left",
                                _ => "Unknown",
                            };
                            state_chip(ui, state_label, state_color);
                            state_chip(ui, &space.kind, if space.kind == "group" { color_accent() } else { color_success() });
                            state_chip(ui, if space.active { "Active" } else { "Inactive" }, if space.active { color_success() } else { color_muted() });
                        });
                    });

                    ui.add_space(12.0);

                    if space.local_state == "invite_pending" {
                        notice(
                            ui,
                            "Invitation pending",
                            "This is a foreign space invite. Accept it to join, or decline it. Connecting does not accept invites automatically.",
                            color_warning(),
                        );
                        ui.add_space(8.0);
                    } else if space.local_state == "removed" {
                        notice(
                            ui,
                            "Removed from space",
                            "The owner removed this device from the group. The space has been disconnected locally.",
                            color_error(),
                        );
                        ui.add_space(8.0);
                    } else if space.local_state == "left" {
                        notice(
                            ui,
                            "Left space",
                            "This device has left the foreign space. Create or accept a new invite to join again.",
                            color_muted(),
                        );
                        ui.add_space(8.0);
                    }

                    ui.horizontal_wrapped(|ui| {
                        if space.can_accept_invite && ui
                            .add(primary_button("Accept Invite"))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.send_job(ApiJob::AcceptSpaceInvite { space_id: space.id.clone() });
                        }

                        if space.can_decline_invite && ui
                            .add(danger_button("Decline Invite"))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.send_job(ApiJob::DeclineSpaceInvite { space_id: space.id.clone() });
                        }

                        if space.can_disconnect && ui
                            .add(danger_button("Disconnect Space"))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.send_job(ApiJob::DeactivateSpace { space_id: space.id.clone() });
                        }

                        if space.can_connect && ui
                            .add(primary_button("Connect Space"))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.send_job(ApiJob::ActivateSpace { space_id: space.id.clone() });
                        }

                        if space.can_leave && ui
                            .add(danger_button("Leave Group"))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.send_job(ApiJob::LeaveSpace { space_id: space.id.clone() });
                        }

                        let can_delete_local_copy = space.local_state == "removed" || space.local_state == "left";
                        if (space.role == "owner" || can_delete_local_copy) && ui
                            .add(danger_button(if space.role == "owner" { "Delete Space" } else { "Delete Local Copy" }))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.send_job(ApiJob::DeleteSpace { space_id: space.id.clone() });
                        }

                        ui.label(
                            egui::RichText::new("Disconnect only affects local activity. Leave exits a foreign group. Deleted/removed foreign spaces can be cleared locally.")
                                .color(color_muted())
                                .size(12.5),
                        );
                    });

                    ui.add_space(14.0);

                    egui::Frame::none()
                        .fill(color_panel().linear_multiply(0.82))
                        .stroke(egui::Stroke::new(1.0, color_border().linear_multiply(0.45)))
                        .rounding(egui::Rounding::same(18))
                        .inner_margin(egui::Margin::symmetric(16, 13))
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new("Per-space add-ons")
                                            .color(color_text())
                                            .size(15.0)
                                            .strong(),
                                    );

                                    ui.add_space(2.0);

                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{} configured desired state(s).",
                                            space.addon_count
                                        ))
                                        .color(color_muted())
                                        .size(12.5),
                                    );
                                });

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if space.can_manage_addons {
                                            state_chip(ui, "Owner managed", color_success());
                                        } else {
                                            state_chip(ui, "Owner controlled", color_muted());
                                        }
                                    },
                                );
                            });
                        });

                    ui.add_space(10.0);

                    glass_panel(ui, |ui| {
                        ui.heading(egui::RichText::new("Members").color(color_text()).size(16.0));

                        if space.members.is_empty() {
                            ui.label(egui::RichText::new("No accepted members yet.").color(color_muted()));
                        } else {
                            for member in &space.members {
                                ui.horizontal_wrapped(|ui| {
                                    mono_line(ui, "Peer", &ellipsize(member, 42));

                                    if space.can_remove_members && member != &space.owner_device_id && ui
                                        .add(danger_button("Remove"))
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .clicked()
                                    {
                                        self.send_job(ApiJob::RemoveSpaceMember {
                                            space_id: space.id.clone(),
                                            peer_id: member.clone(),
                                        });
                                    }
                                });
                            }
                        }

                        ui.separator();

                        if !space.can_invite_members {
                            ui.label(
                                egui::RichText::new("This is a foreign space. Only the owner can invite or remove members.")
                                    .color(color_muted()),
                            );
                            return;
                        }

                        if device_candidates.is_empty() {
                            ui.label(
                                egui::RichText::new(
                                    "No discovered, connected, or trusted device IDs available yet. Open Discover/Devices or connect a peer first.",
                                )
                                .color(color_muted()),
                            );
                        } else {
                            ui.label(egui::RichText::new("Pick a device to invite").color(color_text()).strong());
                            ui.add_space(4.0);

                            for (peer_id, label, source) in &device_candidates {
                                let already_member = space.members.iter().any(|member| member == peer_id);
                                ui.horizontal_wrapped(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(egui::RichText::new(label).color(color_text()).strong());
                                        ui.label(
                                            egui::RichText::new(format!("{} · {}", source, ellipsize(peer_id, 28)))
                                                .color(color_muted())
                                                .size(12.5),
                                        );
                                    });

                                    if already_member {
                                        state_chip(ui, "Already member", color_success());
                                    } else if ui
                                        .add(primary_button("Invite this device"))
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .clicked()
                                    {
                                        self.space_member_peer_id = peer_id.clone();
                                    }
                                });
                            }

                            ui.separator();
                        }

                        ui.horizontal_wrapped(|ui| {
                            ui.label("Peer ID");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.space_member_peer_id)
                                    .desired_width(210.0)
                                    .hint_text("auto-filled from device picker"),
                            );

                            if ui
                                .add(primary_button("Send Invite"))
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                let peer_id = self.space_member_peer_id.trim().to_string();
                                if peer_id.is_empty() {
                                    self.log("Pick a device to invite or enter a Peer ID first.");
                                } else {
                                    self.send_job(ApiJob::AddSpaceMember {
                                        space_id: space.id.clone(),
                                        peer_id,
                                    });
                                }
                            }
                        });
                    });

                    if self.show_advanced {
                        ui.separator();
                        mono_line(ui, "Space ID", &ellipsize(&space.id, 60));
                        mono_line(ui, "Kind", &space.kind);
                        mono_line(ui, "Role", &space.role);
                        mono_line(ui, "State", &space.local_state);
                        mono_line(ui, "Owner", &space.owner_device_id);
                        mono_line(ui, "Active", if space.active { "true" } else { "false" });
                        mono_line(ui, "Add-ons", &space.addon_count.to_string());
                    }
                });

                ui.add_space(12.0);
            }
    }
}


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
