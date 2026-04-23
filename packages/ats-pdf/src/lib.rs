//! Markdown-to-PDF via `markdown2pdf` (US-5).

use std::fs;
use std::path::Path;

use ats_core::ports::{PdfError, PdfWriter};
use markdown2pdf::config::ConfigSource;

const PDF_MAGIC: &[u8] = b"%PDF-";

/// Embedded TOML styling shipped with the crate. Compiled in so rendering is
/// deterministic and does not depend on `~/markdown2pdfrc.toml` or any file
/// on the user’s machine.
const RESUME_STYLE_TOML: &str = include_str!("resume_style.toml");

/// Renders styled PDFs using the embedded resume style configuration.
pub struct Markdown2PdfWriter;

impl Markdown2PdfWriter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Markdown2PdfWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl PdfWriter for Markdown2PdfWriter {
    fn render(&self, markdown: &str, out: &Path) -> Result<(), PdfError> {
        let fname = out
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| PdfError::Render("output path has no file name".into()))?;
        let tmp = out
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!("{fname}.tmp"));
        // We do NOT auto-create the parent directory — missing parents are a
        // caller error and must bubble up as `PdfError::Render` → exit 7.
        let path_str: String = tmp.to_string_lossy().into();
        if let Err(e) = markdown2pdf::parse_into_file(
            markdown.to_string(),
            &path_str,
            ConfigSource::Embedded(RESUME_STYLE_TOML),
            None,
        ) {
            let _ = fs::remove_file(&tmp);
            return Err(PdfError::Render(e.to_string()));
        }
        if let Err(e) = fs::rename(&tmp, out) {
            let _ = fs::remove_file(&tmp);
            return Err(PdfError::Render(e.to_string()));
        }
        if !out.exists() {
            return Err(PdfError::Render("expected pdf path missing after render".into()));
        }
        let head = first_bytes(out, 5).map_err(PdfError::Render)?;
        if head != PDF_MAGIC {
            return Err(PdfError::Render("output is not a valid PDF (missing %PDF- magic)".into()));
        }
        Ok(())
    }
}

fn first_bytes(path: &Path, n: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut v = vec![0u8; n];
    f.read_exact(&mut v).map_err(|e| e.to_string())?;
    Ok(v)
}

pub const CRATE_NAME: &str = "ats-pdf";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_markdown_yields_valid_pdf() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out.pdf");
        Markdown2PdfWriter::new()
            .render(
                "# Hello\n\n**World** and some *italic* text.\n",
                &out,
            )
            .expect("render");
        let bytes = fs::read(&out).expect("read pdf");
        assert!(bytes.len() > 200, "expected non-trivial PDF, got {} bytes", bytes.len());
        assert_eq!(&bytes[..5], b"%PDF-");
    }
}
