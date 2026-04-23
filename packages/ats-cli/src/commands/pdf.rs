//! `ats pdf --out <path>` — render Markdown from stdin to PDF (US-5).

use std::io::Read;
use std::path::PathBuf;

use ats_core::audit::RunFolder;
use ats_core::config::Config;
use ats_core::AtsError;
use ats_core::PdfWriter;
use ats_pdf::Markdown2PdfWriter;

/// Renders PDF from all stdin bytes to `out` (no stdout).
pub async fn handle(
    _cfg: &Config,
    run_folder: &mut RunFolder,
    out: &PathBuf,
) -> Result<(), AtsError> {
    let mut md = String::new();
    std::io::stdin()
        .read_to_string(&mut md)
        .map_err(AtsError::Io)?;
    let writer = Markdown2PdfWriter::new();
    let out_path = out.clone();
    tokio::task::spawn_blocking(move || writer.render(&md, &out_path))
        .await
        .map_err(|e| AtsError::Other(format!("pdf task join: {e}")))?
        .map_err(AtsError::from)?;
    let len = std::fs::metadata(&out)
        .map_err(AtsError::Io)?
        .len() as u64;
    run_folder
        .set_extra("bytes_written", len)
        .map_err(|e| AtsError::Other(e.to_string()))?;
    run_folder
        .set_extra("output_path", out.display().to_string())
        .map_err(|e| AtsError::Other(e.to_string()))?;
    Ok(())
}
