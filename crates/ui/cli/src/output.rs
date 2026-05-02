use crate::error::{CliError, Result};
use rootcause::prelude::*;
use serde::Serialize;
use uptrakit_build_info::BuildInfo;

/// Output format for CLI responses.
///
/// # Design note (`#[non_exhaustive]`)
///
/// `#[non_exhaustive]` is intentionally absent here. `OutputFormat` implements
/// `clap::ValueEnum`, which requires exhaustive matching inside this crate when
/// dispatching output formatting. The only consumers outside the binary are the
/// intra-workspace integration tests, which also match exhaustively. Adding
/// `#[non_exhaustive]` would break those test `match` arms without providing
/// any semver-safety benefit for a crate-internal type.
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

/// Implemented by every type that can be formatted as human-readable CLI output.
///
/// Command handler functions return typed response objects. `main.rs` calls
/// [`print_output`] with the response, and the `Human` format branch delegates
/// to this method. JSON/YAML branches serialise via `serde`.
pub trait HumanOutput {
    fn to_human_string(&self) -> String;
}

impl HumanOutput for BuildInfo {
    fn to_human_string(&self) -> String {
        self.render_human()
    }
}

/// Print a typed, serializable value in the requested format.
///
/// - `Human`: calls `value.to_human_string()`.
/// - `Json`: compact JSON via `serde_json`.
/// - `Yaml`: YAML via `serde_yaml_ng`.
pub fn print_output<T: Serialize + HumanOutput>(format: OutputFormat, value: &T) -> Result<()> {
    match format {
        OutputFormat::Human => {
            print!("{}", value.to_human_string());
        }
        OutputFormat::Json => {
            let json = serde_json::to_string(value).context_to()?;
            println!("{json}");
        }
        OutputFormat::Yaml => {
            let yaml = serde_yaml_ng::to_string(value).context_to::<CliError>()?;
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
            let yaml = serde_yaml_ng::to_string(value).context_to::<CliError>()?;
            print!("{yaml}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — assert!(result.is_ok/is_err()) is idiomatic in tests"
    )]

    use super::*;

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

    struct SimpleOutput {
        text: String,
    }

    impl serde::Serialize for SimpleOutput {
        fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
            s.serialize_str(&self.text)
        }
    }

    impl HumanOutput for SimpleOutput {
        fn to_human_string(&self) -> String {
            self.text.clone()
        }
    }

    #[test]
    fn print_output_human_uses_human_string() {
        let val = SimpleOutput {
            text: "Custom human output\n".to_string(),
        };
        let result = print_output(OutputFormat::Human, &val);
        assert!(result.is_ok());
    }

    #[test]
    fn print_output_json_serializes_value() {
        let val = SimpleOutput {
            text: "hello".to_string(),
        };
        let result = print_output(OutputFormat::Json, &val);
        assert!(result.is_ok());
    }
}
