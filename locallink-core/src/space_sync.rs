use crate::addons::AddonRecord;
use crate::config::core_state::CoreRuntimeState;
use crate::config::space_instances::space_instance_state::apply_space_addon_sync_plan;
use crate::config::space_runtime::{
    plan_space_addon_instances, plan_space_addon_sync, SpaceAddonSyncPlan,
};
use crate::config::spaces::SpaceStore;
use std::collections::HashSet;

pub fn plan_space_addon_delta_from_state(
    spaces: &SpaceStore,
    addons: &[AddonRecord],
    connected_peer_ids: &HashSet<String>,
    current_instance_ids: &HashSet<String>,
) -> SpaceAddonSyncPlan {
    let desired = plan_space_addon_instances(spaces, addons, connected_peer_ids);
    plan_space_addon_sync(&desired, current_instance_ids)
}

pub async fn plan_space_addon_delta_from_core_state(
    state: &CoreRuntimeState,
) -> SpaceAddonSyncPlan {
    let connected_peer_ids: HashSet<String> = {
        let connections = state.connections.lock().await;
        connections.keys().cloned().collect()
    };
    let spaces = state.spaces.lock().await.clone();
    let addons = state.addons.lock().await.clone();
    let current_instance_ids = state.space_addon_instances.lock().await.snapshot();

    plan_space_addon_delta_from_state(&spaces, &addons, &connected_peer_ids, &current_instance_ids)
}

pub async fn apply_space_addon_delta_to_core_state(
    state: &CoreRuntimeState,
    sync_plan: &SpaceAddonSyncPlan,
) {
    apply_space_addon_sync_plan(&state.space_addon_instances, sync_plan).await;
}
