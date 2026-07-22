//! Shared wire protocol between the game client and server.
//!
//! Messages are line-oriented UTF-8 text. Each message encodes to a single
//! line *without* a trailing newline; framing is the transport's job (a
//! trailing `\n` for TCP streams, one datagram per message for UDP).
//!
//! Transports are split by responsibility:
//!
//! * **TCP** owns the session lifecycle: [`ClientMessage::Join`] /
//!   [`ClientMessage::Quit`], answered with [`ServerMessage::Ack`] (which hands
//!   the client a session *token*).
//! * **UDP** owns fast gameplay traffic: the client subscribes with
//!   [`ClientMessage::Hello`] and then streams [`ClientMessage::Move`] /
//!   [`ClientMessage::Start`] / [`ClientMessage::Stop`], each tagged with its
//!   token so the server can route it without relying on the source address.
//!   The server pushes [`ServerMessage::Snapshot`] world updates back over the
//!   same channel.
//!
//! This crate is intentionally dependency-free and contains only plain data
//! plus (de)serialization, so both the client and server can depend on it
//! without pulling in networking, ECS, or terminal concerns.

use std::fmt;
use std::fmt::Write as _;
use std::str::FromStr;

/// A movement direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Up => "up",
            Direction::Down => "down",
            Direction::Left => "left",
            Direction::Right => "right",
        }
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Direction {
    type Err = ProtocolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "up" => Ok(Direction::Up),
            "down" => Ok(Direction::Down),
            "left" => Ok(Direction::Left),
            "right" => Ok(Direction::Right),
            other => Err(ProtocolError::UnknownDirection(other.to_owned())),
        }
    }
}

/// A session token, minted by the server at join time and echoed by the client
/// on every UDP datagram so the server can map connectionless packets back to a
/// session (and thus a game-world entity) without trusting the source address.
pub type Token = u64;

/// A message sent from a client to the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessage {
    /// Register a new player session. **TCP only.**
    Join { name: String },
    /// End the session. **TCP only.**
    Quit,
    /// Announce this client's UDP address for a session, subscribing it to
    /// world snapshots. **UDP only.**
    Hello { token: Token },
    /// Nudge the controlled entity in a direction. **UDP only.**
    Move { token: Token, dir: Direction },
    /// Give the controlled entity a velocity. **UDP only.**
    Start { token: Token },
    /// Zero the controlled entity's velocity. **UDP only.**
    Stop { token: Token },
    /// Round-trip probe for latency. The server echoes `nonce` back in a
    /// [`ServerMessage::Pong`]; the client measures RTT on receipt. **UDP only.**
    Ping { token: Token, nonce: u64 },
}

impl ClientMessage {
    /// Encode to a single wire line (no trailing newline).
    pub fn encode(&self) -> String {
        match self {
            ClientMessage::Join { name } => format!("join {name}"),
            ClientMessage::Quit => "quit".to_owned(),
            ClientMessage::Hello { token } => format!("hello {token}"),
            ClientMessage::Move { token, dir } => format!("move {token} {dir}"),
            ClientMessage::Start { token } => format!("start {token}"),
            ClientMessage::Stop { token } => format!("stop {token}"),
            ClientMessage::Ping { token, nonce } => format!("ping {token} {nonce}"),
        }
    }

    /// Parse a single wire line into a message.
    pub fn decode(line: &str) -> Result<Self, ProtocolError> {
        let line = line.trim();
        let mut parts = line.split_whitespace();
        let verb = parts.next().ok_or(ProtocolError::Empty)?;

        let message = match verb {
            "join" => {
                let name = parts.next().ok_or(ProtocolError::MissingArgument("name"))?;
                ClientMessage::Join {
                    name: name.to_owned(),
                }
            }
            "quit" => ClientMessage::Quit,
            "hello" => ClientMessage::Hello {
                token: parse_u64(parts.next(), "token")?,
            },
            "move" => {
                let token = parse_u64(parts.next(), "token")?;
                let dir = parts
                    .next()
                    .ok_or(ProtocolError::MissingArgument("direction"))?
                    .parse()?;
                ClientMessage::Move { token, dir }
            }
            "start" => ClientMessage::Start {
                token: parse_u64(parts.next(), "token")?,
            },
            "stop" => ClientMessage::Stop {
                token: parse_u64(parts.next(), "token")?,
            },
            "ping" => ClientMessage::Ping {
                token: parse_u64(parts.next(), "token")?,
                nonce: parse_u64(parts.next(), "nonce")?,
            },
            other => return Err(ProtocolError::UnknownCommand(other.to_owned())),
        };

        Ok(message)
    }
}

/// The position of a single entity within a world snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntitySnapshot {
    pub id: u64,
    pub x: f32,
    pub y: f32,
}

impl EntitySnapshot {
    /// Encode as `id:x,y` (no surrounding whitespace).
    fn encode(&self) -> String {
        format!("{}:{},{}", self.id, self.x, self.y)
    }

    /// Parse an `id:x,y` token.
    fn parse(token: &str) -> Result<Self, ProtocolError> {
        let (id, rest) = token
            .split_once(':')
            .ok_or(ProtocolError::InvalidArgument("snapshot entry"))?;
        let (x, y) = rest
            .split_once(',')
            .ok_or(ProtocolError::InvalidArgument("snapshot entry"))?;
        Ok(EntitySnapshot {
            id: id
                .parse()
                .map_err(|_| ProtocolError::InvalidArgument("snapshot id"))?,
            x: x.parse()
                .map_err(|_| ProtocolError::InvalidArgument("snapshot x"))?,
            y: y.parse()
                .map_err(|_| ProtocolError::InvalidArgument("snapshot y"))?,
        })
    }
}

