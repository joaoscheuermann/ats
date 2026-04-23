//! Structured logging init. NFC-15 says logs are **always on**, written to
//! stderr, and default to JSON lines. `--log-format=pretty` swaps in the
//! colorized human formatter.
//!
//! Install the subscriber exactly once per process (second calls are
//! best-effort no-ops so tests and re-entrant callers don't panic).
//!
//! # Noise control
//!
//! `chromiumoxide`'s WebSocket handler logs `WS Invalid message: …` at WARN
//! for every CDP event its generated `Message` enum doesn't recognise. That
//! set grows with every Chromium release, so on a current Chrome we can
//! easily see dozens of warnings per navigation — all harmless, all about
//! events we never subscribed to. We install a default directive that mutes
//! those crates below ERROR so `ats`'s own logs stay readable. The user can
//! still override everything via `RUST_LOG`.

use std::sync::Once;

use tracing_subscriber::EnvFilter;

/// Default filter directive applied when `RUST_LOG` is unset.
///
/// - `info` for everything by default (matches tracing_subscriber's default).
/// - `chromiumoxide*=error` silences the `WS Invalid message` / `WS
///   Connection error` spam emitted when Chrome sends CDP messages the
///   generated protocol enum doesn't know.
/// - `tungstenite=error` / `async_tungstenite=error` cover the WS transport
///   layer that sits under chromiumoxide.
const DEFAULT_FILTER: &str = "info,chromiumoxide=error,chromiumoxide_cdp=error,\
     chromiumoxide_types=error,tungstenite=error,async_tungstenite=error";

/// Which formatter to install on stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Machine-readable JSON lines (default).
    Json,
    /// Human-readable pretty output.
    Pretty,
}

impl LogFormat {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "json" => Ok(LogFormat::Json),
            "pretty" => Ok(LogFormat::Pretty),
            other => Err(format!(
                "unknown --log-format value `{other}` (expected `json` or `pretty`)"
            )),
        }
    }
}

impl std::str::FromStr for LogFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        LogFormat::parse(s)
    }
}

static INIT: Once = Once::new();

/// Install the global `tracing_subscriber`. Writes to **stderr** exclusively
/// so subcommand stdout stays clean for pipelines (AC-6.3).
pub fn init(format: LogFormat) {
    INIT.call_once(|| {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
        let builder = tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(filter);
        let _ = match format {
            LogFormat::Json => builder.json().try_init(),
            LogFormat::Pretty => builder.try_init(),
        };
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trip() {
        assert_eq!(LogFormat::parse("json").unwrap(), LogFormat::Json);
        assert_eq!(LogFormat::parse("pretty").unwrap(), LogFormat::Pretty);
        assert!(LogFormat::parse("xml").is_err());
    }
}
