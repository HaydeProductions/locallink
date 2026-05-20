use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

const LOCAL_API_ADDR: &str = "127.0.0.1:47900";

pub fn run_api_client(args: &[String]) -> Result<()> {
    if args.is_empty() || args[0] == "help" || args[0] == "--help" {
        print_help();
        return Ok(());
    }

    let cmd = args[0].as_str();

    let request = match cmd {
        "status" => json!({ "cmd": "status" }),
        "paths" => json!({ "cmd": "paths" }),
        "shutdown" => json!({ "cmd": "shutdown" }),
        "peers" | "list_peers" => json!({ "cmd": "list_peers" }),
        "connections" | "list_connections" => json!({ "cmd": "list_connections" }),
        "addons" | "list_addons" => json!({ "cmd": "list_addons" }),
        "reload-addons" | "reload_addons" => json!({ "cmd": "reload_addons" }),
        "trusted" | "list_trusted_devices" => json!({ "cmd": "list_trusted_devices" }),

        "trust-mac" | "add_trusted_device" => {
            let mac = required_arg(args, "--mac")?;
            let name = optional_arg(args, "--name").unwrap_or_else(|| mac.clone());

            json!({
                "cmd": "add_trusted_device",
                "mac": mac,
                "name": name
            })
        }

        "remove-trusted-mac" | "remove_trusted_device" => {
            let mac = required_arg(args, "--mac")?;
            json!({
                "cmd": "remove_trusted_device",
                "mac": mac
            })
        }

        "connect" | "connect_device" => {
            let mut req = json!({
                "cmd": "connect_device"
            });

            if let Some(peer) = optional_arg(args, "--peer") {
                req["peer_id"] = json!(peer);
            }

            if let Some(mac) = optional_arg(args, "--mac") {
                req["mac"] = json!(mac);
            }

            req
        }

        "send" | "send_message" => {
            let peer = required_arg(args, "--peer")?;
            let service = required_arg(args, "--service")?;
            let data_b64 = data_arg(args)?;

            json!({
                "cmd": "send_message",
                "peer_id": peer,
                "service": service,
                "data_b64": data_b64
            })
        }

        "poll" | "poll_events" => {
            let service = optional_arg(args, "--service");
            let max_events = optional_arg(args, "--max")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(100);

            let mut req = json!({
                "cmd": "poll_events",
                "max_events": max_events
            });

            if let Some(service) = service {
                req["service"] = json!(service);
            }

            req
        }

        "wait" | "wait_events" => {
            let service = optional_arg(args, "--service");
            let max_events = optional_arg(args, "--max")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(100);
            let wait_ms = optional_arg(args, "--wait-ms")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(30_000);

            let mut req = json!({
                "cmd": "wait_events",
                "max_events": max_events,
                "wait_ms": wait_ms
            });

            if let Some(service) = service {
                req["service"] = json!(service);
            }

            req
        }

        "open" | "open_channel" => {
            let peer = required_arg(args, "--peer")?;
            let service = required_arg(args, "--service")?;

            json!({
                "cmd": "open_channel",
                "peer_id": peer,
                "service": service
            })
        }

        "channel-send" | "channel_send" => {
            let peer = required_arg(args, "--peer")?;
            let service = required_arg(args, "--service")?;
            let channel = required_arg(args, "--channel")?;
            let data_b64 = data_arg(args)?;

            json!({
                "cmd": "channel_send",
                "peer_id": peer,
                "service": service,
                "channel_id": channel,
                "data_b64": data_b64
            })
        }

        "channel-close" | "channel_close" => {
            let peer = required_arg(args, "--peer")?;
            let service = required_arg(args, "--service")?;
            let channel = required_arg(args, "--channel")?;
            let reason = optional_arg(args, "--reason").unwrap_or_else(|| "normal".to_string());

            json!({
                "cmd": "channel_close",
                "peer_id": peer,
                "service": service,
                "channel_id": channel,
                "reason": reason
            })
        }

        other => bail!("unknown API helper command: {other}"),
    };

    let response = send_request(&request)?;
    print_pretty_json(&response)?;
    Ok(())
}

fn send_request(request: &Value) -> Result<Value> {
    let mut stream = TcpStream::connect(LOCAL_API_ADDR).with_context(|| {
        format!("could not connect to LocalLink core API at {LOCAL_API_ADDR}; is the core running?")
    })?;

    let line = serde_json::to_string(request)?;
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;

    if response.trim().is_empty() {
        bail!("empty response from LocalLink core API");
    }

    let value: Value = serde_json::from_str(&response)?;
    Ok(value)
}

fn print_pretty_json(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn required_arg(args: &[String], name: &str) -> Result<String> {
    optional_arg(args, name).ok_or_else(|| anyhow::anyhow!("missing required argument {name}"))
}

fn optional_arg(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

fn data_arg(args: &[String]) -> Result<String> {
    if let Some(data_b64) = optional_arg(args, "--data-b64") {
        STANDARD
            .decode(&data_b64)
            .context("--data-b64 was not valid base64")?;
        return Ok(data_b64);
    }

    if let Some(text) = optional_arg(args, "--text") {
        return Ok(STANDARD.encode(text.as_bytes()));
    }

    bail!("missing data argument; use either --text \"hello\" or --data-b64 BASE64")
}

fn print_help() {
    println!("LocalLink API helper");
    println!();
    println!("Usage:");
    println!("  locallink-core --api status");
    println!("  locallink-core --api peers");
    println!("  locallink-core --api trusted");
    println!("  locallink-core --api trust-mac --mac aa:bb:cc:dd:ee:ff --name \"My Laptop\"");
    println!("  locallink-core --api connect --mac aa:bb:cc:dd:ee:ff");
    println!("  locallink-core --api connections");
    println!("  locallink-core --api addons");
}
