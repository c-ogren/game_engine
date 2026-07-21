//! UDP control server: connectionless, low-latency transport for gameplay
//! commands. Each datagram carries exactly one message, so no framing is
//! needed. Session commands (join/quit) are rejected here.

use super::Command;
use anyhow::{Context, Result};
use protocol::{ClientMessage, ServerMessage};
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::Sender;
use std::thread;

pub fn spawn_udp_control_server(server_addr: &'static str, tx: Sender<Command>) {
    thread::spawn(move || {
        if let Err(error) = run(server_addr, tx) {
            eprintln!("UDP control server stopped: {error:#}");
        }
    });
}

fn run(server_addr: &str, tx: Sender<Command>) -> Result<()> {
    let socket = UdpSocket::bind(server_addr)
        .with_context(|| format!("failed to bind UDP server to {server_addr}"))?;

    let mut buffer = [0u8; 1024];

    loop {
        let (bytes_read, client_addr) = match socket.recv_from(&mut buffer) {
            Ok(result) => result,
            Err(error) => {
                eprintln!("Error receiving UDP packet: {error}");
                continue;
            }
        };

        let packet = &buffer[..bytes_read];
        if let Err(error) = handle_packet(packet, client_addr, &socket, &tx) {
            eprintln!("Error handling packet from {client_addr}: {error:#}");
        }
    }
}

fn handle_packet(
    packet: &[u8],
    client_addr: SocketAddr,
    socket: &UdpSocket,
    tx: &Sender<Command>,
) -> Result<()> {
    let line = std::str::from_utf8(packet).context("packet was not valid UTF-8")?;

    let response = match ClientMessage::decode(line) {
        Ok(ClientMessage::Move(direction)) => {
            let _ = tx.send(Command::Move(direction));
            ServerMessage::Ok
        }
        Ok(ClientMessage::Start(id)) => {
            let _ = tx.send(Command::Start(id));
            ServerMessage::Ok
        }
        Ok(ClientMessage::Stop(id)) => {
            let _ = tx.send(Command::Stop(id));
            ServerMessage::Ok
        }
        Ok(ClientMessage::Join { .. } | ClientMessage::Quit) => {
            ServerMessage::Error("session commands require TCP".to_owned())
        }
        Err(error) => ServerMessage::Error(error.to_string()),
    };

    socket.send_to(response.encode().as_bytes(), client_addr)?;
    Ok(())
}
