use crate::config::{psk_bytes, Config};
use crate::protocol::{
    read_frame, write_frame, AuthChallenge, AuthResponse, ChannelClose, ChannelData, ChannelOpen,
    HelloPayload, ServiceData, APP_NAME, FRAME_AUTH_CHALLENGE, FRAME_AUTH_RESPONSE,
    FRAME_BENCH_DATA, FRAME_BENCH_END, FRAME_BENCH_START, FRAME_CHANNEL_CLOSE, FRAME_CHANNEL_DATA,
    FRAME_CHANNEL_OPEN, FRAME_ENCRYPTED, FRAME_HELLO, FRAME_PING, FRAME_PONG, FRAME_SERVICE_DATA,
    PROTOCOL_VERSION, TCP_PORT,
};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chacha20poly1305::{
    aead::{Aead, Payload},
    ChaCha20Poly1305, KeyInit, Nonce,
};
use hmac::{Hmac, Mac};
use rand::{rngs::OsRng, RngCore};
use serde::Serialize;
use sha2::Sha256;
use std::collections::{HashMap, VecDeque};
use std::net::{Ipv6Addr, SocketAddrV6};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout, Duration, Instant};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub bench: bool,
}

#[derive(Debug, Clone)]
pub struct SessionCrypto {
    key: [u8; 32],
    send_dir: u32,
    recv_dir: u32,
}

#[derive(Clone)]
pub struct ConnectedPeer {
    pub device_id: String,
    pub device_name: String,
    pub addr: String,
    pub connected_since: Instant,
    pub last_seen: Arc<Mutex<Instant>>,
    pub writer: Arc<Mutex<OwnedWriteHalf>>,
    pub crypto: SessionCrypto,
    pub send_seq: Arc<Mutex<u64>>,
}

pub type ConnectionRegistry = Arc<Mutex<HashMap<String, ConnectedPeer>>>;

#[derive(Debug, Clone, Serialize)]
pub struct ApiEvent {
    pub kind: String,
    pub peer_id: String,
    pub peer_name: String,
    pub service: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_b64: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    pub received_ms: u128,
}

pub type EventQueue = Arc<Mutex<VecDeque<ApiEvent>>>;

impl SessionCrypto {
    fn cipher(&self) -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new_from_slice(&self.key).expect("valid key length")
    }

    fn nonce(dir: u32, seq: u64) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&dir.to_be_bytes());
        nonce[4..].copy_from_slice(&seq.to_be_bytes());
        nonce
    }

    fn encrypt(&self, seq: u64, inner_kind: u8, payload: &[u8]) -> Result<Vec<u8>> {
        let mut plaintext = Vec::with_capacity(1 + payload.len());
        plaintext.push(inner_kind);
        plaintext.extend_from_slice(payload);

        let nonce_bytes = Self::nonce(self.send_dir, seq);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let aad = seq.to_be_bytes();

        let ciphertext = self
            .cipher()
            .encrypt(
                nonce,
                Payload {
                    msg: &plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| anyhow::anyhow!("encryption failed"))?;

        Ok(ciphertext)
    }

    fn decrypt(&self, seq: u64, ciphertext: &[u8]) -> Result<(u8, Vec<u8>)> {
        let nonce_bytes = Self::nonce(self.recv_dir, seq);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let aad = seq.to_be_bytes();

        let plaintext = self
            .cipher()
            .decrypt(
                nonce,
                Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| anyhow::anyhow!("decryption failed"))?;

        if plaintext.is_empty() {
            bail!("empty encrypted frame");
        }

        Ok((plaintext[0], plaintext[1..].to_vec()))
    }
}

pub async fn tcp_server(
    cfg: Config,
    opts: RunOptions,
    connections: ConnectionRegistry,
    events: EventQueue,
) -> Result<()> {
    let listener =
        TcpListener::bind(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, TCP_PORT, 0, 0)).await?;

    println!("TCP listener started on [::]:{TCP_PORT}");

    loop {
        let (stream, addr) = listener.accept().await?;
        let cfg_clone = cfg.clone();
        let opts_clone = opts.clone();
        let connections_clone = connections.clone();
        let events_clone = events.clone();

        println!("Incoming TCP connection from {addr}");

        tokio::spawn(async move {
            if let Err(err) = handle_connection(
                cfg_clone,
                opts_clone,
                stream,
                "inbound",
                connections_clone,
                events_clone,
            )
            .await
            {
                eprintln!("Inbound connection ended: {err}");
            }
        });
    }
}

