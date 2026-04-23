//! Tee audit records across multiple [`AuditSink`] implementations.
//!
//! Lets the CLI layer install a file sink plus one or more observer sinks
//! (token-usage stderr reporter, etc.) without either one knowing about the
//! other. Each child sink sees the same record in the order they were added;
//! the first I/O error short-circuits the call.

use std::io;
use std::sync::Arc;

use ats_core::ports::{AuditSink, LlmCallRecord};

/// Fan-out sink. Records are forwarded to each child in registration order.
pub struct CompositeAuditSink {
    sinks: Vec<Arc<dyn AuditSink>>,
}

impl CompositeAuditSink {
    pub fn new(sinks: Vec<Arc<dyn AuditSink>>) -> Self {
        Self { sinks }
    }

    pub fn with_sinks<I>(sinks: I) -> Self
    where
        I: IntoIterator<Item = Arc<dyn AuditSink>>,
    {
        Self {
            sinks: sinks.into_iter().collect(),
        }
    }
}

impl AuditSink for CompositeAuditSink {
    fn record(&self, call: &LlmCallRecord) -> io::Result<()> {
        for sink in &self.sinks {
            sink.record(call)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ats_core::ports::TokenUsage;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemSink {
        records: Mutex<Vec<LlmCallRecord>>,
    }

    impl AuditSink for MemSink {
        fn record(&self, call: &LlmCallRecord) -> io::Result<()> {
            self.records.lock().unwrap().push(call.clone());
            Ok(())
        }
    }

    fn rec(outcome: &str) -> LlmCallRecord {
        LlmCallRecord {
            timestamp: "t".into(),
            stage: "keywords".into(),
            model: "m".into(),
            temperature: 0.0,
            seed: None,
            prompt: "p".into(),
            response: "r".into(),
            usage: TokenUsage::ZERO,
            attempt: 1,
            outcome: outcome.into(),
        }
    }

    #[test]
    fn forwards_to_every_child_in_order() {
        let a: Arc<MemSink> = Arc::new(MemSink::default());
        let b: Arc<MemSink> = Arc::new(MemSink::default());
        let composite = CompositeAuditSink::new(vec![a.clone(), b.clone()]);
        composite.record(&rec("ok")).unwrap();
        assert_eq!(a.records.lock().unwrap().len(), 1);
        assert_eq!(b.records.lock().unwrap().len(), 1);
        assert_eq!(a.records.lock().unwrap()[0].outcome, "ok");
    }
}
