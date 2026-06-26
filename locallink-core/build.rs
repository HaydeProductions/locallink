use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src/api.rs");
    println!("cargo:rerun-if-changed=src/space_membership.rs");
    patch_space_membership();
    patch_api();
}

fn patch_space_membership() {
    let path = Path::new("src/space_membership.rs");
    let mut text = read_source(path);
    let original = text.clone();

    if !text.contains("pub fn purge_local_space(") {
        text = text.replace(
            "    pub fn set_owner_enabled(\n",
            r#"    pub fn purge_local_space(
        &mut self,
        spaces: &mut SpaceStore,
        space_id: &str,
    ) -> Option<SpaceRecord> {
        let removed = spaces
            .spaces
            .iter()
            .find(|space| space.space_id == space_id)
            .cloned();
        spaces.spaces.retain(|space| space.space_id != space_id);
        self.records.remove(space_id);
        self.invites.retain(|invite| invite.space_id != space_id);
        removed
    }

    pub fn set_owner_enabled(
"#,
        );
    }

    text = text.replace(
        r#"    pub fn decline_invite(
        &mut self,
        spaces: &mut SpaceStore,
        space_id: &str,
    ) -> Result<SpaceRecord> {
        let record = self.record_mut(space_id)?;
        anyhow::ensure!(
            record.role == SpaceRole::Member,
            "only joined spaces can decline invites"
        );
        if let Some(invite) = record.invite_state.as_mut() {
            invite.status = SpaceInviteStatus::Declined;
        }
        record.left = true;
        let space = space_mut(spaces, space_id)?;
        space.active = false;
        Ok(space.clone())
    }
"#,
        r#"    pub fn decline_invite(
        &mut self,
        spaces: &mut SpaceStore,
        space_id: &str,
    ) -> Result<SpaceRecord> {
        let record = self.record(space_id)?;
        anyhow::ensure!(
            record.role == SpaceRole::Member,
            "only joined spaces can decline invites"
        );
        let space = ensure_space_exists(spaces, space_id)?.clone();
        self.purge_local_space(spaces, space_id);
        Ok(space)
    }
"#,
    );

    if !text.contains("pub fn delete_owned_space(") {
        text = text.replace(
            "    pub fn leave_space(\n",
            r#"    pub fn delete_owned_space(
        &mut self,
        spaces: &mut SpaceStore,
        local_device_id: &str,
        space_id: &str,
    ) -> Result<(SpaceRecord, SpaceSyncUpdate)> {
        self.ensure_owner(local_device_id, space_id)?;
        let space = ensure_space_exists(spaces, space_id)?.clone();
        let (owner_device_id, revision) = {
            let record = self.record_mut(space_id)?;
            record.owner_enabled = false;
            record.key_epoch = 0;
            bump(record);
            (record.owner_device_id.clone(), record.revision)
        };
        let update = SpaceSyncUpdate {
            message_type: "space_deleted".to_string(),
            space_id: space.space_id.clone(),
            owner_device_id,
            revision,
            name: space.name.clone(),
            kind: space.kind.clone(),
            owner_enabled: false,
            members: space.members.clone(),
            key_epoch: 0,
        };
        self.purge_local_space(spaces, space_id);
        Ok((space, update))
    }

    pub fn leave_space(
"#,
        );
    }

    text = text.replace(
        r#"    pub fn leave_space(
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
"#,
        r#"    pub fn leave_space(
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
        let record = self.record(space_id)?;
        anyhow::ensure!(
            record.role == SpaceRole::Member,
            "only joined spaces can be left"
        );
        let space = ensure_space_exists(spaces, space_id)?.clone();
        self.purge_local_space(spaces, space_id);
        Ok(space)
    }
