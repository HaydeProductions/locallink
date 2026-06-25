use crate::config::space_instances::SpaceAddonInstanceSet;
use crate::config::space_runtime::SpaceAddonSyncPlan;
use std::sync::Arc;
use tokio::sync::Mutex;

pub type SharedSpaceAddonInstances = Arc<Mutex<SpaceAddonInstanceSet>>;

pub fn new_shared_space_addon_instances() -> SharedSpaceAddonInstances {
    Arc::new(Mutex::new(SpaceAddonInstanceSet::new()))
}

pub async fn apply_space_addon_sync_plan(
    instances: &SharedSpaceAddonInstances,
    sync_plan: &SpaceAddonSyncPlan,
) {
    instances.lock().await.apply_sync_plan(sync_plan);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::space_runtime::SpaceAddonInstancePlan;
    use crate::config::spaces::SpaceKind;

    fn plan(instance_id: &str) -> SpaceAddonInstancePlan {
        SpaceAddonInstancePlan {
            instance_id: instance_id.to_string(),
            space_id: "office".to_string(),
            space_name: "Office".to_string(),
            space_kind: SpaceKind::Group,
            addon_id: "clipboard".to_string(),
            addon_name: "Clipboard".to_string(),
            executable: "clipboard.exe".to_string(),
            connected_members: Vec::new(),
        }
    }

    #[tokio::test]
    async fn shared_instances_start_empty() {
        let instances = new_shared_space_addon_instances();

        assert!(instances.lock().await.is_empty());
    }

    #[tokio::test]
    async fn shared_instances_apply_sync_plan() {
        let instances = new_shared_space_addon_instances();
        instances.lock().await.mark_present("office:old");
        let sync_plan = SpaceAddonSyncPlan {
            start: vec![plan("office:clipboard")],
            keep: Vec::new(),
            stop: vec!["office:old".to_string()],
        };

        apply_space_addon_sync_plan(&instances, &sync_plan).await;

        let ids = instances.lock().await.sorted_ids();
        assert_eq!(ids, vec!["office:clipboard".to_string()]);
    }
}
