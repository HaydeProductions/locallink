use crate::addons::{load_addon_manifests, AddonRecord};
use crate::config::spaces::{save_space_store, SpaceKind, SpaceRecord, SpaceRegistry};
use crate::config::{
    add_trusted_device, app_paths, load_trusted_devices, mac_is_trusted, normalize_mac,
    remove_trusted_mac, trusted_name_for_macs, Config,
};
use crate::discovery::Peer;
use crate::transport::{
    close_channel, connect_to_peer, disconnect_peer, open_channel, send_channel_data,
    send_service_message, take_events, ApiEvent, ConnectionRegistry, EventQueue, RunOptions,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout, Duration, Instant};
use uuid::Uuid;

pub const LOCAL_API_ADDR: &str = "127.0.0.1:47900";

#[derive(Debug, Deserialize)]
struct ApiRequest {
    cmd: String,

    #[serde(default)]
    peer_id: Option<String>,

    #[serde(default)]
    target_peer_id: Option<String>,

    #[serde(default)]
    space_id: Option<String>,

    #[serde(default)]
    kind: Option<SpaceKind>,

    #[serde(default)]
    service: Option<String>,

    #[serde(default)]
    channel_id: Option<String>,

    #[serde(default)]
    data_b64: Option<String>,

    #[serde(default)]
    reason: Option<String>,

    #[serde(default)]
    max_events: Option<usize>,

    #[serde(default)]
    wait_ms: Option<u64>,

    #[serde(default)]
    consumer_id: Option<String>,

    #[serde(default)]
    mac: Option<String>,

    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Serialize)]
struct ApiResponse<T: Serialize> {
    ok: bool,
    data: Option<T>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    app: String,
    version: String,
    device_id: String,
    device_name: String,
    psk_configured: bool,
    api_addr: String,
    uptime_ms: u128,
}

#[derive(Debug, Serialize)]
struct PeerResponse {
    device_id: String,
    device_name: String,
    addr: String,
    macs: Vec<String>,
    trusted: bool,
    trusted_name: Option<String>,
    connected: bool,
    last_seen_ms_ago: u128,
}

#[derive(Debug, Serialize)]
struct ConnectionResponse {
    device_id: String,
    device_name: String,
    addr: String,
    connected_ms_ago: u128,
    last_seen_ms_ago: u128,
}

#[derive(Debug, Serialize)]
struct SendResponse {
    peer_id: String,
    service: String,
    message_id: String,
}

