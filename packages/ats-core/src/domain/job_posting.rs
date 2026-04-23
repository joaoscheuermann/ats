//! Domain type for a scraped job posting (US-2).
//!
//! The pipeline uses [`JobPosting`] in memory; `ats scrape` serialises it
//! to stdout and to `posting.json` inside the run folder. `markdown` is
//! written verbatim to `posting.md` by later stages.

use serde::{Deserialize, Serialize};

/// In-memory representation of the scraped posting.
///
/// `title` is the raw title as the LLM extracted it (used by the slug
/// sanitizer); `markdown` is the clean-Markdown body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobPosting {
    pub title: String,
    pub markdown: String,
}
