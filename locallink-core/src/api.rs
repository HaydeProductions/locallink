use crate::addons::{load_addon_manifests, AddonRecord};
use crate::config::{
    add_trusted_device, app_paths, load_trusted_devices, mac_is_trusted, normalize_mac,
    remove_trusted_mac, trusted_name_for_macs, Config,
};
use crate::discovery::Peer;
use crate::spaces::{save_space_store, SpaceAddonState, SpaceKind, SpaceRecord, SpaceStore};
use crate::transport::{
    close_channel, connect_to_peer, disconnect_peer, open_channel, send_channel_data,
    send_service_message, send_space_service_message, take_events, ConnectionRegistry, EventQueue,
    RunOptions,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, Mutex};
use tokio::time::{sleep, timeout, Duration, Instant};
use uuid::Uuid;

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
    consumer_id: Option<String>,

    #[serde(default)]
    mac: Option<String>,

    #[serde(default)]
    name: Option<String>,

    #[serde(default)]
    space_id: Option<String>,

    #[serde(default)]
    space_name: Option<String>,

    #[serde(default)]
    space_kind: Option<String>,

    #[serde(default)]
    member_peer_id: Option<String>,

    #[serde(default)]
    target_peer_id: Option<String>,

    #[serde(default)]
    addon_id: Option<String>,

    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ApiResponse<T: Serialize> {
    ok: bool,
    data: Option<T>,
    error: Option<String>,
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

#[derive(Clone)]
struct ApiContext {
    cfg: Config,
    peers: Arc<Mutex<HashMap<String, Peer>>>,
    connections: ConnectionRegistry,
    events: EventQueue,
    addons: Arc<Mutex<Vec<AddonRecord>>>,
    spaces: Arc<Mutex<SpaceStore>>,
    connecting: Arc<Mutex<HashSet<String>>>,
    opts: RunOptions,
    cfg_for_connect: Config,
    shutdown_tx: broadcast::Sender<()>,
    started_at: Instant,
}

pub async fn local_api_server(
    cfg: Config,
    peers: Arc<Mutex<HashMap<String, Peer>>>,
    connections: ConnectionRegistry,
    events: EventQueue,
    addons: Arc<Mutex<Vec<AddonRecord>>>,
    spaces: Arc<Mutex<SpaceStore>>,
    connecting: Arc<Mutex<HashSet<String>>>,
    opts: RunOptions,
    cfg_for_connect: Config,
    shutdown_tx: broadcast::Sender<()>,
    started_at: Instant,
) -> Result<()> {
    let listener = TcpListener::bind(LOCAL_API_ADDR).await?;

    println!("Local addon/control API listening on {LOCAL_API_ADDR}");

    loop {
        let (stream, _addr) = listener.accept().await?;
        let ctx = ApiContext {
            cfg: cfg.clone(),
            peers: peers.clone(),
            connections: connections.clone(),
            events: events.clone(),
            addons: addons.clone(),
            spaces: spaces.clone(),
            connecting: connecting.clone(),
            opts: opts.clone(),
            cfg_for_connect: cfg_for_connect.clone(),
            shutdown_tx: shutdown_tx.clone(),
            started_at,
        };

        tokio::spawn(async move {
            let result = handle_api_client(ctx, stream).await;

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

async fn handle_api_client(ctx: ApiContext, stream: TcpStream) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    let response: String = match timeout(Duration::from_secs(10), lines.next_line()).await {
        Ok(Ok(Some(line))) => match serde_json::from_str::<ApiRequest>(&line) {
            Ok(req) => {
                let cmd_timeout = match req.cmd.as_str() {
                    "wait_events" => Duration::from_secs(40),
                    _ => Duration::from_secs(5),
                };

                match timeout(cmd_timeout, handle_request(ctx, req)).await {
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

async fn handle_request(ctx: ApiContext, req: ApiRequest) -> Result<String> {
    match req.cmd.as_str() {
        "help" => json_ok(json!({
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
                "list_addons",
                "reload_addons",
                "list_spaces",
                "create_space",
                "delete_space",
                "rename_space",
                "add_space_member",
                "remove_space_member",
                "send_space_message",
                "set_space_addon_enabled",
                "list_space_addons",
                "send_message",
                "open_channel",
                "channel_send",
                "channel_close",
                "poll_events",
                "wait_events"
            ]
        })),

        "status" => json_ok(json!({
            "app": "locallink-core",
            "version": env!("CARGO_PKG_VERSION"),
            "device_id": ctx.cfg.device_id.clone(),
            "device_name": ctx.cfg.device_name.clone(),
            "psk_configured": ctx.cfg.psk_b64.is_some(),
            "api_addr": LOCAL_API_ADDR,
            "uptime_ms": ctx.started_at.elapsed().as_millis()
        })),

        "paths" => json_ok(app_paths()?),

        "shutdown" => {
            let _ = ctx.shutdown_tx.send(());
            json_ok(json!({
                "message": "LocalLink Core shutdown requested"
            }))
        }

        "list_trusted_devices" => json_ok(load_trusted_devices()?),

        "add_trusted_device" => {
            let mac = required(req.mac, "add_trusted_device requires mac")?;
            let name = req.name.unwrap_or_else(|| mac.clone());
            json_ok(add_trusted_device(name, mac)?)
        }

        "remove_trusted_device" => remove_trusted_device(ctx, req).await,
        "connect_device" => connect_device(ctx, req).await,
        "list_peers" => list_peers(ctx).await,
        "list_connections" => list_connections(ctx).await,
        "disconnect_device" => disconnect_device(ctx, req).await,

        "list_addons" => json_ok(ctx.addons.lock().await.clone()),
        "reload_addons" => {
            let loaded = load_addon_manifests()?;
            *ctx.addons.lock().await = loaded.clone();
            json_ok(loaded)
        }

        "list_spaces" => json_ok(ctx.spaces.lock().await.clone()),
        "create_space" => create_space(ctx, req).await,
        "delete_space" => delete_space(ctx, req).await,
        "rename_space" => rename_space(ctx, req).await,
        "add_space_member" => add_space_member(ctx, req).await,
        "remove_space_member" => remove_space_member(ctx, req).await,
        "send_space_message" => send_space_message(ctx, req).await,
        "set_space_addon_enabled" => set_space_addon_enabled(ctx, req).await,
        "list_space_addons" => list_space_addons(ctx, req).await,

        "send_message" => {
            let peer_id = required(req.peer_id, "send_message requires peer_id")?;
            let service = required(req.service, "send_message requires service")?;
            let data_b64 = required(req.data_b64, "send_message requires data_b64")?;
            let message_id =
                send_service_message(ctx.connections, &peer_id, &service, &data_b64).await?;
            json_ok(json!({
                "peer_id": peer_id,
                "service": service,
                "message_id": message_id
            }))
        }

        "open_channel" => {
            let peer_id = required(req.peer_id, "open_channel requires peer_id")?;
            let service = required(req.service, "open_channel requires service")?;
            let channel_id = open_channel(ctx.connections, &peer_id, &service).await?;
            json_ok(json!({
                "peer_id": peer_id,
                "service": service,
                "channel_id": channel_id
            }))
        }

        "channel_send" => {
            let peer_id = required(req.peer_id, "channel_send requires peer_id")?;
            let service = required(req.service, "channel_send requires service")?;
            let channel_id = required(req.channel_id, "channel_send requires channel_id")?;
            let data_b64 = required(req.data_b64, "channel_send requires data_b64")?;
            let message_id = send_channel_data(
                ctx.connections,
                &peer_id,
                &service,
                &channel_id,
                &data_b64,
            )
            .await?;
            json_ok(json!({
                "peer_id": peer_id,
                "service": service,
                "channel_id": channel_id,
                "message_id": message_id
            }))
        }

        "channel_close" => {
            let peer_id = required(req.peer_id, "channel_close requires peer_id")?;
            let service = required(req.service, "channel_close requires service")?;
            let channel_id = required(req.channel_id, "channel_close requires channel_id")?;
            let reason = req.reason.unwrap_or_else(|| "normal".to_string());
            let message_id =
                close_channel(ctx.connections, &peer_id, &service, &channel_id, &reason).await?;
            json_ok(json!({
                "peer_id": peer_id,
                "service": service,
                "channel_id": channel_id,
                "message_id": message_id
            }))
        }

        "poll_events" => {
            let max_events = req.max_events.unwrap_or(100).clamp(1, 1000);
            let consumer_id = req.consumer_id.as_deref().unwrap_or("default");
            let response = take_events(
                ctx.events,
                consumer_id,
                req.service.as_deref(),
                max_events,
            )
            .await;
            json_ok(response)
        }

        "wait_events" => wait_events(ctx, req).await,

        other => Ok(serde_json::to_string(&err(format!(
            "unknown command: {other}"
        )))?),
    }
}

fn json_ok<T: Serialize>(data: T) -> Result<String> {
    Ok(serde_json::to_string(&ok(data))?)
}

fn required(value: Option<String>, message: &str) -> Result<String> {
    value.ok_or_else(|| anyhow::anyhow!(message.to_string()))
}

fn parse_space_kind(kind: Option<&str>) -> Result<SpaceKind> {
    match kind.unwrap_or("direct").trim().to_ascii_lowercase().as_str() {
        "direct" => Ok(SpaceKind::Direct),
        "group" => Ok(SpaceKind::Group),
        other => anyhow::bail!("unknown space_kind: {other}"),
    }
}

async fn remove_trusted_device(ctx: ApiContext, req: ApiRequest) -> Result<String> {
    let mac = required(req.mac, "remove_trusted_device requires mac")?;
    let wanted = normalize_mac(&mac);
    let mut matching_peer_ids: Vec<String> = load_trusted_devices()?
        .iter()
        .filter(|device| device.macs.iter().any(|m| normalize_mac(m) == wanted))
        .filter_map(|device| device.device_id.clone())
        .collect();

    matching_peer_ids.extend(
        ctx.peers
            .lock()
            .await
            .values()
            .filter(|peer| peer.macs.iter().any(|m| normalize_mac(m) == wanted))
            .map(|peer| peer.device_id.clone()),
    );
    matching_peer_ids.sort();
    matching_peer_ids.dedup();

    let mut disconnected = Vec::new();

    for peer_id in matching_peer_ids {
        if disconnect_peer(ctx.connections.clone(), &peer_id)
            .await
            .unwrap_or(false)
        {
            disconnected.push(peer_id);
        }
    }

    json_ok(json!({
        "trusted_devices": remove_trusted_mac(&mac)?,
        "disconnected": disconnected
    }))
}

async fn connect_device(ctx: ApiContext, req: ApiRequest) -> Result<String> {
    let target = {
        let peers_guard = ctx.peers.lock().await;
        peers_guard
            .values()
            .find(|peer| {
                req.peer_id
                    .as_ref()
                    .is_some_and(|peer_id| &peer.device_id == peer_id)
                    || req.mac.as_ref().is_some_and(|mac| {
                        let wanted = normalize_mac(mac);
                        peer.macs.iter().any(|m| normalize_mac(m) == wanted)
                    })
            })
            .cloned()
    };

    let Some(peer) = target else {
        anyhow::bail!("No nearby discovered device matched that device ID or MAC");
    };

    if !mac_is_trusted(&peer.macs)? {
        anyhow::bail!("Device is visible, but none of its MAC addresses are trusted yet");
    }

    if ctx.connections.lock().await.contains_key(&peer.device_id) {
        return json_ok(json!({
            "status": "already_connected",
            "device_id": peer.device_id,
            "device_name": peer.device_name,
            "macs": peer.macs
        }));
    }

    {
        let mut guard = ctx.connecting.lock().await;
        if guard.contains(&peer.device_id) {
            return json_ok(json!({
                "status": "already_connecting",
                "device_id": peer.device_id,
                "device_name": peer.device_name,
                "macs": peer.macs
            }));
        }
        guard.insert(peer.device_id.clone());
    }

    let peer_id_connect = peer.device_id.clone();
    let peer_addr = peer.addr;
    let peer_macs = peer.macs.clone();
    let connecting = ctx.connecting.clone();

    tokio::spawn(async move {
        connect_to_peer(
            ctx.cfg_for_connect,
            ctx.opts,
            peer_addr,
            peer_id_connect.clone(),
            peer_macs,
            ctx.connections,
            ctx.events,
        )
        .await;

        connecting.lock().await.remove(&peer_id_connect);
    });

    json_ok(json!({
        "status": "connecting",
        "device_id": peer.device_id,
        "device_name": peer.device_name,
        "macs": peer.macs
    }))
}

async fn list_peers(ctx: ApiContext) -> Result<String> {
    let now = Instant::now();
    let peers_guard = ctx.peers.lock().await;
    let connections_guard = ctx.connections.lock().await;
    let mut response = Vec::new();

    for peer in peers_guard.values() {
        response.push(json!({
            "device_id": peer.device_id.clone(),
            "device_name": peer.device_name.clone(),
            "addr": peer.addr.to_string(),
            "macs": peer.macs.clone(),
            "trusted": mac_is_trusted(&peer.macs).unwrap_or(false),
            "trusted_name": trusted_name_for_macs(&peer.macs).unwrap_or(None),
            "connected": connections_guard.contains_key(&peer.device_id),
            "last_seen_ms_ago": now.duration_since(peer.last_seen).as_millis()
        }));
    }

    response.sort_by(|a, b| a["device_name"].as_str().cmp(&b["device_name"].as_str()));
    json_ok(response)
}

async fn list_connections(ctx: ApiContext) -> Result<String> {
    let now = Instant::now();
    let conns: Vec<_> = ctx.connections.lock().await.values().cloned().collect();
    let mut response = Vec::new();

    for conn in conns {
        let last_seen = *conn.last_seen.lock().await;
        response.push(json!({
            "device_id": conn.device_id,
            "device_name": conn.device_name,
            "addr": conn.addr,
            "connected_ms_ago": now.duration_since(conn.connected_since).as_millis(),
            "last_seen_ms_ago": now.duration_since(last_seen).as_millis()
        }));
    }

    response.sort_by(|a, b| a["device_name"].as_str().cmp(&b["device_name"].as_str()));
    json_ok(response)
}

async fn disconnect_device(ctx: ApiContext, req: ApiRequest) -> Result<String> {
    let mut target_peer_ids = Vec::new();

    if let Some(peer_id) = req.peer_id {
        target_peer_ids.push(peer_id);
    }

    if let Some(mac) = req.mac {
        let wanted = normalize_mac(&mac);
        target_peer_ids.extend(
            ctx.peers
                .lock()
                .await
                .values()
                .filter(|peer| peer.macs.iter().any(|m| normalize_mac(m) == wanted))
                .map(|peer| peer.device_id.clone()),
        );
    }

    target_peer_ids.sort();
    target_peer_ids.dedup();

    anyhow::ensure!(
        !target_peer_ids.is_empty(),
        "disconnect_device requires peer_id or a MAC matching a discovered peer"
    );

    let mut disconnected = Vec::new();

    for peer_id in target_peer_ids {
        if disconnect_peer(ctx.connections.clone(), &peer_id)
            .await
            .unwrap_or(false)
        {
            disconnected.push(peer_id);
        }
    }

    json_ok(json!({
        "disconnected": disconnected
    }))
}

async fn create_space(ctx: ApiContext, req: ApiRequest) -> Result<String> {
    let name = req
        .space_name
        .or(req.name)
        .ok_or_else(|| anyhow::anyhow!("create_space requires space_name"))?;
    let kind = parse_space_kind(req.space_kind.as_deref())?;
    let mut members = Vec::new();

    if let Some(member) = req.member_peer_id.or(req.peer_id) {
        members.push(member);
    }

    let mut record = SpaceRecord {
        space_id: Uuid::new_v4().to_string(),
        name,
        kind,
        members,
        addons: BTreeMap::new(),
    };

    let mut proposed = SpaceStore {
        spaces: vec![record.clone()],
    };
    proposed.validate_and_repair()?;
    record = proposed.spaces.remove(0);

    let mut store = ctx.spaces.lock().await;
    store.spaces.push(record.clone());
    store.validate_and_repair()?;
    save_space_store(&store)?;

    json_ok(record)
}

async fn delete_space(ctx: ApiContext, req: ApiRequest) -> Result<String> {
    let space_id = required(req.space_id, "delete_space requires space_id")?;
    let mut store = ctx.spaces.lock().await;
    let before = store.spaces.len();
    store.spaces.retain(|space| space.space_id != space_id);
    anyhow::ensure!(store.spaces.len() != before, "space not found: {space_id}");
    save_space_store(&store)?;

    json_ok(json!({
        "deleted": space_id
    }))
}

async fn rename_space(ctx: ApiContext, req: ApiRequest) -> Result<String> {
    let space_id = required(req.space_id, "rename_space requires space_id")?;
    let name = req
        .space_name
        .or(req.name)
        .ok_or_else(|| anyhow::anyhow!("rename_space requires space_name"))?;
    let mut store = ctx.spaces.lock().await;
    let space = find_space_mut(&mut store, &space_id)?;
    space.name = name;
    store.validate_and_repair()?;
    let response = find_space(&store, &space_id)?.clone();
    save_space_store(&store)?;

    json_ok(response)
}

async fn add_space_member(ctx: ApiContext, req: ApiRequest) -> Result<String> {
    let space_id = required(req.space_id, "add_space_member requires space_id")?;
    let member = req
        .member_peer_id
        .or(req.peer_id)
        .ok_or_else(|| anyhow::anyhow!("add_space_member requires member_peer_id"))?;
    let mut store = ctx.spaces.lock().await;
    find_space_mut(&mut store, &space_id)?.members.push(member);
    store.validate_and_repair()?;
    let response = find_space(&store, &space_id)?.clone();
    save_space_store(&store)?;

    json_ok(response)
}

async fn remove_space_member(ctx: ApiContext, req: ApiRequest) -> Result<String> {
    let space_id = required(req.space_id, "remove_space_member requires space_id")?;
    let member = req
        .member_peer_id
        .or(req.peer_id)
        .ok_or_else(|| anyhow::anyhow!("remove_space_member requires member_peer_id"))?;
    let mut store = ctx.spaces.lock().await;
    find_space_mut(&mut store, &space_id)?
        .members
        .retain(|existing| existing != &member);
    store.validate_and_repair()?;
    let response = find_space(&store, &space_id)?.clone();
    save_space_store(&store)?;

    json_ok(response)
}

async fn send_space_message(ctx: ApiContext, req: ApiRequest) -> Result<String> {
    let space_id = required(req.space_id, "send_space_message requires space_id")?;
    let service = required(req.service, "send_space_message requires service")?;
    let data_b64 = required(req.data_b64, "send_space_message requires data_b64")?;
    let target_peer_id = req.target_peer_id;
    let space = {
        let store = ctx.spaces.lock().await;
        find_space(&store, &space_id)?.clone()
    };

    let target_members = if let Some(target) = target_peer_id.clone() {
        anyhow::ensure!(
            space.members.iter().any(|member| member == &target),
            "target_peer_id is not a member of space {space_id}"
        );
        vec![target]
    } else {
        space.members.clone()
    };

    let mut deliveries = Vec::new();

    for peer_id in target_members {
        if !ctx.connections.lock().await.contains_key(&peer_id) {
            deliveries.push(json!({
                "peer_id": peer_id,
                "ok": false,
                "error": "not connected"
            }));
            continue;
        }

        match send_space_service_message(
            ctx.connections.clone(),
            &peer_id,
            &space_id,
            &service,
            target_peer_id.clone(),
            &data_b64,
        )
        .await
        {
            Ok(message_id) => deliveries.push(json!({
                "peer_id": peer_id,
                "ok": true,
                "message_id": message_id
            })),
            Err(err) => deliveries.push(json!({
                "peer_id": peer_id,
                "ok": false,
                "error": err.to_string()
            })),
        }
    }

    json_ok(json!({
        "space_id": space_id,
        "service": service,
        "target_peer_id": target_peer_id,
        "deliveries": deliveries
    }))
}

async fn set_space_addon_enabled(ctx: ApiContext, req: ApiRequest) -> Result<String> {
    let space_id = required(req.space_id, "set_space_addon_enabled requires space_id")?;
    let addon_id = required(req.addon_id, "set_space_addon_enabled requires addon_id")?;
    let enabled = req
        .enabled
        .ok_or_else(|| anyhow::anyhow!("set_space_addon_enabled requires enabled"))?;

    anyhow::ensure!(
        ctx.addons.lock().await.iter().any(|addon| addon.id == addon_id),
        "addon not found: {addon_id}"
    );

    let mut store = ctx.spaces.lock().await;
    find_space_mut(&mut store, &space_id)?
        .addons
        .insert(addon_id, SpaceAddonState { enabled });
    store.validate_and_repair()?;
    let response = find_space(&store, &space_id)?.clone();
    save_space_store(&store)?;

    json_ok(response)
}

async fn list_space_addons(ctx: ApiContext, req: ApiRequest) -> Result<String> {
    let addon_snapshot = ctx.addons.lock().await.clone();
    let store = ctx.spaces.lock().await;
    let selected_spaces: Vec<_> = store
        .spaces
        .iter()
        .filter(|space| match req.space_id.as_deref() {
            Some(space_id) => space.space_id == space_id,
            None => true,
        })
        .map(|space| {
            let addons: Vec<_> = addon_snapshot
                .iter()
                .map(|addon| {
                    let enabled = space
                        .addons
                        .get(&addon.id)
                        .map(|state| state.enabled)
                        .unwrap_or(false);
                    json!({
                        "addon_id": addon.id.clone(),
                        "name": addon.name.clone(),
                        "version": addon.version.clone(),
                        "description": addon.description.clone(),
                        "services": addon.services.clone(),
                        "enabled": enabled
                    })
                })
                .collect();
            json!({
                "space_id": space.space_id.clone(),
                "space_name": space.name.clone(),
                "space_kind": space.kind.clone(),
                "addons": addons
            })
        })
        .collect();

    if req.space_id.is_some() && selected_spaces.is_empty() {
        anyhow::bail!("space not found");
    }

    json_ok(selected_spaces)
}

async fn wait_events(ctx: ApiContext, req: ApiRequest) -> Result<String> {
    let max_events = req.max_events.unwrap_or(100).clamp(1, 1000);
    let wait_ms = req.wait_ms.unwrap_or(30_000).clamp(1, 30_000);
    let consumer_id = req.consumer_id.unwrap_or_else(|| "default".to_string());
    let deadline = Instant::now() + Duration::from_millis(wait_ms);

    loop {
        let response = take_events(
            ctx.events.clone(),
            &consumer_id,
            req.service.as_deref(),
            max_events,
        )
        .await;

        if !response.is_empty() || Instant::now() >= deadline {
            break json_ok(response);
        }

        sleep(Duration::from_millis(100)).await;
    }
}

fn find_space<'a>(store: &'a SpaceStore, space_id: &str) -> Result<&'a SpaceRecord> {
    store
        .spaces
        .iter()
        .find(|space| space.space_id == space_id)
        .ok_or_else(|| anyhow::anyhow!("space not found: {space_id}"))
}

fn find_space_mut<'a>(store: &'a mut SpaceStore, space_id: &str) -> Result<&'a mut SpaceRecord> {
    store
        .spaces
        .iter_mut()
        .find(|space| space.space_id == space_id)
        .ok_or_else(|| anyhow::anyhow!("space not found: {space_id}"))
}
