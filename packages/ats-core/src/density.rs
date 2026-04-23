//! Keyword density (AC-4.3) over optimized Markdown: sum of whole-word term
//! matches per unique primary term, divided by the document word count.

use std::collections::HashSet;

use regex::Regex;
use tracing;

use crate::domain::KeywordSet;

/// Result of [`measure`]: match sum, word count, and `numerator / denominator`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DensityReport {
    pub numerator: u32,
    pub denominator: u32,
    pub density: f32,
}

/// Count whole-word, case-insensitive matches for each **unique** primary term
/// (de-duplicated case-insensitively across categories), divide by the word
/// count of `final_md`, emit one `tracing` line for AC-4.3 bands, and a single
/// human line to stderr. Never returns an error.
pub fn measure(final_md: &str, keywords: &KeywordSet) -> DensityReport {
    let denominator = count_words(final_md);
    if denominator == 0 {
        tracing::error!(target: "ats::optimize", "density: word count is zero; density set to 0.0");
    }

    let terms = unique_primary_terms(keywords);
    let mut numerator: u32 = 0;
    for term in terms {
        if term.is_empty() {
            continue;
        }
        let pattern = format!(r"(?i)\b{}\b", regex::escape(&term));
        match Regex::new(&pattern) {
            Ok(re) => {
                numerator = numerator.saturating_add(re.find_iter(final_md).count() as u32);
            }
            Err(err) => {
                tracing::warn!(
                    target: "ats::optimize",
                    term = %term,
                    error = %err,
                    "density: skipping keyword; invalid regex after escape"
                );
            }
        }
    }

    let density = if denominator > 0 {
        numerator as f32 / denominator as f32
    } else {
        0.0
    };

    let report = DensityReport {
        numerator,
        denominator,
        density,
    };
    emit_log_once(&report);
    report
}

fn unique_primary_terms(keywords: &KeywordSet) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for term in keywords.all_primary_terms() {
        let t = term.trim();
        if t.is_empty() {
            continue;
        }
        let key = t.to_lowercase();
        if seen.insert(key) {
            out.push(t.to_string());
        }
    }
    out
}

