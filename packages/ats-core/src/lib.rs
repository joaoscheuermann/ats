//! Core domain types, ports, embedded assets, and cross-stage plumbing for the
//! `ats` resume optimizer CLI.
//!
//! This crate is concretion-free: it owns the data model, the error taxonomy
//! with locked exit codes, the config loader, the filesystem layout seam, the
//! per-invocation run folder helper, the logging initializer, and the locked
//! assets shipped inside the binary. Adapter crates (`ats-scrape`, `ats-llm`,
//! `ats-pdf`) depend on `ats-core`; the CLI wires everything together at
//! `fn main`.

pub mod assets;
pub mod audit;
pub mod config;
pub mod domain;
pub mod error;
pub mod fs_layout;
pub mod logging;
pub mod ports;
pub mod render;
pub mod density;
pub mod pipeline;
pub mod scrape_port;
pub mod slug;
pub mod stage;

pub use audit::{format_run_dir_ts, RunFolder};
pub use config::{Config, ModelStageConfig};
pub use domain::{
    Certification, CertificationKeyword, Degree, HardSkill, IndustryTerm, Job, JobPosting,
    JobTitle, KeywordSet, PersonalInformation, Resume, ResumeYaml, SkillCategory, SoftSkill,
};
pub use error::{AtsError, LlmClass, ScrapeClass, YamlDiag};
pub use fs_layout::{BinaryFsLayout, FsLayout};
pub use logging::{init as init_logging, LogFormat};
pub use ports::{
    AuditSink, ChatMessage, ChatRole, Clock, LlmCallRecord, LlmClient, LlmError, LlmRequest,
    LlmResponse, PdfError, PdfWriter, SystemClock, TokenUsage, VecAuditSink,
};
pub use pipeline::{
    run, RunPipelineError, RunPipelineInput, RunPipelineResult, StagesBundle,
};
pub use render::{
    cache::{hash_key, load_or_render, CacheResult},
    markdown::render_baseline,
    validate::{json_pointer_to_dotted, parse_and_validate},
};
pub use scrape_port::{PageScraper, ScrapeError};
pub use slug::{sanitize_title, MAX_SLUG_CHARS};
