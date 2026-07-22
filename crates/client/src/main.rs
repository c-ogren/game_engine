//! Terminal controller client.
//!
//! Two transports, matching the server:
//!
//! * **TCP** handles the session lifecycle. We connect, `Join`, and read back an
//!   `Ack` carrying our session token; `Quit` closes the session.
//! * **UDP** handles gameplay. We announce ourselves with `Hello { token }`,
//!   stream key presses as `Move`/`Start`/`Stop` (tagged with the token), and
//!   receive per-tick world `Snapshot`s.
//!
//! Incoming snapshots are repainted in place with the shared [`render`] helper,
//! the same way the server draws its local view: the full entity list is
//! re-printed on every tick received.

use anyhow::{Context, Result, bail};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use protocol::{ClientMessage, Direction, ServerMessage, Token};
use std::{
    collections::VecDeque,
    env,
    io::{BufRead, BufReader, Write},
    net::{TcpStream, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// How many rendered frames a server message lingers before fading out. The
/// server ticks at ~60 Hz and we render one frame per received snapshot, so
/// this is roughly two seconds of visibility.
const TOAST_TTL_RENDERS: u32 = 120;

/// Cap on how many messages are shown at once; older ones drop off the top.
const MAX_TOASTS: usize = 5;

/// How often the client probes the server for latency.
const PING_INTERVAL: Duration = Duration::from_secs(1);

/// Microseconds since the Unix epoch, used as ping nonces so a `Pong` carries
/// its own send time and we don't need to track outstanding probes.
fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_micros() as u64)
        .unwrap_or(0)
}

/// A small, thread-safe stack of transient server messages. Producers (the TCP
/// reader and UDP receiver) `push` text; the render loop `drain_frame_lines`
/// once per frame to show them and age them out.
#[derive(Default)]
struct Toasts {
    inner: Mutex<VecDeque<(String, u32)>>,
}

impl Toasts {
    fn push(&self, text: String) {
        let mut queue = self.inner.lock().unwrap();
        queue.push_back((text, TOAST_TTL_RENDERS));
        while queue.len() > MAX_TOASTS {
            queue.pop_front();
        }
    }

    /// Return the current messages as display lines, then age every message by
    /// one frame and drop any that have expired.
    fn drain_frame_lines(&self) -> Vec<String> {
        let mut queue = self.inner.lock().unwrap();
        let lines = queue.iter().map(|(text, _)| text.clone()).collect();
        for (_, remaining) in queue.iter_mut() {
            *remaining = remaining.saturating_sub(1);
        }
        queue.retain(|(_, remaining)| *remaining > 0);
        lines
    }
}

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let (tcp_addr, udp_addr) = match (args.next(), args.next()) {
        (Some(tcp), Some(udp)) => (tcp, udp),
        _ => {
            eprintln!("Usage: client <tcp host:port> <udp host:port>");
            std::process::exit(1);
        }
    };

    // --- Session setup over TCP -------------------------------------------
    let mut stream = TcpStream::connect(&tcp_addr)
        .with_context(|| format!("failed to connect to {tcp_addr}"))?;
    println!("Connected to {tcp_addr} (TCP)");

    // A buffered reader over the TCP stream; we use it to read the join `Ack`
    // synchronously, then hand it to a background thread for later messages.
    let mut reader = BufReader::new(stream.try_clone().context("failed to clone stream")?);

    send_tcp(
        &mut stream,
        &ClientMessage::Join {
            name: "player".to_owned(),
        },
    )?;

    let token = read_ack(&mut reader)?;
    println!("Joined; session token {token}");

    // --- Gameplay setup over UDP ------------------------------------------
    let socket = UdpSocket::bind("0.0.0.0:0").context("failed to bind local UDP socket")?;
    socket
        .connect(&udp_addr)
        .with_context(|| format!("failed to connect UDP to {udp_addr}"))?;
    println!("Streaming input to {udp_addr} (UDP)");

    // Subscribe: tell the server which UDP address to push snapshots to.
    send_udp(&socket, &ClientMessage::Hello { token })?;

    // Transient server messages (acks/errors) shared across the reader threads
    // and shown inside each painted frame for a short while.
    let toasts = Arc::new(Toasts::default());

    // Print any further TCP server messages (errors, etc.) as transient toasts.
    let tcp_toasts = toasts.clone();
    thread::spawn(move || {
        for line in reader.lines() {
            let Ok(line) = line else { break };
            tcp_toasts.push(format!("server: {line}"));
        }
    });

    // Receive world snapshots over UDP and repaint the entity list each tick.
    let recv_socket = socket.try_clone().context("failed to clone UDP socket")?;
    let udp_toasts = toasts.clone();
    // Most recent measured round-trip time, in microseconds (0 = not yet known).
    // The receive thread writes it on every `Pong`; the render reads it.
    let latency = Arc::new(AtomicU64::new(0));
    let recv_latency = latency.clone();
    thread::spawn(move || receive_updates(recv_socket, udp_toasts, recv_latency));

    // Probe latency once a second. The nonce is the send time, so the receive
    // thread can compute RTT from the echoed `Pong` without any shared state.
    let ping_socket = socket.try_clone().context("failed to clone UDP socket")?;
    thread::spawn(move || {
        loop {
            let ping = ClientMessage::Ping {
                token,
                nonce: now_micros(),
            };
            if send_udp(&ping_socket, &ping).is_err() {
                break;
            }
            thread::sleep(PING_INTERVAL);
        }
    });

    render::enter();
    enable_raw_mode()?;
    let result = run(&socket, token, &mut stream);
    disable_raw_mode()?;
    render::leave();
    result
}

