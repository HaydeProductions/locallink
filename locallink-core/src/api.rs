use crate::addons::{load_addon_manifests, AddonRecord};
use crate::config::{
    add_trusted_device, app_paths, load_trusted_devices, mac_is_trusted, normalize_mac,
    register_device_id_for_macs, remove_trusted_mac, trusted_name_for_macs, Config,
};
use crate::discovery::Peer;
use crate::transport::{
    close_channel, connect_to_peer, open_channel, send_channel_data, send_service_message,
    ApiEvent, ConnectionRegistry, EventQueue, RunOptions,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout, Duration, Instant};

pub const LOCAL_API_ADDR: &str = "127.0.0.1:47900";

#[derive(Debug, Deserialize)]
struct ApiRequest {
    cmd: String,

    #[serde(default)]
    peer_id: Option<String>,

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

            let response = remove_trusted_mac(&mac)?;
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

            register_device_id_for_macs(&peer.macs, &peer.device_id)?;

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

            tokio::spawn(async move {
                connect_to_peer(
                    cfg_connect,
                    opts_connect,
                    peer_addr,
                    peer_id_connect.clone(),
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
            let response =
                take_matching_events(events.clone(), req.service.as_deref(), max_events).await;

            Ok(serde_json::to_string(&ok(response))?)
        }

        "wait_events" => {
            let max_events = req.max_events.unwrap_or(100).clamp(1, 1000);
            let wait_ms = req.wait_ms.unwrap_or(30_000).clamp(1, 30_000);
            let deadline = Instant::now() + Duration::from_millis(wait_ms);

            loop {
                let response =
                    take_matching_events(events.clone(), req.service.as_deref(), max_events).await;

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

async fn take_matching_events(
    events: EventQueue,
    service_filter: Option<&str>,
    max_events: usize,
) -> Vec<ApiEvent> {
    let mut q = events.lock().await;
    let mut taken = Vec::new();
    let mut kept = VecDeque::new();

    while let Some(event) = q.pop_front() {
        let service_matches = match service_filter {
            Some(service) => event.service == service,
            None => true,
        };

        if service_matches && taken.len() < max_events {
            taken.push(event);
        } else {
            kept.push_back(event);
        }
    }

    *q = kept;
    taken
}
