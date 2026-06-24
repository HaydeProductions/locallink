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
use spaces::{load_or_create_space_store, SpaceKind, SpaceStore};
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

    start_space_addon_process_manager(addons.clone(), spaces.clone(), connections.clone());

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

#[derive(Clone)]
struct WantedSpaceAddon {
    key: String,
    addon: AddonRecord,
    space_id: String,
    space_kind: SpaceKind,
    space_name: String,
}

fn start_space_addon_process_manager(
    addons: Arc<Mutex<Vec<AddonRecord>>>,
    spaces: Arc<Mutex<SpaceStore>>,
    connections: ConnectionRegistry,
) {
    tokio::spawn(async move {
        let mut children = HashMap::<String, Child>::new();
        let mut suppressed_until_next_activation = HashSet::<String>::new();

        loop {
            let addon_snapshot: HashMap<String, AddonRecord> = addons
                .lock()
                .await
                .iter()
                .cloned()
                .map(|addon| (addon.id.clone(), addon))
                .collect();
            let space_snapshot = spaces.lock().await.clone();
            let connected_peer_ids: HashSet<String> = connections.lock().await.keys().cloned().collect();
            let mut wanted = HashMap::<String, WantedSpaceAddon>::new();

            for space in space_snapshot.spaces {
                let active = space
                    .members
                    .iter()
                    .any(|member| connected_peer_ids.contains(member));

                if !active {
                    continue;
                }

                for (addon_id, state) in &space.addons {
                    if !state.enabled {
                        continue;
                    }

                    let Some(addon) = addon_snapshot.get(addon_id).cloned() else {
                        eprintln!(
                            "Space {} wants missing add-on {}; skipping",
                            space.space_id, addon_id
                        );
                        continue;
                    };

                    let key = format!("{}:{}", space.space_id, addon.id);
                    wanted.insert(
                        key.clone(),
                        WantedSpaceAddon {
                            key,
                            addon,
                            space_id: space.space_id.clone(),
                            space_kind: space.kind.clone(),
                            space_name: space.name.clone(),
                        },
                    );
                }
            }

            suppressed_until_next_activation.retain(|key| wanted.contains_key(key));

            let running_keys: Vec<String> = children.keys().cloned().collect();
            for key in running_keys {
                let exited = children
                    .get_mut(&key)
                    .and_then(|child| child.try_wait().ok())
                    .is_some();

                if exited {
                    children.remove(&key);
                    suppressed_until_next_activation.insert(key.clone());
                    eprintln!(
                        "Space add-on process exited: {key}. It will not restart until the space deactivates and reactivates."
                    );
                    continue;
                }

                if !wanted.contains_key(&key) {
                    if let Some(mut child) = children.remove(&key) {
                        let _ = child.kill();
                        let _ = child.wait();
                        eprintln!("Stopped space add-on: {key}");
                    }
                }
            }

            for (key, wanted_addon) in wanted {
                if children.contains_key(&key) || suppressed_until_next_activation.contains(&key) {
                    continue;
                }

                match launch_space_owned_addon(&wanted_addon) {
                    Ok(child) => {
                        eprintln!(
                            "Started add-on {} for space {}",
                            wanted_addon.addon.name, wanted_addon.space_name
                        );
                        children.insert(key, child);
                    }
                    Err(err) => {
                        suppressed_until_next_activation.insert(key.clone());
                        eprintln!(
                            "Could not start add-on {} for space {}: {err}. It will not be retried until the space deactivates and reactivates.",
                            wanted_addon.addon.name, wanted_addon.space_name
                        );
                    }
                }
            }

            sleep(Duration::from_millis(250)).await;
        }
    });
}

fn launch_space_owned_addon(wanted: &WantedSpaceAddon) -> Result<Child> {
    let exe_path = Path::new(&wanted.addon.addon_dir).join(&wanted.addon.executable);

    if !exe_path.exists() {
        anyhow::bail!("add-on executable not found: {}", exe_path.display());
    }

    let space_kind = match &wanted.space_kind {
        SpaceKind::Direct => "direct",
        SpaceKind::Group => "group",
    };

    let mut command = Command::new(&exe_path);
    command
        .current_dir(Path::new(&wanted.addon.addon_dir))
        .env("LOCALLINK_SPACE_ID", &wanted.space_id)
        .env("LOCALLINK_SPACE_KIND", space_kind)
        .env("LOCALLINK_SPACE_NAME", &wanted.space_name)
        .env("LOCALLINK_CORE_API_ADDR", api::LOCAL_API_ADDR)
        .env("LOCALLINK_ADDON_ID", &wanted.addon.id)
        .env("LOCALLINK_ADDON_INSTANCE_ID", &wanted.key)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW

    Ok(command.spawn()?)
}
