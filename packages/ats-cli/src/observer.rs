//! Stage-agnostic stderr audit observer.
//!
//! Emits one human-readable summary line per LLM audit record (NFC-17).
//! Stage handlers construct one of these, wrap it in an `Arc<dyn AuditSink>`,
//! and fan it out alongside a [`ats_llm::FileAuditSink`] via
//! [`ats_llm::CompositeAuditSink`].

use std::io::{self, Write};
use std::sync::Mutex;

use ats_core::{AuditSink, LlmCallRecord};

/// Observer sink: summarises each audit record as a single line on a
/// provided writer. Production wires `Box<dyn Write + Send>` pointing at
/// `std::io::stderr()`; tests can drop in an in-memory buffer.
pub struct StderrUsageReporter {
    stage: &'static str,
    writer: Mutex<Box<dyn Write + Send>>,
}

impl StderrUsageReporter {
    /// Build a reporter tagged with `stage` (appears at the start of each
    /// line as `[<stage>]`). Writes through `writer`.
    pub fn new(stage: &'static str, writer: Box<dyn Write + Send>) -> Self {
        Self {
            stage,
            writer: Mutex::new(writer),
        }
    }
}

impl AuditSink for StderrUsageReporter {
    fn record(&self, call: &LlmCallRecord) -> io::Result<()> {
        let line = format!(
            "[{stage}] tokens: prompt={p} completion={c} total={t} attempt={a} outcome={o}\n",
            stage = self.stage,
            p = call.usage.prompt,
            c = call.usage.completion,
            t = call.usage.total,
            a = call.attempt,
            o = call.outcome,
        );
        let mut guard = self
            .writer
            .lock()
            .map_err(|_| io::Error::other("reporter writer mutex poisoned"))?;
        guard.write_all(line.as_bytes())?;
        guard.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ats_core::ports::TokenUsage;
    use std::sync::Arc;

    struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedBuffer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn reporter_formats_line() {
        let shared = Arc::new(Mutex::new(Vec::<u8>::new()));
        let reporter = StderrUsageReporter::new("scrape", Box::new(SharedBuffer(shared.clone())));
        reporter
            .record(&LlmCallRecord {
                timestamp: "t".into(),
                stage: "scrape".into(),
                model: "m".into(),
                temperature: 0.0,
                seed: None,
                prompt: "p".into(),
                response: "r".into(),
                usage: TokenUsage {
                    prompt: 100,
                    completion: 50,
                    total: 150,
                },
                attempt: 1,
                outcome: "ok".into(),
            })
            .unwrap();
        drop(reporter);
        let s = String::from_utf8(shared.lock().unwrap().clone()).unwrap();
        assert!(s.contains("[scrape] tokens:"), "got: {s}");
        assert!(s.contains("prompt=100"));
        assert!(s.contains("outcome=ok"));
    }
}
