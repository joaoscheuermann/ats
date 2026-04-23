//! Filesystem layout discovery. Per NFC-20 / AC-6.4, every artifact lives in
//! the same directory as the binary for this version.
//!
//! A trait seam (`FsLayout`) lets tests and later Efforts inject an arbitrary
//! root so unit tests don't need to juggle `current_exe()`.

use std::io;
use std::path::{Path, PathBuf};

/// Abstract "where on disk does this binary keep things" contract.
///
/// Designed narrow on purpose (ISP): each consumer uses only what it needs,
/// and future relocations (installed vs portable) can plug in new
/// implementations without touching callers.
pub trait FsLayout {
    /// Directory containing the binary. All other paths are relative to this.
    fn binary_dir(&self) -> &Path;
    /// Location of `config.json`.
    fn config_path(&self) -> PathBuf;
    /// Baseline cache dir (`cache/`).
    fn cache_dir(&self) -> PathBuf;
    /// Per-invocation runs dir (`runs/`).
    fn runs_dir(&self) -> PathBuf;
    /// Final output dir (`output/` — only `ats run` writes here in the final pipeline).
    fn output_dir(&self) -> PathBuf;

    /// Idempotently create `cache/`, `runs/`, and `output/` under the binary dir.
    fn ensure_dirs(&self) -> io::Result<()> {
        for dir in [self.cache_dir(), self.runs_dir(), self.output_dir()] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

/// Default `FsLayout` rooted at the directory of the current executable.
///
/// Prefer explicit construction via `new_from_current_exe` or `new_rooted_at`
/// so tests and `main` both stay explicit about which root is in use (DI).
#[derive(Debug, Clone)]
pub struct BinaryFsLayout {
    binary_dir: PathBuf,
    config_override: Option<PathBuf>,
}

impl BinaryFsLayout {
    /// Discover `binary_dir` from `std::env::current_exe()`.
    pub fn new_from_current_exe() -> io::Result<Self> {
        let exe = std::env::current_exe()?;
        let parent = exe
            .parent()
            .ok_or_else(|| {
                io::Error::other(format!(
                    "cannot resolve parent of exe: {}",
                    exe.display()
                ))
            })?
            .to_path_buf();
        Ok(Self::new_rooted_at(parent))
    }

    /// Build a layout rooted at an arbitrary directory (used by tests and by
    /// `--config` overrides). No canonicalization — we keep the path as-is to
    /// avoid surprises with Windows UNC prefixes.
    pub fn new_rooted_at(dir: impl Into<PathBuf>) -> Self {
        Self {
            binary_dir: dir.into(),
            config_override: None,
        }
    }

    /// Set an explicit `config.json` path (from the `--config` CLI flag).
    pub fn with_config_override(mut self, path: Option<PathBuf>) -> Self {
        self.config_override = path;
        self
    }
}

impl FsLayout for BinaryFsLayout {
    fn binary_dir(&self) -> &Path {
        &self.binary_dir
    }

    fn config_path(&self) -> PathBuf {
        self.config_override
            .clone()
            .unwrap_or_else(|| self.binary_dir.join("config.json"))
    }

    fn cache_dir(&self) -> PathBuf {
        self.binary_dir.join("cache")
    }

    fn runs_dir(&self) -> PathBuf {
        self.binary_dir.join("runs")
    }

    fn output_dir(&self) -> PathBuf {
        self.binary_dir.join("output")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rooted_layout_returns_expected_children() {
        let dir = tempdir().unwrap();
        let layout = BinaryFsLayout::new_rooted_at(dir.path());
        assert_eq!(layout.config_path(), dir.path().join("config.json"));
        assert_eq!(layout.cache_dir(), dir.path().join("cache"));
        assert_eq!(layout.runs_dir(), dir.path().join("runs"));
        assert_eq!(layout.output_dir(), dir.path().join("output"));
    }

    #[test]
    fn ensure_dirs_creates_all_three() {
        let dir = tempdir().unwrap();
        let layout = BinaryFsLayout::new_rooted_at(dir.path());
        layout.ensure_dirs().unwrap();
        assert!(layout.cache_dir().is_dir());
        assert!(layout.runs_dir().is_dir());
        assert!(layout.output_dir().is_dir());
    }

    #[test]
    fn config_override_wins() {
        let dir = tempdir().unwrap();
        let custom = dir.path().join("custom.json");
        let layout = BinaryFsLayout::new_rooted_at(dir.path())
            .with_config_override(Some(custom.clone()));
        assert_eq!(layout.config_path(), custom);
    }
}
