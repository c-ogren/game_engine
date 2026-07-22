//! Pure game simulation: components, the ECS world, and the fixed-timestep
//! step function. This crate knows nothing about networking or terminals; it
//! only understands the shared [`protocol`] vocabulary well enough to apply
//! movement commands to the world.

use glam::Vec2;
use hecs::{Entity, World};
use protocol::Direction;

#[derive(Clone, Copy, Debug)]
pub struct Position(pub Vec2);

#[derive(Clone, Copy, Debug)]
pub struct Velocity(pub Vec2);

/// Marks an entity as a connected player and carries its display name.
/// Networking details (addresses, sockets) deliberately live outside the ECS,
/// in the server's session registry.
#[derive(Clone, Debug)]
pub struct Player {
    pub name: String,
}

/// The simulated world. The game loop is its sole owner, so no locking is needed;
/// Network threads mutate it only by sending commands to the loop.
pub struct Game {
    pub world: World,
}

impl Game {
    /// Spawn `scenery_count` static demo entities. Players are added later,
    /// once they connect, via [`Game::spawn_player`].
    pub fn new(scenery_count: usize) -> Self {
        let mut world = World::new();

        for i in 0..scenery_count {
            world.spawn((Position(Vec2::new(i as f32, 0.0)), Velocity(Vec2::ZERO)));
        }

        Self { world }
    }

    /// Advance the simulation by one fixed timestep.
    pub fn step(&mut self, dt: f32) {
        for (position, velocity) in self.world.query_mut::<(&mut Position, &Velocity)>() {
            position.0 += velocity.0 * dt;
        }
    }

    /// Spawn a player entity and return its handle. The caller (a connection
    /// handler) owns this handle for the lifetime of the session and uses it
    /// to target subsequent commands.
    pub fn spawn_player(&mut self, name: String) -> Entity {
        self.world
            .spawn((Player { name }, Position(Vec2::ZERO), Velocity(Vec2::ZERO)))
    }

    /// Despawn a player entity (e.g. on disconnect).
    pub fn despawn_player(&mut self, entity: Entity) {
        let _ = self.world.despawn(entity);
    }

    /// Nudge a specific entity one unit in the given direction.
    pub fn move_entity(&mut self, entity: Entity, direction: Direction) {
        if let Ok(mut position) = self.world.get::<&mut Position>(entity) {
            position.0 += direction_delta(direction);
        }
    }

    /// Give an entity a nonzero velocity so it drifts each step.
    pub fn start(&mut self, entity: Entity) {
        self.set_velocity(entity, Vec2::new(1.0, 0.5));
    }

    /// Zero an entity's velocity.
    pub fn stop(&mut self, entity: Entity) {
        self.set_velocity(entity, Vec2::ZERO);
    }

    fn set_velocity(&mut self, entity: Entity, value: Vec2) {
        if let Ok(mut velocity) = self.world.get::<&mut Velocity>(entity) {
            velocity.0 = value;
        }
    }
}

fn direction_delta(direction: Direction) -> Vec2 {
    match direction {
        Direction::Up => Vec2::new(0.0, -1.0),
        Direction::Down => Vec2::new(0.0, 1.0),
        Direction::Left => Vec2::new(-1.0, 0.0),
        Direction::Right => Vec2::new(1.0, 0.0),
    }
}
