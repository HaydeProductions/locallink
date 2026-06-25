use crate::addons::AddonRecord;
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
