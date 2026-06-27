use std::fs;
use std::path::Path;

pub fn run() {
    println!("cargo:rerun-if-changed=src/config.rs");
    println!("cargo:rerun-if-changed=src/space_runtime.rs");
    patch_config();
    patch_space_runtime();
    patch_api_for_connection_contexts();
}

fn patch_config() {
    let path = Path::new("src/config.rs");
    let mut text = read(path);
    let original = text.clone();

    if !text.contains("use std::collections::HashMap;") {
        text = text.replace(
            "use serde::{Deserialize, Serialize};\n",
            "use serde::{Deserialize, Serialize};\nuse std::collections::HashMap;\n",
        );
    }

    if !text.contains("DirectConnectionAddonStore") {
        text = text.replace(
            "pub fn normalize_mac(mac: &str) -> String {\n",
            r#"#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectConnectionAddonState {
    #[serde(default)]
    pub enabled: bool,
}

pub type DirectConnectionAddonStore = HashMap<String, HashMap<String, DirectConnectionAddonState>>;

pub fn direct_connection_addons_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("direct-connection-addons.json"))
}

pub fn direct_connection_space_id(peer_id: &str) -> String {
    format!("direct:{}", peer_id.trim())
}

pub fn direct_connection_peer_id(space_id: &str) -> Option<String> {
    space_id
        .trim()
        .strip_prefix("direct:")
        .map(str::trim)
        .filter(|peer_id| !peer_id.is_empty())
        .map(str::to_string)
}

pub fn load_direct_connection_addon_store() -> Result<DirectConnectionAddonStore> {
    init_app_dirs()?;
    let path = direct_connection_addons_path()?;
    if !path.exists() {
        atomic_write(&path, b"{}\n")?;
    }

    let text = fs::read_to_string(&path)?;
    let mut store: DirectConnectionAddonStore = if text.trim().is_empty() {
        HashMap::new()
    } else {
        serde_json::from_str(&text)?
    };

    normalize_direct_connection_addon_store(&mut store);
    save_direct_connection_addon_store(&store)?;
    Ok(store)
}

pub fn save_direct_connection_addon_store(store: &DirectConnectionAddonStore) -> Result<()> {
    init_app_dirs()?;
    let mut store = store.clone();
    normalize_direct_connection_addon_store(&mut store);
    let text = serde_json::to_string_pretty(&store)?;
    atomic_write(&direct_connection_addons_path()?, text.as_bytes())?;
    Ok(())
}

pub fn direct_connection_addons_for_peer(
    store: &DirectConnectionAddonStore,
    peer_id: &str,
) -> Vec<spaces::SpaceAddonDesiredState> {
    let mut response: Vec<_> = store
        .get(peer_id.trim())
        .into_iter()
        .flat_map(|addons| addons.iter())
        .map(|(addon_id, state)| spaces::SpaceAddonDesiredState {
            addon_id: addon_id.clone(),
            enabled: state.enabled,
        })
        .collect();
    response.sort_by(|left, right| left.addon_id.cmp(&right.addon_id));
    response
}

pub fn set_direct_connection_addon_enabled(
    peer_id: &str,
    addon_id: &str,
    enabled: bool,
) -> Result<Vec<spaces::SpaceAddonDesiredState>> {
    let peer_id = peer_id.trim();
    let addon_id = addon_id.trim();
    anyhow::ensure!(!peer_id.is_empty(), "peer_id cannot be empty");
    anyhow::ensure!(!addon_id.is_empty(), "addon_id cannot be empty");

    let mut store = load_direct_connection_addon_store()?;
    store
        .entry(peer_id.to_string())
        .or_default()
        .insert(addon_id.to_string(), DirectConnectionAddonState { enabled });
    save_direct_connection_addon_store(&store)?;
    Ok(direct_connection_addons_for_peer(&store, peer_id))
}

