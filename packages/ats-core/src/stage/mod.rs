//! Pipeline stages. Each stage is a pure(ish) async function that accepts the
//! ports it depends on (`LlmClient`, `AuditSink`, …) as `&dyn Trait`
//! parameters. Effort 03 lands `keywords`; Efforts 04–07 add the remaining
//! stages.

pub mod keywords;
pub mod optimize;
pub mod scrape;
