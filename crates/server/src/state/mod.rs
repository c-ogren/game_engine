use hecs::Entity;
use protocol::Token;
use std::collections::HashMap;
use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// A connected client's session. This is the *networking* layer: it maps a
/// transport identity to the player's game-world entity so IO threads can route
/// packets without touching the ECS world directly. The player's simulated
/// state (name, position, velocity) lives in the ECS as `game_core::Player`.
///
/// A session is born over TCP at join time. Its `udp_address` is unknown until
/// the client sends a UDP `Hello`; once set, that address is where per-tick
/// world snapshots are pushed.
#[derive(Clone, Debug)]
pub struct Session {
    pub id: u64,
    /// Opaque token echoed by the client on UDP traffic to identify itself.
    pub token: Token,
    /// The TCP peer address the session joined from.
    pub tcp_address: String,
    /// The UDP address to push snapshots to, learned from `Hello`.
    pub udp_address: Option<SocketAddr>,
    pub entity: Entity,
}

/// Shared registry of connected sessions. Cloneable and thread-safe so every
/// connection handler, the UDP listener, and the game loop can share one
/// instance.
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

    /// Register a new session for `tcp_address` controlling `entity`, returning
    /// the assigned session id and the token the client must echo over UDP.
    pub fn register(&self, tcp_address: String, entity: Entity) -> (u64, Token) {
        let id = self.id_counter.fetch_add(1, Ordering::SeqCst);
        let mut sessions = self.sessions.write().unwrap();

        // Mint a token that no live session is already using. Collisions are
        // astronomically unlikely, but routing correctness depends on
        // uniqueness, so we check rather than assume.
        let mut token = mint_token(id);
        while sessions.values().any(|session| session.token == token) {
            token = mint_token(token);
        }

        sessions.insert(
            id,
            Session {
                id,
                token,
                tcp_address,
                udp_address: None,
                entity,
            },
        );
        (id, token)
    }

    /// Remove the session for a TCP address, returning the entity it controlled
    /// (so the caller can despawn it).
    pub fn remove_by_address(&self, tcp_address: &str) -> Option<Entity> {
        let mut sessions = self.sessions.write().unwrap();
        let id = sessions
            .iter()
            .find(|(_, session)| session.tcp_address == tcp_address)
            .map(|(id, _)| *id)?;
        sessions.remove(&id).map(|session| session.entity)
    }

    /// Look up the entity controlled by a session token.
    pub fn entity_for_token(&self, token: Token) -> Option<Entity> {
        self.sessions
            .read()
            .unwrap()
            .values()
            .find(|session| session.token == token)
            .map(|session| session.entity)
    }

    /// Record the UDP address for the session identified by `token`, so the
    /// game loop can push snapshots to it. Returns `true` if the token matched
    /// a live session.
    pub fn set_udp_address(&self, token: Token, address: SocketAddr) -> bool {
        let mut sessions = self.sessions.write().unwrap();
        match sessions.values_mut().find(|session| session.token == token) {
            Some(session) => {
                session.udp_address = Some(address);
                true
            }
            None => false,
        }
    }

    /// Every UDP address currently subscribed to world snapshots.
    pub fn subscribers(&self) -> Vec<SocketAddr> {
        self.sessions
            .read()
            .unwrap()
            .values()
            .filter_map(|session| session.udp_address)
            .collect()
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

/// Derive a hard-to-guess token from a seed using SplitMix64. Dependency-free
/// and good enough to keep tokens unpredictable for a local game server.
fn mint_token(seed: u64) -> Token {
    let entropy = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut z = seed
        .wrapping_add(entropy)
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
