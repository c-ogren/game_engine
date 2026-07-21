//! Network listeners. TCP and UDP transports live in their own submodules;
//! both translate the shared wire [`protocol`] into the server-internal
//! [`Command`] type and forward it to the game loop.

mod tcp;
mod udp;

pub use tcp::spawn_tcp_control_server;
pub use udp::spawn_udp_control_server;

use protocol::{Direction, ServerMessage};
use std::sync::mpsc::Sender;

/// Commands the network listeners forward to the game loop.
///
/// This is an *internal* server type, deliberately distinct from the wire
/// [`protocol`]: the `Ack` variant carries an mpsc reply channel so a
/// connection handler can block until the game loop acknowledges a new
/// player. That channel can never be serialized, which is exactly why it
/// must not live in the shared protocol crate.
#[derive(Debug)]
pub enum Command {
    Move(Direction),
    Start(u64),
    Stop(u64),
    Ack {
        reply: Sender<ServerMessage>,
        id: u64,
    },
}