fn count_words(text: &str) -> u32 {
    text.split_whitespace()
        .filter(|tok| tok.chars().any(|c| c.is_alphanumeric()))
        .count() as u32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DensityBand {
    /// `< 2%` — one `warn!`.
    Low,
    /// `2%–3%` — no `tracing` line for the band.
    Silent,
    /// `3%–5%` — one `info!`.
    Informational,
    /// `> 5%` — one `warn!`.
    Over,
}

fn map_band(d: f32) -> DensityBand {
    if d < 0.02 {
        DensityBand::Low
    } else if d < 0.03 {
        DensityBand::Silent
    } else if d <= 0.05 {
        DensityBand::Informational
    } else {
        DensityBand::Over
    }
}

/// Emit exactly one `tracing` event for the AC-4.3 band plus a stderr summary line.
fn emit_log_once(report: &DensityReport) {
    let d = report.density;
    match map_band(d) {
        DensityBand::Low => {
            tracing::warn!(
                target: "ats::optimize",
                density = d,
                numerator = report.numerator,
                denominator = report.denominator,
                "density.low"
            );
        }
        DensityBand::Silent => {}
        DensityBand::Informational => {
            tracing::info!(
                target: "ats::optimize",
                density = d,
                numerator = report.numerator,
                denominator = report.denominator,
                "density.informational_upper"
            );
        }
        DensityBand::Over => {
            tracing::warn!(
                target: "ats::optimize",
                density = d,
                numerator = report.numerator,
                denominator = report.denominator,
                "density.over_ceiling"
            );
        }
    }

    let pct = d * 100.0;
    eprintln!(
        "Keyword density: {:.1}% ({} matches / {} words)",
        pct, report.numerator, report.denominator
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{HardSkill, KeywordSet, SoftSkill};

    fn one_term_kw(term: &str) -> KeywordSet {
        KeywordSet {
            hard_skills_and_tools: vec![HardSkill {
                primary_term: term.into(),
                acronym: String::new(),
                semantic_cluster: String::new(),
                importance_score: 1,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn map_band_boundaries() {
        assert_eq!(map_band(0.019), DensityBand::Low);
        assert_eq!(map_band(0.02), DensityBand::Silent);
        assert_eq!(map_band(0.029), DensityBand::Silent);
        assert_eq!(map_band(0.03), DensityBand::Informational);
        assert_eq!(map_band(0.05), DensityBand::Informational);
        assert_eq!(map_band(0.0500001), DensityBand::Over);
    }

    #[test]
    fn dedupes_primary_terms_across_categories_case_insensitively() {
        // "rust" and "RUST" in two lists must not double-apply the same pattern:
        // a single "Rust" in the text should sum to 1 match, not 2.
        let set = KeywordSet {
            hard_skills_and_tools: vec![HardSkill {
                primary_term: "rust".into(),
                acronym: String::new(),
                semantic_cluster: String::new(),
                importance_score: 1,
            }],
            soft_skills_and_competencies: vec![SoftSkill {
                primary_term: "RUST".into(),
                semantic_cluster: String::new(),
                importance_score: 1,
            }],
            industry_specific_terminology: vec![],
            certifications_and_credentials: vec![],
            job_titles_and_seniority: vec![],
        };
        let md = "Professional Rust work.";
        let r = measure(md, &set);
        assert_eq!(r.numerator, 1, "one keyword appearance should not be double-counted");
    }

    #[test]
    fn density_low_band() {
        // 1 / 200 = 0.5%
        let padding = (0..200).map(|_| "a").collect::<Vec<_>>().join(" ");
        let md = format!("{padding} kw");
        let set = one_term_kw("kw");
        let r = measure(&md, &set);
        assert_eq!(r.denominator, 201);
        assert_eq!(r.numerator, 1);
        assert!(r.density < 0.02);
        assert_eq!(map_band(r.density), DensityBand::Low);
    }

    #[test]
    fn density_silent_band_no_tracing_beyond_eprint() {
        // 2 / 100 = 2% — in silent band; map_band is Silent.
        let padding = (0..98).map(|_| "a").collect::<Vec<_>>().join(" ");
        let md = format!("{padding} kw kw");
        let set = one_term_kw("kw");
        let r = measure(&md, &set);
        assert_eq!(r.denominator, 100, "got {}", r.denominator);
        assert_eq!(r.numerator, 2);
        let expected = 2.0f32 / 100.0;
        assert!((r.density - expected).abs() < 1e-5, "got {}", r.density);
        assert_eq!(map_band(r.density), DensityBand::Silent);
    }

    #[test]
    fn density_informational_band() {
        // 3 / 100 = 3% — first informational step.
        let padding = (0..97).map(|_| "a").collect::<Vec<_>>().join(" ");
        let md = format!("{padding} kw kw kw");
        let set = one_term_kw("kw");
        let r = measure(&md, &set);
        assert_eq!(r.denominator, 100);
        assert_eq!(r.numerator, 3);
        assert_eq!(map_band(r.density), DensityBand::Informational);
    }

    #[test]
    fn density_over_ceiling() {
        // 6 / 100 = 6% > 5%
        let padding = (0..94).map(|_| "a").collect::<Vec<_>>().join(" ");
        let kws = (0..6).map(|_| "kw").collect::<Vec<_>>().join(" ");
        let md = format!("{padding} {kws}");
        let set = one_term_kw("kw");
        let r = measure(&md, &set);
        assert_eq!(r.denominator, 100);
        assert_eq!(r.numerator, 6);
        assert_eq!(map_band(r.density), DensityBand::Over);
    }

    #[test]
    fn denominator_zero() {
        let set = one_term_kw("x");
        let r = measure("", &set);
        assert_eq!(r.denominator, 0);
        assert_eq!(r.density, 0.0);
    }

    #[test]
    fn multibyte_term_matches() {
        let set = one_term_kw("世界");
        let md = "你好 世界 世界 其他";
        let r = measure(md, &set);
        assert_eq!(r.numerator, 2);
        assert!(r.denominator >= 4);
    }

    #[test]
    fn uses_fixture_optimized_tiers() {
        let set: KeywordSet = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/optimize/keywords.json"
        )))
        .expect("keywords.json in fixtures");
        for name in ["optimized-low.md", "optimized-mid.md", "optimized-hi.md"] {
            let path = format!("{}/tests/fixtures/optimize/{}", env!("CARGO_MANIFEST_DIR"), name);
            let md = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            let r = measure(&md, &set);
            assert!(r.denominator > 0, "{name} should have words");
            assert!(r.density > 0.0, "fixture {name} should be non-trivial");
        }
    }
}
