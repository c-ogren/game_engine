//! UDP control server: connectionless, low-latency transport for gameplay
//! commands. Each datagram carries exactly one message, so no framing is
//! needed. Session commands (join/quit) are rejected here.

use super::Command;
use crate::state::AppState;
use anyhow::{Context, Result};
use hecs::Entity;
use protocol::{ClientMessage, ServerMessage};
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::Sender;
use std::thread;

pub fn spawn_udp_control_server(server_addr: String, tx: Sender<Command>, app_state: AppState) {
    thread::spawn(move || {
        if let Err(error) = run(&server_addr, tx, app_state) {
            log::error!("UDP control server stopped: {error:#}");
        }
    });
}

fn run(server_addr: &str, tx: Sender<Command>, app_state: AppState) -> Result<()> {
    let socket = UdpSocket::bind(server_addr)
        .with_context(|| format!("failed to bind UDP server to {server_addr}"))?;

    log::info!("UDP control server listening on {server_addr}");

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

    let response = match ClientMessage::decode(line) {
        Ok(ClientMessage::Move(direction)) => {
            control(app_state, client_addr, tx, |entity| Command::Move {
                entity,
                dir: direction,
            })
        }
        Ok(ClientMessage::Start(_)) => control(app_state, client_addr, tx, |entity| {
            Command::Start { entity }
        }),
        Ok(ClientMessage::Stop(_)) => control(app_state, client_addr, tx, |entity| Command::Stop {
            entity,
        }),
        Ok(ClientMessage::Join { .. } | ClientMessage::Quit) => {
            ServerMessage::Error("session commands require TCP".to_owned())
        }
        Err(error) => ServerMessage::Error(error.to_string()),
    };

    socket.send_to(response.encode().as_bytes(), client_addr)?;
    Ok(())
}

/// Resolve the entity controlled by `client_addr` and forward a command for it.
///
/// NOTE: a client's UDP source port differs from its TCP port, so this only
/// matches if the same address:port joined over TCP. Real per-player UDP would
/// carry a session token issued at join time rather than relying on the source
/// address; that's a future enhancement.
fn control(
    app_state: &AppState,
    client_addr: SocketAddr,
    tx: &Sender<Command>,
    make_command: impl FnOnce(Entity) -> Command,
) -> ServerMessage {
    match app_state.entity_for_address(&client_addr.to_string()) {
        Some(entity) => {
            let _ = tx.send(make_command(entity));
            ServerMessage::Ok
        }
        None => ServerMessage::Error("no session; join over TCP first".to_owned()),
    }
}
