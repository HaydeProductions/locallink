use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src/api.rs");
    println!("cargo:rerun-if-changed=src/space_membership.rs");
    patch_membership();
    patch_api();
}

fn patch_membership() {
    let path = Path::new("src/space_membership.rs");
    let mut text = read(path);
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

    if !text.contains("pub fn purge_owned_space(") {
        text = text.replace(
            "    pub fn leave_space(\n",
            r#"    pub fn purge_owned_space(
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
            message_type: "space_purged".to_string(),
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
            anyhow::bail!("owner cannot leave their own space; use the space removal action");
        }
        let record = self.record(space_id)?;
        anyhow::ensure!(record.role == SpaceRole::Member, "only joined spaces can be left");
        let space = ensure_space_exists(spaces, space_id)?.clone();
        self.purge_local_space(spaces, space_id);
        Ok(space)
    }
"#,
    );

    text = text.replace(
        r#"        let space = space_mut(spaces, &update.space_id)?;
        space.name = update.name;
        space.kind = update.kind;
        space.members = update.members;
        if removed_by_owner {
            space.active = false;
            space.addons.clear();
        }
        Ok(Some(space.clone()))
"#,
        r#"        if update.message_type == "space_purged" {
            return Ok(self.purge_local_space(spaces, &update.space_id));
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
"#,
    );

    if text != original {
        fs::write(path, text).expect("write membership source");
    }
}

fn patch_api() {
    let path = Path::new("src/api.rs");
    let mut text = read(path);
    let original = text.clone();

    if !text.contains("\"purge_space\" => {") {
        text = text.replace(
            "        \"list_space_invites\" => {\n",
            r#"        "purge_space" => {
            let space_id = req
                .space_id
                .ok_or_else(|| anyhow::anyhow!("purge_space requires space_id"))?;

            let mut store = spaces.lock().await;
            let mut membership = load_or_create_space_membership_store()?;
            membership.ensure_local_records(&store, &cfg.device_id);

            let local_only = membership
                .records
                .get(&space_id)
                .map(|record| {
                    !record.is_owner_for(&cfg.device_id)
                        && (record.left || record.key_epoch == 0 || !record.owner_enabled)
                })
                .unwrap_or(false);

            if local_only {
                let removed = membership
                    .purge_local_space(&mut store, &space_id)
                    .ok_or_else(|| anyhow::anyhow!("unknown local space: {}", space_id))?;
                membership.validate_and_repair(&mut store)?;
                save_space_store(&store)?;
                save_space_membership_store(&membership)?;

                return Ok(serde_json::to_string(&ok(serde_json::json!({
                    "space_id": space_id,
                    "space": removed,
                    "local_only": true
                })))?);
            }

            let (removed, update) = membership.purge_owned_space(&mut store, &cfg.device_id, &space_id)?;
            let deliveries = broadcast_update(connections.clone(), update.clone(), None).await;
            membership.validate_and_repair(&mut store)?;
            save_space_store(&store)?;
            save_space_membership_store(&membership)?;

            Ok(serde_json::to_string(&ok(serde_json::json!({
                "space_id": space_id,
                "space": removed,
                "space_update": update,
                "deliveries": deliveries,
                "local_only": false
            })))?)
        }

        "list_space_invites" => {
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
