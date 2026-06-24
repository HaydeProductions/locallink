use crate::config::{init_app_dirs, spaces_path};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

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
}