#[derive(Debug, Serialize)]
struct SpaceSendPeerResult {
    peer_id: String,
    ok: bool,
    message_id: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct SpaceSendResponse {
    space_id: String,
    service: String,
    target_peer_id: Option<String>,
    deliveries: Vec<SpaceSendPeerResult>,
}

#[derive(Debug, Serialize)]
struct ChannelOpenResponse {
    peer_id: String,
    service: String,
    channel_id: String,
}

#[derive(Debug, Serialize)]
struct ChannelDataResponse {
    peer_id: String,
    service: String,
    channel_id: String,
    message_id: String,
}

#[derive(Debug, Serialize)]
struct ChannelCloseResponse {
    peer_id: String,
    service: String,
    channel_id: String,
    message_id: String,
}

fn ok<T: Serialize>(data: T) -> ApiResponse<T> {
    ApiResponse {
        ok: true,
        data: Some(data),
        error: None,
    }
}

fn err(message: impl Into<String>) -> ApiResponse<()> {
    ApiResponse {
        ok: false,
        data: None,
        error: Some(message.into()),
    }
}

pub async fn local_api_server(
    cfg: Config,
    peers: Arc<Mutex<HashMap<String, Peer>>>,
    connections: ConnectionRegistry,
    events: EventQueue,
    addons: Arc<Mutex<Vec<AddonRecord>>>,
    spaces: SpaceRegistry,
    connecting: Arc<Mutex<HashSet<String>>>,
    opts: RunOptions,
    cfg_for_connect: Config,
    started_at: Instant,
) -> Result<()> {
    let listener = TcpListener::bind(LOCAL_API_ADDR).await?;

    println!("Local addon/control API listening on {LOCAL_API_ADDR}");

    loop {
        let (stream, _addr) = listener.accept().await?;
        let cfg_clone = cfg.clone();
        let peers_clone = peers.clone();
        let connections_clone = connections.clone();
        let events_clone = events.clone();
        let addons_clone = addons.clone();
        let spaces_clone = spaces.clone();
        let connecting_clone = connecting.clone();
        let opts_clone = opts.clone();
        let cfg_for_connect_clone = cfg_for_connect.clone();

        tokio::spawn(async move {
            let result = handle_api_client(
                cfg_clone,
                peers_clone,
                connections_clone,
                events_clone,
                addons_clone,
                spaces_clone,
                connecting_clone,
                opts_clone,
                cfg_for_connect_clone,
                started_at,
                stream,
            )
            .await;

            if let Err(e) = result {
                let msg = e.to_string();
                if !msg.contains("forcibly closed")
                    && !msg.contains("reset by peer")
                    && !msg.contains("broken pipe")
                {
                    eprintln!("Local API client ended: {e}");
                }
            }
        });
    }
}

async fn handle_api_client(
    cfg: Config,
    peers: Arc<Mutex<HashMap<String, Peer>>>,
    connections: ConnectionRegistry,
    events: EventQueue,
    addons: Arc<Mutex<Vec<AddonRecord>>>,
    spaces: SpaceRegistry,
    connecting: Arc<Mutex<HashSet<String>>>,
    opts: RunOptions,
    cfg_for_connect: Config,
    started_at: Instant,
    stream: TcpStream,
) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    let response: String = match timeout(Duration::from_secs(10), lines.next_line()).await {
        Ok(Ok(Some(line))) => match serde_json::from_str::<ApiRequest>(&line) {
            Ok(req) => {
                let cmd_timeout = match req.cmd.as_str() {
                    "wait_events" => Duration::from_secs(40),
                    _ => Duration::from_secs(5),
                };

                match timeout(
                    cmd_timeout,
                    handle_request(
                        &cfg,
                        peers.clone(),
                        connections.clone(),
                        events.clone(),
                        addons.clone(),
                        spaces.clone(),
                        connecting.clone(),
                        opts.clone(),
                        cfg_for_connect.clone(),
                        started_at,
                        req,
                    ),
                )
                .await
                {
                    Ok(Ok(response)) => response,
                    Ok(Err(e)) => serde_json::to_string(&err(format!("request failed: {e}")))?,
                    Err(_) => serde_json::to_string(&err("request timed out"))?,
                }
            }
            Err(e) => serde_json::to_string(&err(format!("invalid JSON request: {e}")))?,
        },
        Ok(Ok(None)) => serde_json::to_string(&err("empty request"))?,
        Ok(Err(e)) => serde_json::to_string(&err(format!("read failed: {e}")))?,
        Err(_) => serde_json::to_string(&err("read timed out"))?,
    };

    write_half.write_all(response.as_bytes()).await?;
    write_half.write_all(b"\n").await?;
    write_half.flush().await?;

    Ok(())
}

