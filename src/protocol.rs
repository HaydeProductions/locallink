use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::Mutex;

pub const APP_NAME: &str = "locallink";
pub const PROTOCOL_VERSION: u32 = 1;

pub const DISCOVERY_PORT: u16 = 47777;
pub const TCP_PORT: u16 = 47800;
pub const MULTICAST_ADDR: &str = "ff02::114";

pub const FRAME_HELLO: u8 = 1;
pub const FRAME_PING: u8 = 2;
pub const FRAME_PONG: u8 = 3;
pub const FRAME_BENCH_START: u8 = 4;
pub const FRAME_BENCH_DATA: u8 = 5;
pub const FRAME_BENCH_END: u8 = 6;
pub const FRAME_AUTH_CHALLENGE: u8 = 7;
pub const FRAME_AUTH_RESPONSE: u8 = 8;
pub const FRAME_ENCRYPTED: u8 = 9;
pub const FRAME_SERVICE_DATA: u8 = 10;
pub const FRAME_CHANNEL_OPEN: u8 = 11;
pub const FRAME_CHANNEL_DATA: u8 = 12;
pub const FRAME_CHANNEL_CLOSE: u8 = 13;

pub const MAX_FRAME_PAYLOAD: usize = 8 * 1024 * 1024 + 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryMessage {
    pub app: String,
    pub version: u32,
    pub kind: String,
    pub device_id: String,
    pub device_name: String,
    pub tcp_port: u16,

    #[serde(default)]
    pub macs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloPayload {
    pub app: String,
    pub version: u32,
    pub device_id: String,
    pub device_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthChallenge {
    pub nonce_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub device_id: String,
    pub hmac_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceData {
    pub service: String,
    pub message_id: String,
    pub data_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelOpen {
    pub service: String,
    pub channel_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelData {
    pub service: String,
    pub channel_id: String,
    pub message_id: String,
    pub data_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelClose {
    pub service: String,
    pub channel_id: String,
    pub message_id: String,
    pub reason: String,
}

#[derive(Debug)]
pub struct Frame {
    pub kind: u8,
    pub seq: u64,
    pub payload: Vec<u8>,
}

pub async fn read_frame(reader: &mut OwnedReadHalf) -> Result<Frame> {
    let mut header = [0u8; 13];
    reader.read_exact(&mut header).await?;

    let kind = header[0];

    let mut seq_bytes = [0u8; 8];
    seq_bytes.copy_from_slice(&header[1..9]);
    let seq = u64::from_be_bytes(seq_bytes);

    let mut len_bytes = [0u8; 4];
    len_bytes.copy_from_slice(&header[9..13]);
    let len = u32::from_be_bytes(len_bytes) as usize;

    if len > MAX_FRAME_PAYLOAD {
        bail!("frame too large: {len} bytes");
    }

    let mut payload = vec![0u8; len];

    if len > 0 {
        reader.read_exact(&mut payload).await?;
    }

    Ok(Frame { kind, seq, payload })
}

pub async fn write_frame(
    writer: &Arc<Mutex<OwnedWriteHalf>>,
    kind: u8,
    seq: u64,
    payload: &[u8],
) -> Result<()> {
    if payload.len() > MAX_FRAME_PAYLOAD {
        bail!("payload too large: {} bytes", payload.len());
    }

    let mut frame = Vec::with_capacity(13 + payload.len());
    frame.push(kind);
    frame.extend_from_slice(&seq.to_be_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);

    let mut guard = writer.lock().await;
    guard.write_all(&frame).await?;
    Ok(())
}
