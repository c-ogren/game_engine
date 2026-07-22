# Some sort of game_engine

A small client/server game engine, organized as a Cargo workspace.

## Crates

- `crates/protocol` — shared wire types (`ClientMessage`, `ServerMessage`, `EntitySnapshot`, `Direction`, `Token`) and their text encode/decode. Dependency-free; both client and server depend on it.
- `crates/game-core` — the ECS simulation (`Position`, `Velocity`, `Game::step`, `Game::snapshot`). Depends on `protocol`, `hecs`, `glam`.
- `crates/render` — the shared, headless terminal repaint (`render::frame`/`paint`/`enter`/`leave`). Used by both binaries so the server's status view and the client's snapshot view print identically.
- `crates/server` — the game loop plus TCP and UDP control servers. Depends on `game-core`, `protocol`, and `render`.
- `crates/client` — the terminal controller. Depends on `protocol`, `render` (and `crossterm`).

## Transports

Responsibilities are split by transport:

- **TCP** — session lifecycle only: `Join` / `Quit`. The join `Ack` hands the client a session *token*.
- **UDP** — all gameplay: the client subscribes with `Hello { token }`, then streams `Move` / `Start` / `Stop` (each tagged with its token). The server pushes a world `Snapshot` to every subscribed client each tick.

## Running

Run the server (game loop + control servers on TCP 9000 / UDP 9001):

```
cargo run -p server
```

Run the client controller (arrow keys move, `s` start, `x` stop, `q` quit). It
takes the TCP address (session) and the UDP address (gameplay):

```
cargo run -p client 127.0.0.1:9000 127.0.0.1:9001
```