fn normalize_direct_connection_addon_store(store: &mut DirectConnectionAddonStore) {
    let mut normalized = DirectConnectionAddonStore::new();
    for (peer_id, addons) in std::mem::take(store) {
        let peer_id = peer_id.trim().to_string();
        if peer_id.is_empty() {
            continue;
        }

        let mut normalized_addons = HashMap::new();
        for (addon_id, state) in addons {
            let addon_id = addon_id.trim().to_string();
            if addon_id.is_empty() {
                continue;
            }
            normalized_addons.insert(addon_id, state);
        }

        if !normalized_addons.is_empty() {
            normalized.insert(peer_id, normalized_addons);
        }
    }
    *store = normalized;
}

pub fn normalize_mac(mac: &str) -> String {
"#,
        );
    }

    if text != original {
        fs::write(path, text).expect("write config source");
    }
}

fn patch_space_runtime() {
    let path = Path::new("src/space_runtime.rs");
    let mut text = read(path);
    let original = text.clone();

    if !text.contains("direct_connection_space_id(peer_id)") {
        text = text.replace(
            r#"        let mut desired_addons: Vec<_> = space.addons.iter().collect();
"#,
            r#"        if space.kind != SpaceKind::Group {
            continue;
        }

        let mut desired_addons: Vec<_> = space.addons.iter().collect();
"#,
        );

        text = text.replace(
            "    plans.sort_by(|left, right| {\n",
            r#"    let direct_cache = crate::config::load_direct_connection_addon_store().unwrap_or_default();
    for peer_id in connected_peer_ids {
        let Some(direct_addons) = direct_cache.get(peer_id) else {
            continue;
        };

        let mut desired_addons: Vec<_> = direct_addons.iter().collect();
        desired_addons.sort_by(|(left, _), (right, _)| left.cmp(right));

        for (addon_id, desired_state) in desired_addons {
            if !desired_state.enabled {
                continue;
            }

            let Some(addon) = addons_by_id.get(addon_id.as_str()) else {
                continue;
            };

            let space_id = crate::config::direct_connection_space_id(peer_id);
            plans.push(SpaceAddonInstancePlan {
                instance_id: format!("{}:{}", space_id, addon.id),
                space_id,
                space_name: format!("Direct connection {}", peer_id),
                space_kind: SpaceKind::Direct,
                addon_id: addon.id.clone(),
                addon_name: addon.name.clone(),
                executable: addon_executable_path(addon),
                connected_members: vec![peer_id.clone()],
            });
        }
    }

    plans.sort_by(|left, right| {
"#,
        );
    }

    if text != original {
        fs::write(path, text).expect("write space runtime source");
    }
}

fn patch_api_for_connection_contexts() {
    let path = Path::new("src/api.rs");
    let mut text = read(path);
    let original = text.clone();

    text = text.replace(
        "use crate::config::{\n    add_trusted_device, app_paths, load_trusted_devices, mac_is_trusted, normalize_mac,\n    remove_trusted_mac, trusted_name_for_macs, Config,\n};",
        "use crate::config::{\n    add_trusted_device, app_paths, direct_connection_addons_for_peer, direct_connection_peer_id,\n    direct_connection_space_id, load_direct_connection_addon_store, load_trusted_devices,\n    mac_is_trusted, normalize_mac, remove_trusted_mac, set_direct_connection_addon_enabled,\n    trusted_name_for_macs, Config,\n};",
    );

    text = text.replace(
        "            .iter()\n        .map(|space| space_view(space, membership, local_device_id))",
        "            .iter()\n        .filter(|space| space.kind == SpaceKind::Group)\n        .map(|space| space_view(space, membership, local_device_id))",
    );

    if !text.contains("fn direct_space_view(") {
        text = text.replace(
            "fn space_views(\n",
            r#"fn direct_space_view(peer_id: &str, peer_name: &str, addon_count: usize) -> SpaceView {
    SpaceView {
        space_id: direct_connection_space_id(peer_id),
        name: if peer_name.trim().is_empty() {
            format!("Direct connection {}", peer_id)
        } else {
            format!("Direct connection to {}", peer_name.trim())
        },
        kind: SpaceKind::Direct,
        active: true,
        members: vec![peer_id.to_string()],
        addon_count,
        role: "direct".to_string(),
        local_state: "direct".to_string(),
        owner_device_id: None,
        invite_status: None,
        left: false,
        can_accept_invite: false,
        can_decline_invite: false,
        can_connect: false,
        can_disconnect: false,
        can_leave: false,
        can_invite_members: false,
        can_remove_members: false,
        can_manage_addons: true,
    }
}

fn sort_space_views(response: &mut [SpaceView]) {
    response.sort_by(|a, b| {
        let bucket = |space: &SpaceView| match space.local_state.as_str() {
            "invite_pending" => 0,
            "owned" => 1,
            "joined" => 2,
            "direct" => 3,
            "removed" => 4,
            "left" => 5,
            _ => 6,
        };
        bucket(a)
            .cmp(&bucket(b))
            .then(a.name.cmp(&b.name))
            .then(a.space_id.cmp(&b.space_id))
    });
}

fn space_views(
"#,
        );

        text = text.replace(
            r#"    response.sort_by(|a, b| {
        let bucket = |space: &SpaceView| match space.local_state.as_str() {
            "invite_pending" => 0,
            "owned" => 1,
            "joined" => 2,
            "removed" => 3,
            "left" => 4,
            _ => 5,
        };
        bucket(a)
            .cmp(&bucket(b))
            .then(a.name.cmp(&b.name))
            .then(a.space_id.cmp(&b.space_id))
    });
    response
"#,
            r#"    sort_space_views(&mut response);
    response
"#,
        );
    }

    text = text.replace(
        "            let kind = req.kind.unwrap_or(SpaceKind::Direct);",
        "            let kind = SpaceKind::Group;",
    );

    if !text.contains("let direct_cache = load_direct_connection_addon_store().unwrap_or_default();") {
        text = text.replace(
            r#"            let response = space_views(&store, &membership, &cfg.device_id);
            Ok(serde_json::to_string(&ok(response))?)
"#,
            r#"            let mut response = space_views(&store, &membership, &cfg.device_id);
            drop(store);

            let direct_cache = load_direct_connection_addon_store().unwrap_or_default();
            let connections_guard = connections.lock().await;
            for peer in connections_guard.values() {
                let addon_count = direct_cache
                    .get(&peer.device_id)
                    .map(|addons| addons.len())
                    .unwrap_or(0);
                response.push(direct_space_view(&peer.device_id, &peer.device_name, addon_count));
            }
            sort_space_views(&mut response);
            Ok(serde_json::to_string(&ok(response))?)
"#,
        );
    }

    if !text.contains("direct_connection_peer_id(&space_id)") {
        text = text.replace(
            r#"            let store = spaces.lock().await;
            let response = store.space_addons(&space_id)?;

            Ok(serde_json::to_string(&ok(response))?)
        }
"#,
            r#"            if let Some(peer_id) = direct_connection_peer_id(&space_id) {
                let cache = load_direct_connection_addon_store()?;
                let response = direct_connection_addons_for_peer(&cache, &peer_id);
                return Ok(serde_json::to_string(&ok(response))?);
            }

            let store = spaces.lock().await;
            let response = store.space_addons(&space_id)?;

            Ok(serde_json::to_string(&ok(response))?)
        }
"#,
        );

        text = text.replace(
            r#"            let mut store = spaces.lock().await;
            let mut updated = store.clone();

            updated.set_addon_enabled(&space_id, &addon_id, enabled)?;
"#,
            r#"            if let Some(peer_id) = direct_connection_peer_id(&space_id) {
                let response = set_direct_connection_addon_enabled(&peer_id, &addon_id, enabled)?;
                return Ok(serde_json::to_string(&ok(response))?);
            }

            let mut store = spaces.lock().await;
            let mut updated = store.clone();

            updated.set_addon_enabled(&space_id, &addon_id, enabled)?;
"#,
        );
    }

    if text != original {
        fs::write(path, text).expect("write API source");
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
        .replace("\r\n", "\n")
}
