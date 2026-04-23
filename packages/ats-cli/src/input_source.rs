//! CLI-only adapter: `InputSource` reads `-` as stdin and everything else as
//! a file path. This is a presentation concern, so it lives in `ats-cli`
//! rather than `ats-core`.

use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputSource {
    /// Read from standard input.
    Stdin,
    /// Read from a file on disk.
    File(PathBuf),
}

/// Read a full UTF-8 string from a file or from `stdin` when [`InputSource`]
/// is [`InputSource::Stdin`]. Callers must ensure at most one source uses
/// stdin (see the `ats optimize` guard in `main.rs`).
pub fn read_input<R: Read + ?Sized>(source: &InputSource, stdin: &mut R) -> io::Result<String> {
    match source {
        InputSource::File(path) => fs::read_to_string(path),
        InputSource::Stdin => {
            let mut buf = String::new();
            stdin.read_to_string(&mut buf)?;
            Ok(buf)
        }
    }
}

impl InputSource {
    /// Returns true when this source reads standard input (`-` on the CLI).
    pub fn is_stdin(&self) -> bool {
        matches!(self, InputSource::Stdin)
    }

    /// Human-readable, snapshot-safe description for run.json.
    pub fn describe(&self) -> String {
        match self {
            InputSource::Stdin => "-".to_string(),
            InputSource::File(p) => p.display().to_string(),
        }
    }
}

impl FromStr for InputSource {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "-" {
            return Ok(InputSource::Stdin);
        }
        Ok(InputSource::File(PathBuf::from(s)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dash_is_stdin() {
        let s: InputSource = "-".parse().unwrap();
        assert!(s.is_stdin());
        assert_eq!(s.describe(), "-");
    }

    #[test]
    fn anything_else_is_a_path() {
        let s: InputSource = "resume.md".parse().unwrap();
        assert!(!s.is_stdin());
        assert_eq!(s.describe(), "resume.md");
    }
}
