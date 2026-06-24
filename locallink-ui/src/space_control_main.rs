use anyhow::{bail, Context, Result};
use eframe::egui;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const LOCAL_API_ADDR: &str = "127.0.0.1:47900";

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LocalLink")
            .with_inner_size([760.0, 720.0])
            .with_min_inner_size([520.0, 520.0]),
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
    Spaces,
    Devices,
    Addons,
    Activity,
}

#[derive(Debug, Clone)]
enum ApiJob {
    Status,
    Peers,
    Trusted,
    Connections,
    Addons,
    Spaces,
    SpaceAddons,
    PollEvents,
    AddTrusted { name: String, mac: String },
    RemoveTrusted { mac: String },
    Connect { mac: Option<String>, peer_id: Option<String> },
    Disconnect { mac: Option<String>, peer_id: Option<String> },
    CreateSpace { name: String, kind: String, member: Option<String> },
    DeleteSpace { space_id: String },
    RenameSpace { space_id: String, name: String },
    AddSpaceMember { space_id: String, member: String },
    RemoveSpaceMember { space_id: String, member: String },
    SetSpaceAddon { space_id: String, addon_id: String, enabled: bool },
    ReloadAddons,
    Shutdown,
}

impl ApiJob {
    fn name(&self) -> &'static str {
        match self {
            ApiJob::Status => "status",
            ApiJob::Peers => "peers",
            ApiJob::Trusted => "trusted",
            ApiJob::Connections => "connections",
            ApiJob::Addons => "addons",
            ApiJob::Spaces => "spaces",
            ApiJob::SpaceAddons => "space_addons",
            ApiJob::PollEvents => "poll_events",
            ApiJob::AddTrusted { .. } => "add_trusted",
            ApiJob::RemoveTrusted { .. } => "remove_trusted",
            ApiJob::Connect { .. } => "connect",
            ApiJob::Disconnect { .. } => "disconnect",
            ApiJob::CreateSpace { .. } => "create_space",
            ApiJob::DeleteSpace { .. } => "delete_space",
            ApiJob::RenameSpace { .. } => "rename_space",
            ApiJob::AddSpaceMember { .. } => "add_space_member",
            ApiJob::RemoveSpaceMember { .. } => "remove_space_member",
            ApiJob::SetSpaceAddon { .. } => "set_space_addon_enabled",
            ApiJob::ReloadAddons => "reload_addons",
            ApiJob::Shutdown => "shutdown",
        }
    }

    fn request(&self) -> Value {
        match self {
            ApiJob::Status => json!({ "cmd": "status" }),
            ApiJob::Peers => json!({ "cmd": "list_peers" }),
            ApiJob::Trusted => json!({ "cmd": "list_trusted_devices" }),
            ApiJob::Connections => json!({ "cmd": "list_connections" }),
            ApiJob::Addons => json!({ "cmd": "list_addons" }),
            ApiJob::Spaces => json!({ "cmd": "list_spaces" }),
            ApiJob::SpaceAddons => json!({ "cmd": "list_space_addons" }),
            ApiJob::PollEvents => json!({ "cmd": "poll_events", "max_events": 200 }),
            ApiJob::AddTrusted { name, mac } => json!({
                "cmd": "add_trusted_device",
                "name": name,
                "mac": mac
            }),
            ApiJob::RemoveTrusted { mac } => json!({
                "cmd": "remove_trusted_device",
                "mac": mac
            }),
            ApiJob::Connect { mac, peer_id } => json!({
                "cmd": "connect_device",
                "mac": mac,
                "peer_id": peer_id
            }),
            ApiJob::Disconnect { mac, peer_id } => json!({
                "cmd": "disconnect_device",
                "mac": mac,
                "peer_id": peer_id
            }),
            ApiJob::CreateSpace { name, kind, member } => json!({
                "cmd": "create_space",
                "space_name": name,
                "space_kind": kind,
                "member_peer_id": member
            }),
            ApiJob::DeleteSpace { space_id } => json!({
                "cmd": "delete_space",
                "space_id": space_id
            }),
            ApiJob::RenameSpace { space_id, name } => json!({
                "cmd": "rename_space",
                "space_id": space_id,
                "space_name": name
            }),
            ApiJob::AddSpaceMember { space_id, member } => json!({
                "cmd": "add_space_member",
                "space_id": space_id,
                "member_peer_id": member
            }),
            ApiJob::RemoveSpaceMember { space_id, member } => json!({
                "cmd": "remove_space_member",
                "space_id": space_id,
                "member_peer_id": member
            }),
            ApiJob::SetSpaceAddon { space_id, addon_id, enabled } => json!({
                "cmd": "set_space_addon_enabled",
                "space_id": space_id,
                "addon_id": addon_id,
                "enabled": enabled
            }),
            ApiJob::ReloadAddons => json!({ "cmd": "reload_addons" }),
            ApiJob::Shutdown => json!({ "cmd": "shutdown" }),
        }
    }
}

