//! ATS keyword extraction result — the typed view over the
//! `ats_keyword_extraction` JSON Schema embedded under
//! `assets/schemas/ats_keyword_extraction.json`.
//!
//! Field names match the schema exactly so `serde_json::from_value` round-trips
//! without extra renames. Five sibling structs sit here rather than in
//! `domain::resume` so the keyword types don't collide with resume
//! [`crate::domain::Certification`] (a keyword-world certification is just a
//! primary term + score, whereas the resume one has issuing org, year, …).

use serde::{Deserialize, Serialize};

/// Technical / tooling entry (hard skill).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HardSkill {
    pub primary_term: String,
    pub acronym: String,
    pub semantic_cluster: String,
    pub importance_score: i32,
}

/// Behavioural / competency entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftSkill {
    pub primary_term: String,
    pub semantic_cluster: String,
    pub importance_score: i32,
}

/// Industry-specific jargon / metric / framework.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndustryTerm {
    pub primary_term: String,
    pub acronym: String,
    pub importance_score: i32,
}

/// Required license, degree, or professional certification. Named
/// `CertificationKeyword` to avoid colliding with [`crate::domain::Certification`]
/// from the resume YAML model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificationKeyword {
    pub primary_term: String,
    pub importance_score: i32,
}

/// Exact role title + seniority indicator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobTitle {
    pub primary_term: String,
    pub importance_score: i32,
}

/// Full extracted keyword catalogue. The five category names match the top-level
/// keys of the `ats_keyword_extraction` JSON Schema verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct KeywordSet {
    pub hard_skills_and_tools: Vec<HardSkill>,
    pub soft_skills_and_competencies: Vec<SoftSkill>,
    pub industry_specific_terminology: Vec<IndustryTerm>,
    pub certifications_and_credentials: Vec<CertificationKeyword>,
    pub job_titles_and_seniority: Vec<JobTitle>,
}

impl KeywordSet {
    /// Iterate every primary term across all five categories. Used by
    /// Effort 05's density metric.
    pub fn all_primary_terms(&self) -> impl Iterator<Item = &str> {
        self.hard_skills_and_tools
            .iter()
            .map(|k| k.primary_term.as_str())
            .chain(
                self.soft_skills_and_competencies
                    .iter()
                    .map(|k| k.primary_term.as_str()),
            )
            .chain(
                self.industry_specific_terminology
                    .iter()
                    .map(|k| k.primary_term.as_str()),
            )
            .chain(
                self.certifications_and_credentials
                    .iter()
                    .map(|k| k.primary_term.as_str()),
            )
            .chain(
                self.job_titles_and_seniority
                    .iter()
                    .map(|k| k.primary_term.as_str()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialises_from_schema_shape() {
        let body = serde_json::json!({
            "hard_skills_and_tools": [{
                "primary_term": "Rust",
                "acronym": "",
                "semantic_cluster": "systems programming",
                "importance_score": 9
            }],
            "soft_skills_and_competencies": [{
                "primary_term": "Communication",
                "semantic_cluster": "collaboration",
                "importance_score": 5
            }],
            "industry_specific_terminology": [{
                "primary_term": "HIPAA",
                "acronym": "Health Insurance Portability and Accountability Act",
                "importance_score": 7
            }],
            "certifications_and_credentials": [{
                "primary_term": "AWS Certified Solutions Architect",
                "importance_score": 6
            }],
            "job_titles_and_seniority": [{
                "primary_term": "Senior Software Engineer",
                "importance_score": 8
            }]
        });
        let parsed: KeywordSet = serde_json::from_value(body).unwrap();
        assert_eq!(parsed.hard_skills_and_tools[0].primary_term, "Rust");
        assert!(!parsed.industry_specific_terminology[0].acronym.is_empty());
    }

    #[test]
    fn all_primary_terms_covers_every_category() {
        let set = KeywordSet {
            hard_skills_and_tools: vec![HardSkill {
                primary_term: "Rust".into(),
                acronym: "".into(),
                semantic_cluster: "".into(),
                importance_score: 9,
            }],
            soft_skills_and_competencies: vec![SoftSkill {
                primary_term: "Leadership".into(),
                semantic_cluster: "".into(),
                importance_score: 4,
            }],
            industry_specific_terminology: vec![IndustryTerm {
                primary_term: "KPI".into(),
                acronym: "Key Performance Indicator".into(),
                importance_score: 7,
            }],
            certifications_and_credentials: vec![CertificationKeyword {
                primary_term: "PMP".into(),
                importance_score: 3,
            }],
            job_titles_and_seniority: vec![JobTitle {
                primary_term: "Staff Engineer".into(),
                importance_score: 8,
            }],
        };
        let terms: Vec<&str> = set.all_primary_terms().collect();
        assert_eq!(
            terms,
            vec!["Rust", "Leadership", "KPI", "PMP", "Staff Engineer"]
        );
    }
}
