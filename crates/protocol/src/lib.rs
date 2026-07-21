//! Shared wire protocol between the game client and server.
//!
//! Messages are line-oriented UTF-8 text. Each message encodes to a single
//! line *without* a trailing newline; framing is the transport's job (a
//! trailing `\n` for TCP streams, one datagram per message for UDP).
//!
//! This crate is intentionally dependency-free and contains only plain data
//! plus (de)serialization, so both the client and server can depend on it
//! without pulling in networking, ECS, or terminal concerns.

use std::fmt;
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

/// A message sent from a client to the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessage {
    /// Register a new player session.
    Join { name: String },
    /// Nudge the controlled entity in a direction.
    Move(Direction),
    /// Give the entity with this id a velocity.
    Start(u64),
    /// Zero the velocity of the entity with this id.
    Stop(u64),
    /// End the session.
    Quit,
}

impl ClientMessage {
    /// Encode to a single wire line (no trailing newline).
    pub fn encode(&self) -> String {
        match self {
            ClientMessage::Join { name } => format!("join {name}"),
            ClientMessage::Move(direction) => format!("move {direction}"),
            ClientMessage::Start(id) => format!("start {id}"),
            ClientMessage::Stop(id) => format!("stop {id}"),
            ClientMessage::Quit => "quit".to_owned(),
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
            "move" => {
                let direction = parts
                    .next()
                    .ok_or(ProtocolError::MissingArgument("direction"))?
                    .parse()?;
                ClientMessage::Move(direction)
            }
            "start" => ClientMessage::Start(parse_id(parts.next())?),
            "stop" => ClientMessage::Stop(parse_id(parts.next())?),
            "quit" => ClientMessage::Quit,
            other => return Err(ProtocolError::UnknownCommand(other.to_owned())),
        };

        Ok(message)
    }
}

/// A message sent from the server back to a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMessage {
    /// Command accepted.
    Ok,
    /// A join was acknowledged; carries the assigned player id.
    Ack { id: u64 },
    /// Something went wrong; carries a human-readable reason.
    Error(String),
}

impl ServerMessage {
    /// Encode to a single wire line (no trailing newline).
    pub fn encode(&self) -> String {
        match self {
            ServerMessage::Ok => "ok".to_owned(),
            ServerMessage::Ack { id } => format!("ack {id}"),
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
                id: parse_id(parts.next())?,
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

fn parse_id(token: Option<&str>) -> Result<u64, ProtocolError> {
    token
        .ok_or(ProtocolError::MissingArgument("id"))?
        .parse::<u64>()
        .map_err(|_| ProtocolError::InvalidArgument("id"))
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
            ClientMessage::Move(Direction::Up),
            ClientMessage::Move(Direction::Left),
            ClientMessage::Start(3),
            ClientMessage::Stop(7),
            ClientMessage::Quit,
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
            ServerMessage::Ack { id: 42 },
            ServerMessage::Error("username required".to_owned()),
        ];

        for message in messages {
            let encoded = message.encode();
            assert_eq!(ServerMessage::decode(&encoded).unwrap(), message);
        }
    }

    #[test]
    fn unknown_command_is_rejected() {
        assert!(matches!(
            ClientMessage::decode("teleport 1"),
            Err(ProtocolError::UnknownCommand(_))
        ));
    }
}