#[derive(Debug)]
enum UiMsg {
    ApiOk { job: String, value: Value },
    ApiErr { job: String, error: String },
}

#[derive(Debug, Clone, Default)]
struct PeerRow {
    device_id: String,
    device_name: String,
    addr: String,
    macs: Vec<String>,
    trusted: bool,
    connected: bool,
}

#[derive(Debug, Clone, Default)]
struct TrustedRow {
    name: String,
    macs: Vec<String>,
    device_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ConnectionRow {
    device_id: String,
    device_name: String,
    addr: String,
}

#[derive(Debug, Clone, Default)]
struct AddonRow {
    id: String,
    name: String,
    version: String,
    description: String,
    services: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct SpaceRow {
    space_id: String,
    name: String,
    kind: String,
    members: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct SpaceAddonRow {
    addon_id: String,
    name: String,
    version: String,
    enabled: bool,
}

#[derive(Debug, Clone, Default)]
struct EventRow {
    kind: String,
    peer_name: String,
    peer_id: String,
    service: String,
    space_id: Option<String>,
    target_peer_id: Option<String>,
    message_id: Option<String>,
    received_ms: u128,
}

struct LocalLinkUi {
    screen: Screen,
    tx: mpsc::Sender<ApiJob>,
    rx: mpsc::Receiver<UiMsg>,
    core_online: bool,
    core_label: String,
    peers: Vec<PeerRow>,
    trusted: Vec<TrustedRow>,
    connections: Vec<ConnectionRow>,
    addons: Vec<AddonRow>,
    spaces: Vec<SpaceRow>,
    space_addons: BTreeMap<String, Vec<SpaceAddonRow>>,
    events: Vec<EventRow>,
    log: Vec<String>,
    loading_count: usize,
    last_refresh: Option<Instant>,
    new_space_name: String,
    new_space_kind: String,
    space_member_peer_id: String,
    rename_space_name: String,
    add_name: String,
    add_mac: String,
}

impl LocalLinkUi {
    fn new() -> Self {
        let (job_tx, job_rx) = mpsc::channel::<ApiJob>();
        let (msg_tx, msg_rx) = mpsc::channel::<UiMsg>();
        std::thread::spawn(move || api_worker(job_rx, msg_tx));

        let refresh_tx = job_tx.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(1500));
            for job in [
                ApiJob::Status,
                ApiJob::Peers,
                ApiJob::Trusted,
                ApiJob::Connections,
                ApiJob::Spaces,
                ApiJob::SpaceAddons,
                ApiJob::PollEvents,
            ] {
                let _ = refresh_tx.send(job);
            }
        });

        let mut app = Self {
            screen: Screen::Spaces,
            tx: job_tx,
            rx: msg_rx,
            core_online: false,
            core_label: String::from("Core offline"),
            peers: Vec::new(),
            trusted: Vec::new(),
            connections: Vec::new(),
            addons: Vec::new(),
            spaces: Vec::new(),
            space_addons: BTreeMap::new(),
            events: Vec::new(),
            log: Vec::new(),
            loading_count: 0,
            last_refresh: None,
            new_space_name: String::from("New space"),
            new_space_kind: String::from("direct"),
            space_member_peer_id: String::new(),
            rename_space_name: String::new(),
            add_name: String::new(),
            add_mac: String::new(),
        };
        app.refresh_all();
        app
    }

