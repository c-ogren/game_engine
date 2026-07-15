use glam::Vec2;
use hecs::World;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub mod listener;

// -- Components
#[derive(Clone, Copy)]
struct Position(Vec2);
#[derive(Clone, Copy)]
struct Velocity(Vec2);

const TICK: Duration = Duration::from_nanos(16_666_667); // 60 FPS, 60 Hz

fn main() {
    let mut world = World::new();

    for i in 0..10 {
        world.spawn((
            Position(Vec2::new(i as f32, 0.0)),
            Velocity(Vec2::new(0.0, 0.0)),
        ));
    }

    let (tx, rx) = mpsc::channel::<listener::Command>();
    listener::spawn_control_server(tx);

    let mut last = Instant::now();
    let mut accu = Duration::ZERO;
    let dt = TICK.as_secs_f32();

    loop {
        let now = Instant::now();

        accu += now - last;
        last = now;

        while let Ok(cmd) = rx.try_recv() {
            apply_command(&mut world, cmd);
        }

        while accu >= TICK {
            step(&mut world, dt);
            accu -= TICK;
        }

        print_entities(&world, accu, dt);
        // TODO: network snapshot / send happens here.
        std::thread::sleep(Duration::from_millis(1)); //avoid busy spinning
    }
}

fn step(world: &mut World, dt: f32) {
    for (pos, vel) in world.query_mut::<(&mut Position, &Velocity)>() {
        pos.0 += vel.0 * dt;
    }
}

fn print_entities(world: &World, accu: Duration, dt: f32) {
    use std::io::Write;
    print!("\x1b[H\x1b[2J");
    println!("Entities in world:");
    println!(
        "Accumulated time: {:.3} ms, dt: {:.3} ms",
        accu.as_secs_f32() * 1000.0,
        dt * 1000.0
    );

    for (entity, pos) in world.query::<(hecs::Entity, &Position)>().iter() {
        println!("\x1b[2KEntity {:?} Position: {:?}\r\n", entity, pos.0);
    }

    std::io::stdout().flush().unwrap();
}

fn apply_command(world: &mut World, cmd: listener::Command) {
    match cmd {
        listener::Command::Stop(id) => {
            let entity = unsafe { world.find_entity_from_id(id.try_into().unwrap()) };
            if let Ok(mut vel) = world.get::<&mut Velocity>(entity) {
                vel.0 = glam::Vec2::ZERO;
            }
        }
        listener::Command::Start(id) => {
            let entity = unsafe { world.find_entity_from_id(id.try_into().unwrap()) };
            if let Ok(mut vel) = world.get::<&mut Velocity>(entity) {
                vel.0 = glam::Vec2::new(1.0, 0.5);
            }
        }
        listener::Command::List => {
            println!("Listing entities:");
            for (entity, pos) in world.query::<(hecs::Entity, &Position)>().iter() {
                println!("Entity {:?} Position: {:?}", entity, pos.0);
            }
        }
        listener::Command::Move(dir) => {
            let entity = unsafe { world.find_entity_from_id(0) }; // Assuming we want to move entity with ID 0
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
