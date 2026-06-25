use crate::config::core_state::CoreRuntimeState;
use crate::config::space_instances::space_instance_state::SharedSpaceAddonInstances;
use crate::config::space_runtime::SpaceAddonSyncPlan;
use crate::config::space_sync::plan_space_addon_delta_from_state;
use std::collections::HashSet;

pub async fn plan_space_addons_for_core_state(
    state: &CoreRuntimeState,
    current_instances: &SharedSpaceAddonInstances,
) -> SpaceAddonSyncPlan {
    let connected_peer_ids: HashSet<String> = state.connections.lock().await.keys().cloned().collect();
    let spaces = state.spaces.lock().await.clone();
    let addons = state.addons.lock().await.clone();
    let current_instance_ids = current_instances.lock().await.snapshot();

    plan_space_addon_delta_from_state(&spaces, &addons, &connected_peer_ids, &current_instance_ids)
}
