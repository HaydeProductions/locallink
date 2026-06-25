#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_assignments)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::unnecessary_sort_by)]
#![allow(clippy::suspicious_open_options)]

mod addons;
mod api;
mod api_client;
mod config;
mod discovery;
mod protocol;
mod transport;

use anyhow::Result;
use config::core_state::{load_core_runtime_state, CoreRuntimeState};
use config::space_runtime::SpaceAddonRuntimeContext;
use config::{
    acquire_single_instance_lock, config_path, generate_psk_b64, init_app_dirs,
    load_or_create_config, save_config, validate_psk_b64,
};
use std::collections::{HashMap, HashSet};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use tokio::time::{sleep, Duration, Instant};
use transport::{tcp_server, RunOptions};

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
    let loaded_addons = addons::load_addon_manifests()?;
    let loaded_addon_count = loaded_addons.len();
    let runtime_state = load_core_runtime_state(loaded_addons)?;
    let loaded_space_count = runtime_state.spaces.lock().await.spaces.len();

    println!("LocalLink core prototype");
    println!("Version:     {}", env!("CARGO_PKG_VERSION"));
    println!("Device name: {}", cfg.device_name);
    println!("Device ID:   {}", cfg.device_id);
    println!("Config:      {}", config_path()?.display());
    println!("Addons:      {}", loaded_addon_count);
    println!("Spaces:      {}", loaded_space_count);

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

    let cfg_server = cfg.clone();
    let opts_server = opts.clone();
    let connections_server = runtime_state.connections.clone();
    let events_server = runtime_state.events.clone();

    tokio::spawn(async move {
        if let Err(err) =
            tcp_server(cfg_server, opts_server, connections_server, events_server).await
        {
            eprintln!("TCP server error: {err}");
        }
    });

    start_addon_process_manager(runtime_state.clone());

    let cfg_api = cfg.clone();
    let peers_api = runtime_state.peers.clone();
    let connections_api = runtime_state.connections.clone();
    let events_api = runtime_state.events.clone();
    let addons_api = runtime_state.addons.clone();
    let spaces_api = runtime_state.spaces.clone();
    let connecting_api = runtime_state.connecting.clone();
    let opts_api = opts.clone();

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
            started_at,
        )
        .await
        {
            eprintln!("Local API error: {err}");
        }
    });

    discovery::discovery_loop(
        cfg,
        opts,
        runtime_state.peers.clone(),
        runtime_state.connecting.clone(),
        runtime_state.connections.clone(),
        runtime_state.events.clone(),
    )
    .await
}

fn start_addon_process_manager(state: CoreRuntimeState) {
    tokio::spawn(async move {
        let mut children = HashMap::<String, Child>::new();
        let mut suppressed_until_space_change = HashSet::<String>::new();

        loop {
            let action_plan =
                config::space_sync::plan_space_addon_actions_from_core_state(
                    &state,
                    api::LOCAL_API_ADDR,
                )
                .await;

            let wanted: HashSet<String> = action_plan
                .start
                .iter()
                .map(|context| context.instance_id.clone())
                .chain(action_plan.keep.iter().cloned())
                .collect();

            suppressed_until_space_change.retain(|id| wanted.contains(id));

            for id in action_plan.stop {
                suppressed_until_space_change.remove(&id);

                if let Some(mut child) = children.remove(&id) {
                    let _ = child.kill();
                    let _ = child.wait();
                    eprintln!("Stopped add-on: {id}");
                }

                state
                    .space_addon_instances
                    .lock()
                    .await
                    .mark_absent(&id);
            }

            let running_ids: Vec<String> = children.keys().cloned().collect();

            for id in running_ids {
                let exited = match children.get_mut(&id).map(|child| child.try_wait()) {
                    Some(Ok(Some(_))) => true,
                    Some(Ok(None)) => false,
                    Some(Err(_)) => true,
                    None => false,
                };

                if exited {
                    children.remove(&id);
                    suppressed_until_space_change.insert(id.clone());
                    state
                        .space_addon_instances
                        .lock()
                        .await
                        .mark_absent(&id);

                    eprintln!(
                        "Add-on process exited: {id}. It will not be restarted until its space state changes."
                    );
                    continue;
                }

                if !wanted.contains(&id) {
                    if let Some(mut child) = children.remove(&id) {
                        let _ = child.kill();
                        let _ = child.wait();

                        state
                            .space_addon_instances
                            .lock()
                            .await
                            .mark_absent(&id);

                        eprintln!("Stopped add-on: {id}");
                    }
                }
            }

            for id in action_plan.keep {
                if children.contains_key(&id) {
                    state
                        .space_addon_instances
                        .lock()
                        .await
                        .mark_present(id);
                } else {
                    state
                        .space_addon_instances
                        .lock()
                        .await
                        .mark_absent(&id);
                }
            }

            for context in action_plan.start {
                let id = context.instance_id.clone();

                if children.contains_key(&id)
                    || suppressed_until_space_change.contains(&id)
                {
                    continue;
                }

                match launch_core_owned_addon(&context) {
                    Ok(child) => {
                        eprintln!("Started add-on: {id}");

                        state
                            .space_addon_instances
                            .lock()
                            .await
                            .mark_present(id.clone());

                        children.insert(id, child);
                    }
                    Err(err) => {
                        state
                            .space_addon_instances
                            .lock()
                            .await
                            .mark_absent(&id);

                        suppressed_until_space_change.insert(id.clone());

                        eprintln!(
                            "Could not start add-on {id}: {err}. It will not be retried until its space state changes."
                        );
                    }
                }
            }

            sleep(Duration::from_millis(250)).await;
        }
    });
}

fn launch_core_owned_addon(context: &SpaceAddonRuntimeContext) -> Result<Child> {
    let exe_path = Path::new(&context.executable);

    if !exe_path.exists() {
        anyhow::bail!("add-on executable not found: {}", exe_path.display());
    }

    let mut command = Command::new(exe_path);

    if let Some(addon_dir) = exe_path.parent() {
        command.current_dir(addon_dir);
    }

    command
        .envs(&context.env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW

    Ok(command.spawn()?)
}