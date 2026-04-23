//! Append-only JSON-lines audit writer.
//!
//! Each call to [`FileAuditSink::record`] serialises the supplied
//! [`LlmCallRecord`] and writes a single line terminated with `\n`. Writes are
//! serialised through an internal [`Mutex`] so concurrent audits from multiple
//! tasks cannot interleave partial JSON.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ats_core::ports::{AuditSink, LlmCallRecord};

/// Append-only JSON-lines sink backed by a single file on disk.
pub struct FileAuditSink {
    path: PathBuf,
    inner: Mutex<BufWriter<File>>,
}

impl FileAuditSink {
    /// Create or truncate the file at `path`. Any existing content is
    /// replaced — the run folder is brand-new per invocation, so there is no
    /// prior state worth preserving.
    pub fn create(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)?;
        Ok(Self {
            path,
            inner: Mutex::new(BufWriter::new(file)),
        })
    }

    /// Truncate/ create the file, write `initial` records, then keep the handle
    /// open for more [`AuditSink::record`] calls.
    pub fn create_with_records(
        path: impl Into<PathBuf>,
        initial: impl IntoIterator<Item = LlmCallRecord>,
    ) -> io::Result<Self> {
        let s = Self::create(path)?;
        for r in initial {
            s.record(&r)?;
        }
        Ok(s)
    }

    /// Absolute path of the audit log. Primarily useful in tests.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl AuditSink for FileAuditSink {
    fn record(&self, call: &LlmCallRecord) -> io::Result<()> {
        let line = serde_json::to_string(call)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("audit sink mutex poisoned"))?;
        guard.write_all(line.as_bytes())?;
        guard.write_all(b"\n")?;
        guard.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ats_core::ports::TokenUsage;
    use tempfile::tempdir;

    fn sample(attempt: u32, outcome: &str) -> LlmCallRecord {
        LlmCallRecord {
            timestamp: "2026-04-22T14:30:15Z".into(),
            stage: "keywords".into(),
            model: "x/y".into(),
            temperature: 0.0,
            seed: Some(42),
            prompt: "prompt-bytes".into(),
            response: "response-bytes".into(),
            usage: TokenUsage {
                prompt: 100,
                completion: 50,
                total: 150,
            },
            attempt,
            outcome: outcome.into(),
        }
    }

    #[test]
    fn round_trip_three_records() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("llm-audit.jsonl");
        let sink = FileAuditSink::create(&path).unwrap();
        sink.record(&sample(1, "transient")).unwrap();
        sink.record(&sample(2, "transient")).unwrap();
        sink.record(&sample(3, "ok")).unwrap();
        drop(sink);
        let body = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 3);
        for line in &lines {
            let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(parsed["stage"], "keywords");
            assert_eq!(parsed["usage"]["total"], 150);
        }
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(lines[2]).unwrap()["outcome"],
            "ok"
        );
    }

    #[test]
    fn create_makes_parent_dirs() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("deep/nested/log.jsonl");
        let sink = FileAuditSink::create(&path).unwrap();
        sink.record(&sample(1, "ok")).unwrap();
        assert!(path.exists());
    }
}