/// Read the join acknowledgement from the TCP stream and return the token.
fn read_ack(reader: &mut BufReader<TcpStream>) -> Result<Token> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        bail!("server closed the connection before acknowledging join");
    }

    match ServerMessage::decode(line.trim()) {
        Ok(ServerMessage::Ack { token, .. }) => Ok(token),
        Ok(ServerMessage::Error(reason)) => bail!("join rejected: {reason}"),
        Ok(other) => bail!("expected an ack, got {other:?}"),
        Err(error) => bail!("could not parse server ack: {error}"),
    }
}

/// The main input loop: translate key presses into UDP gameplay messages.
fn run(socket: &UdpSocket, token: Token, stream: &mut TcpStream) -> Result<()> {
    loop {
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };

        // Ignore release events.
        if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
            continue;
        }

        let message = match key.code {
            KeyCode::Up => ClientMessage::Move {
                token,
                dir: Direction::Up,
            },
            KeyCode::Down => ClientMessage::Move {
                token,
                dir: Direction::Down,
            },
            KeyCode::Left => ClientMessage::Move {
                token,
                dir: Direction::Left,
            },
            KeyCode::Right => ClientMessage::Move {
                token,
                dir: Direction::Right,
            },
            KeyCode::Char('s') => ClientMessage::Start { token },
            KeyCode::Char('x') => ClientMessage::Stop { token },
            KeyCode::Char('q') => {
                // Quit is a session command, so it goes over TCP.
                let _ = send_tcp(stream, &ClientMessage::Quit);
                break;
            }
            _ => continue,
        };
        send_udp(socket, &message)?;
    }

    Ok(())
}

/// Receive world snapshots on `socket` and repaint the whole entity list on
/// every tick, using the same in-place render as the server. Any pending
/// server messages are shown beneath the entities and fade out over time.
/// `Pong` replies update `latency` (microseconds) for display.
fn receive_updates(socket: UdpSocket, toasts: Arc<Toasts>, latency: Arc<AtomicU64>) {
    let mut buffer = [0u8; 4096];
    loop {
        let bytes_read = match socket.recv(&mut buffer) {
            Ok(0) => continue,
            Ok(bytes_read) => bytes_read,
            Err(_) => break,
        };

        let Ok(line) = std::str::from_utf8(&buffer[..bytes_read]) else {
            continue;
        };

        match ServerMessage::decode(line) {
            Ok(ServerMessage::Snapshot { entities }) => {
                let mut lines = Vec::with_capacity(entities.len() + 4);
                lines.push("Arrow keys move. 's' start, 'x' stop, 'q' quits.".to_owned());

                match latency.load(Ordering::Relaxed) {
                    0 => lines.push("Ping: --".to_owned()),
                    micros => lines.push(format!("Ping: {:.1} ms", micros as f64 / 1000.0)),
                }

                lines.push(format!("Entities in world: {}", entities.len()));
                for entity in &entities {
                    lines.push(format!(
                        "Entity {} Position: ({:.3}, {:.3})",
                        entity.id, entity.x, entity.y
                    ));
                }

                // Fold in any transient server messages, aging them one frame.
                let toast_lines = toasts.drain_frame_lines();
                if !toast_lines.is_empty() {
                    lines.push(String::new());
                    lines.push("Messages:".to_owned());
                    lines.extend(toast_lines);
                }

                render::paint(&lines);
            }
            // Echoed ping: the nonce was our send time, so RTT is now - nonce.
            Ok(ServerMessage::Pong { nonce }) => {
                let rtt = now_micros().saturating_sub(nonce);
                latency.store(rtt, Ordering::Relaxed);
            }
            // The server may also send Ok/Error over UDP (e.g. a rejected Hello).
            Ok(other) => toasts.push(format!("server(udp): {}", other.encode())),
            Err(_) => {}
        }
    }
}

fn send_tcp(stream: &mut TcpStream, message: &ClientMessage) -> Result<()> {
    // TCP is a stream, so every message is newline-framed.
    writeln!(stream, "{}", message.encode())?;
    stream.flush()?;
    Ok(())
}

fn send_udp(socket: &UdpSocket, message: &ClientMessage) -> Result<()> {
    // One datagram per message; no framing needed.
    socket.send(message.encode().as_bytes())?;
    Ok(())
}
