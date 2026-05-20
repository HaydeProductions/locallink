mod addons;
mod api;
mod api_client;
mod config;
mod discovery;
mod protocol;
mod transport;

use addons::{load_addon_manifests, AddonRecord};
use anyhow::Result;
use config::{
    acquire_single_instance_lock, config_path, generate_psk_b64, init_app_dirs,
    load_or_create_config, save_config, validate_psk_b64,
};
use discovery::Peer;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::Instant;
use transport::{tcp_server, ApiEvent, ConnectedPeer, RunOptions};

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

    println!("LocalLink core prototype");
    println!("Version:     {}", env!("CARGO_PKG_VERSION"));
    println!("Device name: {}", cfg.device_name);
    println!("Device ID:   {}", cfg.device_id);
    println!("Config:      {}", config_path()?.display());
    println!("Addons:      {}", loaded_addons.len());

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
    let events = Arc::new(Mutex::new(VecDeque::<ApiEvent>::new()));
    let addons = Arc::new(Mutex::new(Vec::<AddonRecord>::from(loaded_addons)));

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

    let cfg_api = cfg.clone();
    let peers_api = peers.clone();
    let connections_api = connections.clone();
    let events_api = events.clone();
    let addons_api = addons.clone();
    let connecting_api = connecting.clone();
    let opts_api = opts.clone();

    tokio::spawn(async move {
        if let Err(err) = api::local_api_server(
            cfg_api.clone(),
            peers_api,
            connections_api,
            events_api,
            addons_api,
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

    discovery::discovery_loop(cfg, opts, peers, connecting, connections, events).await
}
