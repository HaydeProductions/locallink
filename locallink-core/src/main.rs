mod addons;
mod api;
mod api_client;
mod config;
mod discovery;
mod protocol;
mod spaces;
mod transport;

use addons::{load_addon_manifests, AddonRecord};
use anyhow::Result;
use config::{
    acquire_single_instance_lock, config_path, generate_psk_b64, init_app_dirs,
    load_or_create_config, save_config, validate_psk_b64,
};
use discovery::Peer;
use spaces::load_or_create_space_store;
use std::collections::{HashMap, HashSet};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tokio::time::{sleep, Duration, Instant};
use transport::{tcp_server, ConnectedPeer, ConnectionRegistry, EventStore, RunOptions};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    init_app_dirs()?;

    let args: Vec<String> = std::env::args().collect();

    if let Some(pos) = args.iter().position(|a| a == "--api") {
        let api_args = &args[(pos + 1)..];
        return api_client::run_api_client(api_args);
    }

    if args.iter().any(|a| a == "--gen-psk") {
        let mut cfg = load_or_create_config()?;
        let psk = generate_psk_b64();
        cfg.psk_b64 = Some(psk.clone());
        save_config(&cfg)?;

        println!("Generated and saved new PSK.");
        println!("Copy this exact PSK to the other device:");
        println!("{psk}");
        println!();
        println!("Config: {}", config_path()?.display());
        return Ok(());
    }

    if let Some(pos) = args.iter().position(|a| a == "--set-psk") {
        let Some(psk) = args.get(pos + 1) else {
            anyhow::bail!("Usage: locallink-core --set-psk <base64-psk>");
        };

        validate_psk_b64(psk)?;

        let mut cfg = load_or_create_config()?;
        cfg.psk_b64 = Some(psk.clone());
        save_config(&cfg)?;

        println!("Saved PSK.");
        println!("Config: {}", config_path()?.display());
        return Ok(());
    }

    let _single_instance_lock = acquire_single_instance_lock()?;
    let started_at = Instant::now();

    let opts = RunOptions {
        bench: args.iter().any(|a| a == "--bench"),
    };

    let cfg = load_or_create_config()?;
    let loaded_addons = load_addon_manifests()?;
    let loaded_spaces = load_or_create_space_store()?;

    println!("LocalLink core prototype");
    println!("Version:     {}", env!("CARGO_PKG_VERSION"));
    println!("Device name: {}", cfg.device_name);
    println!("Device ID:   {}", cfg.device_id);
    println!("Config:      {}", config_path()?.display());
    println!("Addons:      {}", loaded_addons.len());
    println!("Spaces:      {}", loaded_spaces.spaces.len());

    if cfg.psk_b64.is_none() {
        println!();
        println!("No PSK configured yet.");
        println!("On one laptop, run:");
        println!("  cargo run --release -- --gen-psk");
        println!();
        println!("Then copy the printed PSK to the other laptop and run:");
        println!("  cargo run --release -- --set-psk \"PASTE_PSK_HERE\"");
        println!();
        println!("You can also run --set-psk on this laptop if needed.");
        return Ok(());
    }

    println!("PSK:         configured");
    println!("Connect:     manual only");

    if opts.bench {
        println!("Benchmark mode: enabled");
    }

    println!();

    let peers = Arc::new(Mutex::new(HashMap::<String, Peer>::new()));
    let connecting = Arc::new(Mutex::new(HashSet::<String>::new()));
    let connections = Arc::new(Mutex::new(HashMap::<String, ConnectedPeer>::new()));
    let events = Arc::new(Mutex::new(EventStore::default()));
    let addons = Arc::new(Mutex::new(Vec::<AddonRecord>::from(loaded_addons)));
    let spaces = Arc::new(Mutex::new(loaded_spaces));
    let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(4);

    let cfg_server = cfg.clone();
    let opts_server = opts.clone();
    let connections_server = connections.clone();
    let events_server = events.clone();

    tokio::spawn(async move {
        if let Err(err) =
            tcp_server(cfg_server, opts_server, connections_server, events_server).await
        {
            eprintln!("TCP server error: {err}");
        }
    });

    start_addon_process_manager(addons.clone(), connections.clone());

    let cfg_api = cfg.clone();
    let peers_api = peers.clone();
    let connections_api = connections.clone();
    let events_api = events.clone();
    let addons_api = addons.clone();
    let spaces_api = spaces.clone();
    let connecting_api = connecting.clone();
    let opts_api = opts.clone();
    let shutdown_tx_api = shutdown_tx.clone();

    tokio::spawn(async move {
        if let Err(err) = api::local_api_server(
            cfg_api.clone(),
            peers_api,
            connections_api,
            events_api,
            addons_api,
            spaces_api,
            connecting_api,
            opts_api,
            cfg_api,
            shutdown_tx_api,
            started_at,
        )
        .await
        {
            eprintln!("Local API error: {err}");
        }
    });

    tokio::select! {
        result = discovery::discovery_loop(cfg, opts, peers, connecting, connections, events) => result,
        _ = shutdown_rx.recv() => {
            println!("LocalLink Core shutdown requested");
            Ok(())
        }
    }
}

