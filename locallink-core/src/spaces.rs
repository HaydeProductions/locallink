use crate::config::{init_app_dirs, spaces_path};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SpaceKind {
    Direct,
    Group,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpaceAddonState {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpaceRecord {
    pub space_id: String,
    pub name: String,
    pub kind: SpaceKind,

    #[serde(default)]
    pub members: Vec<String>,

    #[serde(default)]
    pub addons: HashMap<String, SpaceAddonState>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpaceStore {
    #[serde(default)]
    pub spaces: Vec<SpaceRecord>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SpaceActivationState {
    pub space_id: String,
    pub name: String,
    pub kind: SpaceKind,
    pub active: bool,
    pub connected_members: Vec<String>,
    pub missing_members: Vec<String>,
}

pub type SpaceRegistry = Arc<Mutex<SpaceStore>>;

pub fn new_space_registry(store: SpaceStore) -> SpaceRegistry {
    Arc::new(Mutex::new(store))
}

impl SpaceStore {
    pub fn validate_and_repair(&mut self) -> Result<()> {
        let mut seen_space_ids = HashSet::<String>::new();

        for space in &mut self.spaces {
            space.space_id = space.space_id.trim().to_string();
            space.name = space.name.trim().to_string();

            anyhow::ensure!(!space.space_id.is_empty(), "space_id cannot be empty");
            anyhow::ensure!(
                seen_space_ids.insert(space.space_id.clone()),
                "duplicate space_id: {}",
                space.space_id
            );

            if space.name.is_empty() {
                space.name = space.space_id.clone();
            }

            space.members = normalize_members(&space.members);
            normalize_addons(&mut space.addons)?;

            if space.kind == SpaceKind::Direct {
                anyhow::ensure!(
                    space.members.len() <= 1,
                    "direct space {} cannot contain more than one member",
                    space.space_id
                );
            }
        }

        Ok(())
    }

    pub fn set_addon_enabled(
        &mut self,
        space_id: &str,
        addon_id: &str,
        enabled: bool,
    ) -> Result<SpaceAddonState> {
        let space_id = space_id.trim();
        let addon_id = addon_id.trim();

        anyhow::ensure!(!space_id.is_empty(), "space_id cannot be empty");
        anyhow::ensure!(!addon_id.is_empty(), "addon_id cannot be empty");

        let space = self
            .spaces
            .iter_mut()
            .find(|space| space.space_id == space_id)
            .ok_or_else(|| anyhow::anyhow!("unknown space: {}", space_id))?;

        let state = SpaceAddonState { enabled };
        space.addons.insert(addon_id.to_string(), state.clone());
        self.validate_and_repair()?;

        Ok(state)
    }

    pub fn addon_state(&self, space_id: &str, addon_id: &str) -> Result<Option<SpaceAddonState>> {
        let space_id = space_id.trim();
        let addon_id = addon_id.trim();

        anyhow::ensure!(!space_id.is_empty(), "space_id cannot be empty");
        anyhow::ensure!(!addon_id.is_empty(), "addon_id cannot be empty");

        let space = self
            .spaces
            .iter()
            .find(|space| space.space_id == space_id)
            .ok_or_else(|| anyhow::anyhow!("unknown space: {}", space_id))?;

        Ok(space.addons.get(addon_id).cloned())
    }

    pub fn activation_state(
        &self,
        space_id: &str,
        connected_peer_ids: &HashSet<String>,
    ) -> Result<SpaceActivationState> {
        let space_id = space_id.trim();
        anyhow::ensure!(!space_id.is_empty(), "space_id cannot be empty");

        let space = self
            .spaces
            .iter()
            .find(|space| space.space_id == space_id)
            .ok_or_else(|| anyhow::anyhow!("unknown space: {}", space_id))?;

        Ok(space_activation_state(space, connected_peer_ids))
    }

    pub fn activation_states(
        &self,
        connected_peer_ids: &HashSet<String>,
    ) -> Vec<SpaceActivationState> {
        self.spaces
            .iter()
            .map(|space| space_activation_state(space, connected_peer_ids))
            .collect()
    }
}

pub fn load_or_create_space_store() -> Result<SpaceStore> {
    init_app_dirs()?;

    let path = spaces_path()?;

    if !path.exists() {
        let store = SpaceStore::default();
        save_space_store(&store)?;
        return Ok(store);
    }

    let text = fs::read_to_string(&path)
        .with_context(|| format!("failed to read spaces store: {}", path.display()))?;
    let mut store: SpaceStore = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse spaces store: {}", path.display()))?;

    store.validate_and_repair()?;
    save_space_store(&store)?;

    Ok(store)
}

pub fn save_space_store(store: &SpaceStore) -> Result<()> {
    init_app_dirs()?;

    let mut store = store.clone();
    store.validate_and_repair()?;

    let text = serde_json::to_string_pretty(&store)?;
    atomic_write(&spaces_path()?, text.as_bytes())?;

    Ok(())
}

fn space_activation_state(
    space: &SpaceRecord,
    connected_peer_ids: &HashSet<String>,
) -> SpaceActivationState {
    let mut connected_members = Vec::new();
    let mut missing_members = Vec::new();

    for member in &space.members {
        if connected_peer_ids.contains(member) {
            connected_members.push(member.clone());
        } else {
            missing_members.push(member.clone());
        }
    }

    let active = match space.kind {
        SpaceKind::Direct => space.members.len() == 1 && connected_members.len() == 1,
        SpaceKind::Group => !connected_members.is_empty(),
    };

    SpaceActivationState {
        space_id: space.space_id.clone(),
        name: space.name.clone(),
        kind: space.kind.clone(),
        active,
        connected_members,
        missing_members,
    }
}

fn normalize_members(members: &[String]) -> Vec<String> {
    let mut members: Vec<String> = members
        .iter()
        .map(|member| member.trim().to_string())
        .filter(|member| !member.is_empty())
        .collect();

    members.sort();
    members.dedup();
    members
}

fn normalize_addons(addons: &mut HashMap<String, SpaceAddonState>) -> Result<()> {
    let mut normalized = HashMap::<String, SpaceAddonState>::new();

    for (addon_id, state) in addons.drain() {
        let addon_id = addon_id.trim().to_string();
        anyhow::ensure!(!addon_id.is_empty(), "addon_id cannot be empty");
        normalized.insert(addon_id, state);
    }

    *addons = normalized;
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = path.with_extension("tmp");
    {
        let mut file = File::create(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_store_has_no_spaces() {
        let store = SpaceStore::default();

        assert!(store.spaces.is_empty());
    }

    #[test]
    fn members_are_trimmed_sorted_and_deduped() {
        let mut store = SpaceStore {
            spaces: vec![SpaceRecord {
                space_id: "office".to_string(),
                name: "Office".to_string(),
                kind: SpaceKind::Group,
                members: vec![
                    " laptop ".to_string(),
                    "desktop".to_string(),
                    "laptop".to_string(),
                    "".to_string(),
                ],
                addons: HashMap::new(),
            }],
        };

        store.validate_and_repair().unwrap();

        assert_eq!(
            store.spaces[0].members,
            vec!["desktop".to_string(), "laptop".to_string()]
        );
    }

    #[test]
    fn direct_space_allows_one_member() {
        let mut store = SpaceStore {
            spaces: vec![SpaceRecord {
                space_id: "desktop".to_string(),
                name: "Desktop".to_string(),
                kind: SpaceKind::Direct,
                members: vec!["desktop-peer".to_string()],
                addons: HashMap::new(),
            }],
        };

        assert!(store.validate_and_repair().is_ok());
    }

    #[test]
    fn direct_space_rejects_multiple_members() {
        let mut store = SpaceStore {
            spaces: vec![SpaceRecord {
                space_id: "desktop".to_string(),
                name: "Desktop".to_string(),
                kind: SpaceKind::Direct,
                members: vec!["desktop-peer".to_string(), "laptop-peer".to_string()],
                addons: HashMap::new(),
            }],
        };

        assert!(store.validate_and_repair().is_err());
    }

    #[test]
    fn group_space_allows_multiple_members() {
        let mut store = SpaceStore {
            spaces: vec![SpaceRecord {
                space_id: "office".to_string(),
                name: "Office".to_string(),
                kind: SpaceKind::Group,
                members: vec!["desktop-peer".to_string(), "laptop-peer".to_string()],
                addons: HashMap::new(),
            }],
        };

        assert!(store.validate_and_repair().is_ok());
    }

    #[test]
    fn duplicate_space_ids_are_rejected() {
        let mut store = SpaceStore {
            spaces: vec![
                SpaceRecord {
                    space_id: "office".to_string(),
                    name: "Office".to_string(),
                    kind: SpaceKind::Group,
                    members: Vec::new(),
                    addons: HashMap::new(),
                },
                SpaceRecord {
                    space_id: "office".to_string(),
                    name: "Office Again".to_string(),
                    kind: SpaceKind::Group,
                    members: Vec::new(),
                    addons: HashMap::new(),
                },
            ],
        };

        assert!(store.validate_and_repair().is_err());
    }

    #[test]
    fn set_addon_enabled_records_space_desired_state() {
        let mut store = SpaceStore {
            spaces: vec![SpaceRecord {
                space_id: "office".to_string(),
                name: "Office".to_string(),
                kind: SpaceKind::Group,
                members: Vec::new(),
                addons: HashMap::new(),
            }],
        };

        let state = store
            .set_addon_enabled("office", "clipboard", true)
            .unwrap();

        assert!(state.enabled);
        assert_eq!(
            store.addon_state("office", "clipboard").unwrap(),
            Some(SpaceAddonState { enabled: true })
        );
    }

    #[test]
    fn addon_ids_are_trimmed_when_repaired() {
        let mut addons = HashMap::new();
        addons.insert(" clipboard ".to_string(), SpaceAddonState { enabled: true });

        let mut store = SpaceStore {
            spaces: vec![SpaceRecord {
                space_id: "office".to_string(),
                name: "Office".to_string(),
                kind: SpaceKind::Group,
                members: Vec::new(),
                addons,
            }],
        };

        store.validate_and_repair().unwrap();

        assert!(store.spaces[0].addons.contains_key("clipboard"));
    }

    #[test]
    fn empty_addon_ids_are_rejected() {
        let mut store = SpaceStore {
            spaces: vec![SpaceRecord {
                space_id: "office".to_string(),
                name: "Office".to_string(),
                kind: SpaceKind::Group,
                members: Vec::new(),
                addons: HashMap::new(),
            }],
        };

        assert!(store.set_addon_enabled("office", " ", true).is_err());
    }

    #[test]
    fn direct_space_is_active_only_when_its_member_is_connected() {
        let store = SpaceStore {
            spaces: vec![SpaceRecord {
                space_id: "desktop".to_string(),
                name: "Desktop".to_string(),
                kind: SpaceKind::Direct,
                members: vec!["desktop-peer".to_string()],
                addons: HashMap::new(),
            }],
        };
        let connected = HashSet::from(["desktop-peer".to_string()]);

        let state = store.activation_state("desktop", &connected).unwrap();

        assert!(state.active);
        assert_eq!(state.connected_members, vec!["desktop-peer".to_string()]);
        assert!(state.missing_members.is_empty());
    }

    #[test]
    fn direct_space_is_inactive_when_its_member_is_missing() {
        let store = SpaceStore {
            spaces: vec![SpaceRecord {
                space_id: "desktop".to_string(),
                name: "Desktop".to_string(),
                kind: SpaceKind::Direct,
                members: vec!["desktop-peer".to_string()],
                addons: HashMap::new(),
            }],
        };
        let connected = HashSet::new();

        let state = store.activation_state("desktop", &connected).unwrap();

        assert!(!state.active);
        assert!(state.connected_members.is_empty());
        assert_eq!(state.missing_members, vec!["desktop-peer".to_string()]);
    }

    #[test]
    fn group_space_stays_active_with_partial_members_connected() {
        let store = SpaceStore {
            spaces: vec![SpaceRecord {
                space_id: "office".to_string(),
                name: "Office".to_string(),
                kind: SpaceKind::Group,
                members: vec!["desktop-peer".to_string(), "laptop-peer".to_string()],
                addons: HashMap::new(),
            }],
        };
        let connected = HashSet::from(["desktop-peer".to_string()]);

        let state = store.activation_state("office", &connected).unwrap();

        assert!(state.active);
        assert_eq!(state.connected_members, vec!["desktop-peer".to_string()]);
        assert_eq!(state.missing_members, vec!["laptop-peer".to_string()]);
    }

    #[test]
    fn group_space_is_inactive_when_no_members_are_connected() {
        let store = SpaceStore {
            spaces: vec![SpaceRecord {
                space_id: "office".to_string(),
                name: "Office".to_string(),
                kind: SpaceKind::Group,
                members: vec!["desktop-peer".to_string(), "laptop-peer".to_string()],
                addons: HashMap::new(),
            }],
        };
        let connected = HashSet::new();

        let state = store.activation_state("office", &connected).unwrap();

        assert!(!state.active);
        assert!(state.connected_members.is_empty());
        assert_eq!(
            state.missing_members,
            vec!["desktop-peer".to_string(), "laptop-peer".to_string()]
        );
    }
}
