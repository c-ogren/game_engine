# Some sort of game_engine

A small client/server game engine, organized as a Cargo workspace.

## Crates

- `crates/protocol` — shared wire types (`ClientMessage`, `ServerMessage`, `Direction`) and their text encode/decode. Dependency-free; both client and server depend on it.
- `crates/game-core` — the ECS simulation (`Position`, `Velocity`, `Game::step`). Depends on `protocol`, `hecs`, `glam`.
- `crates/server` — the game loop plus TCP and UDP control servers. Depends on `game-core` and `protocol`.
- `crates/client` — the terminal controller. Depends on `protocol` (and `crossterm`).

## Running

Run the server (game loop + control servers on TCP 9000 / UDP 9001):

```
cargo run -p server
```

Run the client controller (arrow keys move, `s` start, `x` stop, `q` quit):

```
cargo run -p client 127.0.0.1:9000
```
