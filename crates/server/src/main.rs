mod config;
mod listener;
mod state;
use config::Config;

use std::net::UdpSocket;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use game_core::{Game, Player, Position};
use protocol::ServerMessage;

use crate::listener::{Command, spawn_tcp_control_server, spawn_udp_control_server};
use crate::state::AppState;

fn main() {
    let Config {
        tcp_server_addr,
        udp_server_addr,
        tick_rate,
        scenery_count,
    } = Config::from_env();

    if let Err(error) = init_logging() {
        eprintln!("Failed to initialize logging: {error}");
        std::process::exit(1);
    }
    log::info!("server starting (TCP {tcp_server_addr}, UDP {udp_server_addr})");

    let mut game = Game::new(scenery_count);

    let (tx, rx) = mpsc::channel::<Command>();
    let app_state = AppState::new();

    // Bind the UDP socket up front and share it: the listener thread receives on
    // one clone while the game loop sends per-tick snapshots on another. Both
    // clones refer to the same underlying socket.
    let udp_socket = match UdpSocket::bind(&udp_server_addr) {
        Ok(socket) => socket,
        Err(error) => {
            eprintln!("failed to bind UDP server to {udp_server_addr}: {error}");
            std::process::exit(1);
        }
    };
    let listener_socket = match udp_socket.try_clone() {
        Ok(socket) => socket,
        Err(error) => {
            eprintln!("failed to clone UDP socket: {error}");
            std::process::exit(1);
        }
    };

    spawn_udp_control_server(listener_socket, tx.clone(), app_state.clone());
    spawn_tcp_control_server(tcp_server_addr, tx, app_state.clone());

    // When the server came up, so we can report uptime each frame.
    let start = Instant::now();

    let mut last = Instant::now();
    let mut accu = Duration::ZERO;
    let dt = tick_rate.as_secs_f32();

    // Throttle rendering: the sim can tick fast, but the terminal only needs
    // to be repainted at a human-visible rate (~60 Hz here).
    let render_interval = Duration::from_millis(16);
    let mut last_render = Instant::now();

    // Hide the cursor so it doesn't blink/jump around while we repaint, and
    // clear the screen once before the first frame.
    render::enter();

    loop {
        let now = Instant::now();

        accu += now - last;
        last = now;

        while let Ok(command) = rx.try_recv() {
            apply_command(&mut game, command);
        }

        let mut ticked = false;
        while accu >= tick_rate {
            game.step(dt);
            accu -= tick_rate;
            ticked = true;
        }

        // Every tick that advanced the world, push a fresh snapshot to every
        // subscribed client over UDP.
        if ticked {
            broadcast_snapshot(&game, &udp_socket, &app_state);
        }

        if now - last_render >= render_interval {
            print_state(&game, start.elapsed(), accu, dt, &app_state);
            last_render = now;
        }

        std::thread::sleep(Duration::from_millis(1)); // avoid busy spinning
    }
}

/// Serialize the world and send it to every UDP subscriber.
fn broadcast_snapshot(game: &Game, socket: &UdpSocket, app_state: &AppState) {
    let subscribers = app_state.subscribers();
    if subscribers.is_empty() {
        return;
    }

    let message = ServerMessage::Snapshot {
        entities: game.snapshot(),
    };
    let bytes = message.encode();

    for address in subscribers {
        if let Err(error) = socket.send_to(bytes.as_bytes(), address) {
            log::warn!("failed to send snapshot to {address}: {error}");
        }
    }
}

fn apply_command(game: &mut Game, command: Command) {
    match command {
        Command::Join { name, reply } => {
            let entity = game.spawn_player(name);
            if let Err(error) = reply.send(entity) {
                log::error!("failed to return spawned player entity: {error}");
                // The connection went away before we could reply; despawn the
                // now-orphaned player.
                game.despawn_player(entity);
            }
        }
        Command::Leave { entity } => game.despawn_player(entity),
        Command::Move { entity, dir } => game.move_entity(entity, dir),
        Command::Start { entity } => game.start(entity),
        Command::Stop { entity } => game.stop(entity),
    }
}

fn init_logging() -> anyhow::Result<()> {
    use simplelog::{Config, LevelFilter, WriteLogger};

    std::fs::create_dir_all("logs")?;
    let file = std::fs::File::create("logs/server.log")?;
    WriteLogger::init(LevelFilter::Info, Config::default(), file)?;
    Ok(())
}

fn print_state(game: &Game, uptime: Duration, accu: Duration, dt: f32, app_state: &AppState) {
    let Config {
        tcp_server_addr,
        udp_server_addr,
        tick_rate,
        ..
    } = Config::from_env();

    // Build the frame as a list of lines; `render::paint` handles the in-place
    // repaint (cursor home, per-line clear, clear-below, single write).
    let mut lines = Vec::new();
    lines.push(format!(
        "Control server listening on TCP {tcp_server_addr} and UDP {udp_server_addr}"
    ));
    lines.push(format!(
        "Uptime: {:.3} ms, Alpha: {:.3}, dt: {:.3} ms",
        uptime.as_secs_f32() * 1000.0,
        accu.as_secs_f32() / tick_rate.as_secs_f32(),
        dt * 1000.0
    ));
    lines.push("Entities in world:".to_owned());

    // Players carry a `Player` component; scenery entities don't.
    for (entity, position, player) in game
        .world
        .query::<(hecs::Entity, &Position, Option<&Player>)>()
        .iter()
    {
        match player {
            Some(player) => lines.push(format!(
                "Player {} ({}) Position: {:?}",
                entity.id(),
                player.name,
                position.0
            )),
            None => lines.push(format!("Entity {entity:?} Position: {:?}", position.0)),
        }
    }

    lines.push("Sessions (networking):".to_owned());
    for session in app_state.sessions() {
        let udp = session
            .udp_address
            .map(|address| address.to_string())
            .unwrap_or_else(|| "unsubscribed".to_owned());
        lines.push(format!(
            "Session {} (token {}) TCP {} UDP {} -> entity {}",
            session.id,
            session.token,
            session.tcp_address,
            udp,
            session.entity.id()
        ));
    }

    render::paint(&lines);
}
