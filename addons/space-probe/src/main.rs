use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_LOCAL_API_ADDR: &str = "127.0.0.1:47900";
const SERVICE: &str = "locallink.debug.space.probe";
const SEND_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
struct ProbeContext {
    api_addr: String,
    addon_id: String,
    instance_id: String,
    space_id: Option<String>,
    space_kind: Option<String>,
    space_name: Option<String>,
    connected_members: Vec<String>,
    log_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProbePayload {
    message_type: String,
    ts_ms: u128,
    seq: u64,
    origin_instance_id: String,
    origin_device_id: String,
    origin_device_name: String,
    space_id: Option<String>,
    note: String,
}

fn main() -> Result<()> {
    let ctx = ProbeContext::from_env()?;
    ctx.log("============================================================");
    ctx.log("LocalLink Space Probe starting");
    ctx.log(format!("addon_id={}", ctx.addon_id));
    ctx.log(format!("instance_id={}", ctx.instance_id));
    ctx.log(format!("api_addr={}", ctx.api_addr));
    ctx.log(format!("space_id={:?}", ctx.space_id));
    ctx.log(format!("space_kind={:?}", ctx.space_kind));
    ctx.log(format!("space_name={:?}", ctx.space_name));
    ctx.log(format!("connected_members={:?}", ctx.connected_members));
    ctx.log(format!("log_path={}", ctx.log_path.display()));

    loop {
        if let Err(err) = run_probe(&ctx) {
            ctx.log(format!("probe error: {err:#}"));
            thread::sleep(Duration::from_secs(2));
        }
    }
}

fn run_probe(ctx: &ProbeContext) -> Result<()> {
    let status = ctx.api_request(&json!({ "cmd": "status" }))?;
    ensure_ok(&status, "status")?;

    let device_id = status["data"]["device_id"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let device_name = status["data"]["device_name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    ctx.log(format!("core status ok: device={device_name} | {device_id}"));

    let Some(space_id) = ctx.space_id.clone() else {
        ctx.log("LOCALLINK_SPACE_ID is missing; probe is not running in a space context. Idling.");
        loop {
            thread::sleep(Duration::from_secs(10));
        }
    };

    let mut seq = 0u64;
    let mut next_send = Instant::now();

    loop {
        if Instant::now() >= next_send {
            seq = seq.saturating_add(1);
            send_probe_ping(ctx, &device_id, &device_name, &space_id, seq)?;
            next_send = Instant::now() + SEND_INTERVAL;
        }

        poll_probe_events(ctx, &device_id, &device_name, &space_id)?;
    }
}

fn send_probe_ping(
    ctx: &ProbeContext,
    device_id: &str,
    device_name: &str,
    space_id: &str,
    seq: u64,
) -> Result<()> {
    let payload = ProbePayload {
        message_type: "probe_ping".to_string(),
        ts_ms: now_ms(),
        seq,
        origin_instance_id: ctx.instance_id.clone(),
        origin_device_id: device_id.to_string(),
        origin_device_name: device_name.to_string(),
        space_id: Some(space_id.to_string()),
        note: "space probe ping".to_string(),
    };

    let data_b64 = STANDARD.encode(serde_json::to_vec(&payload)?);
    let req = json!({
        "cmd": "send_space_message",
        "space_id": space_id,
        "service": SERVICE,
        "data_b64": data_b64
    });

    let resp = ctx.api_request(&req)?;
    if resp["ok"].as_bool().unwrap_or(false) {
        ctx.log(format!(
            "sent probe_ping seq={} response={}",
            seq,
            compact_json(&resp["data"])
        ));
    } else {
        ctx.log(format!("send_space_message failed: {}", resp["error"]));
    }

    Ok(())
}

fn poll_probe_events(
    ctx: &ProbeContext,
    device_id: &str,
    device_name: &str,
    space_id: &str,
) -> Result<()> {
    let req = json!({
        "cmd": "wait_events",
        "service": SERVICE,
        "consumer_id": ctx.consumer_id(),
        "wait_ms": 1000,
        "max_events": 25
    });

    let resp = ctx.api_request(&req)?;
    ensure_ok(&resp, "wait_events")?;

    let Some(events) = resp["data"].as_array() else {
        return Ok(());
    };

    for event in events {
        handle_probe_event(ctx, device_id, device_name, space_id, event)?;
    }

    Ok(())
}

fn handle_probe_event(
    ctx: &ProbeContext,
    device_id: &str,
    device_name: &str,
    expected_space_id: &str,
    event: &Value,
) -> Result<()> {
    let kind = event["kind"].as_str().unwrap_or("unknown");
    let peer_id = event["peer_id"].as_str().unwrap_or("unknown-peer");
    let event_space_id = event.get("space_id").and_then(|value| value.as_str());

    ctx.log(format!(
        "event kind={} peer={} event_space={:?} expected_space={}",
        kind, peer_id, event_space_id, expected_space_id
    ));

    if kind != "space_service_data" {
        ctx.log("ignored event: not space_service_data");
        return Ok(());
    }

    if event_space_id != Some(expected_space_id) {
        ctx.log("ignored event: space_id mismatch");
        return Ok(());
    }

    let Some(data_b64) = event["data_b64"].as_str() else {
        ctx.log("ignored event: missing data_b64");
        return Ok(());
    };

    let bytes = STANDARD.decode(data_b64).context("decode probe event data_b64")?;
    let payload: ProbePayload = serde_json::from_slice(&bytes).context("parse probe payload")?;

    if payload.origin_instance_id == ctx.instance_id {
        ctx.log(format!("ignored own {} seq={}", payload.message_type, payload.seq));
        return Ok(());
    }

    ctx.log(format!(
        "accepted {} seq={} from {} | instance={} | note={}",
        payload.message_type,
        payload.seq,
        payload.origin_device_name,
        payload.origin_instance_id,
        payload.note
    ));

    if payload.message_type == "probe_ping" {
        send_probe_pong(ctx, device_id, device_name, expected_space_id, &payload)?;
    }

    Ok(())
}

fn send_probe_pong(
    ctx: &ProbeContext,
    device_id: &str,
    device_name: &str,
    space_id: &str,
    ping: &ProbePayload,
) -> Result<()> {
    let payload = ProbePayload {
        message_type: "probe_pong".to_string(),
        ts_ms: now_ms(),
        seq: ping.seq,
        origin_instance_id: ctx.instance_id.clone(),
        origin_device_id: device_id.to_string(),
        origin_device_name: device_name.to_string(),
        space_id: Some(space_id.to_string()),
        note: format!("reply to {} seq {}", ping.origin_instance_id, ping.seq),
    };

    let data_b64 = STANDARD.encode(serde_json::to_vec(&payload)?);
    let req = json!({
        "cmd": "send_space_message",
        "space_id": space_id,
        "service": SERVICE,
        "data_b64": data_b64
    });

    let resp = ctx.api_request(&req)?;
    if resp["ok"].as_bool().unwrap_or(false) {
        ctx.log(format!(
            "sent probe_pong seq={} response={}",
            ping.seq,
            compact_json(&resp["data"])
        ));
    } else {
        ctx.log(format!("send_space_message pong failed: {}", resp["error"]));
    }

    Ok(())
}

impl ProbeContext {
    fn from_env() -> Result<Self> {
        let api_addr = std::env::var("LOCALLINK_CORE_API_ADDR")
            .unwrap_or_else(|_| DEFAULT_LOCAL_API_ADDR.to_string());
        let addon_id = std::env::var("LOCALLINK_ADDON_ID").unwrap_or_else(|_| "space-probe".to_string());
        let instance_id = std::env::var("LOCALLINK_ADDON_INSTANCE_ID")
            .unwrap_or_else(|_| format!("manual:{}", now_ms()));
        let space_id = env_non_empty("LOCALLINK_SPACE_ID");
        let space_kind = env_non_empty("LOCALLINK_SPACE_KIND");
        let space_name = env_non_empty("LOCALLINK_SPACE_NAME");
        let connected_members = std::env::var("LOCALLINK_CONNECTED_MEMBERS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|member| !member.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        let log_path = probe_log_path(&instance_id)?;

        Ok(Self {
            api_addr,
            addon_id,
            instance_id,
            space_id,
            space_kind,
            space_name,
            connected_members,
            log_path,
        })
    }

    fn consumer_id(&self) -> String {
        format!("space-probe:{}", self.instance_id)
    }

    fn api_request(&self, req: &Value) -> Result<Value> {
        let mut stream = TcpStream::connect(&self.api_addr)
            .with_context(|| format!("could not connect to LocalLink Core API at {}", self.api_addr))?;

        let line = serde_json::to_string(req)?;
        stream.write_all(line.as_bytes())?;
        stream.write_all(b"\n")?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut response = String::new();
        reader.read_line(&mut response)?;

        if response.trim().is_empty() {
            bail!("empty response from LocalLink Core API");
        }

        Ok(serde_json::from_str(&response)?)
    }

    fn log(&self, msg: impl AsRef<str>) {
        log_line(&self.log_path, msg.as_ref());
    }
}

fn ensure_ok(value: &Value, label: &str) -> Result<()> {
    if value["ok"].as_bool().unwrap_or(false) {
        Ok(())
    } else {
        bail!("{label} failed: {}", value["error"])
    }
}

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn probe_log_path(instance_id: &str) -> Result<PathBuf> {
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("APPDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());

    let dir = base.join("LocalLink").join("logs");
    fs::create_dir_all(&dir)?;
    Ok(dir.join(format!("space-probe-{}.log", safe_filename(instance_id))))
}

fn safe_filename(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' })
        .collect()
}

fn log_line(path: &Path, msg: &str) {
    let line = format!("[{}] {}\n", now_ms(), msg);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "<json error>".to_string())
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