    fn send_job(&mut self, job: ApiJob) {
        self.loading_count += 1;
        if let Err(err) = self.tx.send(job) {
            self.loading_count = self.loading_count.saturating_sub(1);
            self.log(format!("UI worker unavailable: {err}"));
        }
    }

    fn refresh_all(&mut self) {
        for job in [
            ApiJob::Status,
            ApiJob::Peers,
            ApiJob::Trusted,
            ApiJob::Connections,
            ApiJob::Addons,
            ApiJob::Spaces,
            ApiJob::SpaceAddons,
            ApiJob::PollEvents,
        ] {
            self.send_job(job);
        }
        self.last_refresh = Some(Instant::now());
    }

    fn log(&mut self, msg: impl Into<String>) {
        self.log.push(msg.into());
        if self.log.len() > 160 {
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
                        self.core_online = false;
                        self.core_label = String::from("Core offline");
                    }
                    self.log(format!("{job}: {error}"));
                }
            }
        }
    }

    fn handle_api_ok(&mut self, job: &str, value: Value) {
        match job {
            "status" => self.apply_status(value),
            "peers" => self.apply_peers(value),
            "trusted" => self.apply_trusted(value),
            "connections" => self.apply_connections(value),
            "addons" | "reload_addons" => self.apply_addons(value),
            "spaces" => self.apply_spaces(value),
            "space_addons" => self.apply_space_addons(value),
            "poll_events" => self.apply_events(value),
            "create_space" | "delete_space" | "rename_space" | "add_space_member"
            | "remove_space_member" | "set_space_addon_enabled" => {
                self.log(format!("{job} complete."));
                self.send_job(ApiJob::Spaces);
                self.send_job(ApiJob::SpaceAddons);
            }
            "add_trusted" | "remove_trusted" | "connect" | "disconnect" => {
                self.log(format!("{job} complete."));
                self.send_job(ApiJob::Peers);
                self.send_job(ApiJob::Trusted);
                self.send_job(ApiJob::Connections);
            }
            "shutdown" => {
                self.core_online = false;
                self.core_label = String::from("Core shutdown requested");
            }
            _ => {}
        }
    }

    fn apply_status(&mut self, value: Value) {
        if let Some(data) = value.get("data") {
            self.core_online = true;
            self.core_label = format!(
                "Core online: {} / {} / uptime {}ms",
                str_field(data, "device_name"),
                str_field(data, "version"),
                u128_field(data, "uptime_ms")
            );
        }
    }

    fn apply_peers(&mut self, value: Value) {
        self.peers.clear();
        if let Some(rows) = value.get("data").and_then(|x| x.as_array()) {
            for row in rows {
                self.peers.push(PeerRow {
                    device_id: str_field(row, "device_id"),
                    device_name: str_field(row, "device_name"),
                    addr: str_field(row, "addr"),
                    macs: string_array_field(row, "macs"),
                    trusted: bool_field(row, "trusted"),
                    connected: bool_field(row, "connected"),
                });
            }
        }
    }

    fn apply_trusted(&mut self, value: Value) {
        self.trusted.clear();
        if let Some(rows) = value.get("data").and_then(|x| x.as_array()) {
            for row in rows {
                self.trusted.push(TrustedRow {
                    name: str_field(row, "name"),
                    macs: string_array_field(row, "macs"),
                    device_id: optional_str_field(row, "device_id"),
                });
            }
        }
    }

    fn apply_connections(&mut self, value: Value) {
        self.connections.clear();
        if let Some(rows) = value.get("data").and_then(|x| x.as_array()) {
            for row in rows {
                self.connections.push(ConnectionRow {
                    device_id: str_field(row, "device_id"),
                    device_name: str_field(row, "device_name"),
                    addr: str_field(row, "addr"),
                });
            }
        }
    }

    fn apply_addons(&mut self, value: Value) {
        self.addons.clear();
        if let Some(rows) = value.get("data").and_then(|x| x.as_array()) {
            for row in rows {
                self.addons.push(AddonRow {
                    id: str_field(row, "id"),
                    name: str_field(row, "name"),
                    version: str_field(row, "version"),
                    description: str_field(row, "description"),
                    services: string_array_field(row, "services"),
                });
            }
        }
    }

    fn apply_spaces(&mut self, value: Value) {
        self.spaces.clear();
        let Some(data) = value.get("data") else { return };
        let Some(rows) = data.get("spaces").and_then(|x| x.as_array()) else { return };
        for row in rows {
            self.spaces.push(SpaceRow {
                space_id: str_field(row, "space_id"),
                name: str_field(row, "name"),
                kind: str_field(row, "kind"),
                members: string_array_field(row, "members"),
            });
        }
    }

    fn apply_space_addons(&mut self, value: Value) {
        self.space_addons.clear();
        if let Some(rows) = value.get("data").and_then(|x| x.as_array()) {
            for row in rows {
                let space_id = str_field(row, "space_id");
                let mut addons = Vec::new();
                if let Some(addon_rows) = row.get("addons").and_then(|x| x.as_array()) {
                    for addon in addon_rows {
                        addons.push(SpaceAddonRow {
                            addon_id: str_field(addon, "addon_id"),
                            name: str_field(addon, "name"),
                            version: str_field(addon, "version"),
                            enabled: bool_field(addon, "enabled"),
                        });
                    }
                }
                self.space_addons.insert(space_id, addons);
            }
        }
    }

    fn apply_events(&mut self, value: Value) {
        if let Some(rows) = value.get("data").and_then(|x| x.as_array()) {
            for row in rows {
                self.events.push(EventRow {
                    kind: str_field(row, "kind"),
                    peer_name: str_field(row, "peer_name"),
                    peer_id: str_field(row, "peer_id"),
                    service: str_field(row, "service"),
                    space_id: optional_str_field(row, "space_id"),
                    target_peer_id: optional_str_field(row, "target_peer_id"),
                    message_id: optional_str_field(row, "message_id"),
                    received_ms: u128_field(row, "received_ms"),
                });
            }
        }
        if self.events.len() > 400 {
            let keep_from = self.events.len().saturating_sub(400);
            self.events.drain(0..keep_from);
        }
    }
}

