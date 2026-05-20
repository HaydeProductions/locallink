use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use fs2::FileExt;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub device_id: String,
    pub device_name: String,

    #[serde(default)]
    pub psk_b64: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedDevice {
    pub name: String,
    pub macs: Vec<String>,

    #[serde(default)]
    pub device_id: Option<String>,

    #[serde(default)]
    pub blocked: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppPaths {
    pub app_dir: String,
    pub config_file: String,
    pub trusted_peers_file: String,
    pub trusted_devices_file: String,
    pub addons_dir: String,
    pub logs_dir: String,
    pub runtime_dir: String,
    pub state_dir: String,
    pub lock_file: String,
}

pub fn app_dir() -> Result<PathBuf> {
    let appdata = std::env::var("APPDATA").context("APPDATA environment variable not found")?;
    Ok(PathBuf::from(appdata).join("LocalLink"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("config.json"))
}

pub fn trusted_peers_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("trusted-peers.json"))
}

pub fn trusted_devices_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("trusted-devices.json"))
}

pub fn addons_dir() -> Result<PathBuf> {
    Ok(app_dir()?.join("addons"))
}

pub fn logs_dir() -> Result<PathBuf> {
    Ok(app_dir()?.join("logs"))
}

pub fn runtime_dir() -> Result<PathBuf> {
    Ok(app_dir()?.join("runtime"))
}

pub fn state_dir() -> Result<PathBuf> {
    Ok(app_dir()?.join("state"))
}

pub fn lock_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("locallink-core.lock"))
}

pub fn app_paths() -> Result<AppPaths> {
    Ok(AppPaths {
        app_dir: app_dir()?.display().to_string(),
        config_file: config_path()?.display().to_string(),
        trusted_peers_file: trusted_peers_path()?.display().to_string(),
        trusted_devices_file: trusted_devices_path()?.display().to_string(),
        addons_dir: addons_dir()?.display().to_string(),
        logs_dir: logs_dir()?.display().to_string(),
        runtime_dir: runtime_dir()?.display().to_string(),
        state_dir: state_dir()?.display().to_string(),
        lock_file: lock_path()?.display().to_string(),
    })
}

pub fn init_app_dirs() -> Result<()> {
    fs::create_dir_all(app_dir()?)?;
    fs::create_dir_all(addons_dir()?)?;
    fs::create_dir_all(logs_dir()?)?;
    fs::create_dir_all(runtime_dir()?)?;
    fs::create_dir_all(state_dir()?)?;

    let trusted_peers = trusted_peers_path()?;
    if !trusted_peers.exists() {
        fs::write(&trusted_peers, "[]\n")?;
    }

    let trusted_devices = trusted_devices_path()?;
    if !trusted_devices.exists() {
        fs::write(&trusted_devices, "[]\n")?;
    }

    Ok(())
}

pub fn acquire_single_instance_lock() -> Result<File> {
    init_app_dirs()?;

    let path = lock_path()?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)?;

    match file.try_lock_exclusive() {
        Ok(()) => {
            file.set_len(0)?;
            use std::io::Write;
            writeln!(&file, "pid={}", std::process::id())?;
            Ok(file)
        }
        Err(_) => {
            anyhow::bail!(
                "LocalLink Core already appears to be running. Lock file: {}",
                path.display()
            );
        }
    }
}

pub fn load_or_create_config() -> Result<Config> {
    init_app_dirs()?;

    let path = config_path()?;

    if path.exists() {
        let text = fs::read_to_string(&path)?;
        let cfg: Config = serde_json::from_str(&text)?;
        return Ok(cfg);
    }

    let device_name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown".to_string());

    let cfg = Config {
        device_id: Uuid::new_v4().to_string(),
        device_name,
        psk_b64: None,
    };

    save_config(&cfg)?;
    Ok(cfg)
}

pub fn save_config(cfg: &Config) -> Result<()> {
    init_app_dirs()?;
    fs::write(config_path()?, serde_json::to_string_pretty(cfg)?)?;
    Ok(())
}

