use anyhow::{bail, Context, Result};
use arboard::Clipboard;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const LOCAL_API_ADDR: &str = "127.0.0.1:47900";
const SERVICE: &str = "clipboard-sync";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClipboardPayload {
    ts_ms: u128,
    device_id: String,
    device_name: String,
    text: String,
}

#[derive(Debug, Clone)]
struct Peer {
    device_id: String,
    device_name: String,
}

fn main() -> Result<()> {
    println!("LocalLink Clipboard Sync Add-on");
    println!("Service: {SERVICE}");
    println!("Rule: newest clipboard timestamp wins");
    println!();

    loop {
        match run() {
            Ok(()) => {}
            Err(err) => {
                eprintln!("Clipboard add-on error: {err}");
                thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

fn run() -> Result<()> {
    let status = api_request(&json!({ "cmd": "status" }))?;
    let self_device_id = status["data"]["device_id"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let self_device_name = status["data"]["device_name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let mut clipboard = Clipboard::new().context("opening system clipboard")?;

    let mut last_text = clipboard.get_text().unwrap_or_default();
    let mut last_ts = now_ms();

    println!("Local device: {self_device_name} | {self_device_id}");
    println!("Initial clipboard length: {}", last_text.len());
    println!();

    loop {
        // 1. Receive remote clipboard updates.
        if let Err(err) = receive_remote_events(
            &mut clipboard,
            &mut last_text,
            &mut last_ts,
            &self_device_id,
        ) {
            eprintln!("receive error: {err}");
        }

        // 2. Detect local clipboard changes.
        match clipboard.get_text() {
            Ok(current_text) => {
                if current_text != last_text {
                    last_text = current_text.clone();
                    last_ts = now_ms();

                    let payload = ClipboardPayload {
                        ts_ms: last_ts,
                        device_id: self_device_id.clone(),
                        device_name: self_device_name.clone(),
                        text: current_text,
                    };

                    if let Err(err) = send_to_connected_peers(&payload) {
                        eprintln!("send error: {err}");
                    }
                }
            }
            Err(_) => {
                // Non-text clipboard content or temporary clipboard lock.
            }
        }

        thread::sleep(Duration::from_millis(350));
    }
}

fn receive_remote_events(
    clipboard: &mut Clipboard,
    last_text: &mut String,
    last_ts: &mut u128,
    self_device_id: &str,
) -> Result<()> {
    let resp = api_request(&json!({
        "cmd": "poll_events",
        "service": SERVICE,
        "max_events": 50
    }))?;

    if !resp["ok"].as_bool().unwrap_or(false) {
        bail!("poll_events failed: {}", resp["error"]);
    }

    let Some(events) = resp["data"].as_array() else {
        return Ok(());
    };

    for event in events {
        let Some(data_b64) = event["data_b64"].as_str() else {
            continue;
        };

        let decoded = STANDARD
            .decode(data_b64)
            .context("decoding remote clipboard event base64")?;

        let payload: ClipboardPayload =
            serde_json::from_slice(&decoded).context("parsing clipboard payload")?;

        // Ignore our own messages if they ever loop back.
        if payload.device_id == self_device_id {
            continue;
        }

        // Newest wins.
        if payload.ts_ms > *last_ts && payload.text != *last_text {
            clipboard
                .set_text(payload.text.clone())
                .context("setting clipboard text")?;

            *last_text = payload.text.clone();
            *last_ts = payload.ts_ms;

            println!(
                "Applied clipboard from {} | {} bytes",
                payload.device_name,
                payload.text.len()
            );
        }
    }

    Ok(())
}

fn send_to_connected_peers(payload: &ClipboardPayload) -> Result<()> {
    let peers = connected_peers()?;

    if peers.is_empty() {
        return Ok(());
    }

    let payload_json = serde_json::to_vec(payload)?;
    let data_b64 = STANDARD.encode(payload_json);

    for peer in peers {
        let resp = api_request(&json!({
            "cmd": "send_message",
            "peer_id": peer.device_id,
            "service": SERVICE,
            "data_b64": data_b64
        }));

        match resp {
            Ok(value) => {
                if value["ok"].as_bool().unwrap_or(false) {
                    println!(
                        "Sent clipboard to {} | {} bytes",
                        peer.device_name,
                        payload.text.len()
                    );
                } else {
                    eprintln!("send_message failed: {}", value["error"]);
                }
            }
            Err(err) => {
                eprintln!("send_message error for {}: {err}", peer.device_name);
            }
        }
    }

    Ok(())
}

fn connected_peers() -> Result<Vec<Peer>> {
    let resp = api_request(&json!({ "cmd": "list_connections" }))?;

    if !resp["ok"].as_bool().unwrap_or(false) {
        bail!("list_connections failed: {}", resp["error"]);
    }

    let Some(rows) = resp["data"].as_array() else {
        return Ok(Vec::new());
    };

    let mut peers = Vec::new();

    for row in rows {
        let Some(device_id) = row["device_id"].as_str() else {
            continue;
        };

        let device_name = row["device_name"].as_str().unwrap_or("unknown");

        peers.push(Peer {
            device_id: device_id.to_string(),
            device_name: device_name.to_string(),
        });
    }

    Ok(peers)
}

fn api_request(req: &Value) -> Result<Value> {
    let mut stream = TcpStream::connect(LOCAL_API_ADDR)
        .with_context(|| format!("could not connect to LocalLink Core API at {LOCAL_API_ADDR}"))?;

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

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
