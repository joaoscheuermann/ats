//! Rendering pipeline for `ats render`.
//!
//! - [`validate`] — YAML bytes → typed [`crate::domain::Resume`], with
//!   path-qualified diagnostics (AC-1.3 / AC-1.4).
//! - [`markdown`] — pure [`crate::domain::Resume`] → Markdown renderer that
//!   mirrors the frozen `assets/resume_template.md`.
//! - [`cache`] — SHA-256 keyed content cache under `<binary_dir>/cache/`
//!   (AC-1.5).

pub mod cache;
pub mod markdown;
pub mod validate;

pub use markdown::render_baseline;
