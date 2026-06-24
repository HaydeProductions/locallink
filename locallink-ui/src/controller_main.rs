use anyhow::{bail, Context, Result};
use eframe::egui;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::time::Duration;

const LOCAL_API_ADDR: &str = "127.0.0.1:47900";

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LocalLink")
            .with_inner_size([720.0, 680.0])
            .with_min_inner_size([500.0, 480.0]),
        ..Default::default()
    };

    eframe::run_native(
        "LocalLink",
        options,
        Box::new(|_cc| Ok(Box::new(LocalLinkUi::new()))),
    )
}

#[derive(Debug, Clone)]
enum ApiJob {
    Status,
    ListPeers,
    ListSpaces,
    ListAddons,
    ListSpaceAddons,
    CreateSpace {
        name: String,
        kind: String,
        member: String,
    },
    AddSpaceMember {
        space_id: String,
        member: String,
    },
    SetSpaceAddon {
        space_id: String,
        addon_id: String,
        enabled: bool,
    },
}

impl ApiJob {
    fn name(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::ListPeers => "list_peers",
            Self::ListSpaces => "list_spaces",
            Self::ListAddons => "list_addons",
            Self::ListSpaceAddons => "list_space_addons",
            Self::CreateSpace { .. } => "create_space",
            Self::AddSpaceMember { .. } => "add_space_member",
            Self::SetSpaceAddon { .. } => "set_space_addon_enabled",
        }
    }

    fn request(&self) -> Value {
        match self {
            Self::Status => json!({ "cmd": "status" }),
            Self::ListPeers => json!({ "cmd": "list_peers" }),
            Self::ListSpaces => json!({ "cmd": "list_spaces" }),
            Self::ListAddons => json!({ "cmd": "list_addons" }),
            Self::ListSpaceAddons => json!({ "cmd": "list_space_addons" }),
            Self::CreateSpace { name, kind, member } => json!({
                "cmd": "create_space",
                "space_name": name,
                "space_kind": kind,
                "member_peer_id": empty_to_null(member)
            }),
            Self::AddSpaceMember { space_id, member } => json!({
                "cmd": "add_space_member",
                "space_id": space_id,
                "member_peer_id": member
            }),
            Self::SetSpaceAddon {
                space_id,
                addon_id,
                enabled,
            } => json!({
                "cmd": "set_space_addon_enabled",
                "space_id": space_id,
                "addon_id": addon_id,
                "enabled": enabled
            }),
        }
    }
}

#[derive(Debug)]
enum UiMsg {
    ApiOk { job: String, value: Value },
    ApiErr { job: String, error: String },
}

struct LocalLinkUi {
    tx: mpsc::Sender<ApiJob>,
    rx: mpsc::Receiver<UiMsg>,
    core_status: String,
    peers_json: String,
    spaces_json: String,
    addons_json: String,
    space_addons_json: String,
    log: Vec<String>,
    new_space_name: String,
    new_space_kind: String,
    member_peer_id: String,
    edit_space_id: String,
    edit_addon_id: String,
}

