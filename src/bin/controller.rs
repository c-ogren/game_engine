use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::{
    env,
    io::{self, BufRead, BufReader, Write},
    net::{TcpStream, UdpSocket},
    sync::{
        atomic::AtomicU64,
        mpsc::{self, Sender},
    },
    thread,
    time::Duration,
};

static ID: AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn main() -> Result<()> {
    let udp_addr = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: controller <udp_host:udp_port> <tcp_host:tcp_port>");
        std::process::exit(1);
    });

    let tcp_addr = env::args().nth(2).unwrap_or_else(|| {
        eprintln!("Usage: controller <udp_host:udp_port> <tcp_host:tcp_port>");
        std::process::exit(1);
    });

    /*
     * UDP setup
     */

    let udp_socket = UdpSocket::bind("0.0.0.0:0").context("failed to bind UDP socket")?;

    udp_socket
        .connect(&udp_addr)
        .with_context(|| format!("failed to configure UDP peer {udp_addr}"))?;

    println!("UDP socket {} -> {}", udp_socket.local_addr()?, udp_addr);

    /*
     * TCP setup
     */

    let tcp_stream = TcpStream::connect(&tcp_addr)
        .with_context(|| format!("failed to connect to TCP server {tcp_addr}"))?;

    tcp_stream
        .set_nodelay(true)
        .context("failed to set TCP_NODELAY")?;

    let tcp_write_stream = tcp_stream
        .try_clone()
        .context("failed to clone TCP stream for writing")?;

    let (tcp_tx, tcp_rx) = mpsc::channel::<Vec<u8>>();

    spawn_tcp_writer(tcp_write_stream, tcp_rx);
    spawn_tcp_reader(tcp_stream);

    /*
     * Send the initial player connection/login command through the same
     * channel all future reliable TCP commands will use.
     */

    tcp_tx
        .send(b"n user\n".to_vec())
        .context("TCP writer thread stopped")?;

    println!("Sent player connection request");

    /*
     * UDP response reader
     */

    let udp_receiver = udp_socket
        .try_clone()
        .context("failed to clone UDP socket")?;

    thread::spawn(move || receive_udp_loop(udp_receiver));

    println!("Arrow keys move.");
    println!("L requests a list over TCP.");
    println!("Q quits.");

    enable_raw_mode()?;

    let result = run(&udp_socket, &tcp_tx);

    disable_raw_mode()?;

    result
}

fn spawn_tcp_writer(mut stream: TcpStream, rx: mpsc::Receiver<Vec<u8>>) {
    thread::spawn(move || {
        while let Ok(message) = rx.recv() {
            if let Err(error) = stream.write_all(&message) {
                eprintln!("TCP write error: {error}");
                break;
            }

            if let Err(error) = stream.flush() {
                eprintln!("TCP flush error: {error}");
                break;
            }

            print!(
                "\rTCP sent: {}\r\n",
                String::from_utf8_lossy(&message).trim_end_matches(['\r', '\n'])
            );

            let _ = io::stdout().flush();
        }

        stream.write_all(b"q\n").ok();
        eprintln!("TCP writer stopped");
    });
}

fn spawn_tcp_reader(stream: TcpStream) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut response = String::new();

        loop {
            response.clear();

            match reader.read_line(&mut response) {
                Ok(0) => {
                    eprintln!("TCP server disconnected");
                    break;
                }

                Ok(_) => {
                    println!("\rTCP response: {}", response.trim_end());

                    let _ = io::stdout().flush();
                }

                Err(error) => {
                    eprintln!("TCP read error: {error}");
                    break;
                }
            }
        }
    });
}

fn receive_udp_loop(socket: UdpSocket) {
    let mut buffer = [0_u8; 2048];

    loop {
        match socket.recv(&mut buffer) {
            Ok(bytes_read) => {
                println!(
                    "\rUDP response: {}",
                    String::from_utf8_lossy(&buffer[..bytes_read])
                );

                let _ = io::stdout().flush();
            }

            Err(error) => {
                eprintln!("UDP receive error: {error}");
                break;
            }
        }
    }
}

fn run(udp_socket: &UdpSocket, tcp_tx: &Sender<Vec<u8>>) -> Result<()> {
    loop {
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };

        if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
            continue;
        }

        match key.code {
            /*
             * High-frequency, disposable input goes over UDP.
             */
            KeyCode::Up => {
                udp_socket.send(b"move 0 1")?;
            }

            KeyCode::Down => {
                udp_socket.send(b"move 0 -1")?;
            }

            KeyCode::Left => {
                udp_socket.send(b"move -1 0")?;
            }

            KeyCode::Right => {
                udp_socket.send(b"move 1 0")?;
            }

            KeyCode::Char('s') => {
                udp_socket.send(b"start 0")?;
            }

            KeyCode::Char('x') => {
                udp_socket.send(b"stop 0")?;
            }

            KeyCode::Char('q') => {
                tcp_tx
                    .send(b"q\n".to_vec())
                    .context("failed to send quit command over TCP")?;
                break;
            }

            _ => {}
        }
    }

    Ok(())
}
