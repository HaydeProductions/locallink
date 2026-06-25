use crate::addons::AddonRecord;
use crate::config::spaces::{SpaceKind, SpaceStore};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SpaceAddonInstancePlan {
    pub instance_id: String,
    pub space_id: String,
    pub space_name: String,
    pub space_kind: SpaceKind,
    pub addon_id: String,
    pub addon_name: String,
    pub executable: String,
    pub connected_members: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SpaceAddonRuntimeContext {
    pub instance_id: String,
    pub addon_id: String,
    pub executable: String,
    pub env: BTreeMap<String, String>,
}

impl SpaceAddonInstancePlan {
    pub fn runtime_context(&self, core_api_addr: &str) -> SpaceAddonRuntimeContext {
        let mut env = BTreeMap::new();
        env.insert("LOCALLINK_ADDON_ID".to_string(), self.addon_id.clone());
        env.insert(
            "LOCALLINK_ADDON_INSTANCE_ID".to_string(),
            self.instance_id.clone(),
        );
        env.insert("LOCALLINK_CORE_API_ADDR".to_string(), core_api_addr.to_string());
        env.insert("LOCALLINK_SPACE_ID".to_string(), self.space_id.clone());
        env.insert("LOCALLINK_SPACE_KIND".to_string(), space_kind_env(&self.space_kind));
        env.insert("LOCALLINK_SPACE_NAME".to_string(), self.space_name.clone());

        SpaceAddonRuntimeContext {
            instance_id: self.instance_id.clone(),
            addon_id: self.addon_id.clone(),
            executable: self.executable.clone(),
            env,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SpaceAddonSyncPlan {
    pub start: Vec<SpaceAddonInstancePlan>,
    pub keep: Vec<SpaceAddonInstancePlan>,
    pub stop: Vec<String>,
}

pub fn plan_space_addon_instances(
    store: &SpaceStore,
    addons: &[AddonRecord],
    connected_peer_ids: &HashSet<String>,
) -> Vec<SpaceAddonInstancePlan> {
    let addons_by_id: HashMap<&str, &AddonRecord> = addons
        .iter()
        .map(|addon| (addon.id.as_str(), addon))
        .collect();

    let mut plans = Vec::new();

    for activation in store.activation_states(connected_peer_ids) {
        if !activation.active {
            continue;
        }

        let Some(space) = store
            .spaces
            .iter()
            .find(|space| space.space_id == activation.space_id)
        else {
            continue;
        };

        let mut desired_addons: Vec<_> = space.addons.iter().collect();
        desired_addons.sort_by(|(left, _), (right, _)| left.cmp(right));

        for (addon_id, desired_state) in desired_addons {
            if !desired_state.enabled {
                continue;
            }

            let Some(addon) = addons_by_id.get(addon_id.as_str()) else {
                continue;
            };

            plans.push(SpaceAddonInstancePlan {
                instance_id: format!("{}:{}", space.space_id, addon.id),
                space_id: space.space_id.clone(),
                space_name: space.name.clone(),
                space_kind: space.kind.clone(),
                addon_id: addon.id.clone(),
                addon_name: addon.name.clone(),
                executable: addon.executable.clone(),
                connected_members: activation.connected_members.clone(),
            });
        }
    }

    plans.sort_by(|left, right| {
        left.space_id
            .cmp(&right.space_id)
            .then(left.addon_id.cmp(&right.addon_id))
    });
    plans
}

pub fn plan_space_addon_sync(
    desired: &[SpaceAddonInstancePlan],
    running_instance_ids: &HashSet<String>,
) -> SpaceAddonSyncPlan {
    let desired_ids: HashSet<String> = desired
        .iter()
        .map(|plan| plan.instance_id.clone())
        .collect();

    let mut start = Vec::new();
    let mut keep = Vec::new();

    for plan in desired {
        if running_instance_ids.contains(&plan.instance_id) {
            keep.push(plan.clone());
        } else {
            start.push(plan.clone());
        }
    }

    let mut stop: Vec<String> = running_instance_ids
        .iter()
        .filter(|instance_id| !desired_ids.contains(*instance_id))
        .cloned()
        .collect();

    stop.sort();

    SpaceAddonSyncPlan { start, keep, stop }
}

fn space_kind_env(kind: &SpaceKind) -> String {
    match kind {
        SpaceKind::Direct => "direct".to_string(),
        SpaceKind::Group => "group".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::spaces::{SpaceAddonState, SpaceRecord};

    fn addon(id: &str) -> AddonRecord {
        AddonRecord {
            id: id.to_string(),
            name: format!("{id} add-on"),
            version: "1.0.0".to_string(),
            description: String::new(),
            executable: format!("{id}.exe"),
            services: Vec::new(),
            enabled: false,
            manifest_path: format!("addons/{id}/manifest.json"),
            addon_dir: format!("addons/{id}"),
        }
    }

    fn store_with_space(kind: SpaceKind, members: Vec<&str>) -> SpaceStore {
        let mut addons = HashMap::new();
        addons.insert("clipboard".to_string(), SpaceAddonState { enabled: true });

        SpaceStore {
            spaces: vec![SpaceRecord {
                space_id: "office".to_string(),
                name: "Office".to_string(),
                kind,
                members: members.into_iter().map(str::to_string).collect(),
                addons,
            }],
        }
    }

    fn env_value<'a>(context: &'a SpaceAddonRuntimeContext, key: &str) -> Option<&'a str> {
        context.env.get(key).map(String::as_str)
    }

    #[test]
    fn plans_enabled_addons_for_active_spaces() {
        let store = store_with_space(SpaceKind::Group, vec!["desktop", "laptop"]);
        let addons = vec![addon("clipboard")];
        let connected = HashSet::from(["desktop".to_string()]);

        let plans = plan_space_addon_instances(&store, &addons, &connected);

        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].instance_id, "office:clipboard");
        assert_eq!(plans[0].connected_members, vec!["desktop".to_string()]);
    }

    #[test]
    fn runtime_context_sets_space_and_addon_env() {
        let store = store_with_space(SpaceKind::Group, vec!["desktop"]);
        let addons = vec![addon("clipboard")];
        let connected = HashSet::from(["desktop".to_string()]);
        let plans = plan_space_addon_instances(&store, &addons, &connected);

        let context = plans[0].runtime_context("127.0.0.1:17345");

        assert_eq!(context.instance_id, "office:clipboard");
        assert_eq!(context.addon_id, "clipboard");
        assert_eq!(context.executable, "clipboard.exe");
        assert_eq!(env_value(&context, "LOCALLINK_ADDON_ID"), Some("clipboard"));
        assert_eq!(
            env_value(&context, "LOCALLINK_ADDON_INSTANCE_ID"),
            Some("office:clipboard")
        );
        assert_eq!(
            env_value(&context, "LOCALLINK_CORE_API_ADDR"),
            Some("127.0.0.1:17345")
        );
        assert_eq!(env_value(&context, "LOCALLINK_SPACE_ID"), Some("office"));
        assert_eq!(env_value(&context, "LOCALLINK_SPACE_KIND"), Some("group"));
        assert_eq!(env_value(&context, "LOCALLINK_SPACE_NAME"), Some("Office"));
    }

    #[test]
    fn skips_inactive_spaces() {
        let store = store_with_space(SpaceKind::Direct, vec!["desktop"]);
        let addons = vec![addon("clipboard")];
        let connected = HashSet::new();

        let plans = plan_space_addon_instances(&store, &addons, &connected);

        assert!(plans.is_empty());
    }

    #[test]
    fn skips_disabled_desired_addons() {
        let mut store = store_with_space(SpaceKind::Group, vec!["desktop"]);
        store
            .set_addon_enabled("office", "clipboard", false)
            .unwrap();
        let addons = vec![addon("clipboard")];
        let connected = HashSet::from(["desktop".to_string()]);

        let plans = plan_space_addon_instances(&store, &addons, &connected);

        assert!(plans.is_empty());
    }

    #[test]
    fn skips_missing_addon_manifests() {
        let store = store_with_space(SpaceKind::Group, vec!["desktop"]);
        let addons = Vec::new();
        let connected = HashSet::from(["desktop".to_string()]);

        let plans = plan_space_addon_instances(&store, &addons, &connected);

        assert!(plans.is_empty());
    }

    #[test]
    fn sync_plan_starts_missing_instances() {
        let store = store_with_space(SpaceKind::Group, vec!["desktop"]);
        let addons = vec![addon("clipboard")];
        let connected = HashSet::from(["desktop".to_string()]);
        let desired = plan_space_addon_instances(&store, &addons, &connected);
        let running = HashSet::new();

        let sync = plan_space_addon_sync(&desired, &running);

        assert_eq!(sync.start.len(), 1);
        assert!(sync.keep.is_empty());
        assert!(sync.stop.is_empty());
    }

    #[test]
    fn sync_plan_keeps_matching_instances() {
        let store = store_with_space(SpaceKind::Group, vec!["desktop"]);
        let addons = vec![addon("clipboard")];
        let connected = HashSet::from(["desktop".to_string()]);
        let desired = plan_space_addon_instances(&store, &addons, &connected);
        let running = HashSet::from(["office:clipboard".to_string()]);

        let sync = plan_space_addon_sync(&desired, &running);

        assert!(sync.start.is_empty());
        assert_eq!(sync.keep.len(), 1);
        assert!(sync.stop.is_empty());
    }

    #[test]
    fn sync_plan_stops_orphaned_instances() {
        let desired = Vec::new();
        let running = HashSet::from(["office:clipboard".to_string()]);

        let sync = plan_space_addon_sync(&desired, &running);

        assert!(sync.start.is_empty());
        assert!(sync.keep.is_empty());
        assert_eq!(sync.stop, vec!["office:clipboard".to_string()]);
    }
}