fn start_addon_process_manager(
    addons: Arc<Mutex<Vec<AddonRecord>>>,
    connections: ConnectionRegistry,
) {
    tokio::spawn(async move {
        let mut children = HashMap::<String, Child>::new();
        let mut suppressed_until_next_connection = HashSet::<String>::new();
        let mut had_connections = false;

        loop {
            let has_connections = !connections.lock().await.is_empty();

            if !has_connections {
                if had_connections || !children.is_empty() {
                    stop_all_addon_children(&mut children);
                    eprintln!("Stopped add-ons because there are no active connections");
                }

                suppressed_until_next_connection.clear();
                had_connections = false;
                sleep(Duration::from_millis(250)).await;
                continue;
            }

            if !had_connections {
                suppressed_until_next_connection.clear();
            }
            had_connections = true;

            let addon_snapshot = addons.lock().await.clone();

            let wanted: HashMap<String, AddonRecord> = addon_snapshot
                .into_iter()
                .filter(|addon| addon.enabled)
                .map(|addon| (addon.id.clone(), addon))
                .collect();

            suppressed_until_next_connection.retain(|id| wanted.contains_key(id));

            let running_ids: Vec<String> = children.keys().cloned().collect();

            for id in running_ids {
                let exited = children
                    .get_mut(&id)
                    .and_then(|child| child.try_wait().ok())
                    .is_some();

                if exited {
                    children.remove(&id);
                    suppressed_until_next_connection.insert(id.clone());
                    eprintln!(
                        "Add-on process exited: {id}. It will not be restarted until the next connection cycle."
                    );
                    continue;
                }

                if !wanted.contains_key(&id) {
                    if let Some(mut child) = children.remove(&id) {
                        let _ = child.kill();
                        let _ = child.wait();
                        eprintln!("Stopped add-on: {id}");
                    }
                }
            }

            for (id, addon) in wanted {
                if children.contains_key(&id) || suppressed_until_next_connection.contains(&id) {
                    continue;
                }

                match launch_core_owned_addon(&addon) {
                    Ok(child) => {
                        eprintln!("Started add-on: {}", addon.name);
                        children.insert(id, child);
                    }
                    Err(err) => {
                        suppressed_until_next_connection.insert(id);
                        eprintln!(
                            "Could not start add-on {}: {err}. It will not be retried until the next connection cycle.",
                            addon.name
                        );
                    }
                }
            }

            sleep(Duration::from_millis(250)).await;
        }
    });
}

fn stop_all_addon_children(children: &mut HashMap<String, Child>) {
    for (id, mut child) in children.drain() {
        let _ = child.kill();
        let _ = child.wait();
        eprintln!("Stopped add-on: {id}");
    }
}

fn launch_core_owned_addon(addon: &AddonRecord) -> Result<Child> {
    let exe_path = Path::new(&addon.addon_dir).join(&addon.executable);

    if !exe_path.exists() {
        anyhow::bail!("add-on executable not found: {}", exe_path.display());
    }

    let mut command = Command::new(&exe_path);
    command
        .current_dir(Path::new(&addon.addon_dir))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW

    Ok(command.spawn()?)
}