"#,
    );

    text = text.replace(
        r#"    pub fn apply_owner_update(
        &mut self,
        spaces: &mut SpaceStore,
        local_device_id: &str,
        update: SpaceSyncUpdate,
    ) -> Result<Option<SpaceRecord>> {
        {
            let record = self.record(&update.space_id)?;
            anyhow::ensure!(
                record.role == SpaceRole::Member,
                "only joined spaces apply owner updates"
            );
            anyhow::ensure!(
                record.owner_device_id == update.owner_device_id,
                "owner mismatch"
            );
            if record.left || update.revision <= record.revision {
                return Ok(None);
            }
        }

        let was_locally_member = spaces
            .spaces
            .iter()
            .find(|space| space.space_id == update.space_id)
            .map(|space| space.members.iter().any(|member| member == local_device_id))
            .unwrap_or(false);
        let invite_was_accepted = self
            .records
            .get(&update.space_id)
            .and_then(|record| record.invite_state.as_ref())
            .map(|invite| invite.status == SpaceInviteStatus::Accepted)
            .unwrap_or(false);
        let removed_by_owner = (was_locally_member || invite_was_accepted)
            && !update.members.iter().any(|member| member == local_device_id);

        {
            let record = self.record_mut(&update.space_id)?;
            record.revision = update.revision;
            record.owner_enabled = if removed_by_owner { false } else { update.owner_enabled };
            record.key_epoch = if removed_by_owner { 0 } else { update.key_epoch };
            if removed_by_owner {
                record.left = true;
                record.invite_state = None;
            }
        }

        let space = space_mut(spaces, &update.space_id)?;
        space.name = update.name;
        space.kind = update.kind;
        space.members = update.members;
        if removed_by_owner {
            space.active = false;
            space.addons.clear();
        }
        Ok(Some(space.clone()))
    }
"#,
        r#"    pub fn apply_owner_update(
        &mut self,
        spaces: &mut SpaceStore,
        local_device_id: &str,
        update: SpaceSyncUpdate,
    ) -> Result<Option<SpaceRecord>> {
        let Some(existing_record) = self.records.get(&update.space_id) else {
            return Ok(None);
        };
        anyhow::ensure!(
            existing_record.role == SpaceRole::Member,
            "only joined spaces apply owner updates"
        );
        anyhow::ensure!(
            existing_record.owner_device_id == update.owner_device_id,
            "owner mismatch"
        );
        let deletion_update = update.message_type == "space_deleted"
            || !update.owner_enabled
            || update.key_epoch == 0;
        if existing_record.left && !deletion_update {
            return Ok(None);
        }
        if update.revision <= existing_record.revision && !deletion_update {
            return Ok(None);
        }

        let was_locally_member = spaces
            .spaces
            .iter()
            .find(|space| space.space_id == update.space_id)
            .map(|space| space.members.iter().any(|member| member == local_device_id))
            .unwrap_or(false);
        let invite_was_accepted = existing_record
            .invite_state
            .as_ref()
            .map(|invite| invite.status == SpaceInviteStatus::Accepted)
            .unwrap_or(false);
        let removed_by_owner = (was_locally_member || invite_was_accepted)
            && !update.members.iter().any(|member| member == local_device_id);

        if deletion_update || removed_by_owner {
            return Ok(self.purge_local_space(spaces, &update.space_id));
        }

        {
            let record = self.record_mut(&update.space_id)?;
            record.revision = update.revision;
            record.owner_enabled = update.owner_enabled;
            record.key_epoch = update.key_epoch;
        }

        let space = space_mut(spaces, &update.space_id)?;
        space.name = update.name;
        space.kind = update.kind;
        space.members = update.members;
        Ok(Some(space.clone()))
    }
"#,
    );

    if text != original {
        fs::write(path, text).expect("write patched space_membership.rs");
    }
}

fn patch_api() {
    let path = Path::new("src/api.rs");
    let mut text = read_source(path);
    let original = text.clone();

    if !text.contains("\"delete_space\" => {") {
        text = text.replace(
            "        \"list_space_invites\" => {\n",
            r#"        "delete_space" => {
            let space_id = req
                .space_id
                .ok_or_else(|| anyhow::anyhow!("delete_space requires space_id"))?;

            let mut store = spaces.lock().await;
            let mut membership = load_or_create_space_membership_store()?;
            membership.ensure_local_records(&store, &cfg.device_id);
            let (deleted_space, update) =
                membership.delete_owned_space(&mut store, &cfg.device_id, &space_id)?;
            let deliveries = broadcast_update(connections.clone(), update.clone(), None).await;
            membership.validate_and_repair(&mut store)?;
            save_space_store(&store)?;
            save_space_membership_store(&membership)?;

            Ok(serde_json::to_string(&ok(serde_json::json!({
                "space_id": space_id,
                "deleted_space": deleted_space,
                "space_update": update,
                "deliveries": deliveries
            })))?)
        }

        "list_space_invites" => {
"#,
        );
    }

    text = text.replace(
        r#"            let response = membership.leave_space(&mut store, &cfg.device_id, &space_id)?;
"#,
        r#"            let response = membership.leave_space(&mut store, &cfg.device_id, &space_id)?;
"#,
    );

    if text != original {
        fs::write(path, text).expect("write patched api.rs");
    }
}

fn read_source(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
        .replace("\r\n", "\n")
}
