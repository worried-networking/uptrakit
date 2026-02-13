use std::fmt::Write as _;

use serde::Serialize;

pub const BUILD_FEATURES_ENV: &str = "UPTRAKIT_BUILD_ENABLED_FEATURES";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuildInfo {
    pub binary: String,
    pub version: String,
    pub features: Vec<String>,
    pub target: TargetInfo,
    pub cfg: CfgInfo,
    pub profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TargetInfo {
    pub os: String,
    pub arch: String,
    pub env: String,
    pub family: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CfgInfo {
    pub debug_assertions: bool,
    pub panic_abort: bool,
}

impl BuildInfo {
    pub fn current(binary: &str, version: &str, raw_features: Option<&str>) -> Self {
        let features = parse_enabled_features(raw_features);

        Self {
            binary: binary.to_string(),
            version: version.to_string(),
            features,
            target: TargetInfo {
                os: std::env::consts::OS.to_string(),
                arch: std::env::consts::ARCH.to_string(),
                env: target_env().to_string(),
                family: target_family().to_string(),
            },
            cfg: CfgInfo {
                debug_assertions: cfg!(debug_assertions),
                panic_abort: cfg!(panic = "abort"),
            },
            profile: if cfg!(debug_assertions) {
                "debug".to_string()
            } else {
                "release".to_string()
            },
        }
    }

    pub fn render_human(&self) -> String {
        let mut output = String::new();
        let features = if self.features.is_empty() {
            "none".to_string()
        } else {
            self.features.join(",")
        };

        let _ = writeln!(output, "binary: {}", self.binary);
        let _ = writeln!(output, "version: {}", self.version);
        let _ = writeln!(output, "features: {features}");
        let _ = writeln!(output, "target.os: {}", self.target.os);
        let _ = writeln!(output, "target.arch: {}", self.target.arch);
        let _ = writeln!(output, "target.env: {}", self.target.env);
        let _ = writeln!(output, "target.family: {}", self.target.family);
        let _ = writeln!(
            output,
            "cfg.debug_assertions: {}",
            self.cfg.debug_assertions
        );
        let _ = writeln!(output, "cfg.panic_abort: {}", self.cfg.panic_abort);
        let _ = writeln!(output, "profile: {}", self.profile);

        output
    }
}

pub fn parse_enabled_features(raw_features: Option<&str>) -> Vec<String> {
    normalize_feature_list(raw_features.unwrap_or_default().split(','))
}

pub fn emit_enabled_features_env() {
    let raw_features = std::env::var("CARGO_CFG_FEATURE").ok();
    let normalized = parse_enabled_features(raw_features.as_deref());

    println!(
        "cargo:rustc-env={}={}",
        BUILD_FEATURES_ENV,
        normalized.join(",")
    );
    println!("cargo:rerun-if-env-changed=CARGO_CFG_FEATURE");
}

fn normalize_feature_list<'a>(values: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut normalized = values
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    normalized.sort_unstable();
    normalized.dedup();
    normalized
}

fn target_env() -> &'static str {
    if cfg!(target_env = "gnu") {
        "gnu"
    } else if cfg!(target_env = "musl") {
        "musl"
    } else if cfg!(target_env = "msvc") {
        "msvc"
    } else if cfg!(target_env = "sgx") {
        "sgx"
    } else {
        ""
    }
}

fn target_family() -> &'static str {
    if cfg!(target_family = "unix") {
        "unix"
    } else if cfg!(target_family = "windows") {
        "windows"
    } else if cfg!(target_family = "wasm") {
        "wasm"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::{BuildInfo, parse_enabled_features};

    #[test]
    fn parse_enabled_features_handles_missing() {
        let parsed = parse_enabled_features(None);
        assert!(parsed.is_empty());
    }

    #[test]
    fn parse_enabled_features_sorts_and_deduplicates() {
        let parsed = parse_enabled_features(Some("db-mysql,swagger-ui,db-mysql,db-sqlite"));
        assert_eq!(parsed, vec!["db-mysql", "db-sqlite", "swagger-ui"]);
    }

    #[test]
    fn parse_enabled_features_ignores_whitespace_and_empty_values() {
        let parsed = parse_enabled_features(Some(" , swagger-ui, ,db-sqlite ,,"));
        assert_eq!(parsed, vec!["db-sqlite", "swagger-ui"]);
    }

    #[test]
    fn render_human_uses_stable_keys_and_order() {
        let info = BuildInfo {
            binary: "uptrakit-controller".to_string(),
            version: "1.2.3".to_string(),
            features: vec!["db-sqlite".to_string(), "swagger-ui".to_string()],
            target: super::TargetInfo {
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
                env: "gnu".to_string(),
                family: "unix".to_string(),
            },
            cfg: super::CfgInfo {
                debug_assertions: false,
                panic_abort: true,
            },
            profile: "release".to_string(),
        };

        let rendered = info.render_human();
        assert_eq!(
            rendered,
            "binary: uptrakit-controller\n\
version: 1.2.3\n\
features: db-sqlite,swagger-ui\n\
target.os: linux\n\
target.arch: x86_64\n\
target.env: gnu\n\
target.family: unix\n\
cfg.debug_assertions: false\n\
cfg.panic_abort: true\n\
profile: release\n"
        );
    }

    #[test]
    fn render_human_uses_none_for_empty_feature_list() {
        let info = BuildInfo {
            binary: "uptrakit-agent".to_string(),
            version: "0.0.1".to_string(),
            features: Vec::new(),
            target: super::TargetInfo {
                os: "linux".to_string(),
                arch: "aarch64".to_string(),
                env: "gnu".to_string(),
                family: "unix".to_string(),
            },
            cfg: super::CfgInfo {
                debug_assertions: true,
                panic_abort: false,
            },
            profile: "debug".to_string(),
        };

        assert!(info.render_human().contains("features: none\n"));
    }
}
