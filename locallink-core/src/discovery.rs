use crate::config::space_sync_live::apply_core_space_sync_message;
use crate::config::spaces::SpaceRegistry;
use crate::config::{psk_bytes, Config};
use crate::protocol::{
    DiscoveryMessage, APP_NAME, DISCOVERY_PORT, MULTICAST_ADDR, PROTOCOL_VERSION, TCP_PORT,
};
use crate::transport::{ConnectionRegistry, EventQueue, RunOptions};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::{HashMap, HashSet};
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, Instant};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

const PEER_TTL: Duration = Duration::from_secs(15);
const CORE_SPACE_SERVICE_KIND: &str = "core_space_service";
const CORE_SPACE_SERVICE_AUTH_DOMAIN: &[u8] = b"locallink-core-space-service-v1";

#[derive(Debug, Clone)]
pub struct Peer {
    pub device_id: String,
    pub device_name: String,
    pub addr: SocketAddrV6,
    pub macs: Vec<String>,
    pub last_seen: Instant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CoreSpaceServiceDiscoveryMessage {
    app: String,
    version: u32,
    kind: String,
    device_id: String,
    device_name: String,
    tcp_port: u16,

    #[serde(default)]
    macs: Vec<String>,

    target_device_id: String,
    message_id: String,
    service: String,
    space_id: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    target_peer_id: Option<String>,

    data_b64: String,
    auth_b64: String,
}

pub async fn discovery_loop(
    cfg: Config,
    _opts: RunOptions,
    peers: Arc<Mutex<HashMap<String, Peer>>>,
    _connecting: Arc<Mutex<HashSet<String>>>,
    _connections: ConnectionRegistry,
    _events: EventQueue,
    spaces: SpaceRegistry,
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
    println!("Discovery peer TTL: {}s", PEER_TTL.as_secs());
    println!("Connection mode: manual only");
    println!("Local MAC hints: {}", announce.macs.join(", "));

    start_discovery_receiver(socket, cfg.clone(), peers.clone(), spaces);
    start_peer_expiry(peers.clone());

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

pub async fn send_core_space_service_message(
    cfg: &Config,
    peer_id: &str,
    space_id: &str,
    service: &str,
    data_b64: &str,
) -> Result<String> {
    STANDARD
        .decode(data_b64)
        .context("data_b64 was not valid base64")?;

    let mut message = CoreSpaceServiceDiscoveryMessage {
        app: APP_NAME.to_string(),
        version: PROTOCOL_VERSION,
        kind: CORE_SPACE_SERVICE_KIND.to_string(),
        device_id: cfg.device_id.clone(),
        device_name: cfg.device_name.clone(),
        tcp_port: TCP_PORT,
        macs: local_macs(),
        target_device_id: peer_id.to_string(),
        message_id: Uuid::new_v4().to_string(),
        service: service.to_string(),
        space_id: space_id.to_string(),
        target_peer_id: Some(peer_id.to_string()),
        data_b64: data_b64.to_string(),
        auth_b64: String::new(),
    };

    message.auth_b64 = sign_core_space_service_message(cfg, &message)?;
    let encoded = serde_json::to_vec(&message)?;

    let socket = UdpSocket::bind(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0)).await?;
    let multicast: Ipv6Addr = MULTICAST_ADDR.parse()?;
    let interface_indices = multicast_interface_indices();
    let mut sent_any = false;
    let mut last_error = None;

    for interface_id in interface_indices {
        let target = SocketAddrV6::new(multicast, DISCOVERY_PORT, 0, interface_id);
        match socket.send_to(&encoded, target).await {
            Ok(_) => sent_any = true,
            Err(err) => last_error = Some(err),
        }
    }

    if !sent_any {
        if let Some(err) = last_error {
            anyhow::bail!("could not send core space message: {err}");
        }
        anyhow::bail!("could not send core space message: no multicast interfaces available");
    }

    Ok(message.message_id)
}

fn start_discovery_receiver(
    socket: UdpSocket,
    cfg: Config,
    peers: Arc<Mutex<HashMap<String, Peer>>>,
    spaces: SpaceRegistry,
) {
    tokio::spawn(async move {
        let mut buf = vec![0u8; 65_535];

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

            match msg.kind.as_str() {
                "announce" => handle_announce(&peers, msg, src).await,
                CORE_SPACE_SERVICE_KIND => {
                    let Ok(core_msg) =
                        serde_json::from_slice::<CoreSpaceServiceDiscoveryMessage>(&buf[..len])
                    else {
                        continue;
                    };

                    if let Err(err) = handle_core_space_service_message(
                        &cfg,
                        peers.clone(),
                        spaces.clone(),
                        core_msg,
                        src,
                    )
                    .await
                    {
                        eprintln!("Core space service discovery message ignored: {err}");
                    }
                }
                _ => {}
            }
        }
    });
}

async fn handle_announce(
    peers: &Arc<Mutex<HashMap<String, Peer>>>,
    msg: DiscoveryMessage,
    src: SocketAddr,
) {
    let SocketAddr::V6(src_v6) = src else {
        return;
    };

    upsert_peer(
        peers,
        msg.device_id,
        msg.device_name,
        SocketAddrV6::new(*src_v6.ip(), msg.tcp_port, 0, src_v6.scope_id()),
        msg.macs,
    )
    .await;
}

