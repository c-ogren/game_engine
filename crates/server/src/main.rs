mod listener;
mod player;
mod state;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use game_core::{Game, Position};
use protocol::ServerMessage;

use crate::listener::{Command, spawn_tcp_control_server, spawn_udp_control_server};
use crate::state::AppState;

const TICK: Duration = Duration::from_nanos(16_666_667); // 60 FPS, 60 Hz
const UDP_SERVER_ADDR: &str = "127.0.0.1:9001";
const TCP_SERVER_ADDR: &str = "127.0.0.1:9000";

fn main() {
    let mut game = Game::new(10);

    let (tx, rx) = mpsc::channel::<Command>();
    let app_state = AppState::new();
    spawn_udp_control_server(UDP_SERVER_ADDR, tx.clone());
    spawn_tcp_control_server(TCP_SERVER_ADDR, tx, app_state.clone());

    // When the server came up, so we can report uptime each frame.
    let start = Instant::now();

    let mut last = Instant::now();
    let mut accu = Duration::ZERO;
    let dt = TICK.as_secs_f32();

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

        while accu >= TICK {
            game.step(dt);
            accu -= TICK;
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
        Command::Move(direction) => game.move_entity(direction),
        Command::Start(id) => game.start(id),
        Command::Stop(id) => game.stop(id),
        Command::Ack { reply, id } => {
            if let Err(error) = reply.send(ServerMessage::Ack { id }) {
                eprintln!("Failed to send ack: {error}");
            }
        }
    }
}

fn print_state(game: &Game, uptime: Duration, accu: Duration, dt: f32, app_state: &AppState) {
    use std::fmt::Write as _;
    use std::io::Write as _;

    // Compose the whole frame in memory, then emit it in a single write.
    // \x1b[H homes the cursor without clearing (no blank frame => no flicker),
    // \x1b[K clears each line to its end so stale characters don't linger.
    let mut buf = String::with_capacity(1024);
    buf.push_str("\x1b[H");
    let _ = writeln!(
        buf,
        "\x1b[KControl server listening on TCP {TCP_SERVER_ADDR} and UDP {UDP_SERVER_ADDR}"
    );
    let _ = writeln!(
        buf,
        "\x1b[KUptime: {:.3} ms, Alpha: {:.3}, dt: {:.3} ms",
        uptime.as_secs_f32() * 1000.0,
        accu.as_secs_f32() / TICK.as_secs_f32(),
        dt * 1000.0
    );
    let _ = writeln!(buf, "\x1b[KEntities in world:");

    for (entity, position) in game.world.query::<(hecs::Entity, &Position)>().iter() {
        let _ = writeln!(buf, "\x1b[KEntity {entity:?} Position: {:?}", position.0);
    }

    for (id, player) in app_state.get_players() {
        let _ = writeln!(buf, "\x1b[KPlayer {id}: {player:?}");
    }

    // Clear anything left below the last line (e.g. if the entity count shrank).
    buf.push_str("\x1b[J");

    let mut out = std::io::stdout().lock();
    let _ = out.write_all(buf.as_bytes());
    let _ = out.flush();
}