impl eframe::App for LocalLinkUi {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(150));
        self.pump_messages();

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("LocalLink");
                ui.label(&self.core_label);
                if ui.button("Shutdown Core").clicked() {
                    self.send_job(ApiJob::Shutdown);
                }
                if ui.button("Refresh").clicked() {
                    self.refresh_all();
                }
                if self.loading_count > 0 {
                    ui.label(format!("{} pending", self.loading_count));
                }
            });
        });

        egui::SidePanel::left("tabs").resizable(false).show(ctx, |ui| {
            ui.heading("Sections");
            ui.selectable_value(&mut self.screen, Screen::Spaces, "Spaces");
            ui.selectable_value(&mut self.screen, Screen::Devices, "Devices");
            ui.selectable_value(&mut self.screen, Screen::Addons, "Add-ons");
            ui.selectable_value(&mut self.screen, Screen::Activity, "Activity");
            ui.separator();
            ui.small("The UI configures Core. It does not launch add-on processes.");
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.screen {
            Screen::Spaces => self.screen_spaces(ui),
            Screen::Devices => self.screen_devices(ui),
            Screen::Addons => self.screen_addons(ui),
            Screen::Activity => self.screen_activity(ui),
        });
    }
}

impl LocalLinkUi {
    fn screen_spaces(&mut self, ui: &mut egui::Ui) {
        ui.heading("Spaces");
        ui.label("Direct and group spaces are the user-facing connection layer.");
        ui.separator();

        ui.horizontal_wrapped(|ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut self.new_space_name);
            ui.radio_value(&mut self.new_space_kind, "direct".to_string(), "Direct");
            ui.radio_value(&mut self.new_space_kind, "group".to_string(), "Group");
            ui.label("Initial member peer ID");
            ui.text_edit_singleline(&mut self.space_member_peer_id);
            if ui.button("Create space").clicked() {
                self.send_job(ApiJob::CreateSpace {
                    name: self.new_space_name.trim().to_string(),
                    kind: self.new_space_kind.clone(),
                    member: non_empty(&self.space_member_peer_id),
                });
            }
        });

        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for space in self.spaces.clone() {
                ui.group(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.heading(format!("{} ({})", space.name, space.kind));
                        if ui.button("Delete").clicked() {
                            self.send_job(ApiJob::DeleteSpace {
                                space_id: space.space_id.clone(),
                            });
                        }
                    });
                    ui.monospace(format!("space_id: {}", space.space_id));
                    ui.label(format!("members: {}", space.members.join(", ")));

                    ui.horizontal_wrapped(|ui| {
                        ui.label("Rename to");
                        ui.text_edit_singleline(&mut self.rename_space_name);
                        if ui.button("Rename").clicked() && !self.rename_space_name.trim().is_empty() {
                            self.send_job(ApiJob::RenameSpace {
                                space_id: space.space_id.clone(),
                                name: self.rename_space_name.trim().to_string(),
                            });
                        }
                    });

                    ui.horizontal_wrapped(|ui| {
                        ui.label("Member peer ID");
                        ui.text_edit_singleline(&mut self.space_member_peer_id);
                        if ui.button("Add member").clicked()
                            && !self.space_member_peer_id.trim().is_empty()
                        {
                            self.send_job(ApiJob::AddSpaceMember {
                                space_id: space.space_id.clone(),
                                member: self.space_member_peer_id.trim().to_string(),
                            });
                        }
                    });

                    for member in space.members.clone() {
                        ui.horizontal(|ui| {
                            ui.monospace(&member);
                            if ui.button("Remove member").clicked() {
                                self.send_job(ApiJob::RemoveSpaceMember {
                                    space_id: space.space_id.clone(),
                                    member,
                                });
                            }
                        });
                    }

                    ui.separator();
                    ui.label("Space add-ons");
                    if let Some(addons) = self.space_addons.get(&space.space_id).cloned() {
                        for addon in addons {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(format!("{} {}", addon.name, addon.version));
                                let label = if addon.enabled { "Disable" } else { "Enable" };
                                if ui.button(label).clicked() {
                                    self.send_job(ApiJob::SetSpaceAddon {
                                        space_id: space.space_id.clone(),
                                        addon_id: addon.addon_id.clone(),
                                        enabled: !addon.enabled,
                                    });
                                }
                            });
                        }
                    }
                });
                ui.add_space(12.0);
            }
        });
    }

    fn screen_devices(&mut self, ui: &mut egui::Ui) {
        ui.heading("Devices");
        ui.label("Devices are transport peers. Spaces sit above these secure sessions.");
        ui.separator();

        ui.horizontal_wrapped(|ui| {
            ui.label("Trusted name");
            ui.text_edit_singleline(&mut self.add_name);
            ui.label("MAC");
            ui.text_edit_singleline(&mut self.add_mac);
            if ui.button("Add trusted").clicked() {
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

        ui.separator();
        ui.heading("Nearby peers");
        for peer in self.peers.clone() {
            ui.group(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("{} — {}", peer.device_name, peer.addr));
                    if peer.connected {
                        ui.label("connected");
                    } else if ui.button("Connect").clicked() {
                        self.send_job(ApiJob::Connect {
                            mac: peer.macs.first().cloned(),
                            peer_id: Some(peer.device_id.clone()),
                        });
                    }
                    if ui.button("Disconnect").clicked() {
                        self.send_job(ApiJob::Disconnect {
                            mac: peer.macs.first().cloned(),
                            peer_id: Some(peer.device_id.clone()),
                        });
                    }
                });
                ui.monospace(&peer.device_id);
                ui.label(format!("MACs: {}", peer.macs.join(", ")));
                ui.label(if peer.trusted { "trusted" } else { "not trusted" });
            });
        }

        ui.separator();
        ui.heading("Trusted devices");
        for trusted in self.trusted.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{} — {}", trusted.name, trusted.macs.join(", ")));
                if let Some(id) = &trusted.device_id {
                    ui.monospace(id);
                }
                if let Some(mac) = trusted.macs.first() {
                    if ui.button("Remove").clicked() {
                        self.send_job(ApiJob::RemoveTrusted { mac: mac.clone() });
                    }
                }
            });
        }

        ui.separator();
        ui.heading("Active secure sessions");
        for conn in self.connections.clone() {
            ui.label(format!("{} — {} — {}", conn.device_name, conn.device_id, conn.addr));
        }
    }

    fn screen_addons(&mut self, ui: &mut egui::Ui) {
        ui.heading("Installed add-ons");
        ui.label("Enable add-ons inside a Space. Core owns the add-on processes.");
        if ui.button("Reload manifests").clicked() {
            self.send_job(ApiJob::ReloadAddons);
        }
        ui.separator();

        for addon in self.addons.clone() {
            ui.group(|ui| {
                ui.heading(format!("{} {}", addon.name, addon.version));
                ui.label(&addon.description);
                ui.monospace(format!("id: {}", addon.id));
                ui.label(format!("services: {}", addon.services.join(", ")));
            });
            ui.add_space(8.0);
        }
    }

    fn screen_activity(&mut self, ui: &mut egui::Ui) {
        ui.heading("Activity");
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for event in self.events.iter().rev().take(200) {
                ui.group(|ui| {
                    ui.label(format!(
                        "{} / service={} / peer={} ({})",
                        event.kind, event.service, event.peer_name, event.peer_id
                    ));
                    if let Some(space_id) = &event.space_id {
                        ui.monospace(format!("space_id: {space_id}"));
                    }
                    if let Some(target_peer_id) = &event.target_peer_id {
                        ui.monospace(format!("target_peer_id: {target_peer_id}"));
                    }
                    if let Some(message_id) = &event.message_id {
                        ui.monospace(format!("message_id: {message_id}"));
                    }
                    ui.small(format!("received_ms: {}", event.received_ms));
                });
            }
        });

        ui.separator();
        ui.heading("Log");
        for line in self.log.iter().rev().take(40) {
            ui.label(line);
        }
    }
}

