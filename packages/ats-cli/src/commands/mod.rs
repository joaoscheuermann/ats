//! Subcommand handlers wired by `main::dispatch`.
//!
//! Each module owns a single `pub async fn handle(...)` that accepts the
//! parsed clap args plus the shared context (`FsLayout`, `RunFolder`). Keeping
//! them here — rather than a match arm inside `main.rs` — lets each handler
//! pull in just the ports it needs without bloating `main` (SRP).

pub mod keywords;
pub mod optimize;
pub mod pdf;
pub mod render;
pub mod run;
pub mod scrape;
