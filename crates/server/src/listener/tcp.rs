//! TCP control server: reliable, connection-oriented transport used for
//! session lifecycle (join/quit) as well as gameplay commands.

use super::Command;
use crate::player::Player;
use crate::state::AppState;
use anyhow::Result;
use protocol::{ClientMessage, ServerMessage};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

pub fn spawn_tcp_control_server(
    server_addr: &'static str,
    tx: Sender<Command>,
    app_state: AppState,
) {
    thread::spawn(move || {
        let listener = match TcpListener::bind(server_addr) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("Failed to bind TCP server to {server_addr}: {error}");
                return;
            }
        };

        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let tx = tx.clone();
            let app_state = app_state.clone();

            thread::spawn(move || {
                if let Err(error) = handle_connection(stream, tx, app_state) {
                    eprintln!("Error handling TCP connection: {error:#}");
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
    let mut read_buffer = [0u8; 1024];
    let mut pending = Vec::new();

    loop {
        let bytes_read = stream.read(&mut read_buffer)?;
        if bytes_read == 0 {
            // Connection closed by the peer.
            break;
        }

        pending.extend_from_slice(&read_buffer[..bytes_read]);

        // TCP is a byte stream, so a single read may contain zero, one, or
        // several newline-delimited messages. Process every complete line.
        while let Some(newline_index) = pending.iter().position(|&byte| byte == b'\n') {
            let line = String::from_utf8_lossy(&pending[..newline_index]).into_owned();
            pending.drain(..=newline_index);
            handle_line(line.trim(), &mut stream, &tx, &app_state)?;
        }
    }

    Ok(())
}

fn handle_line(
    line: &str,
    stream: &mut TcpStream,
    tx: &Sender<Command>,
    app_state: &AppState,
) -> Result<()> {
    let message = match ClientMessage::decode(line) {
        Ok(message) => message,
        Err(error) => return respond(stream, &ServerMessage::Error(error.to_string())),
    };

    match message {
        ClientMessage::Join { name } => join(name, stream, tx, app_state)?,
        ClientMessage::Quit => {
            app_state.remove_player(&peer_addr(stream));
            respond(stream, &ServerMessage::Ok)?;
        }
        // Gameplay commands are fire-and-forget to keep the input path quiet.
        ClientMessage::Move(direction) => {
            let _ = tx.send(Command::Move(direction));
        }
        ClientMessage::Start(id) => {
            let _ = tx.send(Command::Start(id));
        }
        ClientMessage::Stop(id) => {
            let _ = tx.send(Command::Stop(id));
        }
    }

    Ok(())
}

fn join(
    name: String,
    stream: &mut TcpStream,
    tx: &Sender<Command>,
    app_state: &AppState,
) -> Result<()> {
    if name.is_empty() {
        return respond(
            stream,
            &ServerMessage::Error("username required".to_owned()),
        );
    }

    let player_id = app_state.get_counter();
    let id = app_state.add_player(Player::new(player_id, name, peer_addr(stream)));

    // Round-trip through the game loop so the client only sees an ack once
    // the simulation has actually observed the new player.
    let (reply_tx, reply_rx) = mpsc::channel::<ServerMessage>();
    if tx
        .send(Command::Ack {
            reply: reply_tx,
            id,
        })
        .is_err()
    {
        return respond(
            stream,
            &ServerMessage::Error("game loop unavailable".to_owned()),
        );
    }

    match reply_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(message) => respond(stream, &message),
        Err(error) => {
            eprintln!("Failed waiting for game-loop acknowledgement: {error}");
            respond(
                stream,
                &ServerMessage::Error("acknowledgement timeout".to_owned()),
            )
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