async fn handle_core_space_service_message(
    cfg: &Config,
    peers: Arc<Mutex<HashMap<String, Peer>>>,
    spaces: SpaceRegistry,
    msg: CoreSpaceServiceDiscoveryMessage,
    src: SocketAddr,
) -> Result<()> {
    if msg.target_device_id != cfg.device_id {
        return Ok(());
    }
    anyhow::ensure!(msg.app == APP_NAME, "core message app mismatch");
    anyhow::ensure!(msg.version == PROTOCOL_VERSION, "core message version mismatch");
    anyhow::ensure!(
        msg.kind == CORE_SPACE_SERVICE_KIND,
        "core message kind mismatch"
    );
    STANDARD
        .decode(&msg.data_b64)
        .context("core message data_b64 was not valid base64")?;
    verify_core_space_service_message(cfg, &msg)?;

    if let SocketAddr::V6(src_v6) = src {
        upsert_peer(
            &peers,
            msg.device_id.clone(),
            msg.device_name.clone(),
            SocketAddrV6::new(*src_v6.ip(), msg.tcp_port, 0, src_v6.scope_id()),
            msg.macs.clone(),
        )
        .await;
    }

    let applied = apply_core_space_sync_message(
        cfg,
        spaces,
        &msg.device_id,
        &msg.device_name,
        &msg.service,
        &msg.data_b64,
    )
    .await?;

    if applied {
        println!(
            "Applied core-level space message from {} service={} space={} message_id={}",
            msg.device_name, msg.service, msg.space_id, msg.message_id
        );
    }

    Ok(())
}

async fn upsert_peer(
    peers: &Arc<Mutex<HashMap<String, Peer>>>,
    device_id: String,
    device_name: String,
    addr: SocketAddrV6,
    macs: Vec<String>,
) {
    let mut peers_guard = peers.lock().await;
    let is_new = !peers_guard.contains_key(&device_id);

    peers_guard.insert(
        device_id.clone(),
        Peer {
            device_id: device_id.clone(),
            device_name: device_name.clone(),
            addr,
            macs: macs.clone(),
            last_seen: Instant::now(),
        },
    );
    drop(peers_guard);

    if is_new {
        println!(
            "Discovered nearby device: {} | {} | {} | macs={}",
            device_name,
            device_id,
            addr,
            macs.join(", ")
        );
    }
}

fn sign_core_space_service_message(
    cfg: &Config,
    message: &CoreSpaceServiceDiscoveryMessage,
) -> Result<String> {
    let psk = psk_bytes(cfg)?;
    let mut mac = <HmacSha256 as Mac>::new_from_slice(&psk)?;
    mac.update(CORE_SPACE_SERVICE_AUTH_DOMAIN);
    mac.update(core_space_service_auth_bytes(message).as_bytes());
    Ok(STANDARD.encode(mac.finalize().into_bytes()))
}

fn verify_core_space_service_message(
    cfg: &Config,
    message: &CoreSpaceServiceDiscoveryMessage,
) -> Result<()> {
    let psk = psk_bytes(cfg)?;
    let received = STANDARD
        .decode(&message.auth_b64)
        .context("core message auth_b64 was not valid base64")?;
    let mut mac = <HmacSha256 as Mac>::new_from_slice(&psk)?;
    mac.update(CORE_SPACE_SERVICE_AUTH_DOMAIN);
    mac.update(core_space_service_auth_bytes(message).as_bytes());
    let expected = mac.finalize().into_bytes().to_vec();
    anyhow::ensure!(received == expected, "core message authentication failed");
    Ok(())
}

fn core_space_service_auth_bytes(message: &CoreSpaceServiceDiscoveryMessage) -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        message.app,
        message.version,
        message.kind,
        message.device_id,
        message.target_device_id,
        message.message_id,
        message.space_id,
        message.service,
        message.target_peer_id.as_deref().unwrap_or(""),
        message.data_b64,
        message.tcp_port,
    )
}

fn start_peer_expiry(peers: Arc<Mutex<HashMap<String, Peer>>>) {
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(5)).await;

            let now = Instant::now();
            let mut peers_guard = peers.lock().await;
            let before = peers_guard.len();

            peers_guard.retain(|_, peer| now.duration_since(peer.last_seen) <= PEER_TTL);

            let expired = before.saturating_sub(peers_guard.len());
            drop(peers_guard);

            if expired > 0 {
                println!("Expired {expired} stale discovered peer(s)");
            }
        }
    });
}

fn multicast_interface_indices() -> Vec<u32> {
    if let Ok(value) = std::env::var("LOCALLINK_DISCOVERY_IFINDEX") {
        if let Ok(index) = value.trim().parse::<u32>() {
            if index != 0 {
                return vec![index];
            }
        }
    }

    let mut indices = Vec::new();

    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                "Get-NetAdapter -Physical | Where-Object { $_.Status -eq 'Up' -and $_.ifIndex -gt 0 } | Select-Object -ExpandProperty ifIndex",
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
