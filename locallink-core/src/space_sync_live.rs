use crate::config::space_membership::{
    load_or_create_space_membership_store, save_space_membership_store, ImportedSpaceInvite,
    SpaceMembershipStore, SpaceSyncUpdate, SPACE_SYNC_SERVICE,
};
use crate::config::spaces::{save_space_store, SpaceRecord, SpaceRegistry, SpaceStore};
use crate::config::{load_or_create_config, Config};
use crate::discovery::send_core_space_service_message;
use crate::transport::{take_events, ApiEvent, ConnectionRegistry, EventQueue};
use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SpaceSyncMessage {
    Invite { invite: ImportedSpaceInvite },
    InviteAccept { space_id: String, peer_id: String },
    InviteDecline { space_id: String, peer_id: String },
    Leave { space_id: String, peer_id: String },
    Update { update: SpaceSyncUpdate },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SyncDeliveryResult {
    pub peer_id: String,
    pub ok: bool,
    pub message_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct SpaceSyncApplyReport {
    pub applied: usize,
    pub ignored: usize,
    pub errors: Vec<String>,
}

pub fn encode_sync_message(message: &SpaceSyncMessage) -> Result<String> {
    Ok(STANDARD.encode(serde_json::to_vec(message)?))
}

pub fn decode_sync_message(data_b64: &str) -> Result<SpaceSyncMessage> {
    let bytes = STANDARD.decode(data_b64)?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub async fn send_sync_message(
    _connections: ConnectionRegistry,
    peer_id: &str,
    space_id: &str,
    message: SpaceSyncMessage,
) -> SyncDeliveryResult {
    let data_b64 = match encode_sync_message(&message) {
        Ok(data_b64) => data_b64,
        Err(err) => {
            return SyncDeliveryResult {
                peer_id: peer_id.to_string(),
                ok: false,
                message_id: None,
                error: Some(err.to_string()),
            };
        }
    };

    let cfg = match load_or_create_config() {
        Ok(cfg) => cfg,
        Err(err) => {
            return SyncDeliveryResult {
                peer_id: peer_id.to_string(),
                ok: false,
                message_id: None,
                error: Some(err.to_string()),
            };
        }
    };

    match send_core_space_service_message(&cfg, peer_id, space_id, SPACE_SYNC_SERVICE, &data_b64)
        .await
    {
        Ok(message_id) => SyncDeliveryResult {
            peer_id: peer_id.to_string(),
            ok: true,
            message_id: Some(message_id),
            error: None,
        },
        Err(err) => SyncDeliveryResult {
            peer_id: peer_id.to_string(),
            ok: false,
            message_id: None,
            error: Some(err.to_string()),
        },
    }
}

pub async fn send_invite(
    connections: ConnectionRegistry,
    peer_id: &str,
    invite: ImportedSpaceInvite,
) -> SyncDeliveryResult {
    let space_id = invite.space_id.clone();
    send_sync_message(
        connections,
        peer_id,
        &space_id,
        SpaceSyncMessage::Invite { invite },
    )
    .await
}

pub async fn send_accept(
    connections: ConnectionRegistry,
    owner_device_id: &str,
    space_id: &str,
    local_device_id: &str,
) -> SyncDeliveryResult {
    send_sync_message(
        connections,
        owner_device_id,
        space_id,
        SpaceSyncMessage::InviteAccept {
            space_id: space_id.to_string(),
            peer_id: local_device_id.to_string(),
        },
    )
    .await
}

pub async fn send_leave(
    connections: ConnectionRegistry,
    owner_device_id: &str,
    space_id: &str,
    local_device_id: &str,
) -> SyncDeliveryResult {
    send_sync_message(
        connections,
        owner_device_id,
        space_id,
        SpaceSyncMessage::Leave {
            space_id: space_id.to_string(),
            peer_id: local_device_id.to_string(),
        },
    )
    .await
}

pub async fn broadcast_update(
    connections: ConnectionRegistry,
    update: SpaceSyncUpdate,
    exclude: Option<&str>,
) -> Vec<SyncDeliveryResult> {
    let mut results = Vec::new();
    let mut seen = HashSet::<String>::new();

    for peer_id in &update.members {
        if peer_id == &update.owner_device_id {
            continue;
        }
        if exclude.map(|excluded| excluded == peer_id).unwrap_or(false) {
            continue;
        }
        if !seen.insert(peer_id.clone()) {
            continue;
        }

        results.push(
            send_sync_message(
                connections.clone(),
                peer_id,
                &update.space_id,
                SpaceSyncMessage::Update {
                    update: update.clone(),
                },
            )
            .await,
        );
    }

    results
}

pub fn imported_invite_from_space(
    space: &SpaceRecord,
    owner_device_id: &str,
    invite_id: String,
    revision: u64,
    owner_enabled: bool,
    key_epoch: u64,
) -> ImportedSpaceInvite {
    ImportedSpaceInvite {
        space_id: space.space_id.clone(),
        name: space.name.clone(),
        kind: space.kind.clone(),
        owner_device_id: owner_device_id.to_string(),
        invite_id,
        revision,
        owner_enabled,
        members: space.members.clone(),
        key_epoch,
    }
}

pub async fn apply_pending_space_sync_events(
    local_device_id: &str,
    spaces: &mut SpaceStore,
    membership: &mut SpaceMembershipStore,
    events: EventQueue,
    connections: ConnectionRegistry,
) -> SpaceSyncApplyReport {
    let incoming = take_events(
        events,
        "__locallink_space_sync__",
        Some(SPACE_SYNC_SERVICE),
        100,
    )
    .await;
    let mut report = SpaceSyncApplyReport::default();

    for event in incoming {
        match apply_event(local_device_id, spaces, membership, connections.clone(), event).await {
            Ok(true) => report.applied += 1,
            Ok(false) => report.ignored += 1,
            Err(err) => report.errors.push(err.to_string()),
        }
    }

    if report.applied > 0 {
        if let Err(err) = save_space_store(spaces) {
            report.errors.push(err.to_string());
        }
        if let Err(err) = save_space_membership_store(membership) {
            report.errors.push(err.to_string());
        }
    }

    report
}

pub async fn apply_core_space_sync_message(
    cfg: &Config,
    spaces: SpaceRegistry,
    peer_id: &str,
    _peer_name: &str,
    service: &str,
    data_b64: &str,
) -> Result<bool> {
    if service != SPACE_SYNC_SERVICE {
        return Ok(false);
    }

    let message = decode_sync_message(data_b64)?;
    let mut store = spaces.lock().await;
    let mut membership = load_or_create_space_membership_store()?;

    let applied =
        apply_message(&cfg.device_id, &mut store, &mut membership, peer_id, message).await?;

    if applied {
        save_space_store(&store)?;
        save_space_membership_store(&membership)?;
    }

    Ok(applied)
}

async fn apply_event(
    local_device_id: &str,
    spaces: &mut SpaceStore,
    membership: &mut SpaceMembershipStore,
    _connections: ConnectionRegistry,
    event: ApiEvent,
) -> Result<bool> {
    if event.kind != "space_service_data" || event.service != SPACE_SYNC_SERVICE {
        return Ok(false);
    }

    let Some(data_b64) = event.data_b64.as_deref() else {
        return Ok(false);
    };

    let message = decode_sync_message(data_b64)?;
    apply_message(local_device_id, spaces, membership, &event.peer_id, message).await
}

async fn apply_message(
    local_device_id: &str,
    spaces: &mut SpaceStore,
    membership: &mut SpaceMembershipStore,
    sender_peer_id: &str,
    message: SpaceSyncMessage,
) -> Result<bool> {
    match message {
        SpaceSyncMessage::Invite { invite } => {
            if invite.owner_device_id != sender_peer_id {
                anyhow::bail!("space invite owner did not match sender");
            }
            if spaces
                .spaces
                .iter()
                .any(|space| space.space_id == invite.space_id)
            {
                return Ok(false);
            }
            membership.import_invite(spaces, local_device_id, invite)?;
            membership.validate_and_repair(spaces)?;
            Ok(true)
        }
        SpaceSyncMessage::InviteAccept { space_id, peer_id } => {
            if peer_id != sender_peer_id {
                anyhow::bail!("space invite acceptance sender did not match peer_id");
            }
            let update = membership.record_member_acceptance(
                spaces,
                local_device_id,
                &space_id,
                &peer_id,
            )?;
            membership.validate_and_repair(spaces)?;

            if let Ok(cfg) = load_or_create_config() {
                let _ = broadcast_update_from_config(&cfg, update, None).await;
            }

            Ok(true)
        }
        SpaceSyncMessage::InviteDecline { .. } => Ok(false),
        SpaceSyncMessage::Leave { space_id, peer_id } => {
            if peer_id != sender_peer_id {
                anyhow::bail!("space leave sender did not match peer_id");
            }
            let update = remove_member_after_leave(
                membership,
                spaces,
                local_device_id,
                &space_id,
                &peer_id,
            )?;
            membership.validate_and_repair(spaces)?;
            if let Some(update) = update {
                if let Ok(cfg) = load_or_create_config() {
                    let _ = broadcast_update_from_config(&cfg, update, Some(&peer_id)).await;
                }
                Ok(true)
            } else {
                Ok(false)
            }
        }
        SpaceSyncMessage::Update { update } => {
            if update.owner_device_id != sender_peer_id {
                anyhow::bail!("space update owner did not match sender");
            }
            let applied = membership
                .apply_owner_update(spaces, local_device_id, update)?
                .is_some();
            membership.validate_and_repair(spaces)?;
            Ok(applied)
        }
    }
}

async fn broadcast_update_from_config(
    cfg: &Config,
    update: SpaceSyncUpdate,
    exclude: Option<&str>,
) -> Vec<SyncDeliveryResult> {
    let mut results = Vec::new();
    let mut seen = HashSet::<String>::new();

    for peer_id in &update.members {
        if peer_id == &update.owner_device_id {
            continue;
        }
        if exclude.map(|excluded| excluded == peer_id).unwrap_or(false) {
            continue;
        }
        if !seen.insert(peer_id.clone()) {
            continue;
        }

        let data_b64 = match encode_sync_message(&SpaceSyncMessage::Update {
            update: update.clone(),
        }) {
            Ok(data_b64) => data_b64,
            Err(err) => {
                results.push(SyncDeliveryResult {
                    peer_id: peer_id.clone(),
                    ok: false,
                    message_id: None,
                    error: Some(err.to_string()),
                });
                continue;
            }
        };

        match send_core_space_service_message(
            cfg,
            peer_id,
            &update.space_id,
            SPACE_SYNC_SERVICE,
            &data_b64,
        )
        .await
        {
            Ok(message_id) => results.push(SyncDeliveryResult {
                peer_id: peer_id.clone(),
                ok: true,
                message_id: Some(message_id),
                error: None,
            }),
            Err(err) => results.push(SyncDeliveryResult {
                peer_id: peer_id.clone(),
                ok: false,
                message_id: None,
                error: Some(err.to_string()),
            }),
        }
    }

    results
}

fn remove_member_after_leave(
    membership: &mut SpaceMembershipStore,
    spaces: &mut SpaceStore,
    local_device_id: &str,
    space_id: &str,
    peer_id: &str,
) -> Result<Option<SpaceSyncUpdate>> {
    let Some(record) = membership.records.get(space_id) else {
        return Ok(None);
    };
    if !record.is_owner_for(local_device_id) {
        return Ok(None);
    }

    if let Some(space) = spaces
        .spaces
        .iter_mut()
        .find(|space| space.space_id == space_id)
    {
        let before = space.members.len();
        space.members.retain(|member| member != peer_id);
        if space.members.len() == before {
            return Ok(None);
        }
    }

    if let Some(record) = membership.records.get_mut(space_id) {
        record.revision = record.revision.saturating_add(1).max(1);
    }

    membership
        .sync_update(spaces, local_device_id, space_id)
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::spaces::{SpaceKind, SpaceRecord};

    #[test]
    fn sync_messages_round_trip_through_base64() {
        let message = SpaceSyncMessage::InviteAccept {
            space_id: "office".to_string(),
            peer_id: "laptop".to_string(),
        };

        let encoded = encode_sync_message(&message).unwrap();
        let decoded = decode_sync_message(&encoded).unwrap();

        assert_eq!(decoded, message);
    }

    #[test]
    fn imported_invite_snapshot_uses_space_metadata() {
        let space = SpaceRecord {
            space_id: "office".to_string(),
            name: "Office".to_string(),
            kind: SpaceKind::Group,
            active: false,
            members: vec!["owner".to_string()],
            addons: Default::default(),
        };

        let invite =
            imported_invite_from_space(&space, "owner", "invite-1".to_string(), 2, true, 1);

        assert_eq!(invite.space_id, "office");
        assert_eq!(invite.owner_device_id, "owner");
        assert_eq!(invite.members, vec!["owner".to_string()]);
    }
}
