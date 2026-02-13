use crate::error::{CliError, Result};
use rootcause::prelude::*;
use serde::Serialize;

/// Output format for CLI responses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Human-readable output (default)
    #[default]
    Human,
    /// Compact JSON output
    Json,
    /// YAML output
    Yaml,
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Human => write!(f, "human"),
            Self::Json => write!(f, "json"),
            Self::Yaml => write!(f, "yaml"),
        }
    }
}

/// Print a typed, serializable value in the requested format.
///
/// - `Human`: prints the pre-formatted `human_text` string.
/// - `Json`: compact JSON via `serde_json`.
/// - `Yaml`: YAML via `serde_yaml_ng`.
pub fn print_output<T: Serialize>(format: OutputFormat, human_text: &str, value: &T) -> Result<()> {
    match format {
        OutputFormat::Human => {
            print!("{human_text}");
        }
        OutputFormat::Json => {
            let json = serde_json::to_string(value).context_to()?;
            println!("{json}");
        }
        OutputFormat::Yaml => {
            let yaml = serde_yaml_ng::to_string(value)
                .map_err(|e| report!(CliError::Other(format!("YAML serialization error: {e}"))))?;
            print!("{yaml}");
        }
    }
    Ok(())
}

/// Print a `serde_json::Value` in the requested format.
///
/// Used by the `api` command which already works with `Value`.
///
/// - `Human`: pretty-printed JSON (current behaviour).
/// - `Json`: compact JSON.
/// - `Yaml`: YAML.
pub fn print_value(format: OutputFormat, value: &serde_json::Value) -> Result<()> {
    match format {
        OutputFormat::Human => {
            let pretty = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
            println!("{pretty}");
        }
        OutputFormat::Json => {
            let json = serde_json::to_string(value).context_to()?;
            println!("{json}");
        }
        OutputFormat::Yaml => {
            let yaml = serde_yaml_ng::to_string(value)
                .map_err(|e| report!(CliError::Other(format!("YAML serialization error: {e}"))))?;
            print!("{yaml}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Sample {
        name: String,
        count: u32,
    }

    #[test]
    fn default_format_is_human() {
        assert_eq!(OutputFormat::default(), OutputFormat::Human);
    }

    #[test]
    fn display_formats() {
        assert_eq!(OutputFormat::Human.to_string(), "human");
        assert_eq!(OutputFormat::Json.to_string(), "json");
        assert_eq!(OutputFormat::Yaml.to_string(), "yaml");
    }

    #[test]
    fn json_output_is_compact() {
        let sample = Sample {
            name: "test".to_string(),
            count: 42,
        };
        let json = serde_json::to_string(&sample).expect("json serialization");
        assert!(
            !json.contains('\n'),
            "compact JSON should not contain newlines"
        );
        assert_eq!(json, r#"{"name":"test","count":42}"#);
    }

    #[test]
    fn yaml_output_is_valid() {
        let sample = Sample {
            name: "test".to_string(),
            count: 42,
        };
        let yaml = serde_yaml_ng::to_string(&sample).expect("yaml serialization");
        let parsed: Sample = serde_yaml_ng::from_str(&yaml).expect("yaml deserialization");
        assert_eq!(parsed, sample);
    }

    #[test]
    fn json_value_compact() {
        let value = serde_json::json!({"key": "value", "num": 1});
        let json = serde_json::to_string(&value).expect("json serialization");
        assert!(!json.contains('\n'));
    }

    #[test]
    fn yaml_value_valid() {
        let value = serde_json::json!({"key": "value", "num": 1});
        let yaml = serde_yaml_ng::to_string(&value).expect("yaml serialization");
        let parsed: serde_json::Value =
            serde_yaml_ng::from_str(&yaml).expect("yaml deserialization");
        assert_eq!(parsed["key"], "value");
        assert_eq!(parsed["num"], 1);
    }
}
