use crate::config::space_runtime::{SpaceAddonInstancePlan, SpaceAddonSyncPlan};
use crate::config::spaces::SpaceKind;
use std::collections::HashSet;

#[path = "space_instance_state.rs"]
pub mod space_instance_state;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpaceAddonInstanceSet {
    instance_ids: HashSet<String>,
}

impl SpaceAddonInstanceSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.instance_ids.is_empty()
    }

    pub fn len(&self) -> usize {
        self.instance_ids.len()
    }

    pub fn contains(&self, instance_id: &str) -> bool {
        self.instance_ids.contains(instance_id)
    }

    pub fn mark_present(&mut self, instance_id: impl Into<String>) -> bool {
        let instance_id = instance_id.into().trim().to_string();
        if instance_id.is_empty() {
            return false;
        }

        self.instance_ids.insert(instance_id)
    }

    pub fn mark_absent(&mut self, instance_id: &str) -> bool {
        self.instance_ids.remove(instance_id)
    }

    pub fn apply_sync_plan(&mut self, sync_plan: &SpaceAddonSyncPlan) {
        for instance_id in &sync_plan.stop {
            self.mark_absent(instance_id);
        }

        for plan in sync_plan.start.iter().chain(sync_plan.keep.iter()) {
            self.mark_present(plan.instance_id.clone());
        }
    }

    pub fn replace<I>(&mut self, instance_ids: I)
    where
        I: IntoIterator<Item = String>,
    {
        self.instance_ids = instance_ids
            .into_iter()
            .map(|instance_id| instance_id.trim().to_string())
            .filter(|instance_id| !instance_id.is_empty())
            .collect();
    }

    pub fn snapshot(&self) -> HashSet<String> {
        self.instance_ids.clone()
    }

    pub fn sorted_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.instance_ids.iter().cloned().collect();
        ids.sort();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(instance_id: &str) -> SpaceAddonInstancePlan {
        SpaceAddonInstancePlan {
            instance_id: instance_id.to_string(),
            space_id: "office".to_string(),
            space_name: "Office".to_string(),
            space_kind: SpaceKind::Group,
            addon_id: "clipboard".to_string(),
            addon_name: "Clipboard".to_string(),
            executable: "clipboard.exe".to_string(),
            addon_dir: "addons/clipboard".to_string(),
            connected_members: Vec::new(),
        }
    }

    #[test]
    fn new_set_is_empty() {
        let set = SpaceAddonInstanceSet::new();

        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn mark_present_dedupes_instance_ids() {
        let mut set = SpaceAddonInstanceSet::new();

        assert!(set.mark_present("office:clipboard"));
        assert!(!set.mark_present("office:clipboard"));

        assert_eq!(set.len(), 1);
        assert!(set.contains("office:clipboard"));
    }

    #[test]
    fn mark_present_trims_instance_ids() {
        let mut set = SpaceAddonInstanceSet::new();

        assert!(set.mark_present(" office:clipboard "));

        assert!(set.contains("office:clipboard"));
        assert!(!set.contains(" office:clipboard "));
    }

    #[test]
    fn mark_absent_removes_instance_ids() {
        let mut set = SpaceAddonInstanceSet::new();
        set.mark_present("office:clipboard");

        assert!(set.mark_absent("office:clipboard"));
        assert!(!set.contains("office:clipboard"));
    }

    #[test]
    fn apply_sync_plan_marks_started_and_kept_instances_present() {
        let mut set = SpaceAddonInstanceSet::new();
        let sync_plan = SpaceAddonSyncPlan {
            start: vec![plan("office:clipboard")],
            keep: vec![plan("office:files")],
            stop: Vec::new(),
        };

        set.apply_sync_plan(&sync_plan);

        assert_eq!(
            set.sorted_ids(),
            vec!["office:clipboard".to_string(), "office:files".to_string()]
        );
    }

    #[test]
    fn apply_sync_plan_removes_stopped_instances() {
        let mut set = SpaceAddonInstanceSet::new();
        set.mark_present("office:clipboard");
        set.mark_present("office:files");
        let sync_plan = SpaceAddonSyncPlan {
            start: Vec::new(),
            keep: vec![plan("office:clipboard")],
            stop: vec!["office:files".to_string()],
        };

        set.apply_sync_plan(&sync_plan);

        assert_eq!(set.sorted_ids(), vec!["office:clipboard".to_string()]);
    }

    #[test]
    fn replace_trims_and_drops_empty_ids() {
        let mut set = SpaceAddonInstanceSet::new();

        set.replace(vec![
            " office:clipboard ".to_string(),
            "".to_string(),
            "desk:files".to_string(),
        ]);

        assert_eq!(
            set.sorted_ids(),
            vec!["desk:files".to_string(), "office:clipboard".to_string()]
        );
    }
}
