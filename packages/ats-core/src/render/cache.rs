//! SHA-256 keyed content cache for baseline renders (AC-1.5).
//!
//! Key = `Sha256(normalize_eol(yaml_bytes) || b"\n---\n" || normalize_eol(RESUME_TEMPLATE))`
//! in lowercase hex. Path = `<fs_layout.cache_dir()>/baseline-<hex>.md`.
//!
//! Both inputs are line-ending-normalised (every `\r` byte stripped) before
//! hashing so that identical logical content produces the same key regardless
//! of whether the caller's file system or `git core.autocrlf` delivered CRLF
//! or LF. Without this, the same YAML + template would hash to one value on
//! Windows checkouts and another on Linux checkouts, orphaning caches across
//! machines. Only the hash key is normalised — cached file contents on disk
//! remain exactly what [`render_baseline`] produced (LF-only).
//!
//! On hit, the cached bytes are returned verbatim without re-rendering and
//! without validating their contents (cached output is trusted — invalidation
//! is driven exclusively by the key). On miss, the renderer runs, and the
//! result is written atomically (`<path>.tmp` → rename) so partial files can
//! never fool a subsequent hit.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::assets::RESUME_TEMPLATE;
use crate::domain::Resume;
use crate::fs_layout::FsLayout;

use super::render_baseline;

/// Outcome of a [`load_or_render`] call.
pub struct CacheResult {
    /// The rendered baseline Markdown (from cache or freshly rendered).
    pub content: String,
    /// `true` iff the cache file already existed and was reused.
    pub cached: bool,
    /// Absolute path of the cache file.
    pub path: PathBuf,
}

/// Compute the cache key for a given YAML input against the embedded
/// [`RESUME_TEMPLATE`]. Exposed for diagnostics (e.g. logging the path a CLI
/// call would have read).
pub fn hash_key(yaml_bytes: &[u8]) -> String {
    hash_key_with_template(yaml_bytes, RESUME_TEMPLATE.as_bytes())
}

/// Testing / future-proof variant: hash with an arbitrary template byte slice.
/// The public API always uses the embedded template; tests use this to prove
/// that editing the template changes the key and that CRLF/LF variants of the
/// template hash identically.
///
/// Both byte slices are normalised via [`normalize_eol`] before feeding SHA-256
/// so hashes are stable across `core.autocrlf` differences between machines.
pub fn hash_key_with_template(yaml_bytes: &[u8], template_bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalize_eol(yaml_bytes));
    hasher.update(b"\n---\n");
    hasher.update(normalize_eol(template_bytes));
    encode_hex_lowercase(&hasher.finalize())
}

/// Strip every `\r` byte so CRLF and LF inputs hash identically. The separator
/// `b"\n---\n"` is literal LF and is not routed through this helper.
fn normalize_eol(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().copied().filter(|b| *b != b'\r').collect()
}

/// Cache-first render. Inserts the rendered file on miss; returns the bytes
/// and a `cached` flag. Does not touch [`FsLayout::runs_dir`] / output dirs.
pub fn load_or_render(
    layout: &dyn FsLayout,
    yaml_bytes: &[u8],
    resume: &Resume,
) -> io::Result<CacheResult> {
    let hex = hash_key(yaml_bytes);
    let cache_dir = layout.cache_dir();
    let path = cache_dir.join(format!("baseline-{hex}.md"));

    if path.exists() {
        let content = fs::read_to_string(&path)?;
        return Ok(CacheResult {
            content,
            cached: true,
            path,
        });
    }

    fs::create_dir_all(&cache_dir)?;
    let rendered = render_baseline(resume);
    write_atomic(&path, rendered.as_bytes())?;
    Ok(CacheResult {
        content: rendered,
        cached: false,
        path,
    })
}

fn write_atomic(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut tmp = target.to_path_buf();
    let mut name = tmp
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    tmp.set_file_name(name);
    fs::write(&tmp, bytes)?;
    // On Windows, `fs::rename` overwrites only when the destination doesn't
    // exist — which is the case here (we short-circuit on `path.exists()`).
    // If an older tmp is lingering, `fs::write` above truncates and rewrites
    // it, so the rename target is always fresh.
    fs::rename(&tmp, target)
}

