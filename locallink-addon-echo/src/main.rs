use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

const LOCAL_API_ADDR: &str = "127.0.0.1:47900";
const SERVICE_IN: &str = "test.echo";
const SERVICE_OUT: &str = "test.echo.reply";

fn main() -> Result<()> {
    println!("LocalLink Echo Addon");
    println!("Listening for service: {SERVICE_IN}");
    println!("Replies on service:   {SERVICE_OUT}");
    println!();

    loop {
        match run_once() {
            Ok(()) => {}
            Err(err) => {
                eprintln!("Addon error: {err}");
                thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

fn run_once() -> Result<()> {
    let req = json!({
        "cmd": "wait_events",
        "service": SERVICE_IN,
        "wait_ms": 30000,
        "max_events": 25
    });

    let resp = api_request(&req)?;

    if !resp["ok"].as_bool().unwrap_or(false) {
        bail!("API error: {}", resp["error"]);
    }

    let Some(events) = resp["data"].as_array() else {
        bail!("API response did not contain data array");
    };

    for event in events {
        handle_event(event)?;
    }

    Ok(())
}

fn handle_event(event: &Value) -> Result<()> {
    let kind = event["kind"].as_str().unwrap_or("unknown");
    let peer_id = event["peer_id"].as_str().context("event missing peer_id")?;
    let peer_name = event["peer_name"].as_str().unwrap_or("unknown-peer");

    let text = match event["data_b64"].as_str() {
        Some(data_b64) => {
            let bytes = STANDARD
                .decode(data_b64)
                .context("event data_b64 was invalid")?;
            String::from_utf8_lossy(&bytes).to_string()
        }
        None => String::new(),
    };

    println!("Event from {peer_name} | {peer_id}");
    println!("  kind: {kind}");
    println!("  text: {text}");
    println!();

    if kind == "service_data" {
        let reply = format!("Echo addon on this device received: {text}");

        let send_req = json!({
            "cmd": "send_message",
            "peer_id": peer_id,
            "service": SERVICE_OUT,
            "data_b64": STANDARD.encode(reply.as_bytes())
        });

        let send_resp = api_request(&send_req)?;

        if !send_resp["ok"].as_bool().unwrap_or(false) {
            eprintln!("Failed to send echo reply: {}", send_resp["error"]);
        } else {
            println!("Sent echo reply to {peer_name}");
            println!();
        }
    }

    Ok(())
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
