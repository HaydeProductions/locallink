use crate::config::{init_app_dirs, spaces_path};
use crate::diagnostics;
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
    pub active: bool,

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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SpaceAddonDesiredState {
    pub addon_id: String,
    pub enabled: bool,
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

    pub fn set_space_active(&mut self, space_id: &str, active: bool) -> Result<SpaceRecord> {
        let space_id = space_id.trim();
        anyhow::ensure!(!space_id.is_empty(), "space_id cannot be empty");

        let space = self
            .spaces
            .iter_mut()
            .find(|space| space.space_id == space_id)
            .ok_or_else(|| anyhow::anyhow!("unknown space: {}", space_id))?;

        space.active = active;
        diagnostics::log(
            "space-state",
            format!(
                "set_space_active space_id={} active={} members={} addons={}",
                space.space_id,
                space.active,
                space.members.len(),
                space.addons.len()
            ),
        );
        let response = space.clone();
        self.validate_and_repair()?;

        Ok(response)
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
        diagnostics::log(
            "space-state",
            format!(
                "set_addon_enabled space_id={} addon_id={} enabled={} space_active={} members={} addon_entries={}",
                space.space_id,
                addon_id,
                enabled,
                space.active,
                space.members.len(),
                space.addons.len()
            ),
        );
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

    pub fn space_addons(&self, space_id: &str) -> Result<Vec<SpaceAddonDesiredState>> {
        let space_id = space_id.trim();
        anyhow::ensure!(!space_id.is_empty(), "space_id cannot be empty");

        let space = self
            .spaces
            .iter()
            .find(|space| space.space_id == space_id)
            .ok_or_else(|| anyhow::anyhow!("unknown space: {}", space_id))?;

        let mut addons: Vec<_> = space
            .addons
            .iter()
            .map(|(addon_id, state)| SpaceAddonDesiredState {
                addon_id: addon_id.clone(),
                enabled: state.enabled,
            })
            .collect();
        addons.sort_by(|left, right| left.addon_id.cmp(&right.addon_id));
        Ok(addons)
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

    SpaceActivationState {
        space_id: space.space_id.clone(),
        name: space.name.clone(),
        kind: space.kind.clone(),
        active: space.active,
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

    fn space_record(
        space_id: &str,
        name: &str,
        kind: SpaceKind,
        members: Vec<&str>,
    ) -> SpaceRecord {
        SpaceRecord {
            space_id: space_id.to_string(),
            name: name.to_string(),
            kind,
            active: false,
            members: members.into_iter().map(str::to_string).collect(),
            addons: HashMap::new(),
        }
    }

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
                active: false,
                members: vec![" b ".to_string(), "a".to_string(), "b".to_string()],
                addons: HashMap::new(),
            }],
        };

        store.validate_and_repair().unwrap();
        assert_eq!(store.spaces[0].members, vec!["a", "b"]);
    }

    #[test]
    fn direct_spaces_allow_at_most_one_member() {
        let mut store = SpaceStore {
            spaces: vec![space_record("direct", "Direct", SpaceKind::Direct, vec!["a", "b"])],
        };

        assert!(store.validate_and_repair().is_err());
    }
}
