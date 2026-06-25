#!/usr/bin/env bash
set -euo pipefail

# LocalLink: send/accept space invites through core-level discovery transport.
# This removes the direct-session requirement from the invite flow.
#
# Run from the repo root:
#   bash phase9_core_invite_transport_patch.sh

if [ ! -d .git ] || [ ! -f Cargo.toml ]; then
  echo "Run this from the LocalLink repo root." >&2
  exit 1
fi

if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "Working tree has uncommitted changes. Commit/stash them first." >&2
  exit 1
fi

git fetch origin
git switch phase9-space-system 2>/dev/null || git switch -c phase9-space-system origin/phase9-space-system

cat > locallink-core/src/discovery.rs <<'RS'
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
RS

cat > locallink-core/src/space_sync_live.rs <<'RS'
use crate::config::space_membership::{
    load_or_create_space_membership_store, save_space_membership_store, ImportedSpaceInvite,
    SpaceMembershipStore, SpaceSyncUpdate, SPACE_SYNC_SERVICE,
};
use crate::config::spaces::{save_space_store, SpaceRecord, SpaceRegistry, SpaceStore};
use crate::config::{load_or_create_config, Config};
use crate::discovery::send_core_space_service_message;
use crate::transport::{take_events, ApiEvent, ConnectionRegistry, EventQueue};
use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SpaceSyncMessage {
    Invite { invite: ImportedSpaceInvite },
    InviteAccept { space_id: String, peer_id: String },
    InviteDecline { space_id: String, peer_id: String },
    Leave { space_id: String, peer_id: String },
    Update { update: SpaceSyncUpdate },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SyncDeliveryResult {
    pub peer_id: String,
    pub ok: bool,
    pub message_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct SpaceSyncApplyReport {
    pub applied: usize,
    pub ignored: usize,
    pub errors: Vec<String>,
}

pub fn encode_sync_message(message: &SpaceSyncMessage) -> Result<String> {
    Ok(STANDARD.encode(serde_json::to_vec(message)?))
}

pub fn decode_sync_message(data_b64: &str) -> Result<SpaceSyncMessage> {
    let bytes = STANDARD.decode(data_b64)?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub async fn send_sync_message(
    _connections: ConnectionRegistry,
    peer_id: &str,
    space_id: &str,
    message: SpaceSyncMessage,
) -> SyncDeliveryResult {
    let data_b64 = match encode_sync_message(&message) {
        Ok(data_b64) => data_b64,
        Err(err) => {
            return SyncDeliveryResult {
                peer_id: peer_id.to_string(),
                ok: false,
                message_id: None,
                error: Some(err.to_string()),
            };
        }
    };

    let cfg = match load_or_create_config() {
        Ok(cfg) => cfg,
        Err(err) => {
            return SyncDeliveryResult {
                peer_id: peer_id.to_string(),
                ok: false,
                message_id: None,
                error: Some(err.to_string()),
            };
        }
    };

    match send_core_space_service_message(&cfg, peer_id, space_id, SPACE_SYNC_SERVICE, &data_b64)
        .await
    {
        Ok(message_id) => SyncDeliveryResult {
            peer_id: peer_id.to_string(),
            ok: true,
            message_id: Some(message_id),
            error: None,
        },
        Err(err) => SyncDeliveryResult {
            peer_id: peer_id.to_string(),
            ok: false,
            message_id: None,
            error: Some(err.to_string()),
        },
    }
}

pub async fn send_invite(
    connections: ConnectionRegistry,
    peer_id: &str,
    invite: ImportedSpaceInvite,
) -> SyncDeliveryResult {
    let space_id = invite.space_id.clone();
    send_sync_message(
        connections,
        peer_id,
        &space_id,
        SpaceSyncMessage::Invite { invite },
    )
    .await
}

pub async fn send_accept(
    connections: ConnectionRegistry,
    owner_device_id: &str,
    space_id: &str,
    local_device_id: &str,
) -> SyncDeliveryResult {
    send_sync_message(
        connections,
        owner_device_id,
        space_id,
        SpaceSyncMessage::InviteAccept {
            space_id: space_id.to_string(),
            peer_id: local_device_id.to_string(),
        },
    )
    .await
}

pub async fn send_leave(
    connections: ConnectionRegistry,
    owner_device_id: &str,
    space_id: &str,
    local_device_id: &str,
) -> SyncDeliveryResult {
    send_sync_message(
        connections,
        owner_device_id,
        space_id,
        SpaceSyncMessage::Leave {
            space_id: space_id.to_string(),
            peer_id: local_device_id.to_string(),
        },
    )
    .await
}

pub async fn broadcast_update(
    connections: ConnectionRegistry,
    update: SpaceSyncUpdate,
    exclude: Option<&str>,
) -> Vec<SyncDeliveryResult> {
    let mut results = Vec::new();
    let mut seen = HashSet::<String>::new();

    for peer_id in &update.members {
        if peer_id == &update.owner_device_id {
            continue;
        }
        if exclude.map(|excluded| excluded == peer_id).unwrap_or(false) {
            continue;
        }
        if !seen.insert(peer_id.clone()) {
            continue;
        }

        results.push(
            send_sync_message(
                connections.clone(),
                peer_id,
                &update.space_id,
                SpaceSyncMessage::Update {
                    update: update.clone(),
                },
            )
            .await,
        );
    }

    results
}

pub fn imported_invite_from_space(
    space: &SpaceRecord,
    owner_device_id: &str,
    invite_id: String,
    revision: u64,
    owner_enabled: bool,
    key_epoch: u64,
) -> ImportedSpaceInvite {
    ImportedSpaceInvite {
        space_id: space.space_id.clone(),
        name: space.name.clone(),
        kind: space.kind.clone(),
        owner_device_id: owner_device_id.to_string(),
        invite_id,
        revision,
        owner_enabled,
        members: space.members.clone(),
        key_epoch,
    }
}

pub async fn apply_pending_space_sync_events(
    local_device_id: &str,
    spaces: &mut SpaceStore,
    membership: &mut SpaceMembershipStore,
    events: EventQueue,
    connections: ConnectionRegistry,
) -> SpaceSyncApplyReport {
    let incoming = take_events(
        events,
        "__locallink_space_sync__",
        Some(SPACE_SYNC_SERVICE),
        100,
    )
    .await;
    let mut report = SpaceSyncApplyReport::default();

    for event in incoming {
        match apply_event(local_device_id, spaces, membership, connections.clone(), event).await {
            Ok(true) => report.applied += 1,
            Ok(false) => report.ignored += 1,
            Err(err) => report.errors.push(err.to_string()),
        }
    }

    if report.applied > 0 {
        if let Err(err) = save_space_store(spaces) {
            report.errors.push(err.to_string());
        }
        if let Err(err) = save_space_membership_store(membership) {
            report.errors.push(err.to_string());
        }
    }

    report
}

pub async fn apply_core_space_sync_message(
    cfg: &Config,
    spaces: SpaceRegistry,
    peer_id: &str,
    _peer_name: &str,
    service: &str,
    data_b64: &str,
) -> Result<bool> {
    if service != SPACE_SYNC_SERVICE {
        return Ok(false);
    }

    let message = decode_sync_message(data_b64)?;
    let mut store = spaces.lock().await;
    let mut membership = load_or_create_space_membership_store()?;

    let applied =
        apply_message(&cfg.device_id, &mut store, &mut membership, peer_id, message).await?;

    if applied {
        save_space_store(&store)?;
        save_space_membership_store(&membership)?;
    }

    Ok(applied)
}

async fn apply_event(
    local_device_id: &str,
    spaces: &mut SpaceStore,
    membership: &mut SpaceMembershipStore,
    _connections: ConnectionRegistry,
    event: ApiEvent,
) -> Result<bool> {
    if event.kind != "space_service_data" || event.service != SPACE_SYNC_SERVICE {
        return Ok(false);
    }

    let Some(data_b64) = event.data_b64.as_deref() else {
        return Ok(false);
    };

    let message = decode_sync_message(data_b64)?;
    apply_message(local_device_id, spaces, membership, &event.peer_id, message).await
}

async fn apply_message(
    local_device_id: &str,
    spaces: &mut SpaceStore,
    membership: &mut SpaceMembershipStore,
    sender_peer_id: &str,
    message: SpaceSyncMessage,
) -> Result<bool> {
    match message {
        SpaceSyncMessage::Invite { invite } => {
            if invite.owner_device_id != sender_peer_id {
                anyhow::bail!("space invite owner did not match sender");
            }
            if spaces
                .spaces
                .iter()
                .any(|space| space.space_id == invite.space_id)
            {
                return Ok(false);
            }
            membership.import_invite(spaces, local_device_id, invite)?;
            membership.validate_and_repair(spaces)?;
            Ok(true)
        }
        SpaceSyncMessage::InviteAccept { space_id, peer_id } => {
            if peer_id != sender_peer_id {
                anyhow::bail!("space invite acceptance sender did not match peer_id");
            }
            let update = membership.record_member_acceptance(
                spaces,
                local_device_id,
                &space_id,
                &peer_id,
            )?;
            membership.validate_and_repair(spaces)?;

            if let Ok(cfg) = load_or_create_config() {
                let _ = broadcast_update_from_config(&cfg, update, None).await;
            }

            Ok(true)
        }
        SpaceSyncMessage::InviteDecline { .. } => Ok(false),
        SpaceSyncMessage::Leave { space_id, peer_id } => {
            if peer_id != sender_peer_id {
                anyhow::bail!("space leave sender did not match peer_id");
            }
            let update = remove_member_after_leave(
                membership,
                spaces,
                local_device_id,
                &space_id,
                &peer_id,
            )?;
            membership.validate_and_repair(spaces)?;
            if let Some(update) = update {
                if let Ok(cfg) = load_or_create_config() {
                    let _ = broadcast_update_from_config(&cfg, update, Some(&peer_id)).await;
                }
                Ok(true)
            } else {
                Ok(false)
            }
        }
        SpaceSyncMessage::Update { update } => {
            if update.owner_device_id != sender_peer_id {
                anyhow::bail!("space update owner did not match sender");
            }
            let applied = membership.apply_owner_update(spaces, update)?.is_some();
            membership.validate_and_repair(spaces)?;
            Ok(applied)
        }
    }
}

async fn broadcast_update_from_config(
    cfg: &Config,
    update: SpaceSyncUpdate,
    exclude: Option<&str>,
) -> Vec<SyncDeliveryResult> {
    let mut results = Vec::new();
    let mut seen = HashSet::<String>::new();

    for peer_id in &update.members {
        if peer_id == &update.owner_device_id {
            continue;
        }
        if exclude.map(|excluded| excluded == peer_id).unwrap_or(false) {
            continue;
        }
        if !seen.insert(peer_id.clone()) {
            continue;
        }

        let data_b64 = match encode_sync_message(&SpaceSyncMessage::Update {
            update: update.clone(),
        }) {
            Ok(data_b64) => data_b64,
            Err(err) => {
                results.push(SyncDeliveryResult {
                    peer_id: peer_id.clone(),
                    ok: false,
                    message_id: None,
                    error: Some(err.to_string()),
                });
                continue;
            }
        };

        match send_core_space_service_message(
            cfg,
            peer_id,
            &update.space_id,
            SPACE_SYNC_SERVICE,
            &data_b64,
        )
        .await
        {
            Ok(message_id) => results.push(SyncDeliveryResult {
                peer_id: peer_id.clone(),
                ok: true,
                message_id: Some(message_id),
                error: None,
            }),
            Err(err) => results.push(SyncDeliveryResult {
                peer_id: peer_id.clone(),
                ok: false,
                message_id: None,
                error: Some(err.to_string()),
            }),
        }
    }

    results
}

fn remove_member_after_leave(
    membership: &mut SpaceMembershipStore,
    spaces: &mut SpaceStore,
    local_device_id: &str,
    space_id: &str,
    peer_id: &str,
) -> Result<Option<SpaceSyncUpdate>> {
    let Some(record) = membership.records.get(space_id) else {
        return Ok(None);
    };
    if !record.is_owner_for(local_device_id) {
        return Ok(None);
    }

    if let Some(space) = spaces
        .spaces
        .iter_mut()
        .find(|space| space.space_id == space_id)
    {
        let before = space.members.len();
        space.members.retain(|member| member != peer_id);
        if space.members.len() == before {
            return Ok(None);
        }
    }

    if let Some(record) = membership.records.get_mut(space_id) {
        record.revision = record.revision.saturating_add(1).max(1);
    }

    membership
        .sync_update(spaces, local_device_id, space_id)
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::spaces::{SpaceKind, SpaceRecord};

    #[test]
    fn sync_messages_round_trip_through_base64() {
        let message = SpaceSyncMessage::InviteAccept {
            space_id: "office".to_string(),
            peer_id: "laptop".to_string(),
        };

        let encoded = encode_sync_message(&message).unwrap();
        let decoded = decode_sync_message(&encoded).unwrap();

        assert_eq!(decoded, message);
    }

    #[test]
    fn imported_invite_snapshot_uses_space_metadata() {
        let space = SpaceRecord {
            space_id: "office".to_string(),
            name: "Office".to_string(),
            kind: SpaceKind::Group,
            active: false,
            members: vec!["owner".to_string()],
            addons: Default::default(),
        };

        let invite =
            imported_invite_from_space(&space, "owner", "invite-1".to_string(), 2, true, 1);

        assert_eq!(invite.space_id, "office");
        assert_eq!(invite.owner_device_id, "owner");
        assert_eq!(invite.members, vec!["owner".to_string()]);
    }
}
RS

python - <<'PY'
from pathlib import Path
path = Path("locallink-core/src/main.rs")
text = path.read_text()
old = '''    discovery::discovery_loop(
        cfg,
        opts,
        runtime_state.peers.clone(),
        runtime_state.connecting.clone(),
        runtime_state.connections.clone(),
        runtime_state.events.clone(),
    )
    .await
}'''
new = '''    discovery::discovery_loop(
        cfg,
        opts,
        runtime_state.peers.clone(),
        runtime_state.connecting.clone(),
        runtime_state.connections.clone(),
        runtime_state.events.clone(),
        runtime_state.spaces.clone(),
    )
    .await
}'''
if old not in text:
    raise SystemExit("Could not find discovery_loop call in main.rs")
path.write_text(text.replace(old, new))
PY

cargo check -p locallink-core

git add locallink-core/src/discovery.rs locallink-core/src/space_sync_live.rs locallink-core/src/main.rs
git commit -m "Send space invites through core discovery transport"
git push -u origin phase9-space-system

echo
echo "Done. Open/refresh the PR from phase9-space-system into redesign/integration."
