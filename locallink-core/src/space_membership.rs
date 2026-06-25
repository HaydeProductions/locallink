use crate::config::{init_app_dirs, state_dir};
use crate::config::spaces::{SpaceKind, SpaceRecord, SpaceStore};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const SPACE_SYNC_SERVICE: &str = "locallink.space.sync";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SpaceRole {
    Owner,
    Member,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SpaceInviteStatus {
    Pending,
    Accepted,
    Declined,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpaceMembershipRecord {
    pub space_id: String,
    pub role: SpaceRole,
    pub owner_device_id: String,
    pub revision: u64,
    pub owner_enabled: bool,
    pub key_epoch: u64,
    pub left: bool,
    #[serde(default)]
    pub invite_state: Option<SpaceInviteState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpaceInviteRecord {
    pub invite_id: String,
    pub space_id: String,
    pub target_peer_id: String,
    pub invited_by: String,
    pub revision: u64,
    pub status: SpaceInviteStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpaceInviteState {
    pub invite_id: String,
    pub invited_by: String,
    pub status: SpaceInviteStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportedSpaceInvite {
    pub space_id: String,
    pub name: String,
    pub kind: SpaceKind,
    pub owner_device_id: String,
    pub invite_id: String,
    pub revision: u64,
    pub owner_enabled: bool,
    #[serde(default)]
    pub members: Vec<String>,
    pub key_epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpaceSyncUpdate {
    pub message_type: String,
    pub space_id: String,
    pub owner_device_id: String,
    pub revision: u64,
    pub name: String,
    pub kind: SpaceKind,
    pub owner_enabled: bool,
    pub members: Vec<String>,
    pub key_epoch: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpaceMembershipStore {
    #[serde(default)]
    pub records: HashMap<String, SpaceMembershipRecord>,
    #[serde(default)]
    pub invites: Vec<SpaceInviteRecord>,
}

impl SpaceMembershipRecord {
    pub fn owner(space_id: String, owner_device_id: String) -> Self {
        Self {
            space_id,
            role: SpaceRole::Owner,
            owner_device_id,
            revision: 1,
            owner_enabled: true,
            key_epoch: 1,
            left: false,
            invite_state: None,
        }
    }

    pub fn joined_pending(invite: &ImportedSpaceInvite) -> Self {
        Self {
            space_id: invite.space_id.clone(),
            role: SpaceRole::Member,
            owner_device_id: invite.owner_device_id.clone(),
            revision: invite.revision.max(1),
            owner_enabled: invite.owner_enabled,
            key_epoch: invite.key_epoch.max(1),
            left: false,
            invite_state: Some(SpaceInviteState {
                invite_id: invite.invite_id.clone(),
                invited_by: invite.owner_device_id.clone(),
                status: SpaceInviteStatus::Pending,
            }),
        }
    }

    pub fn is_owner_for(&self, local_device_id: &str) -> bool {
        self.role == SpaceRole::Owner
            && (self.owner_device_id.is_empty() || self.owner_device_id == local_device_id)
    }
}

impl SpaceMembershipStore {
    pub fn validate_and_repair(&mut self, spaces: &mut SpaceStore) -> Result<()> {
        spaces.validate_and_repair()?;
        let known_space_ids: HashSet<String> = spaces
            .spaces
            .iter()
            .map(|space| space.space_id.clone())
            .collect();

        self.records
            .retain(|space_id, _| known_space_ids.contains(space_id));

        for record in self.records.values_mut() {
            record.space_id = record.space_id.trim().to_string();
            record.owner_device_id = record.owner_device_id.trim().to_string();
            if record.revision == 0 {
                record.revision = 1;
            }
            if record.key_epoch == 0 && !record.left {
                record.key_epoch = 1;
            }
            if record.left {
                record.invite_state = None;
                if let Some(space) = spaces
                    .spaces
                    .iter_mut()
                    .find(|space| space.space_id == record.space_id)
                {
                    space.active = false;
                }
            }
        }

        normalize_invites(&mut self.invites)?;
        self.invites
            .retain(|invite| known_space_ids.contains(&invite.space_id));
        Ok(())
    }

    pub fn ensure_owned_space(&mut self, spaces: &SpaceStore, space_id: &str, owner_device_id: &str) -> Result<()> {
        ensure_space_exists(spaces, space_id)?;
        self.records
            .entry(space_id.to_string())
            .or_insert_with(|| SpaceMembershipRecord::owner(space_id.to_string(), owner_device_id.to_string()));
        Ok(())
    }

    pub fn set_owner_enabled(
        &mut self,
        spaces: &SpaceStore,
        local_device_id: &str,
        space_id: &str,
        enabled: bool,
    ) -> Result<SpaceSyncUpdate> {
        self.ensure_owner(local_device_id, space_id)?;
        let record = self.record_mut(space_id)?;
        record.owner_enabled = enabled;
        bump(record);
        self.sync_update(spaces, local_device_id, space_id)
    }

    pub fn create_invite(
        &mut self,
        spaces: &SpaceStore,
        local_device_id: &str,
        space_id: &str,
        target_peer_id: &str,
    ) -> Result<SpaceInviteRecord> {
        let target_peer_id = target_peer_id.trim();
        anyhow::ensure!(!target_peer_id.is_empty(), "target peer_id cannot be empty");
        self.ensure_owner(local_device_id, space_id)?;
        let space = ensure_space_exists(spaces, space_id)?;
        anyhow::ensure!(
            !space.members.iter().any(|member| member == target_peer_id),
            "peer is already a member of space {}",
            space_id
        );

        if let Some(invite) = self.invites.iter().find(|invite| {
            invite.space_id == space_id
                && invite.target_peer_id == target_peer_id
                && invite.status == SpaceInviteStatus::Pending
        }) {
            return Ok(invite.clone());
        }

        let revision = {
            let record = self.record_mut(space_id)?;
            bump(record);
            record.revision
        };

        let invite = SpaceInviteRecord {
            invite_id: Uuid::new_v4().to_string(),
            space_id: space_id.to_string(),
            target_peer_id: target_peer_id.to_string(),
            invited_by: local_device_id.to_string(),
            revision,
            status: SpaceInviteStatus::Pending,
        };
        self.invites.push(invite.clone());
        Ok(invite)
    }

    pub fn import_invite(
        &mut self,
        spaces: &mut SpaceStore,
        local_device_id: &str,
        invite: ImportedSpaceInvite,
    ) -> Result<SpaceRecord> {
        anyhow::ensure!(invite.owner_device_id != local_device_id, "cannot import own space invite");
        anyhow::ensure!(
            !spaces.spaces.iter().any(|space| space.space_id == invite.space_id),
            "space already exists locally: {}",
            invite.space_id
        );
        let mut members = invite.members.clone();
        members.push(invite.owner_device_id.clone());
        members.sort();
        members.dedup();

        let record = SpaceRecord {
            space_id: invite.space_id.clone(),
            name: invite.name.clone(),
            kind: invite.kind.clone(),
            active: false,
            members,
            addons: HashMap::new(),
        };
        spaces.spaces.push(record.clone());
        self.records.insert(
            invite.space_id.clone(),
            SpaceMembershipRecord::joined_pending(&invite),
        );
        Ok(record)
    }

    pub fn accept_invite(
        &mut self,
        spaces: &mut SpaceStore,
        local_device_id: &str,
        space_id: &str,
    ) -> Result<SpaceRecord> {
        let record = self.record_mut(space_id)?;
        anyhow::ensure!(record.role == SpaceRole::Member, "only joined spaces can accept invites");
        anyhow::ensure!(!record.left, "space has already been left locally: {}", space_id);
        let invite = record
            .invite_state
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("space has no invite state: {}", space_id))?;
        anyhow::ensure!(invite.status == SpaceInviteStatus::Pending, "invite is not pending");
        invite.status = SpaceInviteStatus::Accepted;

        let space = space_mut(spaces, space_id)?;
        space.members.push(local_device_id.to_string());
        space.members.sort();
        space.members.dedup();
        space.active = false;
        Ok(space.clone())
    }

    pub fn decline_invite(&mut self, spaces: &mut SpaceStore, space_id: &str) -> Result<SpaceRecord> {
        let record = self.record_mut(space_id)?;
        anyhow::ensure!(record.role == SpaceRole::Member, "only joined spaces can decline invites");
        if let Some(invite) = record.invite_state.as_mut() {
            invite.status = SpaceInviteStatus::Declined;
        }
        record.left = true;
        let space = space_mut(spaces, space_id)?;
        space.active = false;
        Ok(space.clone())
    }

    pub fn record_member_acceptance(
        &mut self,
        spaces: &mut SpaceStore,
        local_device_id: &str,
        space_id: &str,
        peer_id: &str,
    ) -> Result<SpaceSyncUpdate> {
        let peer_id = peer_id.trim();
        anyhow::ensure!(!peer_id.is_empty(), "peer_id cannot be empty");
        self.ensure_owner(local_device_id, space_id)?;

        for invite in self
            .invites
            .iter_mut()
            .filter(|invite| invite.space_id == space_id && invite.target_peer_id == peer_id)
        {
            invite.status = SpaceInviteStatus::Accepted;
        }
        let space = space_mut(spaces, space_id)?;
        space.members.push(peer_id.to_string());
        space.members.sort();
        space.members.dedup();
        bump(self.record_mut(space_id)?);
        self.sync_update(spaces, local_device_id, space_id)
    }

    pub fn leave_space(
        &mut self,
        spaces: &mut SpaceStore,
        local_device_id: &str,
        space_id: &str,
    ) -> Result<SpaceRecord> {
        if self
            .records
            .get(space_id)
            .map(|record| record.is_owner_for(local_device_id))
            .unwrap_or(false)
        {
            anyhow::bail!("owner cannot leave their own space; delete/remove is separate");
        }
        let record = self.record_mut(space_id)?;
        record.left = true;
        record.key_epoch = 0;
        record.invite_state = None;
        let space = space_mut(spaces, space_id)?;
        space.active = false;
        space.addons.clear();
        space.members.retain(|member| member != local_device_id);
        Ok(space.clone())
    }

    pub fn apply_owner_update(
        &mut self,
        spaces: &mut SpaceStore,
        update: SpaceSyncUpdate,
    ) -> Result<Option<SpaceRecord>> {
        let record = self.record_mut(&update.space_id)?;
        anyhow::ensure!(record.role == SpaceRole::Member, "only joined spaces apply owner updates");
        anyhow::ensure!(record.owner_device_id == update.owner_device_id, "owner mismatch");
        if record.left || update.revision <= record.revision {
            return Ok(None);
        }
        record.revision = update.revision;
        record.owner_enabled = update.owner_enabled;
        record.key_epoch = update.key_epoch;
        let space = space_mut(spaces, &update.space_id)?;
        space.name = update.name;
        space.kind = update.kind;
        space.members = update.members;
        Ok(Some(space.clone()))
    }

    pub fn pending_joined_invites(&self, spaces: &SpaceStore) -> Vec<SpaceRecord> {
        spaces
            .spaces
            .iter()
            .filter(|space| {
                self.records
                    .get(&space.space_id)
                    .and_then(|record| record.invite_state.as_ref())
                    .map(|invite| invite.status == SpaceInviteStatus::Pending)
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    pub fn sync_update(&self, spaces: &SpaceStore, local_device_id: &str, space_id: &str) -> Result<SpaceSyncUpdate> {
        self.ensure_owner(local_device_id, space_id)?;
        let space = ensure_space_exists(spaces, space_id)?;
        let record = self.record(space_id)?;
        Ok(SpaceSyncUpdate {
            message_type: "space_update".to_string(),
            space_id: space.space_id.clone(),
            owner_device_id: record.owner_device_id.clone(),
            revision: record.revision,
            name: space.name.clone(),
            kind: space.kind.clone(),
            owner_enabled: record.owner_enabled,
            members: space.members.clone(),
            key_epoch: record.key_epoch,
        })
    }

    fn ensure_owner(&self, local_device_id: &str, space_id: &str) -> Result<()> {
        let record = self.record(space_id)?;
        anyhow::ensure!(record.is_owner_for(local_device_id), "only the owner can mutate owner-authoritative space state");
        Ok(())
    }

    fn record(&self, space_id: &str) -> Result<&SpaceMembershipRecord> {
        self.records
            .get(space_id)
            .ok_or_else(|| anyhow::anyhow!("space has no membership metadata: {}", space_id))
    }

    fn record_mut(&mut self, space_id: &str) -> Result<&mut SpaceMembershipRecord> {
        self.records
            .get_mut(space_id)
            .ok_or_else(|| anyhow::anyhow!("space has no membership metadata: {}", space_id))
    }
}

pub fn load_or_create_space_membership_store() -> Result<SpaceMembershipStore> {
    init_app_dirs()?;
    let path = space_membership_path()?;
    if !path.exists() {
        let store = SpaceMembershipStore::default();
        save_space_membership_store(&store)?;
        return Ok(store);
    }
    let text = fs::read_to_string(&path)
        .with_context(|| format!("failed to read space membership store: {}", path.display()))?;
    let store = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse space membership store: {}", path.display()))?;
    Ok(store)
}

pub fn save_space_membership_store(store: &SpaceMembershipStore) -> Result<()> {
    init_app_dirs()?;
    let text = serde_json::to_string_pretty(store)?;
    atomic_write(&space_membership_path()?, text.as_bytes())?;
    Ok(())
}

pub fn space_membership_path() -> Result<PathBuf> {
    Ok(state_dir()?.join("space-membership.json"))
}

fn ensure_space_exists<'a>(spaces: &'a SpaceStore, space_id: &str) -> Result<&'a SpaceRecord> {
    spaces
        .spaces
        .iter()
        .find(|space| space.space_id == space_id)
        .ok_or_else(|| anyhow::anyhow!("unknown space: {}", space_id))
}

fn space_mut<'a>(spaces: &'a mut SpaceStore, space_id: &str) -> Result<&'a mut SpaceRecord> {
    spaces
        .spaces
        .iter_mut()
        .find(|space| space.space_id == space_id)
        .ok_or_else(|| anyhow::anyhow!("unknown space: {}", space_id))
}

fn bump(record: &mut SpaceMembershipRecord) {
    record.revision = record.revision.saturating_add(1).max(1);
}

fn normalize_invites(invites: &mut Vec<SpaceInviteRecord>) -> Result<()> {
    for invite in invites.iter_mut() {
        invite.invite_id = invite.invite_id.trim().to_string();
        invite.space_id = invite.space_id.trim().to_string();
        invite.target_peer_id = invite.target_peer_id.trim().to_string();
        invite.invited_by = invite.invited_by.trim().to_string();
        anyhow::ensure!(!invite.invite_id.is_empty(), "invite_id cannot be empty");
        anyhow::ensure!(!invite.space_id.is_empty(), "invite space_id cannot be empty");
        anyhow::ensure!(!invite.target_peer_id.is_empty(), "invite target_peer_id cannot be empty");
        anyhow::ensure!(!invite.invited_by.is_empty(), "invite invited_by cannot be empty");
        if invite.revision == 0 {
            invite.revision = 1;
        }
    }
    invites.sort_by(|left, right| {
        left.space_id
            .cmp(&right.space_id)
            .then(left.target_peer_id.cmp(&right.target_peer_id))
            .then(left.invite_id.cmp(&right.invite_id))
    });
    invites.dedup_by(|left, right| left.invite_id == right.invite_id);
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

    fn spaces() -> SpaceStore {
        SpaceStore {
            spaces: vec![SpaceRecord {
                space_id: "office".to_string(),
                name: "Office".to_string(),
                kind: SpaceKind::Group,
                active: false,
                members: Vec::new(),
                addons: HashMap::new(),
            }],
        }
    }

    #[test]
    fn owner_invite_does_not_auto_add_member() {
        let spaces = spaces();
        let mut store = SpaceMembershipStore::default();
        store.ensure_owned_space(&spaces, "office", "owner").unwrap();

        let invite = store
            .create_invite(&spaces, "owner", "office", "laptop")
            .unwrap();

        assert_eq!(invite.target_peer_id, "laptop");
        assert_eq!(store.invites.len(), 1);
        assert!(spaces.spaces[0].members.is_empty());
    }

    #[test]
    fn non_owner_cannot_invite() {
        let spaces = spaces();
        let mut store = SpaceMembershipStore::default();
        store.ensure_owned_space(&spaces, "office", "owner").unwrap();

        assert!(store
            .create_invite(&spaces, "laptop", "office", "phone")
            .is_err());
    }

    #[test]
    fn imported_invite_requires_acceptance() {
        let mut spaces = SpaceStore::default();
        let mut store = SpaceMembershipStore::default();
        store
            .import_invite(
                &mut spaces,
                "laptop",
                ImportedSpaceInvite {
                    space_id: "office".to_string(),
                    name: "Office".to_string(),
                    kind: SpaceKind::Group,
                    owner_device_id: "owner".to_string(),
                    invite_id: "invite-1".to_string(),
                    revision: 7,
                    owner_enabled: true,
                    members: vec!["owner".to_string()],
                    key_epoch: 3,
                },
            )
            .unwrap();

        let record = store.records.get("office").unwrap();
        assert_eq!(record.role, SpaceRole::Member);
        assert_eq!(record.revision, 7);
        assert_eq!(record.key_epoch, 3);
        assert_eq!(
            record.invite_state.as_ref().map(|state| &state.status),
            Some(&SpaceInviteStatus::Pending)
        );
    }

    #[test]
    fn accepting_invite_adds_local_member_to_cached_space() {
        let mut spaces = SpaceStore::default();
        let mut store = SpaceMembershipStore::default();
        store
            .import_invite(
                &mut spaces,
                "laptop",
                ImportedSpaceInvite {
                    space_id: "office".to_string(),
                    name: "Office".to_string(),
                    kind: SpaceKind::Group,
                    owner_device_id: "owner".to_string(),
                    invite_id: "invite-1".to_string(),
                    revision: 1,
                    owner_enabled: true,
                    members: vec!["owner".to_string()],
                    key_epoch: 1,
                },
            )
            .unwrap();

        let space = store.accept_invite(&mut spaces, "laptop", "office").unwrap();

        assert!(space.members.iter().any(|member| member == "laptop"));
        assert_eq!(
            store
                .records
                .get("office")
                .and_then(|record| record.invite_state.as_ref())
                .map(|state| &state.status),
            Some(&SpaceInviteStatus::Accepted)
        );
    }

    #[test]
    fn owner_records_acceptance_and_gets_update_snapshot() {
        let mut spaces = spaces();
        let mut store = SpaceMembershipStore::default();
        store.ensure_owned_space(&spaces, "office", "owner").unwrap();
        store
            .create_invite(&spaces, "owner", "office", "laptop")
            .unwrap();

        let update = store
            .record_member_acceptance(&mut spaces, "owner", "office", "laptop")
            .unwrap();

        assert!(spaces.spaces[0].members.iter().any(|member| member == "laptop"));
        assert_eq!(store.invites[0].status, SpaceInviteStatus::Accepted);
        assert_eq!(update.message_type, "space_update");
    }

    #[test]
    fn leaving_space_clears_local_runtime_state() {
        let mut spaces = SpaceStore::default();
        let mut store = SpaceMembershipStore::default();
        store
            .import_invite(
                &mut spaces,
                "laptop",
                ImportedSpaceInvite {
                    space_id: "office".to_string(),
                    name: "Office".to_string(),
                    kind: SpaceKind::Group,
                    owner_device_id: "owner".to_string(),
                    invite_id: "invite-1".to_string(),
                    revision: 1,
                    owner_enabled: true,
                    members: vec!["owner".to_string(), "laptop".to_string()],
                    key_epoch: 1,
                },
            )
            .unwrap();
        store.accept_invite(&mut spaces, "laptop", "office").unwrap();
        spaces.spaces[0].active = true;
        spaces.spaces[0]
            .addons
            .insert("clipboard".to_string(), crate::config::spaces::SpaceAddonState { enabled: true });

        let space = store.leave_space(&mut spaces, "laptop", "office").unwrap();
        let record = store.records.get("office").unwrap();

        assert!(record.left);
        assert_eq!(record.key_epoch, 0);
        assert!(!space.active);
        assert!(space.addons.is_empty());
    }
}
