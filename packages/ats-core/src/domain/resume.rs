//! Resume domain model — the serde shape of the YAML contract locked in the
//! journal ticket's `## Research` section.
//!
//! Top-level YAML is `{ cv: { ... } }`; callers of [`Resume::parse_bytes`]
//! receive the inner `cv` block as [`Resume`]. Only `personal_information`
//! and `professional_summary` are required (AC-1.3); every other field
//! defaults to an empty vector so the renderer can simply check
//! `is_empty()` to decide whether to emit a section.

use serde::{Deserialize, Serialize};

/// Contact block rendered on the top of the baseline Markdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalInformation {
    pub full_name: String,
    pub email: String,
    pub phone: String,
    pub linkedin_url: String,
    pub location: String,
}

/// One entry under `cv.skills` — a bold label followed by a joined term list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillCategory {
    pub category: String,
    #[serde(default)]
    pub items: Vec<String>,
}

/// One entry under `cv.work_experience`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub job_title: String,
    pub company_name: String,
    pub location: String,
    pub start_date: String,
    /// Free-form string; `"Present"` is preserved verbatim by the renderer.
    pub end_date: String,
    #[serde(default)]
    pub bullets: Vec<String>,
}

/// One entry under `cv.education`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Degree {
    pub degree_name: String,
    pub institution: String,
    pub location: String,
    pub graduation_date: String,
}

/// One entry under `cv.certifications`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Certification {
    pub title: String,
    pub issuing_organization: String,
    pub year: String,
}

/// Resume as rendered — i.e. the `cv` block, with optional sections defaulted
/// to empty vectors so the renderer can stay branch-light (KISS).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resume {
    pub personal_information: PersonalInformation,
    pub professional_summary: String,
    #[serde(default)]
    pub skills: Vec<SkillCategory>,
    #[serde(default)]
    pub work_experience: Vec<Job>,
    #[serde(default)]
    pub education: Vec<Degree>,
    #[serde(default)]
    pub certifications: Vec<Certification>,
}

/// Top-level YAML shape — the outer `cv` wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeYaml {
    pub cv: Resume,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_optional_vecs_default_to_empty() {
        let yaml = r#"
cv:
  personal_information:
    full_name: "Jane"
    email: "jane@x"
    phone: "555"
    linkedin_url: "https://linkedin"
    location: "Remote"
  professional_summary: "One line."
"#;
        let parsed: ResumeYaml = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(parsed.cv.skills.is_empty());
        assert!(parsed.cv.work_experience.is_empty());
        assert!(parsed.cv.education.is_empty());
        assert!(parsed.cv.certifications.is_empty());
    }

    #[test]
    fn end_date_present_is_preserved_verbatim() {
        let yaml = r#"
cv:
  personal_information:
    full_name: "Jane"
    email: "jane@x"
    phone: "555"
    linkedin_url: "https://linkedin"
    location: "Remote"
  professional_summary: "One line."
  work_experience:
    - job_title: "Engineer"
      company_name: "Acme"
      location: "Remote"
      start_date: "01/2023"
      end_date: "Present"
"#;
        let parsed: ResumeYaml = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(parsed.cv.work_experience[0].end_date, "Present");
    }
}