pub async fn connect_to_peer(
    cfg: Config,
    opts: RunOptions,
    peer_addr: SocketAddrV6,
    peer_id: String,
    connections: ConnectionRegistry,
    events: EventQueue,
) {
    sleep(Duration::from_millis(250)).await;

    match TcpStream::connect(peer_addr).await {
        Ok(stream) => {
            if let Err(err) =
                handle_connection(cfg, opts, stream, "outbound", connections, events).await
            {
                eprintln!("Outbound connection to {peer_id} ended: {err}");
            }
        }
        Err(err) => {
            eprintln!("TCP connect failed to {peer_addr}: {err}");
        }
    }
}

fn random_nonce() -> [u8; 32] {
    let mut nonce = [0u8; 32];
    OsRng.fill_bytes(&mut nonce);
    nonce
}

fn auth_hmac(psk: &[u8], nonce: &[u8], responder_device_id: &str) -> Result<Vec<u8>> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(psk)?;
    mac.update(b"locallink-auth-response-v1");
    mac.update(nonce);
    mac.update(responder_device_id.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

fn derive_session_crypto(
    psk: &[u8],
    my_nonce: &[u8; 32],
    peer_nonce: &[u8; 32],
    my_device_id: &str,
    peer_device_id: &str,
) -> Result<SessionCrypto> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(psk)?;
    mac.update(b"locallink-session-key-v1");

    if my_device_id < peer_device_id {
        mac.update(my_device_id.as_bytes());
        mac.update(peer_device_id.as_bytes());
        mac.update(my_nonce);
        mac.update(peer_nonce);
    } else {
        mac.update(peer_device_id.as_bytes());
        mac.update(my_device_id.as_bytes());
        mac.update(peer_nonce);
        mac.update(my_nonce);
    }

    let derived = mac.finalize().into_bytes();

    let mut key = [0u8; 32];
    key.copy_from_slice(&derived[..32]);

    let my_is_low = my_device_id < peer_device_id;

    Ok(SessionCrypto {
        key,
        send_dir: if my_is_low { 0 } else { 1 },
        recv_dir: if my_is_low { 1 } else { 0 },
    })
}

async fn next_seq(send_seq: &Arc<Mutex<u64>>) -> u64 {
    let mut guard = send_seq.lock().await;
    let seq = *guard;
    *guard += 1;
    seq
}

async fn write_secure_frame(
    writer: &Arc<Mutex<OwnedWriteHalf>>,
    crypto: &SessionCrypto,
    send_seq: &Arc<Mutex<u64>>,
    inner_kind: u8,
    payload: &[u8],
) -> Result<()> {
    let seq = next_seq(send_seq).await;
    let ciphertext = crypto.encrypt(seq, inner_kind, payload)?;
    write_frame(writer, FRAME_ENCRYPTED, seq, &ciphertext).await
}

async fn send_encrypted_payload(
    connections: ConnectionRegistry,
    peer_id: &str,
    inner_kind: u8,
    payload: &[u8],
) -> Result<()> {
    let conn = {
        let guard = connections.lock().await;
        guard
            .get(peer_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("not connected to peer {peer_id}"))?
    };

    let send_result = timeout(
        Duration::from_secs(3),
        write_secure_frame(
            &conn.writer,
            &conn.crypto,
            &conn.send_seq,
            inner_kind,
            payload,
        ),
    )
    .await;

    match send_result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            connections.lock().await.remove(peer_id);
            Err(anyhow::anyhow!(
                "failed to write to peer; connection removed: {e}"
            ))
        }
        Err(_) => {
            connections.lock().await.remove(peer_id);
            Err(anyhow::anyhow!(
                "timed out writing to peer; stale connection removed"
            ))
        }
    }
}

