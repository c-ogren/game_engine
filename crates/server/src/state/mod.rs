use hecs::Entity;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// A connected client's session. This is the *networking* layer: it maps a
/// transport address to the player's game-world entity so IO threads can route
/// packets without touching the ECS world directly. The player's simulated
/// state (name, position, velocity) lives in the ECS as `game_core::Player`.
#[derive(Clone, Debug)]
pub struct Session {
    pub id: u64,
    pub address: String,
    pub entity: Entity,
}

/// Shared registry of connected sessions. Cloneable and thread-safe so every
/// connection handler (and the UDP listener) can share one instance.
#[derive(Debug, Clone)]
pub struct AppState {
    sessions: Arc<RwLock<HashMap<u64, Session>>>,
    id_counter: Arc<AtomicU64>,
}

impl fmt::Display for AppState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AppState {{ sessions: {:?} }}", self.sessions)
    }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            id_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Register a new session for `address` controlling `entity`, returning the
    /// assigned session id.
    pub fn register(&self, address: String, entity: Entity) -> u64 {
        let id = self.id_counter.fetch_add(1, Ordering::SeqCst);
        let mut sessions = self.sessions.write().unwrap();
        sessions.insert(
            id,
            Session {
                id,
                address,
                entity,
            },
        );
        id
    }

    /// Remove the session for a transport address, returning the entity it
    /// controlled (so the caller can despawn it).
    pub fn remove_by_address(&self, address: &str) -> Option<Entity> {
        let mut sessions = self.sessions.write().unwrap();
        let id = sessions
            .iter()
            .find(|(_, session)| session.address == address)
            .map(|(id, _)| *id)?;
        sessions.remove(&id).map(|session| session.entity)
    }

    /// Look up the entity controlled by a transport address.
    pub fn entity_for_address(&self, address: &str) -> Option<Entity> {
        self.sessions
            .read()
            .unwrap()
            .values()
            .find(|session| session.address == address)
            .map(|session| session.entity)
    }

    /// Snapshot of the active sessions, for display.
    pub fn sessions(&self) -> Vec<Session> {
        self.sessions.read().unwrap().values().cloned().collect()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
