//! Config loader for `config.json` sitting next to the binary.
//!
//! No defaults — every field is required (see journal ticket §6 and
//! AC-approved amendment dropping `pdf.style`). On missing or malformed file,
//! produce `AtsError::Config` with a field path via `serde_path_to_error`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AtsError;

/// Full shape of `config.json`. All fields required; no defaults. See the
/// journal ticket for the locked layout.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub openrouter: OpenRouterConfig,
    pub models: ModelsConfig,
    pub scrape: ScrapeConfig,
    pub retries: RetriesConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OpenRouterConfig {
    pub api_key: String,
    pub base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelsConfig {
    pub scrape_to_markdown: ModelStageConfig,
    pub keyword_extraction: ModelStageConfig,
    pub resume_optimization: ModelStageConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelStageConfig {
    pub name: String,
    pub temperature: f32,
    pub seed: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScrapeConfig {
    pub network_idle_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RetriesConfig {
    pub llm_transient_max_attempts: u32,
    pub llm_transient_backoff_ms: Vec<u64>,
    pub schema_validation_max_attempts: u32,
}

impl Config {
    /// Read + parse `config.json` at `path`. Missing file produces a
    /// `Config("config.json not found at <path>")` error; malformed content
    /// produces `Config("<path>: <message>")` with the JSON-path prefix when
    /// `serde_path_to_error` can identify one.
    pub fn load_from(path: &Path) -> Result<Self, AtsError> {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(AtsError::Config(format!(
                    "config.json not found at {}",
                    path.display()
                )));
            }
            Err(err) => {
                return Err(AtsError::Config(format!(
                    "{}: cannot read: {}",
                    path.display(),
                    err
                )));
            }
        };

        let de = &mut serde_json::Deserializer::from_slice(&bytes);
        let config: Config = serde_path_to_error::deserialize(de).map_err(|e| {
            AtsError::Config(format!(
                "{}: {} at `{}`",
                path.display(),
                e.inner(),
                e.path()
            ))
        })?;
        Ok(config)
    }

    /// Build a redacted snapshot suitable for persisting alongside run
    /// artifacts (AC-NFC-19): the `openrouter.api_key` field is masked.
    pub fn redacted_snapshot(&self) -> serde_json::Value {
        let mut snapshot =
            serde_json::to_value(self).expect("Config must serialise as JSON");
        if let Some(obj) = snapshot
            .get_mut("openrouter")
            .and_then(|v| v.as_object_mut())
        {
            obj.insert("api_key".into(), serde_json::Value::from("***"));
        }
        snapshot
    }
}

/// Small helper so consumers don't have to import `PathBuf` just to express
/// "next to the binary".
pub fn default_config_path(binary_dir: &Path) -> PathBuf {
    binary_dir.join("config.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_config_json() -> &'static str {
        r#"{
  "openrouter": { "api_key": "secret", "base_url": "https://openrouter.ai/api/v1" },
  "models": {
    "scrape_to_markdown":  { "name": "x/y", "temperature": 0.0, "seed": 42 },
    "keyword_extraction":  { "name": "x/y", "temperature": 0.0, "seed": 42 },
    "resume_optimization": { "name": "x/y", "temperature": 0.2, "seed": 42 }
  },
  "scrape": { "network_idle_timeout_ms": 30000 },
  "retries": {
    "llm_transient_max_attempts": 5,
    "llm_transient_backoff_ms": [1000, 2000, 4000, 8000, 16000],
    "schema_validation_max_attempts": 3
  }
}"#
    }

    #[test]
    fn load_happy_path() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, sample_config_json()).unwrap();
        let cfg = Config::load_from(&path).expect("happy-path config should parse");
        assert_eq!(cfg.openrouter.api_key, "secret");
        assert_eq!(cfg.models.resume_optimization.temperature, 0.2);
        assert_eq!(cfg.scrape.network_idle_timeout_ms, 30_000);
    }

    #[test]
    fn missing_file_is_config_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        let err = Config::load_from(&path).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        let msg = err.to_string();
        assert!(
            msg.contains("config.json not found at"),
            "got message: {msg}"
        );
    }

    #[test]
    fn missing_field_names_json_path() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        // Drop `models.keyword_extraction.seed`.
        let missing = r#"{
  "openrouter": { "api_key": "x", "base_url": "y" },
  "models": {
    "scrape_to_markdown":  { "name": "x", "temperature": 0.0, "seed": 42 },
    "keyword_extraction":  { "name": "x", "temperature": 0.0 },
    "resume_optimization": { "name": "x", "temperature": 0.2, "seed": 42 }
  },
  "scrape": { "network_idle_timeout_ms": 30000 },
  "retries": {
    "llm_transient_max_attempts": 5,
    "llm_transient_backoff_ms": [1000],
    "schema_validation_max_attempts": 3
  }
}"#;
        fs::write(&path, missing).unwrap();
        let err = Config::load_from(&path).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        let msg = err.to_string();
        assert!(
            msg.contains("models.keyword_extraction"),
            "missing-field error should name the JSON path, got: {msg}"
        );
    }

    #[test]
    fn malformed_json_is_config_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, "{ not json").unwrap();
        let err = Config::load_from(&path).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn redacted_snapshot_masks_api_key() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, sample_config_json()).unwrap();
        let cfg = Config::load_from(&path).unwrap();
        let snap = cfg.redacted_snapshot();
        assert_eq!(snap["openrouter"]["api_key"], "***");
        assert_eq!(
            snap["openrouter"]["base_url"],
            "https://openrouter.ai/api/v1"
        );
    }
}
