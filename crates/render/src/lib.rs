//! Headless terminal repainting.
//!
//! Both the server (its local status view) and the client (incoming world
//! snapshots) want the same flicker-free, in-place repaint: compose the whole
//! frame in memory, home the cursor, clear each line as we go, and clear
//! anything left below. This crate is that shared "printing function" and
//! nothing else — it knows about ANSI escapes, not about the game or the wire
//! protocol.
//!
//! The escapes used:
//! * `\x1b[H`  — move the cursor home (top-left) without clearing, so there's
//!   no blank frame and therefore no flicker.
//! * `\x1b[K`  — clear from the cursor to the end of the line, so stale
//!   characters from a longer previous frame don't linger.
//! * `\x1b[J`  — clear everything below the cursor, e.g. when the entity count
//!   shrank between frames.
//! * `\r\n`    — carriage-return + line-feed, so the next line starts at column
//!   0 even in raw mode (where the terminal doesn't translate `\n`).

use std::io::Write;

/// Compose a full frame from `lines` into a single string, ready to be written
/// to a terminal in one shot.
pub fn frame<I, S>(lines: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut buf = String::from("\x1b[H");
    for line in lines {
        buf.push_str("\x1b[K");
        buf.push_str(line.as_ref());
        buf.push_str("\r\n");
    }
    // Clear anything left below the last line (e.g. if the line count shrank).
    buf.push_str("\x1b[J");
    buf
}

/// Compose and write a full frame to stdout in a single write, then flush.
pub fn paint<I, S>(lines: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let frame = frame(lines);
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(frame.as_bytes());
    let _ = out.flush();
}

/// Hide the cursor and clear the screen. Call once before the first [`paint`]
/// so the cursor doesn't blink/jump while repainting and no stale content
/// remains from before.
pub fn enter() {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(b"\x1b[?25l\x1b[2J");
    let _ = out.flush();
}

/// Show the cursor again. Call when leaving the repaint view.
pub fn leave() {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(b"\x1b[?25h");
    let _ = out.flush();
}
