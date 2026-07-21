use crate::player::Player;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// Tracks connected player sessions. This is a *server* concern (note the
/// network `address` on each player), not part of the game simulation.
#[derive(Debug, Clone)]
pub struct AppState {
    pub players: Arc<RwLock<HashMap<u64, Player>>>,
    pub id_counter: Arc<AtomicU64>,
}

impl fmt::Display for AppState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AppState {{ players: {:?} }}", self.players)
    }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            players: Arc::new(RwLock::new(HashMap::new())),
            id_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn add_player(&self, player: Player) -> u64 {
        let mut players = self.players.write().unwrap();
        players.insert(player.id, player);
        self.id_counter.fetch_add(1, Ordering::SeqCst)
    }

    pub fn remove_player(&self, player_addr: &str) {
        let mut players = self.players.write().unwrap();
        players.retain(|_, player| player.address != player_addr);
    }

    pub fn get_counter(&self) -> u64 {
        self.id_counter.load(Ordering::SeqCst)
    }

    pub fn get_players(&self) -> HashMap<u64, Player> {
        let players = self.players.read().unwrap();
        players.clone()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
