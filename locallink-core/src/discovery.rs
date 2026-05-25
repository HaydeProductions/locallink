use crate::config::Config;
use crate::protocol::{
    DiscoveryMessage, APP_NAME, DISCOVERY_PORT, MULTICAST_ADDR, PROTOCOL_VERSION, TCP_PORT,
};
use crate::transport::{ConnectionRegistry, EventQueue, RunOptions};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, Instant};

#[derive(Debug, Clone)]
pub struct Peer {
    pub device_id: String,
    pub device_name: String,
    pub addr: SocketAddrV6,
    pub macs: Vec<String>,
    pub last_seen: Instant,
}

pub async fn discovery_loop(
    cfg: Config,
    _opts: RunOptions,
    peers: Arc<Mutex<HashMap<String, Peer>>>,
    _connecting: Arc<Mutex<HashSet<String>>>,
    _connections: ConnectionRegistry,
    _events: EventQueue,
) -> Result<()> {
    let socket = UdpSocket::bind(SocketAddrV6::new(
        Ipv6Addr::UNSPECIFIED,
        DISCOVERY_PORT,
        0,
        0,
    ))
    .await?;

    let multicast: Ipv6Addr = MULTICAST_ADDR.parse()?;
    let interface_indices = multicast_interface_indices();
    let mut joined_any = false;

    for interface_id in &interface_indices {
        match socket.join_multicast_v6(&multicast, *interface_id) {
            Ok(()) => {
                joined_any = true;
                println!("Discovery joined [{MULTICAST_ADDR}] on interface {interface_id}");
            }
            Err(err) => {
                eprintln!(
                    "Discovery could not join [{MULTICAST_ADDR}] on interface {interface_id}: {err}"
                );
            }
        }
    }

    if !joined_any {
        socket.join_multicast_v6(&multicast, 0)?;
        println!("Discovery joined [{MULTICAST_ADDR}] on default interface");
    }

    let send_socket = UdpSocket::bind(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0)).await?;

    let macs = local_macs();

    let announce = DiscoveryMessage {
        app: APP_NAME.to_string(),
        version: PROTOCOL_VERSION,
        kind: "announce".to_string(),
        device_id: cfg.device_id.clone(),
        device_name: cfg.device_name.clone(),
        tcp_port: TCP_PORT,
        macs,
    };

    let encoded = serde_json::to_vec(&announce)?;

    println!("Discovery started on UDP [{MULTICAST_ADDR}]:{DISCOVERY_PORT}");
    println!("Discovery interfaces: {:?}", interface_indices);
    println!("Connection mode: manual only");
    println!("Local MAC hints: {}", announce.macs.join(", "));

    start_discovery_receiver(socket, cfg.clone(), peers);

    loop {
        for interface_id in &interface_indices {
            let target = SocketAddrV6::new(multicast, DISCOVERY_PORT, 0, *interface_id);

            if let Err(err) = send_socket.send_to(&encoded, target).await {
                eprintln!("Discovery send error on interface {interface_id}: {err}");
            }
        }

        sleep(Duration::from_secs(2)).await;
    }
}

fn start_discovery_receiver(
    socket: UdpSocket,
    cfg: Config,
    peers: Arc<Mutex<HashMap<String, Peer>>>,
) {
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];

        loop {
            let Ok((len, src)) = socket.recv_from(&mut buf).await else {
                continue;
            };

            let Ok(msg) = serde_json::from_slice::<DiscoveryMessage>(&buf[..len]) else {
                continue;
            };

            if msg.app != APP_NAME || msg.device_id == cfg.device_id {
                continue;
            }

            let SocketAddr::V6(src_v6) = src else {
                continue;
            };

            let peer_addr = SocketAddrV6::new(*src_v6.ip(), msg.tcp_port, 0, src_v6.scope_id());

            let mut peers_guard = peers.lock().await;
            let is_new = !peers_guard.contains_key(&msg.device_id);

            peers_guard.insert(
                msg.device_id.clone(),
                Peer {
                    device_id: msg.device_id.clone(),
                    device_name: msg.device_name.clone(),
                    addr: peer_addr,
                    macs: msg.macs.clone(),
                    last_seen: Instant::now(),
                },
            );
            drop(peers_guard);

            if is_new {
                println!(
                    "Discovered nearby device: {} | {} | {} | macs={}",
                    msg.device_name,
                    msg.device_id,
                    peer_addr,
                    msg.macs.join(", ")
                );
            }
        }
    });
}

fn multicast_interface_indices() -> Vec<u32> {
    let mut indices = Vec::new();

    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                "Get-NetAdapter | Where-Object { $_.Status -eq 'Up' } | Select-Object -ExpandProperty ifIndex",
            ])
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);

            for line in text.lines() {
                if let Ok(index) = line.trim().parse::<u32>() {
                    if index != 0 && !indices.contains(&index) {
                        indices.push(index);
                    }
                }
            }
        }
    }

    if indices.is_empty() {
        indices.push(0);
    }

    indices
}

fn local_macs() -> Vec<String> {
    let mut macs = Vec::new();

    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = std::process::Command::new("getmac")
            .args(["/fo", "csv", "/nh"])
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);

            for line in text.lines() {
                // getmac CSV output usually starts with:
                // "AA-BB-CC-DD-EE-FF","\Device\Tcpip_..."
                let first = line
                    .split(',')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"');

                let normalized = normalize_mac_local(first);

                if !normalized.is_empty() && !macs.contains(&normalized) {
                    macs.push(normalized);
                }
            }
        }
    }

    macs
}

fn normalize_mac_local(mac: &str) -> String {
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