/// A message sent from the server back to a client.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMessage {
    /// Command accepted.
    Ok,
    /// A join was acknowledged; carries the assigned player id and the session
    /// token the client must echo on UDP traffic.
    Ack { id: u64, token: Token },
    /// A world update: every entity's current position. Pushed over UDP each
    /// tick to subscribed clients.
    Snapshot { entities: Vec<EntitySnapshot> },
    /// Reply to a [`ClientMessage::Ping`], echoing its `nonce` so the client
    /// can compute round-trip latency. **UDP only.**
    Pong { nonce: u64 },
    /// Something went wrong; carries a human-readable reason.
    Error(String),
}

impl ServerMessage {
    /// Encode to a single wire line (no trailing newline).
    pub fn encode(&self) -> String {
        match self {
            ServerMessage::Ok => "ok".to_owned(),
            ServerMessage::Ack { id, token } => format!("ack {id} {token}"),
            ServerMessage::Snapshot { entities } => {
                let mut line = String::from("snapshot");
                for entity in entities {
                    // Safe: writing into a String is infallible.
                    let _ = write!(line, " {}", entity.encode());
                }
                line
            }
            ServerMessage::Pong { nonce } => format!("pong {nonce}"),
            ServerMessage::Error(message) => format!("error {message}"),
        }
    }

    /// Parse a single wire line into a message.
    pub fn decode(line: &str) -> Result<Self, ProtocolError> {
        let line = line.trim();
        let mut parts = line.split_whitespace();
        let verb = parts.next().ok_or(ProtocolError::Empty)?;

        let message = match verb {
            "ok" => ServerMessage::Ok,
            "ack" => ServerMessage::Ack {
                id: parse_u64(parts.next(), "id")?,
                token: parse_u64(parts.next(), "token")?,
            },
            "snapshot" => {
                let entities = parts
                    .map(EntitySnapshot::parse)
                    .collect::<Result<Vec<_>, _>>()?;
                ServerMessage::Snapshot { entities }
            }
            "pong" => ServerMessage::Pong {
                nonce: parse_u64(parts.next(), "nonce")?,
            },
            "error" => {
                // Everything after the "error" verb is the reason.
                let reason = line.strip_prefix("error").unwrap_or("").trim();
                ServerMessage::Error(reason.to_owned())
            }
            other => return Err(ProtocolError::UnknownCommand(other.to_owned())),
        };

        Ok(message)
    }
}

fn parse_u64(token: Option<&str>, name: &'static str) -> Result<u64, ProtocolError> {
    token
        .ok_or(ProtocolError::MissingArgument(name))?
        .parse::<u64>()
        .map_err(|_| ProtocolError::InvalidArgument(name))
}

/// Errors produced while decoding a protocol message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    Empty,
    MissingArgument(&'static str),
    InvalidArgument(&'static str),
    UnknownCommand(String),
    UnknownDirection(String),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtocolError::Empty => write!(f, "empty message"),
            ProtocolError::MissingArgument(name) => write!(f, "missing argument: {name}"),
            ProtocolError::InvalidArgument(name) => write!(f, "invalid argument: {name}"),
            ProtocolError::UnknownCommand(command) => write!(f, "unknown command: {command}"),
            ProtocolError::UnknownDirection(direction) => {
                write!(f, "unknown direction: {direction}")
            }
        }
    }
}

impl std::error::Error for ProtocolError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_messages_round_trip() {
        let messages = [
            ClientMessage::Join {
                name: "alice".to_owned(),
            },
            ClientMessage::Quit,
            ClientMessage::Hello { token: 99 },
            ClientMessage::Move {
                token: 12,
                dir: Direction::Up,
            },
            ClientMessage::Move {
                token: 12,
                dir: Direction::Left,
            },
            ClientMessage::Start { token: 3 },
            ClientMessage::Stop { token: 7 },
        ];

        for message in messages {
            let encoded = message.encode();
            assert_eq!(ClientMessage::decode(&encoded).unwrap(), message);
        }
    }

    #[test]
    fn server_messages_round_trip() {
        let messages = [
            ServerMessage::Ok,
            ServerMessage::Ack { id: 42, token: 777 },
            ServerMessage::Snapshot {
                entities: vec![
                    EntitySnapshot {
                        id: 1,
                        x: 0.0,
                        y: 0.0,
                    },
                    EntitySnapshot {
                        id: 2,
                        x: 1.5,
                        y: -3.0,
                    },
                ],
            },
            ServerMessage::Error("username required".to_owned()),
        ];

        for message in messages {
            let encoded = message.encode();
            assert_eq!(ServerMessage::decode(&encoded).unwrap(), message);
        }
    }

    #[test]
    fn empty_snapshot_round_trips() {
        let message = ServerMessage::Snapshot { entities: vec![] };
        assert_eq!(ServerMessage::decode(&message.encode()).unwrap(), message);
    }

    #[test]
    fn unknown_command_is_rejected() {
        assert!(matches!(
            ClientMessage::decode("teleport 1"),
            Err(ProtocolError::UnknownCommand(_))
        ));
    }
}