pub fn normalize_mac(mac: &str) -> String {
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

pub fn load_trusted_devices() -> Result<Vec<TrustedDevice>> {
    init_app_dirs()?;

    let path = trusted_devices_path()?;
    let text = fs::read_to_string(&path)?;
    let mut devices: Vec<TrustedDevice> = serde_json::from_str(&text)?;

    for device in &mut devices {
        device.macs = device
            .macs
            .iter()
            .map(|m| normalize_mac(m))
            .filter(|m| !m.is_empty())
            .collect();
    }

    Ok(devices)
}

pub fn save_trusted_devices(devices: &[TrustedDevice]) -> Result<()> {
    init_app_dirs()?;
    fs::write(
        trusted_devices_path()?,
        serde_json::to_string_pretty(devices)?,
    )?;
    Ok(())
}

pub fn add_trusted_device(name: String, mac: String) -> Result<Vec<TrustedDevice>> {
    let mac = normalize_mac(&mac);
    anyhow::ensure!(!mac.is_empty(), "MAC address must contain 12 hex digits");

    let mut devices = load_trusted_devices()?;

    if let Some(existing) = devices
        .iter_mut()
        .find(|d| d.macs.iter().any(|m| normalize_mac(m) == mac))
    {
        existing.name = name;
        existing.blocked = false;
    } else {
        devices.push(TrustedDevice {
            name,
            macs: vec![mac],
            device_id: None,
            blocked: false,
        });
    }

    save_trusted_devices(&devices)?;
    Ok(devices)
}

pub fn remove_trusted_mac(mac: &str) -> Result<Vec<TrustedDevice>> {
    let mac = normalize_mac(mac);
    let mut devices = load_trusted_devices()?;

    devices.retain_mut(|d| {
        d.macs.retain(|m| normalize_mac(m) != mac);
        !d.macs.is_empty()
    });

    save_trusted_devices(&devices)?;
    Ok(devices)
}

pub fn register_device_id_for_macs(macs: &[String], device_id: &str) -> Result<()> {
    let mut devices = load_trusted_devices()?;
    let normalized: Vec<String> = macs.iter().map(|m| normalize_mac(m)).collect();

    let mut changed = false;

    for trusted in &mut devices {
        if trusted
            .macs
            .iter()
            .any(|m| normalized.iter().any(|n| normalize_mac(m) == *n))
        {
            trusted.device_id = Some(device_id.to_string());
            changed = true;
        }
    }

    if changed {
        save_trusted_devices(&devices)?;
    }

    Ok(())
}

pub fn mac_is_trusted(macs: &[String]) -> Result<bool> {
    let trusted = load_trusted_devices()?;
    let normalized: Vec<String> = macs.iter().map(|m| normalize_mac(m)).collect();

    Ok(trusted.iter().any(|d| {
        !d.blocked
            && d.macs
                .iter()
                .any(|m| normalized.iter().any(|n| normalize_mac(m) == *n))
    }))
}

pub fn trusted_name_for_macs(macs: &[String]) -> Result<Option<String>> {
    let trusted = load_trusted_devices()?;
    let normalized: Vec<String> = macs.iter().map(|m| normalize_mac(m)).collect();

    Ok(trusted
        .iter()
        .find(|d| {
            !d.blocked
                && d.macs
                    .iter()
                    .any(|m| normalized.iter().any(|n| normalize_mac(m) == *n))
        })
        .map(|d| d.name.clone()))
}

pub fn generate_psk_b64() -> String {
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    STANDARD.encode(key)
}

pub fn validate_psk_b64(psk: &str) -> Result<()> {
    let decoded = STANDARD.decode(psk)?;
    anyhow::ensure!(decoded.len() == 32, "PSK must decode to exactly 32 bytes");
    Ok(())
}

pub fn psk_bytes(cfg: &Config) -> Result<Vec<u8>> {
    let Some(psk) = &cfg.psk_b64 else {
        anyhow::bail!("No PSK configured");
    };

    let decoded = STANDARD.decode(psk)?;
    anyhow::ensure!(decoded.len() == 32, "PSK must decode to exactly 32 bytes");
    Ok(decoded)
}
