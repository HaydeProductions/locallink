use std::collections::HashSet;

#[path = "space_instance_state.rs"]
pub mod space_instance_state;

#[path = "space_core_plan.rs"]
pub mod space_core_plan;

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
        let instance_id = instance_id.into();
        if instance_id.trim().is_empty() {
            return false;
        }

        self.instance_ids.insert(instance_id)
    }

    pub fn mark_absent(&mut self, instance_id: &str) -> bool {
        self.instance_ids.remove(instance_id)
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
    fn mark_absent_removes_instance_ids() {
        let mut set = SpaceAddonInstanceSet::new();
        set.mark_present("office:clipboard");

        assert!(set.mark_absent("office:clipboard"));
        assert!(!set.contains("office:clipboard"));
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
