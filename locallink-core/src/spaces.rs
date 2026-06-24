use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use crate::config::{atomic_write, init_app_dirs, spaces_path};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SpaceKind {
    Direct,
    Group,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
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
    pub addons: BTreeMap<String, SpaceAddonState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SpaceStore {
    #[serde(default)]
    pub spaces: Vec<SpaceRecord>,
}

impl SpaceStore {
    pub fn validate_and_repair(&mut self) -> Result<bool> {
        let mut changed = false;
        let mut seen_space_ids = BTreeSet::new();

        for space in &mut self.spaces {
            let trimmed_space_id = space.space_id.trim().to_string();
            anyhow::ensure!(!trimmed_space_id.is_empty(), "space_id must not be empty");
            if trimmed_space_id != space.space_id {
                space.space_id = trimmed_space_id;
                changed = true;
            }
            anyhow::ensure!(
                seen_space_ids.insert(space.space_id.clone()),
                "duplicate space_id: {}",
                space.space_id
            );

            let trimmed_name = space.name.trim().to_string();
            anyhow::ensure!(!trimmed_name.is_empty(), "space name must not be empty");
            if trimmed_name != space.name {
                space.name = trimmed_name;
                changed = true;
            }

            let original_members = space.members.clone();
            let mut members: Vec<String> = space
                .members
                .iter()
                .map(|member| member.trim().to_string())
                .filter(|member| !member.is_empty())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            members.sort();
            if members != original_members {
                space.members = members;
                changed = true;
            }

            if matches!(space.kind, SpaceKind::Direct) {
                anyhow::ensure!(
                    space.members.len() <= 1,
                    "direct space {} has more than one member",
                    space.space_id
                );
            }

            let original_addons = space.addons.clone();
            space.addons = space
                .addons
                .iter()
                .filter_map(|(addon_id, state)| {
                    let addon_id = addon_id.trim().to_string();
                    if addon_id.is_empty() {
                        None
                    } else {
                        Some((addon_id, state.clone()))
                    }
                })
                .collect();
            if space.addons != original_addons {
                changed = true;
            }
        }

        Ok(changed)
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

    if store.validate_and_repair()? {
        save_space_store(&store)?;
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn direct_space(members: Vec<&str>) -> SpaceRecord {
        SpaceRecord {
            space_id: "direct-1".to_string(),
            name: "Desktop".to_string(),
            kind: SpaceKind::Direct,
            members: members.into_iter().map(str::to_string).collect(),
            addons: BTreeMap::new(),
        }
    }

    #[test]
    fn validate_repairs_member_order_and_duplicates() {
        let mut store = SpaceStore {
            spaces: vec![SpaceRecord {
                space_id: " office ".to_string(),
                name: " Office ".to_string(),
                kind: SpaceKind::Group,
                members: vec![
                    "laptop".to_string(),
                    " desktop ".to_string(),
                    "laptop".to_string(),
                    "".to_string(),
                ],
                addons: BTreeMap::new(),
            }],
        };

        assert!(store.validate_and_repair().unwrap());
        let space = &store.spaces[0];
        assert_eq!(space.space_id, "office");
        assert_eq!(space.name, "Office");
        assert_eq!(
            space.members,
            vec!["desktop".to_string(), "laptop".to_string()]
        );
    }

    #[test]
    fn direct_space_rejects_multiple_members() {
        let mut store = SpaceStore {
            spaces: vec![direct_space(vec!["desktop", "laptop"])],
        };

        assert!(store.validate_and_repair().is_err());
    }

    #[test]
    fn group_space_accepts_multiple_members() {
        let mut store = SpaceStore {
            spaces: vec![SpaceRecord {
                space_id: "group-1".to_string(),
                name: "Office".to_string(),
                kind: SpaceKind::Group,
                members: vec!["desktop".to_string(), "laptop".to_string()],
                addons: BTreeMap::new(),
            }],
        };

        assert!(!store.validate_and_repair().unwrap());
    }
}
