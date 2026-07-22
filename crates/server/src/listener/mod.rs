//! Network listeners. TCP and UDP transports live in their own submodules;
//! both translate the shared wire [`protocol`] into the server-internal
//! [`Command`] type and forward it to the game loop.

mod tcp;
mod udp;

pub use tcp::spawn_tcp_control_server;
pub use udp::spawn_udp_control_server;

use hecs::Entity;
use protocol::Direction;
use std::sync::mpsc::Sender;

/// Commands the network listeners forward to the game loop.
///
/// This is an *internal* server type, deliberately distinct from the wire
/// [`protocol`]. Commands are addressed to a specific game-world [`Entity`],
/// which the connection handler learns when it joins; that entity handle can
/// never be serialized, which is why it must not live in the shared protocol
/// crate. The `Join` variant carries an mpsc reply channel so a handler can
/// block until the game loop spawns the player and returns its entity.
#[derive(Debug)]
pub enum Command {
    /// Spawn a player entity; the loop replies with its handle.
    Join { name: String, reply: Sender<Entity> },
    /// Despawn a player entity (disconnect/quit).
    Leave { entity: Entity },
    /// Nudge an entity in a direction.
    Move { entity: Entity, dir: Direction },
    /// Give an entity a velocity.
    Start { entity: Entity },
    /// Zero an entity's velocity.
    Stop { entity: Entity },
    // TODO: track mouse movements as a command, e.g.
    //   Look { entity: Entity, position: (f32, f32) }
    // The client would report cursor coordinates and the game loop would
    // update the player's facing/aim. This needs a matching wire message in
    // the `protocol` crate (e.g. `ClientMessage::Look { x, y }`).
}