async fn handle_request(
    cfg: &Config,
    peers: Arc<Mutex<HashMap<String, Peer>>>,
    connections: ConnectionRegistry,
    events: EventQueue,
    addons: Arc<Mutex<Vec<AddonRecord>>>,
    spaces: SpaceRegistry,
    connecting: Arc<Mutex<HashSet<String>>>,
    opts: RunOptions,
    cfg_for_connect: Config,
    started_at: Instant,
    req: ApiRequest,
) -> Result<String> {
    match req.cmd.as_str() {
        "help" => {
            let response = serde_json::json!({
                "commands": [
                    "status",
                    "paths",
                    "shutdown",
                    "list_peers",
                    "list_connections",
                    "list_trusted_devices",
                    "add_trusted_device",
                    "remove_trusted_device",
                    "connect_device",
                    "disconnect_device",
                    "list_spaces",
                    "create_space",
                    "add_space_member",
                    "remove_space_member",
                    "send_space_message",
                    "list_addons",
                    "reload_addons",
                    "send_message",
                    "open_channel",
                    "channel_send",
                    "channel_close",
                    "poll_events",
                    "wait_events"
                ]
            });
            Ok(serde_json::to_string(&ok(response))?)
        }

        "status" => {
            let response = StatusResponse {
                app: "locallink-core".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                device_id: cfg.device_id.clone(),
                device_name: cfg.device_name.clone(),
                psk_configured: cfg.psk_b64.is_some(),
                api_addr: LOCAL_API_ADDR.to_string(),
                uptime_ms: started_at.elapsed().as_millis(),
            };

            Ok(serde_json::to_string(&ok(response))?)
        }

        "paths" => {
            let response = app_paths()?;
            Ok(serde_json::to_string(&ok(response))?)
        }

        "shutdown" => {
            tokio::spawn(async move {
                sleep(Duration::from_millis(250)).await;
                std::process::exit(0);
            });

            Ok(serde_json::to_string(&ok(serde_json::json!({
                "message": "LocalLink Core shutting down"
            })))?)
        }

        "list_spaces" => {
            let mut response = spaces.lock().await.spaces.clone();
            response.sort_by(|a, b| a.name.cmp(&b.name).then(a.space_id.cmp(&b.space_id)));
            Ok(serde_json::to_string(&ok(response))?)
        }

        "create_space" => {
            let space_id = req.space_id.unwrap_or_else(|| Uuid::new_v4().to_string());
            let name = req.name.unwrap_or_else(|| space_id.clone());
            let kind = req.kind.unwrap_or(SpaceKind::Direct);

            let mut store = spaces.lock().await;
            anyhow::ensure!(
                !store.spaces.iter().any(|space| space.space_id == space_id),
                "space already exists: {}",
                space_id
            );

            store.spaces.push(SpaceRecord {
                space_id: space_id.clone(),
                name,
                kind,
                members: Vec::new(),
                addons: HashMap::new(),
            });
            store.validate_and_repair()?;
            save_space_store(&store)?;

            let response = store
                .spaces
                .iter()
                .find(|space| space.space_id == space_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("space was not created"))?;

            Ok(serde_json::to_string(&ok(response))?)
        }

        "add_space_member" => {
            let space_id = req
                .space_id
                .ok_or_else(|| anyhow::anyhow!("add_space_member requires space_id"))?;
            let peer_id = req
                .peer_id
                .ok_or_else(|| anyhow::anyhow!("add_space_member requires peer_id"))?;

            let mut store = spaces.lock().await;
            let space = store
                .spaces
                .iter_mut()
                .find(|space| space.space_id == space_id)
                .ok_or_else(|| anyhow::anyhow!("unknown space: {}", space_id))?;

            space.members.push(peer_id);
            store.validate_and_repair()?;
            save_space_store(&store)?;

            let response = store
                .spaces
                .iter()
                .find(|space| space.space_id == space_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("unknown space after update: {}", space_id))?;

            Ok(serde_json::to_string(&ok(response))?)
        }

        "remove_space_member" => {
            let space_id = req
                .space_id
                .ok_or_else(|| anyhow::anyhow!("remove_space_member requires space_id"))?;
            let peer_id = req
                .peer_id
                .ok_or_else(|| anyhow::anyhow!("remove_space_member requires peer_id"))?;

            let mut store = spaces.lock().await;
            let space = store
                .spaces
                .iter_mut()
                .find(|space| space.space_id == space_id)
                .ok_or_else(|| anyhow::anyhow!("unknown space: {}", space_id))?;

            space.members.retain(|member| member != &peer_id);
            store.validate_and_repair()?;
            save_space_store(&store)?;

            let response = store
                .spaces
                .iter()
                .find(|space| space.space_id == space_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("unknown space after update: {}", space_id))?;

            Ok(serde_json::to_string(&ok(response))?)
        }

        "send_space_message" => {
            let space_id = req
                .space_id
                .ok_or_else(|| anyhow::anyhow!("send_space_message requires space_id"))?;
            let service = req
                .service
                .ok_or_else(|| anyhow::anyhow!("send_space_message requires service"))?;
            let data_b64 = req
                .data_b64
                .ok_or_else(|| anyhow::anyhow!("send_space_message requires data_b64"))?;
            let target_peer_id = req.target_peer_id;

            let space = {
                let store = spaces.lock().await;
                store
                    .spaces
                    .iter()
                    .find(|space| space.space_id == space_id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("unknown space: {}", space_id))?
            };

            let peer_ids = if let Some(target_peer_id) = &target_peer_id {
                anyhow::ensure!(
                    space.members.iter().any(|member| member == target_peer_id),
                    "target peer is not a member of space {}",
                    space_id
                );
                vec![target_peer_id.clone()]
            } else {
                space.members.clone()
            };

            anyhow::ensure!(!peer_ids.is_empty(), "space has no members: {}", space_id);

            let mut deliveries = Vec::new();

            for peer_id in peer_ids {
                match send_service_message(connections.clone(), &peer_id, &service, &data_b64).await
                {
                    Ok(message_id) => deliveries.push(SpaceSendPeerResult {
                        peer_id,
                        ok: true,
                        message_id: Some(message_id),
                        error: None,
                    }),
                    Err(err) => deliveries.push(SpaceSendPeerResult {
                        peer_id,
                        ok: false,
                        message_id: None,
                        error: Some(err.to_string()),
                    }),
                }
            }

            let response = SpaceSendResponse {
                space_id,
                service,
                target_peer_id,
                deliveries,
            };

            Ok(serde_json::to_string(&ok(response))?)
        }

        "list_trusted_devices" => {
            let response = load_trusted_devices()?;
            Ok(serde_json::to_string(&ok(response))?)
        }

        "add_trusted_device" => {
            let mac = req
                .mac
                .ok_or_else(|| anyhow::anyhow!("add_trusted_device requires mac"))?;
            let name = req.name.unwrap_or_else(|| mac.clone());

            let response = add_trusted_device(name, mac)?;
            Ok(serde_json::to_string(&ok(response))?)
        }

        "remove_trusted_device" => {
            let mac = req
                .mac
                .ok_or_else(|| anyhow::anyhow!("remove_trusted_device requires mac"))?;

            let wanted = normalize_mac(&mac);
            let existing_trusted_devices = load_trusted_devices()?;

            let mut matching_peer_ids: Vec<String> = existing_trusted_devices
                .iter()
                .filter(|device| device.macs.iter().any(|m| normalize_mac(m) == wanted))
                .filter_map(|device| device.device_id.clone())
                .collect();

            {
                let peers_guard = peers.lock().await;
                matching_peer_ids.extend(
                    peers_guard
                        .values()
                        .filter(|peer| peer.macs.iter().any(|m| normalize_mac(m) == wanted))
                        .map(|peer| peer.device_id.clone()),
                );
            }

            matching_peer_ids.sort();
            matching_peer_ids.dedup();

            let mut disconnected = Vec::new();

            for peer_id in matching_peer_ids {
                if disconnect_peer(connections.clone(), &peer_id)
                    .await
                    .unwrap_or(false)
                {
                    disconnected.push(peer_id);
                }
            }

            let trusted_devices = remove_trusted_mac(&mac)?;

            let response = serde_json::json!({
                "trusted_devices": trusted_devices,
                "disconnected": disconnected
            });

            Ok(serde_json::to_string(&ok(response))?)
        }

        "connect_device" => {
            let peer_id = req.peer_id.clone();
            let mac = req.mac.clone();

            let peers_guard = peers.lock().await;

            let target = peers_guard
                .values()
                .find(|peer| {
                    if let Some(peer_id) = &peer_id {
                        if &peer.device_id == peer_id {
                            return true;
                        }
                    }

                    if let Some(mac) = &mac {
                        let wanted = normalize_mac(mac);
                        return peer.macs.iter().any(|m| normalize_mac(m) == wanted);
                    }

                    false
                })
                .cloned();

            drop(peers_guard);

            let Some(peer) = target else {
                anyhow::bail!("No nearby discovered device matched that device ID or MAC");
            };

            if !mac_is_trusted(&peer.macs)? {
                anyhow::bail!("Device is visible, but none of its MAC addresses are trusted yet");
            }

            if connections.lock().await.contains_key(&peer.device_id) {
                let response = serde_json::json!({
                    "status": "already_connected",
                    "device_id": peer.device_id,
                    "device_name": peer.device_name,
                    "macs": peer.macs
                });
                return Ok(serde_json::to_string(&ok(response))?);
            }

            {
                let mut guard = connecting.lock().await;
                if guard.contains(&peer.device_id) {
                    let response = serde_json::json!({
                        "status": "already_connecting",
                        "device_id": peer.device_id,
                        "device_name": peer.device_name,
                        "macs": peer.macs
                    });
                    return Ok(serde_json::to_string(&ok(response))?);
                }
                guard.insert(peer.device_id.clone());
            }

            let cfg_connect = cfg_for_connect.clone();
            let opts_connect = opts.clone();
            let connections_connect = connections.clone();
            let events_connect = events.clone();
            let connecting_connect = connecting.clone();
            let peer_id_connect = peer.device_id.clone();
            let peer_addr = peer.addr;
            let peer_macs = peer.macs.clone();

            tokio::spawn(async move {
                connect_to_peer(
                    cfg_connect,
                    opts_connect,
                    peer_addr,
                    peer_id_connect.clone(),
                    peer_macs,
                    connections_connect,
                    events_connect,
                )
                .await;

                connecting_connect.lock().await.remove(&peer_id_connect);
            });

            let response = serde_json::json!({
                "status": "connecting",
                "device_id": peer.device_id,
                "device_name": peer.device_name,
                "macs": peer.macs
            });

            Ok(serde_json::to_string(&ok(response))?)
        }

        "list_peers" => {
            let now = Instant::now();
            let peers_guard = peers.lock().await;
            let connections_guard = connections.lock().await;

            let mut response: Vec<PeerResponse> = Vec::new();

            for peer in peers_guard.values() {
                response.push(PeerResponse {
                    device_id: peer.device_id.clone(),
                    device_name: peer.device_name.clone(),
                    addr: peer.addr.to_string(),
                    macs: peer.macs.clone(),
                    trusted: mac_is_trusted(&peer.macs).unwrap_or(false),
                    trusted_name: trusted_name_for_macs(&peer.macs).unwrap_or(None),
                    connected: connections_guard.contains_key(&peer.device_id),
                    last_seen_ms_ago: now.duration_since(peer.last_seen).as_millis(),
                });
            }

            response.sort_by(|a, b| a.device_name.cmp(&b.device_name));
            Ok(serde_json::to_string(&ok(response))?)
        }

        "list_connections" => {
            let now = Instant::now();
            let conns: Vec<_> = connections.lock().await.values().cloned().collect();

            let mut response = Vec::new();

            for conn in conns {
                let last_seen = *conn.last_seen.lock().await;

                response.push(ConnectionResponse {
                    device_id: conn.device_id,
                    device_name: conn.device_name,
                    addr: conn.addr,
                    connected_ms_ago: now.duration_since(conn.connected_since).as_millis(),
                    last_seen_ms_ago: now.duration_since(last_seen).as_millis(),
                });
            }

            response.sort_by(|a, b| a.device_name.cmp(&b.device_name));
            Ok(serde_json::to_string(&ok(response))?)
        }

        "disconnect_device" => {
            let mut target_peer_ids = Vec::new();

            if let Some(peer_id) = req.peer_id.clone() {
                target_peer_ids.push(peer_id);
            }

            if let Some(mac) = req.mac.clone() {
                let wanted = normalize_mac(&mac);

                let peers_guard = peers.lock().await;

                for peer in peers_guard.values() {
                    if peer.macs.iter().any(|m| normalize_mac(m) == wanted) {
                        target_peer_ids.push(peer.device_id.clone());
                    }
                }
            }

            target_peer_ids.sort();
            target_peer_ids.dedup();

            if target_peer_ids.is_empty() {
                anyhow::bail!(
                    "disconnect_device requires peer_id or a MAC matching a discovered peer"
                );
            }

            let mut disconnected = Vec::new();

            for peer_id in target_peer_ids {
                if disconnect_peer(connections.clone(), &peer_id)
                    .await
                    .unwrap_or(false)
                {
                    disconnected.push(peer_id);
                }
            }

            let response = serde_json::json!({
                "disconnected": disconnected
            });

            Ok(serde_json::to_string(&ok(response))?)
        }

        "list_addons" => {
            let response = addons.lock().await.clone();
            Ok(serde_json::to_string(&ok(response))?)
        }

        "reload_addons" => {
            let loaded = load_addon_manifests()?;
            *addons.lock().await = loaded.clone();
            Ok(serde_json::to_string(&ok(loaded))?)
        }

        "send_message" => {
            let peer_id = req
                .peer_id
                .ok_or_else(|| anyhow::anyhow!("send_message requires peer_id"))?;
            let service = req
                .service
                .ok_or_else(|| anyhow::anyhow!("send_message requires service"))?;
            let data_b64 = req
                .data_b64
                .ok_or_else(|| anyhow::anyhow!("send_message requires data_b64"))?;

            let message_id =
                send_service_message(connections.clone(), &peer_id, &service, &data_b64).await?;

            let response = SendResponse {
                peer_id,
                service,
                message_id,
            };

            Ok(serde_json::to_string(&ok(response))?)
        }

        "open_channel" => {
            let peer_id = req
                .peer_id
                .ok_or_else(|| anyhow::anyhow!("open_channel requires peer_id"))?;
            let service = req
                .service
                .ok_or_else(|| anyhow::anyhow!("open_channel requires service"))?;

            let channel_id = open_channel(connections.clone(), &peer_id, &service).await?;

            let response = ChannelOpenResponse {
                peer_id,
                service,
                channel_id,
            };

            Ok(serde_json::to_string(&ok(response))?)
        }

        "channel_send" => {
            let peer_id = req
                .peer_id
                .ok_or_else(|| anyhow::anyhow!("channel_send requires peer_id"))?;
            let service = req
                .service
                .ok_or_else(|| anyhow::anyhow!("channel_send requires service"))?;
            let channel_id = req
                .channel_id
                .ok_or_else(|| anyhow::anyhow!("channel_send requires channel_id"))?;
            let data_b64 = req
                .data_b64
                .ok_or_else(|| anyhow::anyhow!("channel_send requires data_b64"))?;

            let message_id = send_channel_data(
                connections.clone(),
                &peer_id,
                &service,
                &channel_id,
                &data_b64,
            )
            .await?;

            let response = ChannelDataResponse {
                peer_id,
                service,
                channel_id,
                message_id,
            };

            Ok(serde_json::to_string(&ok(response))?)
        }

        "channel_close" => {
            let peer_id = req
                .peer_id
                .ok_or_else(|| anyhow::anyhow!("channel_close requires peer_id"))?;
            let service = req
                .service
                .ok_or_else(|| anyhow::anyhow!("channel_close requires service"))?;
            let channel_id = req
                .channel_id
                .ok_or_else(|| anyhow::anyhow!("channel_close requires channel_id"))?;
            let reason = req.reason.unwrap_or_else(|| "normal".to_string());

            let message_id = close_channel(
                connections.clone(),
                &peer_id,
                &service,
                &channel_id,
                &reason,
            )
            .await?;

            let response = ChannelCloseResponse {
                peer_id,
                service,
                channel_id,
                message_id,
            };

            Ok(serde_json::to_string(&ok(response))?)
        }

        "poll_events" => {
            let max_events = req.max_events.unwrap_or(100).clamp(1, 1000);
            let consumer_id = req.consumer_id.as_deref().unwrap_or("default");
            let response = take_events(
                events.clone(),
                consumer_id,
                req.service.as_deref(),
                max_events,
            )
            .await;

            Ok(serde_json::to_string(&ok(response))?)
        }

        "wait_events" => {
            let max_events = req.max_events.unwrap_or(100).clamp(1, 1000);
            let wait_ms = req.wait_ms.unwrap_or(30_000).clamp(1, 30_000);
            let consumer_id = req.consumer_id.unwrap_or_else(|| "default".to_string());
            let deadline = Instant::now() + Duration::from_millis(wait_ms);

            loop {
                let response = take_events(
                    events.clone(),
                    &consumer_id,
                    req.service.as_deref(),
                    max_events,
                )
                .await;

                if !response.is_empty() || Instant::now() >= deadline {
                    break Ok(serde_json::to_string(&ok(response))?);
                }

                sleep(Duration::from_millis(100)).await;
            }
        }

        other => Ok(serde_json::to_string(&err(format!(
            "unknown command: {other}"
        )))?),
    }
}
