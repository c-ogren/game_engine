use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::{
    env,
    io::{Read, Write},
    net::TcpStream,
    thread,
    time::Duration,
};

fn main() -> Result<()> {
    let addr = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: controller <host:port>");
        std::process::exit(1);
    });

    let mut stream =
        TcpStream::connect(&addr).with_context(|| format!("failed to connect to {addr}"))?;

    println!("Connected to {addr}");
    println!("Arrow keys move.");
    println!("Q quits.");

    enable_raw_mode()?;

    // Print anything the server sends back (e.g. the entity list) on a
    // background thread, since the main loop blocks polling for key events.
    let mut reader = stream
        .try_clone()
        .context("failed to clone stream for reader")?;
    thread::spawn(move || {
        let mut buf = [0_u8; 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut out = std::io::stdout();
                    let _ = out.write_all(&buf[..n]);
                    let _ = out.flush();
                }
            }
        }
    });

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

        match key.code {
            KeyCode::Up => {
                stream.write_all(b"\x1b[A")?;
            }

            KeyCode::Down => {
                stream.write_all(b"\x1b[B")?;
            }

            KeyCode::Left => {
                stream.write_all(b"\x1b[D")?;
            }

            KeyCode::Right => {
                stream.write_all(b"\x1b[C")?;
            }

            KeyCode::Char('q') => {
                break;
            }

            KeyCode::Char('l') => {
                stream.write_all(b"l")?;
            }

            KeyCode::Char('s') => {
                stream.write_all(b"start 0\n")?;
            }

            KeyCode::Char('x') => {
                stream.write_all(b"stop 0\n")?;
            }

            _ => {}
        }

        stream.flush()?;
    }

    Ok(())
}