impl LocalLinkUi {
    fn new() -> Self {
        let (job_tx, job_rx) = mpsc::channel::<ApiJob>();
        let (msg_tx, msg_rx) = mpsc::channel::<UiMsg>();
        std::thread::spawn(move || api_worker(job_rx, msg_tx));

        let refresh_tx = job_tx.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(1500));
            let _ = refresh_tx.send(ApiJob::Status);
            let _ = refresh_tx.send(ApiJob::ListPeers);
            let _ = refresh_tx.send(ApiJob::ListSpaces);
            let _ = refresh_tx.send(ApiJob::ListSpaceAddons);
        });

        let mut app = Self {
            tx: job_tx,
            rx: msg_rx,
            core_status: String::from("Core status unknown"),
            peers_json: String::new(),
            spaces_json: String::new(),
            addons_json: String::new(),
            space_addons_json: String::new(),
            log: Vec::new(),
            new_space_name: String::from("New space"),
            new_space_kind: String::from("direct"),
            member_peer_id: String::new(),
            edit_space_id: String::new(),
            edit_addon_id: String::new(),
        };
        app.refresh_all();
        app
    }

    fn refresh_all(&mut self) {
        self.send(ApiJob::Status);
        self.send(ApiJob::ListPeers);
        self.send(ApiJob::ListSpaces);
        self.send(ApiJob::ListAddons);
        self.send(ApiJob::ListSpaceAddons);
    }

    fn send(&mut self, job: ApiJob) {
        if let Err(error) = self.tx.send(job) {
            self.log(format!("UI worker unavailable: {error}"));
        }
    }

    fn log(&mut self, message: impl Into<String>) {
        self.log.push(message.into());
        if self.log.len() > 120 {
            self.log.remove(0);
        }
    }

    fn pump_messages(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                UiMsg::ApiOk { job, value } => self.apply_api_ok(&job, value),
                UiMsg::ApiErr { job, error } => {
                    if job == "status" {
                        self.core_status = String::from("Core offline");
                    }
                    self.log(format!("{job}: {error}"));
                }
            }
        }
    }

    fn apply_api_ok(&mut self, job: &str, value: Value) {
        match job {
            "status" => {
                let data = value.get("data").cloned().unwrap_or_default();
                self.core_status = format!(
                    "Core online: {} / {}",
                    field(&data, "device_name"),
                    field(&data, "version")
                );
            }
            "list_peers" => self.peers_json = pretty(&value),
            "list_spaces" => self.spaces_json = pretty(&value),
            "list_addons" => self.addons_json = pretty(&value),
            "list_space_addons" => self.space_addons_json = pretty(&value),
            "create_space" | "add_space_member" | "set_space_addon_enabled" => {
                self.log(format!("{job} complete"));
                self.send(ApiJob::ListSpaces);
                self.send(ApiJob::ListSpaceAddons);
            }
            _ => {}
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
                ui.label(&self.core_status);
                if ui.button("Refresh").clicked() {
                    self.refresh_all();
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Spaces controller");
            ui.label("The UI configures Core through the local API. It does not own add-on processes.");
            ui.separator();

            ui.horizontal_wrapped(|ui| {
                ui.label("Space name");
                ui.text_edit_singleline(&mut self.new_space_name);
                ui.radio_value(&mut self.new_space_kind, "direct".to_string(), "Direct");
                ui.radio_value(&mut self.new_space_kind, "group".to_string(), "Group");
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("Initial/add member peer ID");
                ui.text_edit_singleline(&mut self.member_peer_id);
                if ui.button("Create space").clicked() {
                    self.send(ApiJob::CreateSpace {
                        name: self.new_space_name.trim().to_string(),
                        kind: self.new_space_kind.clone(),
                        member: self.member_peer_id.trim().to_string(),
                    });
                }
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("Selected space ID");
                ui.text_edit_singleline(&mut self.edit_space_id);
                if ui.button("Add member to selected space").clicked() {
                    self.send(ApiJob::AddSpaceMember {
                        space_id: self.edit_space_id.trim().to_string(),
                        member: self.member_peer_id.trim().to_string(),
                    });
                }
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("Add-on ID");
                ui.text_edit_singleline(&mut self.edit_addon_id);
                if ui.button("Enable add-on for selected space").clicked() {
                    self.send(ApiJob::SetSpaceAddon {
                        space_id: self.edit_space_id.trim().to_string(),
                        addon_id: self.edit_addon_id.trim().to_string(),
                        enabled: true,
                    });
                }
                if ui.button("Disable add-on for selected space").clicked() {
                    self.send(ApiJob::SetSpaceAddon {
                        space_id: self.edit_space_id.trim().to_string(),
                        addon_id: self.edit_addon_id.trim().to_string(),
                        enabled: false,
                    });
                }
            });

            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Spaces");
                ui.code(&self.spaces_json);
                ui.heading("Space add-ons");
                ui.code(&self.space_addons_json);
                ui.heading("Installed add-ons");
                ui.code(&self.addons_json);
                ui.heading("Peers");
                ui.code(&self.peers_json);
                ui.heading("Log");
                for line in self.log.iter().rev().take(40) {
                    ui.label(line);
                }
            });
        });
    }
}

fn api_worker(rx: mpsc::Receiver<ApiJob>, tx: mpsc::Sender<UiMsg>) {
    while let Ok(job) = rx.recv() {
        let job_name = job.name().to_string();
        match api_request(job.request()) {
            Ok(value) => {
                if value.get("ok").and_then(|x| x.as_bool()).unwrap_or(false) {
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

fn empty_to_null(value: &str) -> Value {
    if value.trim().is_empty() {
        Value::Null
    } else {
        Value::String(value.trim().to_string())
    }
}

fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}
