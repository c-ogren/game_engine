//! UDP control server: connectionless, low-latency transport for gameplay.
//! Each datagram carries exactly one message, so no framing is needed.
//!
//! This is the *only* channel for movement. A client first announces itself
//! with `Hello { token }` (the token was minted over TCP at join), which both
//! subscribes its address to world snapshots and lets the server route its
//! subsequent `Move`/`Start`/`Stop` datagrams to the right entity. Session
//! commands (join/quit) are rejected here — those belong to TCP.

use super::Command;
use crate::state::AppState;
use anyhow::{Context, Result};
use hecs::Entity;
use protocol::{ClientMessage, ServerMessage, Token};
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::Sender;
use std::thread;

/// Spawn the UDP receive loop on `socket`. The socket is expected to be bound
/// already; the caller keeps a clone for sending snapshots.
pub fn spawn_udp_control_server(socket: UdpSocket, tx: Sender<Command>, app_state: AppState) {
    thread::spawn(move || {
        if let Err(error) = run(socket, tx, app_state) {
            log::error!("UDP control server stopped: {error:#}");
        }
    });
}

fn run(socket: UdpSocket, tx: Sender<Command>, app_state: AppState) -> Result<()> {
    log::info!(
        "UDP control server listening on {}",
        socket
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| "?".to_owned())
    );

    let mut buffer = [0u8; 1024];

    loop {
        let (bytes_read, client_addr) = match socket.recv_from(&mut buffer) {
            Ok(result) => result,
            Err(error) => {
                log::warn!("error receiving UDP packet: {error}");
                continue;
            }
        };

        let packet = &buffer[..bytes_read];
        if let Err(error) = handle_packet(packet, client_addr, &socket, &tx, &app_state) {
            log::warn!("error handling packet from {client_addr}: {error:#}");
        }
    }
}

fn handle_packet(
    packet: &[u8],
    client_addr: SocketAddr,
    socket: &UdpSocket,
    tx: &Sender<Command>,
    app_state: &AppState,
) -> Result<()> {
    let line = std::str::from_utf8(packet).context("packet was not valid UTF-8")?;

    // Movement is fire-and-forget: on success we send nothing back (the world
    // snapshot pushed each tick is the client's feedback), which keeps the hot
    // path cheap. We only reply when there's an error the client should see, or
    // to acknowledge a subscription.
    match ClientMessage::decode(line) {
        Ok(ClientMessage::Hello { token }) => {
            if app_state.set_udp_address(token, client_addr) {
                log::info!("UDP subscription from {client_addr} (token {token})");
                reply(socket, client_addr, &ServerMessage::Ok);
            } else {
                reply(
                    socket,
                    client_addr,
                    &ServerMessage::Error("unknown token; join over TCP first".to_owned()),
                );
            }
        }
        Ok(ClientMessage::Move { token, dir }) => {
            forward(app_state, token, client_addr, socket, tx, |entity| {
                Command::Move { entity, dir }
            });
        }
        Ok(ClientMessage::Start { token }) => {
            forward(app_state, token, client_addr, socket, tx, |entity| {
                Command::Start { entity }
            });
        }
        Ok(ClientMessage::Stop { token }) => {
            forward(app_state, token, client_addr, socket, tx, |entity| {
                Command::Stop { entity }
            });
        }
        Ok(ClientMessage::Ping { nonce, .. }) => {
            // Pure round-trip echo; no session state involved, so we reply to
            // the sender regardless of token.
            reply(socket, client_addr, &ServerMessage::Pong { nonce });
        }
        Ok(ClientMessage::Join { .. } | ClientMessage::Quit) => {
            reply(
                socket,
                client_addr,
                &ServerMessage::Error("session commands require TCP".to_owned()),
            );
        }
        Err(error) => reply(
            socket,
            client_addr,
            &ServerMessage::Error(error.to_string()),
        ),
    }

    Ok(())
}

/// Resolve the entity for a session `token` and forward a command for it.
/// Success is silent; an unknown token gets an error datagram back.
fn forward(
    app_state: &AppState,
    token: Token,
    client_addr: SocketAddr,
    socket: &UdpSocket,
    tx: &Sender<Command>,
    make_command: impl FnOnce(Entity) -> Command,
) {
    match app_state.entity_for_token(token) {
        Some(entity) => {
            let _ = tx.send(make_command(entity));
        }
        None => reply(
            socket,
            client_addr,
            &ServerMessage::Error("unknown token; join over TCP first".to_owned()),
        ),
    }
}

fn reply(socket: &UdpSocket, client_addr: SocketAddr, message: &ServerMessage) {
    if let Err(error) = socket.send_to(message.encode().as_bytes(), client_addr) {
        log::warn!("failed to reply to {client_addr}: {error}");
    }
}
