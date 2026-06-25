use crate::config::space_instances::SpaceAddonInstanceSet;
use std::sync::Arc;
use tokio::sync::Mutex;

pub type SharedSpaceAddonInstances = Arc<Mutex<SpaceAddonInstanceSet>>;

pub fn new_shared_space_addon_instances() -> SharedSpaceAddonInstances {
    Arc::new(Mutex::new(SpaceAddonInstanceSet::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shared_instances_start_empty() {
        let instances = new_shared_space_addon_instances();

        assert!(instances.lock().await.is_empty());
    }
}
