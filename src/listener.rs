use core::str;
use std::io::Read;
use std::net::TcpListener;
use std::sync::mpsc::Sender;
use std::thread;

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy)]
pub enum Command {
    Stop(u64),
    Start(u64),
    Move(Direction),
    List,
}

pub fn spawn_control_server(tx: Sender<Command>) {
    thread::spawn(move || {
        let listener = TcpListener::bind("127.0.0.1:9000").unwrap();
        println!("Control server listening on 127.0.0.1:9000");
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let tx = tx.clone();

            thread::spawn(move || {
                if let Err(e) = handle_client(stream, tx) {
                    eprintln!("Error handling client: {}", e);
                }
            });
        }
    });
}

pub fn handle_client(mut stream: std::net::TcpStream, tx: Sender<Command>) -> std::io::Result<()> {
    let mut read_buffer = [0_u8; 256];
    let mut pending = Vec::new();

    loop {
        let bytes_read = stream.read(&mut read_buffer)?;

        if bytes_read == 0 {
            break;
        }

        pending.extend_from_slice(&read_buffer[..bytes_read]);

        while let Some(consumed) = parse_next_input(&pending, &tx) {
            pending.drain(..consumed);
        }
    }

    Ok(())
}

fn parse_next_input(input: &[u8], tx: &Sender<Command>) -> Option<usize> {
    // Arrow keys commonly arrive as:
    // Up    = ESC [ A
    // Down  = ESC [ B
    // Right = ESC [ C
    // Left  = ESC [ D
    if input.starts_with(b"\x1b[A") {
        tx.send(Command::Move(Direction::Up)).ok()?;
        return Some(3);
    }

    if input.starts_with(b"\x1b[B") {
        tx.send(Command::Move(Direction::Down)).ok()?;
        return Some(3);
    }

    if input.starts_with(b"\x1b[C") {
        tx.send(Command::Move(Direction::Right)).ok()?;
        return Some(3);
    }

    if input.starts_with(b"\x1b[D") {
        tx.send(Command::Move(Direction::Left)).ok()?;
        return Some(3);
    }

    // TCP may split an arrow-key sequence across multiple reads.
    if input == b"\x1b" || input == b"\x1b[" {
        return None;
    }

    // Keep supporting line-based commands such as:
    // start 4
    // stop 4
    // list
    if let Some(newline_index) = input.iter().position(|byte| *byte == b'\n') {
        let line: &[u8] = &input[..newline_index];
        let line: &[u8] = line.strip_suffix(b"\r").unwrap_or(line);

        if let Ok(line) = std::str::from_utf8(line)
            && let Some(command) = parse_command(line)
        {
            let _ = tx.send(command);
        }

        return Some(newline_index + 1);
    }

    // Discard an unknown escape sequence or unsupported byte so the parser
    // cannot become permanently stuck.
    if input.starts_with(b"\x1b") && input.len() >= 3 {
        return Some(3);
    }

    None
}

fn parse_command(line: &str) -> Option<Command> {
    let mut parts: std::str::SplitWhitespace<'_> = line.split_whitespace();

    let command = match parts.next()? {
        "stop" => Command::Stop(parts.next()?.parse().ok()?),
        "start" => Command::Start(parts.next()?.parse().ok()?),
        "list" => Command::List,
        _ => return None,
    };

    // Reject trailing arguments such as `list nonsense`.
    if parts.next().is_some() {
        return None;
    }

    Some(command)
}