pub async fn disconnect_peer(connections: ConnectionRegistry, peer_id: &str) -> Result<bool> {
    let conn = {
        let mut guard = connections.lock().await;
        guard.remove(peer_id)
    };

    if let Some(conn) = conn {
        let mut writer = conn.writer.lock().await;
        let _ = writer.shutdown().await;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub async fn send_service_message(
    connections: ConnectionRegistry,
    peer_id: &str,
    service: &str,
    data_b64: &str,
) -> Result<String> {
    STANDARD
        .decode(data_b64)
        .context("data_b64 was not valid base64")?;

    let message = ServiceData {
        service: service.to_string(),
        message_id: Uuid::new_v4().to_string(),
        data_b64: data_b64.to_string(),
    };

    let payload = serde_json::to_vec(&message)?;
    send_encrypted_payload(connections, peer_id, FRAME_SERVICE_DATA, &payload).await?;

    Ok(message.message_id)
}

pub async fn open_channel(
    connections: ConnectionRegistry,
    peer_id: &str,
    service: &str,
) -> Result<String> {
    let channel = ChannelOpen {
        service: service.to_string(),
        channel_id: Uuid::new_v4().to_string(),
    };

    let payload = serde_json::to_vec(&channel)?;
    send_encrypted_payload(connections, peer_id, FRAME_CHANNEL_OPEN, &payload).await?;

    Ok(channel.channel_id)
}

pub async fn send_channel_data(
    connections: ConnectionRegistry,
    peer_id: &str,
    service: &str,
    channel_id: &str,
    data_b64: &str,
) -> Result<String> {
    STANDARD
        .decode(data_b64)
        .context("data_b64 was not valid base64")?;

    let msg = ChannelData {
        service: service.to_string(),
        channel_id: channel_id.to_string(),
        message_id: Uuid::new_v4().to_string(),
        data_b64: data_b64.to_string(),
    };

    let payload = serde_json::to_vec(&msg)?;
    send_encrypted_payload(connections, peer_id, FRAME_CHANNEL_DATA, &payload).await?;

    Ok(msg.message_id)
}

pub async fn close_channel(
    connections: ConnectionRegistry,
    peer_id: &str,
    service: &str,
    channel_id: &str,
    reason: &str,
) -> Result<String> {
    let msg = ChannelClose {
        service: service.to_string(),
        channel_id: channel_id.to_string(),
        message_id: Uuid::new_v4().to_string(),
        reason: reason.to_string(),
    };

    let payload = serde_json::to_vec(&msg)?;
    send_encrypted_payload(connections, peer_id, FRAME_CHANNEL_CLOSE, &payload).await?;

    Ok(msg.message_id)
}

async fn register_connection(
    connections: &ConnectionRegistry,
    peer_id: &str,
    peer_name: &str,
    peer_addr: &str,
    writer: Arc<Mutex<OwnedWriteHalf>>,
    crypto: SessionCrypto,
    send_seq: Arc<Mutex<u64>>,
) {
    let conn = ConnectedPeer {
        device_id: peer_id.to_string(),
        device_name: peer_name.to_string(),
        addr: peer_addr.to_string(),
        connected_since: Instant::now(),
        last_seen: Arc::new(Mutex::new(Instant::now())),
        writer,
        crypto,
        send_seq,
    };

    connections.lock().await.insert(peer_id.to_string(), conn);
}

pub async fn handle_connection(
    cfg: Config,
    opts: RunOptions,
    stream: TcpStream,
    direction: &'static str,
    connections: ConnectionRegistry,
    events: EventQueue,
) -> Result<()> {
    let psk = psk_bytes(&cfg)?;

    let peer_addr = stream.peer_addr()?;
    let peer_addr_string = peer_addr.to_string();

    let (mut reader, writer_half) = stream.into_split();
    let writer = Arc::new(Mutex::new(writer_half));
    let send_seq = Arc::new(Mutex::new(1u64));

    let hello = HelloPayload {
        app: APP_NAME.to_string(),
        version: PROTOCOL_VERSION,
        device_id: cfg.device_id.clone(),
        device_name: cfg.device_name.clone(),
    };

    let hello_bytes = serde_json::to_vec(&hello)?;
    write_frame(&writer, FRAME_HELLO, 0, &hello_bytes).await?;

    let my_nonce = random_nonce();
    let challenge = AuthChallenge {
        nonce_b64: STANDARD.encode(my_nonce),
    };

    write_frame(
        &writer,
        FRAME_AUTH_CHALLENGE,
        0,
        &serde_json::to_vec(&challenge)?,
    )
    .await?;

    println!("Connection active [{direction}] with {peer_addr}");
    println!("Waiting for PSK authentication...");

    let mut peer_name = "unknown".to_string();
    let mut peer_device_id: Option<String> = None;
    let mut peer_nonce: Option<[u8; 32]> = None;

    let mut auth_ok = false;
    let mut crypto: Option<SessionCrypto> = None;
    let mut registered = false;
    let mut heartbeat_started = false;
    let mut benchmark_started = false;

    let mut bench_active = false;
    let mut bench_bytes: u64 = 0;
    let mut bench_start = Instant::now();

    loop {
        let frame = match read_frame(&mut reader).await {
            Ok(frame) => frame,
            Err(e) => {
                if let Some(peer_id) = peer_device_id.as_deref() {
                    connections.lock().await.remove(peer_id);
                    eprintln!("Removed disconnected peer from registry: {peer_name} | {peer_id}");
                }
                return Err(e);
            }
        };

        if frame.kind == FRAME_ENCRYPTED {
            let Some(ref crypto_state) = crypto else {
                bail!("received encrypted frame before secure session was established");
            };

            let (inner_kind, inner_payload) = crypto_state.decrypt(frame.seq, &frame.payload)?;

            if let Some(peer_id) = peer_device_id.as_deref() {
                if let Some(conn) = connections.lock().await.get(peer_id).cloned() {
                    *conn.last_seen.lock().await = Instant::now();
                }
            }

            handle_post_auth_frame(
                &writer,
                crypto_state,
                &send_seq,
                inner_kind,
                frame.seq,
                inner_payload,
                &peer_name,
                peer_device_id.as_deref().unwrap_or("unknown"),
                &mut bench_active,
                &mut bench_bytes,
                &mut bench_start,
                events.clone(),
            )
            .await?;

            continue;
        }

        match frame.kind {
            FRAME_HELLO => {
                let hello: HelloPayload = serde_json::from_slice(&frame.payload)?;

                if hello.device_id == cfg.device_id {
                    continue;
                }

                peer_name = hello.device_name.clone();
                peer_device_id = Some(hello.device_id.clone());

                println!(
                    "HELLO from {} | {} over {}",
                    hello.device_name, hello.device_id, peer_addr
                );
            }

            FRAME_AUTH_CHALLENGE => {
                let challenge: AuthChallenge = serde_json::from_slice(&frame.payload)?;
                let decoded = STANDARD
                    .decode(challenge.nonce_b64)
                    .context("invalid challenge nonce")?;

                if decoded.len() != 32 {
                    bail!("challenge nonce was not 32 bytes");
                }

                let mut nonce = [0u8; 32];
                nonce.copy_from_slice(&decoded);
                peer_nonce = Some(nonce);

                let proof = auth_hmac(&psk, &nonce, &cfg.device_id)?;

                let response = AuthResponse {
                    device_id: cfg.device_id.clone(),
                    hmac_b64: STANDARD.encode(proof),
                };

                write_frame(
                    &writer,
                    FRAME_AUTH_RESPONSE,
                    0,
                    &serde_json::to_vec(&response)?,
                )
                .await?;
            }

            FRAME_AUTH_RESPONSE => {
                let response: AuthResponse = serde_json::from_slice(&frame.payload)?;

                let received = STANDARD
                    .decode(response.hmac_b64)
                    .context("invalid auth response hmac")?;

                let expected = auth_hmac(&psk, &my_nonce, &response.device_id)?;

                if received != expected {
                    bail!("PSK authentication failed for {}", response.device_id);
                }

                auth_ok = true;

                if peer_device_id.is_none() {
                    peer_device_id = Some(response.device_id.clone());
                }

                println!("PSK authentication OK for {}", response.device_id);

                if crypto.is_none() {
                    if let (Some(peer_id), Some(peer_nonce_value)) =
                        (peer_device_id.as_deref(), peer_nonce.as_ref())
                    {
                        crypto = Some(derive_session_crypto(
                            &psk,
                            &my_nonce,
                            peer_nonce_value,
                            &cfg.device_id,
                            peer_id,
                        )?);

                        println!("Secure encrypted session established with {peer_name}");
                    }
                }

                if auth_ok && crypto.is_some() && !registered {
                    if let (Some(peer_id), Some(crypto_state)) =
                        (peer_device_id.as_deref(), crypto.clone())
                    {
                        registered = true;

                        register_connection(
                            &connections,
                            peer_id,
                            &peer_name,
                            &peer_addr_string,
                            writer.clone(),
                            crypto_state.clone(),
                            send_seq.clone(),
                        )
                        .await;

                        println!("Registered connected peer: {peer_name} | {peer_id}");
                    }
                }

                if crypto.is_some() && !heartbeat_started {
                    if let Some(peer_id) = peer_device_id.clone() {
                        heartbeat_started = true;
                        start_heartbeat(
                            writer.clone(),
                            crypto.clone().unwrap(),
                            send_seq.clone(),
                            peer_id,
                            peer_name.clone(),
                            connections.clone(),
                        );
                    }
                }

                if crypto.is_some() && direction == "outbound" && opts.bench && !benchmark_started {
                    benchmark_started = true;
                    let bench_writer = writer.clone();
                    let bench_crypto = crypto.clone().unwrap();
                    let bench_send_seq = send_seq.clone();

                    tokio::spawn(async move {
                        sleep(Duration::from_secs(2)).await;

                        if let Err(err) =
                            run_benchmark(bench_writer, bench_crypto, bench_send_seq).await
                        {
                            eprintln!("Benchmark failed: {err}");
                        }
                    });
                }
            }

            other => {
                println!("Ignoring unauthenticated plaintext frame kind {other} from {peer_addr}");
            }
        }
    }
}

async fn handle_post_auth_frame(
    writer: &Arc<Mutex<OwnedWriteHalf>>,
    crypto: &SessionCrypto,
    send_seq: &Arc<Mutex<u64>>,
    kind: u8,
    seq: u64,
    payload: Vec<u8>,
    peer_name: &str,
    peer_device_id: &str,
    bench_active: &mut bool,
    bench_bytes: &mut u64,
    bench_start: &mut Instant,
    events: EventQueue,
) -> Result<()> {
    match kind {
        FRAME_PING => {
            write_secure_frame(writer, crypto, send_seq, FRAME_PONG, &seq.to_be_bytes()).await?;
        }

        FRAME_PONG => {}

        FRAME_BENCH_START => {
            *bench_active = true;
            *bench_bytes = 0;
            *bench_start = Instant::now();
            println!("Encrypted benchmark started by {peer_name} ({peer_device_id})");
        }

        FRAME_BENCH_DATA => {
            if *bench_active {
                *bench_bytes += payload.len() as u64;
            }
        }

        FRAME_BENCH_END => {
            if *bench_active {
                let elapsed = bench_start.elapsed().as_secs_f64();
                let mb_s = (*bench_bytes as f64 / 1_048_576.0) / elapsed;
                let mbit_s = (*bench_bytes as f64 * 8.0 / 1_000_000.0) / elapsed;

                println!(
                    "Encrypted benchmark received from {peer_name}: {:.2} MB/s, {:.2} Mbit/s",
                    mb_s, mbit_s
                );

                *bench_active = false;
            }
        }

        FRAME_SERVICE_DATA => {
            let msg: ServiceData = serde_json::from_slice(&payload)?;
            println!(
                "Service message from {peer_name} service={} message_id={}",
                msg.service, msg.message_id
            );

            push_event(
                events,
                ApiEvent {
                    kind: "service_data".to_string(),
                    peer_id: peer_device_id.to_string(),
                    peer_name: peer_name.to_string(),
                    service: msg.service,
                    channel_id: None,
                    message_id: Some(msg.message_id),
                    data_b64: Some(msg.data_b64),
                    reason: None,
                    received_ms: now_ms(),
                },
            )
            .await;
        }

        FRAME_CHANNEL_OPEN => {
            let msg: ChannelOpen = serde_json::from_slice(&payload)?;
            println!(
                "Channel open from {peer_name} service={} channel_id={}",
                msg.service, msg.channel_id
            );

            push_event(
                events,
                ApiEvent {
                    kind: "channel_open".to_string(),
                    peer_id: peer_device_id.to_string(),
                    peer_name: peer_name.to_string(),
                    service: msg.service,
                    channel_id: Some(msg.channel_id),
                    message_id: None,
                    data_b64: None,
                    reason: None,
                    received_ms: now_ms(),
                },
            )
            .await;
        }

        FRAME_CHANNEL_DATA => {
            let msg: ChannelData = serde_json::from_slice(&payload)?;

            push_event(
                events,
                ApiEvent {
                    kind: "channel_data".to_string(),
                    peer_id: peer_device_id.to_string(),
                    peer_name: peer_name.to_string(),
                    service: msg.service,
                    channel_id: Some(msg.channel_id),
                    message_id: Some(msg.message_id),
                    data_b64: Some(msg.data_b64),
                    reason: None,
                    received_ms: now_ms(),
                },
            )
            .await;
        }

        FRAME_CHANNEL_CLOSE => {
            let msg: ChannelClose = serde_json::from_slice(&payload)?;
            println!(
                "Channel close from {peer_name} service={} channel_id={} reason={}",
                msg.service, msg.channel_id, msg.reason
            );

            push_event(
                events,
                ApiEvent {
                    kind: "channel_close".to_string(),
                    peer_id: peer_device_id.to_string(),
                    peer_name: peer_name.to_string(),
                    service: msg.service,
                    channel_id: Some(msg.channel_id),
                    message_id: Some(msg.message_id),
                    data_b64: None,
                    reason: Some(msg.reason),
                    received_ms: now_ms(),
                },
            )
            .await;
        }

        other => {
            println!("Unknown encrypted frame kind {other}");
        }
    }

    Ok(())
}

async fn push_event(events: EventQueue, event: ApiEvent) {
    let mut q = events.lock().await;
    q.push_back(event);

    while q.len() > 4096 {
        q.pop_front();
    }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn start_heartbeat(
    writer: Arc<Mutex<OwnedWriteHalf>>,
    crypto: SessionCrypto,
    send_seq: Arc<Mutex<u64>>,
    peer_id: String,
    peer_name: String,
    connections: ConnectionRegistry,
) {
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(5)).await;

            if let Err(err) = write_secure_frame(&writer, &crypto, &send_seq, FRAME_PING, &[]).await
            {
                eprintln!("Encrypted heartbeat stopped for {peer_name} | {peer_id}: {err}");
                connections.lock().await.remove(&peer_id);
                eprintln!("Removed stale connection for {peer_name} | {peer_id}");
                break;
            }
        }
    });
}

pub async fn run_benchmark(
    writer: Arc<Mutex<OwnedWriteHalf>>,
    crypto: SessionCrypto,
    send_seq: Arc<Mutex<u64>>,
) -> Result<()> {
    println!("Starting encrypted outbound benchmark for 10 seconds...");

    let payload = vec![0u8; 1024 * 1024];
    let start = Instant::now();
    let mut bytes_sent: u64 = 0;

    write_secure_frame(&writer, &crypto, &send_seq, FRAME_BENCH_START, &[]).await?;

    while start.elapsed() < Duration::from_secs(10) {
        write_secure_frame(&writer, &crypto, &send_seq, FRAME_BENCH_DATA, &payload).await?;
        bytes_sent += payload.len() as u64;
    }

    write_secure_frame(&writer, &crypto, &send_seq, FRAME_BENCH_END, &[]).await?;

    let elapsed = start.elapsed().as_secs_f64();
    let mb_s = (bytes_sent as f64 / 1_048_576.0) / elapsed;
    let mbit_s = (bytes_sent as f64 * 8.0 / 1_000_000.0) / elapsed;

    println!(
        "Encrypted benchmark sent: {:.2} MB/s, {:.2} Mbit/s",
        mb_s, mbit_s
    );

    Ok(())
}
