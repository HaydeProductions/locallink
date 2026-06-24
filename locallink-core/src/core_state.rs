use crate::addons::AddonRecord;
use crate::config::spaces::{load_or_create_space_store, new_space_registry, SpaceRegistry, SpaceStore};
use crate::discovery::Peer;
use crate::transport::{ConnectedPeer, ConnectionRegistry, EventQueue, EventStore};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;

pub type PeerRegistry = Arc<Mutex<HashMap<String, Peer>>>;
pub type ConnectingRegistry = Arc<Mutex<HashSet<String>>>;
pub type AddonRegistry = Arc<Mutex<Vec<AddonRecord>>>;

#[derive(Clone)]
pub struct CoreRuntimeState {
    pub peers: PeerRegistry,
    pub connecting: ConnectingRegistry,
    pub connections: ConnectionRegistry,
    pub events: EventQueue,
    pub addons: AddonRegistry,
    pub spaces: SpaceRegistry,
}

impl CoreRuntimeState {
    pub fn new(addons: Vec<AddonRecord>, spaces: SpaceStore) -> Self {
        Self {
            peers: Arc::new(Mutex::new(HashMap::<String, Peer>::new())),
            connecting: Arc::new(Mutex::new(HashSet::<String>::new())),
            connections: Arc::new(Mutex::new(HashMap::<String, ConnectedPeer>::new())),
            events: Arc::new(Mutex::new(EventStore::default())),
            addons: Arc::new(Mutex::new(addons)),
            spaces: new_space_registry(spaces),
        }
    }
}

pub fn load_core_runtime_state(addons: Vec<AddonRecord>) -> Result<CoreRuntimeState> {
    let spaces = load_or_create_space_store()?;
    Ok(CoreRuntimeState::new(addons, spaces))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runtime_state_starts_with_loaded_spaces() {
        let mut spaces = SpaceStore::default();
        spaces.spaces.push(crate::config::spaces::SpaceRecord {
            space_id: "office".to_string(),
            name: "Office".to_string(),
            kind: crate::config::spaces::SpaceKind::Group,
            members: vec!["desktop".to_string()],
            addons: HashMap::new(),
        });

        let state = CoreRuntimeState::new(Vec::new(), spaces);

        assert_eq!(state.spaces.lock().await.spaces.len(), 1);
        assert!(state.peers.lock().await.is_empty());
        assert!(state.connecting.lock().await.is_empty());
        assert!(state.connections.lock().await.is_empty());
        assert!(state.addons.lock().await.is_empty());
    }
}