fn api_worker(rx: mpsc::Receiver<ApiJob>, tx: mpsc::Sender<UiMsg>) {
    while let Ok(job) = rx.recv() {
        let job_name = job.name().to_string();
        match api_request(job.request()) {
            Ok(value) => {
                let api_ok = value.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
                if api_ok {
                    let _ = tx.send(UiMsg::ApiOk { job: job_name, value });
                } else {
                    let error = value
                        .get("error")
                        .and_then(|x| x.as_str())
                        .unwrap_or("unknown API error")
                        .to_string();
                    let _ = tx.send(UiMsg::ApiErr { job: job_name, error });
                }
            }
            Err(error) => {
                let _ = tx.send(UiMsg::ApiErr {
                    job: job_name,
                    error: error.to_string(),
                });
            }
        }
    }
}

fn api_request(req: Value) -> Result<Value> {
    let mut stream = TcpStream::connect(LOCAL_API_ADDR)
        .with_context(|| format!("could not connect to LocalLink Core API at {LOCAL_API_ADDR}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

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

    Ok(serde_json::from_str(&response)?)
}

fn str_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

fn optional_str_field(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(ToString::to_string)
}

fn bool_field(value: &Value, key: &str) -> bool {
    value.get(key).and_then(|x| x.as_bool()).unwrap_or(false)
}

fn u128_field(value: &Value, key: &str) -> u128 {
    value.get(key).and_then(|x| x.as_u64()).unwrap_or(0) as u128
}

fn string_array_field(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(|x| x.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
