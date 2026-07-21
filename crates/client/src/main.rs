//! Terminal controller client. Captures key events with crossterm and sends
//! them to the server as shared [`protocol`] messages over TCP.

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use protocol::{ClientMessage, Direction};
use std::{
    env,
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    thread,
    time::Duration,
};

fn main() -> Result<()> {
    let addr = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: client <host:port>");
        std::process::exit(1);
    });

    let mut stream =
        TcpStream::connect(&addr).with_context(|| format!("failed to connect to {addr}"))?;

    println!("Connected to {addr}");
    println!("Arrow keys move. 's' start, 'x' stop, 'q' quits.");

    // Register a session before entering raw mode.
    send(
        &mut stream,
        &ClientMessage::Join {
            name: "player".to_owned(),
        },
    )?;

    // Print server responses on a background thread, since the main loop
    // blocks polling for key events.
    let reader = stream.try_clone().context("failed to clone stream")?;
    thread::spawn(move || {
        let reader = BufReader::new(reader);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            // Raw mode: use \r\n so output starts at column 0.
            print!("server: {line}\r\n");
            let _ = std::io::stdout().flush();
        }
    });

    enable_raw_mode()?;
    let result = run(&mut stream);
    disable_raw_mode()?;
    result
}

fn run(stream: &mut TcpStream) -> Result<()> {
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
            KeyCode::Up => ClientMessage::Move(Direction::Up),
            KeyCode::Down => ClientMessage::Move(Direction::Down),
            KeyCode::Left => ClientMessage::Move(Direction::Left),
            KeyCode::Right => ClientMessage::Move(Direction::Right),
            KeyCode::Char('s') => ClientMessage::Start(0),
            KeyCode::Char('x') => ClientMessage::Stop(0),
            KeyCode::Char('q') => {
                let _ = send(stream, &ClientMessage::Quit);
                break;
            }
            _ => continue,
        };

        send(stream, &message)?;
    }

    Ok(())
}

fn send(stream: &mut TcpStream, message: &ClientMessage) -> Result<()> {
    // TCP is a stream, so every message is newline-framed.
    writeln!(stream, "{}", message.encode())?;
    stream.flush()?;
    Ok(())
}
