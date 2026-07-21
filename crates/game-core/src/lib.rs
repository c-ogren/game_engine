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

/// The simulated world plus stable handles to the entities we spawned.
pub struct Game {
    pub world: World,
    /// Entity handles indexed by command id, so callers never fabricate a
    /// handle from a raw id (which is unsafe and can silently alias the wrong
    /// entity).
    pub entities: Vec<Entity>,
}

impl Game {
    /// Spawn `entity_count` entities laid out along the x axis, at rest.
    pub fn new(entity_count: usize) -> Self {
        let mut world = World::new();
        let mut entities = Vec::with_capacity(entity_count);

        for i in 0..entity_count {
            let entity = world.spawn((Position(Vec2::new(i as f32, 0.0)), Velocity(Vec2::ZERO)));
            entities.push(entity);
        }

        Self { world, entities }
    }

    /// Advance the simulation by one fixed timestep.
    pub fn step(&mut self, dt: f32) {
        for (position, velocity) in self.world.query_mut::<(&mut Position, &Velocity)>() {
            position.0 += velocity.0 * dt;
        }
    }

    /// Nudge the first entity one unit in the given direction.
    pub fn move_entity(&mut self, direction: Direction) {
        let Some(&entity) = self.entities.first() else {
            return;
        };

        if let Ok(mut position) = self.world.get::<&mut Position>(entity) {
            position.0 += direction_delta(direction);
        }
    }

    /// Give the entity with `id` a nonzero velocity.
    pub fn start(&mut self, id: u64) {
        self.set_velocity(id, Vec2::new(1.0, 0.5));
    }

    /// Zero the velocity of the entity with `id`.
    pub fn stop(&mut self, id: u64) {
        self.set_velocity(id, Vec2::ZERO);
    }

    fn set_velocity(&mut self, id: u64, value: Vec2) {
        let Some(&entity) = self.entities.get(id as usize) else {
            return;
        };

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
