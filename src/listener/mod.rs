use crate::state::AppState;
use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::{
    net::{SocketAddr, TcpListener, UdpSocket},
    sync::mpsc::Sender,
    thread,
};

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug)]
pub enum Command {
    Stop(u64),
    Start(u64),
    Move(Direction),
    Ack((Sender<String>, u64)),
}

pub fn spawn_tcp_control_server(
    server_addr: &'static str,
    tx: Sender<Command>,
    app_state: AppState,
) {
    thread::spawn(move || {
        let listener = TcpListener::bind(server_addr).unwrap();
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                continue;
            };
            let tx = tx.clone();
            let app_state = app_state.clone();

            thread::spawn(move || {
                if let Err(error) = handle_tcp_connection(stream, tx, app_state) {
                    eprintln!("Error handling TCP connection: {error:#}");
                }
            });
        }
    });
}

fn handle_tcp_connection(
    mut stream: std::net::TcpStream,
    tx: Sender<Command>,
    app_state: AppState,
) -> Result<()> {
    let mut read_buffer = [0u8; 1024];
    let mut pending = vec![];

    loop {
        let bytes_read = stream.read(&mut read_buffer)?;

        if bytes_read == 0 {
            // Connection closed
            break;
        }

        pending.extend_from_slice(&read_buffer[..bytes_read]);

        while let Some(consumed) = process_pending(&mut pending, &tx, &mut stream, &app_state) {
            pending.drain(..consumed);
        }
    }

    Ok(())
}

fn process_pending(
    pending: &[u8],
    tx: &Sender<Command>,
    stream: &mut std::net::TcpStream,
    app_state: &AppState,
) -> Option<usize> {
    // TCP may deliver only part of a command. Wait until a complete
    // newline-terminated message is available.
    let newline_index = pending.iter().position(|&byte| byte == b'\n')?;
    let consumed = newline_index + 1;

    let line = std::str::from_utf8(&pending[..newline_index])
        .ok()?
        .trim_end_matches('\r');

    if line.starts_with('q') {
        app_state.remove_player(
            &stream
                .peer_addr()
                .map(|addr| addr.to_string())
                .unwrap_or_default(),
        );
        let _ = writeln!(stream, "ok");
        let _ = stream.flush();
        return Some(consumed);
    }

    let Some(username) = line.strip_prefix('n') else {
        let _ = writeln!(stream, "error unknown command");
        let _ = stream.flush();
        return Some(consumed);
    };

    let username = username.trim();

    if username.is_empty() {
        let _ = writeln!(stream, "error username required");
        let _ = stream.flush();
        return Some(consumed);
    }

    let player_id = app_state.get_counter();

    let id = app_state.add_player(crate::player::Player::new(
        player_id,
        username.to_owned(),
        stream
            .peer_addr()
            .map(|addr| addr.to_string())
            .unwrap_or_default(),
    ));

    let (reply_tx, reply_rx) = std::sync::mpsc::channel::<String>();

    if tx.send(Command::Ack((reply_tx, id))).is_err() {
        let _ = writeln!(stream, "error game loop unavailable");
        let _ = stream.flush();
        return Some(consumed);
    }

    match reply_rx.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(reply) => {
            // Newline is required because the client uses read_line().
            let _ = writeln!(stream, "{reply}");
            let _ = stream.flush();
        }

        Err(error) => {
            eprintln!("Failed waiting for game-loop acknowledgement: {error}");

            let _ = writeln!(stream, "error acknowledgement timeout");
            let _ = stream.flush();
        }
    }

    Some(consumed)
}

pub fn spawn_udp_control_server(server_addr: &'static str, tx: Sender<Command>) {
    thread::spawn(move || {
        if let Err(error) = run_udp_control_server(server_addr, tx) {
            eprintln!("UDP control server stopped: {error:#}");
        }
    });
}

fn run_udp_control_server(server_addr: &str, tx: Sender<Command>) -> Result<()> {
    let socket = UdpSocket::bind(server_addr)
        .with_context(|| format!("failed to bind UDP server to {server_addr}"))?;

    println!("UDP control server listening on {server_addr}");

    let mut buffer = [0_u8; 1024];

    loop {
        let (bytes_read, client_addr) = match socket.recv_from(&mut buffer) {
            Ok(result) => result,

            Err(error) => {
                eprintln!("Error receiving UDP packet: {error}");
                continue;
            }
        };

        let packet = &buffer[..bytes_read];

        if let Err(error) = handle_udp_packet(packet, client_addr, &socket, &tx) {
            eprintln!("Error handling packet from {client_addr}: {error:#}");
        }
    }
}

fn handle_udp_packet(
    packet: &[u8],
    client_addr: SocketAddr,
    socket: &UdpSocket,
    tx: &Sender<Command>,
) -> Result<()> {
    let input = std::str::from_utf8(packet)
        .context("packet was not valid UTF-8")?
        .trim();

    match input {
        "move 0 1" => {
            tx.send(Command::Move(Direction::Up))
                .context("game-loop command channel closed")?;

            socket.send_to(b"ok", client_addr)?;
        }

        "move 0 -1" => {
            tx.send(Command::Move(Direction::Down))
                .context("game-loop command channel closed")?;

            socket.send_to(b"ok", client_addr)?;
        }

        "move -1 0" => {
            tx.send(Command::Move(Direction::Left))
                .context("game-loop command channel closed")?;

            socket.send_to(b"ok", client_addr)?;
        }

        "move 1 0" => {
            tx.send(Command::Move(Direction::Right))
                .context("game-loop command channel closed")?;

            socket.send_to(b"ok", client_addr)?;
        }

        command if command.starts_with("start ") => {
            let entity_id = parse_entity_id(command, "start")?;

            tx.send(Command::Start(entity_id))
                .context("game-loop command channel closed")?;

            socket.send_to(b"ok", client_addr)?;
        }

        command if command.starts_with("stop ") => {
            let entity_id = parse_entity_id(command, "stop")?;

            tx.send(Command::Stop(entity_id))
                .context("game-loop command channel closed")?;

            socket.send_to(b"ok", client_addr)?;
        }

        _ => {
            anyhow::bail!("unknown command: {input}");
        }
    }

    Ok(())
}

fn parse_entity_id(input: &str, expected_command: &str) -> Result<u64> {
    let mut parts = input.split_whitespace();

    let command = parts.next().context("missing command")?;

    if command != expected_command {
        anyhow::bail!("expected {expected_command}, got {command}");
    }

    let entity_id = parts
        .next()
        .context("missing entity ID")?
        .parse::<u64>()
        .context("entity ID must be an unsigned integer")?;

    if parts.next().is_some() {
        anyhow::bail!("unexpected trailing arguments");
    }

    Ok(entity_id)
}
