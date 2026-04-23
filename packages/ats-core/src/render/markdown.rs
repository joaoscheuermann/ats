//! Pure Markdown renderer for the baseline resume.
//!
//! Structure follows `assets/resume_template.md` byte-for-byte: section
//! order, blank-line separators (two blank lines between blocks), and
//! bullet characters all match the template (the template uses `*`, so we
//! emit `*`). Optional subtrees that are missing or empty in the YAML are
//! omitted entirely — no empty headings, no placeholder sentinels.
//!
//! `"Present"` is preserved verbatim, and date ranges use the EN DASH
//! (U+2013) exactly as in the template. The output always ends with a
//! single `\n`; no trailing spaces on any line.

use std::fmt::Write as _;

use crate::domain::{Certification, Degree, Job, PersonalInformation, Resume, SkillCategory};

/// Render `resume` into the baseline Markdown string (AC-1.1 / AC-1.2).
///
/// The output is a sequence of section blocks joined by two blank lines
/// (`\n\n\n`), with a single trailing newline. Blocks are emitted in a
/// fixed order:
///
/// 1. Contact header (`# {full_name}` + pipe-joined identifiers).
/// 2. `## Professional Summary` + the trimmed summary.
/// 3. `## Skills` (only when at least one category has items).
/// 4. `## Work Experience` (only when non-empty).
/// 5. `## Education` (only when non-empty).
/// 6. `## Certifications` (only when non-empty).
pub fn render_baseline(resume: &Resume) -> String {
    let mut blocks: Vec<String> = Vec::with_capacity(6);

    blocks.push(render_header(&resume.personal_information));
    blocks.push(render_summary(&resume.professional_summary));

    if let Some(b) = render_skills(&resume.skills) {
        blocks.push(b);
    }
    if let Some(b) = render_work_experience(&resume.work_experience) {
        blocks.push(b);
    }
    if let Some(b) = render_education(&resume.education) {
        blocks.push(b);
    }
    if let Some(b) = render_certifications(&resume.certifications) {
        blocks.push(b);
    }

    let mut out = blocks.join("\n\n\n");
    out.push('\n');
    out
}

fn render_header(p: &PersonalInformation) -> String {
    format!(
        "# {}\n{} | {} | {} | {}",
        p.full_name, p.email, p.phone, p.linkedin_url, p.location
    )
}

fn render_summary(summary: &str) -> String {
    // YAML folded blocks (`>`) leave a trailing newline and sometimes an
    // extra space. Trim the tail so no line ever ends in whitespace.
    let trimmed = summary.trim_end();
    format!("## Professional Summary\n{trimmed}")
}

fn render_skills(skills: &[SkillCategory]) -> Option<String> {
    let lines: Vec<String> = skills
        .iter()
        .filter(|c| !c.items.is_empty())
        .map(|c| format!("* **{}:** {}", c.category, c.items.join(", ")))
        .collect();
    if lines.is_empty() {
        return None;
    }
    Some(format!("## Skills\n{}", lines.join("\n")))
}

fn render_work_experience(jobs: &[Job]) -> Option<String> {
    if jobs.is_empty() {
        return None;
    }
    // The frozen template places two blank lines between `## Work
    // Experience` and the first job, and again between every pair of jobs.
    let rendered: Vec<String> = jobs.iter().map(render_job).collect();
    let mut buf = String::from("## Work Experience\n\n\n");
    buf.push_str(&rendered.join("\n\n\n"));
    Some(buf)
}

fn render_job(job: &Job) -> String {
    let mut out = format!(
        "**{}** | **{}**\n{} | {} \u{2013} {}",
        job.job_title, job.company_name, job.location, job.start_date, job.end_date
    );
    for bullet in &job.bullets {
        if bullet.trim().is_empty() {
            continue;
        }
        let _ = write!(out, "\n* {bullet}");
    }
    out
}

fn render_education(degrees: &[Degree]) -> Option<String> {
    if degrees.is_empty() {
        return None;
    }
    let rendered: Vec<String> = degrees
        .iter()
        .map(|d| {
            format!(
                "**{}**\n{}, {} | {}",
                d.degree_name, d.institution, d.location, d.graduation_date
            )
        })
        .collect();
    let mut buf = String::from("## Education\n\n\n");
    buf.push_str(&rendered.join("\n\n\n"));
    Some(buf)
}