fn encode_hex_lowercase(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{PersonalInformation, Resume};
    use crate::fs_layout::BinaryFsLayout;
    use tempfile::tempdir;

    fn sample_resume() -> Resume {
        Resume {
            personal_information: PersonalInformation {
                full_name: "Jane Doe".into(),
                email: "jane@example.com".into(),
                phone: "+1 555-0100".into(),
                linkedin_url: "https://linkedin.com/in/jane".into(),
                location: "Remote".into(),
            },
            professional_summary: "A summary.".into(),
            skills: Vec::new(),
            work_experience: Vec::new(),
            education: Vec::new(),
            certifications: Vec::new(),
        }
    }

    #[test]
    fn hash_is_stable_and_lowercase_hex() {
        let hex = hash_key(b"anything");
        assert_eq!(hex.len(), 64, "sha256 hex is 64 chars, got {hex}");
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(hex, hash_key(b"anything"));
    }

    #[test]
    fn hash_differs_by_yaml_and_by_template() {
        let a = hash_key(b"one");
        let b = hash_key(b"two");
        assert_ne!(a, b, "different YAML must hash differently");

        let same_yaml_other_template =
            hash_key_with_template(b"one", b"## different template body");
        assert_ne!(a, same_yaml_other_template, "template changes must invalidate cache");
    }

    #[test]
    fn first_call_writes_cache_second_call_hits() {
        let dir = tempdir().unwrap();
        let layout = BinaryFsLayout::new_rooted_at(dir.path());
        layout.ensure_dirs().unwrap();

        let yaml = b"cv: { anything: value }"; // bytes are hashed, not parsed here
        let resume = sample_resume();

        let miss = load_or_render(&layout, yaml, &resume).unwrap();
        assert!(!miss.cached, "first call must be a miss");
        assert!(miss.path.is_file(), "cache file must be written");
        let written = fs::read_to_string(&miss.path).unwrap();
        assert_eq!(written, miss.content, "written file matches returned content");

        let hit = load_or_render(&layout, yaml, &resume).unwrap();
        assert!(hit.cached, "second call must be a hit");
        assert_eq!(hit.content, miss.content);
        assert_eq!(hit.path, miss.path);
    }

    #[test]
    fn hash_key_is_stable_across_crlf_vs_lf_yaml() {
        let lf = b"cv:\n  personal_information:\n    full_name: Jane\n";
        let crlf = b"cv:\r\n  personal_information:\r\n    full_name: Jane\r\n";
        assert_eq!(
            hash_key(lf),
            hash_key(crlf),
            "CRLF/LF variants of identical YAML must hash to the same key"
        );
    }

    #[test]
    fn hash_key_changes_when_non_newline_bytes_change() {
        // Guard against over-eager normalisation collapsing meaningful bytes.
        let a = hash_key(b"cv:\n  full_name: Jane\n");
        let b = hash_key(b"cv:\n  full_name: John\n");
        assert_ne!(a, b, "different non-newline content must hash differently");
    }

    #[test]
    fn hash_key_is_stable_if_template_has_crlf() {
        let yaml = b"cv:\n  full_name: Jane\n";
        let template_lf = b"# {{full_name}}\n\n## Summary\n{{summary}}\n";
        let template_crlf = b"# {{full_name}}\r\n\r\n## Summary\r\n{{summary}}\r\n";
        assert_eq!(
            hash_key_with_template(yaml, template_lf),
            hash_key_with_template(yaml, template_crlf),
            "CRLF template must hash identically to LF template"
        );
    }

    #[test]
    fn different_yaml_bytes_yield_different_cache_files() {
        let dir = tempdir().unwrap();
        let layout = BinaryFsLayout::new_rooted_at(dir.path());
        layout.ensure_dirs().unwrap();

        let resume = sample_resume();
        let a = load_or_render(&layout, b"one", &resume).unwrap();
        let b = load_or_render(&layout, b"two", &resume).unwrap();
        assert_ne!(a.path, b.path);
    }
}
