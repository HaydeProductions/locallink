use crate::config::addons_dir;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(target_os = "windows")]
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddonManifest {
    pub id: String,
    pub name: String,
    pub version: String,

    #[serde(default)]
    pub description: String,

    #[serde(default)]
    pub executable: String,

    #[serde(default)]
    pub services: Vec<String>,

    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AddonRecord {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub executable: String,
    pub services: Vec<String>,
    pub enabled: bool,
    pub manifest_path: String,
    pub addon_dir: String,
}

pub fn create_example_addon_manifest() -> Result<()> {
    // No-op.
    // Example add-ons should not be recreated automatically once the user removes them.
    Ok(())
}

pub fn load_addon_manifests() -> Result<Vec<AddonRecord>> {
    create_example_addon_manifest()?;

    let root = addons_dir()?;
    fs::create_dir_all(&root)?;

    let mut records = Vec::new();

    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join("manifest.json");

        if !manifest_path.exists() {
            continue;
        }

        match load_one_manifest(&path, &manifest_path) {
            Ok(record) => records.push(record),
            Err(err) => {
                eprintln!(
                    "Failed to load addon manifest {}: {err}",
                    manifest_path.display()
                );
            }
        }
    }

    records.sort_by(|a, b| a.name.cmp(&b.name));
    stop_disabled_addon_executables_best_effort(&records);
    Ok(records)
}

fn load_one_manifest(addon_dir: &Path, manifest_path: &PathBuf) -> Result<AddonRecord> {
    let text = fs::read_to_string(manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;

    let manifest: AddonManifest = serde_json::from_str(&text)
        .with_context(|| format!("parsing {}", manifest_path.display()))?;

    anyhow::ensure!(!manifest.id.trim().is_empty(), "addon id cannot be empty");
    anyhow::ensure!(
        !manifest.name.trim().is_empty(),
        "addon name cannot be empty"
    );
    anyhow::ensure!(
        !manifest.version.trim().is_empty(),
        "addon version cannot be empty"
    );

    Ok(AddonRecord {
        id: manifest.id,
        name: manifest.name,
        version: manifest.version,
        description: manifest.description,
        executable: manifest.executable,
        services: manifest.services,
        enabled: manifest.enabled,
        manifest_path: manifest_path.display().to_string(),
        addon_dir: addon_dir.display().to_string(),
    })
}

fn stop_disabled_addon_executables_best_effort(records: &[AddonRecord]) {
    for addon in records {
        if addon.enabled || addon.executable.trim().is_empty() {
            continue;
        }

        if let Err(err) = stop_addon_executable(addon) {
            eprintln!("Could not stop disabled add-on {}: {err}", addon.name);
        }
    }
}

#[cfg(target_os = "windows")]
fn stop_addon_executable(addon: &AddonRecord) -> Result<()> {
    let exe_path = Path::new(&addon.addon_dir).join(&addon.executable);
    let exe_path = exe_path.canonicalize().unwrap_or(exe_path);
    let exe_path = exe_path.display().to_string();

    let script = r#"
$target = [Environment]::GetEnvironmentVariable('LOCALLINK_ADDON_EXE')
if ([string]::IsNullOrWhiteSpace($target)) { exit 0 }
Get-CimInstance Win32_Process |
    Where-Object { $_.ExecutablePath -eq $target } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
"#;

    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script])
        .env("LOCALLINK_ADDON_EXE", exe_path)
        .status()
        .context("running disabled add-on stop command")?;

    if !status.success() {
        anyhow::bail!("disabled add-on stop command exited with {status}");
    }

    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn stop_addon_executable(_addon: &AddonRecord) -> Result<()> {
    Ok(())
}