fn render_certifications(certs: &[Certification]) -> Option<String> {
    if certs.is_empty() {
        return None;
    }
    let lines: Vec<String> = certs
        .iter()
        .map(|c| format!("* **{}**, {}, {}", c.title, c.issuing_organization, c.year))
        .collect();
    Some(format!("## Certifications\n{}", lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Certification, Degree, Job, PersonalInformation, Resume, SkillCategory};

    fn sample_personal() -> PersonalInformation {
        PersonalInformation {
            full_name: "Jane Doe".into(),
            email: "jane@example.com".into(),
            phone: "+1 555-0100".into(),
            linkedin_url: "https://linkedin.com/in/jane".into(),
            location: "Remote, US".into(),
        }
    }

    fn minimal_resume() -> Resume {
        Resume {
            personal_information: sample_personal(),
            professional_summary: "Senior engineer with 10 years experience.".into(),
            skills: Vec::new(),
            work_experience: Vec::new(),
            education: Vec::new(),
            certifications: Vec::new(),
        }
    }

    #[test]
    fn minimal_renders_only_required_sections() {
        let rendered = render_baseline(&minimal_resume());
        assert!(rendered.starts_with("# Jane Doe\n"));
        assert!(rendered.contains("## Professional Summary"));
        assert!(!rendered.contains("## Skills"));
        assert!(!rendered.contains("## Work Experience"));
        assert!(!rendered.contains("## Education"));
        assert!(!rendered.contains("## Certifications"));
        assert!(rendered.ends_with('\n'));
    }

    #[test]
    fn header_contains_all_five_identifiers_pipe_joined() {
        let rendered = render_baseline(&minimal_resume());
        let second_line = rendered.lines().nth(1).unwrap();
        assert_eq!(
            second_line,
            "jane@example.com | +1 555-0100 | https://linkedin.com/in/jane | Remote, US"
        );
    }

    #[test]
    fn summary_trims_trailing_whitespace() {
        let mut r = minimal_resume();
        r.professional_summary = "Hello world.\n\n".into();
        let rendered = render_baseline(&r);
        assert!(rendered.contains("## Professional Summary\nHello world."));
        assert!(!rendered.contains("Hello world.\n\n\n\n"));
    }

    #[test]
    fn skills_section_skipped_when_every_category_is_empty() {
        let mut r = minimal_resume();
        r.skills = vec![SkillCategory {
            category: "Empty".into(),
            items: Vec::new(),
        }];
        assert!(!render_baseline(&r).contains("## Skills"));
    }

    #[test]
    fn skills_section_uses_star_bullet_and_joins_items() {
        let mut r = minimal_resume();
        r.skills = vec![SkillCategory {
            category: "Languages".into(),
            items: vec!["Rust".into(), "Python".into()],
        }];
        let rendered = render_baseline(&r);
        assert!(rendered.contains("## Skills\n* **Languages:** Rust, Python"));
    }

    #[test]
    fn work_experience_preserves_present_and_en_dash() {
        let mut r = minimal_resume();
        r.work_experience = vec![Job {
            job_title: "Staff Engineer".into(),
            company_name: "Acme".into(),
            location: "Remote".into(),
            start_date: "01/2020".into(),
            end_date: "Present".into(),
            bullets: vec!["Did great things.".into()],
        }];
        let rendered = render_baseline(&r);
        assert!(rendered.contains("01/2020 \u{2013} Present"));
        assert!(rendered.contains("* Did great things."));
    }

    #[test]
    fn work_experience_omits_bullets_when_empty() {
        let mut r = minimal_resume();
        r.work_experience = vec![Job {
            job_title: "Engineer".into(),
            company_name: "Corp".into(),
            location: "NYC".into(),
            start_date: "01/2018".into(),
            end_date: "12/2019".into(),
            bullets: Vec::new(),
        }];
        let rendered = render_baseline(&r);
        assert!(rendered.contains("NYC | 01/2018 \u{2013} 12/2019"));
        assert!(!rendered.contains("12/2019\n*"));
    }

    #[test]
    fn education_and_certifications_formatting() {
        let mut r = minimal_resume();
        r.education = vec![Degree {
            degree_name: "BSc in CS".into(),
            institution: "State University".into(),
            location: "City, ST".into(),
            graduation_date: "05/2014".into(),
        }];
        r.certifications = vec![Certification {
            title: "AWS Certified Solutions Architect".into(),
            issuing_organization: "Amazon Web Services".into(),
            year: "2022".into(),
        }];
        let rendered = render_baseline(&r);
        assert!(rendered.contains(
            "## Education\n\n\n**BSc in CS**\nState University, City, ST | 05/2014"
        ));
        assert!(rendered.contains(
            "## Certifications\n* **AWS Certified Solutions Architect**, Amazon Web Services, 2022"
        ));
    }

    #[test]
    fn sections_are_separated_by_two_blank_lines() {
        let mut r = minimal_resume();
        r.skills = vec![SkillCategory {
            category: "Languages".into(),
            items: vec!["Rust".into()],
        }];
        let rendered = render_baseline(&r);
        assert!(rendered.contains("Remote, US\n\n\n## Professional Summary"));
        assert!(rendered.contains("10 years experience.\n\n\n## Skills"));
    }
}
