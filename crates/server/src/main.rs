mod config;
mod listener;
mod state;
use config::Config;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use game_core::{Game, Player, Position};

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
    spawn_udp_control_server(udp_server_addr, tx.clone(), app_state.clone());
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

    // Hide the cursor so it doesn't blink/jump around while we repaint.
    {
        use std::io::Write;
        print!("\x1b[?25l\x1b[2J");
        std::io::stdout().flush().unwrap();
    }

    loop {
        let now = Instant::now();

        accu += now - last;
        last = now;

        while let Ok(command) = rx.try_recv() {
            apply_command(&mut game, command);
        }

        while accu >= tick_rate {
            game.step(dt);
            accu -= tick_rate;
        }

        if now - last_render >= render_interval {
            print_state(&game, start.elapsed(), accu, dt, &app_state);
            last_render = now;
        }

        // TODO: network snapshot / send happens here.

        std::thread::sleep(Duration::from_millis(1)); // avoid busy spinning
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
    use std::fmt::Write as _;
    use std::io::Write as _;
    let Config {
        tcp_server_addr,
        udp_server_addr,
        tick_rate,
        ..
    } = Config::from_env();

    // Compose the whole frame in memory, then emit it in a single write.
    // \x1b[H homes the cursor without clearing (no blank frame => no flicker),
    // \x1b[K clears each line to its end so stale characters don't linger.
    let mut buf = String::with_capacity(1024);
    buf.push_str("\x1b[H");
    let _ = writeln!(
        buf,
        "\x1b[KControl server listening on TCP {tcp_server_addr} and UDP {udp_server_addr}"
    );
    let _ = writeln!(
        buf,
        "\x1b[KUptime: {:.3} ms, Alpha: {:.3}, dt: {:.3} ms",
        uptime.as_secs_f32() * 1000.0,
        accu.as_secs_f32() / tick_rate.as_secs_f32(),
        dt * 1000.0
    );
    let _ = writeln!(buf, "\x1b[KEntities in world:");

    // Players carry a `Player` component; scenery entities don't.
    for (entity, position, player) in game
        .world
        .query::<(hecs::Entity, &Position, Option<&Player>)>()
        .iter()
    {
        match player {
            Some(player) => {
                let _ = writeln!(
                    buf,
                    "\x1b[KPlayer {} ({}) Position: {:?}",
                    entity.id(),
                    player.name,
                    position.0
                );
            }
            None => {
                let _ = writeln!(buf, "\x1b[KEntity {entity:?} Position: {:?}", position.0);
            }
        }
    }

    let _ = writeln!(buf, "\x1b[KSessions (networking):");
    for session in app_state.sessions() {
        let _ = writeln!(
            buf,
            "\x1b[KSession {} at {} -> entity {}",
            session.id,
            session.address,
            session.entity.id()
        );
    }

    // Clear anything left below the last line (e.g. if the entity count shrank).
    buf.push_str("\x1b[J");

    let mut out = std::io::stdout().lock();
    let _ = out.write_all(buf.as_bytes());
    let _ = out.flush();
}
