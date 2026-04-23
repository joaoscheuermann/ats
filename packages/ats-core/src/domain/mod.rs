//! Domain types for the ATS resume optimizer.
//!
//! This module is concretion-free and owns the plain-old-data models used by
//! stages and by the CLI. Parsing/validation logic that turns YAML bytes into
//! a [`resume::Resume`] lives in [`crate::render::validate`].

pub mod job_posting;
pub mod keywords;
pub mod resume;

pub use job_posting::JobPosting;
pub use keywords::{
    CertificationKeyword, HardSkill, IndustryTerm, JobTitle, KeywordSet, SoftSkill,
};
pub use resume::{
    Certification, Degree, Job, PersonalInformation, Resume, ResumeYaml, SkillCategory,
};
