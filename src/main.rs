use glam::Vec2;
use hecs::World;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use game_engine::listener;
use game_engine::state::AppState;

// -- Components
#[derive(Clone, Copy)]
struct Position(Vec2);
#[derive(Clone, Copy)]
struct Velocity(Vec2);

const TICK: Duration = Duration::from_nanos(16_666_667); // 60 FPS, 60 Hz
const UDP_SERVER_ADDR: &str = "127.0.0.1:9001";
const TCP_SERVER_ADDR: &str = "127.0.0.1:9000"; // TODO: unused, but we could add a TCP control server in parallel to the UDP one.

fn main() {
    let mut world = World::new();

    // Keep the real Entity handles, indexed by command id, so we never have to
    // fabricate a handle from a raw id (which is unsafe and can silently alias
    // the wrong entity).
    let mut entities: Vec<hecs::Entity> = Vec::new();
    for i in 0..10 {
        let entity = world.spawn((
            Position(Vec2::new(i as f32, 0.0)),
            Velocity(Vec2::new(0.0, 0.0)),
        ));
        entities.push(entity);
    }

    let (tx, rx) = mpsc::channel::<listener::Command>();
    let app_state = AppState::new();
    listener::spawn_udp_control_server(UDP_SERVER_ADDR, tx.clone());
    listener::spawn_tcp_control_server(TCP_SERVER_ADDR, tx, app_state.clone());

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

        while let Ok(cmd) = rx.try_recv() {
            apply_command(&mut world, &entities, cmd);
        }

        while accu >= TICK {
            step(&mut world, dt);
            accu -= TICK;
        }

        if now - last_render >= render_interval {
            print_entities(&world, start.elapsed(), accu, dt, &app_state);
            last_render = now;
        }
        // TODO: network snapshot / send happens here.
        std::thread::sleep(Duration::from_millis(1)); // avoid busy spinning
    }
}

fn step(world: &mut World, dt: f32) {
    for (pos, vel) in world.query_mut::<(&mut Position, &Velocity)>() {
        pos.0 += vel.0 * dt;
    }
}

fn print_entities(world: &World, uptime: Duration, accu: Duration, dt: f32, app_state: &AppState) {
    use std::fmt::Write as _;
    use std::io::Write as _;

    // Compose the whole frame in memory, then emit it in a single write.
    // \x1b[H homes the cursor without clearing (no blank frame => no flicker),
    // \x1b[K clears each line to its end so stale characters don't linger.
    let mut buf = String::with_capacity(1024);
    buf.push_str("\x1b[H");
    let _ = writeln!(
        buf,
        "\x1b[KControl server listening on TCP {} and UDP {}",
        TCP_SERVER_ADDR, UDP_SERVER_ADDR
    );
    let _ = writeln!(buf, "\x1b[KEntities in world:");
    let _ = writeln!(
        buf,
        "\x1b[KUptime: {:.3} ms, Alpha: {:.3}, dt: {:.3} ms",
        uptime.as_secs_f32() * 1000.0,
        accu.as_secs_f32() / TICK.as_secs_f32(),
        dt * 1000.0
    );

    for (entity, pos) in world.query::<(hecs::Entity, &Position)>().iter() {
        let _ = writeln!(buf, "\x1b[KEntity {:?} Position: {:?}", entity, pos.0);
    }

    for (id, player) in app_state.get_players() {
        let _ = writeln!(buf, "\x1b[KPlayer {}: {:?}", id, player);
    }

    // Clear anything left below the last line (e.g. if the entity count shrank).
    buf.push_str("\x1b[J");

    let mut out = std::io::stdout().lock();
    let _ = out.write_all(buf.as_bytes());
    let _ = out.flush();
}

fn apply_command(world: &mut World, entities: &[hecs::Entity], cmd: listener::Command) {
    match cmd {
        listener::Command::Stop(id) => {
            if let Some(&entity) = entities.get(id as usize) {
                if let Ok(mut vel) = world.get::<&mut Velocity>(entity) {
                    vel.0 = glam::Vec2::ZERO;
                }
            }
        }
        listener::Command::Start(id) => {
            if let Some(&entity) = entities.get(id as usize) {
                if let Ok(mut vel) = world.get::<&mut Velocity>(entity) {
                    vel.0 = glam::Vec2::new(1.0, 0.5);
                }
            }
        }
        listener::Command::Ack((reply, id)) => {
            reply
                .send(format!("ACK {id}").to_string())
                .unwrap_or_else(|e| {
                    eprintln!("Failed to send ACK: {e}");
                });
        }
        listener::Command::Move(dir) => {
            if let Some(&entity) = entities.first() {
                if let Ok(mut pos) = world.get::<&mut Position>(entity) {
                    let delta = match dir {
                        listener::Direction::Up => glam::Vec2::new(0.0, -1.0),
                        listener::Direction::Down => glam::Vec2::new(0.0, 1.0),
                        listener::Direction::Left => glam::Vec2::new(-1.0, 0.0),
                        listener::Direction::Right => glam::Vec2::new(1.0, 0.0),
                    };

                    pos.0 += delta;
                }
            }
        }
    }
}
