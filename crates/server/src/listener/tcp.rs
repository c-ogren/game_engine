//! TCP control server: reliable, connection-oriented transport used *only* for
//! the session lifecycle (join/quit). Each connection owns exactly one player
//! entity for its lifetime. Gameplay commands (movement) travel over UDP; see
//! the `udp` sibling module.
//!
//! Join hands the client a session token in the `Ack`. The client echoes that
//! token on its UDP datagrams so the connectionless transport can be routed
//! back to this session's entity.

use super::Command;
use crate::state::AppState;
use anyhow::Result;
use hecs::Entity;
use protocol::{ClientMessage, ServerMessage};
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::ops::ControlFlow;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

pub fn spawn_tcp_control_server(server_addr: String, tx: Sender<Command>, app_state: AppState) {
    thread::spawn(move || {
        let listener = match TcpListener::bind(&server_addr) {
            Ok(listener) => listener,
            Err(error) => {
                log::error!("failed to bind TCP server to {server_addr}: {error}");
                return;
            }
        };

        log::info!("TCP control server listening on {server_addr}");

        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let tx = tx.clone();
            let app_state = app_state.clone();

            thread::spawn(move || {
                if let Err(error) = handle_connection(stream, tx, app_state) {
                    log::error!("error handling TCP connection: {error:#}");
                }
            });
        }
    });
}

fn handle_connection(
    mut stream: TcpStream,
    tx: Sender<Command>,
    app_state: AppState,
) -> Result<()> {
    let peer = peer_addr(&stream);
    log::info!("TCP connection opened from {peer}");

    // The player entity this connection controls, once it joins.
    let mut player: Option<Entity> = None;
    let outcome = read_messages(&mut stream, &tx, &app_state, &peer, &mut player);

    // Tear down the session and its entity however we exited (quit, clean
    // disconnect, or error).
    if let Some(entity) = player {
        let _ = tx.send(Command::Leave { entity });
        app_state.remove_by_address(&peer);
        log::info!("player at {peer} left");
    }

    outcome
}

fn read_messages(
    stream: &mut TcpStream,
    tx: &Sender<Command>,
    app_state: &AppState,
    peer: &str,
    player: &mut Option<Entity>,
) -> Result<()> {
    let mut read_buffer = [0u8; 1024];
    let mut pending = Vec::new();

    loop {
        let bytes_read = match stream.read(&mut read_buffer) {
            // A clean close by the peer.
            Ok(0) => break,
            Ok(bytes_read) => bytes_read,
            // An abrupt disconnect (common on Windows) is not an error.
            Err(error) if is_disconnect(&error) => break,
            Err(error) => return Err(error.into()),
        };

        pending.extend_from_slice(&read_buffer[..bytes_read]);

        // TCP is a byte stream, so a single read may contain zero, one, or
        // several newline-delimited messages. Process every complete line.
        while let Some(newline_index) = pending.iter().position(|&byte| byte == b'\n') {
            let line = String::from_utf8_lossy(&pending[..newline_index]).into_owned();
            pending.drain(..=newline_index);

            match handle_line(line.trim(), stream, tx, app_state, peer, player) {
                Ok(ControlFlow::Continue(())) => {}
                Ok(ControlFlow::Break(())) => return Ok(()),
                // A write that races with the client closing is a normal
                // disconnect, not a failure worth surfacing as an error.
                Err(error) => {
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(is_disconnect)
                    {
                        return Ok(());
                    }
                    return Err(error);
                }
            }
        }
    }

    Ok(())
}

/// Handle one decoded line. Returns `Break` when the connection should close.
fn handle_line(
    line: &str,
    stream: &mut TcpStream,
    tx: &Sender<Command>,
    app_state: &AppState,
    peer: &str,
    player: &mut Option<Entity>,
) -> Result<ControlFlow<()>> {
    let message = match ClientMessage::decode(line) {
        Ok(message) => message,
        Err(error) => {
            log::warn!("rejecting message {line:?} from {peer}: {error}");
            respond(stream, &ServerMessage::Error(error.to_string()))?;
            return Ok(ControlFlow::Continue(()));
        }
    };

    match message {
        ClientMessage::Join { name } => join(name, stream, tx, app_state, peer, player)?,
        ClientMessage::Quit => {
            respond(stream, &ServerMessage::Ok)?;
            // Session teardown happens in `handle_connection` once we return.
            return Ok(ControlFlow::Break(()));
        }
        // Gameplay and subscription traffic belong to UDP. Reject them here so
        // TCP stays strictly a session-lifecycle channel.
        ClientMessage::Hello { .. }
        | ClientMessage::Move { .. }
        | ClientMessage::Start { .. }
        | ClientMessage::Stop { .. }
        | ClientMessage::Ping { .. } => {
            respond(
                stream,
                &ServerMessage::Error("gameplay commands require UDP".to_owned()),
            )?;
        }
    }

    Ok(ControlFlow::Continue(()))
}

fn join(
    name: String,
    stream: &mut TcpStream,
    tx: &Sender<Command>,
    app_state: &AppState,
    peer: &str,
    player: &mut Option<Entity>,
) -> Result<()> {
    if player.is_some() {
        return respond(stream, &ServerMessage::Error("already joined".to_owned()));
    }
    if name.is_empty() {
        log::warn!("rejecting join with empty username from {peer}");
        return respond(
            stream,
            &ServerMessage::Error("username required".to_owned()),
        );
    }

    // Ask the game loop to spawn the player entity and hand back its handle.
    let (reply_tx, reply_rx) = mpsc::channel::<Entity>();
    if tx
        .send(Command::Join {
            name: name.clone(),
            reply: reply_tx,
        })
        .is_err()
    {
        log::error!("game loop unavailable while spawning player for {peer}");
        return respond(
            stream,
            &ServerMessage::Error("game loop unavailable".to_owned()),
        );
    }

    match reply_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(entity) => {
            let (id, token) = app_state.register(peer.to_owned(), entity);
            *player = Some(entity);
            log::info!("player {id} ({name}) joined from {peer}");
            respond(stream, &ServerMessage::Ack { id, token })
        }
        Err(error) => {
            log::warn!("timed out waiting for game-loop spawn for {peer}: {error}");
            respond(stream, &ServerMessage::Error("spawn timeout".to_owned()))
        }
    }
}

fn respond(stream: &mut TcpStream, message: &ServerMessage) -> Result<()> {
    // The client reads line-by-line, so every response is newline-terminated.
    writeln!(stream, "{}", message.encode())?;
    stream.flush()?;
    Ok(())
}

fn peer_addr(stream: &TcpStream) -> String {
    stream
        .peer_addr()
        .map(|addr| addr.to_string())
        .unwrap_or_default()
}

/// Whether an I/O error represents the peer going away rather than a real
/// failure.
fn is_disconnect(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::BrokenPipe
            | ErrorKind::UnexpectedEof
    )
}
